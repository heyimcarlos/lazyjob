# Ralph Subprocess IPC Protocol

## Status
Researching

## Problem Statement

LazyJob uses ralph autonomous agent loops running as subprocesses. The TUI (main process) must communicate with ralph subprocesses for:
1. **Lifecycle management**: Start, pause, resume, cancel ralph loops
2. **State synchronization**: When ralph modifies data (new job, updated application), TUI must see changes
3. **Progress reporting**: TUI must show ralph progress in real-time
4. **Error propagation**: Errors in ralph must surface to TUI and user
5. **Resource management**: When TUI closes, ralph must be cleanly terminated

Currently there is no spec for this IPC protocol.

---

## Solution Overview

A bidirectional Unix socket IPC protocol with:
1. **Message types** for all TUI↔Ralph communication
2. **Lifecycle protocol** for starting/stopping loops
3. **State sync** via database write notification
4. **Heartbeat** to detect hung/dead ralph processes
5. **Graceful shutdown** sequence

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        TUI Process                          │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐  │
│  │ RalphClient │  │ StateSync   │  │ LifecycleManager   │  │
│  └──────┬──────┘  └──────┬───────┘  └─────────┬──────────┘  │
│         │                │                    │             │
└─────────┼────────────────┼────────────────────┼─────────────┘
          │                │                    │
          │    Unix Domain Socket               │
          │                │                    │
┌─────────┼────────────────┼────────────────────┼─────────────┐
│         ▼                ▼                    ▼             │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐  │
│  │ RalphServer │  │ DbWatcher    │  │ ProcessManager     │  │
│  └──────┬──────┘  └──────┬───────┘  └─────────┬──────────┘  │
│         │                │                    │             │
│         │         ┌───────┴───────┐            │             │
│         │         │   SQLite     │◄───────────┘             │
│         │         │   WAL Mode   │                           │
│         │         └───────────────┘                           │
│  ┌──────┴──────┐                                             │
│  │ Ralph Loop │  (autonomous agent)                         │
│  │  Process   │                                              │
│  └─────────────┘                                              │
└───────────────────────────────────────────────────────────────┘
```

---

## Transport Layer

### Unix Domain Socket

Using `tokio::net::UnixStream` for IPC:

```rust
// lazyjob-ralph/src/ipc/transport.rs

pub struct UnixSocketTransport {
    stream: UnixStream,
    read_buf: BytesMut,
    write_buf: BytesMut,
}

impl AsyncRead for UnixSocketTransport {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        // Read from UnixStream into internal buffer, copy to buf
    }
}

impl AsyncWrite for UnixSocketTransport {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        // Write from buf to internal buffer, flush to UnixStream
    }
}
```

### Socket Path

```rust
const SOCKET_PATH: &str = "/run/user/{uid}/lazyjob/ralph.sock";

const SOCKET_DIR: &str = "/run/user/{uid}/lazyjob";
```

For macOS (no `/run/user`):
```rust
const SOCKET_PATH: &str = "/tmp/lazyjob-{uid}/ralph.sock";
```

---

## Message Protocol

### Message Format

Messages are length-prefixed JSON:

```
┌─────────────────┬─────────────────────────────────────────────┐
│  4 bytes (u32)  │  JSON payload (variable length)           │
│  (big-endian)   │                                           │
└─────────────────┴─────────────────────────────────────────────┘
```

```rust
// lazyjob-ralph/src/ipc/message.rs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IPCMessage {
    pub version: u8,           // Protocol version (1)
    pub msg_type: MessageType,
    pub msg_id: Uuid,         // For correlation/ack
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    // TUI -> Ralph
    StartLoop,
    PauseLoop,
    ResumeLoop,
    CancelLoop,
    GetStatus,

    // Ralph -> TUI
    LoopStarted,
    LoopProgress,
    LoopCompleted,
    LoopError,
    LoopPaused,
    LoopCancelled,
    StatusResponse,

    // Bidirectional
    Heartbeat,
    HeartbeatAck,
    Shutdown,

    // State sync
    DbChanged,
    DbChangedAck,
}
```

### Payload Types

```rust
// StartLoop payload
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StartLoopPayload {
    pub loop_type: LoopType,
    pub config: serde_json::Value,  // Loop-specific config
    pub priority: u8,  // 1-10, higher = more important
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrep,
    SalaryNegotiation,
    OutreachDrafting,
}

// LoopProgress payload
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopProgressPayload {
    pub loop_id: Uuid,
    pub step: String,           // "searching", "analyzing", "drafting"
    pub progress: f32,          // 0.0 - 1.0
    pub message: String,        // Human-readable status
    pub tokens_used: u64,
    pub jobs_processed: u32,
}

// LoopError payload
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopErrorPayload {
    pub loop_id: Uuid,
    pub error_code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub context: Option<serde_json::Value>,  // For debugging
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LLMApiError,
    LLMTimeout,
    LLMContextExceeded,
    DatabaseError,
    NetworkError,
    InvalidConfig,
    RateLimited,
    InternalError,
}

// DbChanged payload
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbChangedPayload {
    pub table: String,
    pub operation: String,  // "insert", "update", "delete"
    pub entity_id: String,
    pub timestamp: DateTime<Utc>,
}
```

---

## IPC Client (TUI Side)

```rust
// lazyjob-ralph/src/ipc/client.rs

pub struct RalphClient {
    transport: UnixSocketTransport,
    pending: HashMap<Uuid, oneshot::Sender<Result<IPCMessage>>>,
    heartbeat_tx: Interval,
}

impl RalphClient {
    pub async fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path).await?;
        Ok(Self {
            transport: UnixSocketTransport::new(stream),
            pending: HashMap::new(),
            heartbeat_tx: interval(Duration::from_secs(30)),
        })
    }

    pub async fn start_loop(&self, loop_type: LoopType, config: serde_json::Value) -> Result<Uuid> {
        let msg_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();

        self.pending.insert(msg_id, tx);

        let msg = IPCMessage {
            version: 1,
            msg_type: MessageType::StartLoop,
            msg_id,
            payload: serde_json::to_value(StartLoopPayload {
                loop_type,
                config,
                priority: 5,
            })?,
            timestamp: Utc::now(),
        };

        self.send(msg).await?;
        let response = rx.await??;

        match response.msg_type {
            MessageType::LoopStarted => {
                let loop_id: Uuid = serde_json::from_value(response.payload)?;
                Ok(loop_id)
            }
            MessageType::LoopError => {
                let error: LoopErrorPayload = serde_json::from_value(response.payload)?;
                Err(RalphError::StartFailed(error.message))
            }
            _ => Err(RalphError::UnexpectedMessage(response.msg_type)),
        }
    }

    pub async fn cancel_loop(&self, loop_id: Uuid) -> Result<()> {
        let msg = IPCMessage {
            version: 1,
            msg_type: MessageType::CancelLoop,
            msg_id: Uuid::new_v4(),
            payload: serde_json::to_value(serde_json::json!({ "loop_id": loop_id }))?,
            timestamp: Utc::now(),
        };
        self.send(msg).await?;
        Ok(())
    }
}
```

---

## IPC Server (Ralph Side)

```rust
// lazyjob-ralph/src/ipc/server.rs

pub struct RalphServer {
    listener: UnixListener,
    transport: Option<UnixSocketTransport>,
    loops: Arc<Mutex<HashMap<Uuid, RunningLoop>>>,
}

struct RunningLoop {
    loop_type: LoopType,
    config: serde_json::Value,
    status: LoopStatus,
    progress: f32,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
enum LoopStatus {
    Running,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

impl RalphServer {
    pub async fn listen(path: &Path) -> Result<Self> {
        // Create socket directory
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(path)?;
        Ok(Self {
            listener,
            transport: None,
            loops: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                // Accept new connections
                result = self.listener.accept() => {
                    let (stream, _) = result?;
                    self.transport = Some(UnixSocketTransport::new(stream));
                }

                // Read messages
                msg = self.transport.as_mut().unwrap().read_message() => {
                    self.handle_message(msg?).await?;
                }

                // Heartbeat check
                _ = self.heartbeat_interval.tick() => {
                    self.send_heartbeat().await?;
                }
            }
        }
    }

    async fn handle_message(&mut self, msg: IPCMessage) -> Result<()> {
        match msg.msg_type {
            MessageType::StartLoop => {
                let payload: StartLoopPayload = serde_json::from_value(msg.payload)?;
                let loop_id = self.spawn_loop(payload).await?;

                let response = IPCMessage {
                    version: 1,
                    msg_type: MessageType::LoopStarted,
                    msg_id: msg.msg_id,
                    payload: serde_json::to_value(loop_id)?,
                    timestamp: Utc::now(),
                };
                self.transport.as_mut().unwrap().send_message(response).await?;
            }

            MessageType::CancelLoop => {
                let payload: serde_json::Value = serde_json::from_value(msg.payload)?;
                let loop_id: Uuid = serde_json::from_value(payload["loop_id"].clone())?;
                self.cancel_loop(loop_id).await?;

                let response = IPCMessage {
                    version: 1,
                    msg_type: MessageType::LoopCancelled,
                    msg_id: msg.msg_id,
                    payload: serde_json::to_value(serde_json::json!({ "loop_id": loop_id }))?,
                    timestamp: Utc::now(),
                };
                self.transport.as_mut().unwrap().send_message(response).await?;
            }

            MessageType::Heartbeat => {
                let response = IPCMessage {
                    version: 1,
                    msg_type: MessageType::HeartbeatAck,
                    msg_id: msg.msg_id,
                    payload: serde_json::to_value(serde_json::json!({
                        "loops": self.get_loop_statuses()
                    }))?,
                    timestamp: Utc::now(),
                };
                self.transport.as_mut().unwrap().send_message(response).await?;
            }

            MessageType::Shutdown => {
                self.shutdown().await?;
            }

            _ => {}
        }
        Ok(())
    }
}
```

---

## State Synchronization

### Database Write Notification

Ralph writes to SQLite. TUI needs to know when data changes.

Option 1: **SQLite NOTIFY** (best)
```sql
-- Ralph sends notify after write
SELECT sqlbd_notify('jobs_changed');

-- TUI listens
LISTEN jobs_changed;
```

Option 2: **Polling** (simpler but less efficient)
Ralph sends `DbChanged` message after each write.

**Recommended**: Use Ralph's `DbChanged` message after writes. Keep it simple for MVP.

```rust
// After Ralph writes to database
async fn after_db_write(&self, table: &str, operation: &str, entity_id: &str) -> Result<()> {
    let msg = IPCMessage {
        version: 1,
        msg_type: MessageType::DbChanged,
        msg_id: Uuid::new_v4(),
        payload: serde_json::to_value(DbChangedPayload {
            table: table.to_string(),
            operation: operation.to_string(),
            entity_id: entity_id.to_string(),
            timestamp: Utc::now(),
        })?,
        timestamp: Utc::now(),
    };
    self.transport.as_ref().unwrap().send_message(msg).await?;
    Ok(())
}
```

### TUI Invalidation

```rust
// TUI side
impl RalphClient {
    async fn handle_db_changed(&self, payload: DbChangedPayload) -> Result<()> {
        match payload.table.as_str() {
            "jobs" => {
                self.job_cache.invalidate(&payload.entity_id);
                self.emit_ui_event(UIEvent::JobUpdated(payload.entity_id));
            }
            "applications" => {
                self.app_cache.invalidate(&payload.entity_id);
                self.emit_ui_event(UIEvent::ApplicationUpdated(payload.entity_id));
            }
            // ... other tables
            _ => {}
        }
        Ok(())
    }
}
```

---

## Lifecycle Protocol

### Starting Ralph

```rust
// lazyjob-tui/src/ralph_manager.rs

pub struct RalphManager {
    ralph_path: PathBuf,
    socket_path: PathBuf,
    client: Option<RalphClient>,
    processes: HashMap<Uuid, Child>,
}

impl RalphManager {
    pub async fn start_ralph(&mut self) -> Result<()> {
        // Ensure socket dir exists
        std::fs::create_dir_all(self.socket_path.parent().unwrap())?;

        // Spawn ralph subprocess
        let child = tokio::process::Command::new(&self.ralph_path)
            .arg("--socket")
            .arg(&self.socket_path)
            .arg("--log-level")
            .arg("info")
            .spawn()?;

        let pid = child.id().unwrap();
        tracing::info!(pid = pid, "Ralph subprocess spawned");

        // Wait for socket to be ready (with timeout)
        let client = self.wait_for_socket(30).await?;

        self.client = Some(client);
        self.processes.insert(Uuid::from_u128(pid as u128), child);

        Ok(())
    }

    async fn wait_for_socket(&self, timeout_secs: u64) -> Result<RalphClient> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);

        loop {
            if Instant::now() > deadline {
                return Err(RalphError::SocketTimeout);
            }

            match RalphClient::connect(&self.socket_path).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    // Socket not ready yet, wait and retry
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}
```

### Ralph Subprocess Entry Point

```rust
// lazyjob-ralph/src/main.rs

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Setup logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Setup panic handler
    std::panic::set_hook(Box::new(|panic_info| {
        tracing::error!(panic = %panic_info, "Ralph panic");
    }));

    // Connect to socket
    let mut server = RalphServer::listen(&args.socket).await?;

    // Initialize LLM providers
    let llm = LLMBuilder::from_env().build()?;

    // Run main loop
    server.run().await?;

    Ok(())
}
```

### Graceful Shutdown

```rust
// On TUI shutdown
impl RalphManager {
    pub async fn shutdown(&mut self) -> Result<()> {
        // 1. Send shutdown message to Ralph
        if let Some(client) = &self.client {
            client.shutdown().await?;
        }

        // 2. Wait for Ralph to finish current work (with timeout)
        let shutdown_timeout = Duration::from_secs(10);
        let deadline = Instant::now() + shutdown_timeout;

        // 3. Force kill if not done
        for (_, child) in self.processes.iter_mut() {
            child.start_kill()?;
        }

        // 4. Cleanup socket
        let _ = std::fs::remove_file(&self.socket_path);

        Ok(())
    }
}

// On Ralph receiving Shutdown message
impl RalphServer {
    async fn shutdown(&mut self) -> Result<()> {
        tracing::info!("Ralph shutting down");

        // Cancel all running loops gracefully
        for (id, loop_) in self.loops.lock().unwrap().iter_mut() {
            if let LoopStatus::Running = loop_.status {
                loop_.status = LoopStatus::Cancelled;
                // Send cancellation to loop task
            }
        }

        // Wait for loops to finish (with timeout)
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Exit
        std::process::exit(0);
    }
}
```

---

## Heartbeat and Process Health

```rust
// Ralph sends heartbeat every 30 seconds
async fn heartbeat_loop(&self) {
    let mut interval = interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        let msg = IPCMessage {
            version: 1,
            msg_type: MessageType::Heartbeat,
            msg_id: Uuid::new_v4(),
            payload: serde_json::to_value(serde_json::json!({
                "memory_mb": get_memory_usage(),
                "cpu_percent": get_cpu_usage(),
            }))?,
            timestamp: Utc::now(),
        };

        self.transport.as_mut().unwrap().send_message(msg).await?;
    }
}

// TUI expects heartbeat within 60 seconds
// If missed, marks ralph as unhealthy and offers restart
```

---

## Error Handling

### Ralph Crash Recovery

```rust
// TUI monitors ralph process
impl RalphManager {
    async fn monitor_process(&mut self, mut child: Child) -> Result<()> {
        loop {
            match child.try_wait()? {
                Some(status) => {
                    // Ralph exited
                    tracing::error!(status = %status, "Ralph process exited");

                    // Notify user
                    self.emit_ui_event(UIEvent::RalphCrashed {
                        exit_code: status.code(),
                    });

                    // Offer restart
                    let should_restart = self.ask_user_restart().await?;

                    if should_restart {
                        self.start_ralph().await?;
                    }

                    break;
                }
                None => {
                    // Still running
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Ok(())
    }
}
```

---

## Protocol Versioning

The protocol includes a version field for future compatibility:

```rust
const PROTOCOL_VERSION: u8 = 1;

impl IPCMessage {
    pub fn validate_version(&self) -> Result<()> {
        if self.version != PROTOCOL_VERSION {
            return Err(RalphError::ProtocolVersionMismatch {
                expected: PROTOCOL_VERSION,
                got: self.version,
            });
        }
        Ok(())
    }
}
```

---

## Open Questions

1. **Socket cleanup**: What if socket file persists after crash? Should check for existing socket and remove before bind.
2. **Multiple Ralph instances**: Should support multiple Ralph processes for parallel loops?
3. **Privileged access**: Does Ralph need root access for any operations?
4. **Socket permissions**: Should socket be restricted to user-only access?

---

## Related Specs

- `01-architecture.md` - Overall architecture
- `06-ralph-loop-integration.md` - Ralph loop details
- `XX-llm-cost-budget-management.md` - Cost attribution to loops
- `XX-error-handling-panic-recovery.md` - Panic handling
