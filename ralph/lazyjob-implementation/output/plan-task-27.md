# Plan: Task 27 — jobs-list-tui (Completion Pass)

## Files to Modify

1. **crates/lazyjob-tui/src/action.rs** — Add `OpenJob(JobId)` variant
2. **crates/lazyjob-tui/src/views/jobs_list.rs** — Add Stage column, fix Enter, fix Applied filter, add application_stages
3. **crates/lazyjob-tui/src/app.rs** — Handle `OpenJob` action, add async `load_jobs` method
4. **crates/lazyjob-tui/src/event_loop.rs** — Call load_jobs on startup (if pool available)
5. **crates/lazyjob-tui/src/lib.rs** — Wire initial job loading in run()

## Types/Functions to Add

### action.rs
- `Action::OpenJob(lazyjob_core::domain::JobId)` — signal to open job detail

### jobs_list.rs
- `application_stages: HashMap<JobId, String>` field — per-job stage display
- `set_application_stages(HashMap<JobId, String>)` method
- Stage column in table (6th column, between Ghost and Posted)
- Fix `handle_key` Enter to return `Action::OpenJob(id)`
- Fix `match_filter_idx` Applied case to check `application_stages`
- Remove `Enter | _` catch-all, use explicit `_` wildcard

### app.rs
- `handle_action(OpenJob(_))` — no-op for now (task 28 wires to JobDetailView)
- `async load_jobs(&mut self)` — query JobRepository, call set_jobs()

## Tests to Add
1. `handle_key_enter_returns_open_job` — Enter with selected job returns OpenJob
2. `handle_key_enter_returns_none_when_empty` — Enter with no jobs returns None
3. `filter_applied_shows_only_applied_jobs` — Applied filter uses application_stages
4. `stage_column_renders_in_table` — Table contains stage text
5. `set_application_stages_updates_filter` — Setting stages makes Applied filter work

## Migrations
None needed.
