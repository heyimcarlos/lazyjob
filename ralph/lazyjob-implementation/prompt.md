# Objective

Build LazyJob incrementally — a lazygit-style Rust TUI for AI-powered job searching, with autonomous agent loops (Ralph) that write tailored resumes and cover letters based on the user's life sheet.

The end goal: a working local app where the user runs `lazyjob tui`, discovers jobs from Greenhouse/Lever and other open job APIs or job banks, and triggers AI-generated cover letters + resumes tailored to each job — all grounded in who they actually are, written in their voice, using their real experience and already existing resumes and cover letters.

## Context

- **Repo**: `/home/lab-admin/repos/lazyjob`
- **Language**: Rust (Cargo workspace)
- **Current state**: Only `src/main.rs` with `println!("Hello, world!")` — nothing is implemented yet
- **Specs directory**: `specs/` — contains detailed specs and implementation plans for every feature
- **Architecture**: 5-crate workspace (`lazyjob-core`, `lazyjob-llm`, `lazyjob-ralph`, `lazyjob-tui`, `lazyjob-cli`) under `/crates`
- **Key tech**: ratatui 0.29, sqlx 0.8 (postgres), tokio 1, clap 4, reqwest (rustls), async-openai, docx-rs
- **Database**: PostgreSQL (not SQLite). Use sqlx with `runtime-tokio` + `postgres` features. Connection via DATABASE_URL env var or config file. Use sqlx::migrate!() for migrations.
- **Code style**: No comments unless complex. thiserror for errors, anyhow for propagation. snake_case/PascalCase/SCREAMING_CASE.

## Your Instructions

You are one iteration in a ralph loop. Many instances of you will run in sequence, each with a fresh context window. You communicate with past and future iterations **only through files on disk**.

### Step 1 — Orient yourself

1. Read `ralph/lazyjob-implementation/progress.md` to understand what previous iterations accomplished
2. Read `ralph/lazyjob-implementation/tasks.json` to find the **first task where `"done": false`**
3. Read the relevant spec file(s) listed in the task's `"spec"` field (under `/home/lab-admin/repos/lazyjob/`)
4. Spin up parallel sub-agents to read the existing codebase files relevant to the task. Your options are `codebase-analyzer`, `codebase-locator`, `codebase-pattern-finder`, and `web-search-researcher`


### Step 2 — Research phase

Before writing code, deeply understand what you're about to build:

1. Read the spec file for this task. Note key types, interfaces, and design decisions.
2. Read related implementation plan files. Understand the full context.
3. Check `specs/rust-patterns.md` if this task uses patterns new to the codebase.
4. Search the existing codebase for any partial implementations or related code that must be considered.
5. Identify all dependencies this code needs (crates, other lazyjob-* modules).
6. **IF YOU HAVEN'T DONE SO"*: Spin up parallel sub-agents to read the existing codebase files relevant to the task. Your options are `codebase-analyzer`, `codebase-locator`, `codebase-pattern-finder`, and `web-search-researcher`

Document your research findings in `ralph/lazyjob-implementation/output/research-task-{id}.md`.

### Step 3 — Plan phase

Before writing code, write a micro-plan:

1. List the files you will create or modify (with paths)
2. List the types/functions/structs you will define
3. List the tests you will write:
   - **Learning tests** (marked `#[cfg(test)]` with comment `// learning test: verifies library behavior`): small tests that demonstrate you understand how a new crate/API works before using it
   - **Unit tests**: test individual functions/methods in isolation
   - **Integration tests**: test interactions between components
4. List any migrations needed

Write the plan as `ralph/lazyjob-implementation/output/plan-task-{id}.md`.

### Step 4 — Implement phase

Now write the actual Rust code:

1. Create/modify the files according to your plan
2. Follow the code style strictly:
   - No comments unless logic is genuinely non-obvious
   - `thiserror` for public error enums, `anyhow` for propagation
   - Group imports: std → external crates → lazyjob-* crates
   - Define `type Result<T> = std::result::Result<T, YourError>` in each module
3. Write learning tests first (to prove you understand the library)
4. Write unit tests for each non-trivial function
5. Keep implementations clean — no TODOs, no dead code, no stubs except where the task explicitly says "stub"

### Step 5 — Verify

After implementing:

1. Run `cargo build` — must pass with zero errors
2. Run `cargo clippy -- -D warnings` — must pass with zero warnings
3. Run `cargo test` — all tests must pass
4. Run `cargo fmt --all` — format everything

If any step fails, debug and fix before proceeding. Do NOT mark the task done if the build fails.

If you encounter a compile error you cannot resolve in 3 attempts, document the blocker in progress.md and move to the next task — do not spin forever.

### Step 6 — Update state

1. Mark the completed task as `"done": true` in `tasks.json`
2. Append a progress entry to `progress.md` in this format:

```
## Task {id}: {name} — DONE
Date: {today}
Files created/modified:
- path/to/file.rs
Key decisions:
- [any important design decisions made]
Learning tests written:
- [describe each learning test and what it proved]
Tests passing: {count}
Next iteration should know:
- [anything that will affect the next task]
```

3. If you discovered the task needs to be split, or a dependency is missing, update `tasks.json` (add new tasks, reorder) and explain why in progress.md.

4. If ALL tasks in tasks.json are done, output: `<promise>COMPLETE</promise>`

### Step 7 - Task delivery
**IMPORTANT:** You cannot move on to the next task unless the steps mentioned below are fully completed.

1. Spin up a terminal. Run the newly implemented feature/changes step by step. Take a screenshot and video of every step you do in the demonstration.
2. Present your work as a single HTML slide deck. Include an index for navigation to the left, and use the video and screenshots taken in step 1.

## Rules

- **One task per iteration.** Do it thoroughly. Don't rush ahead.
- **One slide deck per task.**
- **Read progress.md first.** Never repeat work already done.
- **Always build and test.** A task is not done until `cargo build && cargo test` pass.
- **Learning tests are mandatory** for tasks that introduce a new external crate. Write them before writing the real implementation. They prove you understand the library's API before you depend on it.
- **No half-implementations.** If a task says "implement X", implement X completely — not a stub unless the task description says stub.
- **Respect the spec.** If the spec says use `sqlx query_as!` macros, use them. If it says use `tokio::select!`, use it. The spec exists because these patterns were chosen deliberately.
- **Fix your own bugs.** If `cargo test` fails, diagnose why and fix it. Don't mark a task done with failing tests.
- **Adapt the plan if needed.** If you find a task is blocked by a missing prerequisite, add the prerequisite as a new task before it in tasks.json. Note this in progress.md.
- **Cargo workspace note**: The workspace root Cargo.toml must list all member crates. Each member crate has its own Cargo.toml. Shared dependencies go in the workspace [dependencies] table with `workspace = true`.
- **PostgreSQL, not SQLite.** The specs reference SQLite — ignore that. Use PostgreSQL everywhere. sqlx features: `runtime-tokio`, `postgres`. Connection string from `DATABASE_URL` env var or config. Use `PgPool` not `SqlitePool`. Use PostgreSQL-native types (SERIAL, TIMESTAMPTZ, TEXT[], JSONB) instead of SQLite equivalents. Migrations go in `lazyjob-core/migrations/` and run via `sqlx::migrate!()`.

## Output location

All research and plan documents: `ralph/lazyjob-implementation/output/`
All implementation code: under the appropriate crate directory in the repo root (e.g., `lazyjob-core/src/`, `lazyjob-tui/src/`, etc.)
