# Implementation Plan: Ghost Job Detection

## Status
Draft

## Related Spec
`specs/job-search-ghost-job-detection.md`

## Overview

The Ghost Job Detection subsystem assigns a `ghost_score: f32 ∈ [0.0, 1.0]` to every
discovered job listing using eight locally-computable heuristic signals. Scores are
computed at discovery time, refreshed daily for active jobs, and immediately recalculated
when a user overrides a classification. This is a purely local, offline subsystem — no
external API calls are required.

The system is built around two components: a stateless `GhostSignalExtractor` that
derives intermediate signal values from a `DiscoveredJob` + `PostingHistory` pair, and a
`GhostDetector` struct that aggregates signals into a final score using the weighted-sum
formula defined in the spec. The `GhostBadge` enum translates float scores into display
categories consumed by the TUI feed widget. A `GhostDetectionService` orchestrator handles
batch scoring and the daily refresh loop.

This plan extends the `lazyjob-core` crate. It reads from the `jobs` table written by the
discovery engine (see `specs/job-search-discovery-engine-implementation-plan.md`) and adds
a new `job_postings_history` table. It does not duplicate the enrichment pipeline — ghost
scoring runs after enrichment as a separate pass. The `FeedRanker` (semantic matching plan)
incorporates `ghost_score` into its final sort key as `feed_score = match_score *
(1 - ghost_score) * ...`.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, migration runner
- `specs/job-search-discovery-engine-implementation-plan.md` — `DiscoveredJob`,
  `JobRepository::upsert`, enrichment pipeline, `EnrichedJob`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml (additions only — most deps already present)

[dependencies]
regex       = "1"        # technical term lexicon for description_vagueness
once_cell   = "1"        # Lazy<Regex> patterns compiled once at startup
phf         = { version = "0.11", features = ["macros"] }  # perfect hash for jurisdiction set
# All other deps (uuid, chrono, thiserror, anyhow, sqlx, tracing) already present
```

## Architecture

### Crate Placement

All ghost detection code lives in `lazyjob-core/src/discovery/ghost_detection/`.
It is imported by:
- `lazyjob-ralph` — runs `GhostDetectionService::score_new_jobs()` at the end of each
  discovery loop run (after enrichment, before the TUI is notified)
- `lazyjob-ralph` — a separate scheduled daily-refresh loop calls
  `GhostDetectionService::rescore_active_jobs()`
- `lazyjob-tui` — reads `ghost_score` and `ghost_overridden` from the `jobs` table via
  `JobRepository` (no direct coupling to `GhostDetector`)
- `lazyjob-core/src/discovery/workflow.rs` (in the application workflow actions plan) —
  `ApplyWorkflow` queries ghost score before letting a user apply

### Core Types

```rust
// lazyjob-core/src/discovery/ghost_detection/types.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Intermediate signal values extracted from a job + its posting history.
/// Each field is a normalized scalar in [0.0, 1.0] unless otherwise noted.
#[derive(Debug, Clone)]
pub struct GhostSignals {
    /// Contribution from job age (days since posted). Logarithmically scaled.
    pub days_since_posted_score: f32,

    /// Contribution from lack of updates (days_since_updated == 0 → 0.0, else 1.0 if never touched).
    pub days_since_updated_score: f32,

    /// Contribution from repost count. 0 reposts → 0.0; ≥ 3 reposts → 1.0.
    pub repost_count_score: f32,

    /// Contribution from description vagueness. 0 = rich JD; 1.0 = pure buzzwords.
    pub description_vagueness_score: f32,

    /// 1.0 if job is in a pay-transparency jurisdiction and has no salary range; else 0.0.
    pub salary_absent_score: f32,

    /// 1.0 if no recruiter/team/hiring manager name detected in description; else 0.0.
    pub no_named_contact_score: f32,

    /// 1.0 if company has layoffs in the last 90 days and posts aggressively; else 0.0.
    /// Requires optional `LayoffsRepository`.
    pub company_headcount_declining_score: f32,

    /// Contribution from posting_age × repost ratio.
    pub posting_age_repost_ratio_score: f32,
}

impl GhostSignals {
    /// Signal weights matching the spec table. Verified to sum to 1.0.
    const WEIGHTS: [f32; 8] = [0.20, 0.15, 0.20, 0.15, 0.10, 0.05, 0.10, 0.05];

    /// Weighted-sum ghost score ∈ [0.0, 1.0].
    pub fn compute_score(&self) -> f32 {
        let scores = [
            self.days_since_posted_score,
            self.days_since_updated_score,
            self.repost_count_score,
            self.description_vagueness_score,
            self.salary_absent_score,
            self.no_named_contact_score,
            self.company_headcount_declining_score,
            self.posting_age_repost_ratio_score,
        ];
        let raw: f32 = scores.iter().zip(Self::WEIGHTS.iter()).map(|(s, w)| s * w).sum();
        raw.clamp(0.0, 1.0)
    }

    /// Human-readable explanations for TUI tooltip (highest-contributing signals first).
    pub fn explain(&self) -> Vec<SignalExplanation> {
        let mut contributions = vec![
            SignalExplanation {
                signal: "Age (days since posted)",
                score: self.days_since_posted_score,
                weight: 0.20,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.days_since_posted_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "Never updated",
                score: self.days_since_updated_score,
                weight: 0.15,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.days_since_updated_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "Repost count",
                score: self.repost_count_score,
                weight: 0.20,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.repost_count_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "Vague description",
                score: self.description_vagueness_score,
                weight: 0.15,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.description_vagueness_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "No salary (pay transparency law applies)",
                score: self.salary_absent_score,
                weight: 0.10,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.salary_absent_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "No named contact",
                score: self.no_named_contact_score,
                weight: 0.05,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.no_named_contact_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "Company with recent layoffs posting aggressively",
                score: self.company_headcount_declining_score,
                weight: 0.10,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.company_headcount_declining_score * 100.0
                ),
            },
            SignalExplanation {
                signal: "Posting age/repost ratio",
                score: self.posting_age_repost_ratio_score,
                weight: 0.05,
                message: format!(
                    "Contributes {:.0}% to ghost score",
                    self.posting_age_repost_ratio_score * 100.0
                ),
            },
        ];
        // Sort by weighted contribution descending
        contributions.sort_by(|a, b| {
            (b.score * b.weight)
                .partial_cmp(&(a.score * a.weight))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Filter out zero contributors
        contributions.retain(|e| e.score * e.weight > 0.01);
        contributions
    }
}

#[derive(Debug, Clone)]
pub struct SignalExplanation {
    pub signal: &'static str,
    pub score: f32,
    pub weight: f32,
    pub message: String,
}

/// Historical posting record for a (company, title, location) triple.
#[derive(Debug, Clone)]
pub struct PostingHistory {
    /// Number of times this (company_normalized, title_normalized, location_normalized)
    /// triple has appeared in distinct discovery runs.
    pub repost_count: u32,
    /// First time this triple was seen in any discovery run.
    pub first_seen_at: DateTime<Utc>,
    /// Most recent time this triple was seen.
    pub last_seen_at: DateTime<Utc>,
    /// How many times the raw listing content changed (description hash changed).
    pub update_count: u32,
}

/// TUI display category derived from the float score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhostBadge {
    /// score < 0.30 and not overridden — show normally.
    None,
    /// 0.30 ≤ score < 0.60 — yellow warning badge.
    PossiblyStale,
    /// score ≥ 0.60 — red ghost badge; deprioritized in feed.
    LikelyGhost,
    /// User explicitly marked as real; always show at normal priority.
    UserOverridden,
}

impl GhostBadge {
    pub fn from_score(score: f32, overridden: bool) -> Self {
        if overridden {
            return Self::UserOverridden;
        }
        if score >= 0.60 {
            Self::LikelyGhost
        } else if score >= 0.30 {
            Self::PossiblyStale
        } else {
            Self::None
        }
    }

    /// Short display string for the TUI badge column (max 4 chars).
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::PossiblyStale => "⚠ ",
            Self::LikelyGhost => "👻",
            Self::UserOverridden => "✓ ",
        }
    }

    /// ratatui style colour for the badge.
    pub fn style(self) -> ratatui::style::Style {
        use ratatui::style::{Color, Style};
        match self {
            Self::None => Style::default(),
            Self::PossiblyStale => Style::default().fg(Color::Yellow),
            Self::LikelyGhost => Style::default().fg(Color::Red),
            Self::UserOverridden => Style::default().fg(Color::Green),
        }
    }
}

/// Result of a batch scoring pass.
#[derive(Debug, Default)]
pub struct BatchScoringReport {
    pub scored: usize,
    pub skipped_overridden: usize,
    pub errors: Vec<(Uuid, String)>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/discovery/ghost_detection/repository.rs

use async_trait::async_trait;
use uuid::Uuid;
use crate::discovery::ghost_detection::types::PostingHistory;

#[async_trait]
pub trait GhostDetectionRepository: Send + Sync {
    /// Upsert a posting history record. Called on every discovery run for each job.
    /// Increments `repost_count` if the triple already exists, else inserts.
    async fn upsert_posting_history(
        &self,
        company_normalized: &str,
        title_normalized: &str,
        location_normalized: &str,
    ) -> Result<PostingHistory, GhostDetectionError>;

    /// Fetch posting history for a specific triple. Returns None if never seen before.
    async fn get_posting_history(
        &self,
        company_normalized: &str,
        title_normalized: &str,
        location_normalized: &str,
    ) -> Result<Option<PostingHistory>, GhostDetectionError>;

    /// Write ghost score and signals JSON to the jobs table.
    async fn save_ghost_score(
        &self,
        job_id: Uuid,
        score: f32,
        signals_json: &str,
    ) -> Result<(), GhostDetectionError>;

    /// Write user override flag.
    async fn set_ghost_override(
        &self,
        job_id: Uuid,
        overridden: bool,
    ) -> Result<(), GhostDetectionError>;

    /// List jobs needing daily rescore: not dismissed, ghost_score IS NOT NULL, not overridden.
    async fn list_jobs_for_rescore(&self) -> Result<Vec<Uuid>, GhostDetectionError>;

    /// Count how many active postings a company has in the last 7 days
    /// (used to detect "posting aggressively while laying off" signal).
    async fn count_recent_company_postings(
        &self,
        company_normalized: &str,
        since_days: u32,
    ) -> Result<u32, GhostDetectionError>;
}

/// Optional: pluggable layoffs data source.
#[async_trait]
pub trait LayoffsRepository: Send + Sync {
    /// Returns true if the company had a recorded layoff event in the past `days` days.
    async fn had_layoff(
        &self,
        company_normalized: &str,
        within_days: u32,
    ) -> Result<bool, GhostDetectionError>;
}
```

### GhostDetector (core logic)

```rust
// lazyjob-core/src/discovery/ghost_detection/detector.rs

use once_cell::sync::Lazy;
use regex::Regex;
use std::{collections::HashSet, sync::Arc};

use crate::discovery::models::EnrichedJob;
use super::types::{GhostSignals, PostingHistory};
use super::repository::LayoffsRepository;

/// Stateless classifier. All state it holds is read-only reference data
/// (jurisdiction set, regex patterns). Safe to clone and share across threads.
pub struct GhostDetector {
    /// Normalized jurisdiction strings that have pay-transparency laws.
    transparency_jurisdictions: HashSet<String>,
    /// Optional layoffs data source.
    layoffs_repo: Option<Arc<dyn LayoffsRepository>>,
    /// Recent posting threshold for "aggressive posting" signal.
    aggressive_posting_threshold: u32,
}

/// Technical term patterns used in description_vagueness scoring.
/// Built once at process startup; avoids repeated regex compilation.
static TECH_TERM_PATTERNS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?xi)
        \b(
            # Programming languages
            rust|python|golang|go\b|java\b|kotlin|swift|typescript|javascript|
            c\+\+|c\#|scala|elixir|haskell|ruby|php|perl|clojure|erlang|
            # Frameworks
            react|vue|angular|next\.?js|svelte|django|flask|fastapi|rails|
            spring|axum|actix|rocket|express|laravel|symfony|
            # Databases
            postgres|postgresql|mysql|sqlite|mongodb|redis|elasticsearch|
            cassandra|dynamodb|bigquery|snowflake|
            # Cloud & infra
            aws|gcp|azure|kubernetes|k8s|docker|terraform|helm|
            # Tools
            kafka|rabbitmq|grpc|graphql|rest|openapi|
            # General data/ML
            pytorch|tensorflow|spark|flink|dbt|airflow|pandas|numpy
        )\b",
    )
    .expect("TECH_TERM_PATTERNS regex is valid")
});

static SPECIFICITY_PATTERNS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?xi)
        # Team/project/manager mentions
        (we\s+use|our\s+stack|our\s+team|you'll\s+work\s+with|hiring\s+manager|
         reporting\s+to|team\s+of\s+\d|squad|chapter|tribe|pod\b|
         # Budget/scope
         \$\d+[km]?\s+budget|series\s+[a-c]|
         # Named tools with context
         migrating\s+to|built\s+on|powered\s+by)",
    )
    .expect("SPECIFICITY_PATTERNS regex is valid")
});

static NAMED_CONTACT_PATTERNS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?xi)
        # Recruiter name patterns
        (contact\s+[A-Z][a-z]+|reach\s+out\s+to\s+[A-Z]|
         # Team lead mention
         led\s+by|managed\s+by|head\s+of\s+[A-Z]|
         # Hiring manager
         hiring\s+manager\s+is|your\s+future\s+manager)",
    )
    .expect("NAMED_CONTACT_PATTERNS regex is valid")
});

impl GhostDetector {
    pub fn new() -> Self {
        Self {
            transparency_jurisdictions: build_transparency_jurisdictions(),
            layoffs_repo: None,
            aggressive_posting_threshold: 10,
        }
    }

    pub fn with_layoffs_db(mut self, repo: Arc<dyn LayoffsRepository>) -> Self {
        self.layoffs_repo = Some(repo);
        self
    }

    /// Extract all signal values for a single job.
    /// `now` is passed explicitly so tests can freeze time.
    pub fn extract_signals(
        &self,
        job: &EnrichedJob,
        history: &PostingHistory,
        recent_posting_count: u32,
        now: chrono::DateTime<chrono::Utc>,
    ) -> GhostSignals {
        GhostSignals {
            days_since_posted_score: self.score_days_since_posted(job, now),
            days_since_updated_score: self.score_days_since_updated(job, history, now),
            repost_count_score: self.score_repost_count(history),
            description_vagueness_score: self.score_description_vagueness(&job.description_text),
            salary_absent_score: self.score_salary_absent(job),
            no_named_contact_score: self.score_no_named_contact(&job.description_text),
            company_headcount_declining_score: 0.0, // filled in async path
            posting_age_repost_ratio_score: self.score_posting_age_repost_ratio(history, now),
        }
    }

    /// Async variant that also queries the layoffs repo.
    pub async fn extract_signals_async(
        &self,
        job: &EnrichedJob,
        history: &PostingHistory,
        recent_posting_count: u32,
        now: chrono::DateTime<chrono::Utc>,
    ) -> GhostSignals {
        let mut signals = self.extract_signals(job, history, recent_posting_count, now);
        signals.company_headcount_declining_score = self
            .score_company_headcount_declining(&job.company_name_normalized, recent_posting_count)
            .await;
        signals
    }

    // --- Individual signal scorers ---

    fn score_days_since_posted(
        &self,
        job: &EnrichedJob,
        now: chrono::DateTime<chrono::Utc>,
    ) -> f32 {
        let days = (now - job.posted_at).num_days().max(0) as f32;
        // Logarithmic: 0 days → 0.0, 30 days → 0.75, 60 days → 1.0
        (days + 1.0_f32).ln() / 61.0_f32.ln()
    }

    fn score_days_since_updated(
        &self,
        job: &EnrichedJob,
        history: &PostingHistory,
        now: chrono::DateTime<chrono::Utc>,
    ) -> f32 {
        // If the listing was never updated (update_count == 0) and it's > 14 days old, score 1.0.
        // Otherwise scale by how long ago the last update was.
        if history.update_count == 0 {
            let days_old = (now - job.posted_at).num_days().max(0) as f32;
            if days_old > 14.0 {
                return 1.0;
            }
        }
        let days_since = (now - history.last_seen_at).num_days().max(0) as f32;
        (days_since / 60.0).clamp(0.0, 1.0)
    }

    fn score_repost_count(&self, history: &PostingHistory) -> f32 {
        // 0 reposts → 0.0, 1 repost → 0.33, 2 reposts → 0.67, 3+ → 1.0
        (history.repost_count as f32 / 3.0).clamp(0.0, 1.0)
    }

    fn score_description_vagueness(&self, description: &str) -> f32 {
        let skill_tokens = TECH_TERM_PATTERNS.find_iter(description).count();
        let specificity_markers = SPECIFICITY_PATTERNS.find_iter(description).count();
        let total = (skill_tokens + specificity_markers) as f32;
        (1.0 - (total / 8.0)).clamp(0.0, 1.0)
    }

    fn score_salary_absent(&self, job: &EnrichedJob) -> f32 {
        let has_salary = job.salary_min.is_some() || job.salary_max.is_some();
        if has_salary {
            return 0.0;
        }
        // Check if any jurisdiction token matches known pay-transparency states
        let location_lower = job.location.to_lowercase();
        for jurisdiction in &self.transparency_jurisdictions {
            if location_lower.contains(jurisdiction.as_str()) {
                return 1.0;
            }
        }
        0.0
    }

    fn score_no_named_contact(&self, description: &str) -> f32 {
        if NAMED_CONTACT_PATTERNS.is_match(description) {
            0.0
        } else {
            1.0
        }
    }

    fn score_posting_age_repost_ratio(
        &self,
        history: &PostingHistory,
        now: chrono::DateTime<chrono::Utc>,
    ) -> f32 {
        let window_days = (now - history.first_seen_at).num_days().max(1) as f32;
        // ratio: reposts per 30 days, capped at 1.0 when repost_rate ≥ 3 per 30 days
        let rate_per_30_days = (history.repost_count as f32 / window_days) * 30.0;
        (rate_per_30_days / 3.0).clamp(0.0, 1.0)
    }

    async fn score_company_headcount_declining(
        &self,
        company_normalized: &str,
        recent_posting_count: u32,
    ) -> f32 {
        let Some(repo) = &self.layoffs_repo else {
            return 0.0; // feature not configured
        };
        let had_layoff = repo
            .had_layoff(company_normalized, 90)
            .await
            .unwrap_or(false);
        if had_layoff && recent_posting_count >= self.aggressive_posting_threshold {
            1.0
        } else {
            0.0
        }
    }
}

/// Static jurisdiction set embedded at compile time.
/// Normalized: lowercase, no punctuation, canonical city/state/country names.
fn build_transparency_jurisdictions() -> HashSet<String> {
    // As of 2026. Updated with each binary release.
    [
        // US states
        "california", "ca", "new york", "ny", "colorado", "co",
        "washington", "wa", "illinois", "il", "new jersey", "nj",
        "massachusetts", "ma", "nevada", "nv", "maryland", "md",
        // US cities
        "new york city", "nyc", "los angeles", "seattle", "chicago",
        "denver", "jersey city",
        // International
        "united kingdom", "uk", "england", "eu", "european union",
        "germany", "france", "netherlands", "spain", "sweden",
        "norway", "denmark", "austria", "belgium", "ireland",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
```

### SQLite Schema

```sql
-- Migration 014: ghost_job_detection
-- Applied by lazyjob-core/migrations/014_ghost_job_detection.sql

-- Add ghost detection columns to jobs table.
ALTER TABLE jobs ADD COLUMN ghost_score REAL;
ALTER TABLE jobs ADD COLUMN ghost_overridden BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE jobs ADD COLUMN ghost_signals_json TEXT;
ALTER TABLE jobs ADD COLUMN ghost_scored_at TEXT; -- ISO-8601 UTC

-- Partial index for efficient daily rescore query.
CREATE INDEX idx_jobs_needs_rescore
    ON jobs(ghost_scored_at)
    WHERE ghost_overridden = FALSE
      AND status != 'Dismissed';

-- Posting history table for repost tracking.
CREATE TABLE job_postings_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Composite key (normalized).
    company_normalized TEXT NOT NULL,
    title_normalized   TEXT NOT NULL,
    location_normalized TEXT NOT NULL,

    -- Tracking counters.
    repost_count  INTEGER NOT NULL DEFAULT 0,
    update_count  INTEGER NOT NULL DEFAULT 0,

    first_seen_at TEXT NOT NULL,  -- ISO-8601 UTC
    last_seen_at  TEXT NOT NULL,  -- ISO-8601 UTC

    UNIQUE(company_normalized, title_normalized, location_normalized)
);

CREATE INDEX idx_job_postings_history_company
    ON job_postings_history(company_normalized);

-- Optional: layoffs database table (Phase 2, populated by ralph loop).
CREATE TABLE IF NOT EXISTS layoff_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    company_normalized  TEXT NOT NULL,
    event_date          TEXT NOT NULL,  -- ISO-8601 date
    affected_count      INTEGER,        -- may be NULL if not reported
    source_url          TEXT,
    fetched_at          TEXT NOT NULL,
    UNIQUE(company_normalized, event_date)
);

CREATE INDEX idx_layoff_events_company_date
    ON layoff_events(company_normalized, event_date);
```

### Module Structure

```
lazyjob-core/
  migrations/
    014_ghost_job_detection.sql
  src/
    discovery/
      ghost_detection/
        mod.rs            ← pub re-exports: GhostDetector, GhostBadge, GhostSignals,
                             GhostDetectionService, SqliteGhostDetectionRepository
        types.rs          ← GhostSignals, PostingHistory, GhostBadge, BatchScoringReport
        detector.rs       ← GhostDetector, signal scoring functions, jurisdiction set
        repository.rs     ← GhostDetectionRepository trait, LayoffsRepository trait
        sqlite_repo.rs    ← SqliteGhostDetectionRepository (sqlx impl)
        service.rs        ← GhostDetectionService (orchestration)
        normalize.rs      ← normalize_company(), normalize_title(), normalize_location()
```

## Implementation Phases

### Phase 1 — Core Classifier (MVP)

**Goal**: `GhostDetector::extract_signals()` + weighted sum computes a score for any
enriched job using 6 of the 8 signals (excluding layoffs; excluding the async path).

#### Step 1.1 — Migration 014

Create `lazyjob-core/migrations/014_ghost_job_detection.sql` with the schema above.
Register it in the `Database::run_migrations()` migration runner (see persistence plan).

Verification:
```bash
cargo test -p lazyjob-core ghost_detection::tests::migration_applies
```

#### Step 1.2 — Normalization helpers

Create `normalize.rs`:

```rust
// lazyjob-core/src/discovery/ghost_detection/normalize.rs

/// Normalize a company name for dedup key usage.
/// Strips common suffixes (Inc, LLC, Ltd, Corp), lowercases, collapses whitespace.
pub fn normalize_company(name: &str) -> String {
    let suffixes = [" inc", " llc", " ltd", " corp", " co", " gmbh", " bv", " ag"];
    let mut s = name.to_lowercase();
    for sfx in &suffixes {
        s = s.trim_end_matches(sfx).to_string();
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a job title. Strips seniority level suffixes for canonical grouping
/// (so "Senior Engineer" and "Staff Engineer" can still group separately by title).
pub fn normalize_title(title: &str) -> String {
    // Lowercase, strip special chars, collapse whitespace.
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize location: lowercase, strip punctuation, collapse whitespace.
pub fn normalize_location(location: &str) -> String {
    location
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == ',')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
```

Tests:
```rust
#[test]
fn test_normalize_company_strips_suffix() {
    assert_eq!(normalize_company("Acme Corp"), "acme");
    assert_eq!(normalize_company("Google LLC"), "google");
}

#[test]
fn test_normalize_title() {
    assert_eq!(normalize_title("Senior Software Engineer"), "senior software engineer");
    assert_eq!(normalize_title("ML/AI Researcher"), "ml ai researcher");
}
```

#### Step 1.3 — `GhostSignals` and `GhostBadge`

Create `types.rs` as described in Core Types above. Add unit tests for:
- `compute_score()` with all-zero signals → 0.0
- `compute_score()` with all-one signals → 1.0
- `from_score(0.25, false)` → `GhostBadge::None`
- `from_score(0.45, false)` → `GhostBadge::PossiblyStale`
- `from_score(0.65, false)` → `GhostBadge::LikelyGhost`
- `from_score(0.65, true)` → `GhostBadge::UserOverridden`

#### Step 1.4 — `GhostDetector` (sync signals only)

Implement `detector.rs` with all 7 sync signal scorers.

Key API usages:
- `once_cell::sync::Lazy<Regex>` for `TECH_TERM_PATTERNS`, `SPECIFICITY_PATTERNS`,
  `NAMED_CONTACT_PATTERNS`
- `regex::Regex::find_iter(text).count()` for counting matches
- `chrono::Duration::num_days()` for age calculations

Unit tests with frozen `now`:
```rust
#[test]
fn test_description_vagueness_rich_jd() {
    let detector = GhostDetector::new();
    // A JD with Rust, postgres, kubernetes, Docker, AWS, Redis, kafka, terraform → 8+ terms → 0.0
    let score = detector.score_description_vagueness(RICH_JD_FIXTURE);
    assert!(score < 0.1, "Rich JD should have low vagueness: {score}");
}

#[test]
fn test_description_vagueness_buzzword_jd() {
    let detector = GhostDetector::new();
    let score = detector.score_description_vagueness(
        "Passionate innovator. Growth mindset. Competitive compensation. Dynamic team."
    );
    assert!(score > 0.9, "Buzzword JD should have high vagueness: {score}");
}

#[test]
fn test_salary_absent_in_ca() {
    let mut job = fixture_job();
    job.salary_min = None;
    job.salary_max = None;
    job.location = "San Francisco, CA".to_string();
    let score = GhostDetector::new().score_salary_absent(&job);
    assert_eq!(score, 1.0);
}

#[test]
fn test_salary_absent_non_transparency_state() {
    let mut job = fixture_job();
    job.salary_min = None;
    job.location = "Austin, TX".to_string();
    let score = GhostDetector::new().score_salary_absent(&job);
    assert_eq!(score, 0.0);
}
```

#### Step 1.5 — `SqliteGhostDetectionRepository`

```rust
// lazyjob-core/src/discovery/ghost_detection/sqlite_repo.rs

use sqlx::SqlitePool;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

pub struct SqliteGhostDetectionRepository {
    pool: SqlitePool,
}

impl SqliteGhostDetectionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GhostDetectionRepository for SqliteGhostDetectionRepository {
    async fn upsert_posting_history(
        &self,
        company_normalized: &str,
        title_normalized: &str,
        location_normalized: &str,
    ) -> Result<PostingHistory, GhostDetectionError> {
        // Use INSERT OR IGNORE then UPDATE to atomically increment repost_count.
        // SQLite `ON CONFLICT(...)` with DO UPDATE is the cleanest approach.
        let now = Utc::now().to_rfc3339();
        sqlx::query!(
            r#"
            INSERT INTO job_postings_history
                (company_normalized, title_normalized, location_normalized,
                 repost_count, update_count, first_seen_at, last_seen_at)
            VALUES (?, ?, ?, 0, 0, ?, ?)
            ON CONFLICT(company_normalized, title_normalized, location_normalized)
            DO UPDATE SET
                repost_count = repost_count + 1,
                last_seen_at = excluded.last_seen_at
            "#,
            company_normalized,
            title_normalized,
            location_normalized,
            now,
            now,
        )
        .execute(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;

        // Fetch updated record
        let row = sqlx::query!(
            r#"
            SELECT repost_count, update_count, first_seen_at, last_seen_at
            FROM job_postings_history
            WHERE company_normalized = ? AND title_normalized = ? AND location_normalized = ?
            "#,
            company_normalized,
            title_normalized,
            location_normalized,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;

        Ok(PostingHistory {
            repost_count: row.repost_count as u32,
            update_count: row.update_count as u32,
            first_seen_at: DateTime::parse_from_rfc3339(&row.first_seen_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| GhostDetectionError::DataParsing(e.to_string()))?,
            last_seen_at: DateTime::parse_from_rfc3339(&row.last_seen_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| GhostDetectionError::DataParsing(e.to_string()))?,
        })
    }

    async fn save_ghost_score(
        &self,
        job_id: Uuid,
        score: f32,
        signals_json: &str,
    ) -> Result<(), GhostDetectionError> {
        let job_id_str = job_id.to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query!(
            r#"
            UPDATE jobs
            SET ghost_score = ?,
                ghost_signals_json = ?,
                ghost_scored_at = ?
            WHERE id = ?
            "#,
            score,
            signals_json,
            now,
            job_id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;
        Ok(())
    }

    async fn set_ghost_override(
        &self,
        job_id: Uuid,
        overridden: bool,
    ) -> Result<(), GhostDetectionError> {
        let job_id_str = job_id.to_string();
        sqlx::query!(
            "UPDATE jobs SET ghost_overridden = ? WHERE id = ?",
            overridden,
            job_id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;
        Ok(())
    }

    async fn list_jobs_for_rescore(&self) -> Result<Vec<Uuid>, GhostDetectionError> {
        let rows = sqlx::query!(
            r#"
            SELECT id FROM jobs
            WHERE ghost_overridden = FALSE
              AND status != 'Dismissed'
              AND ghost_scored_at IS NOT NULL
            ORDER BY ghost_scored_at ASC
            LIMIT 500
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;

        rows.iter()
            .map(|r| {
                Uuid::parse_str(&r.id)
                    .map_err(|e| GhostDetectionError::DataParsing(e.to_string()))
            })
            .collect()
    }

    async fn count_recent_company_postings(
        &self,
        company_normalized: &str,
        since_days: u32,
    ) -> Result<u32, GhostDetectionError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count FROM jobs
            WHERE company_name_normalized = ?
              AND discovered_at >= datetime('now', '-' || ? || ' days')
            "#,
            company_normalized,
            since_days,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;
        Ok(row.count as u32)
    }

    async fn get_posting_history(
        &self,
        company_normalized: &str,
        title_normalized: &str,
        location_normalized: &str,
    ) -> Result<Option<PostingHistory>, GhostDetectionError> {
        let row = sqlx::query!(
            r#"
            SELECT repost_count, update_count, first_seen_at, last_seen_at
            FROM job_postings_history
            WHERE company_normalized = ? AND title_normalized = ? AND location_normalized = ?
            "#,
            company_normalized,
            title_normalized,
            location_normalized,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;

        let Some(row) = row else { return Ok(None) };
        Ok(Some(PostingHistory {
            repost_count: row.repost_count as u32,
            update_count: row.update_count as u32,
            first_seen_at: DateTime::parse_from_rfc3339(&row.first_seen_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| GhostDetectionError::DataParsing(e.to_string()))?,
            last_seen_at: DateTime::parse_from_rfc3339(&row.last_seen_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| GhostDetectionError::DataParsing(e.to_string()))?,
        }))
    }
}
```

Verification: `#[sqlx::test(migrations = "migrations")]` tests confirming upsert
increments `repost_count` on second call for the same triple.

#### Step 1.6 — `GhostDetectionService`

```rust
// lazyjob-core/src/discovery/ghost_detection/service.rs

use std::sync::Arc;
use chrono::Utc;
use tracing::{info, warn};

use crate::discovery::models::EnrichedJob;
use super::{
    detector::GhostDetector,
    normalize::{normalize_company, normalize_title, normalize_location},
    repository::GhostDetectionRepository,
    types::BatchScoringReport,
};

pub struct GhostDetectionService {
    detector: GhostDetector,
    repo: Arc<dyn GhostDetectionRepository>,
}

impl GhostDetectionService {
    pub fn new(detector: GhostDetector, repo: Arc<dyn GhostDetectionRepository>) -> Self {
        Self { detector, repo }
    }

    /// Score a batch of newly discovered jobs. Called at the end of each discovery run.
    /// Returns a report of how many jobs were scored and any per-job errors.
    #[tracing::instrument(skip(self, jobs), fields(job_count = jobs.len()))]
    pub async fn score_new_jobs(
        &self,
        jobs: &[EnrichedJob],
    ) -> BatchScoringReport {
        let mut report = BatchScoringReport::default();
        let now = Utc::now();

        for job in jobs {
            let company_n = normalize_company(&job.company_name);
            let title_n = normalize_title(&job.title);
            let location_n = normalize_location(&job.location);

            // Upsert posting history (increments repost_count).
            let history = match self.repo.upsert_posting_history(&company_n, &title_n, &location_n).await {
                Ok(h) => h,
                Err(e) => {
                    warn!(job_id = %job.id, error = %e, "Failed to upsert posting history");
                    report.errors.push((job.id, e.to_string()));
                    continue;
                }
            };

            let recent_count = self
                .repo
                .count_recent_company_postings(&company_n, 7)
                .await
                .unwrap_or(0);

            let signals = self
                .detector
                .extract_signals_async(job, &history, recent_count, now)
                .await;

            let score = signals.compute_score();
            let signals_json = serde_json::to_string(&signals).unwrap_or_default();

            if let Err(e) = self.repo.save_ghost_score(job.id, score, &signals_json).await {
                warn!(job_id = %job.id, error = %e, "Failed to save ghost score");
                report.errors.push((job.id, e.to_string()));
                continue;
            }

            info!(
                job_id = %job.id,
                ghost_score = score,
                "Ghost score computed"
            );
            report.scored += 1;
        }

        report
    }

    /// Daily rescore of all active, non-overridden jobs.
    /// Called by the ralph scheduled loop (`LoopType::GhostRescore`).
    #[tracing::instrument(skip(self, job_repo), fields())]
    pub async fn rescore_active_jobs(
        &self,
        job_repo: &dyn crate::persistence::jobs::JobRepository,
    ) -> BatchScoringReport {
        let mut report = BatchScoringReport::default();
        let job_ids = match self.repo.list_jobs_for_rescore().await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(error = %e, "Failed to list jobs for rescore");
                return report;
            }
        };

        info!(jobs_to_rescore = job_ids.len(), "Starting daily ghost rescore");
        let now = Utc::now();

        for job_id in job_ids {
            let job = match job_repo.get_by_id(job_id).await {
                Ok(Some(j)) => j,
                Ok(None) => continue, // deleted between list and fetch
                Err(e) => {
                    report.errors.push((job_id, e.to_string()));
                    continue;
                }
            };

            let company_n = normalize_company(&job.company_name);
            let title_n = normalize_title(&job.title);
            let location_n = normalize_location(&job.location);

            let history = match self
                .repo
                .get_posting_history(&company_n, &title_n, &location_n)
                .await
            {
                Ok(Some(h)) => h,
                Ok(None) => PostingHistory {
                    repost_count: 0,
                    update_count: 0,
                    first_seen_at: job.posted_at,
                    last_seen_at: job.posted_at,
                },
                Err(e) => {
                    report.errors.push((job_id, e.to_string()));
                    continue;
                }
            };

            let recent_count = self
                .repo
                .count_recent_company_postings(&company_n, 7)
                .await
                .unwrap_or(0);

            let signals = self
                .detector
                .extract_signals_async(&job, &history, recent_count, now)
                .await;

            let score = signals.compute_score();
            let signals_json = serde_json::to_string(&signals).unwrap_or_default();

            if let Err(e) = self.repo.save_ghost_score(job_id, score, &signals_json).await {
                report.errors.push((job_id, e.to_string()));
            } else {
                report.scored += 1;
            }
        }

        report
    }

    /// Called when user manually marks a job as real (override = true) or removes the override.
    pub async fn set_override(
        &self,
        job_id: uuid::Uuid,
        overridden: bool,
    ) -> Result<(), GhostDetectionError> {
        self.repo.set_ghost_override(job_id, overridden).await
    }
}
```

Verification: integration test using `#[sqlx::test]` that:
1. Creates two enriched jobs with the same `(company, title, location)` triple
2. Calls `score_new_jobs(&[job1])`, then `score_new_jobs(&[job2])`
3. Confirms `repost_count` is 2 after both runs
4. Confirms both jobs have `ghost_score` set

### Phase 2 — Layoffs Data Integration (Optional Feature)

**Goal**: Populate `layoff_events` table from `layoffs.fyi` public data. Gate behind
`[ghost_detection] layoffs_db = true` in `~/.config/lazyjob/config.toml`.

#### Step 2.1 — `SqliteLayoffsRepository`

```rust
pub struct SqliteLayoffsRepository {
    pool: SqlitePool,
}

#[async_trait]
impl LayoffsRepository for SqliteLayoffsRepository {
    async fn had_layoff(&self, company_normalized: &str, within_days: u32) -> Result<bool, GhostDetectionError> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) as count FROM layoff_events
            WHERE company_normalized = ?
              AND event_date >= date('now', '-' || ? || ' days')
            "#,
            company_normalized,
            within_days,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(GhostDetectionError::Database)?;
        Ok(row.count > 0)
    }
}
```

#### Step 2.2 — Layoffs Ralph Loop

Add `LoopType::LayoffsSync` to the orchestration layer. This loop:
1. Fetches the layoffs.fyi GitHub-hosted CSV
   (`https://raw.githubusercontent.com/roger-that-dev/layoffs-data/main/layoffs.csv`)
   using `reqwest`
2. Parses with `csv::Reader` (add `csv = "1"` to Cargo.toml)
3. Normalizes company name with `normalize_company()`
4. Upserts into `layoff_events`
5. Default schedule: weekly (`0 0 * * 0`)
6. Opt-in only: loop only registered if `[ghost_detection] layoffs_db = true` in config

```toml
# ~/.config/lazyjob/config.toml
[ghost_detection]
layoffs_db = false  # Set to true to enable layoffs.fyi sync (optional feature)
```

Verification: `#[sqlx::test]` — seed `layoff_events` with a row for "acme" dated 30 days
ago; confirm `had_layoff("acme", 90)` returns `true`.

### Phase 3 — TUI Integration

**Goal**: Display `GhostBadge` in the jobs feed list widget. Show a tooltip with the
explain output on `?` press.

#### Step 3.1 — Update `JobListItem` model

```rust
// lazyjob-tui/src/views/jobs_feed/types.rs

pub struct JobListItem {
    pub id: Uuid,
    pub title: String,
    pub company: String,
    pub location: String,
    pub posted_days_ago: i64,
    pub match_score: Option<f32>,
    pub ghost_badge: GhostBadge,   // ← new
    pub ghost_score: Option<f32>,  // ← new (for tooltip)
}
```

#### Step 3.2 — Feed list rendering

In `JobsFeedView::render_list()`, add the badge as a styled `Span` before the title:

```rust
fn render_job_row(item: &JobListItem) -> ratatui::text::Line {
    use ratatui::text::{Line, Span};

    let badge = Span::styled(
        item.ghost_badge.label(),
        item.ghost_badge.style(),
    );
    let title = Span::raw(format!(" {} — {}", item.title, item.company));
    Line::from(vec![badge, title])
}
```

#### Step 3.3 — Ghost tooltip popup

When the user presses `?` on a job with `ghost_score.is_some()`:

1. `App` transitions to `Modal::GhostExplanation { job_id, signals_json }`
2. Parse `signals_json` into `GhostSignals`, call `signals.explain()`
3. Render a `ratatui::widgets::Paragraph` inside a centered `Clear` overlay
4. Show each `SignalExplanation` with its weighted contribution
5. Dismiss on `Esc` or `q`

```rust
fn render_ghost_tooltip(frame: &mut Frame, area: Rect, signals: &GhostSignals, score: f32) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    use ratatui::layout::Alignment;

    let explanations = signals.explain();
    let text: Vec<String> = std::iter::once(format!("Ghost Score: {:.0}%", score * 100.0))
        .chain(explanations.iter().map(|e| {
            format!(
                "  {:<45} {:+.0}%",
                e.signal,
                e.score * e.weight * 100.0
            )
        }))
        .collect();

    let popup_area = centered_rect(60, (explanations.len() + 4) as u16, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(text.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Ghost Job Analysis"))
            .alignment(Alignment::Left),
        popup_area,
    );
}
```

#### Step 3.4 — Ghost override keybind

In Normal mode on the jobs list, add:

| Key | Action |
|---|---|
| `g` | Toggle `ghost_overridden` for selected job |
| `?` | Show ghost explanation tooltip |

`g` dispatches `Action::ToggleGhostOverride(job_id)` → `GhostDetectionService::set_override()`.

Verification: manual test — start TUI, open a ghost job, press `?`, confirm tooltip shows.
Press `g`, confirm badge switches to `UserOverridden` (green checkmark).

### Phase 4 — Daily Rescore Ralph Loop

**Goal**: Background daily rescore of all active jobs to catch listings that have aged
into ghost territory since discovery.

#### Step 4.1 — `LoopType::GhostRescore`

Add `GhostRescore` variant to the `LoopType` enum (see ralph orchestration plan).

```rust
impl LoopType {
    pub fn default_schedule(&self) -> Option<&'static str> {
        match self {
            Self::GhostRescore => Some("0 2 * * *"), // 2 AM daily
            // ...
        }
    }

    pub fn concurrency_limit(&self) -> usize {
        match self {
            Self::GhostRescore => 1, // Only one rescore at a time
            // ...
        }
    }
}
```

#### Step 4.2 — Ralph worker for GhostRescore

In `lazyjob-ralph/src/workers/ghost_rescore.rs`:

```rust
pub async fn run_ghost_rescore(
    service: Arc<GhostDetectionService>,
    job_repo: Arc<dyn JobRepository>,
    sender: broadcast::Sender<WorkerEvent>,
) -> anyhow::Result<()> {
    let report = service.rescore_active_jobs(job_repo.as_ref()).await;

    let _ = sender.send(WorkerEvent::Progress {
        message: format!(
            "Ghost rescore complete: {} scored, {} errors",
            report.scored,
            report.errors.len()
        ),
    });

    Ok(())
}
```

### Phase 5 — Feedback Loop and Weight Calibration (Future)

**Goal**: Collect user feedback on ghost classification accuracy and provide a mechanism
to tune weights over time.

#### Step 5.1 — `ghost_feedback` table

```sql
CREATE TABLE ghost_feedback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id      TEXT NOT NULL REFERENCES jobs(id),
    was_ghost   BOOLEAN NOT NULL,   -- user confirmed: was this actually a ghost?
    created_at  TEXT NOT NULL
);
```

#### Step 5.2 — Feedback collection in TUI

After applying to a job that was flagged as ghost, or after receiving a rejection from a
flagged listing, prompt:
> "Was this a ghost job? [y/N]"

Store response in `ghost_feedback`. This data is available for future weight tuning but
is not acted on in the MVP — the weights remain hardcoded at the spec values.

#### Step 5.3 — Accuracy report (CLI subcommand)

```
lazyjob ghost-accuracy
  True positive rate (correctly flagged ghosts): 72%
  False positive rate (real jobs flagged):       8%
  Feedback sample size: 47 jobs
```

Uses `ghost_feedback` table. Displayed as a separate CLI command, not in TUI.

## Key Crate APIs

| API | Usage |
|---|---|
| `once_cell::sync::Lazy<Regex>` | Compile tech term regex patterns once at startup |
| `regex::Regex::find_iter(text).count()` | Count technical term matches for vagueness |
| `chrono::DateTime<Utc>` arithmetic, `.num_days()` | Days since posted/updated |
| `sqlx::query!` macro with `ON CONFLICT ... DO UPDATE SET` | Atomic repost count increment |
| `sqlx::query!` with `datetime('now', '-N days')` | Time-windowed count queries |
| `serde_json::to_string(&signals)` | Serialize signals struct for storage |
| `serde_json::from_str::<GhostSignals>(&row.ghost_signals_json)` | Deserialize for tooltip |
| `ratatui::widgets::Clear` | Erase background before tooltip overlay |
| `ratatui::text::Span::styled(label, style)` | Colored ghost badge in list row |
| `#[sqlx::test(migrations = "migrations")]` | In-memory SQLite with migrations for tests |

## Error Handling

```rust
// lazyjob-core/src/discovery/ghost_detection/error.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GhostDetectionError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Data parsing error: {0}")]
    DataParsing(String),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Layoffs repository error: {0}")]
    LayoffsRepo(String),
}

pub type Result<T> = std::result::Result<T, GhostDetectionError>;
```

Design decisions:
- `Database` variant wraps `sqlx::Error` directly — callers can inspect if needed
- `DataParsing` is a `String` to avoid exposing `chrono::ParseError` in the public API
- Per-job errors in `score_new_jobs()` are collected into `BatchScoringReport.errors`,
  not propagated — one bad job should not abort the entire batch

## Testing Strategy

### Unit Tests

All in `lazyjob-core/src/discovery/ghost_detection/` with `#[cfg(test)]` modules:

1. **Signal scorers** — each of the 7 sync signal functions tested with:
   - Boundary values (0 days, 60 days, 61+ days for `days_since_posted`)
   - Non-transparency state job with no salary → score 0.0
   - Transparency state job with salary → score 0.0
   - Transparency state job without salary → score 1.0

2. **`GhostSignals::compute_score()`** — verified against hand-calculated values:
   - All signals at 0.5 → score ≈ 0.5
   - High repost_count + high vagueness + old age → score > 0.6

3. **`GhostBadge::from_score()`** — all four branches

4. **`normalize_*` functions** — edge cases: empty string, all-caps, Unicode company names

5. **Regex patterns** — `TECH_TERM_PATTERNS` matches `rust`, `kubernetes`, `postgresql`;
   does not match `rusticity`, `kubernete` (partial)

### Integration Tests

Using `#[sqlx::test(migrations = "migrations")]`:

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_upsert_posting_history_increments_repost_count(pool: SqlitePool) {
    let repo = SqliteGhostDetectionRepository::new(pool);

    // First discovery
    let h1 = repo.upsert_posting_history("acme", "engineer", "remote").await.unwrap();
    assert_eq!(h1.repost_count, 0);

    // Second discovery — same triple
    let h2 = repo.upsert_posting_history("acme", "engineer", "remote").await.unwrap();
    assert_eq!(h2.repost_count, 1);
}

#[sqlx::test(migrations = "migrations")]
async fn test_save_and_retrieve_ghost_score(pool: SqlitePool) {
    // Insert a job, then save ghost score, then confirm it's readable.
}

#[sqlx::test(migrations = "migrations")]
async fn test_list_jobs_for_rescore_excludes_overridden(pool: SqlitePool) {
    // Insert two jobs: one overridden, one not. Confirm only the non-overridden one is returned.
}
```

### End-to-End Tests

In `lazyjob-ralph/tests/ghost_detection_e2e.rs`:

1. Seed the SQLite DB with 20 jobs:
   - 5 old (90 days), vague, no salary in CA, 3+ reposts → expect LikelyGhost
   - 5 recent, specific JD, salary present, first posting → expect None
   - 5 borderline → expect PossiblyStale
   - 5 user-overridden → expect UserOverridden

2. Run `GhostDetectionService::score_new_jobs()`

3. Query `ghost_score`, `ghost_overridden` from DB and assert badge categories

4. Run `set_override(job_id, true)` on a LikelyGhost job, confirm badge flips to UserOverridden

### TUI Tests

TUI widget tests are out of scope for Phase 1 (no headless ratatui test driver). Covered
by manual testing checklist:

- [ ] Ghost badge renders on jobs with `ghost_score ≥ 0.30`
- [ ] `?` key opens tooltip with explanation list
- [ ] `g` toggles override; badge updates on next render cycle
- [ ] Ghost-overridden jobs show green `✓` badge
- [ ] Jobs with `ghost_score = NULL` show no badge

## Open Questions

1. **False positive rate target**: The spec asks for < 10% false positive rate but provides
   no labeled dataset to validate against. In Phase 5, collect user feedback to calibrate.
   The current weights are spec-prescribed heuristics, not empirically derived.

2. **Repost detection granularity**: Using `(company, title, location)` as the composite
   key means location variation (e.g., "New York" vs. "New York, NY" vs. "NYC") can
   break repost detection. `normalize_location()` partially mitigates this but won't catch
   all variations. Phase 4 could add fuzzy matching via `strsim::jaro_winkler` on the
   location component (threshold ≥ 0.88) before inserting a new history record vs.
   incrementing an existing one.

3. **layoffs.fyi scraping legality**: The layoffs.fyi ToS doesn't explicitly forbid
   automated fetching of public data, but the `roger-that-dev/layoffs-data` GitHub repo
   (community mirror) is a more stable, explicitly public data source. Defaulting to that
   repo's CSV avoids scraping layoffs.fyi directly. The feature is opt-in and off by default.

4. **Ghost notification on applied jobs**: When a job the user applied to crosses the
   `ghost_score ≥ 0.60` threshold on daily rescore, should the system emit a notification?
   Design decision: emit a `NotificationEvent::AppliedJobNowGhost { job_id }` that the
   `NotificationScheduler` delivers as a desktop notification. Default: opt-in via
   `[notifications] ghost_alert = true` config key.

5. **Weight configuration**: Should users be able to override signal weights in config?
   Phase 1: hardcoded. Phase 5: `[ghost_detection.weights]` TOML section parsed at startup
   and validated to sum to 1.0 within ±0.001; falls back to hardcoded defaults if invalid.

6. **`GhostSignals` serialization**: `serde_json::to_string(&signals)` requires
   `GhostSignals` to implement `serde::Serialize`. Since all fields are `f32`, this is
   trivial — add `#[derive(Serialize, Deserialize)]` to `GhostSignals`. The stored JSON
   is used only for tooltip reconstruction; its schema is not public API.

## Related Specs

- `specs/job-search-discovery-engine.md` — `DiscoveredJob`, `EnrichedJob`, `JobRepository`
- `specs/job-search-semantic-matching.md` — `FeedRanker` consumes `ghost_score` as a
  multiplicative factor in the final feed score formula
- `specs/09-tui-design-keybindings.md` — modal popup system used for ghost tooltip
- `specs/10-application-workflow.md` — `ApplyWorkflow` checks `ghost_score` before apply
- `specs/04-sqlite-persistence.md` — migration runner, `Database` struct
- `specs/agentic-ralph-orchestration.md` — `LoopType::GhostRescore` scheduling
