# Implementation Plan: Cross-Source Application Deduplication

## Status
Draft

## Related Spec
`specs/XX-application-cross-source-deduplication.md`

## Overview

LazyJob aggregates job listings from multiple sources (LinkedIn, Greenhouse, Lever, direct
careers pages). The same position can appear across several boards simultaneously, each with
a distinct `source_id`. Without deduplication, a user who discovers a Stripe Senior Software
Engineer posting on both LinkedIn and Greenhouse would see two job records, two kanban cards,
and potentially create two applications — inflating pipeline metrics and wasting tailoring effort.

This plan defines a fingerprint-based deduplication layer that runs synchronously during job
ingestion, detects probable duplicates, groups them in SQLite, and gives the user full control
over how groups are consolidated. The deduplication engine operates on the `Job` domain type
already defined in `lazyjob-core`, is invoked by `JobIngestionService`, and exposes a review
queue consumed by the TUI's Deduplication Review view.

The consolidation model separates _matching_ (algorithmic, automatic) from _merging_ (user-
driven or policy-driven). Matches are stored as `job_duplicate_groups`; a merge produces a
`ConsolidatedJob` record that acts as an umbrella linking all source copies. Applications are
always attached to the `ConsolidatedJob` once created, so the pipeline view stays clean even
before the user resolves the review queue.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobIngestionService`, `UpsertOutcome`, `JobRepository`
- `specs/application-state-machine-implementation-plan.md` — `Application`, `ApplicationId`, `ApplicationStage`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
# Existing from discovery / platform plans
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4"] }
tracing     = "0.1"
thiserror   = "2"
anyhow      = "1"
tokio       = { version = "1", features = ["macros", "rt-multi-thread"] }
strsim      = "0.11"    # jaro_winkler for title / company fuzzy matching
once_cell   = "1"       # Lazy<Regex> for company suffix stripping
regex       = "1"       # legal suffix normalizer

[dev-dependencies]
rusqlite    = { version = "0.31", features = ["bundled"] }
tempfile    = "3"
tokio       = { version = "1", features = ["full"] }
```

## Architecture

### Crate Placement

All deduplication code lives in `lazyjob-core/src/dedup/`. The same crate already owns
`Job`, `JobId`, `Application`, and `ApplicationId`, so no cross-crate boundary is introduced.
`lazyjob-ralph` imports `DeduplicationService` to run the fingerprint pass after each source
fetch. `lazyjob-tui` imports `DuplicateReviewService` to populate the review queue view.

### Core Types

```rust
// lazyjob-core/src/dedup/types.rs

use crate::jobs::JobId;
use crate::applications::ApplicationId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Newtype: a stable identifier for a canonical job (the "primary" record across sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConsolidatedJobId(Uuid);

impl ConsolidatedJobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConsolidatedJobId {
    fn default() -> Self {
        Self::new()
    }
}

/// A group of job IDs that the engine believes represent the same position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub id: DuplicateGroupId,
    /// All job IDs in this group. At least 2 entries.
    pub job_ids: Vec<JobId>,
    /// Similarity score of the weakest pair in the group (0.0–1.0).
    pub min_similarity: f32,
    /// Which job was elected as the primary (highest-priority source).
    pub primary_job_id: JobId,
    pub status: GroupStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum GroupStatus {
    /// Detected; awaiting user decision.
    PendingReview,
    /// User merged into a ConsolidatedJob.
    Merged,
    /// User confirmed these are distinct jobs.
    ConfirmedDistinct,
    /// System auto-merged (score >= auto_merge_threshold from config).
    AutoMerged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DuplicateGroupId(Uuid);

impl DuplicateGroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// The canonical umbrella record linking all source job copies to one application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidatedJob {
    pub id: ConsolidatedJobId,
    /// The "winning" job record whose fields are shown to the user.
    pub primary_job_id: JobId,
    /// All source job IDs, including the primary.
    pub source_job_ids: Vec<JobId>,
    /// Single application (if any) attached to this consolidated record.
    pub application_id: Option<ApplicationId>,
    pub created_at: DateTime<Utc>,
}

/// The computed fingerprint for a single job. Not persisted; recomputed during matching.
#[derive(Debug, Clone)]
pub struct JobFingerprint {
    /// Lowercase, legal-suffix-stripped company name. e.g. "stripe".
    pub normalized_company: String,
    /// Tokenized, lowercased, deduplicated title tokens. e.g. ["senior", "software", "engineer"].
    pub title_tokens: Vec<String>,
    /// Normalized location string (lowercase city/state only, no "Remote" qualifier).
    pub normalized_location: String,
    /// SHA-256 (first 8 bytes) of trimmed description text. Used for exact-match fast path.
    pub description_prefix_hash: u64,
}

/// Signal breakdown returned by fingerprint comparison.
#[derive(Debug, Clone)]
pub struct SimilarityBreakdown {
    pub company_match: bool,
    pub title_similarity: f32,
    pub location_match: bool,
    pub description_exact: bool,
    /// Final weighted score: 0.0–1.0.
    pub composite_score: f32,
}

/// Source priority for field resolution during merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum SourcePriority {
    /// Lowest numeric value = highest priority (Ord used for sorting).
    DirectCareers = 0,
    Greenhouse    = 1,
    Lever         = 2,
    LinkedIn      = 3,
    Adzuna        = 4,
    Unknown       = 5,
}

impl SourcePriority {
    pub fn from_source_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "direct" | "careers" => Self::DirectCareers,
            "greenhouse"         => Self::Greenhouse,
            "lever"              => Self::Lever,
            "linkedin"           => Self::LinkedIn,
            "adzuna"             => Self::Adzuna,
            _                   => Self::Unknown,
        }
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/dedup/traits.rs

use async_trait::async_trait;
use crate::dedup::types::*;
use crate::jobs::Job;

#[async_trait]
pub trait DuplicateGroupRepository: Send + Sync {
    async fn insert_group(&self, group: &DuplicateGroup) -> Result<(), DedupError>;
    async fn update_group_status(
        &self,
        id: DuplicateGroupId,
        status: GroupStatus,
        resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), DedupError>;
    async fn list_pending(&self) -> Result<Vec<DuplicateGroup>, DedupError>;
    async fn find_by_job_id(&self, job_id: crate::jobs::JobId) -> Result<Option<DuplicateGroup>, DedupError>;
}

#[async_trait]
pub trait ConsolidatedJobRepository: Send + Sync {
    async fn insert(&self, record: &ConsolidatedJob) -> Result<(), DedupError>;
    async fn find_by_primary(&self, job_id: crate::jobs::JobId) -> Result<Option<ConsolidatedJob>, DedupError>;
    async fn link_application(
        &self,
        id: ConsolidatedJobId,
        application_id: crate::applications::ApplicationId,
    ) -> Result<(), DedupError>;
}
```

### SQLite Schema

```sql
-- migration 018_cross_source_dedup.sql

CREATE TABLE IF NOT EXISTS job_duplicate_groups (
    id              TEXT NOT NULL PRIMARY KEY,  -- DuplicateGroupId (UUID)
    job_ids         TEXT NOT NULL,              -- JSON array of JobId strings
    primary_job_id  TEXT NOT NULL REFERENCES jobs(id),
    min_similarity  REAL NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending_review'
                        CHECK (status IN ('pending_review','merged','confirmed_distinct','auto_merged')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    resolved_at     TEXT
);

-- Allows fast lookup by a single job_id within any group.
-- We store the job_ids JSON array and query with json_each() for correctness,
-- but this index on primary_job_id covers the most common lookup path.
CREATE INDEX IF NOT EXISTS idx_dup_groups_primary_job
    ON job_duplicate_groups(primary_job_id);

CREATE INDEX IF NOT EXISTS idx_dup_groups_status
    ON job_duplicate_groups(status)
    WHERE status = 'pending_review';

CREATE TABLE IF NOT EXISTS consolidated_jobs (
    id              TEXT NOT NULL PRIMARY KEY,  -- ConsolidatedJobId (UUID)
    primary_job_id  TEXT NOT NULL REFERENCES jobs(id),
    source_job_ids  TEXT NOT NULL,              -- JSON array of JobId strings
    application_id  TEXT REFERENCES applications(id),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_consolidated_primary
    ON consolidated_jobs(primary_job_id);

-- Job-level flag so IngestionService can skip already-grouped jobs in future passes.
ALTER TABLE jobs ADD COLUMN dedup_group_id TEXT REFERENCES job_duplicate_groups(id);
```

### Module Structure

```
lazyjob-core/
  src/
    dedup/
      mod.rs           # pub use; module registry
      types.rs         # DuplicateGroup, ConsolidatedJob, JobFingerprint, SourcePriority
      traits.rs        # DuplicateGroupRepository, ConsolidatedJobRepository
      fingerprint.rs   # JobFingerprint::generate(), similarity(), normalizer helpers
      engine.rs        # DeduplicationEngine (pure, no I/O): find_groups()
      service.rs       # DeduplicationService (async, owns repos): run_dedup_pass(), merge_group()
      review.rs        # DuplicateReviewService: list_pending(), confirm_distinct(), merge()
      sqlite.rs        # SqliteDuplicateGroupRepo, SqliteConsolidatedJobRepo
      error.rs         # DedupError
```

## Implementation Phases

### Phase 1 — Fingerprinting and In-Memory Detection (MVP)

**Step 1.1 — Define `DedupError`**

File: `lazyjob-core/src/dedup/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum DedupError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Group not found: {0:?}")]
    GroupNotFound(DuplicateGroupId),

    #[error("Cannot merge group in status {0:?}")]
    InvalidGroupStatus(GroupStatus),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, DedupError>;
```

**Step 1.2 — Company name normalizer**

File: `lazyjob-core/src/dedup/fingerprint.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;

// Strip common legal suffixes. Sorted longest-first to avoid partial replacement.
static LEGAL_SUFFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\s*[,.]?\s*(incorporated|corporation|limited|llc|llp|ltd|inc|corp|co\.?)\.?\s*$"
    ).unwrap()
});

static WHITESPACE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\s+").unwrap()
});

/// Returns a lowercase, suffix-stripped, collapsed company name.
/// E.g. "Stripe, Inc." → "stripe"
pub fn normalize_company(name: &str) -> String {
    let stripped = LEGAL_SUFFIX_RE.replace(name, "");
    WHITESPACE_RE.replace_all(stripped.trim(), " ")
        .to_ascii_lowercase()
}

/// Returns lowercase, punctuation-stripped title tokens deduplicated in order.
/// E.g. "Senior Software Engineer (Remote)" → ["senior", "software", "engineer", "remote"]
pub fn tokenize_title(title: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .filter_map(|t| {
            if seen.insert(t.to_string()) { Some(t.to_string()) } else { None }
        })
        .collect()
}

/// Normalize a location string to "city state" lowercase.
/// Strips country, zip codes, "Remote", "Hybrid" qualifiers.
pub fn normalize_location(location: &str) -> String {
    static REMOTE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(remote|hybrid|onsite|on-site|anywhere)\b").unwrap()
    });
    static ZIP_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b\d{5}(-\d{4})?\b").unwrap()
    });
    let s = REMOTE_RE.replace_all(location, "");
    let s = ZIP_RE.replace_all(&s, "");
    WHITESPACE_RE.replace_all(s.trim(), " ")
        .to_ascii_lowercase()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string()
}

/// Compute description prefix hash (first 512 bytes of trimmed text → first 8 bytes of SHA-256).
pub fn description_prefix_hash(description: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let prefix: &str = &description.trim()[..description.trim().len().min(512)];
    let mut h = DefaultHasher::new();
    prefix.hash(&mut h);
    h.finish()
}
```

**Step 1.3 — `JobFingerprint::generate()` and `SimilarityBreakdown::compute()`**

```rust
// lazyjob-core/src/dedup/fingerprint.rs (continued)

use crate::jobs::Job;
use super::types::{JobFingerprint, SimilarityBreakdown};

impl JobFingerprint {
    pub fn generate(job: &Job) -> Self {
        Self {
            normalized_company:    normalize_company(&job.company_name),
            title_tokens:          tokenize_title(&job.title),
            normalized_location:   normalize_location(&job.location.as_deref().unwrap_or("")),
            description_prefix_hash: description_prefix_hash(
                job.description.as_deref().unwrap_or("")
            ),
        }
    }
}

impl SimilarityBreakdown {
    /// Compute similarity between two fingerprints.
    ///
    /// Weights (must sum to 1.0):
    ///   company exact match:  0.50
    ///   title jaro-winkler:   0.40
    ///   location match:       0.10
    ///
    /// If description prefix hashes match, return 1.0 immediately (exact duplicate).
    pub fn compute(a: &JobFingerprint, b: &JobFingerprint) -> Self {
        // Fast path: identical description prefix ⟹ same posting
        if a.description_prefix_hash == b.description_prefix_hash
            && a.description_prefix_hash != 0
        {
            return Self {
                company_match: true,
                title_similarity: 1.0,
                location_match: true,
                description_exact: true,
                composite_score: 1.0,
            };
        }

        let company_match = a.normalized_company == b.normalized_company;
        // If companies don't match at all, skip the rest — can't be the same job.
        if !company_match {
            return Self {
                company_match: false,
                title_similarity: 0.0,
                location_match: false,
                description_exact: false,
                composite_score: 0.0,
            };
        }

        let title_a = a.title_tokens.join(" ");
        let title_b = b.title_tokens.join(" ");
        let title_similarity = strsim::jaro_winkler(&title_a, &title_b) as f32;

        let location_match = a.normalized_location == b.normalized_location
            || a.normalized_location.is_empty()
            || b.normalized_location.is_empty();

        let composite_score =
            0.50_f32 /* company_match = true, so full score */
            + title_similarity * 0.40
            + if location_match { 0.10 } else { 0.0 };

        Self {
            company_match,
            title_similarity,
            location_match,
            description_exact: false,
            composite_score: composite_score.min(1.0),
        }
    }
}
```

Verification: unit-test `SimilarityBreakdown::compute` with pairs:
- "Stripe Senior SWE / SF" vs "Stripe Senior Software Engineer / San Francisco" → score ≥ 0.85
- "Stripe SWE" vs "Stripe Product Manager" → score ≤ 0.65
- "Stripe SWE" vs "Brex SWE" → company_match=false, score=0.0

**Step 1.4 — `DeduplicationEngine` (pure, no I/O)**

File: `lazyjob-core/src/dedup/engine.rs`

```rust
use crate::jobs::Job;
use super::fingerprint::{JobFingerprint, SimilarityBreakdown};
use super::types::{DuplicateGroup, DuplicateGroupId, GroupStatus, SourcePriority};
use chrono::Utc;

pub struct DeduplicationEngine {
    /// Minimum composite score to consider two jobs duplicates.
    pub match_threshold: f32,
    /// Minimum composite score for automatic merge without user review.
    pub auto_merge_threshold: f32,
}

impl Default for DeduplicationEngine {
    fn default() -> Self {
        Self {
            match_threshold:      0.85,
            auto_merge_threshold: 0.97,
        }
    }
}

impl DeduplicationEngine {
    /// Given a slice of jobs, return groups of probable duplicates.
    /// O(n²) — acceptable for typical batch sizes of ≤ 500 jobs per discovery run.
    ///
    /// Jobs already assigned a `dedup_group_id` in SQLite are passed in with
    /// `existing_group_id: Some(...)` — this function only generates NEW groups
    /// from the provided slice.
    pub fn find_new_groups(&self, jobs: &[Job]) -> Vec<DuplicateGroup> {
        let fingerprints: Vec<JobFingerprint> = jobs
            .iter()
            .map(JobFingerprint::generate)
            .collect();

        let mut assigned: Vec<Option<usize>> = vec![None; jobs.len()];
        let mut groups: Vec<Vec<usize>> = Vec::new();

        for i in 0..jobs.len() {
            if assigned[i].is_some() {
                continue;
            }

            let mut group_indices = vec![i];

            for j in (i + 1)..jobs.len() {
                if assigned[j].is_some() {
                    continue;
                }
                let breakdown = SimilarityBreakdown::compute(&fingerprints[i], &fingerprints[j]);
                if breakdown.composite_score >= self.match_threshold {
                    group_indices.push(j);
                    assigned[j] = Some(groups.len());
                }
            }

            assigned[i] = Some(groups.len());
            if group_indices.len() > 1 {
                groups.push(group_indices);
            }
        }

        groups
            .into_iter()
            .map(|indices| {
                let job_ids: Vec<_> = indices.iter().map(|&i| jobs[i].id).collect();
                // Elect primary by SourcePriority (lowest enum value wins).
                let primary_job_id = job_ids.iter().copied().min_by_key(|id| {
                    let job = jobs.iter().find(|j| j.id == *id).unwrap();
                    SourcePriority::from_source_name(job.source.as_deref().unwrap_or(""))
                }).unwrap();

                let min_sim = self.compute_min_similarity(&indices, &fingerprints);

                let status = if min_sim >= self.auto_merge_threshold {
                    GroupStatus::AutoMerged
                } else {
                    GroupStatus::PendingReview
                };

                DuplicateGroup {
                    id:             DuplicateGroupId::new(),
                    job_ids,
                    min_similarity: min_sim,
                    primary_job_id,
                    status,
                    created_at:     Utc::now(),
                    resolved_at:    None,
                }
            })
            .collect()
    }

    fn compute_min_similarity(&self, indices: &[usize], fps: &[JobFingerprint]) -> f32 {
        let mut min = f32::MAX;
        for &i in indices {
            for &j in indices {
                if i >= j { continue; }
                let score = SimilarityBreakdown::compute(&fps[i], &fps[j]).composite_score;
                if score < min { min = score; }
            }
        }
        if min == f32::MAX { 1.0 } else { min }
    }
}
```

Verification: test `find_new_groups` with 4 jobs where 2 are obvious duplicates — expect 1 group of 2.

---

### Phase 2 — SQLite Persistence

**Step 2.1 — Apply migration 018**

File: `lazyjob-core/migrations/018_cross_source_dedup.sql` (DDL from Architecture section above).

**Step 2.2 — `SqliteDuplicateGroupRepository`**

File: `lazyjob-core/src/dedup/sqlite.rs`

```rust
use rusqlite::{Connection, params};
use crate::dedup::types::{DuplicateGroup, DuplicateGroupId, GroupStatus};
use crate::dedup::traits::DuplicateGroupRepository;
use crate::jobs::JobId;
use crate::dedup::error::Result;
use chrono::Utc;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SqliteDuplicateGroupRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteDuplicateGroupRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl DuplicateGroupRepository for SqliteDuplicateGroupRepository {
    async fn insert_group(&self, group: &DuplicateGroup) -> Result<()> {
        let conn = self.conn.lock().await;
        let job_ids_json = serde_json::to_string(&group.job_ids)?;
        conn.execute(
            "INSERT INTO job_duplicate_groups
                (id, job_ids, primary_job_id, min_similarity, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO NOTHING",
            params![
                group.id.to_string(),
                job_ids_json,
                group.primary_job_id.to_string(),
                group.min_similarity,
                serde_json::to_string(&group.status)?,
                group.created_at.to_rfc3339(),
            ],
        )?;

        // Update each job's dedup_group_id pointer.
        for job_id in &group.job_ids {
            conn.execute(
                "UPDATE jobs SET dedup_group_id = ?1 WHERE id = ?2",
                params![group.id.to_string(), job_id.to_string()],
            )?;
        }
        Ok(())
    }

    async fn update_group_status(
        &self,
        id: DuplicateGroupId,
        status: GroupStatus,
        resolved_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE job_duplicate_groups SET status = ?1, resolved_at = ?2 WHERE id = ?3",
            params![
                serde_json::to_string(&status)?,
                resolved_at.map(|t| t.to_rfc3339()),
                id.to_string(),
            ],
        )?;
        Ok(())
    }

    async fn list_pending(&self) -> Result<Vec<DuplicateGroup>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, job_ids, primary_job_id, min_similarity, status, created_at, resolved_at
               FROM job_duplicate_groups
              WHERE status = 'pending_review'
              ORDER BY created_at DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;

        rows.map(|r| {
            let (id, job_ids_json, primary_id, min_sim, status_json, created_at_str, resolved_str) = r?;
            Ok(DuplicateGroup {
                id:             DuplicateGroupId::parse(&id),
                job_ids:        serde_json::from_str(&job_ids_json)?,
                primary_job_id: JobId::parse(&primary_id),
                min_similarity: min_sim as f32,
                status:         serde_json::from_str(&status_json)?,
                created_at:     created_at_str.parse().unwrap(),
                resolved_at:    resolved_str.and_then(|s| s.parse().ok()),
            })
        }).collect()
    }

    async fn find_by_job_id(&self, job_id: JobId) -> Result<Option<DuplicateGroup>> {
        let conn = self.conn.lock().await;
        // Use json_each to search inside the JSON array.
        let mut stmt = conn.prepare(
            "SELECT g.id, g.job_ids, g.primary_job_id, g.min_similarity, g.status,
                    g.created_at, g.resolved_at
               FROM job_duplicate_groups g, json_each(g.job_ids) jid
              WHERE jid.value = ?1
              LIMIT 1"
        )?;
        // ... (row mapping identical to list_pending)
        let _ = stmt; // implementation elided for brevity — same row mapper
        Ok(None) // placeholder
    }
}
```

**Step 2.3 — `SqliteConsolidatedJobRepository`**

```rust
// lazyjob-core/src/dedup/sqlite.rs (continued)

pub struct SqliteConsolidatedJobRepository {
    conn: Arc<Mutex<Connection>>,
}

#[async_trait]
impl ConsolidatedJobRepository for SqliteConsolidatedJobRepository {
    async fn insert(&self, record: &ConsolidatedJob) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO consolidated_jobs (id, primary_job_id, source_job_ids, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                record.id.to_string(),
                record.primary_job_id.to_string(),
                serde_json::to_string(&record.source_job_ids)?,
                record.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    async fn find_by_primary(&self, job_id: JobId) -> Result<Option<ConsolidatedJob>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, primary_job_id, source_job_ids, application_id, created_at
               FROM consolidated_jobs WHERE primary_job_id = ?1"
        )?;
        let mut rows = stmt.query_map(params![job_id.to_string()], |row| {
            Ok(ConsolidatedJob {
                id:              ConsolidatedJobId::parse(row.get::<_, String>(0)?),
                primary_job_id:  JobId::parse(row.get::<_, String>(1)?),
                source_job_ids:  serde_json::from_str(&row.get::<_, String>(2)?).unwrap(),
                application_id:  row.get::<_, Option<String>>(3)?
                    .map(|s| ApplicationId::parse(&s)),
                created_at:      row.get::<_, String>(4)?.parse().unwrap(),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    async fn link_application(&self, id: ConsolidatedJobId, application_id: ApplicationId) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE consolidated_jobs SET application_id = ?1 WHERE id = ?2",
            params![application_id.to_string(), id.to_string()],
        )?;
        Ok(())
    }
}
```

Verification: `#[cfg(test)]` with in-memory rusqlite — insert a group, query `list_pending()`, assert one result. Insert a `ConsolidatedJob`, call `link_application`, re-fetch, assert `application_id` is set.

---

### Phase 3 — Service Layer

**Step 3.1 — `DeduplicationService`**

File: `lazyjob-core/src/dedup/service.rs`

```rust
use std::sync::Arc;
use crate::jobs::{Job, JobId, JobRepository};
use crate::dedup::{
    engine::DeduplicationEngine,
    traits::{DuplicateGroupRepository, ConsolidatedJobRepository},
    types::{ConsolidatedJob, ConsolidatedJobId, DuplicateGroup, GroupStatus},
    error::{DedupError, Result},
};
use chrono::Utc;
use tracing::{info, warn};

pub struct DeduplicationService {
    engine:       DeduplicationEngine,
    group_repo:   Arc<dyn DuplicateGroupRepository>,
    consol_repo:  Arc<dyn ConsolidatedJobRepository>,
    job_repo:     Arc<dyn JobRepository>,
}

impl DeduplicationService {
    pub fn new(
        group_repo:  Arc<dyn DuplicateGroupRepository>,
        consol_repo: Arc<dyn ConsolidatedJobRepository>,
        job_repo:    Arc<dyn JobRepository>,
    ) -> Self {
        Self {
            engine:      DeduplicationEngine::default(),
            group_repo,
            consol_repo,
            job_repo,
        }
    }

    /// Run a full deduplication pass over the given job IDs.
    ///
    /// - Generates fingerprints, forms groups, persists them.
    /// - AutoMerged groups are immediately consolidated.
    /// - PendingReview groups are left for the user.
    ///
    /// Returns a summary of detected groups.
    #[tracing::instrument(skip(self, job_ids))]
    pub async fn run_pass(&self, job_ids: &[JobId]) -> Result<DedupReport> {
        let jobs = self.job_repo.find_by_ids(job_ids).await?;
        let unassigned: Vec<Job> = jobs
            .into_iter()
            .filter(|j| j.dedup_group_id.is_none())
            .collect();

        if unassigned.is_empty() {
            return Ok(DedupReport::default());
        }

        let groups = self.engine.find_new_groups(&unassigned);

        let mut pending_count = 0usize;
        let mut auto_merged_count = 0usize;

        for group in &groups {
            self.group_repo.insert_group(group).await?;

            match group.status {
                GroupStatus::AutoMerged => {
                    self.consolidate_group(group).await?;
                    auto_merged_count += 1;
                }
                GroupStatus::PendingReview => {
                    pending_count += 1;
                    info!(
                        group_id = %group.id,
                        count = group.job_ids.len(),
                        "Duplicate group pending user review"
                    );
                }
                _ => {}
            }
        }

        Ok(DedupReport {
            total_groups:    groups.len(),
            auto_merged:     auto_merged_count,
            pending_review:  pending_count,
        })
    }

    /// Create a ConsolidatedJob for a group and mark the group Merged.
    async fn consolidate_group(&self, group: &DuplicateGroup) -> Result<()> {
        let consol = ConsolidatedJob {
            id:              ConsolidatedJobId::new(),
            primary_job_id:  group.primary_job_id,
            source_job_ids:  group.job_ids.clone(),
            application_id:  None,
            created_at:      Utc::now(),
        };
        self.consol_repo.insert(&consol).await?;
        self.group_repo.update_group_status(
            group.id,
            GroupStatus::Merged,
            Some(Utc::now()),
        ).await?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct DedupReport {
    pub total_groups:   usize,
    pub auto_merged:    usize,
    pub pending_review: usize,
}
```

**Step 3.2 — `DuplicateReviewService`**

File: `lazyjob-core/src/dedup/review.rs`

```rust
use std::sync::Arc;
use chrono::Utc;
use crate::dedup::{
    traits::{DuplicateGroupRepository, ConsolidatedJobRepository},
    types::{ConsolidatedJob, ConsolidatedJobId, DuplicateGroup, DuplicateGroupId, GroupStatus},
    error::{DedupError, Result},
};

pub struct DuplicateReviewService {
    group_repo:  Arc<dyn DuplicateGroupRepository>,
    consol_repo: Arc<dyn ConsolidatedJobRepository>,
}

impl DuplicateReviewService {
    pub fn new(
        group_repo:  Arc<dyn DuplicateGroupRepository>,
        consol_repo: Arc<dyn ConsolidatedJobRepository>,
    ) -> Self {
        Self { group_repo, consol_repo }
    }

    /// Return all groups awaiting user review, sorted newest-first.
    pub async fn list_pending(&self) -> Result<Vec<DuplicateGroup>> {
        self.group_repo.list_pending().await
    }

    /// User chose: these jobs are NOT duplicates.
    pub async fn confirm_distinct(&self, group_id: DuplicateGroupId) -> Result<()> {
        self.group_repo.update_group_status(
            group_id,
            GroupStatus::ConfirmedDistinct,
            Some(Utc::now()),
        ).await
    }

    /// User chose: merge all source jobs in the group to the primary.
    /// Creates a ConsolidatedJob if one doesn't exist already.
    pub async fn merge_group(&self, group: &DuplicateGroup) -> Result<ConsolidatedJobId> {
        if !matches!(group.status, GroupStatus::PendingReview) {
            return Err(DedupError::InvalidGroupStatus(group.status));
        }

        let consol = ConsolidatedJob {
            id:             ConsolidatedJobId::new(),
            primary_job_id: group.primary_job_id,
            source_job_ids: group.job_ids.clone(),
            application_id: None,
            created_at:     Utc::now(),
        };
        let id = consol.id;
        self.consol_repo.insert(&consol).await?;
        self.group_repo.update_group_status(
            group.id,
            GroupStatus::Merged,
            Some(Utc::now()),
        ).await?;
        Ok(id)
    }
}
```

**Step 3.3 — Field resolution when merging**

File: `lazyjob-core/src/dedup/service.rs` (helper)

```rust
/// Select the best field value from multiple (source, value) pairs.
/// Sources are sorted by `SourcePriority` (lower = preferred).
/// An empty/None value is skipped; the first non-empty value wins.
pub fn resolve_field<'a>(candidates: &[(SourcePriority, Option<&'a str>)]) -> Option<&'a str> {
    let mut sorted: Vec<_> = candidates.iter().collect();
    sorted.sort_by_key(|(p, _)| *p);
    sorted.into_iter()
        .find_map(|(_, v)| v.filter(|s| !s.trim().is_empty()))
}
```

Verification: unit-test `resolve_field` with LinkedIn returning empty salary and Greenhouse returning "$130k–$160k" — expect Greenhouse value.

---

### Phase 4 — Integration with Discovery Pipeline

**Step 4.1 — Hook into `JobIngestionService`**

In `lazyjob-core/src/discovery/service.rs`, after `JobRepository::upsert_batch()`, call:

```rust
// After upsert, run dedup on the batch of job IDs that were Inserted or Updated.
let new_ids: Vec<JobId> = upserted
    .iter()
    .filter(|(outcome, _)| matches!(outcome, UpsertOutcome::Inserted))
    .map(|(_, id)| *id)
    .collect();

if !new_ids.is_empty() {
    let report = self.dedup_service.run_pass(&new_ids).await
        .context("deduplication pass failed")?;
    tracing::info!(
        auto_merged = report.auto_merged,
        pending_review = report.pending_review,
        "Deduplication pass complete"
    );
}
```

The `DeduplicationService` is injected into `JobIngestionService` via `Arc<DeduplicationService>`.
This keeps the discovery and dedup layers independently testable.

**Step 4.2 — Prevent duplicate applications**

In `lazyjob-core/src/workflow/apply.rs` (`ApplyWorkflow::execute`), before creating a new
application, check whether the target job is part of a `ConsolidatedJob` that already has
an `application_id`:

```rust
if let Some(consol) = self.consol_repo.find_by_primary(options.job_id).await? {
    if let Some(existing_app_id) = consol.application_id {
        return Err(WorkflowError::DuplicateApplication {
            existing_application_id: existing_app_id,
            consolidated_job_id:     consol.id,
        });
    }
}
```

This allows the TUI to show a non-fatal confirmation dialog: "You already applied to this job
from another source. Apply again?"

---

### Phase 5 — TUI Review View

File: `lazyjob-tui/src/views/dedup_review.rs`

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use crate::app::AppState;

pub struct DedupReviewView {
    list_state:     ListState,
    pending_groups: Vec<DuplicateGroup>,
    selected_group: Option<DuplicateGroup>,
}

impl DedupReviewView {
    /// Render the full review view.
    /// Layout: left 50% = pending groups list; right 50% = group detail.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        self.render_groups_list(frame, chunks[0]);
        if let Some(ref group) = self.selected_group {
            self.render_group_detail(frame, chunks[1], group);
        }
    }

    fn render_groups_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.pending_groups.iter().map(|g| {
            let label = format!(
                "{} duplicates  sim: {:.0}%",
                g.job_ids.len(),
                g.min_similarity * 100.0
            );
            ListItem::new(label)
        }).collect();

        let list = List::new(items)
            .block(Block::default().title("Pending Review").borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_group_detail(&self, frame: &mut Frame, area: Rect, group: &DuplicateGroup) {
        // Display source jobs table: title, company, source, location
        // Bottom: keybind hints [m]erge  [d]istinct  [↑↓] navigate
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("[m] Merge", Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled("[d] Distinct", Style::default().fg(Color::Yellow)),
            Span::raw("  "),
            Span::styled("[Esc] Cancel", Style::default().fg(Color::Gray)),
        ]))
        .block(Block::default().borders(Borders::TOP));

        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);

        // Job detail table rendering (abbreviated — use ratatui::widgets::Table)
        let _ = (inner, hint, group); // placeholder
    }
}
```

**Keybindings in `DedupReviewView`:**

| Key | Action |
|-----|--------|
| `j` / `↓` | Next group |
| `k` / `↑` | Previous group |
| `m` | Merge selected group |
| `d` | Confirm distinct |
| `?` | Toggle help overlay |
| `Esc` / `q` | Return to Jobs view |

**Confirmation dialog** uses `ratatui::widgets::Clear` to erase the background before rendering
a centered 40×8 box — identical to the pattern in `specs/09-tui-design-keybindings-implementation-plan.md`.

---

## Key Crate APIs

- `strsim::jaro_winkler(&str, &str) -> f64` — title similarity; cast to f32 for all internal math
- `rusqlite::Connection::execute(sql, params![...])` — DDL and DML
- `rusqlite::Connection::prepare(&str)` + `Statement::query_map(params, closure)` — SELECT rows
- `serde_json::to_string(&T)` / `serde_json::from_str::<T>(&str)` — JSON array serialization for `job_ids` column
- `once_cell::sync::Lazy<Regex>` — compile legal suffix / whitespace regexes once at startup
- `regex::Regex::replace_all(text, replacement)` — company name normalization
- `uuid::Uuid::new_v4()` — generate IDs for `DuplicateGroupId`, `ConsolidatedJobId`
- `tokio::sync::Mutex<rusqlite::Connection>` — async-safe SQLite access
- `ratatui::widgets::{List, ListItem, ListState, Table, Row, Cell, Clear, Block}` — TUI rendering
- `tracing::info!` / `tracing::warn!` — structured logging throughout

## Error Handling

```rust
// lazyjob-core/src/dedup/error.rs

#[derive(thiserror::Error, Debug)]
pub enum DedupError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Duplicate group not found: {0:?}")]
    GroupNotFound(DuplicateGroupId),

    #[error("Cannot modify group with status {0:?}")]
    InvalidGroupStatus(GroupStatus),

    #[error("Job repository error: {0}")]
    JobRepo(#[from] crate::jobs::JobError),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, DedupError>;
```

`WorkflowError` (in `lazyjob-core/src/workflow/error.rs`) gets a new variant:

```rust
#[error("Already applied to this job from another source (application: {existing_application_id:?})")]
DuplicateApplication {
    existing_application_id: ApplicationId,
    consolidated_job_id:     ConsolidatedJobId,
},
```

The TUI interprets `DuplicateApplication` as a dismissable confirmation dialog (not a crash).

## Testing Strategy

### Unit Tests

All in `lazyjob-core/src/dedup/` as `#[cfg(test)]` modules.

**Fingerprinting tests** (`fingerprint.rs`):
- `normalize_company("Stripe, Inc.")` → `"stripe"`
- `normalize_company("Meta Platforms, LLC")` → `"meta platforms"`
- `tokenize_title("Senior Software Engineer (Remote)")` → `["senior", "software", "engineer", "remote"]`
- `normalize_location("San Francisco, CA 94107")` → `"san francisco ca"`

**Similarity tests** (`fingerprint.rs`):
- Same company, similar title, same location: `composite_score >= 0.85`
- Same company, very different title: `composite_score <= 0.65`
- Different company: `composite_score == 0.0`
- Description prefix hash match: `composite_score == 1.0`

**Engine tests** (`engine.rs`):
- 4 jobs: 2 Stripe SWE duplicates + 1 Stripe PM + 1 Brex SWE → exactly 1 group of 2
- AutoMerge threshold: a group with `min_sim >= 0.97` has `GroupStatus::AutoMerged`

**Service tests** (`service.rs`):
- `resolve_field` with empty LinkedIn salary + non-empty Greenhouse salary → Greenhouse value wins
- `DuplicateReviewService::merge_group` on `PendingReview` group → `GroupStatus::Merged`, returns `ConsolidatedJobId`
- `DuplicateReviewService::merge_group` on already-`Merged` group → `Err(InvalidGroupStatus)`

### Integration Tests (in-memory SQLite)

```rust
#[tokio::test]
async fn test_dedup_pass_inserts_group_and_consol() {
    let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    apply_migrations(&conn).await; // runs migration 018
    let group_repo = Arc::new(SqliteDuplicateGroupRepository::new(conn.clone()));
    let consol_repo = Arc::new(SqliteConsolidatedJobRepository::new(conn.clone()));
    let job_repo = Arc::new(MockJobRepository::with_jobs(vec![stripe_swe_linkedin(), stripe_swe_greenhouse()]));
    let svc = DeduplicationService::new(group_repo.clone(), consol_repo.clone(), job_repo);

    let report = svc.run_pass(&[job_a_id, job_b_id]).await.unwrap();

    assert_eq!(report.total_groups, 1);
    // auto_merge_threshold default=0.97; description hash matches → auto merged
    assert_eq!(report.auto_merged, 1);
    let pending = group_repo.list_pending().await.unwrap();
    assert_eq!(pending.len(), 0);
}
```

### TUI Tests

- Render `DedupReviewView` with one mock pending group in a 80×24 `TestBackend` — assert the
  rendered output contains "Pending Review" and the group's similarity percentage.
- Simulate `m` keypress → `DuplicateReviewService::merge_group()` is called once.
- Simulate `d` keypress → `confirm_distinct()` is called once.

## Open Questions

1. **Transitive groups**: If A≈B (score 0.90) and B≈C (score 0.90) but A≈C (score 0.82), should
   all three be in one group? The current greedy single-pass algorithm places A,B in a group and
   then C is unassigned (because A and B are already `assigned`). Phase 3+ could use union-find
   for transitive closure, at the cost of O(n² log n) for large batches.

2. **Periodic re-scan**: The spec mentions a "periodic re-check for existing jobs". Phase 1 only
   deduplicates newly ingested jobs. A full re-scan ralph loop (daily, low priority) is needed to
   catch cases where two jobs were ingested weeks apart from different sources.

3. **Symmetric deduplication with existing groups**: If job A is already in group G, and a new job
   B is ingested that matches A, the current implementation will not find it (because A has
   `dedup_group_id != None` and is filtered out). Phase 2 should add a query: "find existing group
   whose `primary_job_id` matches the new job's fingerprint" and extend the group.

4. **Application re-linking after merge**: When the user merges a group that contains a job with
   an existing application, the application should be linked to the new `ConsolidatedJob`. The
   current plan leaves `application_id` as `None` at merge time. Phase 3 should query
   `applications WHERE job_id IN (group.job_ids)` and link the first found.

5. **Source priority list**: The current `SourcePriority` ordering (DirectCareers=0, Greenhouse=1,
   Lever=2, LinkedIn=3) is different from the spec's ordering (LinkedIn first). The spec justifies
   LinkedIn as having "best job descriptions" for display, while direct careers pages have "most
   accurate job details". This should be resolved with a product decision before Phase 1 ships.

## Related Specs

- `specs/XX-application-cross-source-deduplication.md` — source spec
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobIngestionService` hook point
- `specs/application-state-machine-implementation-plan.md` — `ApplicationId` type; `ApplyWorkflow` hook
- `specs/application-workflow-actions-implementation-plan.md` — `ApplyWorkflow::execute` modification
- `specs/11-platform-api-integrations-implementation-plan.md` — `GreenhouseClient`, `LeverClient` source names
