# Plan: Task 21 — ralph-crash-recovery

## Files to Create/Modify

### Create
- `crates/lazyjob-core/src/repositories/ralph_loop_run.rs` — RalphLoopRunRepository + domain types

### Modify
- `crates/lazyjob-core/src/repositories/mod.rs` — add pub mod ralph_loop_run + re-export
- `crates/lazyjob-tui/src/lib.rs` — call recover_pending() in run(), pass pool to App
- `crates/lazyjob-tui/src/app.rs` — add pool: Option<PgPool> field, new_with_pool() constructor

## Types / Functions to Define

### `crates/lazyjob-core/src/repositories/ralph_loop_run.rs`
```rust
pub enum RalphLoopRunStatus { Pending, Running, Done, Failed, Cancelled }
impl RalphLoopRunStatus { pub fn as_str(&self) -> &str }
impl FromStr for RalphLoopRunStatus

pub struct RalphLoopRun {
    pub id: uuid::Uuid,
    pub loop_type: String,
    pub status: RalphLoopRunStatus,
    pub params_json: Option<serde_json::Value>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
impl RalphLoopRun { pub fn new(loop_type: &str) -> Self }

pub struct RalphLoopRunRepository { pool: PgPool }
impl RalphLoopRunRepository {
    pub fn new(pool: PgPool) -> Self
    pub async fn insert_run(&self, run: &RalphLoopRun) -> Result<()>
    pub async fn update_status(&self, id: &uuid::Uuid, status: RalphLoopRunStatus) -> Result<()>
    pub async fn list_pending(&self) -> Result<Vec<RalphLoopRun>>
    pub async fn recover_pending(&self) -> Result<usize>
}

struct RalphLoopRunRow { ... } // sqlx::FromRow intermediate
impl TryFrom<RalphLoopRunRow> for RalphLoopRun
```

## Tests to Write

### Unit Tests (no DB needed)
- `ralph_loop_run_status_as_str` — verifies each variant serializes to expected string
- `ralph_loop_run_status_from_str` — verifies round-trip parsing for all valid values
- `ralph_loop_run_status_from_str_invalid` — verifies parse error on unknown value
- `ralph_loop_run_new_has_pending_status` — verifies RalphLoopRun::new() starts as Pending

### Integration Tests (skip if DATABASE_URL unset)
- `insert_and_list_pending` — insert a run with status Pending, verify list_pending() returns it
- `update_status_changes_status` — insert Pending, update to Running, verify by listing
- `recover_pending_marks_stale_running_as_failed` — insert Running with old started_at, call recover_pending(), verify count and status
- `recover_pending_skips_recent_runs` — insert Running with started_at = now(), verify not recovered
- `recover_pending_ignores_done_runs` — insert Done, verify recover returns 0

## No Migrations Needed
The `ralph_loop_runs` table already exists in migration 001. No 002 needed.

## TUI Wiring
In `run()`: connect to DB if DATABASE_URL is available, call `RalphLoopRunRepository::new(pool.clone()).recover_pending().await`, pass pool (as `Option<PgPool>`) to `App::new_with_pool()`.
App gains `pool: Option<PgPool>` field for future view implementations.
