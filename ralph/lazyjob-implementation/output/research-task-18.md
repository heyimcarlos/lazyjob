# Research: Task 18 — ralph-protocol

## Task Summary
Implement NDJSON protocol types in `lazyjob-ralph/src/protocol.rs`:
- `WorkerCommand` enum (TUI → Ralph subprocess)
- `WorkerEvent` enum (Ralph subprocess → TUI)
- `NdjsonCodec` struct with `encode`/`decode` helpers

## Existing State
- `crates/lazyjob-ralph/` exists as a workspace member
- `src/lib.rs` has only a `version()` fn — no modules yet
- `Cargo.toml` already has: lazyjob-core, thiserror, anyhow, serde, serde_json, tokio, uuid
- No error.rs, no protocol.rs yet

## Key Design Decisions from Spec + Task Description

### WorkerCommand vs IncomingMessage
Task description uses `WorkerCommand` / `WorkerEvent` (not the spec's `IncomingMessage` / `OutgoingMessage`).
Task description takes priority — use WorkerCommand / WorkerEvent.

### loop_type field in WorkerCommand::Start
Task 20 defines the `LoopType` enum. For this task, use `String` for `loop_type` in
`WorkerCommand::Start` — forward-compatible and avoids coupling to an unimplemented type.

### NdjsonCodec API
- `encode(cmd: &WorkerCommand) -> String` — infallible (WorkerCommand is always serializable)
  - Uses `serde_json::to_string().expect(...)` internally
  - Appends `\n`
- `decode(line: &str) -> Result<WorkerEvent>` — fallible (user-provided input may be invalid JSON)

### Serde Tagging
Both enums use `#[serde(tag = "type", rename_all = "snake_case")]` as specified.
This means `WorkerCommand::Cancel` serializes as `{"type":"cancel"}`.

### Error Module
New `RalphError` enum with thiserror:
- `Encode(serde_json::Error)` — for completeness (though encode is made infallible)
- `Decode(String)` — wraps serde_json parse errors with context

## Dependencies
- `serde` with derive — already in workspace deps, already in ralph Cargo.toml
- `serde_json` — already in ralph Cargo.toml
- `thiserror` — already in ralph Cargo.toml

No new deps needed for this task.

## Files to Create/Modify
1. `crates/lazyjob-ralph/src/error.rs` — NEW: RalphError, Result<T>
2. `crates/lazyjob-ralph/src/protocol.rs` — NEW: WorkerCommand, WorkerEvent, NdjsonCodec
3. `crates/lazyjob-ralph/src/lib.rs` — MODIFY: add pub mod error, pub mod protocol, pub use

## Tests Required
- Round-trip serde for all WorkerCommand variants (Start, Cancel)
- Round-trip serde for all WorkerEvent variants (Status, Results, Error, Done)
- NdjsonCodec::encode produces JSON + newline
- NdjsonCodec::decode parses valid NDJSON line
- NdjsonCodec::decode returns error for invalid JSON
- JSON tag format verification ({"type":"cancel"} etc.)
