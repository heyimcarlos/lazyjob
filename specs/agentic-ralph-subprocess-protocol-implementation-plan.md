# Implementation Plan: Ralph Subprocess IPC Protocol

## Status
Draft

## Related Spec
[specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md)

## Overview

The Ralph subprocess IPC protocol defines how `lazyjob-tui` (and any other orchestrating
process) spawns, communicates with, and recovers from Ralph worker child processes. Workers
are short-lived binaries launched via `tokio::process::Command` with `stdin`, `stdout`, and
`stderr` all redirected. Communication uses newline-delimited JSON (NDJSON): the TUI sends
`WorkerCommand` messages on the child's stdin; the worker emits `WorkerEvent` messages on
stdout. Stderr is redirected to a per-run log file — it never enters the TUI event loop.

The protocol is intentionally minimal: the TUI only needs progress signals and terminal
notifications. All durable output (tailored resumes, cover letter drafts, discovered jobs) is
written directly to the shared SQLite database by the worker. This means TUI restarts are
safe — state is always recoverable from SQLite.

The one exception is `MockInterviewLoop`, which is bidirectional and interactive. After
emitting `AwaitingInput`, the worker blocks on stdin waiting for a `WorkerCommand::UserInput`
reply. The `RalphProcessManager` must treat interactive workers differently from
fire-and-forget workers — specifically, interactive workers must never be subject to
inactivity kill timeouts imposed by the process manager (only by the user's own inactivity
timeout in the TUI).

## Prerequisites

### Must be implemented first
- The workspace `Cargo.toml` must be restructured into a multi-crate workspace with a
  `lazyjob-ralph` member before any code in this plan can compile.
- `lazyjob-core` must exist with at minimum the `LoopType` enum and SQLite connection
  plumbing (`sqlx::SqlitePool`) — the `recover_pending()` method needs pool access.
- `specs/20-openapi-mvp-implementation-plan.md` — follow the workspace Cargo setup
  described there before implementing this plan.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
tokio          = { version = "1", features = ["macros", "rt-multi-thread", "time", "sync", "process", "io-util"] }
tokio-util     = { version = "0.7", features = ["codec"] }
futures-util   = "0.3"
bytes          = "1"
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
uuid           = { version = "1", features = ["v4", "serde"] }
thiserror      = "1"
anyhow         = "1"
tracing        = "0.1"
sqlx           = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "migrate", "macros"] }
chrono         = { version = "0.4", features = ["serde"] }
```

In `lazyjob-ralph/Cargo.toml`:

```toml
[package]
name = "lazyjob-ralph"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio.workspace       = true
tokio-util.workspace  = true
futures-util.workspace = true
bytes.workspace       = true
serde.workspace       = true
serde_json.workspace  = true
uuid.workspace        = true
thiserror.workspace   = true
anyhow.workspace      = true
tracing.workspace     = true
sqlx.workspace        = true
chrono.workspace      = true
lazyjob-core = { path = "../lazyjob-core" }
```

---

## Architecture

### Crate Placement

All types in this plan live in `lazyjob-ralph`. The TUI (`lazyjob-tui`) holds an
`Arc<Mutex<RalphProcessManager>>` obtained through dependency injection at startup. No
subprocess logic bleeds into `lazyjob-core` or `lazyjob-tui` — those crates only import
public protocol types (`WorkerCommand`, `WorkerEvent`, `RalphError`).

`lazyjob-core` defines `LoopType` (a pure enum with no process-management concerns).
`lazyjob-ralph` imports it to label workers and serialize it into `ralph_loop_runs`.

### Module Structure

```
lazyjob-ralph/
  src/
    lib.rs              # pub use of public surface
    error.rs            # RalphError (thiserror)
    protocol.rs         # WorkerCommand, WorkerEvent, codec types
    codec.rs            # NdjsonCodec: tokio_util::codec::{Encoder, Decoder}
    process.rs          # RalphProcessManager, ActiveWorker, PendingLoop
    log_manager.rs      # stderr log file management, 7-day rotation
    db.rs               # ralph_loop_runs DDL + repository methods
  migrations/
    001_ralph_loop_runs.sql
```

### Core Types

#### Protocol messages

```rust
// lazyjob-ralph/src/protocol.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use lazyjob_core::LoopType;

/// Commands sent from the TUI to a running worker via its stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerCommand {
    /// Sent immediately after spawn; communicates the job parameters.
    Start {
        loop_id: Uuid,
        params: serde_json::Value,
    },
    /// Request graceful cancellation. Worker completes the current atomic
    /// unit of work, writes any partial result to SQLite, emits Done
    /// { success: false }, then exits.
    Cancel,
    /// Interactive loops only: deliver the user's typed reply to a
    /// worker that emitted AwaitingInput.
    UserInput {
        text: String,
    },
}

/// Events emitted by a worker on its stdout, one JSON object per line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// First message emitted. Confirms which loop_id and loop_type is live.
    Ready {
        loop_id: Uuid,
        loop_type: LoopType,
    },
    /// Progress update. `progress` is in range 0.0..=1.0.
    Status {
        loop_id: Uuid,
        phase: String,
        progress: f32,
        message: String,
    },
    /// Interactive mode: worker is blocked waiting for WorkerCommand::UserInput.
    /// The TUI must forward the next user input via send_user_input().
    AwaitingInput {
        loop_id: Uuid,
        prompt: String,
    },
    /// A chunk of work completed and written to SQLite. `summary` is human-readable.
    ResultChunk {
        loop_id: Uuid,
        summary: String,
    },
    /// Terminal: the worker is about to exit. success=false means cancelled or
    /// partial; success=true means all work is complete.
    Done {
        loop_id: Uuid,
        success: bool,
    },
    /// Terminal: the worker encountered an unrecoverable error and is exiting.
    Error {
        loop_id: Uuid,
        code: String,
        message: String,
    },
}

impl WorkerEvent {
    /// Extract the loop_id from any event variant.
    pub fn loop_id(&self) -> Uuid {
        match self {
            Self::Ready { loop_id, .. }
            | Self::Status { loop_id, .. }
            | Self::AwaitingInput { loop_id, .. }
            | Self::ResultChunk { loop_id, .. }
            | Self::Done { loop_id, .. }
            | Self::Error { loop_id, .. } => *loop_id,
        }
    }

    /// Returns true if this is a terminal event after which the worker exits.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}
```

#### NDJSON codec

```rust
// lazyjob-ralph/src/codec.rs

use bytes::{Buf, BytesMut};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use tokio_util::codec::{Decoder, Encoder};

/// A tokio-util codec that encodes/decodes newline-delimited JSON.
/// T is the outbound type (what we write), D is the inbound type (what we read).
pub struct NdjsonCodec<T, D> {
    _out: PhantomData<T>,
    _in: PhantomData<D>,
}

impl<T, D> NdjsonCodec<T, D> {
    pub fn new() -> Self {
        Self { _out: PhantomData, _in: PhantomData }
    }
}

impl<T: Serialize, D> Encoder<T> for NdjsonCodec<T, D> {
    type Error = std::io::Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let json = serde_json::to_vec(&item)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        dst.extend_from_slice(&json);
        dst.extend_from_slice(b"\n");
        Ok(())
    }
}

impl<T, D: for<'de> Deserialize<'de>> Decoder for NdjsonCodec<T, D> {
    type Item = D;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(pos) = src.iter().position(|&b| b == b'\n') {
            let line = src.split_to(pos + 1);
            let trimmed = &line[..line.len() - 1]; // strip the newline
            let item = serde_json::from_slice(trimmed)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }
}
```

#### Process manager

```rust
// lazyjob-ralph/src/process.rs

use std::collections::HashMap;
use std::path::PathBuf;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;
use lazyjob_core::LoopType;
use sqlx::SqlitePool;

use crate::error::RalphError;
use crate::protocol::{WorkerCommand, WorkerEvent};

/// Capacity of the broadcast channel that fans out WorkerEvent to all
/// TUI subscribers. 512 covers bursts of rapid Status events.
const EVENT_BROADCAST_CAPACITY: usize = 512;

/// How long after sending Cancel before we SIGKILL the child.
const CANCEL_KILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Per-process state tracked by the manager.
struct ActiveWorker {
    loop_type: LoopType,
    /// Handle to the OS process; used for try_wait() and kill().
    child: Child,
    /// Send WorkerCommands to the stdin-writer background task.
    cmd_tx: mpsc::Sender<WorkerCommand>,
    /// True for MockInterviewLoop — must not be auto-killed on inactivity.
    interactive: bool,
}

/// Returned by recover_pending() to describe each run that was active when
/// the TUI last exited.
#[derive(Debug, Clone)]
pub struct PendingLoop {
    pub loop_id: Uuid,
    pub loop_type: LoopType,
    pub params: serde_json::Value,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// Central manager for all Ralph worker subprocesses.
///
/// Holds a broadcast sender that any TUI component can subscribe to. All
/// WorkerEvent messages from all active workers are multiplexed onto this
/// single channel.
pub struct RalphProcessManager {
    /// Absolute path to the ralph binary (same binary, `ralph` subcommand).
    ralph_bin: PathBuf,
    /// Path to the SQLite database, forwarded to each worker via env.
    db_path: PathBuf,
    /// Path to the life sheet YAML, forwarded to each worker via env.
    life_sheet_path: PathBuf,
    /// All currently live workers keyed by their loop_id.
    active: HashMap<Uuid, ActiveWorker>,
    /// Cloning this gives any subscriber a receiver for WorkerEvent.
    event_tx: broadcast::Sender<WorkerEvent>,
    /// Database pool used by recover_pending() and db helpers.
    pool: SqlitePool,
}

impl RalphProcessManager {
    pub fn new(
        ralph_bin: PathBuf,
        db_path: PathBuf,
        life_sheet_path: PathBuf,
        pool: SqlitePool,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            ralph_bin,
            db_path,
            life_sheet_path,
            active: HashMap::new(),
            event_tx,
            pool,
        }
    }

    /// Subscribe to all WorkerEvent messages from all workers.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn a new worker for the given loop type with the given params.
    /// Inserts a row into ralph_loop_runs (status=running), starts the
    /// process, and returns the assigned loop_id.
    pub async fn spawn(
        &mut self,
        loop_type: LoopType,
        params: serde_json::Value,
    ) -> Result<Uuid, RalphError> {
        let loop_id = Uuid::new_v4();
        let interactive = loop_type == LoopType::MockInterview;

        let log_path = crate::log_manager::log_path_for(loop_id)?;
        let log_file = tokio::fs::File::create(&log_path).await
            .map_err(RalphError::LogFileCreate)?;

        let mut child = tokio::process::Command::new(&self.ralph_bin)
            .arg("worker")
            .env("LAZYJOB_DB_PATH", &self.db_path)
            .env("LAZYJOB_LIFE_SHEET_PATH", &self.life_sheet_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(log_file.into_std().await)
            .spawn()
            .map_err(RalphError::Spawn)?;

        let stdin: ChildStdin = child.stdin.take().expect("stdin piped");
        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");

        // Channel for the TUI to push commands to the stdin-writer task.
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>(32);

        // Immediately send the Start command so the worker knows its parameters.
        cmd_tx
            .send(WorkerCommand::Start { loop_id, params: params.clone() })
            .await
            .map_err(|_| RalphError::ChannelClosed)?;

        // Background task: write WorkerCommands to child stdin.
        tokio::spawn(stdin_writer_task(stdin, cmd_rx, loop_id));

        // Background task: read WorkerEvents from child stdout and broadcast them.
        let event_tx = self.event_tx.clone();
        let pool_clone = self.pool.clone();
        tokio::spawn(stdout_reader_task(stdout, loop_id, event_tx, pool_clone));

        // Record the run in SQLite.
        crate::db::insert_loop_run(&self.pool, loop_id, loop_type, &params).await?;

        self.active.insert(loop_id, ActiveWorker {
            loop_type,
            child,
            cmd_tx,
            interactive,
        });

        tracing::info!(loop_id = %loop_id, ?loop_type, "spawned ralph worker");
        Ok(loop_id)
    }

    /// Send user input to an interactive worker (MockInterviewLoop only).
    pub async fn send_user_input(
        &mut self,
        loop_id: Uuid,
        text: String,
    ) -> Result<(), RalphError> {
        let worker = self.active.get(&loop_id)
            .ok_or(RalphError::UnknownLoop(loop_id))?;
        if !worker.interactive {
            return Err(RalphError::NotInteractive(loop_id));
        }
        worker.cmd_tx
            .send(WorkerCommand::UserInput { text })
            .await
            .map_err(|_| RalphError::ChannelClosed)
    }

    /// Request graceful cancellation. Sends Cancel, waits up to 3 s,
    /// then SIGKILL if necessary.
    pub async fn cancel(&mut self, loop_id: Uuid) -> Result<(), RalphError> {
        let worker = self.active.get_mut(&loop_id)
            .ok_or(RalphError::UnknownLoop(loop_id))?;

        // Best-effort: if the cmd channel is already closed, skip to kill.
        let _ = worker.cmd_tx.send(WorkerCommand::Cancel).await;

        let result = tokio::time::timeout(
            CANCEL_KILL_TIMEOUT,
            worker.child.wait(),
        )
        .await;

        if result.is_err() {
            // Timed out: escalate to SIGKILL.
            tracing::warn!(loop_id = %loop_id, "cancel timed out — killing worker");
            let _ = worker.child.kill().await;
            let _ = worker.child.wait().await;
        }

        self.active.remove(&loop_id);
        crate::db::update_loop_run_status(&self.pool, loop_id, "cancelled", None, None).await?;
        tracing::info!(loop_id = %loop_id, "cancelled ralph worker");
        Ok(())
    }

    /// Reap workers whose child processes have exited without emitting Done/Error.
    /// Call this from a periodic health-check task (every 5 s in the TUI event loop).
    pub fn reap_dead_workers(&mut self) {
        let mut dead = Vec::new();
        for (&loop_id, worker) in self.active.iter_mut() {
            match worker.child.try_wait() {
                Ok(Some(status)) => {
                    tracing::warn!(
                        loop_id = %loop_id,
                        ?status,
                        "ralph worker exited unexpectedly"
                    );
                    dead.push(loop_id);
                }
                Ok(None) => {} // still running
                Err(e) => {
                    tracing::error!(loop_id = %loop_id, error = %e, "try_wait failed");
                }
            }
        }
        for loop_id in dead {
            self.active.remove(&loop_id);
            // Synthesize an Error event so TUI panels can reflect the crash.
            let event = WorkerEvent::Error {
                loop_id,
                code: "unexpected_exit".to_string(),
                message: "Worker process exited without sending Done or Error".to_string(),
            };
            let _ = self.event_tx.send(event);
            // Fire and forget the DB update; reap_dead_workers is sync.
            let pool = self.pool.clone();
            tokio::spawn(async move {
                let msg = "unexpected exit detected by reaper";
                let _ = crate::db::update_loop_run_status(
                    &pool, loop_id, "failed",
                    Some("unexpected_exit"), Some(msg),
                ).await;
            });
        }
    }

    /// On TUI startup: finds ralph_loop_runs rows with status='running',
    /// which indicate the TUI crashed while workers were active.
    /// Marks them 'failed', broadcasts synthetic Error events, and returns
    /// their metadata so the TUI can offer a "re-run?" prompt.
    pub async fn recover_pending(
        &mut self,
    ) -> Result<Vec<PendingLoop>, RalphError> {
        let pending = crate::db::find_pending_runs(&self.pool).await?;

        for p in &pending {
            let event = WorkerEvent::Error {
                loop_id: p.loop_id,
                code: "tui_restart".to_string(),
                message: "TUI restarted while loop was running".to_string(),
            };
            // Subscribers may not exist yet; ignore send errors.
            let _ = self.event_tx.send(event);

            crate::db::update_loop_run_status(
                &self.pool, p.loop_id, "failed",
                Some("tui_restart"),
                Some("TUI restarted while loop was running"),
            ).await?;
        }

        tracing::info!(count = pending.len(), "recovered pending ralph loops on startup");
        Ok(pending)
    }
}
```

#### Background tasks (private)

```rust
// Still in lazyjob-ralph/src/process.rs (private functions)

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Reads WorkerCommands from `cmd_rx` and writes them as NDJSON to child stdin.
async fn stdin_writer_task(
    mut stdin: ChildStdin,
    mut cmd_rx: mpsc::Receiver<WorkerCommand>,
    loop_id: Uuid,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        let Ok(mut json) = serde_json::to_vec(&cmd) else {
            tracing::error!(loop_id = %loop_id, "failed to serialize WorkerCommand");
            continue;
        };
        json.push(b'\n');
        if let Err(e) = stdin.write_all(&json).await {
            tracing::warn!(loop_id = %loop_id, error = %e, "stdin write failed — worker may have exited");
            break;
        }
        if let Err(e) = stdin.flush().await {
            tracing::warn!(loop_id = %loop_id, error = %e, "stdin flush failed");
            break;
        }
    }
    // Dropping stdin here closes the pipe — the worker's next stdin read gets EOF.
}

/// Reads NDJSON WorkerEvents from child stdout line-by-line, parses them,
/// broadcasts on event_tx, and updates ralph_loop_runs on terminal events.
async fn stdout_reader_task(
    stdout: ChildStdout,
    loop_id: Uuid,
    event_tx: broadcast::Sender<WorkerEvent>,
    pool: SqlitePool,
) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let event: WorkerEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(
                    loop_id = %loop_id,
                    raw = %line,
                    error = %err,
                    "failed to parse WorkerEvent — skipping"
                );
                continue;
            }
        };

        let is_terminal = event.is_terminal();

        if let WorkerEvent::Done { success, .. } = &event {
            let status = if *success { "done" } else { "failed" };
            let _ = crate::db::update_loop_run_status(
                &pool, loop_id, status, None, None,
            ).await;
        } else if let WorkerEvent::Error { code, message, .. } = &event {
            let _ = crate::db::update_loop_run_status(
                &pool, loop_id, "failed",
                Some(code.as_str()), Some(message.as_str()),
            ).await;
        }

        // Ignore Lagged errors — slow TUI subscribers just miss events.
        let _ = event_tx.send(event);

        if is_terminal {
            break;
        }
    }
}
```

### SQLite Schema

```sql
-- lazyjob-ralph/migrations/001_ralph_loop_runs.sql

CREATE TABLE IF NOT EXISTS ralph_loop_runs (
    id           TEXT    PRIMARY KEY,          -- UUID as text
    loop_type    TEXT    NOT NULL,             -- LoopType serialized snake_case
    params_json  TEXT    NOT NULL,             -- serde_json::Value
    status       TEXT    NOT NULL DEFAULT 'pending', -- pending|running|done|failed|cancelled
    started_at   TEXT,                         -- RFC3339 timestamp
    finished_at  TEXT,                         -- RFC3339 timestamp
    error_code   TEXT,
    error_msg    TEXT,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_ralph_loop_runs_status
    ON ralph_loop_runs (status);

CREATE INDEX IF NOT EXISTS idx_ralph_loop_runs_loop_type
    ON ralph_loop_runs (loop_type);
```

### Log Manager

```rust
// lazyjob-ralph/src/log_manager.rs

use std::path::PathBuf;
use uuid::Uuid;

use crate::error::RalphError;

/// Returns the path for a worker's stderr log file.
/// Creates parent directories if they don't exist.
pub fn log_path_for(loop_id: Uuid) -> Result<PathBuf, RalphError> {
    let log_dir = log_dir()?;
    std::fs::create_dir_all(&log_dir).map_err(RalphError::LogDirCreate)?;
    Ok(log_dir.join(format!("ralph-{}.log", loop_id)))
}

fn log_dir() -> Result<PathBuf, RalphError> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| RalphError::NoHomeDir)?;
    Ok(PathBuf::from(home).join(".lazyjob").join("logs"))
}

/// Delete log files older than 7 days. Call this on TUI startup after
/// recover_pending() to avoid unbounded log accumulation.
pub async fn prune_old_logs() -> Result<(), RalphError> {
    let log_dir = log_dir()?;
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(7 * 24 * 3600);

    let mut dir = match tokio::fs::read_dir(&log_dir).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(RalphError::LogDirRead(e)),
    };

    while let Some(entry) = dir.next_entry().await.map_err(RalphError::LogDirRead)? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let meta = tokio::fs::metadata(&path).await.map_err(RalphError::LogDirRead)?;
        if let Ok(modified) = meta.modified() {
            if modified < cutoff {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }
    Ok(())
}
```

### Database Helper

```rust
// lazyjob-ralph/src/db.rs

use uuid::Uuid;
use sqlx::SqlitePool;
use chrono::Utc;
use lazyjob_core::LoopType;

use crate::error::RalphError;
use crate::process::PendingLoop;

pub async fn insert_loop_run(
    pool: &SqlitePool,
    loop_id: Uuid,
    loop_type: LoopType,
    params: &serde_json::Value,
) -> Result<(), RalphError> {
    let id = loop_id.to_string();
    let lt = serde_json::to_string(&loop_type).map_err(RalphError::Serialize)?;
    let params_json = serde_json::to_string(params).map_err(RalphError::Serialize)?;
    let started_at = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO ralph_loop_runs (id, loop_type, params_json, status, started_at)
         VALUES (?, ?, ?, 'running', ?)",
        id, lt, params_json, started_at,
    )
    .execute(pool)
    .await
    .map_err(RalphError::Database)?;

    Ok(())
}

pub async fn update_loop_run_status(
    pool: &SqlitePool,
    loop_id: Uuid,
    status: &str,
    error_code: Option<&str>,
    error_msg: Option<&str>,
) -> Result<(), RalphError> {
    let id = loop_id.to_string();
    let finished_at = Utc::now().to_rfc3339();

    sqlx::query!(
        "UPDATE ralph_loop_runs
         SET status = ?, finished_at = ?, error_code = ?, error_msg = ?
         WHERE id = ?",
        status, finished_at, error_code, error_msg, id,
    )
    .execute(pool)
    .await
    .map_err(RalphError::Database)?;

    Ok(())
}

/// Returns all runs with status='running' — these represent TUI-crash survivors.
pub async fn find_pending_runs(pool: &SqlitePool) -> Result<Vec<PendingLoop>, RalphError> {
    let rows = sqlx::query!(
        "SELECT id, loop_type, params_json, started_at
         FROM ralph_loop_runs
         WHERE status = 'running'"
    )
    .fetch_all(pool)
    .await
    .map_err(RalphError::Database)?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let loop_id = Uuid::parse_str(&row.id).map_err(|_| RalphError::InvalidUuid(row.id.clone()))?;
        let loop_type: LoopType = serde_json::from_str(&row.loop_type)
            .map_err(RalphError::Deserialize)?;
        let params: serde_json::Value = serde_json::from_str(&row.params_json)
            .map_err(RalphError::Deserialize)?;
        let started_at = row.started_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        result.push(PendingLoop { loop_id, loop_type, params, started_at });
    }
    Ok(result)
}
```

### Error Type

```rust
// lazyjob-ralph/src/error.rs

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RalphError {
    #[error("failed to spawn ralph worker process: {0}")]
    Spawn(#[source] std::io::Error),

    #[error("no active worker with loop_id={0}")]
    UnknownLoop(Uuid),

    #[error("loop {0} is not interactive — cannot send UserInput")]
    NotInteractive(Uuid),

    #[error("worker command channel closed unexpectedly")]
    ChannelClosed,

    #[error("SQLite error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("deserialize error: {0}")]
    Deserialize(serde_json::Error),

    #[error("invalid UUID in ralph_loop_runs: {0}")]
    InvalidUuid(String),

    #[error("failed to create log file: {0}")]
    LogFileCreate(#[source] std::io::Error),

    #[error("failed to create log directory: {0}")]
    LogDirCreate(#[source] std::io::Error),

    #[error("failed to read log directory: {0}")]
    LogDirRead(#[source] std::io::Error),

    #[error("cannot determine home directory (HOME / USERPROFILE not set)")]
    NoHomeDir,
}

pub type Result<T> = std::result::Result<T, RalphError>;
```

---

## Implementation Phases

### Phase 1 — Protocol Types and NDJSON Codec (Foundation)

**Goal**: Define all message types and the serialization codec; nothing that touches processes yet.

#### Step 1.1 — Workspace restructure

Convert the single-package `Cargo.toml` to a workspace. Add `lazyjob-ralph` as a member.

```toml
# Cargo.toml (root)
[workspace]
members = [
    "lazyjob-core",
    "lazyjob-ralph",
    "lazyjob-llm",
    "lazyjob-tui",
    "lazyjob-cli",
]
resolver = "2"

[workspace.dependencies]
# ... (full list from Prerequisites section)
```

```
mkdir -p lazyjob-ralph/src lazyjob-ralph/migrations
```

**File**: `lazyjob-ralph/Cargo.toml` — as specified in Prerequisites.

#### Step 1.2 — LoopType enum in lazyjob-core

```rust
// lazyjob-core/src/loop_type.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    MockInterview,
    InterviewPrep,
    SalaryResearch,
    NetworkingOutreach,
}
```

**Verification**: `cargo check -p lazyjob-core` passes.

#### Step 1.3 — protocol.rs

Implement `WorkerCommand`, `WorkerEvent`, and their helper methods as shown in the Core Types section.

**Verification**: 
```rust
// Round-trip test (in protocol.rs #[cfg(test)])
let cmd = WorkerCommand::Start {
    loop_id: Uuid::nil(),
    params: serde_json::json!({"job_id": "123"}),
};
let json = serde_json::to_string(&cmd).unwrap();
assert!(json.contains("\"type\":\"start\""));
let back: WorkerCommand = serde_json::from_str(&json).unwrap();
```

#### Step 1.4 — codec.rs and error.rs

Implement `NdjsonCodec<T, D>` and `RalphError` as shown above.

**Verification**:
```rust
// Encode then decode a WorkerEvent::Status round-trip using BytesMut.
use tokio_util::codec::{Encoder, Decoder};
use bytes::BytesMut;

let mut codec: NdjsonCodec<WorkerEvent, WorkerEvent> = NdjsonCodec::new();
let event = WorkerEvent::Status {
    loop_id: Uuid::nil(),
    phase: "test".into(),
    progress: 0.5,
    message: "half done".into(),
};
let mut buf = BytesMut::new();
codec.encode(event.clone(), &mut buf).unwrap();
assert!(buf.ends_with(b"\n"));
let decoded = codec.decode(&mut buf).unwrap().unwrap();
// decoded should match event (requires PartialEq on WorkerEvent)
```

---

### Phase 2 — SQLite Schema and DB Helper

**Goal**: Create `ralph_loop_runs` table and all CRUD operations needed by the process manager.

#### Step 2.1 — Migration file

Write `lazyjob-ralph/migrations/001_ralph_loop_runs.sql` (DDL in Schema section).

#### Step 2.2 — db.rs

Implement `insert_loop_run`, `update_loop_run_status`, `find_pending_runs` using `sqlx::query!`
macros (checked at compile time against the migration).

Key API surface:
- `sqlx::query!` — compile-time verified SQL
- `pool.fetch_all()` — returns `Vec<Row>`
- `pool.execute()` — for INSERT/UPDATE

**Verification**:
```rust
// Integration test using sqlx::test attribute (applies migrations automatically)
#[sqlx::test(migrations = "migrations")]
async fn test_insert_and_find_pending(pool: SqlitePool) {
    let id = Uuid::new_v4();
    insert_loop_run(&pool, id, LoopType::JobDiscovery, &serde_json::json!({})).await.unwrap();
    let pending = find_pending_runs(&pool).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].loop_id, id);
}
```

---

### Phase 3 — Process Manager (Core)

**Goal**: Implement `RalphProcessManager` with `spawn()`, `cancel()`, `reap_dead_workers()`,
and `recover_pending()`.

#### Step 3.1 — log_manager.rs

Implement `log_path_for()` and `prune_old_logs()`.

**Key API**: `tokio::fs::File::create()`, `tokio::fs::read_dir()`.

**Verification**: `log_path_for(Uuid::new_v4())` returns a path under `~/.lazyjob/logs/` and
calling it twice creates the directory only once.

#### Step 3.2 — process.rs: ActiveWorker, background tasks

Implement `stdin_writer_task` and `stdout_reader_task` as private async functions. The reader
task is the heart of the protocol — it must:
1. Parse each line as a `WorkerEvent`.
2. Broadcast on `event_tx`.
3. On `Done` or `Error`: call `update_loop_run_status`, then `break`.

Key API:
- `tokio::io::BufReader::new(stdout).lines()` — async line iterator
- `broadcast::Sender::send()` — returns Err only if there are zero receivers; ignore safely
- `mpsc::Receiver::recv()` — blocks until a command arrives or all senders drop

#### Step 3.3 — process.rs: RalphProcessManager::spawn()

Sequence:
1. Generate `Uuid::new_v4()` for `loop_id`.
2. Create log file via `log_manager::log_path_for()`.
3. Call `tokio::process::Command::new(&self.ralph_bin).arg("worker")` with all stdio redirections.
4. Take `child.stdin` and `child.stdout` (must happen before spawn, via `.stdin(Stdio::piped())`).
5. Create `(cmd_tx, cmd_rx)` mpsc pair.
6. Send `WorkerCommand::Start` on `cmd_tx`.
7. `tokio::spawn` both background tasks.
8. Call `db::insert_loop_run()`.
9. Insert `ActiveWorker` into `self.active`.

Key API:
- `tokio::process::Command::new(path)` — spawns an async child
- `.stdin(std::process::Stdio::piped())` — must be set before `.spawn()`
- `child.stdin.take()` / `child.stdout.take()` — must be called on the `Child` before moving it
- `tokio::spawn(async move { ... })` — detached background tasks

#### Step 3.4 — process.rs: cancel(), reap_dead_workers(), recover_pending()

- **cancel**: `cmd_tx.send(WorkerCommand::Cancel)`, then `tokio::time::timeout(3s, child.wait())`,
  then `child.kill()` if timed out.
- **reap_dead_workers**: iterate `active`, call `child.try_wait()` (non-blocking), collect dead
  entries, synthesize `WorkerEvent::Error` for each, update DB in a detached `tokio::spawn`.
- **recover_pending**: `db::find_pending_runs()`, broadcast synthetic `Error` events, call
  `update_loop_run_status(... "failed" ...)` for each.

Key API:
- `tokio::time::timeout(duration, future)` — returns `Err(Elapsed)` on timeout
- `Child::try_wait()` — returns `Ok(None)` if still running, `Ok(Some(status))` if exited
- `Child::kill()` — async SIGKILL on Unix

**Verification**:
```rust
// Integration test: spawn a worker that immediately exits, then reap it.
// Use a test binary that prints '{"type":"done","loop_id":"...","success":true}\n' then exits.
```

---

### Phase 4 — Worker Side: Protocol Implementation

**Goal**: Implement the worker entrypoint — the code that runs inside the child process.

#### Step 4.1 — Worker entrypoint in lazyjob-cli

When the TUI spawns a worker, it calls `ralph worker`. The CLI binary handles this subcommand:

```rust
// lazyjob-cli/src/main.rs

match args.subcommand {
    Subcommand::Worker => lazyjob_ralph::worker::run_worker().await?,
    Subcommand::Tui   => lazyjob_tui::run().await?,
}
```

#### Step 4.2 — Worker bootstrap in lazyjob-ralph

```rust
// lazyjob-ralph/src/worker/mod.rs

pub async fn run_worker() -> anyhow::Result<()> {
    // Read the first line from stdin — it must be WorkerCommand::Start.
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut first_line = String::new();
    stdin.read_line(&mut first_line).await?;

    let cmd: WorkerCommand = serde_json::from_str(first_line.trim())
        .context("expected WorkerCommand::Start as first message")?;

    let WorkerCommand::Start { loop_id, params } = cmd else {
        anyhow::bail!("first message must be Start, got something else");
    };

    // Resolve the loop type from the DB.
    let db_path = std::env::var("LAZYJOB_DB_PATH")?;
    let pool = SqlitePool::connect(&format!("sqlite:{}", db_path)).await?;
    let loop_type = crate::db::get_loop_type(&pool, loop_id).await?;

    // Emit Ready.
    emit(WorkerEvent::Ready { loop_id, loop_type })?;

    // Dispatch to the appropriate loop handler.
    match loop_type {
        LoopType::JobDiscovery      => loops::job_discovery::run(loop_id, params, stdin, &pool).await,
        LoopType::ResumeTailoring   => loops::resume_tailoring::run(loop_id, params, stdin, &pool).await,
        LoopType::MockInterview     => loops::mock_interview::run(loop_id, params, stdin, &pool).await,
        // ... all LoopType variants
    }
}

/// Write a WorkerEvent as NDJSON to stdout.
pub fn emit(event: WorkerEvent) -> anyhow::Result<()> {
    use std::io::Write;
    let mut json = serde_json::to_vec(&event)?;
    json.push(b'\n');
    std::io::stdout().write_all(&json)?;
    std::io::stdout().flush()?;
    Ok(())
}
```

#### Step 4.3 — Cancellation handling in workers

Each worker must periodically poll for a `WorkerCommand::Cancel` on stdin without blocking.
The standard pattern:

```rust
// lazyjob-ralph/src/worker/cancel.rs

use tokio::sync::watch;

/// A CancelToken allows workers to check for cancellation between LLM calls.
pub struct CancelToken {
    rx: watch::Receiver<bool>,
}

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }
}

pub struct CancelTokenSource {
    tx: watch::Sender<bool>,
}

impl CancelTokenSource {
    pub fn new() -> (CancelTokenSource, CancelToken) {
        let (tx, rx) = watch::channel(false);
        (CancelTokenSource { tx }, CancelToken { rx })
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }
}

/// Spawns a background task that reads stdin looking for WorkerCommand::Cancel
/// and fires the cancel token when found.
pub fn watch_stdin_for_cancel(
    mut stdin: tokio::io::BufReader<tokio::io::Stdin>,
    source: CancelTokenSource,
    loop_id: Uuid,
) {
    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if let Ok(WorkerCommand::Cancel) = serde_json::from_str(line.trim()) {
                        tracing::info!(loop_id = %loop_id, "received Cancel command");
                        source.cancel();
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "stdin read error in cancel watcher");
                    break;
                }
            }
        }
    });
}
```

Workers check `cancel_token.is_cancelled()` between LLM calls. When true, they emit
`WorkerEvent::Done { success: false }` and return.

#### Step 4.4 — Interactive mode (MockInterviewLoop)

The mock interview loop uses a different stdin pattern — it explicitly waits for
`WorkerCommand::UserInput` after each `AwaitingInput` event:

```rust
// lazyjob-ralph/src/worker/loops/mock_interview.rs

pub async fn run(
    loop_id: Uuid,
    params: serde_json::Value,
    mut stdin: tokio::io::BufReader<tokio::io::Stdin>,
    pool: &SqlitePool,
) -> anyhow::Result<()> {
    // ... setup ...
    for round in 0..MAX_ROUNDS {
        // Ask the LLM for an interview question.
        let question = llm.ask_question(&context).await?;
        emit(WorkerEvent::AwaitingInput {
            loop_id,
            prompt: question.clone(),
        })?;

        // Block on stdin for UserInput (or Cancel).
        let user_text = read_user_input(&mut stdin, loop_id).await?;
        if user_text.is_none() {
            // Received Cancel.
            emit(WorkerEvent::Done { loop_id, success: false })?;
            return Ok(());
        }

        let answer = user_text.unwrap();
        let feedback = llm.score_answer(&question, &answer, &context).await?;
        // Write feedback to SQLite, emit ResultChunk.
        // ...
    }
    emit(WorkerEvent::Done { loop_id, success: true })?;
    Ok(())
}

async fn read_user_input(
    stdin: &mut tokio::io::BufReader<tokio::io::Stdin>,
    loop_id: Uuid,
) -> anyhow::Result<Option<String>> {
    let mut line = String::new();
    loop {
        line.clear();
        stdin.read_line(&mut line).await?;
        match serde_json::from_str::<WorkerCommand>(line.trim())? {
            WorkerCommand::UserInput { text } => return Ok(Some(text)),
            WorkerCommand::Cancel => return Ok(None),
            WorkerCommand::Start { .. } => {} // ignore duplicate starts
        }
    }
}
```

**Verification**: Spawn a mock interview worker in a test, write simulated `UserInput` events
to its stdin, read `AwaitingInput` events from stdout, verify the conversation progresses.

---

### Phase 5 — TUI Integration

**Goal**: Wire `RalphProcessManager` into `AppState` and the TUI event loop.

#### Step 5.1 — AppState holds the manager

```rust
// lazyjob-tui/src/app.rs

use std::sync::Arc;
use tokio::sync::Mutex;
use lazyjob_ralph::process::RalphProcessManager;

pub struct AppState {
    pub ralph: Arc<Mutex<RalphProcessManager>>,
    pub ralph_events: tokio::sync::broadcast::Receiver<WorkerEvent>,
    // ... other state
}
```

#### Step 5.2 — Event loop integration

In the main TUI event loop (`tokio::select!`), add a branch for Ralph events:

```rust
// lazyjob-tui/src/event_loop.rs

tokio::select! {
    // Crossterm keyboard/resize events
    Some(Ok(event)) = crossterm_event_stream.next() => {
        handle_crossterm_event(&mut app, event).await?;
    }

    // Ralph worker events
    Ok(worker_event) = app.ralph_events.recv() => {
        handle_ralph_event(&mut app, worker_event).await?;
    }

    // 60fps render tick
    _ = render_interval.tick() => {
        render(&mut terminal, &app)?;
    }

    // 5s health check tick for reaping dead workers
    _ = health_tick.tick() => {
        app.ralph.lock().await.reap_dead_workers();
    }
}
```

#### Step 5.3 — Startup sequence

```rust
// lazyjob-tui/src/startup.rs

pub async fn run_startup(app: &mut AppState) -> anyhow::Result<()> {
    // Prune old log files (fire-and-forget; don't block startup).
    tokio::spawn(lazyjob_ralph::log_manager::prune_old_logs());

    // Recover any runs that were live when the TUI last crashed.
    let pending = app.ralph.lock().await.recover_pending().await?;

    if !pending.is_empty() {
        // Show a recovery dialog: "N loops were interrupted. Re-run?"
        app.pending_recovery_loops = pending;
        app.mode = AppMode::RecoveryDialog;
    }

    Ok(())
}
```

---

## Key Crate APIs

The following specific APIs will be called (not just crate names):

- `tokio::process::Command::new(path).arg("worker").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(file).spawn()` — spawns the worker child
- `Child::stdin.take()` / `Child::stdout.take()` — must be called on `Child` before it is moved into `ActiveWorker`
- `Child::try_wait()` — synchronous (non-blocking) status check; returns `Ok(None)` if still running
- `Child::kill()` — async SIGKILL (Unix: SIGKILL via libc, Windows: TerminateProcess)
- `Child::wait()` — async wait for the child to exit (used inside `tokio::time::timeout`)
- `tokio::time::timeout(duration, future)` — returns `Result<T, Elapsed>`; used for the 3-second cancel grace period
- `tokio::io::BufReader::new(stdout).lines()` — async line iterator via `AsyncBufReadExt::lines()`
- `tokio::io::AsyncWriteExt::write_all(&mut stdin, &bytes)` / `flush()` — write NDJSON line to child stdin
- `tokio::sync::broadcast::channel(capacity)` — fan-out channel; `Sender::send()` returns `Err(SendError)` only if there are zero receivers
- `tokio::sync::mpsc::channel(capacity)` — the per-worker command channel; `Sender::send().await` blocks if full
- `tokio::sync::watch::channel(initial)` — used for the CancelToken; cheap clone, only latest value stored
- `sqlx::query!("...", args).execute(pool).await` — compile-time verified SQL execution
- `sqlx::query!("...", args).fetch_all(pool).await` — returns `Vec<Row>`
- `Uuid::new_v4()` — generate a random loop ID
- `serde_json::to_vec(&value)` / `serde_json::from_str::<T>(&s)` — serialize/deserialize NDJSON frames
- `tokio::fs::File::create(path).await` — create the stderr log file asynchronously
- `tokio::fs::read_dir(path).await` — iterate log directory for pruning

---

## Error Handling

```rust
// lazyjob-ralph/src/error.rs (complete definition — already shown above)
#[derive(Debug, thiserror::Error)]
pub enum RalphError {
    Spawn(std::io::Error),         // tokio::process::Command::spawn() failed
    UnknownLoop(Uuid),             // loop_id not in active map
    NotInteractive(Uuid),          // send_user_input called on non-interactive loop
    ChannelClosed,                 // mpsc::Sender::send().await returned Err
    Database(sqlx::Error),         // any SQLite error
    Serialize(serde_json::Error),  // JSON serialization error
    Deserialize(serde_json::Error),// JSON deserialization error
    InvalidUuid(String),           // malformed UUID in DB
    LogFileCreate(std::io::Error), // cannot create ~/.lazyjob/logs/ralph-*.log
    LogDirCreate(std::io::Error),  // cannot mkdir ~/.lazyjob/logs/
    LogDirRead(std::io::Error),    // readdir error during log pruning
    NoHomeDir,                     // HOME / USERPROFILE not set
}
```

**Error propagation rules**:
- `RalphProcessManager` public methods return `Result<_, RalphError>` — callers (TUI) can
  match on specific variants to show user-facing messages.
- Background tasks (`stdin_writer_task`, `stdout_reader_task`) use `tracing::warn/error!` for
  non-fatal errors and silently return on fatal ones — they must never panic as they run on
  the tokio thread pool.
- Worker-side code uses `anyhow::Result` internally and converts to `RalphError` at the
  boundary if it surfaces via a `WorkerEvent::Error` message.

---

## Testing Strategy

### Unit tests

**protocol.rs round-trips** (in `#[cfg(test)]` within `protocol.rs`):
```rust
#[test]
fn test_worker_command_serde_tag() {
    let cmd = WorkerCommand::Cancel;
    let json = serde_json::to_string(&cmd).unwrap();
    assert_eq!(json, r#"{"type":"cancel"}"#);
    let back: WorkerCommand = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, WorkerCommand::Cancel));
}

#[test]
fn test_worker_event_is_terminal() {
    let e = WorkerEvent::Done { loop_id: Uuid::nil(), success: true };
    assert!(e.is_terminal());
    let e2 = WorkerEvent::Status { loop_id: Uuid::nil(), phase: "x".into(), progress: 0.5, message: "".into() };
    assert!(!e2.is_terminal());
}
```

**NdjsonCodec round-trip** (in `codec.rs`):
```rust
#[test]
fn test_codec_encode_decode() {
    use tokio_util::codec::{Encoder, Decoder};
    let mut codec: NdjsonCodec<WorkerEvent, WorkerEvent> = NdjsonCodec::new();
    let event = WorkerEvent::Ready { loop_id: Uuid::nil(), loop_type: LoopType::JobDiscovery };
    let mut buf = BytesMut::new();
    codec.encode(event, &mut buf).unwrap();
    assert!(buf.last() == Some(&b'\n'));
    let decoded = codec.decode(&mut buf).unwrap().unwrap();
    assert!(matches!(decoded, WorkerEvent::Ready { .. }));
}
```

**CancelToken** (in `cancel.rs`):
```rust
#[test]
fn test_cancel_token_default_not_cancelled() {
    let (_source, token) = CancelTokenSource::new();
    assert!(!token.is_cancelled());
}
#[test]
fn test_cancel_token_after_cancel() {
    let (source, token) = CancelTokenSource::new();
    source.cancel();
    assert!(token.is_cancelled());
}
```

### Integration tests (with SQLite)

Use `#[sqlx::test(migrations = "migrations")]` which auto-applies migrations to an
in-memory SQLite instance:

```rust
// lazyjob-ralph/tests/db_tests.rs

#[sqlx::test(migrations = "migrations")]
async fn test_insert_then_find_pending(pool: SqlitePool) {
    let id = Uuid::new_v4();
    db::insert_loop_run(&pool, id, LoopType::JobDiscovery, &serde_json::json!({})).await.unwrap();
    let pending = db::find_pending_runs(&pool).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].loop_id, id);
}

#[sqlx::test(migrations = "migrations")]
async fn test_update_status_to_done(pool: SqlitePool) {
    let id = Uuid::new_v4();
    db::insert_loop_run(&pool, id, LoopType::JobDiscovery, &serde_json::json!({})).await.unwrap();
    db::update_loop_run_status(&pool, id, "done", None, None).await.unwrap();
    let pending = db::find_pending_runs(&pool).await.unwrap();
    assert!(pending.is_empty());
}
```

### Integration tests (with real subprocess)

Create a test helper binary `tests/fake_worker.rs` that:
1. Reads `WorkerCommand::Start` from stdin.
2. Emits `WorkerEvent::Ready`.
3. Emits `WorkerEvent::Status { progress: 0.5 }`.
4. Emits `WorkerEvent::Done { success: true }`.
5. Exits 0.

```rust
// lazyjob-ralph/tests/process_manager_test.rs

#[tokio::test]
async fn test_spawn_and_receive_done() {
    let pool = setup_test_pool().await;
    let mut manager = RalphProcessManager::new(
        PathBuf::from("tests/fake_worker"),
        PathBuf::from(":memory:"),
        PathBuf::from(""),
        pool,
    );
    let mut rx = manager.subscribe();
    let loop_id = manager.spawn(LoopType::JobDiscovery, serde_json::json!({})).await.unwrap();

    let mut events = vec![];
    while let Ok(e) = rx.recv().await {
        let terminal = e.is_terminal();
        events.push(e);
        if terminal { break; }
    }

    assert!(events.iter().any(|e| matches!(e, WorkerEvent::Ready { .. })));
    assert!(events.iter().any(|e| matches!(e, WorkerEvent::Done { success: true, .. })));
    assert_eq!(events.last().unwrap().loop_id(), loop_id);
}
```

### Cancel test

```rust
#[tokio::test]
async fn test_cancel_sends_command_then_kills_if_needed() {
    // Use a fake_worker that sleeps 60s after Ready — simulates a hung worker.
    // cancel() should return within ~4s (3s timeout + kill).
    let start = std::time::Instant::now();
    manager.cancel(loop_id).await.unwrap();
    assert!(start.elapsed().as_secs() < 5);
}
```

### Interactive mode test

```rust
#[tokio::test]
async fn test_interactive_worker_user_input() {
    // fake_worker_interactive: emits AwaitingInput, waits for UserInput,
    // echoes text back as ResultChunk, then Done.
    let loop_id = manager.spawn(LoopType::MockInterview, serde_json::json!({})).await.unwrap();
    // Wait for AwaitingInput
    // ...
    manager.send_user_input(loop_id, "test answer".to_string()).await.unwrap();
    // Verify ResultChunk with "test answer" summary arrives
}
```

---

## Open Questions

1. **Auto-restart on `done=false`**: Should `RalphProcessManager` support an
   `auto_restart: bool` field on `ActiveWorker` that re-invokes `spawn()` with the same
   params when a worker exits with `Done { success: false }`? The spec leaves this to the
   TUI layer — recommended answer: no auto-restart in Phase 1; expose `PendingLoop` to the
   TUI and let the user decide.

2. **Kill timeout configurability**: The 3-second SIGKILL timeout is hardcoded as
   `CANCEL_KILL_TIMEOUT`. Should it be pulled from `lazyjob.toml` under `[ralph]
   kill_timeout_secs`? Recommended: yes in Phase 2, via an `OrchestratorConfig` struct
   injected at construction time.

3. **Stderr in TUI**: Should the user be able to view `~/.lazyjob/logs/ralph-<id>.log`
   from the TUI (e.g., pressing `?` on a failed loop)? Recommended: yes in Phase 3 — add
   a `LogViewerPanel` that tails the log file with `tokio::fs::File::open()` +
   `AsyncBufReadExt::lines()`.

4. **Windows support**: `Child::kill()` on Windows sends `TerminateProcess` — this is
   correct. However `Stdio::piped()` for stderr requires the log file handle to be created
   before spawn and passed as `Into<Stdio>`. `tokio::fs::File` does not directly impl
   `Into<Stdio>` — use `.into_std().await` as shown in Phase 3, Step 3.2.

5. **Multiple subscribers at broadcast capacity**: If the broadcast channel fills (512 slots)
   because a subscriber is slow, late subscribers receive `RecvError::Lagged(n)`. The TUI
   event loop should handle this gracefully by logging a warning and continuing — lagged
   progress events are not data loss (worker writes to SQLite regardless).

6. **Worker binary path**: The spec says `std::env::current_exe()` — this works for the
   production binary but not in tests. The `RalphProcessManager::new()` constructor accepts
   an explicit `ralph_bin: PathBuf` so tests can pass the path to `tests/fake_worker`.

---

## Related Specs

- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — sits above this layer; uses `RalphProcessManager` via `LoopDispatch`
- [specs/09-tui-design-keybindings.md](./09-tui-design-keybindings.md) — TUI event loop that consumes `broadcast::Receiver<WorkerEvent>`
- [specs/10-application-workflow.md](./10-application-workflow.md) — triggers `LoopDispatch::dispatch_suggestion()` on stage transitions
- [specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md) — the only interactive `LoopType`; defines the bidirectional stdin protocol
- [specs/16-privacy-security.md](./16-privacy-security.md) — log files under `~/.lazyjob/logs/` must be considered sensitive; log rotation and 7-day deletion are part of the privacy posture
- [specs/XX-ralph-process-orphan-cleanup.md](./XX-ralph-process-orphan-cleanup.md) — extends `recover_pending()` with PID-based orphan detection
- [specs/XX-ralph-ipc-protocol.md](./XX-ralph-ipc-protocol.md) — alternative transport design (Unix domain sockets) for consideration if stdio proves limiting
