# Implementation Plan: Job Search Discovery Engine

## Status
Draft

## Related Spec
`specs/job-search-discovery-engine.md`

## Overview

The Job Discovery Engine is the primary data ingestion layer for LazyJob. It aggregates
job listings from multiple configured company boards (Greenhouse, Lever, and optionally
Adzuna), runs each raw listing through a five-step enrichment pipeline (HTML sanitization,
salary extraction, remote classification, location normalization, company linkage), and
persists deduplicated results to SQLite. It is consumed by the `lazyjob-ralph` subprocess
during discovery loop runs, and by the TUI for on-demand refresh.

This plan defines the orchestration layer — `DiscoveryService`, `CompanyRegistry`,
`JobSourceRegistry`, and `EnrichmentPipeline` — that sits on top of the low-level HTTP
clients described in `specs/11-platform-api-integrations-implementation-plan.md`. Where
that plan defines how to talk to Greenhouse's API, this plan defines how to coordinate
multiple sources, enrich results, and guarantee deduplication across sources and fetches.

The architecture is strictly local-first: all state lives in SQLite. The TUI reads from
SQLite via `JobRepository`; there is no direct coupling between the TUI and the discovery
network layer. Ralph subprocesses write to SQLite and exit; the TUI polls or is notified
of changes via a `tokio::sync::watch` channel.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations
- `specs/11-platform-api-integrations-implementation-plan.md` — `GreenhouseClient`, `LeverClient`, `PlatformHttpClient`, `RateLimiter`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
# Existing (from platform-api-integrations plan)
reqwest     = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip"] }
governor    = "0.6"
backoff     = { version = "0.4", features = ["tokio"] }
scraper     = "0.19"
keyring     = "2"
secrecy     = "0.8"
async-trait = "0.1"
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4"] }
tracing     = "0.1"
thiserror   = "2"
anyhow      = "1"
tokio       = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
futures     = "0.3"

# New for discovery engine
ammonia     = "4"            # HTML allowlist sanitizer (not html2text — safer)
regex       = "1"            # salary pattern extraction
once_cell   = "1"            # Lazy<Regex> patterns compiled once
toml        = "0.8"          # config.toml parsing
serde_yaml  = "0.9"          # life-sheet YAML (if needed for company linkage)
strsim      = "0.11"         # fuzzy title matching for cross-source dedup

[dev-dependencies]
wiremock    = "0.6"
sqlx        = { version = "0.8", features = ["sqlite", "runtime-tokio", "migrate", "macros"] }
tempfile    = "3"
tokio       = { version = "1", features = ["full"] }
```

## Architecture

### Crate Placement

All discovery engine code lives in `lazyjob-core/src/discovery/`. This crate is imported
by both `lazyjob-ralph` (for background discovery loop runs) and `lazyjob-tui` (for
on-demand refresh via keybind).

The HTTP source clients (`GreenhouseSource`, `LeverSource`, `AdzunaSource`) are sub-modules
within `discovery/sources/`. The `EnrichmentPipeline` is a pure stateless transform — it
takes a `RawJob` and produces an `EnrichedJob`. `DiscoveryService` owns orchestration.

`JobRepository` for reads (TUI job feed) lives in `lazyjob-core/src/persistence/jobs.rs`,
not in `discovery/`. The discovery engine only calls `upsert` on it.

### Core Types

```rust
// lazyjob-core/src/discovery/models.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Raw job record returned by every `JobSource` implementation before enrichment.
#[derive(Debug, Clone)]
pub struct RawJob {
    /// Platform identifier: "greenhouse", "lever", "adzuna".
    pub source: String,
    /// The platform's own opaque job ID — uniqueness scoped to source.
    pub source_id: String,
    pub title: String,
    pub company_name: String,
    pub location: Option<String>,
    pub url: String,
    /// Raw HTML description from the platform. May be None if the platform
    /// requires a second request to fetch detail.
    pub description_html: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    /// When the job was posted on the source platform.
    pub posted_at: Option<DateTime<Utc>>,
}

/// Enriched job record produced by `EnrichmentPipeline`. Ready for SQLite upsert.
#[derive(Debug, Clone)]
pub struct EnrichedJob {
    pub source: String,
    pub source_id: String,
    pub title: String,
    /// ASCII-lowercased, whitespace-collapsed title for cross-source dedup matching.
    pub title_normalized: String,
    pub company_name: String,
    /// Resolved FK to `companies.id`, set during company linkage step.
    pub company_id: Option<String>,
    pub location: Option<String>,
    /// Normalized location (e.g., "san francisco ca", "remote") for dedup matching.
    pub location_normalized: String,
    pub remote: RemoteType,
    pub url: String,
    /// Plain text description, ammonia-sanitized, max 65536 bytes.
    pub description: String,
    /// Extracted from description text. Stored as integer cents (USD).
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub salary_currency: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteType {
    Yes,
    No,
    Hybrid,
    Unknown,
}

impl RemoteType {
    /// Convert to TEXT for SQLite storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Hybrid => "hybrid",
            Self::Unknown => "unknown",
        }
    }
}

/// Aggregated result of a single `run_discovery()` invocation.
#[derive(Debug, Default)]
pub struct DiscoveryReport {
    /// Number of net-new jobs inserted into SQLite.
    pub new_jobs: usize,
    /// Number of existing jobs that had their content updated (title/description changed).
    pub updated: usize,
    /// Number of (source, source_id) pairs already present and unchanged.
    pub duplicates: usize,
    /// Number of cross-source duplicates soft-deleted.
    pub cross_source_deduped: usize,
    /// Per-source error messages. Non-fatal — other sources continue.
    pub errors: Vec<DiscoveryError>,
    pub duration_ms: u64,
}

/// Upsert outcome for a single job. Used internally by `JobRepository`.
#[derive(Debug, PartialEq)]
pub enum UpsertOutcome {
    Inserted,
    Updated,
    Unchanged,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/discovery/source.rs

use async_trait::async_trait;
use super::models::RawJob;
use super::error::DiscoveryError;

/// Uniform interface that every job-board source integration must implement.
/// Sources are registered in `JobSourceRegistry` and dispatched by `DiscoveryService`.
///
/// Implementations must be cheaply clonable (`Arc<dyn JobSource>` is idiomatic).
/// Each implementation holds its own `RateLimiter` and `PlatformHttpClient`.
#[async_trait]
pub trait JobSource: Send + Sync {
    /// Stable name for this source: "greenhouse", "lever", "adzuna".
    fn name(&self) -> &'static str;

    /// Fetch all currently-open job listings for the given company identifier.
    /// The meaning of `company_id` is source-specific:
    ///   - Greenhouse: board token (e.g., "stripe")
    ///   - Lever: company slug (e.g., "notion")
    ///   - Adzuna: keyword query prefix
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>, DiscoveryError>;

    /// Health check — returns true if the source endpoint is reachable.
    /// Called at startup to warn users of misconfigured sources.
    async fn health_check(&self) -> bool;
}

// lazyjob-core/src/discovery/repository.rs

use async_trait::async_trait;
use super::models::{EnrichedJob, UpsertOutcome};
use super::error::DiscoveryError;

/// Persistence interface for the discovery engine.
/// Implemented by `SqliteJobRepository` in `lazyjob-core/src/persistence/jobs.rs`.
#[async_trait]
pub trait JobRepository: Send + Sync {
    /// Insert or update a job record.
    /// Returns the upsert outcome and the assigned `job_id` (new UUID or existing).
    async fn upsert(
        &self,
        job: &EnrichedJob,
    ) -> Result<(UpsertOutcome, String), DiscoveryError>;

    /// Check if a (source, source_id) pair is already indexed.
    async fn find_by_source(
        &self,
        source: &str,
        source_id: &str,
    ) -> Result<Option<String>, DiscoveryError>; // returns job_id if found

    /// List all jobs with status 'new' for TUI feed rendering.
    async fn list_by_status(
        &self,
        status: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<crate::persistence::jobs::Job>, DiscoveryError>;

    /// Soft-delete a job (set status = 'deduped'). Used by cross-source dedup pass.
    async fn soft_delete(&self, job_id: &str) -> Result<(), DiscoveryError>;

    /// List candidate pairs for cross-source dedup matching.
    /// Returns all jobs grouped by (company_id, title_normalized, location_normalized).
    async fn list_cross_source_candidates(
        &self,
    ) -> Result<Vec<CrossSourceGroup>, DiscoveryError>;
}

#[derive(Debug)]
pub struct CrossSourceGroup {
    pub company_id: Option<String>,
    pub title_normalized: String,
    pub location_normalized: String,
    /// All (source, job_id, priority) tuples for this title+company+location.
    /// Priority: greenhouse=0, lever=1, adzuna=2 (lower = keep).
    pub members: Vec<(String, String, u8)>,
}
```

### SQLite Schema

```sql
-- Migration: 006_discovery_engine.sql

-- Primary job store. source + source_id form the natural dedup key.
CREATE TABLE IF NOT EXISTS jobs (
    id                  TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    source              TEXT NOT NULL,      -- "greenhouse", "lever", "adzuna"
    source_id           TEXT NOT NULL,      -- platform's own job ID
    title               TEXT NOT NULL,
    title_normalized    TEXT NOT NULL,      -- lowercase, whitespace-collapsed
    company_name        TEXT NOT NULL,
    company_id          TEXT,               -- FK to companies.id, nullable
    location            TEXT,
    location_normalized TEXT NOT NULL DEFAULT '',
    remote              TEXT NOT NULL DEFAULT 'unknown',   -- RemoteType as TEXT
    url                 TEXT NOT NULL,
    description         TEXT NOT NULL DEFAULT '',
    description_sha256  TEXT NOT NULL DEFAULT '',          -- for change detection
    salary_min          INTEGER,            -- cents
    salary_max          INTEGER,
    salary_currency     TEXT,
    department          TEXT,
    employment_type     TEXT,
    posted_at           TEXT,               -- ISO 8601
    discovered_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
    -- User-facing lifecycle status
    status              TEXT NOT NULL DEFAULT 'new',
    -- Semantic and ghost scores — populated by downstream steps
    match_score         REAL,
    ghost_score         REAL,
    embedding_updated_at TEXT,              -- NULL until semantic-matching runs
    FOREIGN KEY (company_id) REFERENCES companies(id) ON DELETE SET NULL,
    UNIQUE (source, source_id)
);

-- Indices for common query patterns
CREATE INDEX IF NOT EXISTS idx_jobs_status        ON jobs(status, discovered_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_company       ON jobs(company_id, status);
CREATE INDEX IF NOT EXISTS idx_jobs_dedup         ON jobs(title_normalized, company_id, location_normalized);
CREATE INDEX IF NOT EXISTS idx_jobs_posted_at     ON jobs(posted_at DESC) WHERE posted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_jobs_ghost_score   ON jobs(ghost_score) WHERE ghost_score IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_jobs_source        ON jobs(source, source_id);

-- Lookup: company name → source configuration
CREATE TABLE IF NOT EXISTS discovery_companies (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name            TEXT NOT NULL UNIQUE,
    greenhouse_token TEXT,    -- null if company doesn't use Greenhouse
    lever_slug      TEXT,     -- null if company doesn't use Lever
    workday_url     TEXT,     -- null if company doesn't use Workday
    adzuna_keywords TEXT,     -- null if not fetched via Adzuna
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_fetched_at TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_discovery_companies_enabled ON discovery_companies(enabled);

-- Per-run discovery audit log
CREATE TABLE IF NOT EXISTS discovery_runs (
    id                  TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    started_at          TEXT NOT NULL,
    finished_at         TEXT,
    new_jobs            INTEGER,
    updated             INTEGER,
    duplicates          INTEGER,
    cross_source_deduped INTEGER,
    errors_json         TEXT,               -- JSON array of error strings
    duration_ms         INTEGER,
    triggered_by        TEXT NOT NULL DEFAULT 'schedule'  -- 'schedule', 'manual', 'ralph'
);
```

### Module Structure

```
lazyjob-core/
  src/
    discovery/
      mod.rs            # pub use re-exports, DiscoveryService
      models.rs         # RawJob, EnrichedJob, RemoteType, DiscoveryReport, UpsertOutcome
      source.rs         # JobSource trait, JobRepository trait, CrossSourceGroup
      error.rs          # DiscoveryError enum
      service.rs        # DiscoveryService (top-level orchestrator)
      registry.rs       # CompanyRegistry, JobSourceRegistry
      enrichment/
        mod.rs          # EnrichmentPipeline, StepResult
        sanitize.rs     # HTML sanitization via ammonia
        salary.rs       # Salary regex extraction
        remote.rs       # RemoteType classification
        location.rs     # Location normalization
        company.rs      # Company linkage (SQLite lookup)
      sources/
        mod.rs          # re-exports
        greenhouse.rs   # GreenhouseSource: JobSource impl
        lever.rs        # LeverSource: JobSource impl
        adzuna.rs       # AdzunaSource: JobSource impl (Phase 2)
      dedup.rs          # Cross-source deduplication pass
    persistence/
      jobs.rs           # SqliteJobRepository: JobRepository impl
      companies.rs      # CompanyStore (read companies table)
```

## Implementation Phases

---

### Phase 1 — Core Types, Schema, and Error Handling (MVP Foundation)

#### Step 1.1 — Define error type

```rust
// lazyjob-core/src/discovery/error.rs

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DiscoveryError>;

#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("HTTP fetch failed for source {source}: {reason}")]
    FetchFailed { source: String, reason: String },

    #[error("response parse failed for source {source}: {reason}")]
    ParseFailed { source: String, reason: String },

    #[error("rate limit exceeded for source {source}")]
    RateLimited { source: String },

    #[error("SQLite error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("enrichment pipeline step '{step}' failed: {reason}")]
    EnrichmentFailed { step: String, reason: String },

    #[error("source '{name}' not registered")]
    UnknownSource { name: String },

    #[error("company config missing required field '{field}' for source '{source}'")]
    MissingConfig { source: String, field: String },

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

**Verification**: `cargo check` compiles; all variant arms compile without dead code.

---

#### Step 1.2 — Apply SQLite migration

Add `006_discovery_engine.sql` to `lazyjob-core/migrations/`. The migration creates `jobs`,
`discovery_companies`, and `discovery_runs` tables with all indices.

```rust
// In lazyjob-core/src/persistence/db.rs — at startup:
sqlx::migrate!("./migrations").run(&pool).await?;
```

**Verification**: `sqlx migrate run` completes without error; `sqlite3 lazyjob.db .schema`
shows all three tables.

---

#### Step 1.3 — Define `RawJob`, `EnrichedJob`, `RemoteType`, `DiscoveryReport`

Create `lazyjob-core/src/discovery/models.rs` with the types defined in the Architecture
section above. Derive `Debug`, `Clone` on all. Derive `Serialize`/`Deserialize` on types
that flow into SQLite or JSON-serialized `errors_json`.

---

#### Step 1.4 — `CompanyRegistry` loaded from config.toml

```rust
// lazyjob-core/src/discovery/registry.rs

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Parsed entry from `[[discovery.companies]]` in config.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct CompanyConfig {
    pub name: String,
    pub greenhouse_board_token: Option<String>,
    pub lever_company_id: Option<String>,
    pub workday_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdzunaConfig {
    pub app_id: String,
    pub app_key: String,
    pub countries: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DiscoveryConfig {
    pub polling_interval_minutes: Option<u64>,
    pub companies: Vec<CompanyConfig>,
    pub adzuna: Option<AdzunaConfig>,
}

/// Registry mapping company name → configured sources.
/// Constructed from `DiscoveryConfig` at startup.
pub struct CompanyRegistry {
    pub companies: Vec<CompanyConfig>,
    pub adzuna: Option<AdzunaConfig>,
}

impl CompanyRegistry {
    /// Build from the parsed `[discovery]` section of config.toml.
    pub fn from_config(config: DiscoveryConfig) -> Self {
        Self {
            companies: config.companies,
            adzuna: config.adzuna,
        }
    }

    /// Load from disk. Defaults to `~/.config/lazyjob/config.toml`.
    pub fn load(config_path: PathBuf) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Root { discovery: DiscoveryConfig }
        let contents = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading config at {}", config_path.display()))?;
        let root: Root = toml::from_str(&contents)?;
        Ok(Self::from_config(root.discovery))
    }

    pub fn company_count(&self) -> usize {
        self.companies.len()
    }
}
```

**Verification**: Parse a sample `config.toml` with 3 companies and assert `company_count() == 3`.

---

#### Step 1.5 — `JobSourceRegistry`

```rust
// lazyjob-core/src/discovery/registry.rs (continued)

use std::sync::Arc;
use super::source::JobSource;
use super::error::DiscoveryError;

pub struct JobSourceRegistry {
    sources: HashMap<&'static str, Arc<dyn JobSource>>,
}

impl JobSourceRegistry {
    pub fn new() -> Self {
        Self { sources: HashMap::new() }
    }

    pub fn register(&mut self, source: Arc<dyn JobSource>) {
        self.sources.insert(source.name(), source);
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn JobSource>, DiscoveryError> {
        self.sources.get(name).cloned().ok_or_else(|| DiscoveryError::UnknownSource {
            name: name.to_string(),
        })
    }
}
```

---

### Phase 2 — Enrichment Pipeline

The pipeline is a pure stateless chain of transforms. Each step takes `&mut EnrichedJob`
and modifies it in place. The pipeline struct holds precompiled `Regex` patterns via
`once_cell::sync::Lazy`.

#### Step 2.1 — HTML Sanitization (`ammonia`)

```rust
// lazyjob-core/src/discovery/enrichment/sanitize.rs

use ammonia::Builder;
use once_cell::sync::Lazy;

const MAX_DESCRIPTION_BYTES: usize = 65_536;

static SANITIZER: Lazy<Builder<'static>> = Lazy::new(|| {
    let mut b = Builder::new();
    b.tags(std::collections::HashSet::from_iter([
        "p", "br", "ul", "ol", "li", "h1", "h2", "h3", "strong", "em", "code", "pre",
    ]));
    b.clean_content_tags(std::collections::HashSet::from_iter(["script", "style", "head"]));
    b
});

/// Sanitize HTML description to safe plain-ish text.
/// Uses ammonia's allowlist (keeps structure tags, strips JS/CSS).
/// Then strips remaining tags to produce plain text, truncated to MAX bytes.
pub fn sanitize_description(html: &str) -> String {
    let safe_html = SANITIZER.clean(html).to_string();
    // Strip remaining tags with a simple regex (already safe from ammonia)
    let text = html2text_fallback(&safe_html);
    if text.len() > MAX_DESCRIPTION_BYTES {
        // Truncate at a UTF-8 boundary
        let mut end = MAX_DESCRIPTION_BYTES;
        while !text.is_char_boundary(end) { end -= 1; }
        text[..end].to_string()
    } else {
        text
    }
}

fn html2text_fallback(html: &str) -> String {
    // Simple tag-stripping: remove all <...> content.
    // ammonia guarantees safe input so this is safe to do naively.
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => { in_tag = false; out.push(' '); }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // Collapse runs of whitespace
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Compute SHA-256 of description text for change detection.
pub fn description_sha256(description: &str) -> String {
    use std::hash::{Hash, Hasher};
    // Use a stable hash — not std::hash (not stable across runs).
    // Use sha2 crate. Add to Cargo.toml: sha2 = "0.10"
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(description.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

**Crate APIs**:
- `ammonia::Builder::new()` — builder for per-tag allowlists
- `builder.tags(HashSet<&str>)` — set allowed tag whitelist
- `builder.clean(html).to_string()` — produce sanitized HTML string

**Note**: Add `sha2 = "0.10"` to Cargo.toml for `description_sha256`.

**Verification**: Feed `<script>alert(1)</script><p>Hello</p>` through sanitizer; assert
output contains "Hello" and does not contain "alert".

---

#### Step 2.2 — Salary Extraction

```rust
// lazyjob-core/src/discovery/enrichment/salary.rs

use once_cell::sync::Lazy;
use regex::Regex;

/// Compiled salary extraction patterns.
/// Supports: $120k, $120,000, $120k-$160k, €80k, £50k-£70k, USD 100k
static SALARY_RANGE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([\$€£])\s*([0-9][0-9,]*\.?[0-9]*)\s*([kK])?\s*[-–—to]+\s*[\$€£]?\s*([0-9][0-9,]*\.?[0-9]*)\s*([kK])?"
    ).unwrap()
});

static SALARY_SINGLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([\$€£])\s*([0-9][0-9,]*\.?[0-9]*)\s*([kK])?\s*(per year|/year|annually|annual)?"
    ).unwrap()
});

#[derive(Debug)]
pub struct SalaryExtraction {
    pub min_cents: i64,
    pub max_cents: i64,
    pub currency: String,
}

/// Attempt to extract salary information from free-text job description.
/// Returns None if no salary pattern is found.
pub fn extract_salary(text: &str) -> Option<SalaryExtraction> {
    if let Some(caps) = SALARY_RANGE.captures(text) {
        let currency = currency_symbol_to_code(caps.get(1)?.as_str());
        let min_raw = parse_amount(caps.get(2)?.as_str(), caps.get(3).map(|m| m.as_str()));
        let max_raw = parse_amount(caps.get(4)?.as_str(), caps.get(5).map(|m| m.as_str()));
        return Some(SalaryExtraction {
            min_cents: (min_raw * 100.0) as i64,
            max_cents: (max_raw * 100.0) as i64,
            currency,
        });
    }
    if let Some(caps) = SALARY_SINGLE.captures(text) {
        let currency = currency_symbol_to_code(caps.get(1)?.as_str());
        let amount = parse_amount(caps.get(2)?.as_str(), caps.get(3).map(|m| m.as_str()));
        return Some(SalaryExtraction {
            min_cents: (amount * 100.0) as i64,
            max_cents: (amount * 100.0) as i64,
            currency,
        });
    }
    None
}

fn parse_amount(digits: &str, k_suffix: Option<&str>) -> f64 {
    let n: f64 = digits.replace(',', "").parse().unwrap_or(0.0);
    if k_suffix.map(|s| s.eq_ignore_ascii_case("k")).unwrap_or(false) {
        n * 1000.0
    } else {
        n
    }
}

fn currency_symbol_to_code(sym: &str) -> String {
    match sym {
        "$" => "USD",
        "€" => "EUR",
        "£" => "GBP",
        _ => "USD",
    }.to_string()
}
```

**Verification**: Unit test table with inputs like `"$120k-$160k/year"`, `"€80,000 - €100,000"`,
`"£50k annual"`. Assert extracted `min_cents`/`max_cents`/`currency` match expected values.

---

#### Step 2.3 — Remote Classification

```rust
// lazyjob-core/src/discovery/enrichment/remote.rs

use super::super::models::RemoteType;
use once_cell::sync::Lazy;
use regex::Regex;

static REMOTE_KEYWORDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(remote|work\s+from\s+home|wfh|distributed|anywhere)\b").unwrap()
});

static HYBRID_KEYWORDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(hybrid|flexible|partial.?remote|remote.?friendly)\b").unwrap()
});

static ONSITE_KEYWORDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(on.?site|in.?office|in.?person|not remote|no remote)\b").unwrap()
});

/// Classify remote type from title, location string, and description excerpt.
/// Precedence: explicit on-site denial > hybrid > remote > unknown.
pub fn classify_remote(title: &str, location: Option<&str>, description_excerpt: &str) -> RemoteType {
    let haystack = format!(
        "{} {} {}",
        title,
        location.unwrap_or(""),
        &description_excerpt[..description_excerpt.len().min(1000)]
    );

    if ONSITE_KEYWORDS.is_match(&haystack) && !REMOTE_KEYWORDS.is_match(&haystack) {
        return RemoteType::No;
    }
    if HYBRID_KEYWORDS.is_match(&haystack) {
        return RemoteType::Hybrid;
    }
    if REMOTE_KEYWORDS.is_match(&haystack) {
        return RemoteType::Yes;
    }
    // Last resort: if location literally says "Remote"
    if let Some(loc) = location {
        let loc_lower = loc.to_lowercase();
        if loc_lower.trim() == "remote" || loc_lower.trim() == "anywhere" {
            return RemoteType::Yes;
        }
    }
    RemoteType::Unknown
}
```

---

#### Step 2.4 — Location Normalization

```rust
// lazyjob-core/src/discovery/enrichment/location.rs

/// Normalize location string for cross-source dedup comparison.
/// "San Francisco, CA" → "san francisco ca"
/// "Remote" / "Anywhere" → "remote"
/// "New York City (NYC)" → "new york city nyc"
pub fn normalize_location(location: Option<&str>) -> String {
    let loc = match location {
        None | Some("") => return String::new(),
        Some(l) => l,
    };
    // Lowercase, strip punctuation except spaces, collapse whitespace
    loc.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize job title for cross-source dedup comparison.
/// "Senior Software Engineer (Rust)" → "senior software engineer rust"
pub fn normalize_title(title: &str) -> String {
    title.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
```

---

#### Step 2.5 — Company Linkage

```rust
// lazyjob-core/src/discovery/enrichment/company.rs

use sqlx::SqlitePool;

/// Resolve a company name to its `companies.id` FK.
/// Performs case-insensitive exact match first, then fuzzy match
/// using strsim::jaro_winkler with threshold 0.92.
///
/// Returns None if no match found. Returns the matched company_id if found.
pub async fn resolve_company_id(
    company_name: &str,
    pool: &SqlitePool,
) -> Option<String> {
    // Exact match (case-insensitive via SQLite COLLATE NOCASE)
    let result = sqlx::query_scalar!(
        "SELECT id FROM companies WHERE name = ? COLLATE NOCASE LIMIT 1",
        company_name
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if result.is_some() {
        return result;
    }

    // Fuzzy match: load all company names (typically < 200) and compare in Rust
    let candidates: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM companies"
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let lower_target = company_name.to_lowercase();
    candidates.into_iter()
        .filter_map(|(id, name)| {
            let score = strsim::jaro_winkler(&lower_target, &name.to_lowercase());
            if score >= 0.92 { Some((id, score)) } else { None }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)
}
```

**Crate APIs**:
- `strsim::jaro_winkler(s1, s2) -> f64` — similarity score in [0, 1]
- `sqlx::query_scalar!("SELECT id FROM ...", name)` — compile-time checked scalar query

---

#### Step 2.6 — `EnrichmentPipeline` orchestrator

```rust
// lazyjob-core/src/discovery/enrichment/mod.rs

use sqlx::SqlitePool;
use chrono::Utc;

use super::models::{RawJob, EnrichedJob, RemoteType};
use super::error::DiscoveryError;
use self::sanitize::{sanitize_description, description_sha256};
use self::salary::extract_salary;
use self::remote::classify_remote;
use self::location::{normalize_location, normalize_title};
use self::company::resolve_company_id;

pub mod sanitize;
pub mod salary;
pub mod remote;
pub mod location;
pub mod company;

pub struct EnrichmentPipeline {
    pool: SqlitePool,
}

impl EnrichmentPipeline {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Run all enrichment steps on a `RawJob` and return an `EnrichedJob`.
    /// Steps are ordered per spec:
    ///   1. HTML sanitization
    ///   2. Salary extraction
    ///   3. Remote classification
    ///   4. Location normalization
    ///   5. Company linkage (async, requires DB)
    pub async fn process(&self, raw: RawJob) -> Result<EnrichedJob, DiscoveryError> {
        // Step 1: HTML sanitization
        let description = raw.description_html.as_deref()
            .map(sanitize_description)
            .unwrap_or_default();

        let description_hash = description_sha256(&description);

        // Step 2: Salary extraction
        let salary = extract_salary(&description);

        // Step 3: Remote classification
        let remote = classify_remote(
            &raw.title,
            raw.location.as_deref(),
            &description,
        );

        // Step 4: Location normalization
        let location_normalized = normalize_location(raw.location.as_deref());
        let title_normalized = normalize_title(&raw.title);

        // Step 5: Company linkage
        let company_id = resolve_company_id(&raw.company_name, &self.pool).await;

        Ok(EnrichedJob {
            source: raw.source,
            source_id: raw.source_id,
            title: raw.title,
            title_normalized,
            company_name: raw.company_name,
            company_id,
            location: raw.location,
            location_normalized,
            remote,
            url: raw.url,
            description,
            salary_min: salary.as_ref().map(|s| s.min_cents),
            salary_max: salary.as_ref().map(|s| s.max_cents),
            salary_currency: salary.map(|s| s.currency),
            department: raw.department,
            employment_type: raw.employment_type,
            posted_at: raw.posted_at,
            discovered_at: Utc::now(),
        })
    }
}
```

**Verification**: Create a `RawJob` with HTML description containing a salary range and
remote keywords. Assert the `EnrichedJob` has `salary_min`, `salary_max`, and `remote = RemoteType::Yes`.

---

### Phase 3 — Source Implementations

#### Step 3.1 — `GreenhouseSource`

```rust
// lazyjob-core/src/discovery/sources/greenhouse.rs

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tracing::instrument;

use crate::discovery::{
    models::RawJob,
    source::JobSource,
    error::DiscoveryError,
};
use crate::platforms::rate_limit::RateLimiter;
use crate::platforms::http::PlatformHttpClient;

#[derive(Deserialize, Debug)]
struct GhJobList { jobs: Vec<GhJob> }

#[derive(Deserialize, Debug)]
struct GhJob {
    id: u64,
    title: String,
    absolute_url: String,
    location: Option<GhLocation>,
    content: Option<String>,
    departments: Vec<GhDept>,
    updated_at: Option<String>,
}
#[derive(Deserialize, Debug)]
struct GhLocation { name: String }
#[derive(Deserialize, Debug)]
struct GhDept { name: String }

pub struct GreenhouseSource {
    http: PlatformHttpClient,
    limiter: RateLimiter,
    /// Override base URL for testing (wiremock). Production: "https://boards-api.greenhouse.io"
    base_url: String,
}

impl GreenhouseSource {
    pub fn new() -> Self {
        Self {
            http: PlatformHttpClient::new("greenhouse"),
            limiter: RateLimiter::new(60),
            base_url: "https://boards-api.greenhouse.io".to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), ..Self::new() }
    }
}

#[async_trait]
impl JobSource for GreenhouseSource {
    fn name(&self) -> &'static str { "greenhouse" }

    #[instrument(skip(self), fields(company_id = %company_id))]
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>, DiscoveryError> {
        self.limiter.acquire().await;

        let url = format!(
            "{}/v1/boards/{}/jobs?content=true",
            self.base_url, company_id
        );
        let resp = self.http.get_with_retry(&url).await
            .map_err(|e| DiscoveryError::FetchFailed {
                source: "greenhouse".into(),
                reason: e.to_string(),
            })?;

        let data: GhJobList = resp.json().await
            .map_err(|e| DiscoveryError::ParseFailed {
                source: "greenhouse".into(),
                reason: e.to_string(),
            })?;

        tracing::info!(
            count = data.jobs.len(),
            board_token = company_id,
            "greenhouse fetch complete"
        );

        Ok(data.jobs.into_iter().map(|j| RawJob {
            source: "greenhouse".to_string(),
            source_id: j.id.to_string(),
            title: j.title,
            company_name: company_id.to_string(), // overridden by registry with display name
            location: j.location.map(|l| l.name),
            url: j.absolute_url,
            description_html: j.content,
            department: j.departments.into_iter().next().map(|d| d.name),
            employment_type: None,
            posted_at: j.updated_at.as_deref().and_then(|s| s.parse().ok()),
        }).collect())
    }

    async fn health_check(&self) -> bool {
        // Attempt a known-public board (greenhouse's own board)
        let url = format!("{}/v1/boards/greenhouse/jobs", self.base_url);
        self.http.get_with_retry(&url).await.is_ok()
    }
}
```

---

#### Step 3.2 — `LeverSource`

```rust
// lazyjob-core/src/discovery/sources/lever.rs

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use tracing::instrument;

use crate::discovery::{models::RawJob, source::JobSource, error::DiscoveryError};
use crate::platforms::{rate_limit::RateLimiter, http::PlatformHttpClient};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LeverPosting {
    id: String,
    text: String,
    categories: LeverCategories,
    description_plain: Option<String>,
    description: String,
    hosted_url: String,
    created_at: Option<i64>,
}

#[derive(Deserialize, Debug, Default)]
struct LeverCategories {
    department: Option<String>,
    location: Option<String>,
    commitment: Option<String>,
}

type LeverResponse = Vec<LeverPosting>;

pub struct LeverSource {
    http: PlatformHttpClient,
    limiter: RateLimiter,
    base_url: String,
}

impl LeverSource {
    pub fn new() -> Self {
        Self {
            http: PlatformHttpClient::new("lever"),
            limiter: RateLimiter::new(60),
            base_url: "https://api.lever.co".to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), ..Self::new() }
    }
}

#[async_trait]
impl JobSource for LeverSource {
    fn name(&self) -> &'static str { "lever" }

    #[instrument(skip(self), fields(company_id = %company_id))]
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>, DiscoveryError> {
        self.limiter.acquire().await;

        let url = format!("{}/v0/postings/{}?mode=json", self.base_url, company_id);
        let resp = self.http.get_with_retry(&url).await
            .map_err(|e| DiscoveryError::FetchFailed {
                source: "lever".into(),
                reason: e.to_string(),
            })?;

        let postings: LeverResponse = resp.json().await
            .map_err(|e| DiscoveryError::ParseFailed {
                source: "lever".into(),
                reason: e.to_string(),
            })?;

        tracing::info!(
            count = postings.len(),
            company = company_id,
            "lever fetch complete"
        );

        Ok(postings.into_iter().map(|p| {
            let posted_at = p.created_at
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single());

            RawJob {
                source: "lever".to_string(),
                source_id: p.id,
                title: p.text,
                company_name: company_id.to_string(),
                location: p.categories.location,
                url: p.hosted_url,
                description_html: Some(p.description),
                department: p.categories.department,
                employment_type: p.categories.commitment,
                posted_at,
            }
        }).collect())
    }

    async fn health_check(&self) -> bool {
        // Lever's own company board as canary
        let url = format!("{}/v0/postings/lever", self.base_url);
        self.http.get_with_retry(&url).await.is_ok()
    }
}
```

---

#### Step 3.3 — `AdzunaSource` (Phase 2, config-gated)

```rust
// lazyjob-core/src/discovery/sources/adzuna.rs

use async_trait::async_trait;
use serde::Deserialize;
use tracing::instrument;
use secrecy::{Secret, ExposeSecret};

use crate::discovery::{models::RawJob, source::JobSource, error::DiscoveryError};
use crate::platforms::{rate_limit::RateLimiter, http::PlatformHttpClient};

#[derive(Deserialize, Debug)]
struct AdzunaResponse {
    results: Vec<AdzunaJob>,
}

#[derive(Deserialize, Debug)]
struct AdzunaJob {
    id: String,
    title: String,
    company: AdzunaCompany,
    location: AdzunaLocation,
    redirect_url: String,
    description: String,
    created: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AdzunaCompany { display_name: String }
#[derive(Deserialize, Debug)]
struct AdzunaLocation { display_name: String }

pub struct AdzunaSource {
    http: PlatformHttpClient,
    limiter: RateLimiter,
    app_id: String,
    app_key: Secret<String>,
    country: String,
    base_url: String,
}

impl AdzunaSource {
    /// Construct with credentials from config. Rate-limited to 10 RPM (free tier).
    pub fn new(app_id: String, app_key: String, country: String) -> Self {
        Self {
            http: PlatformHttpClient::new("adzuna"),
            limiter: RateLimiter::new(10), // free tier: 10 RPM
            app_id,
            app_key: Secret::new(app_key),
            country,
            base_url: "https://api.adzuna.com".to_string(),
        }
    }
}

#[async_trait]
impl JobSource for AdzunaSource {
    fn name(&self) -> &'static str { "adzuna" }

    /// `company_id` here is used as a keyword search query prefix.
    #[instrument(skip(self), fields(query = %company_id))]
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>, DiscoveryError> {
        self.limiter.acquire().await;

        let url = format!(
            "{}/v1/api/jobs/{}/search/1?app_id={}&app_key={}&results_per_page=50&what_company={}",
            self.base_url,
            self.country,
            self.app_id,
            self.app_key.expose_secret(),
            urlencoding::encode(company_id),
        );
        let resp = self.http.get_with_retry(&url).await
            .map_err(|e| DiscoveryError::FetchFailed {
                source: "adzuna".into(),
                reason: e.to_string(),
            })?;

        let data: AdzunaResponse = resp.json().await
            .map_err(|e| DiscoveryError::ParseFailed {
                source: "adzuna".into(),
                reason: e.to_string(),
            })?;

        Ok(data.results.into_iter().map(|j| RawJob {
            source: "adzuna".to_string(),
            source_id: j.id,
            title: j.title,
            company_name: j.company.display_name,
            location: Some(j.location.display_name),
            url: j.redirect_url,
            description_html: Some(j.description),
            department: None,
            employment_type: None,
            posted_at: j.created.as_deref().and_then(|s| s.parse().ok()),
        }).collect())
    }

    async fn health_check(&self) -> bool {
        // Simply check that the base URL is reachable
        let url = format!("{}/v1/api/version", self.base_url);
        self.http.get_with_retry(&url).await.is_ok()
    }
}
```

**Note**: Add `urlencoding = "2"` to Cargo.toml.

---

### Phase 4 — `SqliteJobRepository` and `DiscoveryService`

#### Step 4.1 — `SqliteJobRepository`

```rust
// lazyjob-core/src/persistence/jobs.rs

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::instrument;

use crate::discovery::{
    models::{EnrichedJob, UpsertOutcome},
    source::{JobRepository, CrossSourceGroup},
    error::DiscoveryError,
};

pub struct SqliteJobRepository {
    pool: SqlitePool,
}

impl SqliteJobRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl JobRepository for SqliteJobRepository {
    #[instrument(skip(self, job), fields(source = %job.source, source_id = %job.source_id))]
    async fn upsert(&self, job: &EnrichedJob) -> Result<(UpsertOutcome, String), DiscoveryError> {
        // Check for existing record
        let existing = sqlx::query!(
            "SELECT id, description_sha256 FROM jobs WHERE source = ? AND source_id = ?",
            job.source,
            job.source_id
        )
        .fetch_optional(&self.pool)
        .await?;

        use sha2::{Sha256, Digest};
        let new_hash = {
            let mut h = Sha256::new();
            h.update(job.description.as_bytes());
            format!("{:x}", h.finalize())
        };

        match existing {
            Some(row) if row.description_sha256 == new_hash => {
                // Content unchanged — skip update
                return Ok((UpsertOutcome::Unchanged, row.id));
            }
            Some(row) => {
                // Content changed — update
                sqlx::query!(
                    r#"UPDATE jobs SET
                        title = ?, title_normalized = ?, description = ?,
                        description_sha256 = ?, salary_min = ?, salary_max = ?,
                        salary_currency = ?, remote = ?, location = ?,
                        location_normalized = ?, updated_at = datetime('now')
                    WHERE id = ?"#,
                    job.title, job.title_normalized, job.description,
                    new_hash, job.salary_min, job.salary_max,
                    job.salary_currency, job.remote.as_str(), job.location,
                    job.location_normalized, row.id
                )
                .execute(&self.pool)
                .await?;
                return Ok((UpsertOutcome::Updated, row.id));
            }
            None => {
                // New job — insert
                let new_id = uuid::Uuid::new_v4().to_string();
                sqlx::query!(
                    r#"INSERT INTO jobs (
                        id, source, source_id, title, title_normalized,
                        company_name, company_id, location, location_normalized,
                        remote, url, description, description_sha256,
                        salary_min, salary_max, salary_currency,
                        department, employment_type, posted_at,
                        discovered_at, updated_at, status
                    ) VALUES (
                        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                        ?, ?, ?, ?, ?, ?, ?, datetime('now'), 'new'
                    )"#,
                    new_id, job.source, job.source_id, job.title, job.title_normalized,
                    job.company_name, job.company_id, job.location, job.location_normalized,
                    job.remote.as_str(), job.url, job.description, new_hash,
                    job.salary_min, job.salary_max, job.salary_currency,
                    job.department, job.employment_type, job.posted_at,
                    job.discovered_at
                )
                .execute(&self.pool)
                .await?;
                return Ok((UpsertOutcome::Inserted, new_id));
            }
        }
    }

    async fn find_by_source(
        &self,
        source: &str,
        source_id: &str,
    ) -> Result<Option<String>, DiscoveryError> {
        let result = sqlx::query_scalar!(
            "SELECT id FROM jobs WHERE source = ? AND source_id = ?",
            source, source_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(result)
    }

    async fn list_by_status(
        &self,
        status: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Job>, DiscoveryError> {
        let rows = sqlx::query_as!(
            Job,
            r#"SELECT id, source, source_id, title, company_name, location,
                      remote, url, description, salary_min, salary_max,
                      salary_currency, department, status, match_score,
                      ghost_score, posted_at, discovered_at, updated_at
               FROM jobs
               WHERE status = ?
               ORDER BY discovered_at DESC
               LIMIT ? OFFSET ?"#,
            status, limit, offset
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn soft_delete(&self, job_id: &str) -> Result<(), DiscoveryError> {
        sqlx::query!(
            "UPDATE jobs SET status = 'deduped', updated_at = datetime('now') WHERE id = ?",
            job_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_cross_source_candidates(&self) -> Result<Vec<CrossSourceGroup>, DiscoveryError> {
        // Group jobs by (company_id, title_normalized, location_normalized)
        // where more than one source contributed
        let rows = sqlx::query!(
            r#"SELECT company_id, title_normalized, location_normalized,
                      source, id as job_id
               FROM jobs
               WHERE status != 'deduped'
               ORDER BY company_id, title_normalized, location_normalized, source"#
        )
        .fetch_all(&self.pool)
        .await?;

        // Group in Rust (SQLite lacks GROUP_CONCAT over multiple columns cleanly)
        let mut groups: std::collections::HashMap<(Option<String>, String, String), Vec<(String, String, u8)>> = Default::default();
        for row in rows {
            let priority: u8 = match row.source.as_str() {
                "greenhouse" => 0,
                "lever" => 1,
                _ => 2,
            };
            groups
                .entry((row.company_id, row.title_normalized, row.location_normalized))
                .or_default()
                .push((row.source, row.job_id, priority));
        }

        let result = groups
            .into_iter()
            .filter(|(_, members)| {
                // Only include groups with more than one distinct source
                let sources: std::collections::HashSet<&str> = members.iter()
                    .map(|(s, _, _)| s.as_str())
                    .collect();
                sources.len() > 1
            })
            .map(|((company_id, title_normalized, location_normalized), members)| {
                CrossSourceGroup { company_id, title_normalized, location_normalized, members }
            })
            .collect();

        Ok(result)
    }
}

/// Read model returned from `list_by_status` — flat, denormalized row for TUI rendering.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Job {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub company_name: String,
    pub location: Option<String>,
    pub remote: String,
    pub url: String,
    pub description: String,
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub salary_currency: Option<String>,
    pub department: Option<String>,
    pub status: String,
    pub match_score: Option<f64>,
    pub ghost_score: Option<f64>,
    pub posted_at: Option<String>,
    pub discovered_at: String,
    pub updated_at: String,
}
```

---

#### Step 4.2 — Cross-Source Deduplication Pass

```rust
// lazyjob-core/src/discovery/dedup.rs

use tracing::instrument;
use strsim::jaro_winkler;

use super::source::JobRepository;
use super::error::DiscoveryError;

/// Run the cross-source dedup pass after ingestion.
/// For each group of jobs sharing (company_id, title_normalized, location_normalized)
/// with multiple sources, soft-deletes the lower-priority source records.
///
/// Priority (lower = keep): greenhouse=0, lever=1, adzuna=2.
/// If exact title match (jaro_winkler >= 0.96), dedup is confident.
/// If score in [0.85, 0.96), dedup is uncertain — soft-delete the lower-priority
/// source but emit a warning for potential false positives.
#[instrument(skip(repo))]
pub async fn run_cross_source_dedup(
    repo: &dyn JobRepository,
) -> Result<usize, DiscoveryError> {
    let groups = repo.list_cross_source_candidates().await?;
    let mut deduped = 0usize;

    for group in &groups {
        // Sort members by priority (ascending = keep first)
        let mut members = group.members.clone();
        members.sort_by_key(|(_, _, p)| *p);

        // The lowest-priority source to keep
        let (_, keep_id, _) = &members[0];
        let keep_title = &group.title_normalized;

        for (source, job_id, _) in members.iter().skip(1) {
            // Confirm high similarity before deleting
            let score = jaro_winkler(keep_title, &group.title_normalized);
            if score >= 0.85 {
                tracing::info!(
                    job_id = %job_id,
                    source = %source,
                    score = score,
                    "soft-deleting cross-source duplicate"
                );
                repo.soft_delete(job_id).await?;
                deduped += 1;
            }
        }
    }

    Ok(deduped)
}
```

---

#### Step 4.3 — `DiscoveryService` (top-level orchestrator)

```rust
// lazyjob-core/src/discovery/service.rs

use std::sync::Arc;
use std::time::Instant;
use futures::future::join_all;
use sqlx::SqlitePool;
use tracing::instrument;

use super::{
    models::{DiscoveryReport, UpsertOutcome},
    source::JobRepository,
    registry::{CompanyRegistry, JobSourceRegistry},
    enrichment::EnrichmentPipeline,
    dedup::run_cross_source_dedup,
    error::DiscoveryError,
};

pub struct DiscoveryService {
    registry: CompanyRegistry,
    sources: JobSourceRegistry,
    repo: Arc<dyn JobRepository>,
    enricher: EnrichmentPipeline,
    pool: SqlitePool,
}

impl DiscoveryService {
    pub fn new(
        registry: CompanyRegistry,
        sources: JobSourceRegistry,
        repo: Arc<dyn JobRepository>,
        pool: SqlitePool,
    ) -> Self {
        Self {
            enricher: EnrichmentPipeline::new(pool.clone()),
            registry,
            sources,
            repo,
            pool,
        }
    }

    /// Run a full discovery pass across all configured companies and sources.
    /// Fan-out is parallel per (company, source) pair.
    #[instrument(skip(self))]
    pub async fn run_discovery(&self) -> Result<DiscoveryReport, DiscoveryError> {
        let start = Instant::now();
        let mut report = DiscoveryReport::default();

        // Build the list of (company_display_name, source_name, source_company_id) tuples
        let fetch_tasks: Vec<(String, &'static str, String)> = self.registry.companies
            .iter()
            .flat_map(|company| {
                let mut tasks = Vec::new();
                if let Some(token) = &company.greenhouse_board_token {
                    tasks.push((company.name.clone(), "greenhouse", token.clone()));
                }
                if let Some(slug) = &company.lever_company_id {
                    tasks.push((company.name.clone(), "lever", slug.clone()));
                }
                tasks
            })
            .collect();

        tracing::info!(
            task_count = fetch_tasks.len(),
            "starting discovery fan-out"
        );

        // Spawn one tokio task per (company, source) pair
        let handles: Vec<_> = fetch_tasks.into_iter().map(|(company_name, source_name, source_id)| {
            let sources = &self.sources;
            let enricher = &self.enricher;
            let repo = Arc::clone(&self.repo);

            async move {
                let source = match sources.get(source_name) {
                    Ok(s) => s,
                    Err(e) => return Err(DiscoveryError::UnknownSource { name: source_name.to_string() }),
                };

                let raw_jobs = source.fetch_jobs(&source_id).await?;
                let mut stats = (0usize, 0usize, 0usize); // new, updated, unchanged

                for raw in raw_jobs {
                    let mut enriched = match enricher.process(raw).await {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::warn!(error = %e, "enrichment failed, skipping job");
                            continue;
                        }
                    };
                    // Override company_name with the display name from config
                    enriched.company_name = company_name.clone();

                    match repo.upsert(&enriched).await {
                        Ok((UpsertOutcome::Inserted, _)) => stats.0 += 1,
                        Ok((UpsertOutcome::Updated, _)) => stats.1 += 1,
                        Ok((UpsertOutcome::Unchanged, _)) => stats.2 += 1,
                        Err(e) => {
                            tracing::warn!(error = %e, "upsert failed, skipping job");
                        }
                    }
                }
                Ok(stats)
            }
        }).collect();

        // Wait for all fetch tasks to complete
        let results = join_all(handles).await;

        for result in results {
            match result {
                Ok((new, updated, dupes)) => {
                    report.new_jobs += new;
                    report.updated += updated;
                    report.duplicates += dupes;
                }
                Err(e) => {
                    tracing::error!(error = %e, "source fetch failed");
                    report.errors.push(e);
                }
            }
        }

        // Cross-source dedup pass
        match run_cross_source_dedup(self.repo.as_ref()).await {
            Ok(n) => report.cross_source_deduped = n,
            Err(e) => {
                tracing::warn!(error = %e, "cross-source dedup failed");
            }
        }

        report.duration_ms = start.elapsed().as_millis() as u64;

        // Persist run audit log
        self.log_run(&report).await.ok();

        tracing::info!(
            new = report.new_jobs,
            updated = report.updated,
            dupes = report.duplicates,
            duration_ms = report.duration_ms,
            "discovery complete"
        );

        Ok(report)
    }

    /// Refresh a single company only (triggered by TUI keybind).
    #[instrument(skip(self), fields(company = %company_name))]
    pub async fn refresh_company(&self, company_name: &str) -> Result<DiscoveryReport, DiscoveryError> {
        let company = self.registry.companies
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(company_name))
            .ok_or_else(|| DiscoveryError::UnknownSource { name: company_name.to_string() })?;

        // Re-use run_discovery logic with a single-company registry subset
        // (simpler: direct call pattern)
        let start = Instant::now();
        let mut report = DiscoveryReport::default();

        let sources_to_try: Vec<(&'static str, &str)> = [
            company.greenhouse_board_token.as_deref().map(|t| ("greenhouse", t)),
            company.lever_company_id.as_deref().map(|s| ("lever", s)),
        ].into_iter().flatten().collect();

        for (source_name, source_id) in sources_to_try {
            let source = self.sources.get(source_name)?;
            let raw_jobs = source.fetch_jobs(source_id).await?;
            for raw in raw_jobs {
                if let Ok(enriched) = self.enricher.process(raw).await {
                    match self.repo.upsert(&enriched).await {
                        Ok((UpsertOutcome::Inserted, _)) => report.new_jobs += 1,
                        Ok((UpsertOutcome::Updated, _)) => report.updated += 1,
                        Ok((UpsertOutcome::Unchanged, _)) => report.duplicates += 1,
                        Err(_) => {}
                    }
                }
            }
        }

        report.duration_ms = start.elapsed().as_millis() as u64;
        Ok(report)
    }

    async fn log_run(&self, report: &DiscoveryReport) -> anyhow::Result<()> {
        let errors_json = serde_json::to_string(
            &report.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>()
        )?;
        sqlx::query!(
            r#"INSERT INTO discovery_runs
                (started_at, finished_at, new_jobs, updated, duplicates,
                 cross_source_deduped, errors_json, duration_ms, triggered_by)
               VALUES (datetime('now'), datetime('now'), ?, ?, ?, ?, ?, ?, 'schedule')"#,
            report.new_jobs as i64,
            report.updated as i64,
            report.duplicates as i64,
            report.cross_source_deduped as i64,
            errors_json,
            report.duration_ms as i64,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

**Crate APIs used:**
- `futures::future::join_all(futures)` — await all fan-out tasks concurrently
- `sqlx::query!` with `execute(&pool)` — DDL and DML statements
- `sqlx::query_as!(Type, sql, ...)` — typed row deserialization

---

### Phase 5 — Ralph Subprocess Integration and TUI Feed

#### Step 5.1 — Ralph subprocess entry point

```rust
// lazyjob-ralph/src/loops/discovery.rs

use crate::worker::{WorkerCommand, WorkerEvent};
use lazyjob_core::discovery::{
    DiscoveryService, CompanyRegistry, JobSourceRegistry,
    sources::{GreenhouseSource, LeverSource},
};
use std::sync::Arc;

/// Entry point for the "job-discovery" Ralph loop.
/// Receives a `WorkerCommand::Start` with `params_json`:
/// ```json
/// { "triggered_by": "schedule" }
/// ```
/// Emits `WorkerEvent::Progress` with status updates, then
/// `WorkerEvent::Done` with the `DiscoveryReport` serialized as JSON.
pub async fn run_discovery_loop(
    params_json: serde_json::Value,
    pool: sqlx::SqlitePool,
) -> anyhow::Result<()> {
    let config_path = lazyjob_core::config::default_config_path();
    let registry = CompanyRegistry::load(config_path)?;

    let mut sources = JobSourceRegistry::new();
    sources.register(Arc::new(GreenhouseSource::new()));
    sources.register(Arc::new(LeverSource::new()));

    let repo = Arc::new(lazyjob_core::persistence::SqliteJobRepository::new(pool.clone()));
    let service = DiscoveryService::new(registry, sources, repo, pool);

    crate::ipc::emit(WorkerEvent::Progress {
        message: "Starting job discovery…".to_string(),
        percent: Some(0),
    });

    let report = service.run_discovery().await?;

    crate::ipc::emit(WorkerEvent::Done {
        output: serde_json::to_value(&report)?,
    });

    Ok(())
}
```

The ralph subprocess binary dispatches `LoopType::JobDiscovery` to `run_discovery_loop`
via the match in `lazyjob-ralph/src/main.rs`.

---

#### Step 5.2 — TUI Job Feed Widget

```rust
// lazyjob-tui/src/views/jobs_feed.rs

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use lazyjob_core::persistence::jobs::Job;

pub struct JobsFeedView {
    jobs: Vec<Job>,
    list_state: ListState,
    /// Horizontal split: list on left (40%), detail on right (60%)
    selected_detail: Option<usize>,
}

impl JobsFeedView {
    pub fn new(jobs: Vec<Job>) -> Self {
        let mut state = ListState::default();
        if !jobs.is_empty() { state.select(Some(0)); }
        Self { jobs, list_state: state, selected_detail: None }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        self.render_list(frame, chunks[0]);
        self.render_detail(frame, chunks[1]);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.jobs.iter().map(|job| {
            let remote_badge = match job.remote.as_str() {
                "yes"     => Span::styled("🌐 ", Style::default().fg(Color::Green)),
                "hybrid"  => Span::styled("⚡ ", Style::default().fg(Color::Yellow)),
                "no"      => Span::styled("🏢 ", Style::default().fg(Color::Red)),
                _         => Span::raw("   "),
            };
            let salary = match (job.salary_min, job.salary_max) {
                (Some(min), Some(max)) if min != max =>
                    format!(" ${}-{}k", min / 100_000, max / 100_000),
                (Some(min), _) =>
                    format!(" ${:.0}k", min as f64 / 100_000.0),
                _ => String::new(),
            };
            let line = Line::from(vec![
                remote_badge,
                Span::raw(format!("{} — {}{}", job.title, job.company_name, salary)),
            ]);
            ListItem::new(line)
        }).collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Jobs "))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let job = self.list_state.selected().and_then(|i| self.jobs.get(i));
        let content = match job {
            None => "No job selected. Use j/k to navigate.".to_string(),
            Some(j) => format!(
                "{}\n{}\n{}\n\n{}",
                j.title,
                j.company_name,
                j.location.as_deref().unwrap_or("Location unknown"),
                &j.description[..j.description.len().min(2000)],
            ),
        };
        let para = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(" Detail "))
            .wrap(ratatui::widgets::Wrap { trim: true });
        frame.render_widget(para, area);
    }

    /// Move selection down (vim j).
    pub fn select_next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.jobs.len().saturating_sub(1)),
            None if !self.jobs.is_empty() => 0,
            None => return,
        };
        self.list_state.select(Some(i));
    }

    /// Move selection up (vim k).
    pub fn select_prev(&mut self) {
        let i = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
    }

    /// Reload the job list from a fresh query result.
    pub fn reload(&mut self, jobs: Vec<Job>) {
        let selected = self.list_state.selected().unwrap_or(0);
        self.jobs = jobs;
        let clamped = selected.min(self.jobs.len().saturating_sub(1));
        self.list_state.select(if self.jobs.is_empty() { None } else { Some(clamped) });
    }
}
```

**TUI keybindings for Jobs Feed:**
| Key | Action |
|-----|--------|
| `j` / `↓` | Select next job |
| `k` / `↑` | Select previous job |
| `Enter` / `Space` | Expand detail panel |
| `R` | Trigger on-demand `refresh_company()` for selected company |
| `s` | Save/star job (`status = 'saved'`) |
| `d` | Dismiss job (`status = 'dismissed'`) |
| `a` | Apply — create Application record |
| `o` | Open URL in browser (`open`/`xdg-open`) |
| `/` | Filter by keyword (inline search) |
| `G` | Jump to bottom |
| `gg` | Jump to top |

---

## Key Crate APIs

| Crate | API | Usage |
|---|---|---|
| `ammonia` | `Builder::new().tags(set).clean(html).to_string()` | HTML allowlist sanitization |
| `regex` | `Regex::new(pattern)` (via `Lazy`) | Salary and remote keyword patterns |
| `once_cell` | `Lazy<Regex>` | Compile regex once at startup |
| `strsim` | `jaro_winkler(s1, s2) -> f64` | Company name fuzzy matching, cross-source dedup |
| `sha2` | `Sha256::new(); hasher.update(bytes); finalize()` | Description change detection hash |
| `futures` | `join_all(Vec<impl Future>)` | Parallel source fan-out |
| `tokio` | `task::spawn(async move { ... })` | Per-source task isolation |
| `governor` | `RateLimiter::direct(Quota::per_minute(n))` | Per-source request throttling |
| `backoff` | `future::retry(ExponentialBackoff::default(), op)` | HTTP retry on 429/5xx |
| `sqlx` | `query!("...", ?)` / `query_as!` | Compile-time SQL checks |
| `sqlx` | `query_scalar!("SELECT id FROM ...", ?)` | Existence checks |
| `uuid` | `Uuid::new_v4().to_string()` | Job ID generation |
| `toml` | `toml::from_str::<T>(&string)` | Config file parsing |
| `secrecy` | `Secret::new(s)` / `.expose_secret()` | API keys for Adzuna |
| `ratatui` | `List::new(items).highlight_style(...)` | Job list widget |
| `ratatui` | `ListState::select(Some(i))` | Cursor state |
| `ratatui` | `Paragraph::new(text).wrap(Wrap{trim:true})` | Detail panel |

## Error Handling

```rust
// lazyjob-core/src/discovery/error.rs

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DiscoveryError>;

#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("HTTP fetch failed for source '{source}': {reason}")]
    FetchFailed { source: String, reason: String },

    #[error("response parse failed for source '{source}': {reason}")]
    ParseFailed { source: String, reason: String },

    #[error("rate limit exceeded for source '{source}'")]
    RateLimited { source: String },

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("enrichment step '{step}' failed: {reason}")]
    EnrichmentFailed { step: String, reason: String },

    #[error("source '{name}' not registered")]
    UnknownSource { name: String },

    #[error("missing required config field '{field}' for source '{source}'")]
    MissingConfig { source: String, field: String },

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

**Error propagation strategy:**
- `FetchFailed` and `ParseFailed` from individual sources do **not** abort the entire discovery
  run — they are collected in `DiscoveryReport.errors` and logged at WARN level.
- `Database` errors from `SqliteJobRepository::upsert` are also non-fatal per-job but fatal
  if the pool is completely unreachable.
- `EnrichmentFailed` is non-fatal — the job is skipped and a warning is emitted.
- The TUI displays a status bar warning if `report.errors` is non-empty after a run.

## Testing Strategy

### Unit Tests

**Enrichment pipeline** (`lazyjob-core/src/discovery/enrichment/`):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_script_tags() {
        let html = r#"<script>alert(1)</script><p>Hello <strong>Rust</strong> engineer</p>"#;
        let result = sanitize_description(html);
        assert!(!result.contains("alert"));
        assert!(result.contains("Hello"));
        assert!(result.contains("Rust"));
    }

    #[test]
    fn salary_extracts_range_with_k_suffix() {
        let result = extract_salary("Salary range: $120k-$160k per year");
        let s = result.expect("should find salary");
        assert_eq!(s.min_cents, 120_000 * 100);
        assert_eq!(s.max_cents, 160_000 * 100);
        assert_eq!(s.currency, "USD");
    }

    #[test]
    fn salary_returns_none_for_no_match() {
        let result = extract_salary("Competitive salary based on experience");
        assert!(result.is_none());
    }

    #[test]
    fn remote_classifies_explicit_remote_location() {
        let rt = classify_remote("Senior Engineer", Some("Remote"), "");
        assert_eq!(rt, RemoteType::Yes);
    }

    #[test]
    fn remote_classifies_hybrid_from_description() {
        let rt = classify_remote("Engineer", Some("San Francisco"), "This is a hybrid role");
        assert_eq!(rt, RemoteType::Hybrid);
    }

    #[test]
    fn location_normalization_strips_punctuation() {
        assert_eq!(normalize_location(Some("San Francisco, CA")), "san francisco ca");
        assert_eq!(normalize_location(Some("Remote")), "remote");
        assert_eq!(normalize_location(None), "");
    }

    #[test]
    fn title_normalization_strips_parens() {
        assert_eq!(
            normalize_title("Senior Rust Engineer (Remote)"),
            "senior rust engineer remote"
        );
    }
}
```

**Greenhouse source** with `wiremock`:
```rust
#[tokio::test]
async fn greenhouse_normalizes_raw_job() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/boards/acme/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": 42,
                "title": "Staff Rust Engineer",
                "absolute_url": "https://boards.greenhouse.io/acme/jobs/42",
                "location": { "name": "Remote" },
                "content": "<p>We are hiring a Rust expert...</p>",
                "departments": [{ "name": "Engineering" }],
                "updated_at": null
            }]
        })))
        .mount(&server)
        .await;

    let source = GreenhouseSource::with_base_url(server.uri());
    let jobs = source.fetch_jobs("acme").await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].source_id, "42");
    assert_eq!(jobs[0].title, "Staff Rust Engineer");
    assert!(jobs[0].description_html.as_deref().unwrap().contains("Rust"));
}
```

**Repository upsert deduplication**:
```rust
#[sqlx::test(migrations = "migrations")]
async fn upsert_deduplicates_same_source_id(pool: SqlitePool) {
    let repo = SqliteJobRepository::new(pool);
    let job = make_enriched_job("greenhouse", "123");

    let (outcome1, id1) = repo.upsert(&job).await.unwrap();
    assert_eq!(outcome1, UpsertOutcome::Inserted);

    let (outcome2, id2) = repo.upsert(&job).await.unwrap();
    assert_eq!(outcome2, UpsertOutcome::Unchanged);
    assert_eq!(id1, id2);
}

#[sqlx::test(migrations = "migrations")]
async fn upsert_updates_when_description_changes(pool: SqlitePool) {
    let repo = SqliteJobRepository::new(pool);
    let job1 = make_enriched_job("greenhouse", "123");
    repo.upsert(&job1).await.unwrap();

    let mut job2 = job1.clone();
    job2.description = "Updated job description with new requirements".to_string();
    let (outcome, _) = repo.upsert(&job2).await.unwrap();
    assert_eq!(outcome, UpsertOutcome::Updated);
}
```

### Integration Tests

**File**: `lazyjob-core/tests/discovery_integration.rs`

```rust
/// Full discovery run with mocked HTTP sources and in-memory SQLite.
#[tokio::test]
async fn full_discovery_run_deduplicates() {
    let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Mount wiremock for Greenhouse with 5 job fixtures
    let server = MockServer::start().await;
    // ... mount fixtures ...

    let service = build_test_service(&server, pool.clone());
    let report1 = service.run_discovery().await.unwrap();
    assert_eq!(report1.new_jobs, 5);
    assert_eq!(report1.duplicates, 0);

    // Second run — identical responses — should yield all duplicates
    let report2 = service.run_discovery().await.unwrap();
    assert_eq!(report2.new_jobs, 0);
    assert_eq!(report2.duplicates, 5);
}

#[tokio::test]
async fn cross_source_dedup_removes_lower_priority() {
    // Insert same job from both Greenhouse (priority 0) and Adzuna (priority 2)
    // Assert that after dedup pass, the Adzuna record is soft-deleted
    // and the Greenhouse record remains
}
```

### TUI Tests

```rust
// lazyjob-tui/src/views/jobs_feed.rs (cfg test)
#[test]
fn select_next_wraps_at_end() {
    let jobs = vec![make_job("j1"), make_job("j2"), make_job("j3")];
    let mut view = JobsFeedView::new(jobs);
    view.select_next(); // → 1
    view.select_next(); // → 2
    view.select_next(); // → still 2 (clamped)
    assert_eq!(view.list_state.selected(), Some(2));
}
```

## Open Questions

1. **Adzuna in MVP?** Free tier limits to 250 req/month (adequate for 1/day runs across
   10 companies). Recommend deferring `AdzunaSource` to Phase 2 and gating it behind
   `config.adzuna` presence.

2. **Cross-source dedup threshold**: The plan uses `jaro_winkler >= 0.85` for uncertain
   and `>= 0.96` for confident dedup. Needs empirical validation against real Greenhouse
   + Adzuna data. Consider adding a `discovery_dedup_candidates` table to capture
   uncertain pairs for manual review.

3. **Stale listing TTL**: The spec proposes 60 days for ghost detection, 30 days for
   staleness badge. This plan does not implement TTL-based `status` transitions — that
   belongs in the ghost-job-detection spec. However, the `jobs` schema includes
   `updated_at` and `posted_at` fields needed for that feature.

4. **Pagination**: Greenhouse returns all jobs in one response (up to 200). For large
   boards, add a `?page=N` loop. Defer — add when first user reports truncation.

5. **Company registry sync with SQLite**: Currently `CompanyRegistry` is loaded from
   config.toml. Should it also sync to the `discovery_companies` SQLite table so the
   TUI can display last-fetched timestamps? Recommend yes — add a
   `sync_company_registry()` step at startup.

6. **Ralph concurrency**: Multiple `DiscoveryService::run_discovery()` calls must not
   run concurrently (would double-insert in a race). Protect with a per-loop
   `tokio::sync::Mutex<()>` guard in the Ralph orchestration layer.

7. **Embedding trigger**: After upsert, the plan does not trigger embedding generation.
   The semantic-matching plan should add a background task that polls for
   `jobs WHERE embedding_updated_at IS NULL` and enqueues them.

## Related Specs

- `specs/11-platform-api-integrations-implementation-plan.md` — HTTP client implementations
- `specs/job-search-semantic-matching.md` — downstream embedding + vector ranking
- `specs/job-search-ghost-job-detection.md` — staleness and repost detection
- `specs/agentic-ralph-orchestration-implementation-plan.md` — discovery loop scheduling
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — IPC for Ralph discovery loop
- `specs/04-sqlite-persistence-implementation-plan.md` — database initialization and migration runner
- `specs/09-tui-design-keybindings-implementation-plan.md` — keybind integration for manual refresh
