# Spec: Ralph Loop Orchestration

**JTBD**: Let AI handle tedious job search work autonomously while I focus on high-signal decisions
**Topic**: How the TUI schedules, queues, and dispatches ralph loop workers across all loop types
**Domain**: agentic

---

## What

This spec defines the orchestration layer that sits above the IPC protocol: which loop types exist, how they are scheduled (on-demand vs. periodic), how concurrency is bounded, how loop priority is assigned, and how `PostTransitionSuggestion` events from the application-tracking domain are translated into ralph loop dispatches. The central type is `LoopType` and the central entry point is `lazyjob-ralph/src/dispatch.rs`.

## Why

Without an orchestration layer, the TUI would need to know which specific ralph binary flags to pass for every user action. That coupling would make adding loop types require changes across the TUI, the protocol layer, and the binary. Orchestration separates "what the user triggered" (a state transition, a menu action) from "how ralph handles it" (which loop type, which params). It also prevents resource exhaustion — a user could trigger three simultaneous resume tailoring runs without a concurrency cap.

## How

### Loop type registry

```rust
// lazyjob-ralph/src/loop_types.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    // Fire-and-forget background loops
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrepGeneration,    // generates question set + company cheat sheet
    SalaryIntelligence,
    NetworkingOutreachDraft,

    // Interactive (bidirectional): MockInterviewLoop is the ONLY interactive loop.
    // It uses WorkerCommand::UserInput and WorkerEvent::AwaitingInput.
    MockInterviewLoop,
}

impl LoopType {
    /// Maximum concurrent instances of this loop type allowed system-wide.
    pub fn concurrency_limit(self) -> usize {
        match self {
            Self::MockInterviewLoop => 1,  // Interactive; only one at a time
            Self::JobDiscovery => 1,       // Expensive; avoid duplicate discovery runs
            _ => 3,                        // Most loops allow up to 3 parallel
        }
    }

    /// Relative scheduling priority. Higher number = higher priority.
    pub fn priority(self) -> u8 {
        match self {
            Self::MockInterviewLoop => 10,    // Interactive; user is waiting
            Self::ResumeTailoring => 9,       // User-triggered, blocking a apply action
            Self::CoverLetterGeneration => 9,
            Self::NetworkingOutreachDraft => 7,
            Self::InterviewPrepGeneration => 6,
            Self::SalaryIntelligence => 6,
            Self::CompanyResearch => 5,
            Self::JobDiscovery => 3,          // Background; runs on schedule
        }
    }

    pub fn is_interactive(self) -> bool {
        self == Self::MockInterviewLoop
    }

    pub fn cli_subcommand(self) -> &'static str {
        match self {
            Self::JobDiscovery => "job-discovery",
            Self::CompanyResearch => "company-research",
            Self::ResumeTailoring => "resume-tailor",
            Self::CoverLetterGeneration => "cover-letter",
            Self::InterviewPrepGeneration => "interview-prep",
            Self::SalaryIntelligence => "salary-intelligence",
            Self::NetworkingOutreachDraft => "networking-draft",
            Self::MockInterviewLoop => "mock-interview",
        }
    }
}
```

### Dispatch layer: translating user events to loop launches

```rust
// lazyjob-ralph/src/dispatch.rs

use crate::loop_types::LoopType;
use lazyjob_core::application::PostTransitionSuggestion;

pub struct LoopDispatch {
    manager: Arc<Mutex<RalphProcessManager>>,
    queue: Arc<Mutex<LoopQueue>>,
}

impl LoopDispatch {
    /// Called by TUI when a PostTransitionSuggestion is emitted after
    /// an application stage change (application-workflow-actions.md).
    pub async fn dispatch_suggestion(
        &self,
        suggestion: PostTransitionSuggestion,
        application_id: Uuid,
    ) -> Result<Option<Uuid>> {
        let (loop_type, params) = match suggestion {
            PostTransitionSuggestion::GenerateInterviewPrep => (
                LoopType::InterviewPrepGeneration,
                serde_json::json!({ "application_id": application_id }),
            ),
            PostTransitionSuggestion::RunSalaryComparison => (
                LoopType::SalaryIntelligence,
                serde_json::json!({ "application_id": application_id }),
            ),
            PostTransitionSuggestion::GenerateCompanyCheatSheet => (
                LoopType::CompanyResearch,
                serde_json::json!({ "application_id": application_id }),
            ),
            PostTransitionSuggestion::UpdateReferralOutcome => (
                LoopType::NetworkingOutreachDraft,
                serde_json::json!({ "application_id": application_id, "mode": "referral_outcome" }),
            ),
        };
        self.enqueue(loop_type, params).await
    }

    /// Enqueue a loop. If concurrency limit is not reached, spawns immediately.
    /// Otherwise, adds to the priority queue.
    pub async fn enqueue(
        &self,
        loop_type: LoopType,
        params: serde_json::Value,
    ) -> Result<Option<Uuid>> {
        let mut queue = self.queue.lock().await;
        let manager = self.manager.lock().await;

        let active_of_type = manager.count_active(loop_type);
        if active_of_type < loop_type.concurrency_limit() {
            drop(queue);
            drop(manager);
            let loop_id = self.manager.lock().await.spawn(loop_type, params).await?;
            Ok(Some(loop_id))
        } else {
            queue.push(QueuedLoop { loop_type, params, priority: loop_type.priority() });
            Ok(None)
        }
    }

    /// Called from `reap_dead_workers()` — when a slot opens, drain the queue.
    pub async fn drain_queue(&self) { ... }
}
```

### Scheduled (periodic) loops

`JobDiscovery` runs on a user-configured schedule, not on demand. The scheduler is a tokio task:

```rust
// lazyjob-ralph/src/scheduler.rs

pub struct LoopScheduler {
    dispatch: Arc<LoopDispatch>,
    config: SchedulerConfig,
}

pub struct SchedulerConfig {
    /// Cron expression for job discovery. Default: "0 8 * * *" (8am daily)
    pub job_discovery_cron: String,
    /// Maximum number of scheduled loops running concurrently
    pub max_scheduled_concurrent: usize,
}

impl LoopScheduler {
    pub async fn run(self) {
        let mut interval = build_cron_interval(&self.config.job_discovery_cron);
        loop {
            interval.tick().await;
            let params = serde_json::json!({
                "mode": "scheduled",
                "source": "cron"
            });
            let _ = self.dispatch.enqueue(LoopType::JobDiscovery, params).await;
        }
    }
}
```

`SchedulerConfig` keys live in `lazyjob.toml` under `[ralph.scheduler]`:

```toml
[ralph.scheduler]
job_discovery_cron = "0 8 * * *"
max_scheduled_concurrent = 2
```

### SQLite result writes: ralph writes, TUI reads

All ralph workers write their durable results directly to the shared SQLite DB using `sqlx`. Workers open the DB in WAL mode (`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`). The TUI never receives result data over the IPC pipe — it reads from SQLite after receiving `WorkerEvent::Done`. This means:

- Job discovery results → `jobs` + `job_embeddings` tables
- Resume tailoring → `resume_versions` table
- Cover letter → `cover_letter_versions` table
- Company research → `company_records` table (via `CompanyRepository`)
- Interview prep → `interview_prep_sessions` table
- Salary intelligence → `offer_market_data` table
- Networking draft → `outreach_drafts` table (associated with a `profile_contacts` row)
- Mock interview → `mock_interview_sessions` table

Workers use their own connection pool (1-4 connections, WAL mode allows concurrent reads). The TUI's connection pool handles contention with `busy_timeout=5000ms`.

### Concurrency cap and queue

A simple priority queue (BinaryHeap ordered by `priority * -1` for min-heap semantics) holds queued loops. The queue is bounded at 20 entries — if full, the `enqueue()` call returns an `Err(RalphError::QueueFull)` and the TUI shows a user-facing message.

### Loop run persistence in `ralph_loop_runs`

Every loop spawn writes a `ralph_loop_runs` row (defined in `agentic-ralph-subprocess-protocol.md`). The orchestration layer updates status transitions:

- `spawn()` → INSERT with `status='pending'`, then UPDATE to `status='running'` when `WorkerEvent::Ready` arrives
- `WorkerEvent::Done { success: true }` → UPDATE `status='done'`, set `finished_at`
- `WorkerEvent::Done { success: false }` or `WorkerEvent::Error` → UPDATE `status='failed'` or `status='cancelled'`

## Interface

```rust
// lazyjob-ralph/src/loop_types.rs
pub enum LoopType { JobDiscovery, CompanyResearch, ResumeTailoring, CoverLetterGeneration,
                    InterviewPrepGeneration, SalaryIntelligence, NetworkingOutreachDraft,
                    MockInterviewLoop }
impl LoopType {
    pub fn concurrency_limit(self) -> usize;
    pub fn priority(self) -> u8;
    pub fn is_interactive(self) -> bool;
    pub fn cli_subcommand(self) -> &'static str;
}

// lazyjob-ralph/src/dispatch.rs
pub struct LoopDispatch { ... }
impl LoopDispatch {
    pub async fn dispatch_suggestion(&self, suggestion: PostTransitionSuggestion, application_id: Uuid) -> Result<Option<Uuid>>;
    pub async fn enqueue(&self, loop_type: LoopType, params: serde_json::Value) -> Result<Option<Uuid>>;
    pub async fn drain_queue(&self);
}

// lazyjob-ralph/src/scheduler.rs
pub struct LoopScheduler { ... }
impl LoopScheduler { pub async fn run(self); }
```

## Open Questions

- Should users be able to configure per-loop-type concurrency limits in `lazyjob.toml`, or are they hardcoded? Hardcoded is safer for MVP.
- The scheduler uses a cron expression — should we also support interval-based scheduling (e.g., `every 6h`)? Both are valid UX models.
- Should queued (but not yet spawned) loops be visible in the TUI's Ralph panel, or only active loops?
- When `NetworkingOutreachDraft` is triggered by `PostTransitionSuggestion::UpdateReferralOutcome`, what params are needed? Does it need the `contact_id` or is `application_id` sufficient to look up the relevant contact?

## Implementation Tasks

- [ ] Define `LoopType` enum in `lazyjob-ralph/src/loop_types.rs` with `concurrency_limit()`, `priority()`, `is_interactive()`, `cli_subcommand()` methods
- [ ] Implement `LoopDispatch` in `lazyjob-ralph/src/dispatch.rs` with `enqueue()` (immediate spawn or queue), `dispatch_suggestion()` mapping `PostTransitionSuggestion` variants to `LoopType`+params, and `drain_queue()`
- [ ] Implement bounded priority queue (`BinaryHeap<QueuedLoop>`, cap 20, `priority` field) in `lazyjob-ralph/src/queue.rs`
- [ ] Implement `LoopScheduler` in `lazyjob-ralph/src/scheduler.rs` with cron expression parsing (use `cron` crate) and daily `JobDiscovery` dispatch
- [ ] Add `ralph_loop_runs` status update calls in `LoopDispatch` at each lifecycle event (pending → running → done/failed/cancelled)
- [ ] Write `[ralph.scheduler]` config section documentation in `lazyjob.toml` example file
- [ ] Integration test: spawn `JobDiscovery` to concurrency limit, verify 3rd enqueue goes to queue, verify drain after first completes
