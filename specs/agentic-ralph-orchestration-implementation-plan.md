# Implementation Plan: Ralph Loop Orchestration

## Status
Draft

## Related Spec
[specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md)

## Overview

The Ralph orchestration layer sits between the TUI/application events and the raw subprocess
management primitives defined in `agentic-ralph-subprocess-protocol.md`. Its job is to answer
three questions: *Which* loop type should run? *When* can it run (given concurrency limits)?
*How* does a user-facing event (stage transition, menu action) become a subprocess invocation?

Concretely, this means three collaborating types: `LoopType` (the taxonomy of all loop
categories with their concurrency limits and scheduling priority), `LoopDispatch` (the
central coordinator that either spawns a loop immediately or queues it in a bounded priority
queue), and `LoopScheduler` (the cron-driven background task that periodically triggers
`JobDiscovery` without user interaction). All three live in `lazyjob-ralph`.

The design follows the "dispatch then forget" model: the TUI hands a `LoopType` + opaque
JSON params to `LoopDispatch::enqueue()` and receives either `Some(loop_id)` (spawned
immediately) or `None` (queued). Progress events arrive on the existing `broadcast::Receiver<WorkerEvent>`
from `RalphProcessManager`. The orchestration layer is not in the event streaming path — it
only manages lifecycle transitions.

## Prerequisites

### Must be implemented first
- `specs/agentic-ralph-subprocess-protocol.md` — `RalphProcessManager`, `WorkerCommand`,
  `WorkerEvent`, and `ralph_loop_runs` DDL must exist before the orchestration layer can
  dispatch or queue loops.
- `specs/10-application-workflow.md` — `PostTransitionSuggestion` enum must be defined in
  `lazyjob-core::application` before `LoopDispatch::dispatch_suggestion()` can compile.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
cron          = "0.12"          # cron expression parsing and next-tick calculation
tokio         = { version = "1", features = ["macros", "rt-multi-thread", "time", "sync", "process"] }
uuid          = { version = "1", features = ["v4", "serde"] }
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
thiserror     = "1"
anyhow        = "1"
tracing       = "0.1"
```

In `lazyjob-ralph/Cargo.toml` (additions beyond the subprocess protocol plan):

```toml
[dependencies]
cron.workspace = true
```

---

## Architecture

### Crate Placement

All orchestration types live exclusively in `lazyjob-ralph`. The TUI (`lazyjob-tui`) and
`lazyjob-core` are consumers — they receive `Arc<LoopDispatch>` via dependency injection.
`lazyjob-core` defines the `PostTransitionSuggestion` type (domain event) and `lazyjob-ralph`
translates it to a `LoopType`; this keeps the domain layer free of subprocess concerns.

### Module Structure

```
lazyjob-ralph/
  src/
    lib.rs              # re-exports public surface
    error.rs            # RalphError (thiserror)
    protocol.rs         # WorkerCommand, WorkerEvent  (from subprocess spec)
    process.rs          # RalphProcessManager          (from subprocess spec)
    loop_types.rs       # LoopType enum + all methods
    queue.rs            # LoopQueue (BinaryHeap-backed priority queue)
    dispatch.rs         # LoopDispatch — central coordinator
    scheduler.rs        # LoopScheduler — cron-driven periodic dispatch
    config.rs           # OrchestratorConfig, SchedulerConfig (serde)
```

### Core Types

```rust
// lazyjob-ralph/src/loop_types.rs

use serde::{Deserialize, Serialize};

/// Every category of background or interactive AI work the app can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    // Background (fire-and-forget) loops
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrepGeneration,
    SalaryIntelligence,
    NetworkingOutreachDraft,
    // Interactive (bidirectional) loop — only one at a time
    MockInterviewLoop,
}

impl LoopType {
    /// Maximum number of concurrently active instances of this loop type.
    pub fn concurrency_limit(self) -> usize {
        match self {
            Self::MockInterviewLoop => 1,
            Self::JobDiscovery      => 1,
            _                       => 3,
        }
    }

    /// Scheduling priority (higher = higher priority). Used by BinaryHeap.
    pub fn priority(self) -> u8 {
        match self {
            Self::MockInterviewLoop         => 10,
            Self::ResumeTailoring           => 9,
            Self::CoverLetterGeneration     => 9,
            Self::NetworkingOutreachDraft   => 7,
            Self::InterviewPrepGeneration   => 6,
            Self::SalaryIntelligence        => 6,
            Self::CompanyResearch           => 5,
            Self::JobDiscovery              => 3,
        }
    }

    /// True only for MockInterviewLoop which requires bidirectional stdin/stdout.
    pub fn is_interactive(self) -> bool {
        self == Self::MockInterviewLoop
    }

    /// CLI subcommand string passed as the first positional arg to the ralph binary.
    pub fn cli_subcommand(self) -> &'static str {
        match self {
            Self::JobDiscovery              => "job-discovery",
            Self::CompanyResearch           => "company-research",
            Self::ResumeTailoring           => "resume-tailor",
            Self::CoverLetterGeneration     => "cover-letter",
            Self::InterviewPrepGeneration   => "interview-prep",
            Self::SalaryIntelligence        => "salary-intelligence",
            Self::NetworkingOutreachDraft   => "networking-draft",
            Self::MockInterviewLoop         => "mock-interview",
        }
    }

    /// Friendly display name for TUI panels.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::JobDiscovery              => "Job Discovery",
            Self::CompanyResearch           => "Company Research",
            Self::ResumeTailoring           => "Resume Tailoring",
            Self::CoverLetterGeneration     => "Cover Letter",
            Self::InterviewPrepGeneration   => "Interview Prep",
            Self::SalaryIntelligence        => "Salary Intelligence",
            Self::NetworkingOutreachDraft   => "Outreach Draft",
            Self::MockInterviewLoop         => "Mock Interview",
        }
    }
}

impl std::fmt::Display for LoopType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
```

### Priority Queue Type

```rust
// lazyjob-ralph/src/queue.rs

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use uuid::Uuid;
use serde_json::Value;
use crate::loop_types::LoopType;

/// An entry waiting in the dispatch queue.
#[derive(Debug, Clone)]
pub struct QueuedLoop {
    pub id:        Uuid,
    pub loop_type: LoopType,
    pub params:    Value,
    pub priority:  u8,
}

// BinaryHeap is a max-heap; we want highest priority at the top.
impl PartialEq for QueuedLoop {
    fn eq(&self, other: &Self) -> bool { self.priority == other.priority }
}
impl Eq for QueuedLoop {}
impl PartialOrd for QueuedLoop {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for QueuedLoop {
    fn cmp(&self, other: &Self) -> Ordering { self.priority.cmp(&other.priority) }
}

pub const MAX_QUEUE_CAPACITY: usize = 20;

pub struct LoopQueue {
    heap: BinaryHeap<QueuedLoop>,
}

impl LoopQueue {
    pub fn new() -> Self {
        Self { heap: BinaryHeap::with_capacity(MAX_QUEUE_CAPACITY) }
    }

    /// Returns false if the queue is at capacity.
    pub fn push(&mut self, entry: QueuedLoop) -> bool {
        if self.heap.len() >= MAX_QUEUE_CAPACITY {
            return false;
        }
        self.heap.push(entry);
        true
    }

    /// Pop the highest-priority queued loop.
    pub fn pop(&mut self) -> Option<QueuedLoop> {
        self.heap.pop()
    }

    pub fn len(&self) -> usize { self.heap.len() }
    pub fn is_empty(&self) -> bool { self.heap.is_empty() }

    /// Returns a snapshot for TUI display — sorted by priority descending.
    pub fn snapshot(&self) -> Vec<&QueuedLoop> {
        let mut items: Vec<&QueuedLoop> = self.heap.iter().collect();
        items.sort_by(|a, b| b.priority.cmp(&a.priority));
        items
    }
}
```

### Dispatch Layer

```rust
// lazyjob-ralph/src/dispatch.rs

use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
use serde_json::Value;
use tracing::{info, warn};

use lazyjob_core::application::PostTransitionSuggestion;
use crate::{
    error::RalphError,
    loop_types::LoopType,
    process::RalphProcessManager,
    queue::{LoopQueue, QueuedLoop},
};

pub struct LoopDispatch {
    manager: Arc<Mutex<RalphProcessManager>>,
    queue:   Arc<Mutex<LoopQueue>>,
}

impl LoopDispatch {
    pub fn new(manager: Arc<Mutex<RalphProcessManager>>) -> Self {
        Self {
            manager,
            queue: Arc::new(Mutex::new(LoopQueue::new())),
        }
    }

    /// Translate a `PostTransitionSuggestion` (emitted by the application-workflow
    /// domain after a stage change) into a loop dispatch.
    pub async fn dispatch_suggestion(
        &self,
        suggestion: PostTransitionSuggestion,
        application_id: Uuid,
    ) -> Result<Option<Uuid>, RalphError> {
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
                serde_json::json!({
                    "application_id": application_id,
                    "mode": "referral_outcome"
                }),
            ),
        };
        self.enqueue(loop_type, params).await
    }

    /// Core entry point: either spawns the loop immediately (if under the
    /// concurrency limit) or adds it to the bounded priority queue.
    ///
    /// Returns `Ok(Some(loop_id))` on immediate spawn, `Ok(None)` on queue,
    /// `Err(RalphError::QueueFull)` if the queue is at capacity.
    pub async fn enqueue(
        &self,
        loop_type: LoopType,
        params: Value,
    ) -> Result<Option<Uuid>, RalphError> {
        let manager = self.manager.lock().await;
        let active_count = manager.count_active(loop_type);

        if active_count < loop_type.concurrency_limit() {
            drop(manager);  // release before spawning (spawn re-acquires)
            let loop_id = self.manager.lock().await.spawn(loop_type, params).await?;
            info!(loop_type = %loop_type, %loop_id, "loop spawned immediately");
            Ok(Some(loop_id))
        } else {
            drop(manager);
            let entry = QueuedLoop {
                id: Uuid::new_v4(),
                loop_type,
                params,
                priority: loop_type.priority(),
            };
            let mut queue = self.queue.lock().await;
            let queued = queue.push(entry);
            if !queued {
                warn!(loop_type = %loop_type, "dispatch queue full; rejecting enqueue");
                return Err(RalphError::QueueFull { loop_type });
            }
            info!(loop_type = %loop_type, queue_depth = queue.len(), "loop queued");
            Ok(None)
        }
    }

    /// Attempt to drain queued loops into newly freed slots.
    /// Called by RalphProcessManager::reap_dead_workers() via a callback.
    pub async fn drain_queue(&self) {
        loop {
            let manager = self.manager.lock().await;
            // Peek at the head of the queue without holding both locks simultaneously
            let mut queue = self.queue.lock().await;
            let Some(next) = queue.pop() else { break };

            let active = manager.count_active(next.loop_type);
            if active >= next.loop_type.concurrency_limit() {
                // Put it back — concurrency slot still full for this type
                queue.push(next);
                break;
            }
            drop(queue);
            drop(manager);

            match self.manager.lock().await.spawn(next.loop_type, next.params).await {
                Ok(loop_id) => {
                    info!(loop_type = %next.loop_type, %loop_id, "queued loop spawned");
                }
                Err(e) => {
                    warn!(error = %e, "failed to spawn queued loop");
                }
            }
        }
    }

    /// Returns a snapshot of currently queued (not yet spawned) loops for TUI display.
    pub async fn queued_snapshot(&self) -> Vec<(Uuid, LoopType)> {
        let queue = self.queue.lock().await;
        queue.snapshot()
            .iter()
            .map(|q| (q.id, q.loop_type))
            .collect()
    }
}
```

### Scheduler

```rust
// lazyjob-ralph/src/scheduler.rs

use std::str::FromStr;
use std::sync::Arc;
use tokio::time::sleep_until;
use tracing::{error, info};

use crate::{dispatch::LoopDispatch, error::RalphError, loop_types::LoopType};

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Cron expression for job discovery. Default: "0 8 * * *" (8am daily)
    pub job_discovery_cron: String,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { job_discovery_cron: "0 8 * * *".to_string() }
    }
}

pub struct LoopScheduler {
    dispatch: Arc<LoopDispatch>,
    config:   SchedulerConfig,
}

impl LoopScheduler {
    pub fn new(dispatch: Arc<LoopDispatch>, config: SchedulerConfig) -> Self {
        Self { dispatch, config }
    }

    /// Runs forever. Spawns `JobDiscovery` on the configured cron schedule.
    /// Cancelled when the tokio runtime shuts down (via select! or task abort).
    pub async fn run(self) {
        let schedule = match cron::Schedule::from_str(&self.config.job_discovery_cron) {
            Ok(s)  => s,
            Err(e) => {
                error!(error = %e, cron = %self.config.job_discovery_cron,
                       "invalid job_discovery_cron expression; scheduler disabled");
                return;
            }
        };

        info!(cron = %self.config.job_discovery_cron, "job discovery scheduler started");

        for next_tick in schedule.upcoming(chrono::Utc) {
            let now = chrono::Utc::now();
            let duration = (next_tick - now).to_std().unwrap_or_default();
            sleep_until(tokio::time::Instant::now() + duration).await;

            info!("scheduled job discovery firing");
            let params = serde_json::json!({
                "mode":   "scheduled",
                "source": "cron"
            });
            match self.dispatch.enqueue(LoopType::JobDiscovery, params).await {
                Ok(Some(id)) => info!(%id, "scheduled job discovery started"),
                Ok(None)     => info!("job discovery already at concurrency limit; queued"),
                Err(RalphError::QueueFull { .. }) => {
                    error!("dispatch queue full; skipping scheduled job discovery");
                }
                Err(e) => error!(error = %e, "failed to schedule job discovery"),
            }
        }
    }
}
```

### Configuration Type

```rust
// lazyjob-ralph/src/config.rs

use serde::{Deserialize, Serialize};
use crate::scheduler::SchedulerConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestratorConfig {
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    /// Maximum number of loops that can be waiting in the dispatch queue.
    /// Defaults to 20 (MAX_QUEUE_CAPACITY).
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
            queue_capacity: 20,
        }
    }
}

fn default_queue_capacity() -> usize { 20 }
```

Corresponding section in `lazyjob.toml`:

```toml
[ralph.scheduler]
job_discovery_cron = "0 8 * * *"

[ralph.orchestration]
queue_capacity = 20
```

### SQLite Schema

The `ralph_loop_runs` table is defined in the subprocess spec. The orchestration layer only
adds writes at additional lifecycle points. No new DDL is needed.

Lifecycle state transitions from the orchestration layer's perspective:

| Event                          | `status` column value |
|--------------------------------|-----------------------|
| `LoopDispatch::enqueue()` queued (no slot) | `pending` (row NOT yet written) |
| `RalphProcessManager::spawn()` called | INSERT with `status='pending'` |
| `WorkerEvent::Ready` received  | UPDATE `status='running'` |
| `WorkerEvent::Done { success: true }` | UPDATE `status='done'`, set `finished_at` |
| `WorkerEvent::Done { success: false }` | UPDATE `status='cancelled'` |
| `WorkerEvent::Error`           | UPDATE `status='failed'`, set `error_code`, `error_msg` |

Queued (not-yet-spawned) entries live only in the in-memory `LoopQueue`. They are not
persisted because they haven't started yet — if the TUI restarts, the queue is empty.

### Trait Definitions

`RalphProcessManager::count_active()` must be added to the process manager API to support
concurrency checks:

```rust
// Additions to lazyjob-ralph/src/process.rs

impl RalphProcessManager {
    /// Count currently active (spawned, not yet reaped) workers of a given loop type.
    pub fn count_active(&self, loop_type: LoopType) -> usize {
        self.active
            .values()
            .filter(|w| w.loop_type == loop_type)
            .count()
    }

    /// List all active workers — (loop_id, loop_type, started_at) — for TUI display.
    pub fn active_snapshot(&self) -> Vec<(Uuid, LoopType)> {
        self.active
            .iter()
            .map(|(id, w)| (*id, w.loop_type))
            .collect()
    }
}
```

---

## Implementation Phases

### Phase 1 — Core Dispatch (MVP)

**Goal:** `LoopType` enum is defined, `LoopQueue` works, `LoopDispatch::enqueue()` correctly
spawns or queues. No cron scheduler yet.

#### Step 1.1 — `LoopType` enum

**File:** `lazyjob-ralph/src/loop_types.rs`

Implement the full enum and all four methods:
- `concurrency_limit(self) -> usize`
- `priority(self) -> u8`
- `is_interactive(self) -> bool`
- `cli_subcommand(self) -> &'static str`
- `display_name(self) -> &'static str`
- `impl Display` using `display_name()`

**Verification:** `cargo test -p lazyjob-ralph loop_types` passes. `LoopType::JobDiscovery.cli_subcommand()` returns `"job-discovery"`.

#### Step 1.2 — `LoopQueue`

**File:** `lazyjob-ralph/src/queue.rs`

Implement `QueuedLoop` with `Ord` (max-heap by `priority`), `LoopQueue` wrapping
`BinaryHeap<QueuedLoop>` with `push()`/`pop()`/`snapshot()`/`len()`/`is_empty()`.

`push()` returns `false` if `heap.len() >= MAX_QUEUE_CAPACITY`. No panics.

**Crate API:**
- `std::collections::BinaryHeap::push`, `pop`, `iter`
- `std::cmp::Ordering`, `PartialOrd`, `Ord` manual implementations

**Verification:** Unit test pushes 20 identical entries, 21st returns `false`. `pop()` always returns the highest-priority entry.

#### Step 1.3 — `count_active()` on `RalphProcessManager`

**File:** `lazyjob-ralph/src/process.rs` (addition)

Add `count_active(loop_type: LoopType) -> usize` and `active_snapshot()` to the existing
`RalphProcessManager`. These are read-only lookups over `self.active: HashMap<Uuid, ActiveWorker>`.

**Verification:** Unit test with a mocked process map returns correct counts.

#### Step 1.4 — `LoopDispatch`

**File:** `lazyjob-ralph/src/dispatch.rs`

Implement `LoopDispatch::new()`, `enqueue()`, `drain_queue()`, `queued_snapshot()`.

Key correctness invariant: the check of `count_active()` and the subsequent `spawn()` must
be guarded. Since both take `Mutex<RalphProcessManager>`, the lock is released between the
check and the spawn. This is a benign TOCTOU — worst case we spawn one extra loop that
temporarily exceeds the limit by 1 before the next `reap_dead_workers()` call. Document this
in a code comment. Avoid holding both `manager` and `queue` locks simultaneously.

**Verification:**
```rust
#[tokio::test]
async fn test_enqueue_respects_concurrency_limit() {
    // Spawn a mock manager where count_active always returns limit
    // enqueue() should queue instead of spawn
    // drain_queue() with available slot should spawn the queued entry
}
```

#### Step 1.5 — `dispatch_suggestion()`

**File:** `lazyjob-ralph/src/dispatch.rs`

Implement the `dispatch_suggestion()` method with exhaustive match on all four
`PostTransitionSuggestion` variants. Compile error if a new variant is added to
`PostTransitionSuggestion` without updating this match.

**Verification:** Each variant maps to the expected `LoopType` in unit tests. No runtime match
is needed — use a `match` with no wildcard arm.

#### Step 1.6 — `RalphError` extension

**File:** `lazyjob-ralph/src/error.rs`

Add `QueueFull { loop_type: LoopType }` variant:

```rust
#[derive(thiserror::Error, Debug)]
pub enum RalphError {
    // ... existing variants from subprocess spec ...
    #[error("dispatch queue is full; cannot enqueue {loop_type}")]
    QueueFull { loop_type: LoopType },
}
```

**Verification:** `RalphError::QueueFull { loop_type: LoopType::JobDiscovery }` formats correctly via `Display`.

---

### Phase 2 — Cron Scheduler

**Goal:** `LoopScheduler` runs as a background tokio task and triggers `JobDiscovery` on
a configurable cron schedule.

#### Step 2.1 — Add `cron` crate

```toml
# workspace Cargo.toml
cron = "0.12"

# lazyjob-ralph/Cargo.toml
cron.workspace = true
chrono = { workspace = true, features = ["clock"] }
```

#### Step 2.2 — `SchedulerConfig` + `OrchestratorConfig`

**File:** `lazyjob-ralph/src/config.rs`

Implement both structs with `#[derive(Debug, Clone, Deserialize, Serialize)]` and
`Default`. `SchedulerConfig::default()` uses `"0 8 * * *"` (8am UTC daily).

**Verification:** `serde_json::from_str::<SchedulerConfig>(r#"{"job_discovery_cron":"0 */6 * * *"}"#)` deserializes correctly.

#### Step 2.3 — `LoopScheduler::run()`

**File:** `lazyjob-ralph/src/scheduler.rs`

Use `cron::Schedule::from_str(&config.job_discovery_cron)` to parse. Iterate
`schedule.upcoming(chrono::Utc)` for future ticks. Compute duration to next tick
with `(next_tick - chrono::Utc::now()).to_std()`. Use `tokio::time::sleep` for the wait.

Handle parse errors from `cron::Schedule::from_str` by logging and returning early —
the scheduler is optional, not critical path.

**Crate APIs:**
- `cron::Schedule::from_str(&str) -> Result<Schedule, cron::error::Error>`
- `cron::Schedule::upcoming(tz) -> impl Iterator<Item = DateTime<Tz>>`
- `chrono::Utc::now() -> DateTime<Utc>`
- `(chrono::Duration).to_std() -> Result<std::time::Duration>`
- `tokio::time::sleep(std::time::Duration)`

**Verification:**
```rust
#[tokio::test]
async fn scheduler_dispatches_on_schedule() {
    // Use a cron that fires every second: "* * * * * *"
    // With tokio::time::pause() + advance(), confirm enqueue() is called
}
```

#### Step 2.4 — Wire into `AppState`

**File:** `lazyjob-tui/src/app/state.rs` (or `lazyjob-cli/src/main.rs`)

On startup:
```rust
let dispatch = Arc::new(LoopDispatch::new(Arc::clone(&process_manager)));
let scheduler = LoopScheduler::new(Arc::clone(&dispatch), config.ralph.scheduler.clone());
tokio::spawn(scheduler.run());
```

The scheduler task runs indefinitely until the runtime shuts down. No explicit join handle
is needed — tokio drops background tasks on runtime shutdown.

**Verification:** App starts; `tracing::info!` log line `"job discovery scheduler started"` appears.

---

### Phase 3 — TUI Integration (Ralph Panel)

**Goal:** The TUI's Ralph panel shows both active and queued loops, updated in real-time.

#### Step 3.1 — `RalphPanelState`

**File:** `lazyjob-tui/src/panels/ralph.rs`

```rust
pub struct RalphPanelState {
    /// Active workers from RalphProcessManager::active_snapshot()
    pub active: Vec<(Uuid, LoopType, String)>,  // (id, type, current_phase)
    /// Queued (waiting) entries from LoopDispatch::queued_snapshot()
    pub queued: Vec<(Uuid, LoopType)>,
    pub selected_index: usize,
}
```

The panel is refreshed on every `WorkerEvent` (which arrives via `broadcast::Receiver`)
and on a 1-second periodic tick. Refreshing on every event is sufficient for active loops;
queued loops change only when `enqueue()` or `drain_queue()` is called.

#### Step 3.2 — Panel rendering

The Ralph panel renders two sections in a vertical `Layout::default().constraints(...)`:
1. **Active** — each active worker as a `ListItem` with loop type name + current phase
   from the last `WorkerEvent::Status`. A spinner Unicode character cycles on each render tick.
2. **Queued** — each queued entry as a `ListItem` with loop type name + `(queued #N)` label.

```
╭─ Ralph Loops ────────────────────────╮
│ ⣿ Resume Tailoring   — analyzing JD │  ← active
│ ⣿ Job Discovery      — 42 found     │  ← active
│   Cover Letter       (queued #1)     │  ← queued
╰──────────────────────────────────────╯
```

**Crate APIs:**
- `ratatui::widgets::{List, ListItem, ListState}`
- `ratatui::style::{Color, Modifier, Style}`
- `ratatui::layout::{Constraint, Direction, Layout}`

#### Step 3.3 — Cancel action

When the user presses `x` on a selected active loop, the TUI calls
`RalphProcessManager::cancel(loop_id)`. The cancellation protocol (3-second timeout, SIGKILL
escalation) is handled inside `process.rs` per the subprocess spec. The orchestration layer's
`drain_queue()` is called by the TUI's event loop after receiving `WorkerEvent::Done` from
a cancelled worker.

```rust
// In the TUI's action dispatcher
Action::CancelSelectedLoop => {
    if let Some((loop_id, _)) = panel.selected_active() {
        process_manager.lock().await.cancel(loop_id).await?;
    }
}
```

#### Step 3.4 — Queue-full user notification

When `LoopDispatch::enqueue()` returns `Err(RalphError::QueueFull)`, the TUI shows a
one-line error in its status bar:

```rust
app.status_bar.set_error("Ralph queue is full. Cancel a running loop to free space.");
```

The error auto-clears after 5 seconds.

---

### Phase 4 — Resource Budgets and Token Limits

**Goal:** Add per-loop-type soft token budget so runaway loops don't consume unlimited API spend.

#### Step 4.1 — Token budget in `LoopType`

```rust
impl LoopType {
    /// Soft token budget per run in thousands of tokens. None = unlimited.
    pub fn token_budget_k(self) -> Option<u32> {
        match self {
            Self::JobDiscovery              => Some(50),
            Self::CompanyResearch           => Some(30),
            Self::ResumeTailoring           => Some(40),
            Self::CoverLetterGeneration     => Some(20),
            Self::InterviewPrepGeneration   => Some(60),
            Self::SalaryIntelligence        => Some(25),
            Self::NetworkingOutreachDraft   => Some(15),
            Self::MockInterviewLoop         => None,   // Unbounded (user controls session length)
        }
    }
}
```

Ralph workers receive this budget in their `Start` params JSON:

```rust
// In RalphProcessManager::spawn() — adds budget to params
let mut params = params;
if let Some(budget_k) = loop_type.token_budget_k() {
    params["_token_budget_k"] = serde_json::json!(budget_k);
}
```

Workers check `params["_token_budget_k"]` and emit `WorkerEvent::Error { code: "budget_exceeded" }` if cumulative usage crosses the limit. The orchestration layer treats this as a normal error termination.

#### Step 4.2 — Concurrency-limit config override

For future extensibility, read per-type overrides from `lazyjob.toml` if present:

```toml
[ralph.concurrency]
resume_tailoring = 2
job_discovery = 1
```

`LoopType::concurrency_limit()` checks a `once_cell::sync::OnceCell<HashMap<LoopType, usize>>`
initialized from config at startup. Falls back to the hardcoded default if not configured.

```rust
use once_cell::sync::OnceCell;
static CONCURRENCY_OVERRIDES: OnceCell<HashMap<LoopType, usize>> = OnceCell::new();

impl LoopType {
    pub fn concurrency_limit(self) -> usize {
        CONCURRENCY_OVERRIDES
            .get()
            .and_then(|m| m.get(&self).copied())
            .unwrap_or_else(|| self.default_concurrency_limit())
    }

    fn default_concurrency_limit(self) -> usize {
        match self {
            Self::MockInterviewLoop => 1,
            Self::JobDiscovery      => 1,
            _                       => 3,
        }
    }
}

pub fn init_concurrency_overrides(overrides: HashMap<LoopType, usize>) {
    let _ = CONCURRENCY_OVERRIDES.set(overrides);
}
```

---

## Key Crate APIs

| Crate | API | Usage |
|-------|-----|-------|
| `cron` | `Schedule::from_str(&str)` | Parse cron expression |
| `cron` | `schedule.upcoming(chrono::Utc)` → `Iterator<Item = DateTime<Utc>>` | Get next fire times |
| `chrono` | `Utc::now()`, `Duration::to_std()` | Convert cron time to sleep duration |
| `tokio::time` | `sleep(Duration)` | Wait until next cron tick |
| `tokio::sync` | `Mutex<T>`, `broadcast::channel()` | Shared state, event fan-out |
| `std::collections` | `BinaryHeap<QueuedLoop>` | Priority queue backing store |
| `uuid` | `Uuid::new_v4()` | Loop instance IDs |
| `serde_json` | `json!({ ... })`, `Value` | Opaque params for workers |
| `once_cell` | `OnceCell<HashMap<LoopType, usize>>` | Concurrency overrides (Phase 4) |
| `ratatui` | `List`, `ListItem`, `Layout` | Ralph panel rendering |

---

## Error Handling

```rust
// lazyjob-ralph/src/error.rs

#[derive(thiserror::Error, Debug)]
pub enum RalphError {
    #[error("subprocess error: {0}")]
    Subprocess(#[from] std::io::Error),

    #[error("JSON codec error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("dispatch queue is full; cannot enqueue {loop_type}")]
    QueueFull { loop_type: LoopType },

    #[error("loop {loop_id} not found in active workers")]
    LoopNotFound { loop_id: uuid::Uuid },

    #[error("operation requires interactive loop; {loop_type} is not interactive")]
    NotInteractive { loop_type: LoopType },

    #[error("invalid cron expression '{expr}': {source}")]
    InvalidCron {
        expr:   String,
        source: cron::error::Error,
    },

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, RalphError>;
```

TUI callers convert `RalphError::QueueFull` to a user-facing status bar message. All other
variants are logged at `tracing::error!` level and not propagated to the user.

---

## Testing Strategy

### Unit Tests

**`queue.rs`**
```rust
#[test]
fn queue_respects_capacity() {
    let mut q = LoopQueue::new();
    for _ in 0..MAX_QUEUE_CAPACITY {
        assert!(q.push(make_entry(LoopType::JobDiscovery, 3)));
    }
    assert!(!q.push(make_entry(LoopType::ResumeTailoring, 9)));
}

#[test]
fn queue_pops_highest_priority_first() {
    let mut q = LoopQueue::new();
    q.push(make_entry(LoopType::JobDiscovery, 3));       // priority 3
    q.push(make_entry(LoopType::ResumeTailoring, 9));    // priority 9
    q.push(make_entry(LoopType::CompanyResearch, 5));    // priority 5
    assert_eq!(q.pop().unwrap().loop_type, LoopType::ResumeTailoring);
    assert_eq!(q.pop().unwrap().loop_type, LoopType::CompanyResearch);
    assert_eq!(q.pop().unwrap().loop_type, LoopType::JobDiscovery);
}
```

**`loop_types.rs`**
```rust
#[test]
fn all_loop_types_have_unique_cli_subcommands() {
    use std::collections::HashSet;
    let types = [LoopType::JobDiscovery, LoopType::CompanyResearch,
                 LoopType::ResumeTailoring, LoopType::CoverLetterGeneration,
                 LoopType::InterviewPrepGeneration, LoopType::SalaryIntelligence,
                 LoopType::NetworkingOutreachDraft, LoopType::MockInterviewLoop];
    let cmds: HashSet<_> = types.iter().map(|t| t.cli_subcommand()).collect();
    assert_eq!(cmds.len(), types.len());
}

#[test]
fn mock_interview_is_the_only_interactive_type() {
    use strum::IntoEnumIterator;
    let interactive: Vec<_> = LoopType::iter().filter(|t| t.is_interactive()).collect();
    assert_eq!(interactive, vec![LoopType::MockInterviewLoop]);
}
```

**`dispatch.rs`** (uses `mockall` to mock `RalphProcessManager`)
```rust
#[tokio::test]
async fn enqueue_spawns_immediately_under_limit() {
    let mut mock_mgr = MockRalphProcessManager::new();
    mock_mgr.expect_count_active().return_const(0usize);
    mock_mgr.expect_spawn().returning(|_, _| Ok(Uuid::new_v4()));
    let dispatch = LoopDispatch::new(Arc::new(Mutex::new(mock_mgr)));
    let result = dispatch.enqueue(LoopType::ResumeTailoring, serde_json::json!({})).await;
    assert!(result.unwrap().is_some());
}

#[tokio::test]
async fn enqueue_queues_when_at_concurrency_limit() {
    let mut mock_mgr = MockRalphProcessManager::new();
    // concurrency_limit for ResumeTailoring = 3; return 3 active
    mock_mgr.expect_count_active().return_const(3usize);
    let dispatch = LoopDispatch::new(Arc::new(Mutex::new(mock_mgr)));
    let result = dispatch.enqueue(LoopType::ResumeTailoring, serde_json::json!({})).await;
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn enqueue_returns_err_when_queue_full() {
    let mut mock_mgr = MockRalphProcessManager::new();
    mock_mgr.expect_count_active().return_const(3usize);
    let dispatch = LoopDispatch::new(Arc::new(Mutex::new(mock_mgr)));
    // Fill the queue
    for _ in 0..20 {
        dispatch.enqueue(LoopType::ResumeTailoring, serde_json::json!({})).await.unwrap();
    }
    // 21st should fail
    let result = dispatch.enqueue(LoopType::ResumeTailoring, serde_json::json!({})).await;
    assert!(matches!(result, Err(RalphError::QueueFull { .. })));
}
```

**`scheduler.rs`**
```rust
#[tokio::test]
async fn scheduler_fires_on_cron() {
    tokio::time::pause();  // Control time
    let (fired_tx, mut fired_rx) = tokio::sync::mpsc::channel(1);

    // Build a mock dispatch that signals when enqueue is called
    let mock_dispatch = MockLoopDispatch::new_with_fn(move |lt, _| {
        assert_eq!(lt, LoopType::JobDiscovery);
        let _ = fired_tx.try_send(());
        Ok(Some(Uuid::new_v4()))
    });

    let config = SchedulerConfig { job_discovery_cron: "0 * * * *".to_string() }; // hourly
    let scheduler = LoopScheduler::new(Arc::new(mock_dispatch), config);
    tokio::spawn(scheduler.run());

    // Advance time past the first firing
    tokio::time::advance(tokio::time::Duration::from_secs(3601)).await;
    tokio::time::resume();
    assert!(fired_rx.try_recv().is_ok());
}
```

### Integration Tests

**`tests/orchestration_integration.rs`**

Spawn a real (echo-only) ralph binary that immediately emits `WorkerEvent::Done { success: true }`.
Verify:
1. `enqueue()` up to the concurrency limit spawns immediately each time.
2. The `(limit + 1)`th enqueue returns `None` (queued).
3. After `WorkerEvent::Done` triggers `drain_queue()`, the queued loop is spawned.
4. All three runs appear in `ralph_loop_runs` with correct status transitions.

```rust
// tests/orchestration_integration.rs

#[tokio::test]
async fn queued_loop_drains_after_slot_opens() {
    let db = setup_test_db().await;
    let mgr = RalphProcessManager::new(echo_ralph_bin(), db.path(), PathBuf::new());
    let dispatch = Arc::new(LoopDispatch::new(Arc::new(Mutex::new(mgr))));

    // JobDiscovery limit = 1; spawn first immediately
    let id1 = dispatch.enqueue(LoopType::JobDiscovery, json!({})).await.unwrap();
    assert!(id1.is_some());

    // Second goes to queue
    let id2 = dispatch.enqueue(LoopType::JobDiscovery, json!({})).await.unwrap();
    assert!(id2.is_none());

    // Wait for the first to complete (echo binary exits immediately)
    tokio::time::sleep(Duration::from_millis(200)).await;
    dispatch.drain_queue().await;

    // Verify the queued entry was spawned and appears in ralph_loop_runs
    let runs: Vec<String> = sqlx::query_scalar("SELECT status FROM ralph_loop_runs")
        .fetch_all(&db.pool())
        .await.unwrap();
    assert!(runs.iter().any(|s| s == "done"));
}
```

---

## Open Questions

1. **Concurrency limit config:** Should users be able to override per-type concurrency limits
   in `lazyjob.toml`? Phase 4 sketches this with `OnceCell`, but MVP can hardcode them.
   Deferred to post-MVP based on user feedback.

2. **Queue visibility in TUI:** Should queued (not yet spawned) loops appear in the Ralph
   panel? Phase 3 includes this, but it adds complexity to the panel state. If users are
   confused by "ghost" entries, we may opt to only show active loops.

3. **Scheduler interval vs. cron:** The spec uses a cron expression. An alternative is a
   simple `interval` in the config (e.g., `job_discovery_interval_hours = 8`). Cron is more
   powerful but harder to explain in docs. Consider offering both.

4. **`UpdateReferralOutcome` params:** `dispatch_suggestion()` passes `application_id` for
   the `NetworkingOutreachDraft` loop. The loop may also need `contact_id`. Whether the loop
   can derive `contact_id` from `application_id` via a DB lookup (in the worker) or needs it
   passed explicitly needs clarification before implementing the networking spec.

5. **Drain-on-complete coupling:** `drain_queue()` must be called after every worker
   completes. Currently the design assumes the TUI's event loop calls it after receiving
   `WorkerEvent::Done`. An alternative is to register a callback with `RalphProcessManager`
   so the process manager calls `drain_queue()` automatically. The callback approach removes
   the coupling but adds complexity. Decide before Phase 1 is complete.

6. **Token budget enforcement:** Phase 4 adds `_token_budget_k` to params. Workers must
   actually check this value and emit `Error` when exceeded. The orchestration plan sketches
   this but workers are owned by the subprocess spec. Ensure cross-spec ownership is clear.

---

## Related Specs

- [agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) — defines
  `RalphProcessManager`, `WorkerEvent`, `WorkerCommand`, and `ralph_loop_runs` DDL.
- [10-application-workflow.md](./10-application-workflow.md) — defines
  `PostTransitionSuggestion` which `dispatch_suggestion()` maps to loop types.
- [09-tui-design-keybindings.md](./09-tui-design-keybindings.md) — the TUI event loop that
  calls `drain_queue()` and renders the Ralph panel.
- [agentic-prompt-templates.md](./agentic-prompt-templates.md) — each `LoopType` has a
  corresponding prompt template; the `cli_subcommand()` maps to a template key.
- [XX-llm-cost-budget-management.md](./XX-llm-cost-budget-management.md) — token budget
  per loop type (Phase 4) feeds into the global cost tracking system.
