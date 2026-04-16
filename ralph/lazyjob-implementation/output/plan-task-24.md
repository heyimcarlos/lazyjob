# Plan: Task 24 — discovery-service

## Files to Create/Modify

1. `crates/lazyjob-core/src/discovery/service.rs` (NEW)
   - `SourceConfig { source: String, company_id: String }`
   - `DiscoveryStats { jobs_found, jobs_new, jobs_updated, errors }` with `impl Add`
   - `DiscoveryProgress { source, company_id, message }`
   - `DiscoveryService::run_discovery(pool, sources, progress_tx) -> Result<DiscoveryStats>`

2. `crates/lazyjob-core/src/discovery/mod.rs` (MODIFY)
   - Add `pub mod service;`
   - Re-export `SourceConfig`, `DiscoveryStats`, `DiscoveryProgress`, `DiscoveryService`

3. `crates/lazyjob-core/src/repositories/job.rs` (MODIFY)
   - Change `upsert_discovered` to return `Result<bool>` (true=new)
   - Use `RETURNING (xmax = 0) AS is_new` via `fetch_one` + parse bool row

4. `crates/lazyjob-cli/src/main.rs` (MODIFY)
   - Add `Ralph(RalphArgs)` to `Commands` enum
   - `RalphArgs` with subcommand `JobDiscovery { source, company_id }`
   - Handler: calls `DiscoveryService::run_discovery`, prints stats

## Types / Functions

```rust
// service.rs
pub struct SourceConfig { pub source: String, pub company_id: String }
pub struct DiscoveryStats { pub jobs_found: usize, pub jobs_new: usize, pub jobs_updated: usize, pub errors: usize }
pub struct DiscoveryProgress { pub source: String, pub company_id: String, pub message: String }
pub struct DiscoveryService;
impl DiscoveryService {
    pub async fn run_discovery(
        pool: &PgPool,
        sources: Vec<SourceConfig>,
        progress_tx: Option<mpsc::Sender<DiscoveryProgress>>,
    ) -> Result<DiscoveryStats>
}

// Private helper
async fn discover_one(pool: PgPool, cfg: SourceConfig, tx: Option<mpsc::Sender<DiscoveryProgress>>) -> SourceResult
```

## Tests

### Learning Test
- `futures_join_all_collects_from_parallel_futures` — proves `futures::future::join_all` aggregates results

### Unit Tests
- `discovery_stats_add_aggregates_correctly` — tests stats + operator
- `source_config_clones_and_formats` — basic field access
- `discover_one_unknown_source_returns_error` — graceful error for unsupported source
- `run_discovery_empty_sources_returns_zero_stats` — empty input
- `run_discovery_aggregates_from_multiple_sources` — uses MockJobSource
- `run_discovery_sends_progress_events` — verifies channel receives messages

### Integration Tests (skip without DATABASE_URL)
- `upsert_discovered_returns_true_for_new` — verifies xmax trick for new row
- `upsert_discovered_returns_false_for_update` — verifies xmax trick for update
- `run_discovery_with_wiremock_greenhouse` — full flow with mock HTTP

## Migrations
None needed — migration 002 already has the partial unique index.

## Notes
- `futures` crate already in workspace (task 10)
- `DiscoveryStats::default()` derives for zero-initialization
- Each source future returns its own partial `DiscoveryStats`, aggregated at the end
- On source error: increment `errors`, send error progress message, continue with other sources
