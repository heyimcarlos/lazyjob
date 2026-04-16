# Spec: Ralph Process Orphan Cleanup

## Context

When the TUI terminates Ralph subprocesses (via kill or crash), orphaned Ralph processes may remain. This spec addresses orphan detection, zombie handling, and resource cleanup.

## Motivation

- **Resource leaks**: Orphaned processes consume memory and CPU
- **Port conflicts**: Old Ralph processes holding socket/port bindings
- **State corruption**: Multiple Ralph processes for same loop type
- **User confusion**: "Why is ralph still running after I quit?"

## Design

### Orphan Detection

```rust
pub struct ProcessCleanupService {
    socket_path: PathBuf,
    pid_file_path: PathBuf,
    grace_period_secs: u64 = 5,
}

impl ProcessCleanupService {
    /// Check if a Ralph process is orphaned
    pub async fn is_orphaned(&self, pid: u32) -> Result<bool> {
        // Check if process exists
        if !process_exists(pid) {
            return Ok(true);  // Process is gone = orphaned (from our perspective)
        }

        // Check if process is zombie
        if is_zombie_process(pid) {
            return Ok(true);  // Zombie should be reaped by init/session leader
        }

        // Check if process is still responding to socket
        if self.is_socket_responding(pid).await? {
            return Ok(false);  // Process is alive and healthy
        }

        // Process exists but socket not responding = orphaned
        Ok(true)
    }

    /// Find all orphaned Ralph processes
    pub async fn find_orphans(&self) -> Result<Vec<OrphanInfo>> {
        let mut orphans = vec![];

        // Check PID file
        if let Some(stored_pid) = self.read_pid_file()? {
            if self.is_orphaned(stored_pid).await? {
                orphans.push(OrphanInfo {
                    pid: stored_pid,
                    reason: OrphanReason::NotResponding,
                    started_at: self.get_process_start_time(stored_pid)?,
                });
            }
        }

        // Scan for stale socket files
        if self.socket_path.exists() {
            // Try to connect - if fails, socket is stale
            if RalphClient::connect(&self.socket_path).await.is_err() {
                // Socket exists but no process listening = stale
                orphans.push(OrphanInfo {
                    pid: 0,  // Unknown
                    reason: OrphanReason::StaleSocket,
                    started_at: self.socket_mtime()?,
                });
            }
        }

        Ok(orphans)
    }
}

pub struct OrphanInfo {
    pub pid: u32,
    pub reason: OrphanReason,
    pub started_at: DateTime<Utc>,
}

pub enum OrphanReason {
    ProcessDead,
    ZombieProcess,
    NotResponding,
    StaleSocket,
}
```

### Process Group Cleanup

```rust
impl ProcessCleanupService {
    /// Kill entire process group (Ralph + any children)
    pub async fn kill_process_group(&self, pid: u32) -> Result<()> {
        // Get process group ID
        let pgid = get_pgid(pid)?;

        // Send SIGTERM to entire group
        signal::kill_pg(pgid, signal::SIGTERM)?;

        // Wait for graceful shutdown (with timeout)
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                // Force kill
                signal::kill_pg(pgid, signal::SIGKILL)?;
                break;
            }

            if !process_exists(pid) {
                break;  // Process exited
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }
}
```

### Startup Lock

```rust
pub struct StartupLock {
    lock_path: PathBuf,
}

impl StartupLock {
    /// Acquire exclusive lock for Ralph startup
    pub async fn acquire(&self) -> Result<StartupLockGuard> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.lock_path)?;

        // Non-blocking fcntl lock
        match fdlock::try_lock(&file) {
            Ok(_) => {
                // Write PID
                let pid = std::process::id();
                file.write_all(format!("{}", pid).as_bytes())?;
                file.flush()?;

                Ok(StartupLockGuard { file, lock_path: self.lock_path.clone() })
            }
            Err(_) => {
                // Check if existing process is still alive
                let existing_pid = self.read_pid()?;
                if self.is_process_alive(existing_pid).await? {
                    Err(RalphStartupError::AlreadyRunning { pid: existing_pid })
                } else {
                    // Stale lock file, remove and retry
                    std::fs::remove_file(&self.lock_path)?;
                    self.acquire().await
                }
            }
        }
    }
}

pub struct StartupLockGuard {
    file: File,
    lock_path: PathBuf,
}

impl Drop for StartupLockGuard {
    fn drop(&mut self) {
        // Release lock by removing file
        let _ = std::fs::remove_file(&self.lock_path);
    }
}
```

### Resource Cleanup

```rust
pub struct ResourceCleanup {
    temp_dirs: Vec<PathBuf>,
    log_files: Vec<PathBuf>,
}

impl ResourceCleanup {
    pub fn track_temp_dir(&mut self, path: PathBuf) {
        self.temp_dirs.push(path);
    }

    pub fn cleanup(&self) -> Result<CleanupReport> {
        let mut cleaned = vec![];
        let mut errors = vec![];

        // Clean temp directories
        for dir in &self.temp_dirs {
            match Self::rm_rf(dir) {
                Ok(_) => cleaned.push(dir.clone()),
                Err(e) => errors.push(CleanupError { path: dir.clone(), error: e }),
            }
        }

        // Rotate/clean old log files
        for log in &self.log_files {
            if let Ok(metadata) = std::fs::metadata(log) {
                if metadata.len() > 100 * 1024 * 1024 {  // > 100MB
                    // Truncate or compress
                    Self::rotate_log(log)?;
                }
            }
        }

        Ok(CleanupReport { cleaned, errors })
    }
}
```

### TUI Startup Sequence with Cleanup

```rust
pub struct RalphManager {
    cleanup: ProcessCleanupService,
}

impl RalphManager {
    pub async fn ensure_clean_startup(&mut self) -> Result<()> {
        // 1. Find and clean orphans from previous session
        let orphans = self.cleanup.find_orphans().await?;
        for orphan in &orphans {
            tracing::warn!(pid = orphan.pid, reason = ?orphan.reason, "Cleaning up orphan");
            if orphan.pid > 0 {
                self.cleanup.kill_process_group(orphan.pid).await?;
            }
            if orphan.reason == OrphanReason::StaleSocket {
                std::fs::remove_file(&self.cleanup.socket_path)?;
            }
        }

        // 2. Acquire startup lock
        let _lock = self.cleanup.acquire_startup_lock().await?;

        // 3. Spawn Ralph
        self.start_ralph_process().await?;

        Ok(())
    }
}
```

### Shutdown Sequence

```rust
impl RalphManager {
    pub async fn graceful_shutdown(&mut self) -> Result<()> {
        tracing::info!("Initiating graceful Ralph shutdown");

        // 1. Send shutdown command via IPC
        if let Some(client) = &self.client {
            client.shutdown().await?;
        }

        // 2. Wait for Ralph to finish current work (with timeout)
        let shutdown_timeout = Duration::from_secs(10);
        let deadline = Instant::now() + shutdown_timeout;

        while Instant::now() < deadline {
            if self.is_ralph_idle().await? {
                break;  // Ralph is idle, can exit cleanly
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 3. Force kill if still running
        if let Some(pid) = self.ralph_pid {
            self.cleanup.kill_process_group(pid).await?;
        }

        // 4. Cleanup resources
        self.cleanup.cleanup()?;

        // 5. Remove socket file
        let _ = std::fs::remove_file(&self.cleanup.socket_path);

        Ok(())
    }
}
```

## Implementation Notes

- Use `procfs` crate to inspect process state on Linux
- Use `sysinfo` crate for cross-platform process info
- Lock file uses `fcntl` for atomic locking
- Log rotation: gzip logs older than 7 days

## Open Questions

1. **User notification**: Should user be told "cleaned up 2 orphan processes"?
2. **Auto-cleanup interval**: Periodic cleanup while TUI is running?
3. **Signal handling**: What signals should Ralph respond to?

## Related Specs

- `XX-ralph-ipc-protocol.md` - IPC protocol
- `06-ralph-loop-integration.md` - Ralph lifecycle
- `XX-ralph-log-management.md` - Log file handling