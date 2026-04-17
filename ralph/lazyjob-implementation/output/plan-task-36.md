# Plan: Task 36 — Resume Tailor Ralph Loop

## Files to Create

### `crates/lazyjob-cli/src/worker.rs` (new)
- `emit_event(event: &WorkerEvent)` — serialize + write to stdout
- `run_worker(db_url: &str) -> Result<()>` — read stdin, dispatch to handler
- `run_resume_tailor(params: Value, db_url: &str) -> Result<()>` — execute ResumeTailor pipeline
- `run_cover_letter(params: Value, db_url: &str) -> Result<()>` — execute CoverLetterService pipeline

## Files to Modify

### `crates/lazyjob-cli/src/main.rs`
- Add `mod worker;`
- Add `#[command(hide = true)] Worker` variant to Commands enum
- Add match arm calling `worker::run_worker(db_url)`

### `crates/lazyjob-cli/Cargo.toml`
- Add `lazyjob-ralph = { workspace = true }` dependency

### `crates/lazyjob-tui/src/app.rs`
- Add `RalphUpdate::Started { id, loop_type }` variant
- Add `RalphCommand` enum: `Spawn { loop_type, params }`, `Cancel { run_id }`
- Add `ralph_cmd_tx: mpsc::UnboundedSender<RalphCommand>` field to App
- Update `App::new()` to take `ralph_cmd_tx`
- Wire `TailorResume(job_id)` → send `RalphCommand::Spawn` + auto-switch to Ralph panel
- Wire `GenerateCoverLetter(job_id)` → send `RalphCommand::Spawn`
- Wire `CancelRalphLoop(run_id)` → send `RalphCommand::Cancel`

### `crates/lazyjob-tui/src/lib.rs`
- Keep `ralph_tx` alive (remove underscore prefix)
- Create `mpsc::unbounded_channel()` for RalphCommand
- Pass `ralph_tx`, `ralph_cmd_rx` to `run_event_loop`
- Pass `ralph_cmd_tx` to `App::new()`

### `crates/lazyjob-tui/src/event_loop.rs`
- Accept `ralph_tx: broadcast::Sender<RalphUpdate>` and `ralph_cmd_rx: mpsc::UnboundedReceiver<RalphCommand>`
- Create `RalphProcessManager::new()`
- Add select branch for `ralph_cmd_rx` → spawn/cancel on process_manager
- Add select branch for process_manager subscriber → map to RalphUpdate, send on ralph_tx

### `crates/lazyjob-tui/src/views/ralph_panel.rs`
- Handle `RalphUpdate::Started` variant in `on_update()` — create ActiveEntry with correct loop_type

## Tests

### CLI Tests
- `parse_worker_subcommand` — verify Worker variant parses
- `worker_event_serialization` — verify emit_event format

### TUI Tests
- `tailor_resume_sends_spawn_command` — handle_action TailorResume → pending spawn
- `generate_cover_letter_sends_spawn_command` — same for cover letter
- `cancel_ralph_loop_sends_cancel_command` — handle_action Cancel → pending cancel
- `started_update_sets_loop_type` — ralph_panel Started variant

### Integration Tests
- Worker subprocess echo test with mock binary (already exists in process_manager)

## No New Dependencies
All crates already in workspace. Only adding lazyjob-ralph to lazyjob-cli Cargo.toml.
