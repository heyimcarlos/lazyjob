# Research: Task 19 — ralph-process-manager

## Task Summary
Implement `RalphProcessManager` in `lazyjob-ralph/src/process_manager.rs`.

## Existing State

### lazyjob-ralph crate already has:
- `src/error.rs` — `RalphError { Decode(String) }` + `Result<T>`
- `src/protocol.rs` — `WorkerCommand`, `WorkerEvent`, `NdjsonCodec`
- `src/lib.rs` — minimal: pub mod error, pub mod protocol

### Dependencies already in crate:
- `tokio = { workspace = true }` (with "full" feature — includes process, io, time, sync)
- `uuid = { workspace = true }` (v4 + serde)
- `serde_json = { workspace = true }`
- All others (thiserror, anyhow, serde, lazyjob-core) present

## Protocol Design

WorkerCommand is sent to subprocess stdin (NDJSON):
- `{"type":"start","loop_type":"job_discovery","params":{}}` → subprocess starts working
- `{"type":"cancel"}` → subprocess should gracefully shut down

WorkerEvent is received from subprocess stdout (NDJSON lines):
- `{"type":"status","phase":"init","progress":0.1,"message":"..."}` → progress update
- `{"type":"results","data":{...}}` → results payload
- `{"type":"error","code":"...","message":"..."}` → error
- `{"type":"done","success":true}` → completed

## Key Design Decisions

### subprocess invocation
Spawns `<current_exe> worker` with piped stdin/stdout. The `worker` subcommand will be added in task 20+. For tests we supply a custom binary via `with_binary()`.

### Event broadcast shape
`broadcast::Sender<(RunId, WorkerEvent)>` — tags each event with its RunId so subscribers know which subprocess produced the event.

### Error additions needed
- `RalphError::Io(#[from] std::io::Error)` — for spawn/stdin write failures
- `RalphError::NotFound(String)` — for cancel of unknown RunId

### Test strategy for mock subprocess
Create a temp executable shell script that:
1. Reads one line from stdin (the WorkerCommand::Start)
2. Emits a Status WorkerEvent
3. Emits a Done WorkerEvent
4. Exits

This allows testing `spawn()` end-to-end without a real ralph binary.
`cancel()` is tested against a long-running `sleep` process.

### tokio::process::ChildStdin
`ChildStdin` is moved out of `Child` via `take()` and stored separately in `ProcessHandle`. This is necessary because `Child` does not allow concurrent access to its stdin handle.

## Files to modify
1. `crates/lazyjob-ralph/src/error.rs` — add Io + NotFound variants
2. `crates/lazyjob-ralph/src/process_manager.rs` — NEW: full implementation + tests
3. `crates/lazyjob-ralph/src/lib.rs` — add pub mod process_manager + re-exports
