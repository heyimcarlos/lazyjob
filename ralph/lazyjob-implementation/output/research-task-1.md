# Research: Task 1 — Workspace Setup

## Current State
- Single `Cargo.toml` at root with `name = "lazyjob"`, edition 2024
- Single `src/main.rs` with `println!("Hello, world!")`
- No workspace, no sub-crates

## Target Architecture
5-crate workspace:
- `lazyjob-core` (lib) — domain models, persistence, state
- `lazyjob-llm` (lib) — LLM provider abstraction
- `lazyjob-ralph` (lib) — Ralph subprocess integration
- `lazyjob-tui` (lib) — TUI views and widgets
- `lazyjob-cli` (bin) — CLI entry point, moves existing main.rs here

## Key Decisions from Spec
- `resolver = "2"` for workspace
- Shared dependencies in `[workspace.dependencies]`
- Each crate uses `workspace = true` for shared deps
- Edition 2024 (already set in current Cargo.toml)

## Dependencies per Crate (from spec)

### Workspace shared deps
- uuid 1.8 (v4, serde), chrono 0.4 (serde), thiserror 1.0, anyhow 1.0
- tokio 1.0 (full), serde 1.0 (derive), serde_yaml 0.9, serde_json 1.0
- tracing 0.1, tracing-subscriber 0.3

### lazyjob-core
- uuid, chrono, thiserror, anyhow, serde, tokio, serde_json, serde_yaml

### lazyjob-llm
- tokio, serde, serde_json, thiserror, anyhow
- (async-trait, reqwest to be added in later tasks)

### lazyjob-ralph
- tokio, serde, serde_json, uuid, thiserror, anyhow

### lazyjob-tui
- lazyjob-core (path), lazyjob-llm (path), lazyjob-ralph (path)
- (ratatui, crossterm to be added in later tasks)

### lazyjob-cli
- lazyjob-core (path), lazyjob-tui (path)
- tokio, anyhow, tracing, tracing-subscriber
- (clap to be added in later tasks)

## Approach
1. Convert root Cargo.toml to workspace definition
2. Remove `src/main.rs` (move content to lazyjob-cli)
3. Create each crate with minimal lib.rs/main.rs
4. Only add dependencies that are needed NOW (avoid premature deps)
5. Write a learning test in lazyjob-cli that imports types from lazyjob-core
