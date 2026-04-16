# Plan: Task 20 — ralph-loop-types

## Files to Create
- `crates/lazyjob-ralph/src/loop_types.rs` — LoopType enum + methods + QueuedLoop + LoopDispatch
- `crates/lazyjob-ralph/src/loop_scheduler.rs` — LoopScheduler

## Files to Modify
- `crates/lazyjob-ralph/src/error.rs` — add CronParse variant
- `crates/lazyjob-ralph/src/lib.rs` — add pub mod loop_types, loop_scheduler + re-exports
- `Cargo.toml` — add cron = "0.12" to workspace.dependencies
- `crates/lazyjob-ralph/Cargo.toml` — add cron workspace dep

## Types/Functions

### loop_types.rs
```rust
pub enum LoopType { JobDiscovery, CompanyResearch, ResumeTailor, CoverLetter, InterviewPrep }
impl LoopType {
    pub fn concurrency_limit(&self) -> usize
    pub fn priority(&self) -> u8
    pub fn is_interactive(&self) -> bool
    pub fn cli_subcommand(&self) -> &str
}

pub struct QueuedLoop { pub loop_type: LoopType, pub params: serde_json::Value, enqueued_at: std::time::Instant }
impl Ord/PartialOrd for QueuedLoop  // by priority, then enqueued_at (earlier = higher)

pub struct LoopDispatch { heap: BinaryHeap<QueuedLoop>, cap: usize }
impl LoopDispatch {
    pub fn new() -> Self  // cap = 20
    pub fn enqueue(&mut self, loop_type: LoopType, params: Value) -> Result<()>  // error if full
    pub fn drain_next(&mut self) -> Option<QueuedLoop>
    pub fn len(&self) -> usize
    pub fn is_empty(&self) -> bool
}
```

### loop_scheduler.rs
```rust
pub struct LoopScheduler { schedule: cron::Schedule, last_checked: DateTime<Utc> }
impl LoopScheduler {
    pub fn new(expr: &str) -> Result<Self>
    pub fn should_run(&mut self, now: DateTime<Utc>) -> bool
    pub fn next_run_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>>
}
```

## Tests

### Learning Tests
- `cron_schedule_parses_standard_expr` — proves `cron::Schedule::from_str()` accepts a standard 6-field cron expression and returns upcoming times
- `cron_schedule_upcoming_iterator` — proves `schedule.upcoming(Utc)` returns an iterator with correct next-tick values

### Unit Tests (loop_types.rs)
- `all_loop_types_have_positive_priority` — priority > 0 for all variants
- `cover_letter_higher_priority_than_discovery` — CoverLetter.priority() > JobDiscovery.priority()
- `job_discovery_concurrency_limit_is_1` — concurrency limit constraint
- `interactive_only_for_interview_prep` — only InterviewPrep.is_interactive()
- `cli_subcommands_are_kebab_case` — contains '-' separator
- `loop_dispatch_priority_ordering` — higher priority drains first
- `loop_dispatch_respects_cap` — enqueue 21st returns Err
- `loop_dispatch_drain_empty_returns_none`
- `queued_loop_earlier_enqueue_wins_tie` — same priority, earlier enqueued comes first

### Unit Tests (loop_scheduler.rs)
- `scheduler_fires_on_matching_tick` — time advance past a scheduled tick → should_run = true
- `scheduler_silent_before_tick` — no advance → should_run = false
- `scheduler_next_run_after_returns_future_time`
- `scheduler_rejects_invalid_cron` — malformed expression returns Err
