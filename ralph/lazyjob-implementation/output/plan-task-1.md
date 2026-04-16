# Plan: Task 1 — Workspace Setup

## Files to Create/Modify

### Modify
- `Cargo.toml` — convert to workspace root (remove [package], add [workspace])

### Delete
- `src/main.rs` — content moves to lazyjob-cli

### Create
- `lazyjob-core/Cargo.toml`
- `lazyjob-core/src/lib.rs`
- `lazyjob-llm/Cargo.toml`
- `lazyjob-llm/src/lib.rs`
- `lazyjob-ralph/Cargo.toml`
- `lazyjob-ralph/src/lib.rs`
- `lazyjob-tui/Cargo.toml`
- `lazyjob-tui/src/lib.rs`
- `lazyjob-cli/Cargo.toml`
- `lazyjob-cli/src/main.rs`

## Types/Functions/Structs

### lazyjob-core/src/lib.rs
- `pub fn version() -> &'static str` — returns crate version for cross-crate test

### lazyjob-cli/src/main.rs
- `fn main()` — prints hello with version from core

### All other lib.rs
- Empty stubs (just a comment-free file)

## Tests

### Learning test (lazyjob-cli)
- Test in `lazyjob-core/src/lib.rs` `#[cfg(test)]` module: verify version() returns expected string
- Test in `lazyjob-cli` integration test: verify lazyjob-core types are accessible from cli crate

## Dependency Strategy
- Only add deps each crate needs RIGHT NOW for compilation
- Future tasks will add specific deps as needed (sqlx, ratatui, clap, etc.)
- Workspace deps: serde, thiserror, anyhow, tokio, uuid, chrono (these are universal)
