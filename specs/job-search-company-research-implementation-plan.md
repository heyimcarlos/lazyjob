# Implementation Plan: Company Research Pipeline

## Status
Draft

## Related Spec
`specs/job-search-company-research.md`

## Overview

The Company Research Pipeline builds and maintains a `CompanyRecord` for each company
the user encounters during job search. It aggregates data from public sources — company
website About/Careers pages, tech stack inference from job descriptions, and (in Phase 2)
Google News RSS, Glassdoor, Crunchbase, and layoffs.fyi — and stores a normalized,
enriched record in SQLite. The `CompanyRecord` is the **canonical company entity** for
all of LazyJob: ghost detection reads headcount signals from it, cover letter generation
uses mission statement and culture signals, interview preparation surfaces the full record
as a cheat sheet.

This plan defines four independent modules: (1) `CompanyRepository` — SQLite persistence
with staleness tracking; (2) `CompanyNormalizer` — deterministic name-normalization and
suffix stripping; (3) `CompanyResearcher` — async HTTP + LLM extraction orchestrator;
(4) `CompanyService` — high-level API consumed by discovery, ralph loops, and the TUI.
It also defines the Phase 2 `NewsAggregator` and `GlassdoorScraper` stubs.

The pipeline is deliberately invoked asynchronously after job ingestion so it never
blocks the discovery loop. It can also be triggered manually from the TUI (`r` key on
a company record) and is scheduled for daily stale-record refresh by the ralph scheduler.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`,
  migration runner, `#[sqlx::test]` pattern
- `specs/job-search-discovery-engine-implementation-plan.md` — `DiscoveredJob`,
  `CompanyConfig`, `JobRepository`, enrichment pipeline
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait,
  `ChatMessage`, `RenderedPrompt`
- `specs/17-ralph-prompt-templates-implementation-plan.md` — `TemplateRegistry`,
  `LoopType::CompanyResearch`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
# Already present from discovery engine plan
reqwest      = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip"] }
scraper      = "0.19"     # HTML parsing for About/Careers pages
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
chrono       = { version = "0.4", features = ["serde"] }
uuid         = { version = "1", features = ["v4", "serde"] }
tracing      = "0.1"
thiserror    = "2"
anyhow       = "1"
tokio        = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
async-trait  = "0.1"
once_cell    = "1"
regex        = "1"
strsim       = "0.11"    # Jaro-Winkler for fuzzy company name matching
ammonia      = "4"       # HTML sanitizer for LLM context window

# New for company research
rss          = "2"        # Google News / RSS feed parsing (Phase 2)

[dev-dependencies]
wiremock     = "0.6"
sqlx         = { version = "0.8", features = ["sqlite", "runtime-tokio", "migrate", "macros"] }
tempfile     = "3"
tokio        = { version = "1", features = ["full"] }
```

## Architecture

### Crate Placement

All company research code lives in `lazyjob-core/src/companies/`. This crate is
imported by:
- `lazyjob-ralph` — `CompanyService::enrich()` called after discovery loop ingestion,
  and by the daily stale-refresh ralph loop
- `lazyjob-tui` — reads `CompanyRecord` via `CompanyRepository::get_by_id()` for the
  company detail panel; triggers manual refresh via `CompanyService::enrich_now()`
- `lazyjob-core/src/discovery/ghost_detection/` — reads `employee_count_range` and
  `recent_layoffs` from `CompanyRepository`
- `lazyjob-core/src/applications/cover_letter.rs` — reads `mission_statement`,
  `core_values`, `culture_signals` from `CompanyRepository`

The `CompanyRecord` struct is exported from `lazyjob-core` as a first-class public type
since multiple crates depend on it.

### Core Types

```rust
// lazyjob-core/src/companies/models.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical company record. This is the single source of truth for all company
/// data in LazyJob. Downstream features (ghost detection, cover letters, interview
/// prep) query this record via `CompanyRepository`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyRecord {
    pub id: Uuid,

    /// Display name as-seen in Greenhouse / Lever / config.
    pub name: String,

    /// Lowercase, suffix-stripped, whitespace-collapsed canonical name.
    /// Used for dedup and fuzzy matching.
    pub name_normalized: String,

    pub website_url: Option<String>,

    // Discovery-layer linkage keys
    pub greenhouse_token: Option<String>,
    pub lever_id: Option<String>,

    // --- Phase 1 enriched fields ---
    pub description: Option<String>,
    pub mission_statement: Option<String>,
    pub core_values: Vec<String>,
    pub tech_stack: Vec<String>,
    pub product_areas: Vec<String>,
    pub culture_signals: Vec<String>,
    pub employee_count_range: Option<EmployeeCountRange>,
    pub founded_year: Option<u16>,
    pub hq_location: Option<String>,

    // --- Phase 2 enriched fields ---
    pub funding_stage: Option<FundingStage>,
    pub glassdoor_rating: Option<f32>,
    pub glassdoor_pros: Vec<String>,
    pub glassdoor_cons: Vec<String>,
    pub recent_news: Vec<NewsItem>,

    /// True if a layoffs event for this company appeared in layoffs.fyi
    /// within the last 90 days.
    pub recent_layoffs: bool,

    // --- Metadata ---
    /// When the LLM extraction was last performed.
    pub enriched_at: Option<DateTime<Utc>>,

    /// Which sources contributed data in the last enrichment run.
    pub enrichment_sources: Vec<EnrichmentSource>,

    /// Computed lazily: true if `enriched_at` is None or > 7 days ago.
    /// Not stored in SQLite — computed by repository on read.
    #[serde(skip_serializing)]
    pub is_stale: bool,
}

/// Discrete employee count bucket. Stored as TEXT in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmployeeCountRange {
    Tiny,       // 1–10
    Small,      // 11–50
    Medium,     // 51–200
    Large,      // 201–1000
    Enterprise, // 1001+
    Unknown,
}

impl EmployeeCountRange {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::Tiny       => "tiny",
            Self::Small      => "small",
            Self::Medium     => "medium",
            Self::Large      => "large",
            Self::Enterprise => "enterprise",
            Self::Unknown    => "unknown",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "tiny"       => Self::Tiny,
            "small"      => Self::Small,
            "medium"     => Self::Medium,
            "large"      => Self::Large,
            "enterprise" => Self::Enterprise,
            _            => Self::Unknown,
        }
    }

    /// Infer a bucket from a raw integer headcount (from API metadata).
    pub fn from_count(n: u32) -> Self {
        match n {
            0            => Self::Unknown,
            1..=10       => Self::Tiny,
            11..=50      => Self::Small,
            51..=200     => Self::Medium,
            201..=1000   => Self::Large,
            _            => Self::Enterprise,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FundingStage {
    PreSeed, Seed, SeriesA, SeriesB, SeriesC, SeriesD,
    Public, Bootstrapped, Unknown,
}

impl FundingStage {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::PreSeed     => "pre_seed",
            Self::Seed        => "seed",
            Self::SeriesA     => "series_a",
            Self::SeriesB     => "series_b",
            Self::SeriesC     => "series_c",
            Self::SeriesD     => "series_d",
            Self::Public      => "public",
            Self::Bootstrapped => "bootstrapped",
            Self::Unknown     => "unknown",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "pre_seed"     => Self::PreSeed,
            "seed"         => Self::Seed,
            "series_a"     => Self::SeriesA,
            "series_b"     => Self::SeriesB,
            "series_c"     => Self::SeriesC,
            "series_d"     => Self::SeriesD,
            "public"       => Self::Public,
            "bootstrapped" => Self::Bootstrapped,
            _              => Self::Unknown,
        }
    }
}

/// A single news article captured during Phase 2 enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub snippet: String,
    pub source_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrichmentSource {
    CompanyWebsite,
    JobDescriptionInference,
    Glassdoor,
    Crunchbase,
    LayoffsFyi,
    NewsRss,
}

/// Structured output from the LLM extraction step.
/// Deserialized from the LLM's JSON response.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CompanyExtractionResult {
    pub description: Option<String>,
    pub mission_statement: Option<String>,
    pub core_values: Vec<String>,
    pub tech_stack: Vec<String>,
    pub product_areas: Vec<String>,
    pub culture_signals: Vec<String>,
    /// Optional: LLM may extract employee range from text mentions.
    pub employee_count_range: Option<String>,
    pub founded_year: Option<u16>,
    pub hq_location: Option<String>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/companies/repository.rs

use async_trait::async_trait;
use chrono::Duration;
use uuid::Uuid;

use crate::companies::models::CompanyRecord;

pub type CompanyResult<T> = Result<T, CompanyError>;

#[async_trait]
pub trait CompanyRepository: Send + Sync {
    /// Lookup by normalized name. Returns None if not found.
    async fn get_by_name(&self, name_normalized: &str) -> CompanyResult<Option<CompanyRecord>>;

    /// Lookup by primary key.
    async fn get_by_id(&self, id: Uuid) -> CompanyResult<Option<CompanyRecord>>;

    /// Lookup by Greenhouse board token.
    async fn get_by_greenhouse_token(&self, token: &str) -> CompanyResult<Option<CompanyRecord>>;

    /// Lookup by Lever company slug.
    async fn get_by_lever_id(&self, lever_id: &str) -> CompanyResult<Option<CompanyRecord>>;

    /// Insert or update by `name_normalized`. Returns the stored record.
    async fn upsert(&self, record: &CompanyRecord) -> CompanyResult<CompanyRecord>;

    /// Returns companies whose `enriched_at` is NULL or older than `older_than`.
    async fn list_stale(&self, older_than: Duration) -> CompanyResult<Vec<CompanyRecord>>;

    /// Returns all companies matching a fuzzy search term (for TUI company list).
    async fn search(&self, query: &str, limit: u32) -> CompanyResult<Vec<CompanyRecord>>;

    /// Returns all companies linked to a specific job (via company_id on jobs table).
    async fn list_all(&self) -> CompanyResult<Vec<CompanyRecord>>;
}

// lazyjob-core/src/companies/researcher.rs

#[async_trait]
pub trait CompanyEnricher: Send + Sync {
    /// Full enrichment pass for one company. Returns the updated record.
    async fn enrich(&self, record: &CompanyRecord) -> CompanyResult<CompanyRecord>;
}
```

### SQLite Schema

```sql
-- migration: 007_company_research.sql

CREATE TABLE IF NOT EXISTS companies (
    id                  TEXT PRIMARY KEY,   -- UUID v4
    name                TEXT NOT NULL,
    name_normalized     TEXT NOT NULL UNIQUE,
    website_url         TEXT,
    greenhouse_token    TEXT,
    lever_id            TEXT,

    -- Enriched data stored as JSON arrays / scalars
    description         TEXT,
    mission_statement   TEXT,
    core_values         TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    tech_stack          TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    product_areas       TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    culture_signals     TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
    employee_count_range TEXT,
    funding_stage       TEXT,
    founded_year        INTEGER,
    hq_location         TEXT,
    glassdoor_rating    REAL,
    glassdoor_pros      TEXT NOT NULL DEFAULT '[]',  -- JSON array
    glassdoor_cons      TEXT NOT NULL DEFAULT '[]',  -- JSON array
    recent_news         TEXT NOT NULL DEFAULT '[]',  -- JSON array of NewsItem
    recent_layoffs      INTEGER NOT NULL DEFAULT 0,  -- boolean

    enriched_at         TEXT,  -- RFC 3339 datetime
    enrichment_sources  TEXT NOT NULL DEFAULT '[]',  -- JSON array of EnrichmentSource

    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_companies_greenhouse_token
    ON companies (greenhouse_token) WHERE greenhouse_token IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_companies_lever_id
    ON companies (lever_id) WHERE lever_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_companies_enriched_at
    ON companies (enriched_at);  -- NULL-first for list_stale() efficiency

-- Track per-field staleness for Phase 3 fine-grained refresh scheduling.
-- Not used in Phase 1; added here so the schema is forward-compatible.
CREATE TABLE IF NOT EXISTS company_enrichment_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    company_id    TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    source        TEXT NOT NULL,      -- EnrichmentSource db string
    field_set     TEXT NOT NULL,      -- which fields were updated (JSON array)
    enriched_at   TEXT NOT NULL,
    success       INTEGER NOT NULL DEFAULT 1,
    error_message TEXT
);

CREATE INDEX IF NOT EXISTS idx_enrichment_log_company
    ON company_enrichment_log (company_id, enriched_at DESC);
```

### Module Structure

```
lazyjob-core/
  src/
    companies/
      mod.rs            # Re-exports: CompanyRecord, CompanyRepository, CompanyService
      models.rs         # CompanyRecord, EmployeeCountRange, FundingStage, NewsItem, …
      normalizer.rs     # CompanyNormalizer: name normalization, suffix stripping
      repository.rs     # CompanyRepository trait + SqliteCompanyRepository impl
      researcher.rs     # CompanyResearcher: HTTP fetch + LLM extraction
      tech_stack.rs     # TechStackLexicon: offline regex inference from job descriptions
      service.rs        # CompanyService: high-level API (enrich, refresh_stale, search)
      news.rs           # NewsAggregator (Phase 2): RSS / Google News fetcher
      error.rs          # CompanyError enum
```

## Implementation Phases

### Phase 1 — Core Domain: Types, Normalizer, Repository, Tech Stack Lexicon (MVP)

**Goal**: Standing up the `CompanyRecord` SQLite schema, repository, and the offline
tech-stack inference pass. No LLM calls yet — proves the data model is correct.

#### Step 1.1 — `CompanyError` enum

File: `lazyjob-core/src/companies/error.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompanyError {
    #[error("Company not found: {name_normalized}")]
    NotFound { name_normalized: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("HTTP fetch failed for {url}: {source}")]
    HttpFetch { url: String, #[source] source: reqwest::Error },

    #[error("LLM extraction failed: {0}")]
    LlmExtraction(#[from] anyhow::Error),

    #[error("JSON deserialization failed: {0}")]
    JsonDeser(#[from] serde_json::Error),

    #[error("Invalid employee count range: {0}")]
    InvalidCountRange(String),
}

pub type CompanyResult<T> = Result<T, CompanyError>;
```

#### Step 1.2 — `CompanyNormalizer`

File: `lazyjob-core/src/companies/normalizer.rs`

Normalizes company names to a canonical form for deduplication and fuzzy matching.

```rust
use once_cell::sync::Lazy;
use regex::Regex;

/// Legal suffixes to strip during normalization.
static SUFFIX_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i),?\s+(inc\.?|llc\.?|corp\.?|ltd\.?|co\.?|plc\.?|gmbh|s\.a\.?|b\.v\.?|ag)\s*$"
    ).unwrap()
});

/// Collapse multiple whitespace characters into one.
static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

pub struct CompanyNormalizer;

impl CompanyNormalizer {
    /// Returns a lowercase, suffix-stripped, whitespace-collapsed name.
    ///
    /// "Stripe, Inc." → "stripe"
    /// "OpenAI B.V."  → "openai"
    /// "Meta Platforms LLC" → "meta platforms"
    pub fn normalize(name: &str) -> String {
        let stripped = SUFFIX_PATTERN.replace(name, "");
        let lower = stripped.to_lowercase();
        WHITESPACE.replace_all(lower.trim(), " ").to_string()
    }

    /// Returns `true` if `a` and `b` refer to the same company.
    ///
    /// First tries exact normalized match; falls back to
    /// Jaro-Winkler similarity ≥ 0.92 for abbreviations / spacing variants.
    pub fn is_same_company(a: &str, b: &str) -> bool {
        let na = Self::normalize(a);
        let nb = Self::normalize(b);
        if na == nb {
            return true;
        }
        strsim::jaro_winkler(&na, &nb) >= 0.92
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_inc_suffix() {
        assert_eq!(CompanyNormalizer::normalize("Stripe, Inc."), "stripe");
    }

    #[test]
    fn strips_llc_suffix() {
        assert_eq!(CompanyNormalizer::normalize("OpenAI LLC"), "openai");
    }

    #[test]
    fn preserves_multi_word() {
        assert_eq!(
            CompanyNormalizer::normalize("Meta Platforms, Inc."),
            "meta platforms"
        );
    }

    #[test]
    fn fuzzy_match_abbreviation() {
        assert!(CompanyNormalizer::is_same_company("Stripe Inc", "Stripe"));
    }
}
```

#### Step 1.3 — `TechStackLexicon` (offline inference)

File: `lazyjob-core/src/companies/tech_stack.rs`

Uses a compiled regex of known technology terms to extract the tech stack from job
description text — no external API calls required.

```rust
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Full-word regex matching known technology terms.
/// Extend this list as new technologies become relevant.
static TECH_PATTERN: Lazy<Regex> = Lazy::new(|| {
    let terms = [
        // Languages
        "Rust", "Go", "Python", "TypeScript", "JavaScript", "Java", "Kotlin",
        "Swift", "C\\+\\+", "C#", "Ruby", "Scala", "Elixir", "Haskell", "OCaml",
        // Databases
        "PostgreSQL", "MySQL", "SQLite", "Redis", "Cassandra", "DynamoDB",
        "MongoDB", "Elasticsearch", "ClickHouse", "Snowflake", "BigQuery",
        // Infrastructure / Cloud
        "Kubernetes", "Docker", "AWS", "GCP", "Azure", "Terraform", "Pulumi",
        "Kafka", "RabbitMQ", "NATS", "gRPC", "GraphQL", "REST",
        // Frameworks
        "React", "Next\\.js", "Vue", "Angular", "Svelte", "Axum", "Actix",
        "FastAPI", "Django", "Rails", "Spring", "Gin", "Echo",
        // ML / AI
        "PyTorch", "TensorFlow", "JAX", "ONNX", "LangChain", "LlamaIndex",
        // Observability
        "Prometheus", "Grafana", "Datadog", "OpenTelemetry", "Jaeger",
    ];
    let pattern = terms
        .iter()
        .map(|t| format!(r"(?i)\b{}\b", t))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&pattern).unwrap()
});

pub struct TechStackLexicon;

impl TechStackLexicon {
    /// Extract unique technology mentions from one or more job description strings.
    /// Returns results sorted and deduplicated (case-insensitive).
    pub fn extract(descriptions: &[&str]) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        for desc in descriptions {
            for m in TECH_PATTERN.find_iter(desc) {
                seen.insert(m.as_str().to_string());
            }
        }
        let mut result: Vec<String> = seen.into_iter().collect();
        result.sort();
        result
    }
}
```

#### Step 1.4 — `SqliteCompanyRepository`

File: `lazyjob-core/src/companies/repository.rs`

The repository maps `CompanyRecord` to/from SQLite rows. JSON arrays are stored as
TEXT columns using `serde_json::to_string` / `from_str`.

```rust
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{models::*, error::CompanyResult, CompanyError};

pub struct SqliteCompanyRepository {
    pool: SqlitePool,
}

impl SqliteCompanyRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn compute_is_stale(enriched_at: Option<DateTime<Utc>>) -> bool {
        match enriched_at {
            None => true,
            Some(t) => Utc::now() - t > Duration::days(7),
        }
    }
}

#[async_trait]
impl CompanyRepository for SqliteCompanyRepository {
    async fn get_by_name(&self, name_normalized: &str) -> CompanyResult<Option<CompanyRecord>> {
        let row = sqlx::query!(
            "SELECT * FROM companies WHERE name_normalized = ?",
            name_normalized
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| row_to_record(r)))
    }

    async fn get_by_id(&self, id: Uuid) -> CompanyResult<Option<CompanyRecord>> {
        let id_str = id.to_string();
        let row = sqlx::query!(
            "SELECT * FROM companies WHERE id = ?",
            id_str
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| row_to_record(r)))
    }

    async fn get_by_greenhouse_token(&self, token: &str) -> CompanyResult<Option<CompanyRecord>> {
        let row = sqlx::query!(
            "SELECT * FROM companies WHERE greenhouse_token = ?",
            token
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_to_record(r)))
    }

    async fn get_by_lever_id(&self, lever_id: &str) -> CompanyResult<Option<CompanyRecord>> {
        let row = sqlx::query!(
            "SELECT * FROM companies WHERE lever_id = ?",
            lever_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| row_to_record(r)))
    }

    async fn upsert(&self, record: &CompanyRecord) -> CompanyResult<CompanyRecord> {
        let id_str = record.id.to_string();
        let core_values = serde_json::to_string(&record.core_values)?;
        let tech_stack  = serde_json::to_string(&record.tech_stack)?;
        let product_areas = serde_json::to_string(&record.product_areas)?;
        let culture_signals = serde_json::to_string(&record.culture_signals)?;
        let glassdoor_pros = serde_json::to_string(&record.glassdoor_pros)?;
        let glassdoor_cons = serde_json::to_string(&record.glassdoor_cons)?;
        let recent_news  = serde_json::to_string(&record.recent_news)?;
        let enrichment_sources = serde_json::to_string(&record.enrichment_sources)?;
        let employee_str = record.employee_count_range.as_ref()
            .map(|e| e.to_db_str().to_string());
        let funding_str  = record.funding_stage.as_ref()
            .map(|f| f.to_db_str().to_string());
        let enriched_at_str = record.enriched_at.map(|t| t.to_rfc3339());
        let recent_layoffs_i = record.recent_layoffs as i64;

        sqlx::query!(
            r#"
            INSERT INTO companies (
                id, name, name_normalized, website_url, greenhouse_token, lever_id,
                description, mission_statement, core_values, tech_stack, product_areas,
                culture_signals, employee_count_range, funding_stage, founded_year,
                hq_location, glassdoor_rating, glassdoor_pros, glassdoor_cons,
                recent_news, recent_layoffs, enriched_at, enrichment_sources, updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?, datetime('now')
            )
            ON CONFLICT(name_normalized) DO UPDATE SET
                name                 = excluded.name,
                website_url          = excluded.website_url,
                greenhouse_token     = COALESCE(excluded.greenhouse_token, companies.greenhouse_token),
                lever_id             = COALESCE(excluded.lever_id, companies.lever_id),
                description          = COALESCE(excluded.description, companies.description),
                mission_statement    = COALESCE(excluded.mission_statement, companies.mission_statement),
                core_values          = excluded.core_values,
                tech_stack           = excluded.tech_stack,
                product_areas        = excluded.product_areas,
                culture_signals      = excluded.culture_signals,
                employee_count_range = COALESCE(excluded.employee_count_range, companies.employee_count_range),
                funding_stage        = COALESCE(excluded.funding_stage, companies.funding_stage),
                founded_year         = COALESCE(excluded.founded_year, companies.founded_year),
                hq_location          = COALESCE(excluded.hq_location, companies.hq_location),
                glassdoor_rating     = COALESCE(excluded.glassdoor_rating, companies.glassdoor_rating),
                glassdoor_pros       = excluded.glassdoor_pros,
                glassdoor_cons       = excluded.glassdoor_cons,
                recent_news          = excluded.recent_news,
                recent_layoffs       = excluded.recent_layoffs,
                enriched_at          = excluded.enriched_at,
                enrichment_sources   = excluded.enrichment_sources,
                updated_at           = datetime('now')
            "#,
            id_str, record.name, record.name_normalized, record.website_url,
            record.greenhouse_token, record.lever_id,
            record.description, record.mission_statement, core_values,
            tech_stack, product_areas, culture_signals, employee_str, funding_str,
            record.founded_year, record.hq_location, record.glassdoor_rating,
            glassdoor_pros, glassdoor_cons, recent_news, recent_layoffs_i,
            enriched_at_str, enrichment_sources
        )
        .execute(&self.pool)
        .await?;

        // Return the freshly-written record from DB to pick up ON CONFLICT merges.
        self.get_by_name(&record.name_normalized)
            .await?
            .ok_or_else(|| CompanyError::NotFound {
                name_normalized: record.name_normalized.clone(),
            })
    }

    async fn list_stale(&self, older_than: Duration) -> CompanyResult<Vec<CompanyRecord>> {
        let cutoff = (Utc::now() - older_than).to_rfc3339();
        let rows = sqlx::query!(
            r#"
            SELECT * FROM companies
            WHERE enriched_at IS NULL
               OR enriched_at < ?
            ORDER BY enriched_at ASC NULLS FIRST
            LIMIT 50
            "#,
            cutoff
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_record).collect())
    }

    async fn search(&self, query: &str, limit: u32) -> CompanyResult<Vec<CompanyRecord>> {
        let pattern = format!("%{}%", query.to_lowercase());
        let rows = sqlx::query!(
            "SELECT * FROM companies WHERE name_normalized LIKE ? ORDER BY name LIMIT ?",
            pattern,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_record).collect())
    }

    async fn list_all(&self) -> CompanyResult<Vec<CompanyRecord>> {
        let rows = sqlx::query!("SELECT * FROM companies ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(row_to_record).collect())
    }
}

/// Convert a sqlx query! row macro result into a `CompanyRecord`.
/// The `row_to_record` function is private to the repository module.
fn row_to_record(r: /* sqlx query! anonymous type */ impl CompanyRow) -> CompanyRecord {
    // NOTE: In practice the sqlx query! macro produces an anonymous struct.
    // The conversion expands to field reads + serde_json::from_str for JSON columns.
    // Shown here in pseudocode for clarity; actual code uses the macro-generated type.
    let enriched_at = r.enriched_at
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.with_timezone(&Utc));

    CompanyRecord {
        id: Uuid::parse_str(&r.id).unwrap_or_else(|_| Uuid::nil()),
        name: r.name,
        name_normalized: r.name_normalized,
        website_url: r.website_url,
        greenhouse_token: r.greenhouse_token,
        lever_id: r.lever_id,
        description: r.description,
        mission_statement: r.mission_statement,
        core_values: serde_json::from_str(&r.core_values).unwrap_or_default(),
        tech_stack: serde_json::from_str(&r.tech_stack).unwrap_or_default(),
        product_areas: serde_json::from_str(&r.product_areas).unwrap_or_default(),
        culture_signals: serde_json::from_str(&r.culture_signals).unwrap_or_default(),
        employee_count_range: r.employee_count_range.map(|s| EmployeeCountRange::from_db_str(&s)),
        funding_stage: r.funding_stage.map(|s| FundingStage::from_db_str(&s)),
        founded_year: r.founded_year.map(|y| y as u16),
        hq_location: r.hq_location,
        glassdoor_rating: r.glassdoor_rating.map(|v| v as f32),
        glassdoor_pros: serde_json::from_str(&r.glassdoor_pros).unwrap_or_default(),
        glassdoor_cons: serde_json::from_str(&r.glassdoor_cons).unwrap_or_default(),
        recent_news: serde_json::from_str(&r.recent_news).unwrap_or_default(),
        recent_layoffs: r.recent_layoffs != 0,
        enriched_at,
        enrichment_sources: serde_json::from_str(&r.enrichment_sources).unwrap_or_default(),
        is_stale: SqliteCompanyRepository::compute_is_stale(enriched_at),
    }
}
```

**Verification**: `cargo test -p lazyjob-core companies::repository` with
`#[sqlx::test(migrations = "migrations")]` tests. Confirm upsert merges fields correctly
(Greenhouse token from first upsert is preserved on second upsert that sets lever_id).

---

### Phase 2 — LLM Extraction: `CompanyResearcher`

**Goal**: HTTP fetch of company About/Careers pages, HTML sanitization, and LLM
structured extraction into `CompanyExtractionResult`. Produces `EnrichmentSource::CompanyWebsite`.

#### Step 2.1 — HTML Fetcher

File: `lazyjob-core/src/companies/researcher.rs`

```rust
use ammonia::Builder as AmmoniaBuilder;
use reqwest::Client;

/// Fetch raw HTML from a URL, sanitize it to plain text for LLM ingestion.
///
/// Uses `ammonia` in allowlist mode to strip all tags; the resulting text is
/// ~60-80% smaller than raw HTML and avoids injecting HTML entities into the prompt.
pub async fn fetch_page_text(
    http: &Client,
    url: &str,
    max_chars: usize,
) -> Result<String, CompanyError> {
    let response = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; LazyJob/1.0)")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| CompanyError::HttpFetch { url: url.to_string(), source: e })?;

    let html = response
        .text()
        .await
        .map_err(|e| CompanyError::HttpFetch { url: url.to_string(), source: e })?;

    // Strip all HTML tags, keep only text content.
    let text = AmmoniaBuilder::empty()
        .tags(std::collections::HashSet::new())  // allowlist is empty → strip all tags
        .clean(&html)
        .to_string();

    // Truncate to avoid bloating the LLM context window.
    let truncated: String = text.chars().take(max_chars).collect();
    Ok(truncated)
}
```

#### Step 2.2 — LLM Extraction Prompt

The extraction prompt is embedded at compile time and follows the anti-fabrication
constraint from the spec. Stored as `lazyjob-llm/templates/company_research.toml`:

```toml
[company_research]
system = """
You are a structured data extractor. Your job is to extract factual company information
from the provided web page text.

RULES:
- Extract ONLY information that is explicitly stated in the source text.
- Do NOT infer, guess, or expand beyond what is written.
- Set any field to null if the information is not clearly present.
- tech_stack must contain only specific technology names (e.g. "Rust", "PostgreSQL"),
  not generic terms (e.g. "modern database", "fast language").
- core_values and culture_signals must be verbatim phrases or paraphrases from the text,
  not generic workplace clichés you invented.

Respond with a JSON object matching this schema exactly:
{
  "description": "<2-3 sentence company description or null>",
  "mission_statement": "<mission/vision statement or null>",
  "core_values": ["<value>", ...],
  "tech_stack": ["<Technology>", ...],
  "product_areas": ["<product area>", ...],
  "culture_signals": ["<culture phrase>", ...],
  "employee_count_range": "<tiny|small|medium|large|enterprise|null>",
  "founded_year": <integer or null>,
  "hq_location": "<City, Country or null>"
}
"""

user_template = """
Extract structured company information from the following web page text.

Company name: {company_name}
Source URL: {source_url}

--- BEGIN PAGE TEXT ---
{page_text}
--- END PAGE TEXT ---
"""
```

#### Step 2.3 — `CompanyResearcher` struct

```rust
// lazyjob-core/src/companies/researcher.rs

use std::sync::Arc;
use crate::llm::{ChatMessage, LlmProvider, Role};
use super::{models::*, error::CompanyResult, CompanyError};

/// Concurrency-safe enricher. Holds an HTTP client and LLM provider.
pub struct CompanyResearcher {
    llm: Arc<dyn LlmProvider>,
    http: reqwest::Client,
    /// Maximum characters of page text to send to the LLM.
    /// Default: 12_000 characters ≈ ~3000 tokens at 4 chars/token.
    max_page_chars: usize,
}

impl CompanyResearcher {
    pub fn new(llm: Arc<dyn LlmProvider>, http: reqwest::Client) -> Self {
        Self { llm, http, max_page_chars: 12_000 }
    }

    /// Construct with a custom HTTP base URL — used in tests with `wiremock`.
    #[cfg(test)]
    pub fn with_http(llm: Arc<dyn LlmProvider>, http: reqwest::Client) -> Self {
        Self { llm, http, max_page_chars: 4_000 }
    }

    /// Run the full enrichment pipeline for a company.
    ///
    /// 1. Fetch About page (required).
    /// 2. Optionally fetch Careers page if About URL is provided.
    /// 3. Send combined text to LLM for structured extraction.
    /// 4. Merge with tech stack inference from `job_descriptions`.
    /// 5. Return the updated `CompanyRecord`.
    pub async fn enrich(
        &self,
        record: &CompanyRecord,
        job_descriptions: &[&str],
    ) -> CompanyResult<CompanyRecord> {
        let website_url = match &record.website_url {
            Some(u) => u.clone(),
            None => {
                tracing::warn!(
                    company = %record.name,
                    "No website URL configured — skipping LLM enrichment"
                );
                return Ok(record.clone());
            }
        };

        // Attempt to identify an About page URL.
        let about_url = derive_about_url(&website_url);
        let careers_url = derive_careers_url(&website_url);

        // Fetch pages concurrently, tolerate individual failures.
        let (about_text, careers_text) = tokio::join!(
            fetch_page_text(&self.http, &about_url, self.max_page_chars),
            fetch_page_text(&self.http, &careers_url, self.max_page_chars / 2),
        );

        let combined_text = match (about_text, careers_text) {
            (Ok(a), Ok(c))  => format!("{}\n\n---CAREERS---\n\n{}", a, c),
            (Ok(a), Err(_)) => a,
            (Err(_), Ok(c)) => c,
            (Err(e), Err(_)) => {
                tracing::error!(
                    company = %record.name,
                    error = %e,
                    "Failed to fetch any page for company — skipping enrichment"
                );
                return Ok(record.clone());
            }
        };

        let extraction = self.extract_from_text(&record.name, &about_url, &combined_text).await?;

        // Offline tech stack inference — free and never fails.
        let inferred_stack = TechStackLexicon::extract(job_descriptions);

        // Merge: LLM-extracted stack + offline-inferred stack, deduplicated.
        let mut merged_stack = extraction.tech_stack.clone();
        for t in inferred_stack {
            if !merged_stack.contains(&t) {
                merged_stack.push(t);
            }
        }
        merged_stack.sort();

        let mut updated = record.clone();
        updated.description         = extraction.description.or(updated.description);
        updated.mission_statement   = extraction.mission_statement.or(updated.mission_statement);
        updated.core_values         = extraction.core_values;
        updated.tech_stack          = merged_stack;
        updated.product_areas       = extraction.product_areas;
        updated.culture_signals     = extraction.culture_signals;
        updated.employee_count_range = extraction.employee_count_range
            .as_deref()
            .map(EmployeeCountRange::from_db_str)
            .or(updated.employee_count_range);
        updated.founded_year        = extraction.founded_year.or(updated.founded_year);
        updated.hq_location         = extraction.hq_location.or(updated.hq_location);
        updated.enriched_at         = Some(chrono::Utc::now());
        updated.enrichment_sources  = vec![
            EnrichmentSource::CompanyWebsite,
            EnrichmentSource::JobDescriptionInference,
        ];

        Ok(updated)
    }

    /// Send page text to the LLM and parse the JSON response.
    pub async fn extract_from_text(
        &self,
        company_name: &str,
        source_url: &str,
        page_text: &str,
    ) -> CompanyResult<CompanyExtractionResult> {
        let system_msg = COMPANY_RESEARCH_SYSTEM_PROMPT.to_string();
        let user_msg = COMPANY_RESEARCH_USER_TEMPLATE
            .replace("{company_name}", company_name)
            .replace("{source_url}", source_url)
            .replace("{page_text}", page_text);

        let response = self.llm
            .chat(&[
                ChatMessage { role: Role::System, content: system_msg },
                ChatMessage { role: Role::User, content: user_msg },
            ])
            .await
            .map_err(|e| CompanyError::LlmExtraction(e.into()))?;

        let result: CompanyExtractionResult = serde_json::from_str(&response.content)
            .map_err(CompanyError::JsonDeser)?;
        Ok(result)
    }
}

/// Append `/about` to the base URL, strip trailing slashes first.
fn derive_about_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{}/about", trimmed)
}

/// Append `/careers` to the base URL.
fn derive_careers_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{}/careers", trimmed)
}

static COMPANY_RESEARCH_SYSTEM_PROMPT: &str = include_str!(
    "../../lazyjob-llm/templates/company_research_system.txt"
);

static COMPANY_RESEARCH_USER_TEMPLATE: &str = include_str!(
    "../../lazyjob-llm/templates/company_research_user.txt"
);
```

**Verification**: `cargo test -p lazyjob-core companies::researcher` with `wiremock`
serving a mocked company website HTML page. Assert that `extract_from_text` produces a
non-empty `tech_stack` when the page contains "We use Rust and PostgreSQL".

---

### Phase 3 — `CompanyService`: Orchestration and Staleness Refresh

**Goal**: High-level API that ties together the repository and researcher. This is what
ralph loops and the TUI call.

#### Step 3.1 — `CompanyService`

```rust
// lazyjob-core/src/companies/service.rs

use std::sync::Arc;
use chrono::Duration;
use tracing::{info, warn};

use super::{
    models::CompanyRecord,
    repository::CompanyRepository,
    researcher::CompanyResearcher,
    normalizer::CompanyNormalizer,
    error::{CompanyResult, CompanyError},
};

pub struct CompanyService {
    repo: Arc<dyn CompanyRepository>,
    researcher: Arc<CompanyResearcher>,
    stale_threshold_days: u32,
}

impl CompanyService {
    pub fn new(
        repo: Arc<dyn CompanyRepository>,
        researcher: Arc<CompanyResearcher>,
    ) -> Self {
        Self { repo, researcher, stale_threshold_days: 7 }
    }

    /// Look up or create a company stub, then enrich it if stale.
    ///
    /// Called by the discovery loop after ingesting jobs for a company.
    /// Returns immediately if the company was enriched within `stale_threshold_days`.
    pub async fn ensure_enriched(
        &self,
        name: &str,
        website_url: Option<&str>,
        job_descriptions: &[&str],
    ) -> CompanyResult<CompanyRecord> {
        let name_normalized = CompanyNormalizer::normalize(name);

        let existing = self.repo.get_by_name(&name_normalized).await?;
        let record = match existing {
            Some(r) if !r.is_stale => {
                info!(company = %name, "Company data is fresh, skipping enrichment");
                return Ok(r);
            }
            Some(r) => r,
            None => {
                // Create a stub record with minimal data.
                let stub = CompanyRecord {
                    id: uuid::Uuid::new_v4(),
                    name: name.to_string(),
                    name_normalized: name_normalized.clone(),
                    website_url: website_url.map(str::to_string),
                    greenhouse_token: None,
                    lever_id: None,
                    description: None,
                    mission_statement: None,
                    core_values: vec![],
                    tech_stack: vec![],
                    product_areas: vec![],
                    culture_signals: vec![],
                    employee_count_range: None,
                    funding_stage: None,
                    founded_year: None,
                    hq_location: None,
                    glassdoor_rating: None,
                    glassdoor_pros: vec![],
                    glassdoor_cons: vec![],
                    recent_news: vec![],
                    recent_layoffs: false,
                    enriched_at: None,
                    enrichment_sources: vec![],
                    is_stale: true,
                };
                self.repo.upsert(&stub).await?
            }
        };

        // Run enrichment.
        let enriched = self.researcher.enrich(&record, job_descriptions).await
            .unwrap_or_else(|e| {
                warn!(company = %name, error = %e, "Enrichment failed, keeping existing data");
                record.clone()
            });

        self.repo.upsert(&enriched).await
    }

    /// Explicitly trigger a fresh enrichment — called by TUI `r` keybind.
    pub async fn enrich_now(
        &self,
        company_id: uuid::Uuid,
        job_descriptions: &[&str],
    ) -> CompanyResult<CompanyRecord> {
        let record = self.repo.get_by_id(company_id).await?
            .ok_or_else(|| CompanyError::NotFound {
                name_normalized: company_id.to_string(),
            })?;

        let enriched = self.researcher.enrich(&record, job_descriptions).await?;
        self.repo.upsert(&enriched).await
    }

    /// Refresh all stale company records — called by the daily ralph scheduler.
    ///
    /// Returns a `RefreshReport` summarizing what was refreshed, what failed.
    pub async fn refresh_stale_companies(
        &self,
        job_descriptions_by_company: &std::collections::HashMap<String, Vec<String>>,
    ) -> CompanyResult<RefreshReport> {
        let stale = self.repo
            .list_stale(Duration::days(self.stale_threshold_days as i64))
            .await?;

        let total = stale.len();
        let mut refreshed = 0usize;
        let mut failed = 0usize;

        for record in stale {
            let empty: Vec<String> = vec![];
            let descs = job_descriptions_by_company
                .get(&record.name_normalized)
                .unwrap_or(&empty);
            let desc_refs: Vec<&str> = descs.iter().map(|s| s.as_str()).collect();

            match self.researcher.enrich(&record, &desc_refs).await {
                Ok(enriched) => {
                    if let Err(e) = self.repo.upsert(&enriched).await {
                        warn!(company = %record.name, error = %e, "Failed to save enrichment");
                        failed += 1;
                    } else {
                        refreshed += 1;
                    }
                }
                Err(e) => {
                    warn!(company = %record.name, error = %e, "Enrichment failed");
                    failed += 1;
                }
            }
        }

        Ok(RefreshReport { total, refreshed, failed })
    }
}

#[derive(Debug)]
pub struct RefreshReport {
    pub total: usize,
    pub refreshed: usize,
    pub failed: usize,
}
```

**Verification**: Integration test — insert three companies with `enriched_at` set to 8
days ago. Call `refresh_stale_companies`. Assert `RefreshReport.refreshed == 3` with a
`wiremock` HTTP server returning a mock About page and a mock LLM response.

---

### Phase 4 — TUI: `CompanyView` Panel

**Goal**: A TUI panel that renders a `CompanyRecord` and supports manual refresh.

#### Step 4.1 — `CompanyView` widget

File: `lazyjob-tui/src/views/company_view.rs`

```rust
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget, Wrap},
};
use crate::state::AppState;

/// Renders a full-screen company detail panel.
pub struct CompanyView<'a> {
    pub record: &'a lazyjob_core::companies::models::CompanyRecord,
    pub is_loading: bool,
}

impl<'a> Widget for CompanyView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Split vertically: header (3 lines) | body (rest) | footer (1 line)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        // Header: company name + staleness badge
        let stale_badge = if self.record.is_stale {
            Span::styled(" [STALE] ", Style::default().fg(Color::Yellow))
        } else {
            let age = self.record.enriched_at
                .map(|t| format!(" [updated {}] ", format_age(t)))
                .unwrap_or_else(|| " [never enriched] ".to_string());
            Span::styled(age, Style::default().fg(Color::Green))
        };

        let loading_badge = if self.is_loading {
            Span::styled(" [refreshing…] ", Style::default().fg(Color::Cyan))
        } else {
            Span::raw("")
        };

        let header = Paragraph::new(Line::from(vec![
            Span::styled(
                &self.record.name,
                Style::default().add_modifier(Modifier::BOLD),
            ),
            stale_badge,
            loading_badge,
        ]))
        .block(Block::default().borders(Borders::ALL).title("Company"));
        header.render(chunks[0], buf);

        // Body: split into 2 columns
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(chunks[1]);

        // Left column: description, mission, culture signals
        let mut left_lines = vec![];
        if let Some(desc) = &self.record.description {
            left_lines.push(Line::from(Span::styled("Description", Style::default().add_modifier(Modifier::BOLD))));
            left_lines.push(Line::from(desc.as_str()));
            left_lines.push(Line::raw(""));
        }
        if let Some(mission) = &self.record.mission_statement {
            left_lines.push(Line::from(Span::styled("Mission", Style::default().add_modifier(Modifier::BOLD))));
            left_lines.push(Line::from(mission.as_str()));
            left_lines.push(Line::raw(""));
        }
        if !self.record.culture_signals.is_empty() {
            left_lines.push(Line::from(Span::styled("Culture", Style::default().add_modifier(Modifier::BOLD))));
            for s in &self.record.culture_signals {
                left_lines.push(Line::from(format!("  • {}", s)));
            }
        }
        let left = Paragraph::new(left_lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::RIGHT));
        left.render(body_chunks[0], buf);

        // Right column: tech stack, employee range, founded year
        let mut right_lines = vec![];
        if !self.record.tech_stack.is_empty() {
            right_lines.push(Line::from(Span::styled("Tech Stack", Style::default().add_modifier(Modifier::BOLD))));
            for t in &self.record.tech_stack {
                right_lines.push(Line::from(format!("  {}", t)));
            }
            right_lines.push(Line::raw(""));
        }
        if let Some(range) = &self.record.employee_count_range {
            right_lines.push(Line::from(format!(
                "Employees: {}",
                format_employee_range(range)
            )));
        }
        if let Some(year) = self.record.founded_year {
            right_lines.push(Line::from(format!("Founded: {}", year)));
        }
        if let Some(loc) = &self.record.hq_location {
            right_lines.push(Line::from(format!("HQ: {}", loc)));
        }
        let right = Paragraph::new(right_lines)
            .wrap(Wrap { trim: true });
        right.render(body_chunks[1], buf);

        // Footer: keybind hints
        let footer = Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": refresh  "),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": close"),
        ]));
        footer.render(chunks[2], buf);
    }
}

fn format_age(t: chrono::DateTime<chrono::Utc>) -> String {
    let days = (chrono::Utc::now() - t).num_days();
    if days == 0 { "today".to_string() }
    else if days == 1 { "1 day ago".to_string() }
    else { format!("{} days ago", days) }
}

fn format_employee_range(r: &lazyjob_core::companies::models::EmployeeCountRange) -> &'static str {
    use lazyjob_core::companies::models::EmployeeCountRange::*;
    match r {
        Tiny       => "1–10",
        Small      => "11–50",
        Medium     => "51–200",
        Large      => "201–1000",
        Enterprise => "1001+",
        Unknown    => "unknown",
    }
}
```

#### Step 4.2 — TUI event handler for `r` key

In `lazyjob-tui/src/app.rs`, dispatch the refresh action when the user presses `r` in
the company view:

```rust
// In the keybinding handler for CompanyView context:
KeyCode::Char('r') => {
    if let Some(company_id) = self.state.selected_company_id {
        self.state.company_loading = true;
        let service = self.company_service.clone();
        let descs = self.state.job_descriptions_for_company(company_id);
        tokio::spawn(async move {
            let result = service.enrich_now(company_id, &descs).await;
            // Send result back via mpsc channel to the event loop.
            let _ = refresh_tx.send(CompanyRefreshResult { company_id, result });
        });
    }
}
```

**Verification**: Launch the TUI against a test SQLite database. Navigate to a company
with `is_stale = true`. Press `r`. Confirm the loading badge appears and disappears when
enrichment completes. Confirm `enriched_at` is updated in SQLite.

---

### Phase 5 — Ralph Loop Integration

**Goal**: Wire the company researcher into the discovery ralph loop and the daily stale
refresh scheduled loop.

#### Step 5.1 — Trigger enrichment after discovery ingestion

In `lazyjob-ralph/src/loops/discovery.rs`, after `JobIngestionService::ingest()` returns:

```rust
// After ingesting jobs for a company batch:
let company_name = &config.name;
let website_url = config.website_url.as_deref();
let descriptions: Vec<&str> = ingested_jobs.iter()
    .map(|j| j.description.as_str())
    .collect();

// Spawn enrichment as a background task — does not block discovery of the next company.
let service = self.company_service.clone();
let name = company_name.clone();
let url = website_url.map(str::to_string);
tokio::spawn(async move {
    if let Err(e) = service.ensure_enriched(&name, url.as_deref(), &descriptions).await {
        tracing::warn!(company = %name, error = %e, "Company enrichment failed");
    }
});
```

#### Step 5.2 — Daily stale refresh ralph loop

Add `LoopType::CompanyRefresh` to the ralph orchestrator. Triggered daily at 03:00 local
time via the `cron` crate scheduler:

```rust
// lazyjob-ralph/src/loops/company_refresh.rs

pub async fn run_company_refresh_loop(
    company_service: Arc<CompanyService>,
    job_repo: Arc<dyn JobRepository>,
) -> anyhow::Result<()> {
    // Build job description map: company_normalized → Vec<String>
    let all_jobs = job_repo.list_active().await?;
    let mut descs_by_company: HashMap<String, Vec<String>> = HashMap::new();
    for job in all_jobs {
        descs_by_company
            .entry(CompanyNormalizer::normalize(&job.company_name))
            .or_default()
            .push(job.description.clone());
    }

    let report = company_service
        .refresh_stale_companies(&descs_by_company)
        .await?;

    tracing::info!(
        total   = report.total,
        refreshed = report.refreshed,
        failed  = report.failed,
        "Company refresh loop complete"
    );
    Ok(())
}
```

**Verification**: Call `run_company_refresh_loop` in an integration test with a seeded
SQLite DB containing 3 stale companies. Assert `report.refreshed == 3`.

---

### Phase 6 — Phase 2 Sources: News RSS, Glassdoor (Clipboard), Crunchbase

**Goal**: Add `EnrichmentSource::NewsRss`, `Glassdoor`, and `Crunchbase` sources behind
feature flags so they do not affect MVP stability.

#### Step 6.1 — `NewsAggregator` (RSS)

File: `lazyjob-core/src/companies/news.rs`

Uses the `rss` crate to fetch Google News RSS feeds (no API key required).

```rust
use rss::Channel;

pub struct NewsAggregator {
    http: reqwest::Client,
}

impl NewsAggregator {
    /// Fetch recent news for a company name from Google News RSS.
    /// Returns up to `limit` items sorted by published_at descending.
    pub async fn fetch_news(
        &self,
        company_name: &str,
        limit: usize,
    ) -> CompanyResult<Vec<NewsItem>> {
        // Google News RSS: no API key, returns structured XML.
        let query = urlencoding::encode(company_name);
        let url = format!(
            "https://news.google.com/rss/search?q={}&hl=en-US&gl=US&ceid=US:en",
            query
        );

        let bytes = self.http
            .get(&url)
            .send()
            .await
            .map_err(|e| CompanyError::HttpFetch { url: url.clone(), source: e })?
            .bytes()
            .await
            .map_err(|e| CompanyError::HttpFetch { url, source: e })?;

        let channel = Channel::read_from(&bytes[..])
            .map_err(|e| CompanyError::LlmExtraction(anyhow::anyhow!(e)))?;

        let items: Vec<NewsItem> = channel
            .items()
            .iter()
            .take(limit)
            .filter_map(|item| {
                let title = item.title()?.to_string();
                let url = item.link()?.to_string();
                let snippet = item.description()
                    .map(|d| {
                        // Strip HTML from snippets.
                        ammonia::Builder::empty()
                            .tags(Default::default())
                            .clean(d)
                            .to_string()
                    })
                    .unwrap_or_default();
                let published_at = item.pub_date()
                    .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                let source_name = item.source()
                    .map(|s| s.title().unwrap_or("Unknown").to_string())
                    .unwrap_or_else(|| "Google News".to_string());

                Some(NewsItem { title, url, published_at, snippet, source_name })
            })
            .collect();

        Ok(items)
    }
}
```

#### Step 6.2 — Glassdoor via clipboard paste

Per the spec recommendation, Glassdoor is exposed as a user-triggered clipboard-paste
flow rather than automated scraping (ToS compliance):

```rust
// lazyjob-core/src/companies/glassdoor_clipboard.rs

/// Parse a Glassdoor company page pasted as plain text by the user.
/// The TUI prompts: "Paste the Glassdoor page text, then press Ctrl+D".
pub fn parse_glassdoor_paste(text: &str) -> GlassdoorSummary {
    // Heuristic: look for patterns like "4.2 ★" (rating) in the text.
    static RATING_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(\d\.\d)\s*(?:★|stars?|out of 5)").unwrap()
    });

    let rating = RATING_RE.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f32>().ok());

    GlassdoorSummary { rating, pros: vec![], cons: vec![] }
}

#[derive(Debug)]
pub struct GlassdoorSummary {
    pub rating: Option<f32>,
    pub pros: Vec<String>,
    pub cons: Vec<String>,
}
```

**Verification**: Unit test `parse_glassdoor_paste` with sample text containing
"4.1 ★ out of 5". Assert `rating == Some(4.1)`.

---

## Key Crate APIs

| Crate | API | Usage |
|---|---|---|
| `reqwest::Client` | `client.get(url).header(k, v).timeout(d).send().await` | Fetching About/Careers pages |
| `ammonia::Builder::empty()` | `.tags(HashSet::new()).clean(html).to_string()` | HTML → text for LLM |
| `regex::Regex` | `PATTERN.find_iter(text)` | Tech stack extraction |
| `once_cell::sync::Lazy` | `static X: Lazy<Regex> = Lazy::new(...)` | Compile-once patterns |
| `strsim::jaro_winkler` | `jaro_winkler(&a, &b) >= 0.92` | Fuzzy company name match |
| `sqlx::query!` | `query!("INSERT INTO companies ... ON CONFLICT ... DO UPDATE")` | Upsert with merge |
| `chrono::Utc::now()` | `Utc::now() - t > Duration::days(7)` | Staleness computation |
| `rss::Channel` | `Channel::read_from(&bytes[..])` | Google News RSS parsing |
| `serde_json::from_str` | `from_str::<Vec<String>>(&json_col)` | JSON TEXT column decode |
| `tokio::join!` | `tokio::join!(fetch_page_text(...), fetch_page_text(...))` | Parallel page fetch |

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum CompanyError {
    #[error("Company not found: {name_normalized}")]
    NotFound { name_normalized: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("HTTP fetch failed for {url}: {source}")]
    HttpFetch {
        url: String,
        #[source] source: reqwest::Error,
    },

    #[error("LLM extraction failed: {0}")]
    LlmExtraction(#[from] anyhow::Error),

    #[error("JSON deserialization failed: {0}")]
    JsonDeser(#[from] serde_json::Error),

    #[error("RSS parse error: {0}")]
    RssParse(String),
}
```

**Failure policy**: `CompanyService::ensure_enriched()` degrades gracefully — if HTTP
fetch fails, the company stub is still written to SQLite with no enriched data. Callers
(discovery loop, TUI) should not treat enrichment failure as fatal. Only `enrich_now()`
(user-triggered) propagates errors to the TUI for display.

## Testing Strategy

### Unit Tests

**`CompanyNormalizer`**:
- `normalize("Stripe, Inc.")` → `"stripe"`
- `normalize("OpenAI B.V.")` → `"openai"`
- `normalize("Meta Platforms LLC")` → `"meta platforms"`
- `is_same_company("Stripe Inc", "Stripe")` → `true`
- `is_same_company("Stripe", "Google")` → `false`

**`TechStackLexicon`**:
- `extract(&["We use Rust and PostgreSQL on Kubernetes"])` → `["Kubernetes", "PostgreSQL", "Rust"]`
- Empty input → `[]`
- Multiple descriptions → deduplicates correctly

**`CompanyExtractionResult` deserialization**:
- Valid LLM JSON response deserializes without panic.
- Fields missing from JSON default to `None` / empty vec.
- `employee_count_range: null` → `None`.

### Integration Tests (via `#[sqlx::test]`)

```rust
#[sqlx::test(migrations = "migrations")]
async fn upsert_merges_greenhouse_token(pool: SqlitePool) {
    let repo = SqliteCompanyRepository::new(pool);

    // First upsert: sets greenhouse_token
    let record1 = CompanyRecord { greenhouse_token: Some("gh-stripe".to_string()), ..stub("Stripe") };
    repo.upsert(&record1).await.unwrap();

    // Second upsert: sets lever_id, should not clear greenhouse_token
    let record2 = CompanyRecord { lever_id: Some("stripe".to_string()), ..stub("Stripe") };
    let result = repo.upsert(&record2).await.unwrap();

    assert_eq!(result.greenhouse_token, Some("gh-stripe".to_string()));
    assert_eq!(result.lever_id, Some("stripe".to_string()));
}

#[sqlx::test(migrations = "migrations")]
async fn list_stale_returns_unenriched(pool: SqlitePool) {
    let repo = SqliteCompanyRepository::new(pool);
    for name in ["Stripe", "Anthropic", "OpenAI"] {
        repo.upsert(&stub(name)).await.unwrap();  // enriched_at = None
    }
    let stale = repo.list_stale(Duration::days(7)).await.unwrap();
    assert_eq!(stale.len(), 3);
}
```

**`CompanyResearcher` integration tests** (with `wiremock`):

```rust
#[tokio::test]
async fn extracts_tech_stack_from_page() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .set_body_string("<html><body>We build with Rust, PostgreSQL, and Kubernetes.</body></html>"))
        .mount(&mock_server)
        .await;

    let mock_llm = Arc::new(MockLlmProvider::returning(r#"{
        "description": "A fintech company.",
        "mission_statement": null,
        "core_values": [],
        "tech_stack": ["Rust", "PostgreSQL", "Kubernetes"],
        "product_areas": ["payments"],
        "culture_signals": [],
        "employee_count_range": "large",
        "founded_year": null,
        "hq_location": null
    }"#));

    let http = reqwest::Client::new();
    let researcher = CompanyResearcher::with_http(mock_llm, http);
    let mut record = stub("Stripe");
    record.website_url = Some(mock_server.uri());

    let enriched = researcher.enrich(&record, &[]).await.unwrap();
    assert!(enriched.tech_stack.contains(&"Rust".to_string()));
    assert_eq!(enriched.enrichment_sources, vec![
        EnrichmentSource::CompanyWebsite,
        EnrichmentSource::JobDescriptionInference,
    ]);
    assert!(enriched.enriched_at.is_some());
}
```

### TUI Tests

The `CompanyView` widget can be exercised with `ratatui::backend::TestBackend`:

```rust
#[test]
fn renders_stale_badge_when_stale() {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let record = CompanyRecord { is_stale: true, name: "Stripe".to_string(), ..Default::default() };
    terminal.draw(|f| f.render_widget(CompanyView { record: &record, is_loading: false }, f.size())).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Check that "[STALE]" appears somewhere in the buffer.
    let content: String = buffer.content().iter().map(|c| c.symbol().to_string()).collect();
    assert!(content.contains("STALE"));
}
```

## Open Questions

1. **Per-field staleness vs. single `is_stale`**: The spec's Open Questions section flags
   that a single staleness flag is too coarse — ghost detection needs monthly refresh, tech
   stack needs weekly, news needs daily. Phase 1 uses a single 7-day TTL. Phase 3 should
   add per-source `company_enrichment_log` TTL queries so each source has its own refresh
   cadence. The `company_enrichment_log` table is already included in the schema for this.

2. **About page discovery heuristics**: `derive_about_url()` currently just appends `/about`.
   Many companies use `/about-us`, `/company`, `/who-we-are`, or have no dedicated About
   page. A Phase 2 improvement: fetch the homepage, parse `<a href>` links for canonical
   About/Careers page URLs using the `scraper` crate before fetching.

3. **Glassdoor ToS**: The plan implements clipboard-paste for Phase 2 per the spec
   recommendation. Before implementing any automated Glassdoor fetching (even read-only),
   consult legal. The clipboard approach requires explicit user action and is defensible.

4. **LLM response validation**: The current plan calls `serde_json::from_str()` and
   propagates `JsonDeser` errors. A more robust approach is to use a lenient deserializer
   that accepts partial JSON (e.g., `serde_json::Value` then manually extract fields) so
   a single missing field doesn't abort the entire enrichment. Implement in Phase 3.

5. **Crunchbase access**: Crunchbase's free tier requires an API key and has strict rate
   limits. Phase 2 should register for a free Crunchbase Basic API key and store it in the
   OS keychain via the `keyring` crate. The API endpoint is
   `GET /api/v4/entities/organizations/{permalink}?card_ids=fields`.

6. **`CompanyRecord` as the canonical entity — enforcement**: This must be documented in
   `CLAUDE.md` as a hard rule before implementation begins: ghost detection, cover letter
   generation, and interview prep must **not** define their own company structs. They must
   import `CompanyRecord` from `lazyjob-core::companies`.

## Related Specs

- `specs/job-search-discovery-engine.md` — triggers enrichment after job ingestion
- `specs/job-search-ghost-job-detection.md` — consumes `employee_count_range` and `recent_layoffs`
- `specs/08-cover-letter-generation.md` — consumes `mission_statement`, `culture_signals`, `tech_stack`
- `specs/agentic-ralph-orchestration.md` — schedules `LoopType::CompanyRefresh` daily
- `specs/17-ralph-prompt-templates.md` — provides the `company_research` template
- `specs/04-sqlite-persistence.md` — migration runner, `SqlitePool` setup
