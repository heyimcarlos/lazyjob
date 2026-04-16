# Research: Task 21 — ralph-crash-recovery

## Key Findings

### Table Already Exists
The `ralph_loop_runs` table was created in migration 001 with:
- id UUID PRIMARY KEY
- loop_type TEXT NOT NULL
- status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','done','failed','cancelled'))
- params_json JSONB (nullable)
- started_at TIMESTAMPTZ (nullable)
- finished_at TIMESTAMPTZ (nullable)
- created_at TIMESTAMPTZ NOT NULL DEFAULT now()

No migration 002 is needed. The table is already there.

### Repository Pattern
Follow the pattern from `JobRepository`:
- Constructor takes `PgPool`
- Uses runtime `sqlx::query` / `sqlx::query_as` (no compile-time macros, no DATABASE_URL at build time)
- Intermediate `*Row` struct with `sqlx::FromRow` for reading
- Domain struct separate from DB row struct
- `Result<T>` from `crate::error`

### TUI Integration
`run()` in `lazyjob-tui/src/lib.rs` is async and accepts `Arc<Config>`. It creates App and calls `run_event_loop()`.
The cleanest wiring is:
1. Connect to DB in `run()` (optional — skip if DATABASE_URL not set)
2. Call `recover_pending()` 
3. Pass the pool to App for future use

App::new() is synchronous, so actual recovery must happen before App creation (in the async `run()` function).

### Status Enum
Status values per the CHECK constraint: pending, running, done, failed, cancelled
Rust enum variant names: Pending, Running, Done, Failed, Cancelled (PascalCase)
Stored as snake_case TEXT in PG.

### Recovery Logic
Find runs WHERE status = 'running' AND started_at < now() - interval '30 seconds'.
Update their status to 'failed'.
Returns count of recovered runs.

Rationale: if a run is 'running' but older than 30s and the TUI just started, the previous process crashed without cleanup.

### No New Crates
All dependencies already in workspace: sqlx, uuid, chrono, serde_json.
