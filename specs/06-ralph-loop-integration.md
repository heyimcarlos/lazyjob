# Ralph Loop Integration

## Status
Researching

## Problem Statement

LazyJob is powered by "Ralph loops" — autonomous agent loops that run AI-powered job search tasks in the background. The TUI is the user-facing interface, while Ralph subprocesses handle the AI work.

Key challenges:
1. **Lifecycle Management**: TUI is long-running; Ralph loops are short-lived (minutes to hours)
2. **IPC**: How does the TUI send commands to Ralph and receive updates?
3. **State Sharing**: How does Ralph access job/contact data from the TUI's SQLite?
4. **Interruption**: How does the user cancel a running Ralph loop?
5. **Crash Recovery**: What happens if Ralph crashes? TUI restarts?
6. **Parallelism**: Can multiple Ralph loops run simultaneously?

This spec defines the architecture for TUI ↔ Ralph subprocess integration.

---

## Research Findings

### Tokio Process Management

The `tokio::process::Command` API provides async process spawning with piped stdin/stdout/stderr:

**Spawning a Child Process**
```rust
use tokio::process::Command;
use std::process::Stdio;

let mut child = Command::new("ralph")
    .arg("job-discovery")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("failed to spawn ralph");
```

**Reading stdout as a Stream**
```rust
use tokio::io::AsyncBufReadExt;
use tokio::process::Child;

let stdout = child.stdout.as_mut().expect("stdout not piped");
let mut lines = stdout.lines();

while let Some(line) = lines.next_line().await.expect("read error") {
    // Parse JSON message from Ralph
    let msg: RalphMessage = serde_json::from_str(&line)?;
    // Handle message
}
```

**Writing to stdin**
```rust
use tokio::io::AsyncWriteExt;

let stdin = child.stdin.as_mut().expect("stdin not piped");
stdin.write_all(b"{\"command\":\"cancel\"}\n").await?;
stdin.flush().await?;
```

**Waiting for Exit**
```rust
let status = child.wait().await?;
println!("Ralph exited with: {}", status);
```

**Killing a Process**
```rust
child.kill().await.expect("kill failed");
```

### Unix Domain Sockets (Alternative IPC)

For more robust IPC, Unix domain sockets can be used:

**Server (TUI)**
```rust
use tokio::net::UnixListener;

let listener = UnixListener::bind("/tmp/lazyjob-ralph.sock").unwrap();
loop {
    match listener.accept().await {
        Ok((stream, _addr)) => {
            // Handle Ralph connection
        }
        Err(e) => { /* connection failed */ }
    }
}
```

**Client (Ralph)**
```rust
use tokio::net::UnixStream;

let stream = UnixStream::connect("/tmp/lazyjob-ralph.sock").await?;
```

**Pros of Unix Sockets**:
- More reliable than stdio for long-running connections
- Can survive TUI restart (if Ralph is daemonized)
- Bidirectional communication

**Cons**:
- More complex (Ralph needs to act as client)
- Requires socket file management
- TUI must act as server

### Stdio vs Unix Sockets

| Aspect | Stdio (Pipes) | Unix Sockets |
|--------|---------------|--------------|
| Complexity | Simple | Moderate |
| Reliability | Good for short-lived | Better for long-lived |
| Bidirectional | Yes (stdin + stdout) | Yes |
| Survives TUI restart | No | Possible |
| File descriptor management | None | Must clean up socket file |
| Rust async support | Excellent | Excellent |

---

## Design Options

### Option A: Stdio JSON Protocol (Recommended)

**Description**: Ralph is spawned as a subprocess. TUI ↔ Ralph communication via newline-delimited JSON over stdin/stdout.

**Protocol**:
- TUI → Ralph: Commands via stdin
- Ralph → TUI: Events via stdout
- Ralph → TUI: Errors via stderr (optional, for debugging)

**Message Types**:
```json
// TUI → Ralph: Start a loop
{"type": "start", "loop": "job_discovery", "params": {...}}

// TUI → Ralph: Cancel
{"type": "cancel"}

// TUI → Ralph: Pause/Resume
{"type": "pause"}
{"type": "resume"}

// Ralph → TUI: Status update
{"type": "status", "phase": "searching", "progress": 0.5, "message": "Searching LinkedIn..."}

// Ralph → TUI: Results
{"type": "results", "loop": "job_discovery", "data": {...}}

// Ralph → TUI: Error
{"type": "error", "code": "rate_limited", "message": "..."}

// Ralph → TUI: Done
{"type": "done", "success": true}
```

**Pros**:
- Simple to implement
- Ralph is a pure CLI tool (easy to test)
- Natural async reading via `lines()`
- Works with any language (not just Rust)
- Stdout/stderr separation

**Cons**:
- Ralph subprocess must be spawned fresh for each loop
- Can't persist Ralph state across TUI restarts
- No connection persistence between commands

**Best for**: MVP, simplicity, language-agnostic

### Option B: Unix Domain Socket with Daemonized Ralph

**Description**: Ralph runs as a long-lived daemon process. TUI connects via Unix socket.

**Pros**:
- Ralph can maintain state between commands
- Survives TUI restart
- More reliable for long-running operations
- Can multiplex multiple loop types

**Cons**:
- Ralph must be installed as a system service
- More complex lifecycle management
- Socket file cleanup on exit
- Harder to debug

**Best for**: Production, persistent background processing

### Option C: HTTP/REST with Ralph Server

**Description**: Ralph runs as a local HTTP server (localhost:18777). TUI makes REST calls.

**Pros**:
- Well-understood HTTP patterns
- Easy to debug with curl
- Can add authentication later
- Works across machines (if needed)

**Cons**:
- HTTP overhead for local IPC
- Ralph must manage HTTP server lifecycle
- Port conflict potential

**Best for**: When Ralph might run remotely

### Option D: Shared SQLite with Advisory Locking

**Description**: Ralph directly reads/writes to the same SQLite database. TUI polls for changes.

**Pros**:
- No IPC complexity
- Natural state persistence
- Both TUI and Ralph see same data

**Cons**:
- No real-time progress updates
- Polling overhead
- SQLite locking complexity with many processes

**Best for**: Simple tasks without progress feedback

---

## Recommended Approach

**Option A: Stdio JSON Protocol** for MVP.

Rationale:
1. Simplest to implement and debug
2. Ralph is a standalone CLI (easy to test independently)
3. Language-agnostic (could implement Ralph in Python later)
4. Progress updates via JSON are sufficient for UX
5. Ralph state persists via SQLite (shared with TUI)

---

## Architecture

### Ralph CLI Interface

Ralph is invoked as a CLI tool with subcommands:

```bash
# Start a job discovery loop
ralph job-discovery --life-sheet ~/.lazyjob/life-sheet.yaml --db ~/.lazyjob/lazyjob.db

# Start a company research loop
ralph company-research "Stripe" --db ~/.lazyjob/lazyjob.db

# Start a resume tailoring loop
ralph resume-tailor --job-id <uuid> --life-sheet ~/.lazyjob/life-sheet.yaml --output ~/.lazyjob/resumes/

# Start an interview prep loop
ralph interview-prep --job-id <uuid> --db ~/.lazyjob/lazyjob.db

# Interactive mode (reads commands from stdin)
ralph interactive --db ~/.lazyjob/lazyjob.db
```

### Ralph Process Manager

```rust
// lazyjob-ralph/src/process.rs

use tokio::process::{Child, Command, Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, broadcast};
use tokio::time::{timeout, Duration};
use std::collections::HashMap;

pub struct RalphProcessManager {
    ralph_path: String,
    db_path: PathBuf,
    life_sheet_path: PathBuf,
    running_processes: HashMap<LoopId, ChildHandle>,
    event_tx: broadcast::Sender<RalphEvent>,
}

pub struct ChildHandle {
    loop_id: LoopId,
    process: Child,
    cancel_tx: oneshot::Sender<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RalphCommand {
    Start { loop_type: LoopType, params: serde_json::Value },
    Cancel { loop_id: LoopId },
    Pause { loop_id: LoopId },
    Resume { loop_id: LoopId },
    Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RalphEvent {
    Started { loop_id: LoopId, loop_type: LoopType },
    Status { loop_id: LoopId, phase: String, progress: f32, message: String },
    Results { loop_id: LoopId, loop_type: LoopType, data: serde_json::Value },
    Error { loop_id: LoopId, code: String, message: String },
    Done { loop_id: LoopId, success: bool },
}

impl RalphProcessManager {
    pub fn new(
        ralph_path: String,
        db_path: PathBuf,
        life_sheet_path: PathBuf,
    ) -> Self {
        Self {
            ralph_path,
            db_path,
            life_sheet_path,
            running_processes: HashMap::new(),
            event_tx: broadcast::channel(100).0,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RalphEvent> {
        self.event_tx.subscribe()
    }

    pub async fn start_loop(
        &mut self,
        loop_type: LoopType,
        params: serde_json::Value,
    ) -> Result<LoopId> {
        let loop_id = LoopId::new();

        // Create cancellation token
        let (cancel_tx, cancel_rx) = oneshot::channel();

        // Build command
        let mut cmd = Command::new(&self.ralph_path);
        cmd.arg(loop_type.to_string())
            .arg("--loop-id".to_string(), loop_id.to_string())
            .arg("--db".to_string(), self.db_path.display().to_string())
            .arg("--life-sheet".to_string(), self.life_sheet_path.display().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set up environment
        cmd.env("RUST_LOG", "info");

        let mut child = cmd.spawn()
            .map_err(|e| RalphError::SpawnFailed(e.to_string()))?;

        // Spawn task to read stdout
        let stdout = child.stdout.take().expect("stdout not piped");
        let loop_id_clone = loop_id.clone();
        let event_tx_clone = self.event_tx.clone();

        let read_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(msg) = serde_json::from_str::<RalphMessage>(&line) {
                    let event = msg_to_event(loop_id_clone.clone(), msg);
                    let _ = event_tx_clone.send(event);
                }
            }
        });

        // Spawn task to handle cancellation
        let loop_id_cancel = loop_id.clone();
        let cancel_handle = tokio::spawn(async move {
            cancel_rx.await.ok();
            // Send SIGTERM to process
        });

        // Store handle
        self.running_processes.insert(loop_id.clone(), ChildHandle {
            loop_id: loop_id.clone(),
            process: child,
            cancel_tx,
        });

        // Emit started event
        self.event_tx.send(RalphEvent::Started {
            loop_id: loop_id.clone(),
            loop_type,
        }).ok();

        Ok(loop_id)
    }

    pub async fn cancel_loop(&mut self, loop_id: &LoopId) -> Result<()> {
        if let Some(handle) = self.running_processes.remove(loop_id) {
            handle.cancel_tx.send(()).ok();

            // Kill the process
            handle.process.kill().await
                .map_err(|e| RalphError::KillFailed(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn wait_for_completion(&mut self, loop_id: &LoopId, timeout: Duration)
        -> Result<Option<RalphEvent>> {
        let mut rx = self.event_tx.subscribe();

        let result = timeout(timeout, async {
            while let Ok(event) = rx.recv().await {
                match &event {
                    RalphEvent::Done { loop_id: id, .. } if id == loop_id => {
                        return Some(event);
                    }
                    RalphEvent::Error { loop_id: id, .. } if id == loop_id => {
                        return Some(event);
                    }
                    _ => continue,
                }
            }
            None
        }).await;

        Ok(result.ok().flatten())
    }
}
```

### Ralph JSON Protocol Handler

```rust
// lazyjob-ralph/src/protocol.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingMessage {
    Start {
        loop_id: String,
        params: serde_json::Value,
    },
    Cancel,
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    Status {
        phase: String,
        progress: f32,
        message: String,
    },
    Results {
        data: serde_json::Value,
    },
    Error {
        code: String,
        message: String,
    },
    Done {
        success: bool,
    },
}

pub fn send_status(
    stdout: &mut tokio::io::WriteHalf<ChildStdin>,
    phase: &str,
    progress: f32,
    message: &str,
) -> impl Future<Output = std::io::Result<()>> + '_ {
    let msg = OutgoingMessage::Status {
        phase: phase.to_string(),
        progress,
        message: message.to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    stdout.write_all(json.as_bytes()).and_then(|_| stdout.write_all(b"\n"))
}

pub fn send_results(
    stdout: &mut tokio::io::WriteHalf<ChildStdin>,
    data: serde_json::Value,
) -> impl Future<Output = std::io::Result<()>> + '_ {
    let msg = OutgoingMessage::Results { data };
    let json = serde_json::to_string(&msg).unwrap();
    stdout.write_all(json.as_bytes()).and_then(|_| stdout.write_all(b"\n"))
}
```

### Ralph Main Loop Example (Pseudo-code)

```rust
// Ralph binary (simplified)

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::JobDiscovery { loop_id, db_path, life_sheet_path } => {
            job_discovery_loop(loop_id, db_path, life_sheet_path).await?;
        }
        Commands::CompanyResearch { loop_id, company, db_path } => {
            company_research_loop(loop_id, company, db_path).await?;
        }
        Commands::ResumeTailor { loop_id, job_id, life_sheet_path, output_path } => {
            resume_tailor_loop(loop_id, job_id, life_sheet_path, output_path).await?;
        }
        Commands::InterviewPrep { loop_id, job_id, db_path } => {
            interview_prep_loop(loop_id, job_id, db_path).await?;
        }
    }

    Ok(())
}

async fn job_discovery_loop(
    loop_id: LoopId,
    db_path: PathBuf,
    life_sheet_path: PathBuf,
) -> Result<()> {
    // Set up stdin reader for cancellation
    let stdin = tokio::io::stdin();
    let mut lines = stdin.lines();

    // Load life sheet
    let life_sheet = serde_yaml::from_path(&life_sheet_path)?;

    // Load configured companies
    let config = Config::load()?;
    let companies = config.discovery.companies;

    // Get repository
    let pool = SqlitePool::connect(&format!("sqlite:{}?mode=rwc", db_path.display())).await?;
    let repo = JobRepository::new(&pool);

    // Initialize LLM provider
    let llm = LLMBuilder::from_config(&config.llm).build()?;
    let matcher = JobMatcher::new(llm);

    // Discovery loop
    for (i, company) in companies.iter().enumerate() {
        // Check for cancellation
        if let Ok(Some(Ok(line))) = tokio::time::timeout(
            Duration::from_millis(100),
            lines.next_line()
        ).await {
            if let Ok(IncomingMessage::Cancel) = serde_json::from_str(&line) {
                send_status(stdout, "cancelled", 0.0, "Cancelled by user").await?;
                return Ok(());
            }
        }

        send_status(
            stdout,
            "fetching",
            i as f32 / companies.len() as f32,
            &format!("Fetching jobs from {}...", company.name)
        ).await?;

        // Fetch jobs from Greenhouse/Lever
        let jobs = discover_company_jobs(company).await?;

        for job in jobs {
            repo.insert(&job).await?;

            // Generate embedding
            let embedding = matcher.embed_job(&job).await?;
            repo.update_embedding(job.id, embedding).await?;
        }

        send_status(
            stdout,
            "fetched",
            i as f32 / companies.len() as f32,
            &format!("Found {} jobs from {}", jobs.len(), company.name)
        ).await?;
    }

    // Find matching jobs
    send_status(stdout, "matching", 0.9, "Finding matching jobs...").await?;
    let sheet = load_life_sheet(&life_sheet_path)?;
    let matching = matcher.find_matching_jobs(&jobs, &sheet, 20).await?;

    // Save results
    repo.save_discovery_results(&matching).await?;

    send_results(stdout, serde_json::json!({
        "new_jobs": new_job_count,
        "matching_jobs": matching,
    })).await?;

    send_status(stdout, "done", 1.0, "Discovery complete").await?;
    Ok(())
}
```

### TUI Ralph Panel

```rust
// lazyjob-tui/src/views/ralph_panel.rs

pub struct RalphPanel {
    manager: Arc<Mutex<RalphProcessManager>>,
    events: broadcast::Receiver<RalphEvent>,
    active_loops: Vec<ActiveLoop>,
    history: Vec<CompletedLoop>,
}

pub struct ActiveLoop {
    loop_id: LoopId,
    loop_type: LoopType,
    phase: String,
    progress: f32,
    message: String,
    started_at: DateTime<Utc>,
}

impl View for RalphPanel {
    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(3),  // Header
            Constraint::Fill(1),    // Content
            Constraint::Length(1),  // Status bar
        ]).areas(area);

        // Header
        let header = Block::bordered()
            .title("Ralph Loops")
            .title_style(Style::new().bold());
        frame.render_widget(header, chunks[0]);

        // Active loops list
        let items: Vec<ListItem> = self.active_loops.iter().map(|loop_| {
            let progress_bar = "█".repeat((loop_.progress * 20.0) as usize)
                & "-".repeat(20 - (loop_.progress * 20.0) as usize);
            ListItem::new(format!(
                "[{}] {} - {}% - {}",
                loop_.loop_type.icon(),
                loop_.message,
                (loop_.progress * 100.0) as u32,
                progress_bar
            ))
        }).collect();

        let list = List::new(items)
            .block(Block::bordered())
            .style(Style::new().fg(Color::Blue));
        frame.render_widget(list, chunks[1]);

        // Status bar
        let status = if self.active_loops.is_empty() {
            "No active loops".to_string()
        } else {
            format!("{} active loop(s)", self.active_loops.len())
        };
        frame.render_widget(Paragraph::new(status), chunks[2]);
    }
}

impl RalphPanel {
    pub async fn handle_event(&mut self, event: RalphEvent) {
        match event {
            RalphEvent::Started { loop_id, loop_type } => {
                self.active_loops.push(ActiveLoop {
                    loop_id,
                    loop_type,
                    phase: "starting".to_string(),
                    progress: 0.0,
                    message: "Initializing...".to_string(),
                    started_at: Utc::now(),
                });
            }
            RalphEvent::Status { loop_id, phase, progress, message } => {
                if let Some(loop_) = self.active_loops.iter_mut().find(|l| l.loop_id == loop_id) {
                    loop_.phase = phase;
                    loop_.progress = progress;
                    loop_.message = message;
                }
            }
            RalphEvent::Done { loop_id, success } => {
                self.active_loops.retain(|l| l.loop_id != loop_id);
                self.history.push(CompletedLoop { loop_id, success, finished_at: Utc::now() });
            }
            RalphEvent::Error { loop_id, code, message } => {
                // Show error in UI
                if let Some(loop_) = self.active_loops.iter_mut().find(|l| l.loop_id == loop_id) {
                    loop_.message = format!("ERROR: {} - {}", code, message);
                    loop_.progress = 1.0;
                }
            }
            _ => {}
        }
    }
}
```

### State Synchronization

Since both TUI and Ralph access the same SQLite database, state synchronization is natural:

```rust
// Ralph writes directly to SQLite
async fn save_discovery_results(db: &SqlitePool, results: &[DiscoveryResult]) -> Result<()> {
    sqlx::query!(
        "INSERT INTO discovery_results (id, loop_id, discovered_at, data) VALUES (?, ?, datetime('now'), ?)",
        Uuid::new_v4(),
        loop_id,
        serde_json::to_string(&results)?
    )
    .execute(db)
    .await?;
    Ok(())
}

// TUI polls or uses NOTIFY for changes
async fn poll_for_changes(pool: &SqlitePool, last_check: DateTime<Utc>) -> Result<Vec<Change>> {
    sqlx::query_as!(
        Change,
        "SELECT * FROM activity_log WHERE created_at > ? ORDER BY created_at",
        last_check
    )
    .fetch_all(pool)
    .await
}
```

### Crash Recovery

```rust
impl RalphProcessManager {
    pub async fn cleanup_dead_processes(&mut self) {
        let mut dead = Vec::new();

        for (loop_id, handle) in &mut self.running_processes {
            match handle.process.try_wait() {
                Ok(Some(status)) => {
                    // Process exited
                    dead.push(loop_id.clone());
                    self.event_tx.send(RalphEvent::Done {
                        loop_id: loop_id.clone(),
                        success: status.success(),
                    }).ok();
                }
                Ok(None) => {
                    // Still running
                }
                Err(e) => {
                    // Error checking status
                    dead.push(loop_id.clone());
                    self.event_tx.send(RalphEvent::Error {
                        loop_id: loop_id.clone(),
                        code: "process_error".to_string(),
                        message: e.to_string(),
                    }).ok();
                }
            }
        }

        for loop_id in dead {
            self.running_processes.remove(&loop_id);
        }
    }

    pub async fn restart_pending_loops(&mut self) -> Result<()> {
        // Query SQLite for loops that were "in progress" when TUI crashed
        let pending = sqlx::query_as!(
            PendingLoop,
            "SELECT * FROM ralph_loops WHERE status = 'in_progress' AND updated_at < datetime('now', '-1 hour')"
        )
        .fetch_all(&*self.pool)
        .await?;

        for pending_loop in pending {
            // Re-start the loop
            self.start_loop(pending_loop.loop_type, pending_loop.params).await?;
        }

        Ok(())
    }
}
```

### Loop Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoopType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailor,
    CoverLetterGeneration,
    InterviewPrep,
    SalaryNegotiation,
    Networking,
}

impl LoopType {
    pub fn to_string(&self) -> &'static str {
        match self {
            LoopType::JobDiscovery => "job-discovery",
            LoopType::CompanyResearch => "company-research",
            LoopType::ResumeTailor => "resume-tailor",
            LoopType::CoverLetterGeneration => "cover-letter",
            LoopType::InterviewPrep => "interview-prep",
            LoopType::SalaryNegotiation => "salary-negotiation",
            LoopType::Networking => "networking",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            LoopType::JobDiscovery => "🔍",
            LoopType::CompanyResearch => "🏢",
            LoopType::ResumeTailor => "📄",
            LoopType::CoverLetterGeneration => "✉️",
            LoopType::InterviewPrep => "🎯",
            LoopType::SalaryNegotiation => "💰",
            LoopType::Networking => "🤝",
        }
    }
}
```

---

## Failure Modes

1. **Ralph Process Won't Start**: Return error to user, suggest checking installation
2. **Ralph Crashes Mid-Loop**: Detect via `wait()`, emit error event, mark loop as failed
3. **User Cancels**: Send message to Ralph via stdin (if supported) or kill process
4. **TUI Restarts Mid-Loop**: On startup, query SQLite for "in_progress" loops, offer to resume or cancel
5. **SQLite Lock Timeout**: Ralph uses same WAL-mode DB, `busy_timeout` handles contention
6. **Network Failure in Ralph**: Ralph handles retries, emits error event if unrecoverable

---

## Open Questions

1. **Ralph as Separate Crate or Binary**: Should Ralph be `lazyjob-ralph` crate compiled as binary, or a completely separate project?
2. **Ralph Configuration**: How does Ralph get LLM API keys? Shared config file or environment variables?
3. **Progress Persistence**: Should Ralph periodically save progress to SQLite so it can resume after crash?
4. **Multiple Concurrent Loops**: Should we limit to 1 loop per type, or allow parallel loops?
5. **Ralph Logging**: stdout/stderr handling for debugging? Log file redirection?

---

## Dependencies

```toml
# lazyjob-ralph/Cargo.toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "1"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
futures = "0.3"

# Ralph CLI (separate binary)
clap = { version = "4", features = ["derive"] }

# Database (for Ralph binary)
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }
```

---

## Sources

- [Tokio Process Documentation](https://docs.rs/tokio/1.49.0/tokio/process/)
- [Tokio Unix Listener](https://docs.rs/tokio/1.49.0/tokio/net/struct.UnixListener)
- [Tokio AsyncBufReadExt](https://docs.rs/tokio/1.49.0/tokio/io/trait.AsyncBufReadExt)
- [JSON Lines Format](https://jsonlines.org/)
