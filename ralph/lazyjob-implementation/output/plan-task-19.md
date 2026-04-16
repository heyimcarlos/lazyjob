# Plan: Task 19 — ralph-process-manager

## Files to create/modify

1. `crates/lazyjob-ralph/src/error.rs` — add `Io(#[from] std::io::Error)` and `NotFound(String)`
2. `crates/lazyjob-ralph/src/process_manager.rs` — NEW: full implementation
3. `crates/lazyjob-ralph/src/lib.rs` — add `pub mod process_manager`, re-export `RunId` + `RalphProcessManager`

## Types to define

### `RunId`
- Newtype around `Uuid`
- Derives: `Debug, Clone, Copy, PartialEq, Eq, Hash`
- Methods: `new() -> Self`, `as_uuid() -> &Uuid`
- Impls: `Default`, `Display`

### `ProcessHandle` (private)
- `child: tokio::process::Child`
- `stdin: tokio::process::ChildStdin`

### `RalphProcessManager`
- `binary_path: PathBuf`
- `running: HashMap<RunId, ProcessHandle>`
- `event_tx: broadcast::Sender<(RunId, WorkerEvent)>`
- Methods:
  - `new() -> Self` — uses `current_exe()`
  - `with_binary(PathBuf) -> Self` — for testing
  - `subscribe() -> broadcast::Receiver<(RunId, WorkerEvent)>`
  - `async spawn(&mut self, loop_type: &str, params: Value) -> Result<RunId>`
  - `async cancel(&mut self, &RunId) -> Result<()>`
  - `active_runs(&self) -> Vec<RunId>`

## spawn() implementation
1. Generate `RunId::new()`
2. `tokio::process::Command::new(&binary_path).arg("worker").stdin(piped).stdout(piped).stderr(null).spawn()`
3. `take()` stdin and stdout from child
4. Write `WorkerCommand::Start { loop_type, params }` to stdin via `NdjsonCodec::encode`
5. Spawn tokio task: `BufReader::new(stdout).lines()` loop, decode each line via `NdjsonCodec::decode`, broadcast `(run_id, event)`
6. Store `ProcessHandle { child, stdin }` in `self.running`
7. Return `run_id`

## cancel() implementation
1. Look up `ProcessHandle` in `self.running` — error `RalphError::NotFound` if missing
2. Write `WorkerCommand::Cancel` to stdin (ignore write errors)
3. `tokio::time::timeout(3s, child.wait())` — wait for graceful exit
4. If timeout: `child.kill().await` (SIGKILL)
5. `self.running.remove(run_id)`

## Tests to write

### Learning tests (2)
- `tokio_process_piped_stdout` — spawns `echo` command, reads line via `BufReader::lines()`. Proves the tokio async process API works.
- `tokio_process_stdin_write` — spawns `cat`, writes to stdin via `write_all`, reads back via stdout `BufReader`. Proves bidirectional pipe communication.

### Unit tests (5)
- `run_id_is_unique` — two `RunId::new()` differ
- `run_id_display_is_uuid_format` — Display impl produces valid UUID string
- `spawn_emits_worker_events` — uses temp script subprocess, collects Status + Done events
- `cancel_terminates_running_process` — spawns `sleep 60`, cancels, verifies removed from active_runs
- `cancel_unknown_run_returns_not_found` — cancels non-existent RunId, checks `RalphError::NotFound`

## No migrations needed
Process manager is in-memory; crash recovery (DB persistence) is task 21.
