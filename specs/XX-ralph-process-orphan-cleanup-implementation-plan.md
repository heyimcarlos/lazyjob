# Implementation Plan: Ralph Process Orphan Cleanup

## Status
Draft

## Related Spec
[specs/XX-ralph-process-orphan-cleanup.md](./XX-ralph-process-orphan-cleanup.md)

## Overview

When the LazyJob TUI exits abnormally (panic, SIGKILL, power loss) while Ralph worker
subprocesses are active, those subprocesses become orphaned — they may continue consuming
memory and CPU, hold partial DB write transactions open, or accumulate as zombies in the
process table. The existing `recover_pending()` method in `RalphProcessManager` handles the
SQLite side (marking orphaned `ralph_loop_runs` rows as `failed`), but it does not interact
with the OS to verify whether the processes are still alive or terminate them if they are.

This plan closes that gap. It extends `lazyjob-ralph` with three tightly-scoped components:
(1) a `ProcessCleanupService` that detects and terminates orphaned OS processes using PID
tracking in SQLite and the `sysinfo` crate for cross-platform process introspection;
(2) a `StartupLock` using advisory `fcntl` file locking (via `fs2`) that prevents two LazyJob
instances from managing the same Ralph process set; and (3) a `ResourceCleanup` helper that
deletes stale temp directories and rotates oversized log files on shutdown.

The design is OS-agnostic via `sysinfo` for process enumeration and `nix` for Unix signal
delivery. On non-Unix targets (Windows) the signal-based kill path degrades gracefully to
`Child::kill()`. All cleanup is opportunistic: failures are logged as warnings but never
propagate to the TUI as hard errors. The user sees a startup banner only when orphans were
actually killed.

## Prerequisites

### Must be implemented first
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — `RalphProcessManager`,
  `RalphError`, `ralph_loop_runs` schema, and `recover_pending()` must exist before this plan
  extends them. The `os_pid` column migration in this plan is **additive** — it must be applied
  on top of `001_ralph_loop_runs.sql`.
- `specs/XX-ralph-ipc-protocol.md` (if socket transport is adopted) — `StaleSocket`
  detection path in `find_orphans()` depends on the socket path convention defined there.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
sysinfo    = "0.30"
nix        = { version = "0.27", features = ["signal", "process"] }
fs2        = "0.4"
```

In `lazyjob-ralph/Cargo.toml`:

```toml
sysinfo.workspace    = true
nix.workspace        = true
fs2.workspace        = true
```

`sysinfo` is the primary process-info crate. `nix` is used only for `kill(2)` / `killpg(2)`
on Unix — it is `#[cfg(unix)]` gated. `fs2` provides `FileExt::try_lock_exclusive()`, which
maps to `fcntl(F_SETLK)` on Unix and `LockFileEx` on Windows.

---

## Architecture

### Crate Placement

All types live in `lazyjob-ralph`. This plan adds three new modules:
- `lazyjob-ralph/src/cleanup.rs` — `ProcessCleanupService`, `OrphanInfo`, `OrphanReason`,
  `ResourceCleanup`, `CleanupReport`
- `lazyjob-ralph/src/lock.rs` — `StartupLock`, `StartupLockGuard`, `LockError`
- `lazyjob-ralph/src/db.rs` — **extended** with `os_pid` column and PID CRUD helpers

`RalphProcessManager` in `process.rs` gains two new public methods: `ensure_clean_startup()`
and `graceful_shutdown()`. The TUI calls both — startup before any spawn, shutdown before
process exit.

### Core Types

```rust
// lazyjob-ralph/src/cleanup.rs

use std::path::PathBuf;
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use sysinfo::{Pid, Process, ProcessStatus, System};
use tracing::{info, warn, error};

/// All information about a detected orphan process.
#[derive(Debug, Clone)]
pub struct OrphanInfo {
    /// OS process ID. Zero means unknown (socket-only orphan).
    pub pid: u32,
    /// Why this process was classified as orphaned.
    pub reason: OrphanReason,
    /// When the process or socket file was created (best effort).
    pub started_at: Option<DateTime<Utc>>,
    /// The ralph_loop_runs UUID, if this orphan corresponds to a DB row.
    pub loop_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanReason {
    /// ralph_loop_runs row says 'running' but the stored PID no longer exists.
    ProcessDead,
    /// PID exists but its `ProcessStatus` is `Zombie`.
    ZombieProcess,
    /// PID exists and is alive, but the TUI is starting fresh — it was left
    /// from a prior session that crashed before calling graceful_shutdown().
    LeftFromCrashedSession,
    /// Unix socket file exists on disk but no process is listening on it.
    StaleSocket,
}

pub struct ProcessCleanupService {
    /// Path to the Unix domain socket used by Ralph (may not exist if using stdio).
    pub socket_path: Option<PathBuf>,
    /// Grace period before escalating SIGTERM → SIGKILL.
    pub grace_period: Duration,
}

impl ProcessCleanupService {
    pub fn new(socket_path: Option<PathBuf>) -> Self {
        Self {
            socket_path,
            grace_period: Duration::from_secs(5),
        }
    }
}

/// Summary of a single cleanup pass.
#[derive(Debug, Default)]
pub struct CleanupReport {
    pub orphans_killed: Vec<OrphanInfo>,
    pub stale_sockets_removed: u32,
    pub temp_dirs_removed: Vec<PathBuf>,
    pub log_files_rotated: Vec<PathBuf>,
    pub errors: Vec<String>,
}

/// Tracks temp dirs and log files for cleanup on shutdown.
pub struct ResourceCleanup {
    temp_dirs: Vec<PathBuf>,
    log_files: Vec<PathBuf>,
    log_max_bytes: u64,
}

impl ResourceCleanup {
    pub fn new() -> Self {
        Self {
            temp_dirs: Vec::new(),
            log_files: Vec::new(),
            log_max_bytes: 100 * 1024 * 1024, // 100 MiB
        }
    }

    pub fn track_temp_dir(&mut self, path: PathBuf) {
        self.temp_dirs.push(path);
    }

    pub fn track_log_file(&mut self, path: PathBuf) {
        self.log_files.push(path);
    }
}
```

```rust
// lazyjob-ralph/src/lock.rs

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use fs2::FileExt;
use thiserror::Error;

/// Advisory lock preventing two LazyJob processes from managing the same Ralph set.
pub struct StartupLock {
    pub lock_path: PathBuf,
}

/// RAII guard: releases the lock on drop.
pub struct StartupLockGuard {
    /// Keep the file open — lock is held for its lifetime.
    _file: File,
    lock_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum LockError {
    #[error("another LazyJob instance is running (PID {pid})")]
    AlreadyRunning { pid: u32 },

    #[error("failed to open lock file {path}: {source}")]
    IoError { path: PathBuf, source: std::io::Error },

    #[error("stale lock file could not be removed: {0}")]
    StaleLockRemovalFailed(std::io::Error),
}
```

### Trait Definitions

```rust
// In lazyjob-ralph/src/cleanup.rs — the core cleanup interface

impl ProcessCleanupService {
    /// Return all orphaned processes based on the provided DB PID records.
    /// Does NOT kill anything — pure detection.
    pub fn find_orphans(
        &self,
        running_rows: &[(uuid::Uuid, Option<u32>)], // (loop_id, stored_pid)
    ) -> Vec<OrphanInfo>;

    /// Kill a single process group via SIGTERM then SIGKILL after grace_period.
    /// On non-Unix, falls back to sysinfo::Process::kill().
    pub async fn kill_process_group(&self, pid: u32) -> Result<(), CleanupError>;

    /// Remove a stale socket file safely (check it's a socket before unlinking).
    pub fn remove_stale_socket(&self) -> Result<(), CleanupError>;
}

impl ResourceCleanup {
    /// Remove tracked temp dirs and rotate oversized log files.
    /// All errors are collected into CleanupReport.errors — none are propagated.
    pub fn run(&self) -> CleanupReport;
}

impl StartupLock {
    /// Attempt to acquire the lock. Returns LockError::AlreadyRunning if held
    /// by a live process. Removes and retries exactly once if the holding process
    /// is dead (stale lock).
    pub fn try_acquire(&self) -> Result<StartupLockGuard, LockError>;
}
```

### SQLite Schema

Migration `002_ralph_loop_runs_pid.sql` (applied on top of `001_ralph_loop_runs.sql`):

```sql
-- lazyjob-ralph/migrations/002_ralph_loop_runs_pid.sql

-- Add OS PID column for orphan detection.
-- NULL means not yet spawned or PID unknown (pre-migration rows).
ALTER TABLE ralph_loop_runs
    ADD COLUMN os_pid INTEGER;

-- Index for the startup scan: find all running rows with a non-null PID quickly.
CREATE INDEX IF NOT EXISTS idx_ralph_loop_runs_running_pid
    ON ralph_loop_runs (os_pid)
    WHERE status = 'running' AND os_pid IS NOT NULL;
```

`os_pid` is written immediately after `tokio::process::Command::spawn()` returns a `Child`
with a known PID. It is set to NULL on any terminal transition (`done`, `failed`, `cancelled`)
so the startup scan only sees truly-running-or-stuck rows.

### Module Structure

```
lazyjob-ralph/
  src/
    lib.rs
    error.rs          # RalphError (extended with orphan/lock variants)
    protocol.rs       # WorkerCommand, WorkerEvent (unchanged)
    codec.rs          # NdjsonCodec (unchanged)
    process.rs        # RalphProcessManager — gains ensure_clean_startup(), graceful_shutdown()
    cleanup.rs        # ProcessCleanupService, OrphanInfo, ResourceCleanup, CleanupReport
    lock.rs           # StartupLock, StartupLockGuard, LockError
    log_manager.rs    # LogManager — gains rotate_old_logs()
    db.rs             # Extended: write_os_pid(), clear_os_pid(), find_running_with_pid()
  migrations/
    001_ralph_loop_runs.sql          # From subprocess protocol plan
    002_ralph_loop_runs_pid.sql      # This plan: ADD COLUMN os_pid
```

---

## Implementation Phases

### Phase 1 — Database Extension and PID Tracking (MVP)

**Goal**: Ensure every spawned worker records its OS PID in SQLite so orphan detection is
possible on next startup even after a TUI crash.

#### Step 1.1 — Migration file

Write `lazyjob-ralph/migrations/002_ralph_loop_runs_pid.sql` (DDL in Schema section above).

**Verification**: `sqlx migrate run` on a database with the existing schema succeeds. Query
`PRAGMA table_info(ralph_loop_runs)` shows an `os_pid` column.

#### Step 1.2 — DB helpers in `db.rs`

Add three functions alongside the existing `insert_run()` / `update_run_terminal()`:

```rust
/// Called immediately after Child::id() is known. Idempotent (os_pid can only
/// be set once per loop run; a second call with a different PID is a no-op that
/// logs a warning).
pub async fn write_os_pid(
    pool: &SqlitePool,
    loop_id: Uuid,
    pid: u32,
) -> Result<(), RalphError> {
    sqlx::query!(
        "UPDATE ralph_loop_runs SET os_pid = ? WHERE id = ? AND os_pid IS NULL",
        pid,
        loop_id.to_string(),
    )
    .execute(pool)
    .await
    .map_err(RalphError::Db)?;
    Ok(())
}

/// Called at terminal transition. Clears PID so the startup scan skips this row.
pub async fn clear_os_pid(pool: &SqlitePool, loop_id: Uuid) -> Result<(), RalphError> {
    sqlx::query!(
        "UPDATE ralph_loop_runs SET os_pid = NULL WHERE id = ?",
        loop_id.to_string(),
    )
    .execute(pool)
    .await
    .map_err(RalphError::Db)?;
    Ok(())
}

/// Returns all rows in status='running' with a non-null os_pid.
/// Used by ensure_clean_startup() for orphan detection.
pub async fn find_running_with_pid(
    pool: &SqlitePool,
) -> Result<Vec<(Uuid, u32)>, RalphError> {
    let rows = sqlx::query!(
        "SELECT id, os_pid FROM ralph_loop_runs
         WHERE status = 'running' AND os_pid IS NOT NULL"
    )
    .fetch_all(pool)
    .await
    .map_err(RalphError::Db)?;

    rows.into_iter()
        .map(|r| {
            let id = Uuid::parse_str(&r.id)
                .map_err(|e| RalphError::InvalidUuid(e.to_string()))?;
            let pid = r.os_pid.expect("guaranteed non-null by WHERE clause") as u32;
            Ok((id, pid))
        })
        .collect()
}
```

#### Step 1.3 — Wire into `process.rs`

In `RalphProcessManager::spawn()`, after `tokio::process::Command::spawn()` returns `child`:

```rust
let pid = child.id().expect("process has not yet been waited on");
db::write_os_pid(&self.pool, loop_id, pid).await?;
```

In `update_run_terminal()` (called by `stdout_reader_task` on Done/Error/Cancelled events):

```rust
db::clear_os_pid(&self.pool, loop_id).await?;
```

**Verification**: Write an integration test using `tests/fake_worker` that spawns, queries
`ralph_loop_runs`, and asserts `os_pid IS NOT NULL` while running and `NULL` after Done.

---

### Phase 2 — ProcessCleanupService (Orphan Detection + Kill)

**Goal**: Implement cross-platform process detection and Unix signal-based termination.

#### Step 2.1 — `cleanup.rs`: `find_orphans()`

```rust
use sysinfo::{Pid, ProcessStatus, System, UpdateKind};

impl ProcessCleanupService {
    /// Pure detection: does not kill anything.
    /// `running_rows` comes from `db::find_running_with_pid()`.
    pub fn find_orphans(
        &self,
        running_rows: &[(Uuid, u32)],
    ) -> Vec<OrphanInfo> {
        // Refresh sysinfo once for all PIDs in this call.
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new()
                .with_status(UpdateKind::Always),
        );

        let mut orphans = Vec::new();

        for &(loop_id, pid) in running_rows {
            let sysinfo_pid = Pid::from_u32(pid);
            match sys.process(sysinfo_pid) {
                None => {
                    // PID not in process table at all — process exited without us noticing.
                    orphans.push(OrphanInfo {
                        pid,
                        reason: OrphanReason::ProcessDead,
                        started_at: None,
                        loop_id: Some(loop_id),
                    });
                }
                Some(proc) if matches!(proc.status(), ProcessStatus::Zombie) => {
                    orphans.push(OrphanInfo {
                        pid,
                        reason: OrphanReason::ZombieProcess,
                        started_at: None,
                        loop_id: Some(loop_id),
                    });
                }
                Some(_) => {
                    // Process is alive — it was left over from a crashed session.
                    orphans.push(OrphanInfo {
                        pid,
                        reason: OrphanReason::LeftFromCrashedSession,
                        started_at: None,
                        loop_id: Some(loop_id),
                    });
                }
            }
        }

        // Also check for a stale socket file if configured.
        if let Some(ref socket) = self.socket_path {
            if socket.exists() && !self.can_connect_socket(socket) {
                orphans.push(OrphanInfo {
                    pid: 0,
                    reason: OrphanReason::StaleSocket,
                    started_at: Self::file_mtime(socket),
                    loop_id: None,
                });
            }
        }

        orphans
    }

    fn can_connect_socket(&self, path: &std::path::Path) -> bool {
        // Sync connect attempt — orphan detection runs before the tokio runtime
        // is fully started, so we use std::os::unix::net::UnixStream.
        #[cfg(unix)]
        {
            std::os::unix::net::UnixStream::connect(path).is_ok()
        }
        #[cfg(not(unix))]
        { false }
    }

    fn file_mtime(path: &std::path::Path) -> Option<DateTime<Utc>> {
        let meta = std::fs::metadata(path).ok()?;
        let systime = meta.modified().ok()?;
        Some(DateTime::<Utc>::from(systime))
    }
}
```

**Key design decision**: `sysinfo::System::refresh_processes_specifics()` with
`ProcessesToUpdate::All` and only `with_status()` minimises memory overhead — we don't need
CPU times, memory stats, or command lines for orphan detection.

#### Step 2.2 — `cleanup.rs`: `kill_process_group()`

```rust
impl ProcessCleanupService {
    pub async fn kill_process_group(&self, pid: u32) -> Result<(), CleanupError> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid as NixPid;

            // Attempt to get the process group ID.
            let pgid = nix::unistd::getpgid(Some(NixPid::from_raw(pid as i32)))
                .map_err(|e| CleanupError::Signal(format!("getpgid({pid}): {e}")))?;

            // Send SIGTERM to the entire process group.
            let _ = signal::killpg(pgid, Signal::SIGTERM);
            info!(pid, pgid = pgid.as_raw(), "sent SIGTERM to process group");

            // Poll for exit during grace period.
            let deadline = Instant::now() + self.grace_period;
            while Instant::now() < deadline {
                // Check if lead process is gone.
                let mut sys = System::new();
                sys.refresh_process(Pid::from_u32(pid));
                if sys.process(Pid::from_u32(pid)).is_none() {
                    info!(pid, "process group exited after SIGTERM");
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            // Escalate to SIGKILL.
            warn!(pid, "grace period expired, escalating to SIGKILL");
            let _ = signal::killpg(pgid, Signal::SIGKILL);
            // Brief wait for kernel to reap.
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(())
        }

        #[cfg(not(unix))]
        {
            // On Windows, use sysinfo to kill the process.
            let mut sys = System::new();
            sys.refresh_process(Pid::from_u32(pid));
            if let Some(proc) = sys.process(Pid::from_u32(pid)) {
                proc.kill();
            }
            Ok(())
        }
    }

    pub fn remove_stale_socket(&self) -> Result<(), CleanupError> {
        let Some(ref path) = self.socket_path else { return Ok(()) };
        if !path.exists() { return Ok(()) }

        // Verify it's actually a socket before unlinking to avoid removing
        // a regular file that happens to share the path (paranoia check).
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let meta = std::fs::metadata(path)
                .map_err(|e| CleanupError::Io(e))?;
            if !meta.file_type().is_socket() {
                warn!(path = %path.display(), "socket path exists but is not a socket, skipping removal");
                return Ok(());
            }
        }

        std::fs::remove_file(path)
            .map_err(CleanupError::Io)?;
        info!(path = %path.display(), "removed stale socket");
        Ok(())
    }
}
```

**Platform note**: `nix::unistd::getpgid` is only available on Unix. The `#[cfg(unix)]`
guard prevents compilation errors on Windows without a separate build feature flag.

#### Step 2.3 — Error type

```rust
// In lazyjob-ralph/src/error.rs — extend RalphError

#[derive(Debug, Error)]
pub enum CleanupError {
    #[error("signal delivery failed: {0}")]
    Signal(String),

    #[error("I/O error during cleanup: {0}")]
    Io(#[from] std::io::Error),

    #[error("sysinfo query failed: {0}")]
    SysInfo(String),
}
```

**Verification**: Unit test `test_find_orphans_dead_pid` inserts a fake `(loop_id, 999999)`
pair (PID 999999 is extremely unlikely to exist) and asserts `find_orphans()` returns one
`OrphanReason::ProcessDead` entry. Use `std::process::id()` for a known-live PID in
`test_find_orphans_live_pid` and assert `LeftFromCrashedSession` is returned.

---

### Phase 3 — StartupLock

**Goal**: Prevent two LazyJob instances from managing the same process set, and provide a
PID file that records the TUI's own process ID for external tooling.

#### Step 3.1 — `lock.rs` implementation

```rust
use fs2::FileExt;

impl StartupLock {
    /// Default lock path: `~/.lazyjob/lazyjob.lock`.
    pub fn default_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from(".lazyjob"))
            .join("lazyjob")
            .join("lazyjob.lock")
    }

    /// Try to acquire the lock once. Does not retry on stale lock automatically —
    /// caller must call `try_acquire_with_stale_cleanup()` for the full behavior.
    fn try_acquire_inner(&self) -> Result<StartupLockGuard, LockError> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(|e| LockError::IoError { path: self.lock_path.clone(), source: e })?;

        file.try_lock_exclusive()
            .map_err(|_| {
                // Read the PID from the file to provide a useful error message.
                let pid = Self::read_pid_from_file(&file).unwrap_or(0);
                LockError::AlreadyRunning { pid }
            })?;

        // Write our own PID into the lock file for diagnostics.
        let our_pid = std::process::id();
        use std::io::Write;
        let mut f = &file;
        let _ = f.write_all(our_pid.to_string().as_bytes());
        let _ = f.flush();

        Ok(StartupLockGuard { _file: file, lock_path: self.lock_path.clone() })
    }

    /// Primary entry point: acquire, or if locked by a dead process, clean up and retry once.
    pub fn try_acquire(&self) -> Result<StartupLockGuard, LockError> {
        match self.try_acquire_inner() {
            Ok(guard) => Ok(guard),
            Err(LockError::AlreadyRunning { pid }) => {
                // Check if the holder is still alive.
                if pid > 0 && Self::is_process_alive(pid) {
                    return Err(LockError::AlreadyRunning { pid });
                }
                // Stale lock: remove and retry once.
                warn!(pid, "removing stale lock file from dead process");
                std::fs::remove_file(&self.lock_path)
                    .map_err(LockError::StaleLockRemovalFailed)?;
                self.try_acquire_inner()
            }
            Err(other) => Err(other),
        }
    }

    fn is_process_alive(pid: u32) -> bool {
        let mut sys = System::new();
        sys.refresh_process(Pid::from_u32(pid));
        sys.process(Pid::from_u32(pid)).is_some()
    }

    fn read_pid_from_file(file: &File) -> Option<u32> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = file;
        let _ = f.seek(SeekFrom::Start(0));
        let mut buf = String::new();
        f.read_to_string(&mut buf).ok()?;
        buf.trim().parse::<u32>().ok()
    }
}

impl Drop for StartupLockGuard {
    fn drop(&mut self) {
        // fs2 releases the lock automatically when the File is dropped.
        // We also remove the file to leave a clean state.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}
```

**Verification**: Spawn two threads, each calling `StartupLock::try_acquire()` on the same
path concurrently. Assert exactly one succeeds and one returns `LockError::AlreadyRunning`.

#### Step 3.2 — Handle stale lock after TUI crash

The guard's `Drop` impl removes the lock file on normal exit. After a crash (SIGKILL), the
lock file remains but holds the crashed PID. On next startup, `try_acquire()` reads that PID,
detects the process is dead via `sysinfo`, removes the stale file, and re-acquires. This
loop runs at most twice (one removal + one retry) to avoid infinite recursion.

---

### Phase 4 — ResourceCleanup and Log Rotation

**Goal**: Delete stale temp directories on shutdown and rotate Ralph log files that exceed
the size threshold.

#### Step 4.1 — `cleanup.rs`: `ResourceCleanup::run()`

```rust
impl ResourceCleanup {
    pub fn run(&self) -> CleanupReport {
        let mut report = CleanupReport::default();

        for dir in &self.temp_dirs {
            match std::fs::remove_dir_all(dir) {
                Ok(_) => {
                    info!(path = %dir.display(), "removed temp dir");
                    report.temp_dirs_removed.push(dir.clone());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Already gone — not an error.
                }
                Err(e) => {
                    warn!(path = %dir.display(), error = %e, "failed to remove temp dir");
                    report.errors.push(format!("temp dir {}: {e}", dir.display()));
                }
            }
        }

        for log in &self.log_files {
            match std::fs::metadata(log) {
                Ok(meta) if meta.len() > self.log_max_bytes => {
                    match Self::rotate_log(log) {
                        Ok(_) => {
                            info!(path = %log.display(), size = meta.len(), "rotated log file");
                            report.log_files_rotated.push(log.clone());
                        }
                        Err(e) => {
                            warn!(path = %log.display(), error = %e, "failed to rotate log");
                            report.errors.push(format!("rotate {}: {e}", log.display()));
                        }
                    }
                }
                _ => {}
            }
        }

        report
    }

    /// Rotate by truncating the file (simple) — gzip deferred to Phase 5.
    fn rotate_log(path: &std::path::Path) -> std::io::Result<()> {
        // Rename to .1, truncate original.
        let rotated = path.with_extension("log.1");
        std::fs::rename(path, &rotated)?;
        // Create a new empty file at the original path.
        std::fs::File::create(path)?;
        Ok(())
    }
}
```

Log rotation is intentionally simple in Phase 1: rename `ralph.log` → `ralph.log.1`, create
a new empty `ralph.log`. Phase 5 adds gzip compression and a 7-day retention sweep via
`log_manager.rs`.

**Verification**: Create a 110 MiB temp file, register it with `ResourceCleanup`, call
`run()`, assert `report.log_files_rotated` has one entry and the original file is empty.

---

### Phase 5 — `ensure_clean_startup()` and `graceful_shutdown()`

**Goal**: Wire all the above components into `RalphProcessManager` so the TUI calls exactly
two methods at the right lifecycle points.

#### Step 5.1 — `process.rs`: `ensure_clean_startup()`

```rust
impl RalphProcessManager {
    /// Called once at TUI startup, before any `spawn()`.
    ///
    /// 1. Try to acquire the startup lock.
    /// 2. Find all 'running' DB rows with a PID.
    /// 3. Classify them as orphans via ProcessCleanupService.
    /// 4. For each orphan: kill if alive, mark DB row 'failed', emit Error event.
    /// 5. Remove stale socket file if present.
    /// 6. Return the list of killed orphans so the TUI can optionally notify the user.
    pub async fn ensure_clean_startup(
        &mut self,
        lock: &StartupLock,
        cleanup: &ProcessCleanupService,
    ) -> Result<Vec<OrphanInfo>, RalphError> {
        // 1. Acquire startup lock (returns immediately; stale handling is inside try_acquire).
        let _guard = lock.try_acquire()
            .map_err(|e| RalphError::StartupLock(e.to_string()))?;
        // Store guard in self so it's held for the session.
        // (RalphProcessManager gains an Option<StartupLockGuard> field.)

        // 2. Find all DB rows with a stored PID.
        let running = db::find_running_with_pid(&self.pool).await?;
        if running.is_empty() {
            return Ok(vec![]);
        }

        // 3. Classify orphans.
        let running_pairs: Vec<(Uuid, u32)> = running.clone();
        let orphans = cleanup.find_orphans(&running_pairs);

        let mut killed = Vec::new();
        for orphan in &orphans {
            match orphan.reason {
                OrphanReason::ProcessDead | OrphanReason::ZombieProcess => {
                    // Process is already gone — just clean up DB.
                }
                OrphanReason::LeftFromCrashedSession => {
                    // 4. Kill the process group.
                    if orphan.pid > 0 {
                        if let Err(e) = cleanup.kill_process_group(orphan.pid).await {
                            warn!(pid = orphan.pid, error = %e, "kill failed during orphan cleanup");
                        }
                    }
                }
                OrphanReason::StaleSocket => {
                    cleanup.remove_stale_socket().ok();
                    continue; // No DB row to update.
                }
            }

            // Mark DB row 'failed'.
            if let Some(loop_id) = orphan.loop_id {
                db::update_run_terminal(
                    &self.pool,
                    loop_id,
                    "failed",
                    Some("orphan cleanup on startup"),
                ).await.ok(); // Non-fatal: best effort.

                // Emit synthetic WorkerEvent::Error so any subscribers know.
                let _ = self.event_tx.send(WorkerEvent::Error {
                    loop_id,
                    message: "process orphaned from previous session".to_string(),
                });
            }

            killed.push(orphan.clone());
        }

        // 5. Remove stale socket if present.
        cleanup.remove_stale_socket().ok();

        info!(
            orphans_killed = killed.len(),
            "startup orphan cleanup complete"
        );
        Ok(killed)
    }

    /// Called by the TUI on exit (Ctrl+C, `:q`, or SIGTERM handler).
    ///
    /// 1. Cancel all active workers (sends WorkerCommand::Cancel, waits for ack or timeout).
    /// 2. Kill any that don't comply within shutdown_timeout.
    /// 3. Run ResourceCleanup.
    /// 4. Release the startup lock (by dropping it).
    pub async fn graceful_shutdown(
        &mut self,
        cleanup: &ResourceCleanup,
        shutdown_timeout: Duration,
    ) -> CleanupReport {
        let mut report = CleanupReport::default();

        // Cancel all active workers.
        let loop_ids: Vec<Uuid> = self.active.keys().copied().collect();
        for loop_id in loop_ids {
            if let Err(e) = self.cancel(loop_id).await {
                warn!(loop_id = %loop_id, error = %e, "cancel during shutdown failed");
                // Try to force-kill via stored PID.
                if let Some(worker) = self.active.get(&loop_id) {
                    if let Some(pid) = worker.os_pid {
                        let _ = ProcessCleanupService::new(None)
                            .kill_process_group(pid)
                            .await;
                    }
                }
            }
        }

        // Wait for active workers to drain, up to shutdown_timeout.
        let deadline = Instant::now() + shutdown_timeout;
        while !self.active.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.reap_dead_workers();
        }

        // Force-kill anything still alive after timeout.
        let stragglers: Vec<(Uuid, u32)> = self.active.values()
            .filter_map(|w| w.os_pid.map(|pid| (w.loop_id, pid)))
            .collect();
        for (loop_id, pid) in stragglers {
            warn!(loop_id = %loop_id, pid, "force-killing straggler on shutdown");
            let _ = ProcessCleanupService::new(None).kill_process_group(pid).await;
        }

        // Run resource cleanup.
        report = cleanup.run();

        report
    }
}
```

**`ActiveWorker` extension**: Add an `os_pid: Option<u32>` field. Populated in `spawn()`
immediately after `child.id()` is available.

#### Step 5.2 — TUI wiring in `lazyjob-tui/src/app.rs`

```rust
// In AppState::new() / TUI startup:
let lock = StartupLock { lock_path: StartupLock::default_path() };
let cleanup_svc = ProcessCleanupService::new(socket_path);
let killed_orphans = ralph_manager.ensure_clean_startup(&lock, &cleanup_svc).await?;

if !killed_orphans.is_empty() {
    // Show a one-time banner: "Cleaned up N orphan Ralph process(es) from previous session."
    app_state.startup_banner = Some(format!(
        "Cleaned up {} orphan process(es) from previous session.",
        killed_orphans.len()
    ));
}

// On exit:
let _report = ralph_manager.graceful_shutdown(&resource_cleanup, Duration::from_secs(10)).await;
```

**TUI startup banner**: `AppState` gains an `Option<String> startup_banner` field. The event
loop renders it as a yellow `Paragraph` at the top of the screen for the first 5 seconds
(cleared on any keypress or after the timeout). This is the only user-visible output from
orphan cleanup.

**Verification**: Integration test:
1. Spawn a fake worker that never exits.
2. Simulate a TUI crash (drop `RalphProcessManager` without calling `graceful_shutdown()`).
3. On a new `RalphProcessManager::new()` + `ensure_clean_startup()`, assert the fake worker
   is gone (sysinfo says PID dead) and its DB row status is `'failed'`.

---

## Key Crate APIs

- `sysinfo::System::refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new().with_status(UpdateKind::Always))` — minimal refresh for process status check
- `sysinfo::System::process(Pid) -> Option<&Process>` — O(1) lookup after refresh
- `sysinfo::Process::status() -> ProcessStatus` — returns `ProcessStatus::Zombie` on Linux for zombie processes
- `nix::sys::signal::killpg(pgid: Pid, sig: Signal) -> Result<()>` — sends signal to process group on Unix
- `nix::unistd::getpgid(pid: Option<Pid>) -> Result<Pid>` — gets process group ID
- `fs2::FileExt::try_lock_exclusive(&self) -> Result<()>` — non-blocking advisory lock
- `fs2::FileExt::unlock(&self) -> Result<()>` — releases the lock (called by `Drop` on `File`)
- `std::fs::File::write_all(buf: &[u8]) -> Result<()>` — write PID to lock file
- `std::os::unix::net::UnixStream::connect(path: &Path) -> Result<UnixStream>` — sync socket probe for stale socket detection
- `tokio::time::sleep(Duration) -> impl Future` — polling loop during SIGTERM grace period
- `dirs::data_local_dir() -> Option<PathBuf>` — cross-platform `~/.local/share` on Linux, `~/Library/Application Support` on macOS

---

## Error Handling

```rust
// Extend RalphError in error.rs:

#[derive(Debug, Error)]
pub enum RalphError {
    // ... existing variants ...

    #[error("startup lock acquisition failed: {0}")]
    StartupLock(String),

    #[error("orphan cleanup signal failed: {0}")]
    OrphanKill(String),

    #[error("startup lock already held by PID {pid}")]
    AnotherInstanceRunning { pid: u32 },
}
```

**Propagation rules**:
- `LockError::AlreadyRunning` is converted to `RalphError::AnotherInstanceRunning` and
  propagated to the TUI, which shows a hard error dialog: "Another LazyJob instance is
  running (PID X). Close it before starting a new session." The TUI exits cleanly.
- `CleanupError::Signal` (kill failure) is demoted to a `tracing::warn!` — orphan cleanup
  failures must never crash the TUI startup sequence.
- `CleanupError::Io` for socket or temp file removal is also `tracing::warn!` — non-fatal.
- All `graceful_shutdown()` errors are collected into `CleanupReport.errors`, never
  propagated — the TUI is exiting anyway.

---

## Testing Strategy

### Unit tests (in `lazyjob-ralph/src/cleanup.rs` and `lock.rs`)

```rust
#[test]
fn test_find_orphans_dead_pid() {
    // PID 999999 is vanishingly unlikely to exist.
    let svc = ProcessCleanupService::new(None);
    let rows = vec![(Uuid::new_v4(), 999_999u32)];
    let orphans = svc.find_orphans(&rows);
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].reason, OrphanReason::ProcessDead);
}

#[test]
fn test_find_orphans_live_pid() {
    let svc = ProcessCleanupService::new(None);
    let own_pid = std::process::id();
    let rows = vec![(Uuid::new_v4(), own_pid)];
    let orphans = svc.find_orphans(&rows);
    assert_eq!(orphans.len(), 1);
    // Our own process is alive — classified as LeftFromCrashedSession.
    assert_eq!(orphans[0].reason, OrphanReason::LeftFromCrashedSession);
}

#[test]
fn test_startup_lock_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.lock");
    let lock = StartupLock { lock_path: path };
    let _guard1 = lock.try_acquire().expect("first acquire should succeed");
    let err = lock.try_acquire().expect_err("second acquire should fail");
    assert!(matches!(err, LockError::AlreadyRunning { .. }));
}

#[test]
fn test_startup_lock_stale_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stale.lock");
    // Write a stale PID (dead process).
    std::fs::write(&path, "999999").unwrap();
    let lock = StartupLock { lock_path: path };
    // Should succeed: stale lock is detected and removed.
    let _guard = lock.try_acquire().expect("should clean stale lock and succeed");
}

#[test]
fn test_resource_cleanup_removes_temp_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ralph_tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut cleanup = ResourceCleanup::new();
    cleanup.track_temp_dir(dir.clone());
    let report = cleanup.run();
    assert_eq!(report.temp_dirs_removed.len(), 1);
    assert!(!dir.exists());
}
```

### Integration tests

```rust
// lazyjob-ralph/tests/orphan_cleanup_integration.rs

#[tokio::test]
async fn test_ensure_clean_startup_kills_orphan() {
    let pool = setup_test_pool().await; // In-memory SQLite with migrations applied.
    let fake_worker_path = PathBuf::from(env!("CARGO_BIN_EXE_fake_worker"));

    // Build a manager and spawn a worker.
    let mut mgr = RalphProcessManager::new(fake_worker_path.clone(), pool.clone());
    let loop_id = mgr.spawn(LoopType::JobDiscovery, serde_json::json!({})).await.unwrap();

    // Verify PID was written.
    let running = db::find_running_with_pid(&pool).await.unwrap();
    assert_eq!(running.len(), 1);
    let (_, pid) = running[0];

    // Simulate TUI crash: drop manager without graceful_shutdown.
    drop(mgr);

    // New session: ensure_clean_startup should kill the orphan.
    let tmp = tempfile::tempdir().unwrap();
    let lock = StartupLock { lock_path: tmp.path().join("lazyjob.lock") };
    let cleanup_svc = ProcessCleanupService::new(None);
    let mut mgr2 = RalphProcessManager::new(fake_worker_path, pool.clone());
    let killed = mgr2.ensure_clean_startup(&lock, &cleanup_svc).await.unwrap();

    assert_eq!(killed.len(), 1);

    // Verify the process is gone.
    let mut sys = System::new();
    sys.refresh_process(Pid::from_u32(pid));
    assert!(sys.process(Pid::from_u32(pid)).is_none());

    // Verify DB row is marked failed.
    let row = sqlx::query!("SELECT status FROM ralph_loop_runs WHERE id = ?", loop_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.status, "failed");
}
```

The `fake_worker` binary (already referenced in the subprocess protocol plan) stays alive
indefinitely unless explicitly killed — suitable for orphan testing without modification.

### TUI tests

No ratatui widget tests are required for this plan — the startup banner is a simple
`Paragraph` rendered from `AppState::startup_banner`. Verify it appears by checking that
`app_state.startup_banner` is `Some(...)` after `ensure_clean_startup()` returns orphans.

---

## Open Questions

1. **User notification detail level**: The spec asks "should user be told about cleaned up
   orphans?" This plan shows a startup banner. Open question: should the banner include loop
   type names (e.g., "Cleaned up job_discovery, resume_tailor") or just a count? Recommendation:
   count only in MVP; loop type names in Phase 5.

2. **Periodic cleanup while TUI is running**: The spec asks about an auto-cleanup interval.
   Recommendation: no periodic scan in MVP. The existing `reap_dead_workers()` (called from
   the 60fps tick) already handles workers that exit normally. Orphan cleanup is only needed
   at startup after a crash — periodic scans would add sysinfo overhead on every tick.

3. **Signal handling in Ralph workers**: What signals should Ralph workers respond to? The
   subprocess protocol plan sends `WorkerCommand::Cancel` over stdin as the primary
   cancellation mechanism, and only escalates to SIGTERM/SIGKILL on timeout. Workers should
   install a `signal_hook::flag` SIGTERM handler that sets an `AtomicBool` checked between
   LLM calls — this is not defined in the current worker spec and should be added to
   `agentic-ralph-subprocess-protocol.md` as an Open Question follow-up.

4. **Cross-platform PID uniqueness**: On Linux, PIDs are recycled. A very narrow race exists
   where a new unrelated process gets the same PID as a dead Ralph worker between the DB row
   persisting and the next startup scan. Mitigation: record `started_at` in `ralph_loop_runs`
   and compare it to `sysinfo::Process::start_time()` — if the process started after the DB
   row's `started_at`, it's a different process. Deferred to Phase 5 (very rare condition).

5. **`dirs` crate vs. hardcoded `~/.lazyjob`**: The lock file path uses `dirs::data_local_dir()`
   for cross-platform support, but the rest of LazyJob uses `~/.lazyjob` directly. Align with
   whichever convention is standardized in the config module when it is implemented.

---

## Related Specs

- [specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) — defines `RalphProcessManager`, `ralph_loop_runs`, and `recover_pending()` that this plan extends
- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — `LoopDispatch` is the caller of `spawn()`; should also call `ensure_clean_startup()` before the first dispatch
- [specs/XX-ralph-ipc-protocol.md](./XX-ralph-ipc-protocol.md) — if socket transport is adopted, the `socket_path` in `ProcessCleanupService` connects to the socket path convention defined there
- [specs/16-privacy-security.md](./16-privacy-security.md) — log files under `~/.lazyjob/logs/` are considered sensitive; log rotation policy (7-day retention, gzip) defined in `ResourceCleanup` Phase 5 aligns with the privacy spec's data minimization requirements
- [specs/06-ralph-loop-integration-implementation-plan.md](./06-ralph-loop-integration-implementation-plan.md) — higher-level Ralph lifecycle; `ensure_clean_startup()` and `graceful_shutdown()` are the startup/shutdown hooks described there
