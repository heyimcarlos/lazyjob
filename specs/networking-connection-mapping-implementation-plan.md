# Implementation Plan: Networking Connection Mapping

## Status
Draft

## Related Spec
[specs/networking-connection-mapping.md](networking-connection-mapping.md)

## Overview

The networking connection mapping module cross-references a user's imported professional contacts against their active job feed to surface warm-path introductions. For any job, it ranks every relevant contact by warmth tier (current employee → recent alumni → distant alumni → second-degree heuristic → cold) and recommends an engagement approach (referral ask, informational interview, reconnect, cold outreach). All tier scoring is computed at query time from live contact data — tiers are never persisted, preventing staleness.

This module is the backbone of JTBD A-4 (warm introductions beat cold applications). It sits in `lazyjob-core/src/networking/` and integrates with `CompanyRepository`, `JobRepository`, the life-sheet `profile_contacts` table, and the TUI job detail view. Because LazyJob has no LinkedIn API access, all contact data is user-imported via LinkedIn CSV export, TUI manual entry, or promotion from application contacts.

The design is intentionally local-first: no graph databases, no external APIs. A simple adjacency-list SQLite schema plus Rust-side scoring is sufficient for the realistic contact set size (<10,000 contacts). The warmth-tier computation and approach classification are pure synchronous functions, keeping the async surface minimal and the unit test surface maximal.

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — connection pool, `run_migrations`, migration framework
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `profile_contacts` table, `ProfileContact` domain type, `LifeSheetId`
- `specs/job-search-company-research-implementation-plan.md` — `CompanyRecord`, `CompanyRepository`, `normalize_company_name()`
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobRecord`, `JobRepository`
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel/view system, keybinding dispatch

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
csv = "1.3"            # LinkedIn CSV parsing; pure Rust, no unsafe
strsim = "0.11"        # Jaro-Winkler for company name fuzzy matching (already used in other modules)
```

No new crates for the core module — `uuid`, `chrono`, `serde`, `serde_json`, `sqlx`, `thiserror`, `anyhow`, `tokio` are already present.

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| Domain types (`ConnectionTier`, `SuggestedApproach`, `WarmPath`, `ProfileContact`) | `lazyjob-core` | `src/networking/types.rs` |
| `ContactRepository` trait + `SqliteContactRepository` | `lazyjob-core` | `src/networking/contact_repo.rs` |
| Company name normalization | `lazyjob-core` | `src/networking/normalize.rs` |
| `ConnectionMapper` (warm path logic) | `lazyjob-core` | `src/networking/connection_mapper.rs` |
| LinkedIn CSV import | `lazyjob-core` | `src/networking/csv_import.rs` |
| `ContactService` (high-level orchestrator) | `lazyjob-core` | `src/networking/contact_service.rs` |
| SQLite migrations (015) | `lazyjob-core` | `migrations/015_networking_contacts.sql` |
| TUI Warm Paths panel | `lazyjob-tui` | `src/views/job_detail/warm_paths.rs` |
| TUI Contact Browser | `lazyjob-tui` | `src/views/contacts/browser.rs` |
| TUI Contact Entry form | `lazyjob-tui` | `src/views/contacts/entry_form.rs` |

### Core Types

```rust
// lazyjob-core/src/networking/types.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Strongly-typed contact ID. Parse-don't-validate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct ContactId(pub Uuid);
impl ContactId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// A previous employer record stored in `profile_contacts.previous_companies_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviousCompany {
    pub company_name: String,
    pub title: Option<String>,
    pub start_year: Option<i32>,
    pub end_year: Option<i32>,
}

/// An educational institution record stored in `profile_contacts.schools_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactSchool {
    pub institution: String,
    pub degree: Option<String>,
    pub graduation_year: Option<i32>,
}

/// How the contact was added to the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum ContactSource {
    #[sqlx(rename = "manual")]
    Manual,
    #[sqlx(rename = "linkedin_csv")]
    LinkedInCsv,
    #[sqlx(rename = "application_promoted")]
    ApplicationPromoted,
}

/// The authoritative contact record from `profile_contacts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileContact {
    pub id: ContactId,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub current_company_name: Option<String>,
    pub current_title: Option<String>,
    /// `None` = currently employed here (still at this company).
    pub current_company_departed: Option<NaiveDate>,
    pub previous_companies: Vec<PreviousCompany>,
    pub schools: Vec<ContactSchool>,
    pub relationship_notes: Option<String>,
    pub last_contacted_at: Option<NaiveDate>,
    pub contact_source: ContactSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Relationship classification between a contact and a target company.
/// `months_since_departure` is `0` for current employees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionTier {
    FirstDegreeCurrentEmployee,
    FirstDegreeRecentAlumni { months_since_departure: u32 },   // 1–24 months
    FirstDegreeAlumni { months_since_departure: u32 },          // 25–60 months
    FirstDegreeDistantAlumni { months_since_departure: u32 },   // > 60 months
    SharedAlumni { institution: String },
    SecondDegreeHeuristic { via_shared_employer: String },
    Cold,
}

impl ConnectionTier {
    /// Numeric score for ranking. Higher = warmer.
    pub fn score(&self) -> u8 {
        match self {
            Self::FirstDegreeCurrentEmployee => 100,
            Self::FirstDegreeRecentAlumni { .. } => 70,
            Self::FirstDegreeAlumni { .. } => 40,
            Self::FirstDegreeDistantAlumni { .. } => 20,
            Self::SharedAlumni { .. } => 15,
            Self::SecondDegreeHeuristic { .. } => 10,
            Self::Cold => 0,
        }
    }

    /// Returns whether a contact is in any first-degree tier.
    pub fn is_first_degree(&self) -> bool {
        matches!(
            self,
            Self::FirstDegreeCurrentEmployee
                | Self::FirstDegreeRecentAlumni { .. }
                | Self::FirstDegreeAlumni { .. }
                | Self::FirstDegreeDistantAlumni { .. }
        )
    }
}

/// Recommended outreach approach given the tier and recency of contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestedApproach {
    RequestReferral,
    InformationalInterview,
    ReconnectFirst,
    ColdOutreach,
}

/// A single ranked entry in the warm paths list for a job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmPath {
    pub contact_id: ContactId,
    pub contact_name: String,
    pub contact_current_title: Option<String>,
    pub company_id: Uuid,
    pub tier: ConnectionTier,
    pub tier_score: u8,
    pub last_contacted_at: Option<NaiveDate>,
    pub relationship_notes: Option<String>,
    pub suggested_approach: SuggestedApproach,
}

/// Summary returned from a batch import operation.
#[derive(Debug, Default)]
pub struct ImportResult {
    pub inserted: usize,
    pub updated: usize,
    pub skipped_duplicates: usize,
    pub parse_errors: Vec<String>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/networking/contact_repo.rs

use async_trait::async_trait;
use crate::networking::types::*;
use crate::error::Result;
use uuid::Uuid;

#[async_trait]
pub trait ContactRepository: Send + Sync {
    async fn list_contacts(&self) -> Result<Vec<ProfileContact>>;
    async fn get_contact(&self, id: &ContactId) -> Result<Option<ProfileContact>>;
    async fn upsert_contact(&self, contact: ProfileContact) -> Result<ContactId>;
    async fn delete_contact(&self, id: &ContactId) -> Result<()>;
    async fn find_by_email(&self, email: &str) -> Result<Option<ProfileContact>>;
    /// Returns contacts whose `current_company_name` (normalized) matches the
    /// normalized form of any alias for the given company.
    async fn contacts_with_company_name(&self, normalized_name: &str) -> Result<Vec<ProfileContact>>;
    /// Returns contacts whose `previous_companies_json` contains a record matching
    /// the given normalized company name.
    async fn contacts_with_previous_company(&self, normalized_name: &str) -> Result<Vec<ProfileContact>>;
    /// Returns total contact count (for TUI status bar).
    async fn count(&self) -> Result<i64>;
}
```

### SQLite Schema

Migration file: `lazyjob-core/migrations/015_networking_contacts.sql`

```sql
-- Extends profile_contacts (created in migration 005 by life-sheet spec).
-- profile_contacts already has: id, first_name, last_name, email, company, position, created_at
-- We add networking-specific columns via ALTER TABLE.

ALTER TABLE profile_contacts ADD COLUMN current_company_departed DATE;
ALTER TABLE profile_contacts ADD COLUMN previous_companies_json   TEXT NOT NULL DEFAULT '[]';
ALTER TABLE profile_contacts ADD COLUMN schools_json              TEXT NOT NULL DEFAULT '[]';
ALTER TABLE profile_contacts ADD COLUMN relationship_notes        TEXT;
ALTER TABLE profile_contacts ADD COLUMN last_contacted_at         DATE;
ALTER TABLE profile_contacts ADD COLUMN contact_source            TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE profile_contacts ADD COLUMN updated_at                DATETIME NOT NULL DEFAULT (datetime('now'));

-- Fast lookup for email-based dedup on import.
CREATE UNIQUE INDEX IF NOT EXISTS idx_profile_contacts_email
    ON profile_contacts (email)
    WHERE email IS NOT NULL;

-- Fast lookup for company-name matching (COLLATE NOCASE handled in SQL query).
CREATE INDEX IF NOT EXISTS idx_profile_contacts_current_company
    ON profile_contacts (current_company_name COLLATE NOCASE)
    WHERE current_company_name IS NOT NULL;

-- Contact-company adjacency table: records the explicit mapping result.
-- This is a cache of ConnectionMapper output — cleared and rebuilt on each import
-- or when the user triggers a manual rescan.
CREATE TABLE IF NOT EXISTS contact_company_links (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    contact_id      TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    company_id      TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    link_type       TEXT NOT NULL,      -- 'current_employee' | 'recent_alumni' | 'alumni' | 'distant_alumni' | 'second_degree' | 'shared_alumni'
    months_since_departure INTEGER,     -- NULL for current employees
    via_shared_employer    TEXT,        -- for second_degree links
    institution            TEXT,        -- for shared_alumni links
    created_at      DATETIME NOT NULL DEFAULT (datetime('now')),
    UNIQUE (contact_id, company_id, link_type)
);

CREATE INDEX IF NOT EXISTS idx_contact_company_links_company
    ON contact_company_links (company_id);

CREATE INDEX IF NOT EXISTS idx_contact_company_links_contact
    ON contact_company_links (contact_id);
```

### Module Structure

```
lazyjob-core/
  src/
    networking/
      mod.rs                  -- pub use re-exports
      types.rs                -- ConnectionTier, SuggestedApproach, WarmPath, ProfileContact, ...
      contact_repo.rs         -- ContactRepository trait + SqliteContactRepository
      normalize.rs            -- normalize_company_name(), company_names_match()
      connection_mapper.rs    -- ConnectionMapper, tier computation, approach classification
      csv_import.rs           -- LinkedInCsvImporter
      contact_service.rs      -- ContactService (high-level orchestrator)
  migrations/
    015_networking_contacts.sql

lazyjob-tui/
  src/
    views/
      job_detail/
        warm_paths.rs         -- WarmPathsPanel widget
      contacts/
        browser.rs            -- ContactBrowserView (full-screen list)
        entry_form.rs         -- ContactEntryForm (modal overlay)
```

## Implementation Phases

### Phase 1 — Core Types, Repository, and Company Name Normalization (MVP)

#### Step 1.1 — Define types (`lazyjob-core/src/networking/types.rs`)

Implement all types in the **Core Types** section verbatim. Add `impl Default for ContactId`, `impl From<Uuid> for ContactId`, and `impl std::fmt::Display for ContactId`.

Verification: `cargo check --package lazyjob-core` passes with no warnings.

#### Step 1.2 — Company name normalization (`lazyjob-core/src/networking/normalize.rs`)

```rust
use once_cell::sync::Lazy;
use regex::Regex;
use strsim::jaro_winkler;

/// Compiled patterns for legal-suffix stripping.
static LEGAL_SUFFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(inc\.?|llc\.?|ltd\.?|corp\.?|co\.?|group|holdings?|international|technologies?|tech|solutions?)\b\.?$")
        .unwrap()
});
static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// Normalize a company name for deduplication matching.
/// Output: lowercase, legal suffix stripped, punctuation removed, whitespace collapsed.
pub fn normalize_company_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let no_suffix = LEGAL_SUFFIX.replace(&lower, "");
    let no_punct: String = no_suffix
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    WHITESPACE.replace_all(no_punct.trim(), " ").to_string()
}

/// Returns true if two normalized names match: exact equality
/// OR Jaro-Winkler similarity ≥ 0.92.
pub fn company_names_match(a: &str, b: &str) -> bool {
    a == b || jaro_winkler(a, b) >= 0.92
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_legal_suffixes() {
        assert_eq!(normalize_company_name("Stripe, Inc."), "stripe");
        assert_eq!(normalize_company_name("DeepMind Technologies"), "deepmind");
        assert_eq!(normalize_company_name("Acme Corp"), "acme");
    }

    #[test]
    fn fuzzy_match_threshold() {
        assert!(company_names_match("google", "gogle")); // typo
        assert!(!company_names_match("apple", "apricot"));
    }
}
```

Key API: `regex::Regex` via `once_cell::sync::Lazy` (zero-cost after first call), `strsim::jaro_winkler(a, b) -> f64`.

Verification: unit tests pass with `cargo test -p lazyjob-core networking::normalize`.

#### Step 1.3 — SQLite migration (`lazyjob-core/migrations/015_networking_contacts.sql`)

Write the DDL from the **SQLite Schema** section. The migration runner uses `sqlx::migrate!("migrations")` — no additional wiring needed.

#### Step 1.4 — `SqliteContactRepository` (`lazyjob-core/src/networking/contact_repo.rs`)

```rust
use sqlx::{Row, SqlitePool};
use crate::networking::types::*;
use crate::error::{NetworkingError, Result};
use async_trait::async_trait;
use uuid::Uuid;

pub struct SqliteContactRepository {
    pool: SqlitePool,
}

impl SqliteContactRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl ContactRepository for SqliteContactRepository {
    async fn upsert_contact(&self, contact: ProfileContact) -> Result<ContactId> {
        let prev_json = serde_json::to_string(&contact.previous_companies)
            .map_err(|e| NetworkingError::Serialization(e.to_string()))?;
        let schools_json = serde_json::to_string(&contact.schools)
            .map_err(|e| NetworkingError::Serialization(e.to_string()))?;
        let id = contact.id.0.to_string();
        sqlx::query!(
            r#"INSERT INTO profile_contacts
               (id, first_name, last_name, email, current_company_name, current_title,
                current_company_departed, previous_companies_json, schools_json,
                relationship_notes, last_contacted_at, contact_source, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))
               ON CONFLICT(id) DO UPDATE SET
                 first_name               = excluded.first_name,
                 last_name                = excluded.last_name,
                 email                    = COALESCE(excluded.email, profile_contacts.email),
                 current_company_name     = excluded.current_company_name,
                 current_title            = excluded.current_title,
                 current_company_departed = excluded.current_company_departed,
                 previous_companies_json  = excluded.previous_companies_json,
                 schools_json             = excluded.schools_json,
                 relationship_notes       = excluded.relationship_notes,
                 last_contacted_at        = excluded.last_contacted_at,
                 contact_source           = excluded.contact_source,
                 updated_at               = datetime('now')"#,
            id,
            contact.first_name,
            contact.last_name,
            contact.email,
            contact.current_company_name,
            contact.current_title,
            contact.current_company_departed,
            prev_json,
            schools_json,
            contact.relationship_notes,
            contact.last_contacted_at,
            contact.contact_source as ContactSource,
        )
        .execute(&self.pool)
        .await
        .map_err(NetworkingError::Database)?;
        Ok(contact.id)
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<ProfileContact>> {
        // ... SELECT + deserialize previous_companies_json / schools_json
        todo!()
    }

    // ... remaining trait methods
}
```

Key APIs:
- `sqlx::query!` macro with `ON CONFLICT(id) DO UPDATE SET` for idempotent upserts
- `serde_json::to_string` / `serde_json::from_str` for JSON columns
- `sqlx::SqlitePool::execute` / `fetch_optional` / `fetch_all`

Verification: `#[sqlx::test(migrations = "migrations")]` integration test — upsert a contact, fetch by email, assert fields match.

### Phase 2 — ConnectionMapper and Tier Computation

#### Step 2.1 — `ConnectionMapper` struct (`lazyjob-core/src/networking/connection_mapper.rs`)

```rust
use std::sync::Arc;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use uuid::Uuid;
use crate::networking::{
    contact_repo::ContactRepository,
    normalize::{company_names_match, normalize_company_name},
    types::*,
};
use crate::company::CompanyRepository;
use crate::jobs::JobRepository;
use crate::error::Result;

pub struct ConnectionMapper {
    pub contact_repo: Arc<dyn ContactRepository>,
    pub company_repo: Arc<dyn CompanyRepository>,
    pub job_repo: Arc<dyn JobRepository>,
}

impl ConnectionMapper {
    pub fn new(
        contact_repo: Arc<dyn ContactRepository>,
        company_repo: Arc<dyn CompanyRepository>,
        job_repo: Arc<dyn JobRepository>,
    ) -> Self {
        Self { contact_repo, company_repo, job_repo }
    }

    /// Warm paths for a specific job, sorted by tier_score DESC.
    pub async fn warm_paths_for_job(&self, job_id: Uuid) -> Result<Vec<WarmPath>> {
        let job = self.job_repo.get(job_id).await?
            .ok_or_else(|| NetworkingError::JobNotFound(job_id))?;
        let company = self.company_repo.find_by_id(job.company_id).await?;
        let Some(company) = company else { return Ok(vec![]); };
        let normalized = normalize_company_name(&company.name);
        self.warm_paths_for_normalized_company(company.id, &normalized, &company.schools_hint).await
    }

    /// All warm paths for a given company_id.
    pub async fn contacts_at_company(&self, company_id: Uuid) -> Result<Vec<WarmPath>> {
        let company = self.company_repo.find_by_id(company_id).await?
            .ok_or_else(|| NetworkingError::CompanyNotFound(company_id))?;
        let normalized = normalize_company_name(&company.name);
        self.warm_paths_for_normalized_company(company_id, &normalized, &company.schools_hint).await
    }

    async fn warm_paths_for_normalized_company(
        &self,
        company_id: Uuid,
        normalized_name: &str,
        _schools_hint: &[String],
    ) -> Result<Vec<WarmPath>> {
        let mut paths: Vec<WarmPath> = Vec::new();
        let today = Utc::now().date_naive();

        // --- First-degree: current company match ---
        let current_contacts = self.contact_repo.contacts_with_company_name(normalized_name).await?;
        for contact in &current_contacts {
            let tier = compute_current_tier(contact, today);
            let approach = classify_approach(&tier, contact.last_contacted_at, today);
            paths.push(WarmPath {
                contact_id: contact.id.clone(),
                contact_name: format!("{} {}", contact.first_name, contact.last_name),
                contact_current_title: contact.current_title.clone(),
                company_id,
                tier_score: tier.score(),
                tier,
                last_contacted_at: contact.last_contacted_at,
                relationship_notes: contact.relationship_notes.clone(),
                suggested_approach: approach,
            });
        }

        // --- First-degree alumni: previous company matches ---
        let alumni_contacts = self.contact_repo.contacts_with_previous_company(normalized_name).await?;
        for contact in &alumni_contacts {
            // Avoid duplicating current-company matches
            if current_contacts.iter().any(|c| c.id == contact.id) { continue; }
            for prev in &contact.previous_companies {
                let prev_normalized = normalize_company_name(&prev.company_name);
                if !company_names_match(&prev_normalized, normalized_name) { continue; }
                let tier = compute_alumni_tier(prev, today);
                let approach = classify_approach(&tier, contact.last_contacted_at, today);
                paths.push(WarmPath {
                    contact_id: contact.id.clone(),
                    contact_name: format!("{} {}", contact.first_name, contact.last_name),
                    contact_current_title: contact.current_title.clone(),
                    company_id,
                    tier_score: tier.score(),
                    tier,
                    last_contacted_at: contact.last_contacted_at,
                    relationship_notes: contact.relationship_notes.clone(),
                    suggested_approach: approach,
                });
                break; // one path per contact per company
            }
        }

        paths.sort_by(|a, b| b.tier_score.cmp(&a.tier_score));
        Ok(paths)
    }
}

/// Determines tier for a contact whose current_company matches the target.
/// `departed` is `None` for currently employed contacts.
fn compute_current_tier(contact: &ProfileContact, today: NaiveDate) -> ConnectionTier {
    match contact.current_company_departed {
        None => ConnectionTier::FirstDegreeCurrentEmployee,
        Some(departed) => {
            let months = months_between(departed, today);
            compute_alumni_tier_from_months(months)
        }
    }
}

/// Determines tier for a contact via a `PreviousCompany` record.
fn compute_alumni_tier(prev: &PreviousCompany, today: NaiveDate) -> ConnectionTier {
    let months = prev.end_year
        .map(|y| {
            let approx_end = NaiveDate::from_ymd_opt(y, 12, 31).unwrap_or(today);
            months_between(approx_end, today)
        })
        .unwrap_or(u32::MAX); // unknown end date → treat as very old
    compute_alumni_tier_from_months(months)
}

fn compute_alumni_tier_from_months(months: u32) -> ConnectionTier {
    match months {
        0..=24  => ConnectionTier::FirstDegreeRecentAlumni { months_since_departure: months },
        25..=60 => ConnectionTier::FirstDegreeAlumni { months_since_departure: months },
        _       => ConnectionTier::FirstDegreeDistantAlumni { months_since_departure: months },
    }
}

fn months_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let years = (to.year() - from.year()) as u32;
    let months = to.month().saturating_sub(from.month());
    years * 12 + months
}

/// Classifies the recommended outreach approach per the spec decision table.
fn classify_approach(
    tier: &ConnectionTier,
    last_contacted: Option<NaiveDate>,
    today: NaiveDate,
) -> SuggestedApproach {
    let days_since_contact = last_contacted
        .map(|d| (today - d).num_days())
        .unwrap_or(i64::MAX);

    match tier {
        ConnectionTier::FirstDegreeCurrentEmployee => {
            if days_since_contact < 90 {
                SuggestedApproach::RequestReferral
            } else {
                SuggestedApproach::ReconnectFirst
            }
        }
        ConnectionTier::FirstDegreeRecentAlumni { .. }
        | ConnectionTier::FirstDegreeAlumni { .. }
        | ConnectionTier::SharedAlumni { .. } => SuggestedApproach::InformationalInterview,
        ConnectionTier::FirstDegreeDistantAlumni { .. } => SuggestedApproach::ReconnectFirst,
        ConnectionTier::SecondDegreeHeuristic { .. }
        | ConnectionTier::Cold => SuggestedApproach::ColdOutreach,
    }
}
```

Verification: unit tests for `compute_current_tier` (departed 6 months ago → `RecentAlumni`), `classify_approach` (current + 45 days → `RequestReferral`), `months_between`.

### Phase 3 — LinkedIn CSV Import

#### Step 3.1 — `LinkedInCsvImporter` (`lazyjob-core/src/networking/csv_import.rs`)

LinkedIn CSV format (column-name-based, not position-based):

```
First Name,Last Name,URL,Email Address,Company,Position,Connected On
```

```rust
use csv::Reader;
use std::path::Path;
use crate::networking::{contact_repo::ContactRepository, types::*};
use crate::networking::normalize::normalize_company_name;
use crate::error::Result;
use uuid::Uuid;
use chrono::NaiveDate;
use std::sync::Arc;

pub struct LinkedInCsvImporter {
    contact_repo: Arc<dyn ContactRepository>,
}

impl LinkedInCsvImporter {
    pub fn new(contact_repo: Arc<dyn ContactRepository>) -> Self {
        Self { contact_repo }
    }

    pub async fn import(&self, csv_path: &Path) -> Result<ImportResult> {
        // Column-name-based parsing — position-independent.
        // Uses tokio::task::spawn_blocking because csv::Reader is sync.
        let csv_path = csv_path.to_path_buf();
        let records = tokio::task::spawn_blocking(move || {
            parse_linkedin_csv(&csv_path)
        })
        .await
        .map_err(|e| NetworkingError::Import(e.to_string()))??;

        let mut result = ImportResult::default();
        for (line, record) in records {
            match self.contact_repo.find_by_email(record.email.as_deref().unwrap_or("")).await {
                Ok(Some(existing)) => {
                    // Merge: update company/position, keep existing relationship notes.
                    let mut updated = existing;
                    updated.current_company_name = record.current_company_name;
                    updated.current_title = record.current_title;
                    updated.contact_source = ContactSource::LinkedInCsv;
                    self.contact_repo.upsert_contact(updated).await?;
                    result.updated += 1;
                }
                Ok(None) => {
                    self.contact_repo.upsert_contact(record).await?;
                    result.inserted += 1;
                }
                Err(e) => {
                    result.parse_errors.push(format!("Line {line}: {e}"));
                }
            }
        }
        Ok(result)
    }
}

/// Sync CSV parsing — runs inside spawn_blocking.
fn parse_linkedin_csv(path: &Path) -> Result<Vec<(usize, ProfileContact)>> {
    let mut rdr = Reader::from_path(path)
        .map_err(|e| NetworkingError::Import(e.to_string()))?;

    // Read headers first for column-name-based access.
    let headers = rdr.headers()
        .map_err(|e| NetworkingError::Import(e.to_string()))?
        .clone();

    let col = |name: &str| -> Option<usize> {
        headers.iter().position(|h| h.trim().eq_ignore_ascii_case(name))
    };

    let first_name_col  = col("First Name");
    let last_name_col   = col("Last Name");
    let email_col       = col("Email Address");
    let company_col     = col("Company");
    let position_col    = col("Position");
    let connected_col   = col("Connected On");

    let mut records = Vec::new();
    for (i, result) in rdr.records().enumerate() {
        let row = result.map_err(|e| NetworkingError::Import(e.to_string()))?;
        let get = |idx: Option<usize>| idx.and_then(|i| row.get(i)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);

        let first_name = get(first_name_col).unwrap_or_else(|| "(unknown)".into());
        let last_name  = get(last_name_col).unwrap_or_default();
        let email      = get(email_col);
        let company    = get(company_col);
        let position   = get(position_col);
        let _connected = get(connected_col); // tracked but not used in Phase 1

        let contact = ProfileContact {
            id: ContactId::new(),
            first_name,
            last_name,
            email,
            current_company_name: company,
            current_title: position,
            current_company_departed: None,
            previous_companies: vec![],
            schools: vec![],
            relationship_notes: None,
            last_contacted_at: None,
            contact_source: ContactSource::LinkedInCsv,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        records.push((i + 2, contact)); // +2: 1 for header, 1 for 1-based
    }
    Ok(records)
}
```

Key APIs:
- `csv::Reader::from_path(path)` — sync reader; must be called in `spawn_blocking`
- `csv::Reader::headers()` — returns `csv::StringRecord` with column names
- `tokio::task::spawn_blocking(|| { ... })` — offloads sync I/O to blocking thread pool
- `csv::StringRecord::get(idx)` — column-index access

Verification: integration test with a fixture `fixtures/linkedin_sample.csv`. Assert `ImportResult { inserted: 3, updated: 0, .. }`. Test column-order variation (reorder columns, assert same result). Test missing email column (assert no panic, columns are `None`).

### Phase 4 — `ContactService` Orchestrator

```rust
// lazyjob-core/src/networking/contact_service.rs

use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;
use crate::networking::{
    connection_mapper::ConnectionMapper,
    contact_repo::{ContactRepository, SqliteContactRepository},
    csv_import::LinkedInCsvImporter,
    types::*,
};
use crate::error::Result;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum ContactEvent {
    ImportCompleted(ImportResult),
    ContactUpserted(ContactId),
    ContactDeleted(ContactId),
}

pub struct ContactService {
    mapper: ConnectionMapper,
    importer: LinkedInCsvImporter,
    contact_repo: Arc<dyn ContactRepository>,
    tx: broadcast::Sender<ContactEvent>,
}

impl ContactService {
    pub fn new(
        mapper: ConnectionMapper,
        contact_repo: Arc<dyn ContactRepository>,
    ) -> (Self, broadcast::Receiver<ContactEvent>) {
        let (tx, rx) = broadcast::channel(64);
        let importer = LinkedInCsvImporter::new(Arc::clone(&contact_repo));
        (Self { mapper, importer, contact_repo, tx }, rx)
    }

    pub async fn import_linkedin_csv(&self, path: &Path) -> Result<ImportResult> {
        let result = self.importer.import(path).await?;
        let _ = self.tx.send(ContactEvent::ImportCompleted(result.clone()));
        Ok(result)
    }

    pub async fn warm_paths_for_job(&self, job_id: Uuid) -> Result<Vec<WarmPath>> {
        self.mapper.warm_paths_for_job(job_id).await
    }

    pub async fn contacts_at_company(&self, company_id: Uuid) -> Result<Vec<WarmPath>> {
        self.mapper.contacts_at_company(company_id).await
    }

    pub async fn upsert_contact(&self, contact: ProfileContact) -> Result<ContactId> {
        let id = self.contact_repo.upsert_contact(contact).await?;
        let _ = self.tx.send(ContactEvent::ContactUpserted(id.clone()));
        Ok(id)
    }

    pub async fn delete_contact(&self, id: &ContactId) -> Result<()> {
        self.contact_repo.delete_contact(id).await?;
        let _ = self.tx.send(ContactEvent::ContactDeleted(id.clone()));
        Ok(())
    }

    pub async fn list_contacts(&self) -> Result<Vec<ProfileContact>> {
        self.contact_repo.list_contacts().await
    }
}
```

Verification: integration test — import a CSV, call `warm_paths_for_job` with a job whose company matches a contact in the CSV, assert the WarmPath list contains that contact at tier `FirstDegreeCurrentEmployee`.

### Phase 5 — TUI Integration

#### Step 5.1 — Warm Paths Panel (`lazyjob-tui/src/views/job_detail/warm_paths.rs`)

The panel renders as a right-side pane in the job detail view.

```rust
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use crate::networking::types::{ConnectionTier, SuggestedApproach, WarmPath};

pub struct WarmPathsPanel {
    pub paths: Vec<WarmPath>,
    pub list_state: ListState,
    pub focused: bool,
}

impl WarmPathsPanel {
    pub fn new() -> Self {
        Self { paths: vec![], list_state: ListState::default(), focused: false }
    }

    pub fn set_paths(&mut self, paths: Vec<WarmPath>) {
        self.paths = paths;
        self.list_state = ListState::default();
        if !self.paths.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn selected(&self) -> Option<&WarmPath> {
        self.list_state.selected().and_then(|i| self.paths.get(i))
    }

    pub fn select_next(&mut self) {
        let n = self.paths.len();
        if n == 0 { return; }
        let next = self.list_state.selected().map_or(0, |i| (i + 1).min(n - 1));
        self.list_state.select(Some(next));
    }

    pub fn select_prev(&mut self) {
        let prev = self.list_state.selected().map_or(0, |i| i.saturating_sub(1));
        self.list_state.select(Some(prev));
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.paths.iter().map(|p| {
            let tier_badge = tier_badge(&p.tier);
            let approach_badge = approach_badge(&p.suggested_approach);
            let title_str = p.contact_current_title.as_deref().unwrap_or("—");
            let line = Line::from(vec![
                tier_badge,
                Span::raw(" "),
                Span::styled(&p.contact_name, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("  {title_str}")),
                Span::raw("  "),
                approach_badge,
            ]);
            ListItem::new(line)
        }).collect();

        let title = if self.paths.is_empty() {
            "Warm Paths (none)"
        } else {
            "Warm Paths (n = press Enter to draft outreach)"
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if self.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            });

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}

fn tier_badge(tier: &ConnectionTier) -> Span<'static> {
    match tier {
        ConnectionTier::FirstDegreeCurrentEmployee =>
            Span::styled("●", Style::default().fg(Color::Green)),
        ConnectionTier::FirstDegreeRecentAlumni { .. } =>
            Span::styled("◉", Style::default().fg(Color::LightGreen)),
        ConnectionTier::FirstDegreeAlumni { .. } =>
            Span::styled("○", Style::default().fg(Color::Yellow)),
        ConnectionTier::FirstDegreeDistantAlumni { .. } =>
            Span::styled("○", Style::default().fg(Color::Gray)),
        ConnectionTier::SharedAlumni { .. } =>
            Span::styled("◌", Style::default().fg(Color::Cyan)),
        ConnectionTier::SecondDegreeHeuristic { .. } =>
            Span::styled("·", Style::default().fg(Color::DarkGray)),
        ConnectionTier::Cold =>
            Span::styled("–", Style::default().fg(Color::DarkGray)),
    }
}

fn approach_badge(approach: &SuggestedApproach) -> Span<'static> {
    match approach {
        SuggestedApproach::RequestReferral =>
            Span::styled("[referral]", Style::default().fg(Color::Green)),
        SuggestedApproach::InformationalInterview =>
            Span::styled("[info]", Style::default().fg(Color::Cyan)),
        SuggestedApproach::ReconnectFirst =>
            Span::styled("[reconnect]", Style::default().fg(Color::Yellow)),
        SuggestedApproach::ColdOutreach =>
            Span::styled("[cold]", Style::default().fg(Color::Gray)),
    }
}
```

Key APIs:
- `ratatui::widgets::List::new(items)` + `ListState` for keyboard-navigable lists
- `ratatui::widgets::ListItem::new(line)` accepting a `ratatui::text::Line`
- `frame.render_stateful_widget(list, area, &mut self.list_state)` — renders with selection highlight
- `Block::default().borders(Borders::ALL)` — standard panel chrome

Keybinding wired in the job detail view controller:
- `n` → toggle focus to `WarmPathsPanel`
- `j`/`k` → `select_next`/`select_prev` when focused
- `Enter` → push `OutreachDraftingView` with the selected `WarmPath.contact_id` + `job_id`
- `Esc` → return focus to job detail main panel

Verification: render a `WarmPathsPanel` with 3 mock paths in a 80×24 buffer using `ratatui::backend::TestBackend`; assert color codes and contact names appear.

#### Step 5.2 — Contact Browser (`lazyjob-tui/src/views/contacts/browser.rs`)

Full-screen contact list with search and sort. Keybinding: `C` from the main nav bar.

Layout:
```
┌─ Contacts (342) ──────────────────────────────────────────────────┐
│ / Filter:                                                          │
├────────────────────────────────────────────────────────────────────┤
│ ● Alice Zhang    Principal Eng   Google         [info] 14d ago     │
│ ● Bob Martinez   SWE II          Stripe                            │
│ ○ Carol Kim      PM              (fmr: Airbnb, 2yr ago)            │
│ · Dave Park      Unknown         (heuristic via Stripe)            │
└────────────────────────────────────────────────────────────────────┘
 n=warm paths  e=edit  d=delete  i=import CSV  q=back
```

The browser holds a `Vec<ProfileContact>` and a `String` filter buffer. Filtering is done in-memory using case-insensitive substring match on name + company fields — no async query needed for <10,000 contacts.

Keybindings:
- `/` → activate filter input mode
- `i` → open LinkedIn CSV path prompt, then call `ContactService::import_linkedin_csv`
- `e` → open `ContactEntryForm` overlay with the selected contact
- `d` → delete confirmation dialog
- `Enter`/`n` → open `WarmPathsPanel` for the contact's current company

#### Step 5.3 — Contact Entry Form (`lazyjob-tui/src/views/contacts/entry_form.rs`)

A modal overlay for manual contact entry. Fields:
- First Name, Last Name (required)
- Email (optional, used as dedup key on import)
- Current Company, Current Title
- Relationship Notes (multiline, up to 5 lines)

Uses `ratatui::widgets::Clear` to erase background before rendering the overlay. Fields cycle with `Tab`/`Shift-Tab`. `Enter` on the last field submits.

### Phase 6 — Contact-Company Link Cache (Optional, Performance)

Once the contact store exceeds ~5,000 contacts, the query-time scan of all contacts against all companies becomes noticeable (O(N × M)). Phase 6 builds a materialized `contact_company_links` table that is rebuilt on each import and on demand.

```rust
// In ContactService
pub async fn rebuild_company_links(&self) -> Result<usize> {
    let contacts = self.contact_repo.list_contacts().await?;
    let companies = self.company_repo.list_all().await?;
    let company_names: Vec<(Uuid, String)> = companies.iter()
        .map(|c| (c.id, normalize_company_name(&c.name)))
        .collect();

    let mut links_written = 0usize;
    for contact in &contacts {
        for (company_id, norm_name) in &company_names {
            if let Some(current) = &contact.current_company_name {
                let norm_current = normalize_company_name(current);
                if company_names_match(&norm_current, norm_name) {
                    self.write_link(contact.id.clone(), *company_id, "current_employee", None).await?;
                    links_written += 1;
                }
            }
            for prev in &contact.previous_companies {
                let norm_prev = normalize_company_name(&prev.company_name);
                if company_names_match(&norm_prev, norm_name) {
                    // compute months_since_departure from end_year
                    // write link of appropriate alumni type
                    links_written += 1;
                }
            }
        }
    }
    Ok(links_written)
}
```

This cache is invalidated (truncated) at the start of each import, rebuilt after. `warm_paths_for_job` switches to a `JOIN`-based query against `contact_company_links` instead of scanning `profile_contacts`.

## Key Crate APIs

| Operation | Crate | API |
|-----------|-------|-----|
| Parse LinkedIn CSV (sync) | `csv 1.3` | `csv::Reader::from_path(path)`, `reader.headers()`, `reader.records()` |
| Offload sync CSV parsing | `tokio` | `tokio::task::spawn_blocking(|| { ... }).await` |
| Company name fuzzy match | `strsim 0.11` | `strsim::jaro_winkler(a, b) -> f64` |
| Regex compile once | `once_cell` | `once_cell::sync::Lazy<Regex>` |
| SQLite upsert | `sqlx` | `sqlx::query!("INSERT ... ON CONFLICT DO UPDATE").execute(&pool).await` |
| JSON column encode | `serde_json` | `serde_json::to_string(&vec)?` / `serde_json::from_str::<Vec<T>>(s)?` |
| Broadcast events | `tokio` | `tokio::sync::broadcast::channel(64)`, `tx.send(event)`, `rx.recv().await` |
| TUI list widget | `ratatui` | `List::new(items)`, `frame.render_stateful_widget(list, area, &mut state)` |
| TUI overlay erase | `ratatui` | `frame.render_widget(ratatui::widgets::Clear, area)` then render the form |
| Date arithmetic | `chrono` | `NaiveDate::from_ymd_opt(y, m, d)`, `(today - departed).num_days()` |

## Error Handling

```rust
// lazyjob-core/src/networking/error.rs

use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum NetworkingError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("CSV import error: {0}")]
    Import(String),

    #[error("job not found: {0}")]
    JobNotFound(Uuid),

    #[error("company not found: {0}")]
    CompanyNotFound(Uuid),

    #[error("contact not found: {0}")]
    ContactNotFound(String),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, NetworkingError>;
```

Each variant is actionable: `Database` → TUI shows "database error", `Import` → TUI shows per-row parse errors inline, `JobNotFound`/`CompanyNotFound` → silent empty list (warm paths are best-effort).

## Testing Strategy

### Unit Tests

**`normalize.rs`** — table-driven test of `normalize_company_name` and `company_names_match`:
```rust
#[test]
fn normalize_cases() {
    let cases = [
        ("Stripe, Inc.", "stripe"),
        ("DeepMind Technologies Ltd", "deepmind"),
        ("Apple", "apple"),
        ("Y Combinator", "y combinator"),
    ];
    for (input, expected) in cases {
        assert_eq!(normalize_company_name(input), expected, "input: {input}");
    }
}
```

**`connection_mapper.rs`** — pure function unit tests using mock repos:
- `compute_current_tier` with `departed = None` → `FirstDegreeCurrentEmployee`
- `compute_current_tier` with `departed = 18 months ago` → `FirstDegreeRecentAlumni { months_since_departure: 18 }`
- `compute_current_tier` with `departed = 48 months ago` → `FirstDegreeAlumni { months_since_departure: 48 }`
- `compute_current_tier` with `departed = 72 months ago` → `FirstDegreeDistantAlumni { months_since_departure: 72 }`
- `classify_approach` with `FirstDegreeCurrentEmployee` + `last_contacted 45 days ago` → `RequestReferral`
- `classify_approach` with `FirstDegreeCurrentEmployee` + `last_contacted 120 days ago` → `ReconnectFirst`
- `classify_approach` with `FirstDegreeCurrentEmployee` + `last_contacted = None` → `ReconnectFirst`
- `classify_approach` with `FirstDegreeRecentAlumni` → always `InformationalInterview`
- `classify_approach` with `Cold` → always `ColdOutreach`

Mock repo: `MockContactRepository` using `std::collections::HashMap<ContactId, ProfileContact>` — no async I/O needed for pure function tests.

**`csv_import.rs`** — file-based fixture tests:
- Happy path: 3-row CSV → `ImportResult { inserted: 3, updated: 0, skipped_duplicates: 0 }`
- Column-order variation: shuffle headers → same result
- Missing optional columns (`Email Address` absent) → contacts inserted with `email: None`
- Malformed row (fewer fields than header) → captured in `parse_errors`, other rows succeed
- Duplicate email (second import of same file) → `ImportResult { updated: 3 }`

### Integration Tests

`#[sqlx::test(migrations = "migrations")]` annotation auto-creates an in-memory SQLite with all migrations applied.

```rust
#[sqlx::test(migrations = "migrations")]
async fn warm_paths_finds_current_employee(pool: SqlitePool) {
    let contact_repo = Arc::new(SqliteContactRepository::new(pool.clone()));
    let company_repo = Arc::new(SqliteCompanyRepository::new(pool.clone()));
    let job_repo = Arc::new(SqliteJobRepository::new(pool.clone()));
    let mapper = ConnectionMapper::new(contact_repo.clone(), company_repo.clone(), job_repo.clone());

    // Arrange
    let company_id = company_repo.upsert(company_fixture("Stripe")).await.unwrap();
    let job_id = job_repo.upsert(job_fixture(company_id)).await.unwrap();
    let contact = contact_fixture("Alice", "Zhang", Some("Stripe, Inc."), None); // currently at Stripe
    contact_repo.upsert_contact(contact).await.unwrap();

    // Act
    let paths = mapper.warm_paths_for_job(job_id).await.unwrap();

    // Assert
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].contact_name, "Alice Zhang");
    assert!(matches!(paths[0].tier, ConnectionTier::FirstDegreeCurrentEmployee));
    assert_eq!(paths[0].suggested_approach, SuggestedApproach::RequestReferral);
}

#[sqlx::test(migrations = "migrations")]
async fn warm_paths_finds_alumni_by_previous_company(pool: SqlitePool) {
    // ... contact with previous_companies=[{company: "Google", end_year: 2023}]
    // job at Google → assert tier = FirstDegreeRecentAlumni
}

#[sqlx::test(migrations = "migrations")]
async fn warm_paths_deduplicates_current_and_alumni(pool: SqlitePool) {
    // Contact whose current company IS the target company but also has it in previous_companies
    // → should only appear once, at CurrentEmployee tier
}
```

### TUI Tests

```rust
#[test]
fn warm_paths_panel_renders_tiers() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut panel = WarmPathsPanel::new();
    panel.set_paths(vec![mock_warm_path_current_employee(), mock_warm_path_alumni()]);
    terminal.draw(|f| panel.render(f, f.size())).unwrap();
    let buf = terminal.backend().buffer().clone();
    // assert "●" appears for current employee, "◉" for recent alumni
    let content = buf_to_string(&buf);
    assert!(content.contains('●'));
    assert!(content.contains('['));
}
```

## Open Questions

1. **Name normalization collision rate**: `normalize_company_name("Apple Inc") == "apple"` and `normalize_company_name("Apple Records") == "apple"` would produce a false match. Phase 1 accepts this and lets the user dismiss irrelevant contacts from the warm paths list. Phase 2 solution: when `CompanyRecord.website` is set, require domain-level verification before matching contacts whose company URL doesn't match. Requires `company_urls` stored in the company record.

2. **LinkedIn CSV schema stability**: LinkedIn changes its CSV export format without notice. The parser is column-name-based (not position-based) to tolerate column reordering. Unknown columns are silently ignored. An explicit warning is logged (via `tracing::warn!`) when an expected column is absent, so users can report schema changes.

3. **Second-degree heuristic**: The shared-former-employer second-degree heuristic is **disabled by default** in Phase 1 (not yet implemented). The `SecondDegreeHeuristic` tier variant is defined in code but `ConnectionMapper` never produces it until Phase 6. This avoids showing noisy, low-confidence results until the confidence UI (badge with "?" tooltip) is built.

4. **School-based matching**: `SharedAlumni` tier requires knowing the user's own schools (from the life sheet) and cross-referencing with contact schools. This depends on `LifeSheet::education` being populated. Phase 1 skips this tier. Phase 4 adds it once the life-sheet education import is stable.

5. **Contact data privacy**: Importing a LinkedIn CSV means importing third-party data (your contacts' professional info). LazyJob's privacy spec requires a one-time in-app consent notice before the first import. The `ContactService::import_linkedin_csv` method should check a `settings.networking.linkedin_import_consent_given` flag and return `NetworkingError::ConsentRequired` if not set, prompting the TUI to show a privacy notice. This is not implemented in Phase 1 but is an explicit prerequisite for any production release.

6. **UTF-8 in LinkedIn CSV**: LinkedIn CSVs sometimes use Windows-1252 encoding for non-ASCII names. The `csv` crate is UTF-8-only. Phase 1 uses `csv::Reader` and tolerates parse errors for non-UTF-8 rows (they end up in `parse_errors`). Phase 3 can add `encoding_rs` for automatic charset detection.

## Related Specs
- [specs/networking-connection-mapping.md](networking-connection-mapping.md) — source spec
- [specs/networking-outreach-drafting.md](networking-outreach-drafting.md) — triggered from `WarmPathsPanel` on Enter
- [specs/networking-referral-management.md](networking-referral-management.md) — referral lifecycle built on `WarmPath.contact_id`
- [specs/profile-life-sheet-data-model.md](profile-life-sheet-data-model.md) — `profile_contacts` base table
- [specs/job-search-company-research.md](job-search-company-research.md) — `CompanyRepository`, `normalize_company_name()`
- [specs/16-privacy-security.md](16-privacy-security.md) — data consent requirements for contact import
