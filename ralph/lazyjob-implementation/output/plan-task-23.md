# Plan: Task 23 — job-sources

## Files to Create

| File | Purpose |
|---|---|
| `lazyjob-core/migrations/002_unique_job_source.sql` | Partial unique index for ON CONFLICT upsert |
| `lazyjob-core/src/discovery/mod.rs` | Module re-exports |
| `lazyjob-core/src/discovery/sources/mod.rs` | `JobSource` trait + `RateLimiter` |
| `lazyjob-core/src/discovery/sources/greenhouse.rs` | `GreenhouseClient` |
| `lazyjob-core/src/discovery/sources/lever.rs` | `LeverClient` |

## Files to Modify

| File | Change |
|---|---|
| `Cargo.toml` | Add `ammonia = "4"`, `wiremock = "0.6"` to workspace deps |
| `lazyjob-core/Cargo.toml` | Add `ammonia` dep; `async-trait` dep; `wiremock` dev-dep |
| `lazyjob-core/src/error.rs` | Add `CoreError::Http(String)` variant |
| `lazyjob-core/src/lib.rs` | Add `pub mod discovery` |
| `lazyjob-core/src/repositories/job.rs` | Add `upsert_discovered(&Job) -> Result<()>` |

## Types to Define

### `discovery/sources/mod.rs`
```rust
pub trait JobSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>>;
}

pub struct RateLimiter {
    interval: Duration,
    last_call: std::sync::Mutex<Option<std::time::Instant>>,
}
impl RateLimiter {
    pub fn new(requests_per_second: u32) -> Self;
    pub async fn wait(&self);  // interior mutability, safe to call on &self
}
```

### `discovery/sources/greenhouse.rs`
```rust
pub struct GreenhouseClient {
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    base_url: String,
}
impl GreenhouseClient {
    pub fn new() -> Self;
    pub fn with_base_url(self, url: impl Into<String>) -> Self;
    pub async fn fetch_jobs(&self, board_token: &str) -> Result<Vec<Job>>;
}

// Private serde types
struct GreenhouseResponse { jobs: Vec<GreenhouseJob> }
struct GreenhouseJob { id: i64, title: String, content: Option<String>, location: Option<GreenhouseLocation>, departments: Option<Vec<GreenhouseDepartment>>, updated_at: Option<String>, absolute_url: Option<String> }
struct GreenhouseLocation { name: Option<String> }
struct GreenhouseDepartment { name: String }
```

### `discovery/sources/lever.rs`
```rust
pub struct LeverClient {
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    base_url: String,
}
impl LeverClient {
    pub fn new() -> Self;
    pub fn with_base_url(self, url: impl Into<String>) -> Self;
    pub async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>>;
}

// Private serde types
struct LeverPosting { id: String, text: String, description: Option<String>, categories: Option<LeverCategories>, created_at: Option<i64>, hosted_url: Option<String> }
struct LeverCategories { location: Option<String>, team: Option<String>, commitment: Option<String> }
```

## Tests to Write

### Learning Tests (proves library behavior before using it)
1. `ammonia_strips_html_tags` — `ammonia::Builder::new().tags(HashSet::new()).clean("<p>Hello <b>World</b></p>").to_string()` == "Hello World"
2. `wiremock_responds_with_json` — start MockServer, mount Mock, make reqwest GET, verify body parsed correctly

### Unit Tests
3. `strip_html_empty_string` — `strip_html("")` == `""`
4. `strip_html_plain_text` — `strip_html("hello world")` == `"hello world"`
5. `strip_html_nested_tags` — `strip_html("<p>Test <b>bold</b></p>")` == `"Test bold"`
6. `strip_html_entities` — HTML entities are decoded (ammonia handles this)
7. `greenhouse_parses_response` — wiremock returns fixture, client returns Vec<Job> with correct fields
8. `greenhouse_returns_error_on_bad_status` — wiremock returns 404, client returns Err
9. `lever_parses_response` — wiremock returns fixture, client returns Vec<Job> with correct fields  
10. `lever_returns_error_on_bad_status` — wiremock returns 500, client returns Err
11. `rate_limiter_allows_first_call_immediately` — first wait() returns quickly
12. `upsert_discovered_inserts_new_job` — integration test (skips without DB)
13. `upsert_discovered_updates_existing_job` — integration test (skips without DB)

## Key Decisions

1. **`RateLimiter::wait(&self)` not `&mut self`** — uses `Mutex<Option<Instant>>` for interior mutability so clients can implement `JobSource` trait with `&self` receivers
2. **`base_url` field on clients** — allows tests to point clients at `MockServer::uri()` without feature flags
3. **`CoreError::Http(String)`** — distinct from `CoreError::Parse` and `CoreError::Io` for HTTP transport errors
4. **Partial unique index in migration 002** — `WHERE source IS NOT NULL AND source_id IS NOT NULL` lets manually-entered jobs (with NULL source) bypass the constraint
5. **`upsert_discovered` in `JobRepository`** — keeps DB logic in repository layer, clients call it after fetch
6. **HTML stripping** — `ammonia::Builder` with empty tag set: strips tags, keeps text nodes, decodes HTML entities
