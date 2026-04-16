# Implementation Plan: Agentic Networking & Referrals

## Status
Draft

## Related Spec
[specs/networking-referrals-agentic.md](networking-referrals-agentic.md)

## Overview

The agentic networking module gives LazyJob an autonomous research layer that makes the
networking flywheel tractable for users with thin professional networks. The core design
principle, enforced throughout, is **agent suggests — human approves — human sends**. The agent
never sends messages, creates accounts, or takes actions visible to third parties without
explicit user confirmation at each step.

Three Ralph background loops drive the feature: `WarmPathFinderLoop` maps a user's existing
contacts to target companies and scores warm introduction paths; `OutreachBriefLoop` generates a
one-page context brief before an informational interview; and `RelationshipHealthLoop` (daily)
scans all active contacts for degraded relationship health and surfaces nurture suggestions.
Each loop reads from and writes to `lazyjob-core` repositories — no direct LLM calls from `lazyjob-ralph`.

The module extends existing `profile_contacts`, `referral_asks`, and `companies` tables with
lightweight additions (warm-path scoring columns, alumni associations, community memberships)
rather than new top-level tables, keeping the schema footprint minimal. All sensitive enrichment
data (email addresses found via Hunter.io, enriched profile data) is stored only locally in
SQLite and is never sent to any external service beyond the API call that retrieved it.

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — `run_migrations`, connection pool
- `specs/networking-connection-mapping-implementation-plan.md` — `ProfileContact`, `ContactRepository`, `ConnectionTier`, `normalize_company_name()`, `profile_contacts` table
- `specs/networking-referral-management-implementation-plan.md` — `ReferralAsk`, `RelationshipStage`, `RelationshipService`, `NetworkingReminderPoller`
- `specs/networking-outreach-drafting-implementation-plan.md` — `OutreachDraft`, `OutreachRepository`, `OutreachFabricationChecker`
- `specs/job-search-company-research-implementation-plan.md` — `CompanyRecord`, `CompanyRepository`, `CompanyService`
- `specs/job-search-ghost-job-detection-implementation-plan.md` — `GhostDetector`, `ghost_score` on `JobRecord`
- `specs/agentic-ralph-orchestration-implementation-plan.md` — `LoopType`, `LoopManager`, `WorkerEvent`
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — `NdjsonCodec`, `RalphProcessManager`
- `specs/17-ralph-prompt-templates-implementation-plan.md` — `TemplateRegistry`, `SimpleTemplateEngine`
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI panel/overlay system

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# New in this module:
csv            = "1.3"       # alumni CSV import (already in networking-connection-mapping)
reqwest        = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
scraper        = "0.21"      # HTML parsing for public LinkedIn profile pages (existing)
secrecy        = "0.8"       # Secret<String> wrapping for any API keys (existing)
# All other deps (uuid, chrono, serde, serde_json, sqlx, thiserror, anyhow, tokio,
# async-trait, once_cell, tracing, strsim) are already declared from prior modules.
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `WarmPath`, `WarmPathScore`, `AlumniAssociation`, `CommunityMembership`, `InformationalBrief`, `NurtureAction` | `lazyjob-core` | `src/networking/agentic/types.rs` |
| `WarmPathFinder` (warm-path scoring engine) | `lazyjob-core` | `src/networking/agentic/warm_path.rs` |
| `AlumniRepository` | `lazyjob-core` | `src/networking/agentic/alumni.rs` |
| `CommunityRepository` | `lazyjob-core` | `src/networking/agentic/community.rs` |
| `InformationalBriefService` | `lazyjob-core` | `src/networking/agentic/brief.rs` |
| `RelationshipHealthScorer` | `lazyjob-core` | `src/networking/agentic/health.rs` |
| `AgenticNetworkingService` (top-level orchestrator) | `lazyjob-core` | `src/networking/agentic/service.rs` |
| SQLite migrations (018) | `lazyjob-core` | `migrations/018_agentic_networking.sql` |
| `WarmPathFinderLoop` Ralph worker | `lazyjob-ralph` | `src/loops/warm_path_finder.rs` |
| `OutreachBriefLoop` Ralph worker | `lazyjob-ralph` | `src/loops/outreach_brief.rs` |
| `RelationshipHealthLoop` Ralph worker | `lazyjob-ralph` | `src/loops/relationship_health.rs` |
| Prompt templates | `lazyjob-ralph` | `prompts/warm_path_finder.toml`, `prompts/outreach_brief.toml`, `prompts/relationship_health.toml` |
| TUI networking agentic view | `lazyjob-tui` | `src/views/networking/agentic.rs` |
| TUI informational brief overlay | `lazyjob-tui` | `src/views/networking/brief_overlay.rs` |
| Module re-export facade | `lazyjob-core` | `src/networking/agentic/mod.rs` |

### Core Types

```rust
// lazyjob-core/src/networking/agentic/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::JobId;
use crate::networking::{ContactId, CompanyId};

// ── Newtype IDs ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct WarmPathId(pub Uuid);
impl WarmPathId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct AlumniAssociationId(pub Uuid);
impl AlumniAssociationId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct CommunityMembershipId(pub Uuid);
impl CommunityMembershipId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct InformationalBriefId(pub Uuid);
impl InformationalBriefId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

// ── Warm-path types ──────────────────────────────────────────────────────────

/// A ranked candidate path to a target company via an existing contact.
/// Computed at query time from contact + company + relationship data;
/// persisted in `warm_paths` table with a TTL to allow stale-marking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmPath {
    pub id: WarmPathId,
    pub job_id: Option<JobId>,         // None = company-level path, not job-specific
    pub company_id: CompanyId,
    pub contact_id: ContactId,
    pub path_type: WarmPathType,
    pub score: WarmPathScore,          // 0..=100
    pub explanation: String,           // human-readable: "Former colleague at Acme (2021-2023)"
    pub suggested_action: SuggestedAction,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub computed_at: DateTime<Utc>,
}

/// Categorical type encoding the nature of the warm connection.
/// Drives `WarmPathScore` calculation weights.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum WarmPathType {
    DirectColleague,        // worked together (same employer, overlapping tenure)
    SharedAlumni,           // same university
    SharedCommunity,        // same Slack/Discord/conference community
    MutualConnection,       // contact knows someone who works at company (2nd degree)
    ColdTargeted,           // no shared context — targeted cold with company research
}

/// Opaque score 0..=100. Higher = warmer path. Not stored as f32 to avoid float
/// comparison bugs when used as SQLite query threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct WarmPathScore(pub u8);

impl WarmPathScore {
    pub fn compute(
        path_type: WarmPathType,
        relationship_stage_score: u8,   // 0..=5 from RelationshipStage::score()
        days_since_last_contact: u32,
        mutual_connections_count: u8,
    ) -> Self {
        let base: u8 = match path_type {
            WarmPathType::DirectColleague   => 70,
            WarmPathType::SharedAlumni      => 50,
            WarmPathType::SharedCommunity   => 35,
            WarmPathType::MutualConnection  => 25,
            WarmPathType::ColdTargeted      => 10,
        };
        // Relationship depth bonus: up to +20 points
        let depth_bonus = relationship_stage_score.saturating_mul(4).min(20);
        // Recency penalty: -1 per 30 days, max -15
        let recency_penalty = (days_since_last_contact / 30).min(15) as u8;
        // Mutual connections bonus: +2 per mutual, max +10
        let mutual_bonus = mutual_connections_count.saturating_mul(2).min(10);

        Self(
            base
                .saturating_add(depth_bonus)
                .saturating_add(mutual_bonus)
                .saturating_sub(recency_penalty)
                .min(100)
        )
    }
}

/// Action the user should take to advance this warm path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestedAction {
    DraftConnectionRequest { reason: String },
    DraftInformationalRequest { reason: String },
    DraftFollowUp { last_contact_days_ago: u32 },
    DraftReferralAsk { job_title: String },
    NoActionNeeded,
}

// ── Alumni / community associations ─────────────────────────────────────────

/// Represents a shared educational institution between the user and a contact.
/// Derived from LifeSheet + contact import data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlumniAssociation {
    pub id: AlumniAssociationId,
    pub contact_id: ContactId,
    pub institution_normalized: String,  // normalized school name
    pub graduation_year_user: Option<u16>,
    pub graduation_year_contact: Option<u16>,
    pub created_at: DateTime<Utc>,
}

/// Shared Slack, Discord, or professional community membership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMembership {
    pub id: CommunityMembershipId,
    pub contact_id: ContactId,
    pub community_name: String,
    pub community_type: CommunityType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum CommunityType { Slack, Discord, MeetupGroup, Conference, Other }

// ── Informational brief ──────────────────────────────────────────────────────

/// One-page brief generated by `OutreachBriefLoop` before an informational
/// interview with a specific contact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InformationalBrief {
    pub id: InformationalBriefId,
    pub contact_id: ContactId,
    pub company_id: Option<CompanyId>,
    /// Markdown-formatted brief: background, recent activity, suggested questions.
    pub content_md: String,
    /// Source signals used: company news, contact's LinkedIn headline, shared context.
    pub signals_used: Vec<BriefSignal>,
    /// Ghost job warning if the target company has a high ghost score on target roles.
    pub ghost_warning: Option<String>,
    pub generated_at: DateTime<Utc>,
    /// TTL: re-generate if older than 7 days.
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BriefSignal {
    CompanyNews { headline: String, date: String },
    ContactHeadline { text: String },
    SharedEmployer { company: String, years: String },
    SharedSchool { institution: String },
    RecentJobPostings { count: u32 },
}

// ── Relationship health ──────────────────────────────────────────────────────

/// Scored health of a relationship, computed daily by `RelationshipHealthLoop`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipHealth {
    pub contact_id: ContactId,
    pub score: RelationshipHealthScore,   // 0..=100
    pub last_interaction_days_ago: u32,
    pub interaction_count_90d: u32,
    pub nurture_actions: Vec<NurtureAction>,
    pub scored_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RelationshipHealthScore(pub u8);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NurtureAction {
    SendCheckIn { draft_hint: String },
    ShareContent { topic_hint: String },
    ScheduleCall { urgency: NurtureUrgency },
    NoActionNeeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NurtureUrgency { High, Medium, Low }
```

### Trait Definitions

```rust
// lazyjob-core/src/networking/agentic/warm_path.rs

use async_trait::async_trait;
use crate::networking::agentic::types::*;
use crate::discovery::JobId;
use crate::networking::CompanyId;

#[async_trait]
pub trait WarmPathRepository: Send + Sync {
    /// Upsert a warm path (keyed on contact_id + company_id + path_type).
    async fn upsert_warm_path(&self, path: &WarmPath) -> Result<(), WarmPathError>;
    /// Fetch all warm paths for a job, ordered by score descending.
    async fn list_for_job(&self, job_id: JobId) -> Result<Vec<WarmPath>, WarmPathError>;
    /// Fetch top N undismissed warm paths across all jobs, ordered by score desc.
    async fn list_top(&self, limit: u32) -> Result<Vec<WarmPath>, WarmPathError>;
    /// Mark a path dismissed (user explicitly dismissed).
    async fn dismiss(&self, id: WarmPathId) -> Result<(), WarmPathError>;
    /// Delete stale paths computed before `before`.
    async fn delete_stale(&self, before: DateTime<Utc>) -> Result<u64, WarmPathError>;
}

/// Pure sync scoring engine — no I/O. Call from within Ralph loop after loading data.
pub struct WarmPathFinder;

impl WarmPathFinder {
    /// Given a list of contacts and a company, compute all viable warm paths.
    /// Returns paths sorted by score descending.
    pub fn find_paths(
        contacts: &[ContactWithContext],   // contact + employer history + alumni
        company: &CompanyRecord,
        user_profile: &LifeSheetSummary,
        active_jobs: &[JobRecord],         // for job-level path generation
    ) -> Vec<WarmPath> { ... }

    /// Classify path type between user profile and contact.
    fn classify_path_type(
        user_employers: &[String],
        user_schools: &[String],
        user_communities: &[String],
        contact: &ContactWithContext,
        company: &CompanyRecord,
    ) -> Option<WarmPathType> { ... }
}

/// Enriched contact for path scoring — combines profile_contacts row
/// with alumni and community membership data.
pub struct ContactWithContext {
    pub contact: ProfileContact,
    pub previous_companies: Vec<String>,    // normalized employer names from import
    pub schools: Vec<String>,               // from alumni_associations table
    pub communities: Vec<String>,           // from community_memberships table
    pub days_since_last_contact: u32,
    pub relationship_stage_score: u8,       // from RelationshipStage::score()
    pub mutual_connections_count: u8,
}
```

```rust
// lazyjob-core/src/networking/agentic/brief.rs

use async_trait::async_trait;

#[async_trait]
pub trait InformationalBriefRepository: Send + Sync {
    async fn save_brief(&self, brief: &InformationalBrief) -> Result<(), AgenticNetworkingError>;
    async fn get_for_contact(&self, contact_id: ContactId) -> Result<Option<InformationalBrief>, AgenticNetworkingError>;
    async fn delete_expired(&self) -> Result<u64, AgenticNetworkingError>;
}

/// Orchestrates brief generation for a specific contact before a planned call.
pub struct InformationalBriefService {
    repo: Arc<dyn InformationalBriefRepository>,
    company_repo: Arc<dyn CompanyRepository>,
    brief_ttl_days: u32,   // default 7
}

impl InformationalBriefService {
    /// Returns an unexpired cached brief, or generates a new one via the Ralph loop.
    /// Callers are responsible for spawning the loop — this method checks the cache only.
    pub async fn get_or_schedule(
        &self,
        contact_id: ContactId,
        loop_manager: &LoopManager,
    ) -> Result<BriefStatus, AgenticNetworkingError> { ... }
}

pub enum BriefStatus {
    Ready(InformationalBrief),
    Generating { loop_id: LoopInstanceId },
    NoBriefNeeded,
}
```

```rust
// lazyjob-core/src/networking/agentic/health.rs

/// Pure sync health scorer — call from background tokio task.
pub struct RelationshipHealthScorer;

impl RelationshipHealthScorer {
    pub fn score(
        contact: &ProfileContact,
        interaction_count_90d: u32,
        last_interaction_days_ago: u32,
        relationship_stage: RelationshipStage,
    ) -> RelationshipHealth {
        let base = Self::base_score(last_interaction_days_ago, relationship_stage);
        let frequency_bonus = Self::frequency_bonus(interaction_count_90d);
        let raw = (base + frequency_bonus).min(100);
        let score = RelationshipHealthScore(raw);
        let nurture_actions = Self::compute_nurture_actions(
            score, last_interaction_days_ago, contact
        );
        RelationshipHealth {
            contact_id: contact.id.clone(),
            score,
            last_interaction_days_ago,
            interaction_count_90d,
            nurture_actions,
            scored_at: Utc::now(),
        }
    }

    fn base_score(days_since: u32, stage: RelationshipStage) -> u8 {
        // Floor: stage minimum. Decay: -2 per 30 days of inactivity.
        let stage_floor: u8 = match stage {
            RelationshipStage::Unknown   => 10,
            RelationshipStage::Identified=> 20,
            RelationshipStage::Contacted => 35,
            RelationshipStage::Replied   => 55,
            RelationshipStage::Warmed    => 70,
            RelationshipStage::Referred  => 85,
            RelationshipStage::Closed    => 40,
        };
        let decay = (days_since / 30).min(30) as u8 * 2;
        stage_floor.saturating_sub(decay)
    }

    fn frequency_bonus(count_90d: u32) -> u8 {
        // +5 per interaction in last 90 days, cap at +20
        (count_90d as u8).saturating_mul(5).min(20)
    }

    fn compute_nurture_actions(
        score: RelationshipHealthScore,
        days: u32,
        contact: &ProfileContact,
    ) -> Vec<NurtureAction> {
        let mut actions = Vec::new();
        if score.0 < 30 && days > 60 {
            actions.push(NurtureAction::SendCheckIn {
                draft_hint: format!("Re-engage with {} after {} days", contact.full_name, days),
            });
        } else if score.0 < 50 && days > 30 {
            actions.push(NurtureAction::ShareContent {
                topic_hint: contact.headline.clone().unwrap_or_default(),
            });
        }
        actions
    }
}
```

### SQLite Schema

```sql
-- migrations/018_agentic_networking.sql

-- Warm paths cache
CREATE TABLE IF NOT EXISTS warm_paths (
    id                  TEXT PRIMARY KEY,
    job_id              TEXT REFERENCES jobs(id) ON DELETE CASCADE,
    company_id          TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    contact_id          TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    path_type           TEXT NOT NULL CHECK (path_type IN (
                            'direct_colleague','shared_alumni','shared_community',
                            'mutual_connection','cold_targeted')),
    score               INTEGER NOT NULL CHECK (score BETWEEN 0 AND 100),
    explanation         TEXT NOT NULL,
    suggested_action    TEXT NOT NULL,   -- JSON blob of SuggestedAction
    dismissed_at        TEXT,
    computed_at         TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS uidx_warm_paths_contact_company_type
    ON warm_paths(contact_id, company_id, path_type);

CREATE INDEX IF NOT EXISTS idx_warm_paths_job_score
    ON warm_paths(job_id, score DESC) WHERE dismissed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_warm_paths_score
    ON warm_paths(score DESC) WHERE dismissed_at IS NULL AND job_id IS NULL;

-- Alumni associations (user ↔ contact shared school)
CREATE TABLE IF NOT EXISTS alumni_associations (
    id                          TEXT PRIMARY KEY,
    contact_id                  TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    institution_normalized      TEXT NOT NULL,
    graduation_year_user        INTEGER,
    graduation_year_contact     INTEGER,
    created_at                  TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS uidx_alumni_contact_institution
    ON alumni_associations(contact_id, institution_normalized);

-- Community memberships (shared Slack/Discord/etc.)
CREATE TABLE IF NOT EXISTS community_memberships (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    community_name  TEXT NOT NULL,
    community_type  TEXT NOT NULL CHECK (community_type IN (
                        'slack','discord','meetup_group','conference','other')),
    created_at      TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS uidx_community_contact_name
    ON community_memberships(contact_id, community_name);

-- Informational briefs
CREATE TABLE IF NOT EXISTS informational_briefs (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    company_id      TEXT REFERENCES companies(id) ON DELETE SET NULL,
    content_md      TEXT NOT NULL,
    signals_used    TEXT NOT NULL,   -- JSON array of BriefSignal
    ghost_warning   TEXT,
    generated_at    TEXT NOT NULL,
    expires_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_briefs_contact_expires
    ON informational_briefs(contact_id, expires_at DESC);

-- Relationship health scores (one row per contact, refreshed daily)
CREATE TABLE IF NOT EXISTS relationship_health (
    contact_id              TEXT PRIMARY KEY REFERENCES profile_contacts(id) ON DELETE CASCADE,
    score                   INTEGER NOT NULL CHECK (score BETWEEN 0 AND 100),
    last_interaction_days_ago INTEGER NOT NULL,
    interaction_count_90d   INTEGER NOT NULL,
    nurture_actions         TEXT NOT NULL,  -- JSON array of NurtureAction
    scored_at               TEXT NOT NULL
);
```

### Module Structure

```
lazyjob-core/
  src/
    networking/
      agentic/
        mod.rs          # re-exports: WarmPath, WarmPathFinder, WarmPathScore, ...
        types.rs        # all domain types
        warm_path.rs    # WarmPathFinder + WarmPathRepository trait
        alumni.rs       # AlumniRepository trait + SqliteAlumniRepository
        community.rs    # CommunityRepository trait + SqliteCommunityRepository
        brief.rs        # InformationalBriefService + InformationalBriefRepository trait
        health.rs       # RelationshipHealthScorer (pure sync)
        service.rs      # AgenticNetworkingService (top-level orchestrator)
        error.rs        # AgenticNetworkingError

lazyjob-ralph/
  src/
    loops/
      warm_path_finder.rs   # WarmPathFinderLoop worker entry
      outreach_brief.rs     # OutreachBriefLoop worker entry
      relationship_health.rs# RelationshipHealthLoop worker entry
  prompts/
    warm_path_finder.toml
    outreach_brief.toml
    relationship_health.toml

lazyjob-tui/
  src/
    views/
      networking/
        agentic.rs          # AgenticNetworkingView (top-level warm-paths panel)
        brief_overlay.rs    # InformationalBriefOverlay
```

## Implementation Phases

### Phase 1 — Alumni & Community Association Data Model (MVP Foundation)

**Goal:** Populate the data model from existing import sources before any agentic loops run.

#### Step 1.1 — SQLite migration 018

File: `lazyjob-core/migrations/018_agentic_networking.sql`

Apply via `run_migrations` in the existing migration runner. No new crate dependencies.

Verification: `sqlx migrate run` succeeds; `sqlite3 lazyjob.db ".tables"` shows all four new tables.

#### Step 1.2 — Alumni association inference from LifeSheet

File: `lazyjob-core/src/networking/agentic/alumni.rs`

```rust
pub struct SqliteAlumniRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait]
impl AlumniRepository for SqliteAlumniRepository {
    async fn upsert_association(&self, assoc: &AlumniAssociation) -> Result<(), AgenticNetworkingError> {
        sqlx::query!(
            r#"INSERT INTO alumni_associations
               (id, contact_id, institution_normalized, graduation_year_user,
                graduation_year_contact, created_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(contact_id, institution_normalized) DO NOTHING"#,
            assoc.id.0.to_string(),
            assoc.contact_id.0.to_string(),
            assoc.institution_normalized,
            assoc.graduation_year_user.map(|y| y as i64),
            assoc.graduation_year_contact.map(|y| y as i64),
            assoc.created_at.to_rfc3339(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

`AlumniInferenceService::infer_from_import()` is called during the LinkedIn CSV import path in
`LinkedInCsvImporter`. It:
1. Loads all `education` rows from `life_sheet_education` for the user.
2. Normalizes institution names via `normalize_institution()` — two `once_cell::sync::Lazy<Regex>`
   patterns: strip suffixes like "University", "College", "Institute of Technology", then
   lowercase + collapse whitespace.
3. For each imported contact with a non-empty `education` field in the CSV:
   - Normalize the contact's school name.
   - Run `strsim::jaro_winkler` against all user schools.
   - Threshold ≥ 0.90: insert an `AlumniAssociation` row.

Key API: `strsim::jaro_winkler(user_school, contact_school)` → `f64`.

Verification: Unit test with 5 user schools and 10 contact rows — assert correct associations created
and no false positives for similar-but-different schools.

#### Step 1.3 — Community membership manual entry

File: `lazyjob-core/src/networking/agentic/community.rs`

Phase 1 only supports manual creation via TUI form (no scraping). `SqliteCommunityRepository`
provides `create`, `list_for_contact`, `delete`.

`CommunityMembershipService::add_community()` normalizes community name (lowercase + collapse
whitespace) before insert to prevent case-sensitive duplicates.

#### Step 1.4 — `ContactWithContext` loader

File: `lazyjob-core/src/networking/agentic/warm_path.rs`

```rust
pub async fn load_contact_with_context(
    pool: &sqlx::SqlitePool,
    contact: ProfileContact,
) -> Result<ContactWithContext, AgenticNetworkingError> {
    let previous_companies: Vec<String> = sqlx::query_scalar!(
        "SELECT company_normalized FROM contact_employment_history WHERE contact_id = ?",
        contact.id.0.to_string()
    ).fetch_all(pool).await?;

    let schools: Vec<String> = sqlx::query_scalar!(
        "SELECT institution_normalized FROM alumni_associations WHERE contact_id = ?",
        contact.id.0.to_string()
    ).fetch_all(pool).await?;

    let communities: Vec<String> = sqlx::query_scalar!(
        "SELECT community_name FROM community_memberships WHERE contact_id = ?",
        contact.id.0.to_string()
    ).fetch_all(pool).await?;

    let last_days: u32 = sqlx::query_scalar!(
        r#"SELECT CAST(julianday('now') - julianday(MAX(occurred_at)) AS INTEGER)
           FROM interaction_logs WHERE contact_id = ?"#,
        contact.id.0.to_string()
    ).fetch_one(pool).await?.unwrap_or(9999);

    let stage_score = contact.relationship_stage.score();
    let mutual_count: u8 = 0; // Phase 2: mutual connection graph

    Ok(ContactWithContext {
        contact,
        previous_companies,
        schools,
        communities,
        days_since_last_contact: last_days,
        relationship_stage_score: stage_score,
        mutual_connections_count: mutual_count,
    })
}
```

Verification: Integration test using `#[sqlx::test(migrations = "migrations")]` — seed one contact
with two alumni rows, verify `ContactWithContext.schools` contains both.

---

### Phase 2 — `WarmPathFinder` Scoring Engine

**Goal:** Implement the pure scoring logic and `WarmPathFinderLoop` Ralph worker.

#### Step 2.1 — `WarmPathFinder::find_paths()`

File: `lazyjob-core/src/networking/agentic/warm_path.rs`

Sorting key: `WarmPathScore` descending. Only include paths with `score > 10` to filter noise.

`classify_path_type()` checks in priority order:
1. Direct colleague: `strsim::jaro_winkler(user_employer, contact_employer) >= 0.92` AND
   employment date ranges overlap by ≥1 day (if both available) OR same company with no
   date disambiguation possible.
2. Shared alumni: `contact.schools` ∩ `user_schools` non-empty (via jaro_winkler ≥ 0.90).
3. Shared community: `contact.communities` ∩ `user_communities` non-empty (exact match after
   normalization).
4. `MutualConnection`: contact works at company (their current employer == company.name_normalized
   via jaro_winkler ≥ 0.92) — this is a first-degree connection TO the company, not truly mutual.
   Mutual second-degree is Phase 4.
5. `ColdTargeted`: contact is at target company but no shared context.

Return `None` (no path) if contact does not work at target company AND no shared context exists.

#### Step 2.2 — `SqliteWarmPathRepository`

File: `lazyjob-core/src/networking/agentic/warm_path.rs`

`upsert_warm_path` uses `ON CONFLICT(contact_id, company_id, path_type) DO UPDATE SET score = excluded.score, explanation = excluded.explanation, suggested_action = excluded.suggested_action, computed_at = excluded.computed_at`.

`list_for_job` joins `warm_paths` → `jobs` via `company_id`, orders by `score DESC`, limit 20.

#### Step 2.3 — `WarmPathFinderLoop` Ralph worker

File: `lazyjob-ralph/src/loops/warm_path_finder.rs`

Loop type: `LoopType::WarmPathFinder`. Priority: medium. Concurrency limit: 1 (only one path
finder runs at a time to avoid DB lock contention on `warm_paths`).

Worker entry point:
```rust
pub async fn run_warm_path_finder(
    params: serde_json::Value,
    event_tx: tokio::sync::mpsc::Sender<WorkerEvent>,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    services: Arc<AppServices>,
) -> anyhow::Result<()> {
    let target_job_ids: Vec<String> = serde_json::from_value(
        params["target_job_ids"].clone()
    ).unwrap_or_default();

    event_tx.send(WorkerEvent::Progress { message: "Loading contacts...".into() }).await?;

    let contacts = services.contact_repo.list_all().await?;
    let mut paths_written = 0u32;

    for job_id_str in &target_job_ids {
        if *cancel_rx.borrow() { break; }

        let job = match services.job_repo.get(JobId(Uuid::parse_str(job_id_str)?)).await? {
            Some(j) => j,
            None => continue,
        };
        let company = match services.company_repo.get(job.company_id).await? {
            Some(c) => c,
            None => continue,
        };

        let mut contacts_with_ctx = Vec::new();
        for contact in &contacts {
            contacts_with_ctx.push(
                load_contact_with_context(&services.pool, contact.clone()).await?
            );
        }

        let user_profile = services.life_sheet_repo.load_summary().await?;
        let paths = WarmPathFinder::find_paths(
            &contacts_with_ctx, &company, &user_profile, &[job.clone()]
        );

        for path in &paths {
            services.warm_path_repo.upsert_warm_path(path).await?;
            paths_written += 1;
        }

        event_tx.send(WorkerEvent::Progress {
            message: format!("Scored {} paths for {}", paths.len(), company.name),
        }).await?;
    }

    // Stale cleanup: delete paths older than 7 days
    let cutoff = Utc::now() - chrono::Duration::days(7);
    services.warm_path_repo.delete_stale(cutoff).await?;

    event_tx.send(WorkerEvent::Complete {
        output: serde_json::json!({ "paths_written": paths_written }),
    }).await?;
    Ok(())
}
```

Triggered automatically by `LoopDispatch::dispatch_suggestion()` in response to
`PostTransitionSuggestion::FindWarmPaths { job_id }` when the user moves a job to `Applied` or
`Shortlisted`. Also triggered manually from the TUI (`w` key in jobs list).

Verification: Integration test — seed 3 contacts (1 former colleague, 1 alumnus, 1 unrelated),
run loop for one job, assert correct path types and score ordering.

---

### Phase 3 — `OutreachBriefLoop` (Informational Interview Brief)

**Goal:** Generate a one-page contextual brief before a planned informational interview.

#### Step 3.1 — Brief content assembly

File: `lazyjob-core/src/networking/agentic/brief.rs`

`BriefContextBuilder::build()` (pure sync, no LLM) assembles:
- `contact.headline`, `contact.company`, `contact.location` from `profile_contacts`
- Shared context from `AlumniAssociation` + employment history
- Company news (reuses `CompanyRecord.recent_news_json` populated by `CompanyResearchLoop`)
- Ghost warning: if company has active jobs with `ghost_score > 0.5`, include advisory text
- Recent interaction log (last 3 entries from `interaction_logs`)

Returns `BriefContext` struct used by the prompt template.

#### Step 3.2 — TOML prompt template

File: `lazyjob-ralph/prompts/outreach_brief.toml`

```toml
[template]
loop_type = "OutreachBrief"
version = "1.0.0"

[system]
content = """
You are a networking coach helping a job seeker prepare for an informational interview.
Generate a concise one-page preparation brief in Markdown. The brief must:
- Use ONLY facts from the provided context — do NOT invent background, credentials, or claims
- Be genuinely useful for a 30-minute conversation
- Flag any topics to avoid (recent layoffs, controversial company news)
- Suggest 5 specific questions based on the contact's role and recent activity
Format: ## Background | ## What They're Working On | ## Shared Context | ## Questions to Ask | ## Topics to Avoid
"""
cache_system_prompt = true

[user]
content = """
Contact: {contact_name}, {contact_headline} at {contact_company}
Location: {contact_location}
Shared context: {shared_context}
Company news: {company_news}
Recent interactions: {recent_interactions}
Ghost warning: {ghost_warning}
"""
```

#### Step 3.3 — `OutreachBriefLoop` worker

File: `lazyjob-ralph/src/loops/outreach_brief.rs`

Params: `{ contact_id: String }`. Loop type: `LoopType::OutreachBrief`. Priority: high (user-
initiated immediately before a scheduled call).

Sequence:
1. Load `ProfileContact` by `contact_id`.
2. Call `BriefContextBuilder::build()`.
3. Render prompt via `TemplateRegistry::render(LoopType::OutreachBrief, &vars)`.
4. Call `LlmProvider::complete()` (not streaming — brief is short enough).
5. Parse response as Markdown text (no JSON schema required).
6. Persist `InformationalBrief` via `InformationalBriefRepository::save_brief()`.
7. Emit `WorkerEvent::Complete { output: brief_id }`.

On LLM failure: emit `WorkerEvent::Error` — do NOT persist a partial brief. The TUI shows an error
overlay; user can retry.

Expiry: `expires_at = generated_at + 7 days`. `InformationalBriefService::get_or_schedule()`
checks expiry before returning a cached brief.

Verification: Mock `LlmProvider` returning a fixture markdown string. Assert brief is persisted,
`signals_used` contains at least one `BriefSignal::SharedEmployer`, brief expires in 7 days.

---

### Phase 4 — `RelationshipHealthLoop` (Daily Nurture Engine)

**Goal:** Score all active contacts daily and surface stale relationships.

#### Step 4.1 — `RelationshipHealthScorer` (pure sync)

File: `lazyjob-core/src/networking/agentic/health.rs`

Already specified in Core Types above. Unit tests for each scoring branch: score=100 for active
Warmed contact, score<30 for Identified contact with no interaction in 90 days.

#### Step 4.2 — `RelationshipHealthLoop` worker

File: `lazyjob-ralph/src/loops/relationship_health.rs`

Loop type: `LoopType::RelationshipHealth`. Scheduled daily at 06:00 local time by `LoopScheduler`
(cron expression `0 6 * * *`). Concurrency limit: 1.

```rust
pub async fn run_relationship_health(
    _params: serde_json::Value,
    event_tx: tokio::sync::mpsc::Sender<WorkerEvent>,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    services: Arc<AppServices>,
) -> anyhow::Result<()> {
    let contacts = services.contact_repo.list_all().await?;
    let mut scored = 0u32;
    let mut needs_nurture = 0u32;

    for contact in &contacts {
        if *cancel_rx.borrow() { break; }

        let count_90d: u32 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) FROM interaction_logs
               WHERE contact_id = ? AND occurred_at > datetime('now', '-90 days')"#,
            contact.id.0.to_string()
        ).fetch_one(&services.pool).await? as u32;

        let last_days: u32 = sqlx::query_scalar!(
            r#"SELECT CAST(julianday('now') - julianday(MAX(occurred_at)) AS INTEGER)
               FROM interaction_logs WHERE contact_id = ?"#,
            contact.id.0.to_string()
        ).fetch_one(&services.pool).await?.unwrap_or(9999);

        let health = RelationshipHealthScorer::score(
            contact,
            count_90d,
            last_days,
            contact.relationship_stage.clone(),
        );

        // Upsert into relationship_health table
        sqlx::query!(
            r#"INSERT INTO relationship_health
               (contact_id, score, last_interaction_days_ago, interaction_count_90d,
                nurture_actions, scored_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(contact_id) DO UPDATE SET
                 score = excluded.score,
                 last_interaction_days_ago = excluded.last_interaction_days_ago,
                 interaction_count_90d = excluded.interaction_count_90d,
                 nurture_actions = excluded.nurture_actions,
                 scored_at = excluded.scored_at"#,
            contact.id.0.to_string(),
            health.score.0 as i64,
            health.last_interaction_days_ago as i64,
            health.interaction_count_90d as i64,
            serde_json::to_string(&health.nurture_actions)?,
            health.scored_at.to_rfc3339(),
        ).execute(&services.pool).await?;

        scored += 1;
        if !health.nurture_actions.is_empty() &&
            !matches!(health.nurture_actions[0], NurtureAction::NoActionNeeded) {
            needs_nurture += 1;
        }
    }

    // Prune relationship_health rows for deleted contacts (handled by ON DELETE CASCADE)

    event_tx.send(WorkerEvent::Complete {
        output: serde_json::json!({
            "scored": scored,
            "needs_nurture": needs_nurture,
        }),
    }).await?;
    Ok(())
}
```

LLM prompt template (`prompts/relationship_health.toml`) is used only for Phase 5 "nurture content
suggestions" — Phase 4 implementation is LLM-free (pure heuristics).

Verification: Seed 10 contacts with varying `interaction_logs` timestamps. Run loop. Assert all 10
rows exist in `relationship_health`, contacts with no interaction in 90 days have score < 50.

---

### Phase 5 — TUI Agentic Networking View

**Goal:** Surface warm paths and relationship health in the TUI without blocking the main event loop.

#### Step 5.1 — `AgenticNetworkingView`

File: `lazyjob-tui/src/views/networking/agentic.rs`

Layout: 50/50 vertical split.
- **Left panel**: "Warm Paths" — ranked list of `WarmPath` rows. Each row: contact name, company,
  path type badge (color-coded), score bar (ratatui `Gauge` widget or `Span` progress), suggested
  action hint.
- **Right panel**: "Relationship Health" — contacts sorted by `health.score` ascending (lowest
  first = most urgent). Each row: contact name, score badge, days since last contact, top nurture
  action.

Keybindings (Normal mode):
- `w` — trigger `WarmPathFinderLoop` for the focused job
- `b` — trigger `OutreachBriefLoop` for the focused contact
- `Enter` — open contact detail panel with full warm-path breakdown
- `d` — dismiss the focused warm path
- `Tab` — switch focus between left/right panels

State struct:
```rust
pub struct AgenticNetworkingView {
    warm_paths: Vec<WarmPath>,
    health_scores: Vec<RelationshipHealth>,
    warm_path_list_state: ListState,
    health_list_state: ListState,
    focus: AgenticNetworkingFocus,
    loop_status: HashMap<ContactId, LoopStatus>,   // for "Generating..." badge
}

enum AgenticNetworkingFocus { WarmPaths, RelationshipHealth }
```

Rendering uses `ratatui::widgets::List` with `ListState` for both panels. Warm path type badge is
a `Span` with style: `DirectColleague`=Green, `SharedAlumni`=Cyan, `SharedCommunity`=Yellow,
`MutualConnection`=Magenta, `ColdTargeted`=DarkGray.

#### Step 5.2 — `InformationalBriefOverlay`

File: `lazyjob-tui/src/views/networking/brief_overlay.rs`

Triggered by `b` key. Checks `InformationalBriefService::get_or_schedule()`:
- If `BriefStatus::Ready(brief)`: renders full Markdown as scrollable `Paragraph` in a
  `ratatui::widgets::Clear`-backed centered popup (80% width, 80% height).
- If `BriefStatus::Generating { loop_id }`: renders a "Generating brief..." spinner (standard
  `throbber` characters: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`).
- `Esc` or `q` closes the overlay.

Verification: Manual TUI test — seed one contact with an expired brief, press `b`, verify
spinner appears. After loop completes (mock via direct DB insert), re-press `b`, verify brief content
renders with scrolling.

---

### Phase 6 — Second-Degree Connection Graph (Post-MVP)

**Goal:** Extend `WarmPathType::MutualConnection` to actual second-degree paths.

> **Note:** Deferred to post-MVP. No LinkedIn API or browser automation required — the feature
> relies solely on data the user already has in their contact list.

Design sketch:

1. `contact_mutual_links` junction table: `(contact_a_id, contact_b_id, link_type TEXT)`. Populated
   by the user via a "link two contacts" TUI form — the user manually attests "Alice knows Bob".

2. `SecondDegreeFinder::find_mutual_paths()` queries:
   ```sql
   SELECT DISTINCT link.contact_b_id
   FROM contact_mutual_links link
   JOIN profile_contacts b ON link.contact_b_id = b.id
   WHERE link.contact_a_id IN (
     SELECT id FROM profile_contacts WHERE company_id = ?
   )
   ```
   This finds second-degree introductions: (user → A → B) where A works at the target company and
   the user knows B.

3. Path type `MutualConnection` score boosted by `+15` per known mutual (cap `+30`).

4. `SuggestedAction::DraftIntroductionRequest { via_contact: ContactId }` — draft a message to
   the mutual asking them to introduce you to the company contact.

Blocked on: user-facing "link contacts" TUI flow. No automated LinkedIn scraping. Never violates
LinkedIn ToS.

---

## Key Crate APIs

```
strsim::jaro_winkler(a: &str, b: &str) -> f64
  — Fuzzy school/employer name matching. Threshold 0.90 for alumni, 0.92 for employers.

once_cell::sync::Lazy<Regex>
  — Compile institution normalizer patterns once; reused across all contact imports.

sqlx::query!(r#"INSERT ... ON CONFLICT DO UPDATE SET ..."#).execute(&pool).await?
  — Upsert pattern for warm_paths, relationship_health.

sqlx::query_scalar!("SELECT COUNT(*) FROM ...").fetch_one(&pool).await?
  — Interaction count queries in RelationshipHealthLoop.

chrono::Duration::days(7)
  — Brief TTL calculation.
  
tokio::sync::watch::Receiver<bool>.borrow()
  — Cancel token check between contact iterations in the health loop.

serde_json::to_string(&nurture_actions)?
  — Serialize NurtureAction vec to TEXT for SQLite storage.

ratatui::widgets::{List, ListState, Paragraph, Clear, Gauge}
  — TUI panels, brief overlay, score visualization.
```

## Error Handling

```rust
// lazyjob-core/src/networking/agentic/error.rs

#[derive(thiserror::Error, Debug)]
pub enum AgenticNetworkingError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("contact not found: {0}")]
    ContactNotFound(String),

    #[error("company not found: {0}")]
    CompanyNotFound(String),

    #[error("life sheet not loaded — import profile before running warm path finder")]
    LifeSheetNotLoaded,

    #[error("brief generation loop failed: {0}")]
    BriefLoopFailed(String),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AgenticNetworkingError>;
```

`WarmPathFinderLoop` and `RelationshipHealthLoop` log individual contact errors via
`tracing::warn!` and continue processing — a single failed contact does not abort the loop.
`OutreachBriefLoop` propagates errors immediately (it's a single-contact operation; failure
is surfaced as a TUI error overlay).

## Testing Strategy

### Unit tests

File: `lazyjob-core/src/networking/agentic/warm_path.rs` (inline `#[cfg(test)]`)

```rust
#[test]
fn test_warm_path_score_direct_colleague_high_stage() {
    let score = WarmPathScore::compute(
        WarmPathType::DirectColleague,
        5,    // Referred stage
        10,   // 10 days since last contact
        2,    // 2 mutual connections
    );
    assert!(score.0 >= 85, "expected score ≥ 85, got {}", score.0);
}

#[test]
fn test_warm_path_score_cold_decays_with_time() {
    let fresh = WarmPathScore::compute(WarmPathType::ColdTargeted, 0, 0, 0);
    let stale = WarmPathScore::compute(WarmPathType::ColdTargeted, 0, 450, 0);
    assert!(stale.0 < fresh.0, "stale cold path should score lower");
    assert!(stale.0 <= 10, "stale cold path should cap at 10");
}

#[test]
fn test_relationship_health_scorer_warmed_active() {
    let contact = mock_contact(RelationshipStage::Warmed);
    let health = RelationshipHealthScorer::score(&contact, 5, 3, RelationshipStage::Warmed);
    assert!(health.score.0 >= 80, "warmed + active should be ≥ 80");
    assert!(matches!(health.nurture_actions[0], NurtureAction::NoActionNeeded));
}

#[test]
fn test_relationship_health_scorer_stale_identified() {
    let contact = mock_contact(RelationshipStage::Identified);
    let health = RelationshipHealthScorer::score(&contact, 0, 120, RelationshipStage::Identified);
    assert!(health.score.0 < 30, "stale identified should be < 30");
    assert!(matches!(health.nurture_actions[0], NurtureAction::SendCheckIn { .. }));
}
```

File: `lazyjob-core/src/networking/agentic/alumni.rs`

```rust
#[test]
fn test_normalize_institution_strips_suffix() {
    assert_eq!(normalize_institution("Stanford University"), "stanford");
    assert_eq!(normalize_institution("MIT"), "mit");
    assert_eq!(normalize_institution("Carnegie Mellon University"), "carnegie mellon");
}

#[test]
fn test_alumni_inference_fuzzy_match() {
    let user_schools = vec!["carnegie mellon".to_string()];
    let contact_school = "Carnegie Mellon Univ.";
    let matched = infer_alumni_match(&user_schools, contact_school);
    assert!(matched.is_some());
}
```

### Integration tests

File: `lazyjob-core/tests/warm_path_integration.rs`

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_warm_path_upsert_and_list(pool: sqlx::SqlitePool) {
    // seed one contact with company_id matching, one with no match
    // run WarmPathFinder::find_paths with mocked data
    // assert correct number of warm paths persisted
    // assert score ordering
    // assert path with DirectColleague type has higher score than ColdTargeted
}
```

File: `lazyjob-core/tests/relationship_health_integration.rs`

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_relationship_health_loop_full_sweep(pool: sqlx::SqlitePool) {
    // seed 5 contacts with varying interaction_logs
    // run run_relationship_health() directly
    // assert all 5 rows in relationship_health
    // assert contact with 0 interactions in 90 days has score < 50
}
```

### TUI tests

Manual golden-path walkthrough:
1. Import a 20-contact CSV that includes 2 colleagues and 1 alumni.
2. Navigate to Networking → Agentic view.
3. Verify warm paths appear with correct badges.
4. Press `b` on a contact — verify "Generating..." spinner.
5. After brief loop mock completes, press `b` again — verify brief overlay renders with scroll.
6. Press `d` on a warm path — verify it disappears from the list.

## Open Questions

1. **Alumni year disambiguation**: If both user and contact graduated from the same school but 20
   years apart, should the path type still be `SharedAlumni`? Current implementation: yes, with
   no year filtering. Phase 2 can add a "graduation year proximity" bonus to `WarmPathScore`.

2. **Contact import school field format**: LinkedIn CSV does not export a structured `education`
   column — only summary text in `Headline` or `Summary`. Phase 1 alumni inference relies on
   any structured `education` field the user has manually entered. A Phase 2 improvement would
   add an LLM extraction pass over the contact's LinkedIn headline for school signals.

3. **How to handle duplicate company names from multi-source imports**: If a contact's employer
   "Google LLC" matches "Google" via jaro_winkler, the company linkage uses the normalized form.
   But if two contacts list "Alphabet Inc." and "Google", they would NOT match currently.
   `normalize_company_name()` from the networking-connection-mapping plan handles legal suffix
   stripping, but brand aliases (Alphabet/Google) require a Phase 2 alias table.

4. **OutreachBriefLoop LLM cost**: A brief requires ~800-1200 tokens. With the system prompt
   cached (Anthropic cache), subsequent calls for the same contact save ~400 tokens. Budget:
   accept up to 10 brief generations per day (≤ 12k tokens total) before surfacing a cost
   warning. Token budget mechanism from the Ralph orchestration plan applies here.

5. **Relationship health score privacy**: `relationship_health` rows contain `nurture_actions`
   with plaintext hints referencing contact names. If `PrivacyMode::Stealth` is enabled, these
   hints must be redacted before logging or display. The privacy layer integration is deferred
   to Phase 5.

## Related Specs

- [networking-connection-mapping.md](networking-connection-mapping.md) — `ProfileContact`, `ContactRepository`, import infrastructure
- [networking-referral-management.md](networking-referral-management.md) — `RelationshipStage`, `ReferralAsk`, `NetworkingReminderPoller`
- [networking-outreach-drafting.md](networking-outreach-drafting.md) — `OutreachDraft`, `OutreachFabricationChecker`
- [job-search-company-research.md](job-search-company-research.md) — `CompanyRecord`, company news data
- [job-search-ghost-job-detection.md](job-search-ghost-job-detection.md) — `ghost_score`, ghost warnings in briefs
- [agentic-ralph-orchestration.md](agentic-ralph-orchestration.md) — `LoopManager`, `PostTransitionSuggestion::FindWarmPaths`
- [17-ralph-prompt-templates.md](17-ralph-prompt-templates.md) — `TemplateRegistry`, `SimpleTemplateEngine`
- [16-privacy-security.md](16-privacy-security.md) — `PrivacyMode`, data redaction (Phase 5)
