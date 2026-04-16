# Spec: Ralph Subprocess IPC Protocol

**JTBD**: Let AI handle tedious job search work autonomously while I focus on high-signal decisions
**Topic**: How the TUI spawns, communicates with, and recovers from ralph subprocess workers
**Domain**: agentic

---

## What

This spec defines the wire protocol and lifecycle management between `lazyjob-tui` and ralph subprocess workers. Ralph workers are short-lived child processes spawned by the TUI via `tokio::process::Command`. The TUI and worker communicate over newline-delimited JSON on stdin/stdout. The worker writes durable results directly to the shared SQLite database; the TUI receives only status events and final notifications through the IPC channel. One special case — `MockInterviewLoop` — is an interactive loop that requires a bidirectional input/output channel while the interview is in progress.

## Why

All AI work runs in ralph workers, not in the TUI process. This separation gives users a responsive interface while background loops run for minutes or hours. If a loop crashes, the TUI continues running. If the TUI is restarted mid-loop, the worker writes its results to SQLite and the TUI recovers state from there. The stdio JSON protocol is the simplest reliable IPC for single-user local tools — no socket files to manage, no port conflicts, and each worker is independently testable as a pure CLI binary.

## How

### Transport layer: newline-delimited JSON over stdio

Every message is a single JSON object followed by `\n`. The worker reads commands from stdin; the TUI reads events from stdout. Stderr is reserved for unstructured debug logs only (redirected to `~/.lazyjob/logs/ralph-<loop_id>.log`).

### TUI → Worker: command messages

```rust
// lazyjob-ralph/src/protocol.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerCommand {
    /// Sent immediately after spawn to communicate parameters
    Start {
        loop_id: Uuid,
        params: serde_json::Value,
    },
    /// Request graceful cancellation; worker drains current unit of work then exits
    Cancel,
    /// Interactive mode only: deliver the user's typed response to a waiting mock loop
    UserInput {
        text: String,
    },
}
```

### Worker → TUI: event messages

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// First message emitted; tells TUI which loop_id is live
    Ready { loop_id: Uuid, loop_type: LoopType },
    /// Progress update; progress is 0.0–1.0
    Status { loop_id: Uuid, phase: String, progress: f32, message: String },
    /// Interactive mode only: worker is paused waiting for UserInput
    AwaitingInput { loop_id: Uuid, prompt: String },
    /// Informational result chunk written to SQLite; data is a summary only
    ResultChunk { loop_id: Uuid, summary: String },
    /// Terminal success; worker will exit after this
    Done { loop_id: Uuid, success: bool },
    /// Terminal failure; worker will exit after this
    Error { loop_id: Uuid, code: String, message: String },
}
```

### Interactive mode for MockInterviewLoop

All other loop types emit events in one direction (worker → TUI) and never block on user input. `MockInterviewLoop` is the only exception: after emitting `AwaitingInput`, the worker blocks on `tokio::io::stdin().lines().next_line()` waiting for a `WorkerCommand::UserInput` response. The TUI must pipe the user's typed text back through stdin. This is modeled as a state machine in the worker:

```
Idle → Ready → [Status* → AwaitingInput → (blocked on stdin) → Status*]* → Done
```

The TUI's `RalphProcessManager` must never time out an `AwaitingInput` worker — only the user's inactivity timeout (configurable, default 10 minutes) should cancel the loop.

### Process management: `RalphProcessManager`

Lives in `lazyjob-ralph/src/process.rs`. Owned by the TUI's `AppState`. Holds one `broadcast::Sender<WorkerEvent>` that the TUI's event loop subscribes to.

```rust
// lazyjob-ralph/src/process.rs

pub struct RalphProcessManager {
    ralph_bin: PathBuf,          // path to ralph binary
    db_path: PathBuf,
    life_sheet_path: PathBuf,
    active: HashMap<Uuid, ActiveWorker>,
    event_tx: broadcast::Sender<WorkerEvent>,
}

struct ActiveWorker {
    loop_type: LoopType,
    process: Child,
    stdin_tx: mpsc::Sender<WorkerCommand>,   // sends to stdin writer task
    interactive: bool,
}

impl RalphProcessManager {
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent> { ... }

    pub async fn spawn(
        &mut self,
        loop_type: LoopType,
        params: serde_json::Value,
    ) -> Result<Uuid, RalphError> { ... }

    /// Only valid for interactive loops (MockInterviewLoop).
    pub async fn send_user_input(&mut self, loop_id: Uuid, text: String)
        -> Result<(), RalphError> { ... }

    pub async fn cancel(&mut self, loop_id: Uuid) -> Result<(), RalphError> { ... }

    /// Called from a periodic health-check task (every 5s).
    pub fn reap_dead_workers(&mut self) { ... }

    /// On TUI startup: find rows in ralph_loop_runs with status='running'
    /// and offer user "resume or cancel" for each.
    pub async fn recover_pending(&mut self, pool: &SqlitePool)
        -> Result<Vec<PendingLoop>, RalphError> { ... }
}
```

### Crash recovery via `ralph_loop_runs` table

```sql
CREATE TABLE IF NOT EXISTS ralph_loop_runs (
    id           TEXT PRIMARY KEY,
    loop_type    TEXT NOT NULL,
    params_json  TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',  -- pending|running|done|failed|cancelled
    started_at   TEXT,
    finished_at  TEXT,
    error_code   TEXT,
    error_msg    TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

On TUI start, `recover_pending()` queries for `status='running'`. If the TUI finds such rows, it means the TUI crashed mid-loop. It emits `WorkerEvent::Error { code: "tui_restart", ... }` on the broadcast channel (so UI panels can reflect the lost run) and marks rows `status='failed'`. The user may choose to re-run the loop manually.

### Cancellation protocol

1. `RalphProcessManager::cancel()` sends `WorkerCommand::Cancel` over stdin.
2. Worker receives it, completes the current atomic unit of work (e.g., finishing one LLM call), writes a partial result to SQLite, emits `WorkerEvent::Done { success: false }`, and exits.
3. If worker doesn't exit within 3 seconds of cancel, `RalphProcessManager` calls `child.kill()`.
4. `ralph_loop_runs` row is updated to `status='cancelled'`.

### Stderr → log file redirection

Workers' stderr is redirected to `~/.lazyjob/logs/ralph-<loop_id>.log` by the process manager. Logs are retained for 7 days. No stderr output ever reaches the TUI event loop — stderr is a debugging artifact only.

## Interface

```rust
// lazyjob-ralph/src/protocol.rs (key public types)
pub enum WorkerCommand { Start { loop_id, params }, Cancel, UserInput { text } }
pub enum WorkerEvent { Ready, Status, AwaitingInput, ResultChunk, Done, Error }

// lazyjob-ralph/src/process.rs
pub struct RalphProcessManager { ... }
impl RalphProcessManager {
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent>;
    pub async fn spawn(&mut self, loop_type: LoopType, params: serde_json::Value) -> Result<Uuid>;
    pub async fn send_user_input(&mut self, loop_id: Uuid, text: String) -> Result<()>;
    pub async fn cancel(&mut self, loop_id: Uuid) -> Result<()>;
    pub fn reap_dead_workers(&mut self);
    pub async fn recover_pending(&mut self, pool: &SqlitePool) -> Result<Vec<PendingLoop>>;
}

// Crash-recovery DDL
// ralph_loop_runs: id, loop_type, params_json, status, started_at, finished_at, error_*
```

## Open Questions

- Should the TUI offer to auto-restart `done=false` loops (e.g., job-discovery that was cancelled mid-run), or always require explicit user action?
- The 3-second kill timeout after cancel — should this be configurable in `lazyjob.toml` under `[ralph]`?
- Should stderr logs be viewable in the TUI (e.g., a `<?>` key on the Ralph panel), or strictly external-only?

## Implementation Tasks

- [ ] Define `WorkerCommand` and `WorkerEvent` enums in `lazyjob-ralph/src/protocol.rs` with serde `tag="type"` derivations
- [ ] Implement `RalphProcessManager::spawn()` with tokio::process stdin/stdout pipe, stdout reader task that parses `WorkerEvent` and broadcasts, and stdin writer task consuming `mpsc::Sender<WorkerCommand>`
- [ ] Implement `RalphProcessManager::cancel()` with 3-second kill fallback using `tokio::time::timeout`
- [ ] Implement `RalphProcessManager::send_user_input()` for interactive `MockInterviewLoop` mode
- [ ] Create `ralph_loop_runs` SQLite table DDL (migration) and implement `recover_pending()` to detect TUI-crash-orphaned runs on startup
- [ ] Implement `reap_dead_workers()` via `child.try_wait()` health check — called from a 5-second periodic task in the TUI event loop
- [ ] Add stderr-to-log-file redirection in process spawner (`~/.lazyjob/logs/ralph-<loop_id>.log`, 7-day retention)
- [ ] Write unit tests for `WorkerCommand`/`WorkerEvent` round-trip JSON serialization and the interactive state machine transitions
