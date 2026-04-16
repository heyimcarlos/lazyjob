# Plan: Task 18 — ralph-protocol

## Files to Create/Modify

### NEW: `crates/lazyjob-ralph/src/error.rs`
- `RalphError` enum (thiserror):
  - `Decode(String)` — wraps JSON decode errors with context
- `pub type Result<T> = std::result::Result<T, RalphError>`

### NEW: `crates/lazyjob-ralph/src/protocol.rs`
- `WorkerCommand` enum — `#[serde(tag="type", rename_all="snake_case")]`
  - `Start { loop_type: String, params: serde_json::Value }`
  - `Cancel`
- `WorkerEvent` enum — `#[serde(tag="type", rename_all="snake_case")]`
  - `Status { phase: String, progress: f32, message: String }`
  - `Results { data: serde_json::Value }`
  - `Error { code: String, message: String }`
  - `Done { success: bool }`
- `NdjsonCodec` struct (zero-size, all methods are associated fns)
  - `encode(cmd: &WorkerCommand) -> String`
  - `decode(line: &str) -> Result<WorkerEvent>`
- Unit tests (in `#[cfg(test)]` block):
  - Learning tests: `serde_tagged_enum_serializes_type_field`, `serde_json_value_roundtrip`
  - Round-trip tests for all WorkerCommand variants
  - Round-trip tests for all WorkerEvent variants
  - NdjsonCodec encode/decode tests
  - Error case tests

### MODIFY: `crates/lazyjob-ralph/src/lib.rs`
- Add `pub mod error`
- Add `pub mod protocol`
- Add re-exports: `pub use error::{RalphError, Result}`, `pub use protocol::{WorkerCommand, WorkerEvent, NdjsonCodec}`
- Keep existing `version()` fn

## No migrations needed for this task.

## No new crate dependencies needed.
