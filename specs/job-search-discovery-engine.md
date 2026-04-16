# Spec: Job Discovery Engine

**JTBD**: Find relevant job opportunities without wasting time on ghost jobs or mismatched roles
**Topic**: Aggregate, enrich, and deduplicate job listings from multiple configured sources into a single local SQLite store
**Domain**: job-search

---

## What

The Job Discovery Engine is the data ingestion layer for LazyJob. It fetches raw job listings from configured company boards (Greenhouse, Lever, and optionally Adzuna), runs each listing through an enrichment pipeline (HTML sanitization, salary extraction, remote classification), deduplicates across sources, and persists results to SQLite. It is invoked by the ralph job-discovery loop and writes directly to the shared WAL-mode database.

## Why

Job seekers currently maintain accounts on 3-5 platforms and manually check each one. 75% of applications receive zero response, partly because candidates waste time on stale or ghost listings. LazyJob must aggregate and normalize listings so the user sees one curated feed rather than a pile of tabs. Without this layer, every downstream feature (semantic matching, ghost detection, application tracking) has no data to work on.

## How

### Data flow

```
Config (company list + tokens)
    → CompanyRegistry::discover_all()
        → GreenhouseSource / LeverSource / AdzunaSource (parallel)
            → EnrichmentPipeline::process(raw_job)
                → deduplication check (source + source_id)
                    → JobRepository::upsert()
```

### Architecture

`DiscoveryService` in `lazyjob-core/src/discovery/service.rs` owns the top-level orchestration. It depends on:
- `CompanyRegistry` — maps company names to their configured sources
- `JobSourceRegistry` — dispatches to concrete `JobSource` trait impls
- `EnrichmentPipeline` — stateless transform chain
- `JobRepository` — the sqlx-backed persistence layer

The service is invoked by `lazyjob-ralph` (the ralph subprocess) during a discovery loop run; it writes results to SQLite and exits. The TUI reads from SQLite via `JobRepository` — no direct coupling between TUI and discovery.

### Enrichment pipeline steps (ordered)

1. **HTML sanitization** — `ammonia` crate, allowlist of safe tags; produces plain text `description`
2. **Salary extraction** — regex scan of description for `$X[-–]$Y` / `€X` / pay-range patterns; populates `salary_min`, `salary_max`, `salary_currency`
3. **Remote classification** — keyword heuristic on title + description + location field; yields `RemoteType::{Yes|No|Hybrid|Unknown}`
4. **Location normalization** — collapse "Remote" / "Anywhere" / "San Francisco, CA" into normalized form
5. **Company linkage** — lookup `company_name` in local `companies` table; set `company_id` FK if found

Embedding generation is NOT done here — it is a separate async step triggered after ingestion (see `job-search-semantic-matching.md`).

### Deduplication strategy

Primary key: `(source, source_id)`. On upsert:
- If `title` or `description` hash changed → update the row and bump `updated_at`
- If identical → skip (count as `duplicates` in `DiscoveryReport`)
- Cross-source duplicates (same job posted on both Greenhouse and Adzuna) are identified post-ingestion by `(company_id, title_normalized, location_normalized)` fuzzy match with a confidence threshold; the lower-priority source record is soft-deleted.

### Multi-source fan-out

Each company in `config.toml` declares which sources it uses. Fan-out is `tokio::spawn`-per-source, joined with `futures::future::join_all`. A `RateLimiter` per source enforces per-source request cadence (default: 60 req/min for Greenhouse/Lever, 10 req/min for Adzuna free tier).

### Config format

```toml
# ~/.config/lazyjob/config.toml
[discovery]
polling_interval_minutes = 60

[[discovery.companies]]
name = "Stripe"
greenhouse_board_token = "stripe"

[[discovery.companies]]
name = "Notion"
lever_company_id = "notion"

[[discovery.companies]]
name = "Airbnb"
greenhouse_board_token = "airbnb"

[discovery.adzuna]
app_id = "abc123"
app_key = "xyz"
countries = ["us"]
```

## Interface

```rust
// lazyjob-core/src/discovery/source.rs

#[async_trait::async_trait]
pub trait JobSource: Send + Sync {
    fn name(&self) -> &str;
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>>;
}

// lazyjob-core/src/discovery/models.rs

pub struct RawJob {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub company_name: String,
    pub location: Option<String>,
    pub url: String,
    pub description_html: String,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
}

pub struct DiscoveredJob {
    pub id: Uuid,
    pub source: String,
    pub source_id: String,
    pub company_id: Option<Uuid>,
    pub title: String,
    pub company_name: String,
    pub location: Option<String>,
    pub remote: RemoteType,
    pub url: String,
    pub description: String,       // sanitized plain text
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    pub salary_currency: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub embedding: Option<Vec<f32>>, // populated by semantic-matching step
    pub match_score: Option<f32>,    // populated by semantic-matching step
    pub ghost_score: Option<f32>,    // populated by ghost-detection step
    pub status: JobStatus,
}

pub enum RemoteType { Yes, No, Hybrid, Unknown }

pub enum JobStatus {
    New,        // just discovered
    Reviewed,   // user has seen it
    Saved,      // user starred it
    Dismissed,  // user skipped it
    Applied,    // an Application record exists
}

// lazyjob-core/src/discovery/service.rs

pub struct DiscoveryService {
    registry: CompanyRegistry,
    repo: Arc<dyn JobRepository>,
    enricher: EnrichmentPipeline,
}

impl DiscoveryService {
    pub async fn run_discovery(&self) -> Result<DiscoveryReport>;
    pub async fn refresh_company(&self, company_name: &str) -> Result<DiscoveryReport>;
}

pub struct DiscoveryReport {
    pub new_jobs: usize,
    pub updated: usize,
    pub duplicates: usize,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}
```

## Open Questions

- **Adzuna in MVP?** The free tier is 250 requests/month — sufficient for one-per-day runs but not hourly polling. Should Adzuna be a Phase 2 source while Greenhouse/Lever cover MVP?
- **Cross-source deduplication threshold**: What similarity threshold on `(title_normalized, company_id)` is "same job"? 0.9 cosine? Exact title match? Need to test against real data.
- **Stale listing TTL**: After how many days without an update should a job be marked `Stale` and deprioritized in the feed? Proposal: 60 days for ghost detection; 30 days for general staleness badge.
- **Adzuna rate limit**: Free tier caps at 250/month. Does this scale for a user with 50+ companies? Need to evaluate Adzuna Paid tier pricing.

## Implementation Tasks

- [ ] Define `JobSource` trait and implement `GreenhouseSource` and `LeverSource` in `lazyjob-core/src/discovery/sources/` — refs: `05-job-discovery-layer.md`, `11-platform-api-integrations.md`
- [ ] Implement `EnrichmentPipeline` with HTML sanitization (ammonia), salary extraction (regex), and remote classification — refs: `05-job-discovery-layer.md`
- [ ] Add `jobs` table DDL and `JobRepository` trait with `upsert`, `find_by_source`, `list_by_status`, `search` methods to `lazyjob-core/src/db/` — refs: `04-sqlite-persistence.md`
- [ ] Implement `CompanyRegistry` that reads from `config.toml` and dispatches to registered sources — refs: `05-job-discovery-layer.md`
- [ ] Implement `DiscoveryService::run_discovery()` with parallel fan-out via `tokio::spawn` + `RateLimiter` per source — refs: `05-job-discovery-layer.md`
- [ ] Add cross-source deduplication pass after initial ingestion using title + company_id normalized match — refs: `agentic-job-matching.md`
- [ ] Implement `AdzunaSource` as optional Phase 2 source gated behind config flag — refs: `agentic-job-matching.md`
- [ ] Wire `DiscoveryService` into `lazyjob-ralph` subprocess so discovery results write directly to SQLite — refs: `06-ralph-loop-integration.md`
