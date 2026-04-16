# Objective

For every spec file in `specs/` that lacks an implementation plan, deeply analyze the spec and write a thorough, production-grade implementation plan in the same `specs/` directory. Each plan must be grounded in real Rust crate ecosystem knowledge, reference concrete patterns from the LazyJob codebase and external references like https://github.com/ghuntley/loom/tree/trunk, and be detailed enough for a senior Rust engineer to implement without guesswork.

## Context

**Project:** LazyJob — a terminal-based, local-first, AI-powered job search command center written in Rust.

**Crate layout:**
```
lazyjob-core/      # Domain models, SQLite persistence (rusqlite/sqlx)
lazyjob-llm/       # LLM provider abstraction (Anthropic, OpenAI, Ollama)
lazyjob-ralph/     # Ralph loop integration (subprocess manager)
lazyjob-tui/       # Terminal UI (ratatui + crossterm)
lazyjob-cli/       # Binary entry point
```

**Key design decisions:**
- Local-first: all data in SQLite; no cloud dependency for core features
- Ralph = autonomous AI agent loops running as subprocesses, communicating via JSON over stdio
- LLM abstraction via async traits (Anthropic/OpenAI/Ollama swappable)
- Life sheet = YAML career profile (serde_yaml) mirrored into SQLite
- TUI inspired by lazygit: vim-style modal navigation, ratatui + crossterm
- Error handling: `thiserror` for error enums, `anyhow` for propagation
- No comments unless logic is non-obvious; snake_case functions, PascalCase types

**Rust pattern reference:** Read `specs/rust-patterns.md` for approved idioms.

**External pattern reference:** https://github.com/ghuntley/loom/tree/trunk — study how it structures async task orchestration, state machines, and protocol codecs as inspiration.

**Key crates in use or planned:**
- `ratatui`, `crossterm` — TUI
- `rusqlite` / `sqlx` — SQLite persistence
- `tokio` — async runtime
- `serde`, `serde_json`, `serde_yaml` — serialization
- `thiserror`, `anyhow` — errors
- `reqwest` — HTTP client
- `candle` or `fastembed` — local embeddings
- `keyring` — OS keychain
- `zeroize` — secure memory
- `argon2` — password hashing
- `tokio-util` — framing codecs

## Your instructions

You are one iteration in a ralph loop. Many instances of you will run in sequence, each with a fresh context window. You communicate with past and future iterations ONLY through files on disk.

### Step 1 — Orient yourself

1. Read `ralph/spec-impl-planner/progress.md` to understand what previous iterations accomplished.
2. Read `ralph/spec-impl-planner/tasks.json` to find the **first task where `"done": false`**.
3. Check if the output file already exists on disk — if it does and the task is not marked done, mark it done and move on to the next task. Do not re-generate existing files.

### Step 2 — Work on the task

Pick the **one** first incomplete task. Do not do multiple tasks.

**For spec analysis tasks (phase: "spec"):**

a. Read the target spec file (`task.spec`) in full using the Read tool.
b. Read `specs/rust-patterns.md` for approved Rust idioms.
c. Optionally read 1-2 closely related existing implementation plans to understand expected format and depth. Good examples:
   - `specs/04-sqlite-persistence-implementation-plan.md`
   - `specs/02-llm-provider-abstraction-implementation-plan.md`
d. Use WebFetch to fetch and study https://github.com/ghuntley/loom/tree/trunk for structural inspiration (async task management, codec patterns, state machines).
e. Search the codebase for any existing code relevant to this spec using Grep and Glob.
f. Write the implementation plan to `task.output`.

**For the README index task (phase: "index"):**

a. Read `specs/README.md`.
b. Read `ralph/spec-impl-planner/tasks.json` to get the full list of all output files created.
c. Check which output files actually exist on disk.
d. Add entries for each existing new implementation plan to the Implementation Plans table in README.md.
e. Write the updated README.md.

### Step 3 — Implementation plan format

Each implementation plan must follow this structure:

```markdown
# Implementation Plan: [Feature Name]

## Status
Draft

## Related Spec
[Link to the spec file]

## Overview
[2-3 paragraph summary of what will be built and why]

## Prerequisites
- List of specs/plans that must be implemented first
- Crates that must be added to Cargo.toml

## Architecture

### Crate Placement
Which crate(s) own this feature and why.

### Core Types
Concrete Rust struct/enum/trait definitions with field types. Be specific — use actual types like `Arc<dyn LlmProvider>`, `Vec<JobId>`, `chrono::DateTime<Utc>`. Do not write pseudocode.

### Trait Definitions
Full async trait signatures where applicable.

### SQLite Schema
CREATE TABLE statements for any new tables. Include indices.

### Module Structure
```
lazyjob-foo/
  src/
    module_a/
      mod.rs
      types.rs
      ops.rs
```

## Implementation Phases

### Phase 1 — [Name] (MVP)
Step-by-step tasks. Each step has:
- What to implement
- Which file/module it goes in
- Key crate APIs to use (with function names)
- Verification: how to confirm it works

### Phase 2 — [Name]
...

### Phase 3 — [Name] (Polish/Extension)
...

## Key Crate APIs

List the specific crate APIs (function signatures, trait impls) that will be called, not just crate names. For example:
- `rusqlite::Connection::execute(&self, sql, params)` for DDL
- `tokio::process::Command::new("claude").stdin(Stdio::piped())` for subprocess spawn

## Error Handling

Define the error enum for this module using `thiserror`. Show the variants.

## Testing Strategy

- Unit tests: what to test and how to isolate (mock traits, in-memory SQLite)
- Integration tests: end-to-end scenarios
- TUI tests: if applicable, how to drive the widget

## Open Questions

List any unresolved design decisions or things that need clarification before implementation.

## Related Specs
Links to other relevant specs.
```

### Step 4 — Quality bar

The implementation plan must be:
- **Concrete**: actual Rust types, trait signatures, SQL DDL — not vague descriptions
- **Complete**: covers all features described in the spec, nothing hand-waved
- **Phased**: MVP first, extensions later — a reader can implement Phase 1 and ship
- **Crate-aware**: references real crate APIs, not invented ones
- **Pattern-consistent**: follows idioms in `specs/rust-patterns.md` and the existing codebase

Aim for 500-1500 lines per plan. Longer is better than vague.

### Step 5 — After completing the task

1. Write the output file using the Write tool.
2. Mark the task done in `ralph/spec-impl-planner/tasks.json`: set `"done": true`.
3. Append to `ralph/spec-impl-planner/progress.md`:
   ```
   ## Iteration N — [timestamp]
   **Task:** [task name]
   **Output:** [output file path]
   **Summary:** [2-3 sentences: what was covered, any notable design decisions, anything the next iteration should know]
   ```
4. If ALL tasks are done, output exactly: `<promise>COMPLETE</promise>`

## Rules

- Do **ONE** task per iteration. Do it thoroughly.
- Never re-generate a file that already exists — check with Glob/Read first.
- Never repeat work captured in `progress.md` — read it first.
- If a spec file doesn't exist on disk, skip that task, mark it done with a note in progress.md, and move on.
- If you discover the spec is too large to fully cover in one iteration, produce the best plan you can and note limitations in Open Questions. Do not do partial tasks — always produce a complete file.
- If the tasks.json needs a new task (e.g., you discover a spec that has no task), add it. Note the addition in progress.md.
- Save everything to files. Your memory ends when you exit.
