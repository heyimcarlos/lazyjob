# Research: Task 20 — ralph-loop-types

## Task Summary
Implement `LoopType` enum, `LoopDispatch` priority queue, and `LoopScheduler` in `lazyjob-ralph/src/loop_types.rs`.

## Existing Codebase State
- `lazyjob-ralph` crate exists with: `error.rs`, `protocol.rs`, `process_manager.rs`, `lib.rs`
- `RalphError` has: `Decode(String)`, `Io(#[from] io::Error)`, `NotFound(String)` variants
- Workspace deps include: `chrono`, `serde`, `serde_json`, `uuid`, `thiserror`
- No cron parsing crate in workspace yet — needed for `LoopScheduler`

## Key Design Decisions

### LoopType variants
Per task description: `JobDiscovery`, `CompanyResearch`, `ResumeTailor`, `CoverLetter`, `InterviewPrep`

### Priority values (u8, higher = more urgent)
- `CoverLetter`: 90 — user-initiated, needs fast response
- `ResumeTailor`: 85 — user-initiated
- `InterviewPrep`: 70 — user-initiated
- `CompanyResearch`: 50 — background enrichment
- `JobDiscovery`: 30 — periodic background sweep

### Concurrency limits
- `JobDiscovery`: 1 (serial to avoid hammering job APIs)
- `CompanyResearch`: 2 (light network + LLM)
- `ResumeTailor`: 3 (CPU/LLM bound, user-triggered)
- `CoverLetter`: 3 (similar to resume)
- `InterviewPrep`: 2 (LLM heavy)

### is_interactive
- `InterviewPrep`: true (mock interview needs stdin I/O loop)
- All others: false

### cli_subcommand
- `JobDiscovery` → "job-discovery"
- `CompanyResearch` → "company-research"
- `ResumeTailor` → "resume-tailor"
- `CoverLetter` → "cover-letter"
- `InterviewPrep` → "interview-prep"

### LoopDispatch (BinaryHeap priority queue, cap 20)
- `QueuedLoop` struct: `loop_type: LoopType`, `params: serde_json::Value`, `enqueued_at: Instant`
- `BinaryHeap<QueuedLoop>` — max-heap, so higher priority pops first
- `Ord` impl on `QueuedLoop` by `priority()` value; tie-break by `enqueued_at` (earlier = higher)
- `enqueue()` returns `Err` if cap exceeded (not silently dropped)
- `drain_next()` -> `Option<QueuedLoop>` via `heap.pop()`
- `len()`, `is_empty()` helpers

### LoopScheduler
- Wraps `cron::Schedule` for cron expression parsing
- Uses `chrono::Utc` for time
- `LoopScheduler::new(expr: &str) -> Result<Self>` — parses cron expr
- `LoopScheduler::should_run(&self, now: DateTime<Utc>) -> bool` — checks if a tick occurred since last check
- Tracks `last_checked: DateTime<Utc>`; advances on each `should_run` call
- Initial `last_checked = now - 1ms` so first call can fire

## Dependencies
- Add `cron = "0.12"` to workspace deps and lazyjob-ralph deps
- `cron` crate uses `chrono` (already in workspace) for time handling
- No other new dependencies needed

## Files to Create/Modify
1. `crates/lazyjob-ralph/src/loop_types.rs` — LoopType, QueuedLoop, LoopDispatch
2. `crates/lazyjob-ralph/src/loop_scheduler.rs` — LoopScheduler
3. `crates/lazyjob-ralph/src/lib.rs` — add pub mods + re-exports
4. `crates/lazyjob-ralph/src/error.rs` — add `CronParse(String)` variant
5. `Cargo.toml` — add cron workspace dep
6. `crates/lazyjob-ralph/Cargo.toml` — add cron dep
