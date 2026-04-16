# Research: Task 24 — discovery-service

## Existing Infrastructure

### JobSource Trait
```rust
#[async_trait]
pub trait JobSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>>;
}
```
Both `GreenhouseClient` and `LeverClient` implement this trait. `fetch_jobs` takes a board token / company slug.

### upsert_discovered
- `JobRepository::upsert_discovered(&self, job: &Job) -> Result<()>`
- Uses `INSERT ... ON CONFLICT (source, source_id) WHERE source IS NOT NULL AND source_id IS NOT NULL DO UPDATE SET ...`
- Currently returns `()` — cannot distinguish new vs updated rows
- Fix: change return type to `Result<bool>` using PostgreSQL `RETURNING (xmax = 0) AS is_new`

### Discovery Mod Exports
Currently exports: `GreenhouseClient`, `JobSource`, `LeverClient`, `RateLimiter`

### Existing deps
- `futures` crate already in workspace (added in task 10 for TUI)
- `reqwest` in lazyjob-core (added in task 23)
- `async-trait` in workspace (added in task 14)

### No SourceConfig type yet
The task references `Vec<SourceConfig>` but no such type exists. Must define it.

## Key Design Decisions

### SourceConfig
```rust
pub struct SourceConfig {
    pub source: String,     // "greenhouse" or "lever"
    pub company_id: String, // board token or company slug
}
```
Simple string-based, easily extensible for new sources.

### DiscoveryStats
```rust
pub struct DiscoveryStats {
    pub jobs_found: usize,
    pub jobs_new: usize,
    pub jobs_updated: usize,
    pub errors: usize,
}
```
Aggregated across all sources.

### DiscoveryProgress channel
Task says "emit progress events via mpsc channel". Progress message sent per source after fetching.
```rust
pub struct DiscoveryProgress {
    pub source: String,
    pub company_id: String,
    pub message: String,
}
```
`run_discovery` takes `Option<mpsc::Sender<DiscoveryProgress>>` — callers that don't need progress can pass None.

### Parallelism
Use `futures::future::join_all` to fan out all (source, company_id) pairs simultaneously.
Each future: create client → fetch_jobs → upsert each job → accumulate stats.

### xmax trick for new vs updated
```sql
INSERT INTO jobs (...) VALUES (...)
ON CONFLICT (...) WHERE ... DO UPDATE SET ...
RETURNING (xmax = 0) AS is_new
```
`xmax = 0` on the returned row means no prior version existed = newly inserted.

### CLI subcommand
Add `ralph job-discovery` under a new `Ralph` top-level subcommand:
```
lazyjob ralph job-discovery --source greenhouse --company-id stripe
```
Prints progress and final stats to stdout.
