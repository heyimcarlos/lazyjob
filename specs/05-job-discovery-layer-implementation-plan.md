# Job Discovery Layer — Implementation Plan

## Spec Reference
- **Spec file**: `specs/05-job-discovery-layer.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
The Job Discovery Layer aggregates job listings from legitimate APIs (Greenhouse, Lever) and matches them to the user's life sheet profile using semantic embeddings. It stores discovered jobs in SQLite and surfaces the most relevant opportunities to the user through the TUI.

## Problem Statement
LazyJob needs to discover job opportunities from multiple sources, aggregate and deduplicate them, enrich with company data, match to user profiles, and track in the database—without relying on fragile scraping.

## Implementation Phases

### Phase 1: Foundation
1. Define `JobSource` trait and `JobSourceRegistry` in `lazyjob-core/src/discovery/sources/`
2. Implement `GreenhouseSource` and `LeverSource` API clients
3. Create `DiscoveredJob`, `RemoteType`, and `DiscoveredJobEnrichment` types
4. Set up configuration loading for company registry in `config.yaml`
5. Add `jobs` table to SQLite schema with all discovery fields

### Phase 2: Core Implementation
1. Build `EnrichmentPipeline` with HTML stripping, salary extraction, remote classification
2. Implement `CompanyRegistry` with config loading from `config.yaml`
3. Build `DiscoveryService` with `refresh_all_companies()` and `discover_company_jobs()`
4. Add `JobRepository` CRUD operations for discovered jobs with deduplication
5. Implement `JobMatcher` with cosine similarity and `find_matching_jobs()`
6. Add SQLite FTS5 for keyword search fallback

### Phase 3: Integration & Polish
1. Wire discovery into TUI dashboard and job list view
2. Add background polling with configurable interval
3. Implement error handling with exponential backoff for rate limiting
4. Add integration tests for API clients with mock servers
5. Unit tests for enrichment pipeline, similarity scoring, deduplication

## Data Model

### New Schema: `jobs` table
```sql
CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,           -- 'greenhouse', 'lever', 'manual'
    source_id TEXT NOT NULL,        -- ID from the source platform
    title TEXT NOT NULL,
    company_name TEXT NOT NULL,
    company_id TEXT,                -- FK to companies table (optional)
    location TEXT,
    remote INTEGER,                 -- 0=unknown, 1=no, 2=hybrid, 3=yes
    url TEXT NOT NULL,
    description TEXT NOT NULL,
    salary_min INTEGER,
    salary_max INTEGER,
    salary_currency TEXT,
    department TEXT,
    employment_type TEXT,
    posted_at TEXT,
    discovered_at TEXT NOT NULL,
    embedding BLOB,                  -- Serialized f32 vector
    description_hash TEXT,           -- For deduplication
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_jobs_source ON jobs(source, source_id);
CREATE INDEX idx_jobs_company ON jobs(company_name);
CREATE INDEX idx_jobs_discovered ON jobs(discovered_at DESC);
```

### New Types
```rust
// lazyjob-core/src/discovery/models.rs
pub struct DiscoveredJob { /* as defined in spec */ }
pub enum RemoteType { Yes, No, Hybrid, Unknown }

// lazyjob-core/src/discovery/sources/mod.rs
pub trait JobSource: Send + Sync { ... }
pub struct GreenhouseSource { ... }
pub struct LeverSource { ... }
pub struct JobSourceRegistry { ... }

// lazyjob-core/src/discovery/companies.rs
pub struct CompanyRegistry { ... }
pub struct CompanyConfig { ... }

// lazyjob-core/src/discovery/matching.rs
pub struct JobMatcher { ... }
pub struct MatchingResult { job: Job, score: f32 }

// lazyjob-core/src/discovery/service.rs
pub struct DiscoveryService { ... }
pub struct DiscoveryReport { new_jobs, updated, duplicates }
```

## API Surface

### lazyjob-core
```rust
// discovery/sources/mod.rs
pub trait JobSource { fn name(), fn fetch_jobs(), fn normalize_job() }
pub struct JobSourceRegistry { pub fn new(), pub fn register(), pub async fn fetch_from_all(), pub async fn fetch_from() }
pub struct GreenhouseSource { pub fn new(), pub async fn fetch_jobs() }
pub struct LeverSource { pub fn new(), pub async fn fetch_jobs() }

// discovery/models.rs
pub struct DiscoveredJob { /* fields */ }
pub enum RemoteType { Yes, No, Hybrid, Unknown }

// discovery/companies.rs
pub struct CompanyRegistry { pub fn from_config(), pub async fn discover_company_jobs(), pub async fn discover_all() }
pub struct CompanyConfig { /* fields */ }

// discovery/enrichment.rs
pub struct EnrichmentPipeline { pub fn new(), pub fn enrich() }
impl EnrichmentPipeline {
    fn strip_html(), fn extract_salary(), fn classify_remote(), fn normalize_location()
}

// discovery/matching.rs
pub struct JobMatcher { pub fn new(), pub async fn embed_job(), pub fn embed_life_sheet(), pub fn similarity(), pub async fn find_matching_jobs() }

// discovery/service.rs
pub struct DiscoveryService { pub async fn refresh_all_companies(), pub async fn search_by_text(), pub async fn find_similar_jobs() }
pub struct DiscoveryReport { pub new_jobs, pub updated, pub duplicates }
```

### lazyjob-cli
```yaml
# config.yaml integration
discovery:
  companies:
    - name: "Stripe"
      greenhouse_board_token: "stripe"
    - name: "Notion"
      lever_company_id: "notion"
  polling:
    enabled: true
    interval_minutes: 60
  matching:
    top_k: 20
```

## Key Technical Decisions

1. **API-only MVP (no scraping)**: Greenhouse + Lever public APIs provide reliable, ToS-compliant job data. Scraping deferred to Phase 2.
2. **In-memory embeddings for matching**: LazyJob's scale (~100s-1000s jobs) doesn't require dedicated vector DB. Embeddings stored as JSON/BLOB in SQLite.
3. **Cosine similarity for matching**: Standard approach, simple to implement, good enough for profile-to-job matching.
4. **Config-driven company registry**: Users configure target companies in `config.yaml` rather than searching all jobs globally.
5. **Async/await throughout**: All I/O-bound operations (API calls, DB ops) are async using tokio.

## File Structure
```
lazyjob/
├── lazyjob-core/
│   ├── src/
│   │   ├── discovery/
│   │   │   ├── mod.rs              # Module re-exports
│   │   │   ├── sources/
│   │   │   │   ├── mod.rs          # JobSource trait, Registry
│   │   │   │   ├── greenhouse.rs   # GreenhouseSource
│   │   │   │   └── lever.rs        # LeverSource
│   │   │   ├── models.rs           # DiscoveredJob, RemoteType
│   │   │   ├── companies.rs        # CompanyRegistry, CompanyConfig
│   │   │   ├── enrichment.rs       # EnrichmentPipeline
│   │   │   ├── matching.rs         # JobMatcher, cosine_similarity
│   │   │   └── service.rs          # DiscoveryService
│   │   ├── models/
│   │   │   └── job.rs              # Job (db model), JobRepository
│   │   └── lib.rs
│   └── Cargo.toml
├── lazyjob-cli/
│   ├── src/
│   │   └── config.rs               # Config loading, discovery section
│   └── Cargo.toml
├── specs/
│   ├── 05-job-discovery-layer.md
│   └── 05-job-discovery-layer-implementation-plan.md
└── SPEC.md
```

## Dependencies

### lazyjob-core/Cargo.toml
```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
scraper = "0.20"                    # HTML parsing for descriptions
ammonia = "4"                       # Safe HTML sanitization
regex = "1"                         # Salary extraction from text
futures = "0.3"                     # futures::future::join_all
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"                    # Embedding serialization
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
wiremock = "1"                      # HTTP mocking for tests
```

### Dependencies on Other Specs
- **04-sqlite-persistence.md** — Must be implemented first for `JobRepository` base
- **03-life-sheet-data-model.md** — Must be implemented first for `LifeSheet` type used in matching
- **02-llm-provider-abstraction.md** — Must be implemented first for `LLMProvider::embed()` used in `JobMatcher`

## Testing Strategy

### Unit Tests
- `EnrichmentPipeline::strip_html()` — various HTML inputs
- `EnrichmentPipeline::extract_salary()` — salary patterns in job text
- `EnrichmentPipeline::classify_remote()` — remote/hybrid/onsite classification
- `cosine_similarity()` — known vectors, edge cases (zero vectors)
- `JobMatcher::find_matching_jobs()` — mock embeddings

### Integration Tests
- `GreenhouseSource::fetch_jobs()` — mock API response
- `LeverSource::fetch_jobs()` — mock API response
- `DiscoveryService::refresh_all_companies()` — full flow with mock registry
- `JobRepository` deduplication — same job inserted twice

### Test Data
```rust
// Mock Greenhouse response
static GREENHOUSE_RESPONSE: &str = r#"{
  "jobs": [{
    "id": 127817,
    "title": "Senior Software Engineer",
    "content": "<p>Build things with Rust</p>",
    "location": {"name": "San Francisco, CA"},
    "departments": [{"name": "Engineering"}]
  }]
}"#;
```

## Open Questions

1. **LinkedIn Integration**: Users may want LinkedIn job search. Consider browser extension approach (out of scope for MVP).
2. **Full-Text Search**: Should SQLite FTS5 be used for keyword search? Yes — add as part of Phase 2.
3. **Job Alert Frequency**: Make polling interval configurable per company via `config.yaml`.
4. **Matching Weights**: Currently equal weighting of skills/experience/education. Could add configurable weights later.
5. **Embedding Truncation**: Job descriptions can be long. Consider summarizing with LLM before embedding.
6. **Description Hash for Deduplication**: Use SHA256 of normalized (lowercased, whitespace-collapsed) description text.

## Effort Estimate
**Rough: 3-4 days**

- Phase 1 (Foundation): 1 day — trait, API clients, config loading
- Phase 2 (Core): 1.5 days — enrichment pipeline, matching, repository
- Phase 3 (Integration): 1-1.5 days — TUI wiring, background polling, tests

Dependencies (03-life-sheet-data-model, 02-llm-provider-abstraction, 04-sqlite-persistence) likely add 2-3 additional days if not already complete.
