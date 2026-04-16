# Implementation Plan: Ralph IPC Protocol (Unix Domain Socket)

## Status
Draft

## Related Spec
[specs/XX-ralph-ipc-protocol.md](./XX-ralph-ipc-protocol.md)

## Overview

The Ralph IPC protocol defines the persistent, multiplexed channel between the TUI process
and a long-running Ralph daemon process. Unlike the subprocess stdin/stdout NDJSON protocol
(which covers per-loop fire-and-forget workers launched by `tokio::process::Command`), this
protocol governs a Unix domain socket connection between:

1. **TUI** — sends lifecycle commands (`StartLoop`, `CancelLoop`, `Shutdown`) and receives
   streaming progress events, heartbeats, and database change notifications.
2. **Ralph daemon** — a long-running subprocess that manages loop workers internally,
   reports aggregate health, and emits `DbChanged` events after every SQLite write so the TUI
   can invalidate caches and re-render stale views.

The two protocols are complementary. The subprocess NDJSON protocol (see
`agentic-ralph-subprocess-protocol-implementation-plan.md`) handles per-loop stdin/stdout
communication inside the Ralph daemon. This plan covers the socket-level channel between
the TUI and the daemon, which provides: multiplexed message routing, heartbeat-based health
monitoring, graceful shutdown sequencing, and protocol versioning for future compatibility.

The transport is a Unix domain socket at a well-known per-user path. Messages are
length-prefixed JSON frames (4-byte big-endian length + UTF-8 JSON body). A
`tokio-util` framing codec (`LengthDelimitedCodec`) handles split-read and partial-write
safely. The TUI holds a `RalphIpcClient` (wraps a `FramedWrite`/`FramedRead` pair) and the
daemon holds a `RalphIpcServer` that accepts exactly one TUI connection at a time.

## Prerequisites

### Must be implemented first
- Workspace `Cargo.toml` with multi-crate layout (`lazyjob-ralph`, `lazyjob-tui`,
  `lazyjob-core`) — see `specs/20-openapi-mvp-implementation-plan.md`.
- `lazyjob-ralph` crate must exist with the subprocess manager from
  `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — the IPC server wraps
  that manager.
- `lazyjob-core` must expose `LoopType` and the SQLite pool.

### Crates to add to `workspace.dependencies`

```toml
[workspace.dependencies]
tokio          = { version = "1", features = ["macros", "rt-multi-thread", "time", "sync", "process", "io-util", "net"] }
tokio-util     = { version = "0.7", features = ["codec"] }
futures-util   = "0.3"
bytes          = "1"
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
uuid           = { version = "1", features = ["v4", "serde"] }
thiserror      = "1"
anyhow         = "1"
tracing        = "0.1"
chrono         = { version = "0.4", features = ["serde"] }
```

In `lazyjob-ralph/Cargo.toml` add:

```toml
[dependencies]
tokio.workspace      = true
tokio-util.workspace = true
futures-util.workspace = true
bytes.workspace      = true
serde.workspace      = true
serde_json.workspace = true
uuid.workspace       = true
thiserror.workspace  = true
anyhow.workspace     = true
tracing.workspace    = true
chrono.workspace     = true
lazyjob-core = { path = "../lazyjob-core" }
```

For `#[cfg(unix)]` socket support no additional crate is needed — `tokio::net::UnixListener`
and `tokio::net::UnixStream` are part of `tokio` with the `"net"` feature.

---

## Architecture

### Crate Placement

| Component | Crate |
|---|---|
| `IpcMessage`, `MessageType`, payload types | `lazyjob-ralph::ipc::message` |
| `LengthDelimitedCodec` adapter | `lazyjob-ralph::ipc::codec` |
| `RalphIpcServer` (daemon side) | `lazyjob-ralph::ipc::server` |
| `RalphIpcClient` (TUI side) | `lazyjob-ralph::ipc::client` |
| `RalphManager` (TUI lifecycle) | `lazyjob-tui::ralph_manager` |
| Socket path resolution | `lazyjob-ralph::ipc::socket_path` |

`lazyjob-tui` imports `lazyjob-ralph::ipc::client::RalphIpcClient` and
`lazyjob-ralph::ipc::message::{IpcMessage, MessageType, StartLoopPayload, …}` — nothing
else from `lazyjob-ralph` bleeds into the TUI.

### Core Types

```rust
// lazyjob-ralph/src/ipc/message.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use lazyjob_core::LoopType;

pub const PROTOCOL_VERSION: u8 = 1;

/// Top-level envelope for every message sent over the socket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcMessage {
    /// Protocol version — always PROTOCOL_VERSION in new messages.
    pub version: u8,
    pub msg_type: MessageType,
    /// Correlation ID. Responses carry the same msg_id as the request.
    pub msg_id: Uuid,
    /// Typed payload. Variants drive which payload struct applies.
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    // ── TUI → Ralph ──────────────────────────────────────────────────────────
    StartLoop,
    CancelLoop,
    PauseLoop,
    ResumeLoop,
    GetStatus,
    Shutdown,

    // ── Ralph → TUI ──────────────────────────────────────────────────────────
    LoopStarted,
    LoopProgress,
    LoopCompleted,
    LoopError,
    LoopCancelled,
    StatusResponse,

    // ── Bidirectional ────────────────────────────────────────────────────────
    Heartbeat,
    HeartbeatAck,

    // ── State sync (Ralph → TUI) ─────────────────────────────────────────────
    DbChanged,
}

// ── Payload structs ──────────────────────────────────────────────────────────

/// TUI → Ralph: request a new loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StartLoopPayload {
    pub loop_type: LoopType,
    /// Loop-specific params (job_id, company_id, etc.) — forwarded verbatim.
    pub params: serde_json::Value,
    /// 1 (lowest) – 10 (highest). The daemon inserts into its priority queue.
    pub priority: u8,
}

/// Ralph → TUI: loop was accepted and assigned an ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopStartedPayload {
    pub loop_id: Uuid,
}

/// Ralph → TUI: streaming progress from an active loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopProgressPayload {
    pub loop_id: Uuid,
    /// Short label: "searching", "analyzing", "drafting", …
    pub step: String,
    /// 0.0 – 1.0. None when progress is indeterminate.
    pub progress: Option<f32>,
    pub message: String,
    pub tokens_used: u64,
}

/// Ralph → TUI: loop finished successfully.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopCompletedPayload {
    pub loop_id: Uuid,
    pub summary: String,
    pub tokens_used: u64,
}

/// Ralph → TUI: loop failed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopErrorPayload {
    pub loop_id: Uuid,
    pub error_code: ErrorCode,
    pub message: String,
    /// True if the TUI can offer a retry button.
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LlmApiError,
    LlmTimeout,
    LlmContextExceeded,
    DatabaseError,
    NetworkError,
    InvalidParams,
    RateLimited,
    InternalError,
}

/// Ralph → TUI: SQLite row was modified; TUI should invalidate view caches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbChangedPayload {
    pub table: String,
    /// "insert" | "update" | "delete"
    pub operation: String,
    /// String representation of the primary key (job_id, application_id, …).
    pub entity_id: String,
    pub loop_id: Option<Uuid>,
}

/// Heartbeat payload (both directions).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    /// Unix timestamp seconds.
    pub sent_at: i64,
    /// Number of active loops (Ralph → TUI heartbeats only).
    pub active_loops: Option<u32>,
}

/// Response to GetStatus.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponsePayload {
    pub active_loops: Vec<ActiveLoopStatus>,
    pub queued_loops: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveLoopStatus {
    pub loop_id: Uuid,
    pub loop_type: LoopType,
    pub progress: Option<f32>,
    pub step: String,
    pub started_at: DateTime<Utc>,
}
```

### Trait Definitions

```rust
// lazyjob-ralph/src/ipc/transport.rs

use tokio::io::{AsyncRead, AsyncWrite};

/// Marker trait — any bidirectional async stream usable as IPC transport.
pub trait IpcTransport: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

impl IpcTransport for tokio::net::UnixStream {}
```

### Module Structure

```
lazyjob-ralph/
  src/
    ipc/
      mod.rs            # pub use from submodules
      message.rs        # IpcMessage, MessageType, all payload structs, ErrorCode
      codec.rs          # IpcCodec wrapping LengthDelimitedCodec
      socket_path.rs    # resolve_socket_path() — platform-aware path resolution
      server.rs         # RalphIpcServer — daemon side
      client.rs         # RalphIpcClient — TUI side
    lib.rs              # pub mod ipc; …
```

---

## Implementation Phases

### Phase 1 — Transport and Codec (MVP Foundation)

#### 1.1 Socket path resolution

File: `lazyjob-ralph/src/ipc/socket_path.rs`

```rust
use std::path::PathBuf;

/// Returns the Unix domain socket path for the current user.
///
/// Linux: $XDG_RUNTIME_DIR/lazyjob/ralph.sock
///        (falls back to /tmp/lazyjob-{uid}/ralph.sock when XDG_RUNTIME_DIR unset)
/// macOS: $TMPDIR/lazyjob-{uid}/ralph.sock
#[cfg(unix)]
pub fn resolve_socket_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let uid = nix_uid();
                PathBuf::from(format!("/tmp/lazyjob-{uid}"))
            });
        base.join("lazyjob").join("ralph.sock")
    }
    #[cfg(target_os = "macos")]
    {
        let uid = nix_uid();
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(tmpdir).join(format!("lazyjob-{uid}")).join("ralph.sock")
    }
}

#[cfg(unix)]
fn nix_uid() -> u32 {
    // SAFETY: getuid is always safe
    unsafe { libc::getuid() }
}
```

Add `libc = "0.2"` to workspace dependencies.

Socket directory is created with `0o700` permissions on startup:

```rust
pub fn ensure_socket_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    Ok(())
}
```

**Stale socket cleanup**: before binding, check if the socket file exists. If yes, attempt
to connect — if connection is refused (no listener), remove the file. If connection succeeds,
return `IpcError::AlreadyRunning`.

```rust
pub async fn cleanup_stale_socket(path: &Path) -> Result<(), IpcError> {
    if !path.exists() {
        return Ok(());
    }
    match tokio::net::UnixStream::connect(path).await {
        Ok(_) => Err(IpcError::AlreadyRunning),
        Err(_) => {
            std::fs::remove_file(path).map_err(IpcError::Io)?;
            Ok(())
        }
    }
}
```

#### 1.2 Framing codec

File: `lazyjob-ralph/src/ipc/codec.rs`

Use `tokio_util::codec::LengthDelimitedCodec` — it handles 4-byte big-endian length prefix
framing, buffer management, and split reads. Wrap it in an `IpcCodec` that adds
JSON (de)serialization.

```rust
use bytes::{Bytes, BytesMut};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio::io::{AsyncRead, AsyncWrite};
use crate::ipc::message::IpcMessage;

pub type IpcFramed<T> = Framed<T, LengthDelimitedCodec>;

/// Wrap any AsyncRead + AsyncWrite into a length-delimited frame stream.
pub fn framed<T: AsyncRead + AsyncWrite>(io: T) -> IpcFramed<T> {
    LengthDelimitedCodec::builder()
        .length_field_length(4)
        .big_endian()
        .max_frame_length(16 * 1024 * 1024) // 16 MiB safety limit
        .new_framed(io)
}

/// Serialize an IpcMessage to a Bytes frame.
pub fn encode(msg: &IpcMessage) -> Result<Bytes, IpcError> {
    let json = serde_json::to_vec(msg).map_err(IpcError::Serialize)?;
    Ok(Bytes::from(json))
}

/// Deserialize a Bytes frame to an IpcMessage.
pub fn decode(frame: BytesMut) -> Result<IpcMessage, IpcError> {
    serde_json::from_slice(&frame).map_err(IpcError::Deserialize)
}
```

**Verification**: write a unit test that round-trips a `StartLoop` message through the codec
using `tokio_test::io::Builder`.

#### 1.3 Error type

File: `lazyjob-ralph/src/ipc/mod.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum IpcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialize error: {0}")]
    Serialize(serde_json::Error),

    #[error("deserialize error: {0}")]
    Deserialize(serde_json::Error),

    #[error("protocol version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u8, got: u8 },

    #[error("connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Ralph daemon is already running on this socket")]
    AlreadyRunning,

    #[error("heartbeat timeout — Ralph daemon appears hung")]
    HeartbeatTimeout,

    #[error("send failed: {0}")]
    Send(String),

    #[error("unexpected message type: {0:?}")]
    UnexpectedMessage(crate::ipc::message::MessageType),
}

pub type Result<T> = std::result::Result<T, IpcError>;
```

---

### Phase 2 — IPC Server (Ralph Daemon Side)

File: `lazyjob-ralph/src/ipc/server.rs`

The Ralph daemon spawns `RalphIpcServer::run()` as a top-level tokio task. It:
1. Binds a `UnixListener`.
2. Accepts exactly one TUI connection at a time (second connection is rejected).
3. Runs a `read_loop` (incoming commands → dispatch) and a `heartbeat_loop` (30s interval).
4. Forwards loop lifecycle commands to the existing `RalphProcessManager`.

```rust
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::{broadcast, mpsc, Mutex},
    time::{interval, MissedTickBehavior},
};
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;
use chrono::Utc;

use crate::ipc::{
    codec::{decode, encode, framed},
    message::*,
    IpcError, Result,
};
use crate::process_manager::RalphProcessManager;

pub struct RalphIpcServer {
    listener: UnixListener,
    process_manager: Arc<Mutex<RalphProcessManager>>,
    /// Receiver for DbChanged events emitted by loop workers.
    db_event_rx: broadcast::Receiver<DbChangedPayload>,
}

impl RalphIpcServer {
    pub async fn bind(
        socket_path: &Path,
        process_manager: Arc<Mutex<RalphProcessManager>>,
        db_event_rx: broadcast::Receiver<DbChangedPayload>,
    ) -> Result<Self> {
        crate::ipc::socket_path::cleanup_stale_socket(socket_path).await?;
        crate::ipc::socket_path::ensure_socket_dir(socket_path)?;
        let listener = UnixListener::bind(socket_path)?;
        tracing::info!(path = %socket_path.display(), "Ralph IPC server listening");
        Ok(Self { listener, process_manager, db_event_rx })
    }

    /// Accept one TUI connection and serve it until it closes or sends Shutdown.
    pub async fn run_once(&mut self) -> Result<()> {
        tracing::info!("Waiting for TUI connection");
        let (stream, _) = self.listener.accept().await?;
        tracing::info!("TUI connected");
        self.serve_connection(stream).await
    }

    async fn serve_connection(&mut self, stream: UnixStream) -> Result<()> {
        let mut framed = framed(stream);

        // Channel: inner tasks → write half
        let (out_tx, mut out_rx) = mpsc::channel::<IpcMessage>(64);

        // Heartbeat task
        let hb_tx = out_tx.clone();
        let heartbeat = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(30));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let msg = IpcMessage {
                    version: PROTOCOL_VERSION,
                    msg_type: MessageType::Heartbeat,
                    msg_id: Uuid::new_v4(),
                    payload: serde_json::to_value(HeartbeatPayload {
                        sent_at: Utc::now().timestamp(),
                        active_loops: None,
                    }).unwrap(),
                    timestamp: Utc::now(),
                };
                if hb_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // DbChanged relay task
        let db_tx = out_tx.clone();
        let mut db_rx = self.db_event_rx.resubscribe();
        let db_relay = tokio::spawn(async move {
            while let Ok(payload) = db_rx.recv().await {
                let msg = IpcMessage {
                    version: PROTOCOL_VERSION,
                    msg_type: MessageType::DbChanged,
                    msg_id: Uuid::new_v4(),
                    payload: serde_json::to_value(&payload).unwrap(),
                    timestamp: Utc::now(),
                };
                if db_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        loop {
            tokio::select! {
                // Outgoing messages
                Some(msg) = out_rx.recv() => {
                    let frame = encode(&msg)?;
                    framed.send(frame).await.map_err(|e| IpcError::Io(e))?;
                }

                // Incoming commands
                frame = framed.next() => {
                    match frame {
                        None => {
                            tracing::info!("TUI disconnected");
                            break;
                        }
                        Some(Err(e)) => {
                            tracing::error!(error = %e, "IPC read error");
                            break;
                        }
                        Some(Ok(bytes)) => {
                            let msg = decode(bytes)?;
                            if msg.version != PROTOCOL_VERSION {
                                return Err(IpcError::VersionMismatch {
                                    expected: PROTOCOL_VERSION,
                                    got: msg.version,
                                });
                            }
                            let should_exit = self.handle_message(msg, &out_tx).await?;
                            if should_exit {
                                break;
                            }
                        }
                    }
                }
            }
        }

        heartbeat.abort();
        db_relay.abort();
        Ok(())
    }

    async fn handle_message(
        &self,
        msg: IpcMessage,
        out_tx: &mpsc::Sender<IpcMessage>,
    ) -> Result<bool> {
        match msg.msg_type {
            MessageType::StartLoop => {
                let payload: StartLoopPayload = serde_json::from_value(msg.payload)
                    .map_err(IpcError::Deserialize)?;
                let mut pm = self.process_manager.lock().await;
                match pm.spawn(payload.loop_type, payload.params, payload.priority).await {
                    Ok(loop_id) => {
                        let reply = reply_msg(msg.msg_id, MessageType::LoopStarted,
                            LoopStartedPayload { loop_id });
                        let _ = out_tx.send(reply).await;
                    }
                    Err(e) => {
                        let reply = reply_msg(msg.msg_id, MessageType::LoopError,
                            LoopErrorPayload {
                                loop_id: Uuid::nil(),
                                error_code: ErrorCode::InternalError,
                                message: e.to_string(),
                                retryable: false,
                            });
                        let _ = out_tx.send(reply).await;
                    }
                }
            }

            MessageType::CancelLoop => {
                let loop_id: Uuid = serde_json::from_value(msg.payload["loop_id"].clone())
                    .map_err(IpcError::Deserialize)?;
                let mut pm = self.process_manager.lock().await;
                pm.cancel(loop_id).await;
                let reply = reply_msg(msg.msg_id, MessageType::LoopCancelled,
                    serde_json::json!({ "loop_id": loop_id }));
                let _ = out_tx.send(reply).await;
            }

            MessageType::GetStatus => {
                let pm = self.process_manager.lock().await;
                let status = pm.active_loop_statuses();
                let reply = reply_msg(msg.msg_id, MessageType::StatusResponse,
                    StatusResponsePayload {
                        active_loops: status,
                        queued_loops: pm.queue_depth() as u32,
                    });
                let _ = out_tx.send(reply).await;
            }

            MessageType::HeartbeatAck => {
                // TUI is alive — reset watchdog timer (Phase 3).
                tracing::debug!("Received HeartbeatAck from TUI");
            }

            MessageType::Shutdown => {
                tracing::info!("TUI requested graceful shutdown");
                return Ok(true);
            }

            other => {
                tracing::warn!(msg_type = ?other, "Unknown message type from TUI — ignoring");
            }
        }
        Ok(false)
    }
}

fn reply_msg<T: serde::Serialize>(
    msg_id: Uuid,
    msg_type: MessageType,
    payload: T,
) -> IpcMessage {
    IpcMessage {
        version: PROTOCOL_VERSION,
        msg_type,
        msg_id,
        payload: serde_json::to_value(payload).unwrap(),
        timestamp: Utc::now(),
    }
}
```

**Verification**: integration test with two `tokio::net::UnixStream` halves piped together
(`UnixStream::pair()`) — send `StartLoop`, assert `LoopStarted` reply contains a UUID.

---

### Phase 3 — IPC Client (TUI Side)

File: `lazyjob-ralph/src/ipc/client.rs`

The client splits the framed socket into read and write halves (via
`tokio::io::split`) so that the send path and receive path run independently. A background
read task routes incoming messages to either a pending-request `oneshot::Sender` (for
correlated replies) or a `broadcast::Sender<IpcMessage>` (for unsolicited push events like
`LoopProgress`, `DbChanged`, `Heartbeat`).

```rust
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::split,
    net::UnixStream,
    sync::{broadcast, oneshot, Mutex},
    time::timeout,
};
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;
use chrono::Utc;

use crate::ipc::{
    codec::{decode, encode, framed},
    message::*,
    IpcError, Result,
};

/// All unsolicited push events from Ralph are broadcast here.
/// TUI widgets subscribe via `client.subscribe_events()`.
pub type EventBus = broadcast::Sender<IpcMessage>;

pub struct RalphIpcClient {
    /// Write half — protected behind Mutex for multi-caller send.
    write_tx: Arc<Mutex<futures_util::stream::SplitSink<
        tokio_util::codec::Framed<tokio::net::UnixStream, tokio_util::codec::LengthDelimitedCodec>,
        bytes::Bytes,
    >>>,
    /// Pending correlated requests: msg_id → oneshot.
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<Result<IpcMessage>>>>>,
    /// Event broadcast for unsolicited server→TUI messages.
    event_bus: EventBus,
    /// Last time a HeartbeatAck was received from Ralph.
    last_ack: Arc<std::sync::Mutex<std::time::Instant>>,
}

impl RalphIpcClient {
    /// Connect to the Ralph daemon socket and spawn the read task.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        let framed = framed(stream);
        let (write_half, mut read_half) = framed.split();

        let pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<Result<IpcMessage>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_bus, _) = broadcast::channel(256);

        let pending_clone = Arc::clone(&pending);
        let event_bus_clone = event_bus.clone();
        let last_ack = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
        let last_ack_clone = Arc::clone(&last_ack);

        tokio::spawn(async move {
            while let Some(frame) = read_half.next().await {
                match frame {
                    Ok(bytes) => {
                        match decode(bytes) {
                            Ok(msg) => {
                                if msg.msg_type == MessageType::HeartbeatAck {
                                    *last_ack_clone.lock().unwrap() =
                                        std::time::Instant::now();
                                }

                                // Correlated reply?
                                let sender = {
                                    let mut pending = pending_clone.lock().await;
                                    pending.remove(&msg.msg_id)
                                };
                                if let Some(tx) = sender {
                                    let _ = tx.send(Ok(msg));
                                } else {
                                    // Unsolicited push event — broadcast to subscribers.
                                    let _ = event_bus_clone.send(msg);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "IPC decode error");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "IPC read error — connection lost");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            write_tx: Arc::new(Mutex::new(write_half)),
            pending,
            event_bus,
            last_ack,
        })
    }

    /// Subscribe to unsolicited push events (LoopProgress, DbChanged, Heartbeat, …).
    pub fn subscribe_events(&self) -> broadcast::Receiver<IpcMessage> {
        self.event_bus.subscribe()
    }

    /// Request Ralph start a loop. Returns the assigned loop_id.
    pub async fn start_loop(
        &self,
        loop_type: LoopType,
        params: serde_json::Value,
        priority: u8,
    ) -> Result<Uuid> {
        let resp = self.request(
            MessageType::StartLoop,
            StartLoopPayload { loop_type, params, priority },
        ).await?;
        match resp.msg_type {
            MessageType::LoopStarted => {
                let p: LoopStartedPayload = serde_json::from_value(resp.payload)
                    .map_err(IpcError::Deserialize)?;
                Ok(p.loop_id)
            }
            MessageType::LoopError => {
                let p: LoopErrorPayload = serde_json::from_value(resp.payload)
                    .map_err(IpcError::Deserialize)?;
                Err(IpcError::Send(p.message))
            }
            other => Err(IpcError::UnexpectedMessage(other)),
        }
    }

    pub async fn cancel_loop(&self, loop_id: Uuid) -> Result<()> {
        self.fire_and_forget(
            MessageType::CancelLoop,
            serde_json::json!({ "loop_id": loop_id }),
        ).await
    }

    pub async fn get_status(&self) -> Result<StatusResponsePayload> {
        let resp = self.request(MessageType::GetStatus, serde_json::Value::Null).await?;
        serde_json::from_value(resp.payload).map_err(IpcError::Deserialize)
    }

    /// Send Shutdown and close the connection. Waits up to 5 seconds for clean exit.
    pub async fn shutdown(&self) -> Result<()> {
        self.fire_and_forget(MessageType::Shutdown, serde_json::Value::Null).await
    }

    /// Check whether Ralph is still healthy (received HeartbeatAck within 90s).
    pub fn is_healthy(&self) -> bool {
        let last = self.last_ack.lock().unwrap();
        last.elapsed() < Duration::from_secs(90)
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    async fn request<T: serde::Serialize>(
        &self,
        msg_type: MessageType,
        payload: T,
    ) -> Result<IpcMessage> {
        let msg_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(msg_id, tx);
        }
        self.send_raw(msg_id, msg_type, payload).await?;
        // 30-second timeout for correlated replies.
        timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| IpcError::HeartbeatTimeout)?
            .map_err(|_| IpcError::ConnectionClosed)?
    }

    async fn fire_and_forget<T: serde::Serialize>(
        &self,
        msg_type: MessageType,
        payload: T,
    ) -> Result<()> {
        self.send_raw(Uuid::new_v4(), msg_type, payload).await
    }

    async fn send_raw<T: serde::Serialize>(
        &self,
        msg_id: Uuid,
        msg_type: MessageType,
        payload: T,
    ) -> Result<()> {
        let msg = IpcMessage {
            version: PROTOCOL_VERSION,
            msg_type,
            msg_id,
            payload: serde_json::to_value(payload).map_err(IpcError::Serialize)?,
            timestamp: Utc::now(),
        };
        let frame = encode(&msg)?;
        let mut write = self.write_tx.lock().await;
        write.send(frame).await.map_err(IpcError::Io)
    }
}
```

---

### Phase 4 — TUI Integration (RalphManager)

File: `lazyjob-tui/src/ralph_manager.rs`

The TUI's `App` struct owns a `RalphManager` that:
1. Spawns the Ralph daemon subprocess (same binary, `ralph` subcommand).
2. Waits for the socket to appear (retry loop, 30s timeout).
3. Creates a `RalphIpcClient`.
4. Subscribes to `event_bus` and routes events to the TUI `AppEvent` channel.

```rust
use std::{path::PathBuf, time::{Duration, Instant}};
use tokio::{process::Child, sync::mpsc};
use lazyjob_ralph::ipc::{client::RalphIpcClient, message::*, socket_path::resolve_socket_path};

pub struct RalphManager {
    child: Option<Child>,
    pub client: Option<RalphIpcClient>,
    socket_path: PathBuf,
}

impl RalphManager {
    pub fn new() -> Self {
        Self {
            child: None,
            client: None,
            socket_path: resolve_socket_path(),
        }
    }

    /// Spawn the Ralph daemon and connect.
    pub async fn start(&mut self, app_event_tx: mpsc::Sender<AppEvent>) -> anyhow::Result<()> {
        let binary = std::env::current_exe()?;

        // Remove stale socket before binding.
        let _ = tokio::fs::remove_file(&self.socket_path).await;

        let child = tokio::process::Command::new(&binary)
            .arg("ralph")
            .arg("--socket")
            .arg(&self.socket_path)
            .kill_on_drop(true)
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(pid, "Ralph daemon spawned");
        self.child = Some(child);

        // Wait up to 30 seconds for socket to appear.
        let client = self.wait_for_socket(Duration::from_secs(30)).await?;

        // Route push events to the TUI AppEvent channel.
        let mut events = client.subscribe_events();
        let tx = app_event_tx.clone();
        tokio::spawn(async move {
            while let Ok(msg) = events.recv().await {
                let event = match msg.msg_type {
                    MessageType::LoopProgress => {
                        if let Ok(p) = serde_json::from_value::<LoopProgressPayload>(msg.payload) {
                            Some(AppEvent::RalphProgress(p))
                        } else { None }
                    }
                    MessageType::LoopCompleted => {
                        if let Ok(p) = serde_json::from_value::<LoopCompletedPayload>(msg.payload) {
                            Some(AppEvent::RalphCompleted(p))
                        } else { None }
                    }
                    MessageType::LoopError => {
                        if let Ok(p) = serde_json::from_value::<LoopErrorPayload>(msg.payload) {
                            Some(AppEvent::RalphError(p))
                        } else { None }
                    }
                    MessageType::DbChanged => {
                        if let Ok(p) = serde_json::from_value::<DbChangedPayload>(msg.payload) {
                            Some(AppEvent::DbChanged(p))
                        } else { None }
                    }
                    MessageType::Heartbeat => Some(AppEvent::RalphHeartbeat),
                    _ => None,
                };
                if let Some(ev) = event {
                    let _ = tx.send(ev).await;
                }
            }
        });

        self.client = Some(client);
        Ok(())
    }

    /// Clean shutdown: send Shutdown command, wait for process exit, remove socket.
    pub async fn shutdown(&mut self) {
        if let Some(ref client) = self.client {
            let _ = client.shutdown().await;
        }
        if let Some(ref mut child) = self.child {
            let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
            let _ = child.start_kill();
        }
        let _ = tokio::fs::remove_file(&self.socket_path).await;
    }

    async fn wait_for_socket(&self, timeout: Duration) -> anyhow::Result<RalphIpcClient> {
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() > deadline {
                anyhow::bail!("Timed out waiting for Ralph IPC socket");
            }
            match RalphIpcClient::connect(&self.socket_path).await {
                Ok(client) => return Ok(client),
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
}
```

The `AppEvent` enum (in `lazyjob-tui`) adds:

```rust
pub enum AppEvent {
    // … existing variants …
    RalphProgress(LoopProgressPayload),
    RalphCompleted(LoopCompletedPayload),
    RalphError(LoopErrorPayload),
    DbChanged(DbChangedPayload),
    RalphHeartbeat,
}
```

TUI `App::handle_event` matches these to invalidate view state and re-render.

---

### Phase 5 — DbChanged Emission Inside Ralph Daemon

The Ralph daemon must emit `DbChanged` events after every SQLite write. This is wired via a
`broadcast::Sender<DbChangedPayload>` passed through the crate:

```rust
// lazyjob-ralph/src/db_notifier.rs

use tokio::sync::broadcast;
use crate::ipc::message::DbChangedPayload;

/// Clone and inject into every repository that writes to SQLite.
pub type DbNotifier = broadcast::Sender<DbChangedPayload>;

pub fn new_notifier() -> (DbNotifier, broadcast::Receiver<DbChangedPayload>) {
    broadcast::channel(256)
}
```

Each repository calls `notifier.send(DbChangedPayload { table, operation, entity_id, loop_id })` after a successful write. Errors from `send()` (no active receivers) are silently ignored — the IPC server may not be connected yet.

Example in `SqliteJobRepository::upsert()`:

```rust
async fn upsert(&self, job: &Job) -> Result<UpsertOutcome> {
    // … SQLite upsert …
    let outcome = …;
    if outcome != UpsertOutcome::Unchanged {
        let _ = self.db_notifier.send(DbChangedPayload {
            table: "jobs".to_string(),
            operation: match outcome {
                UpsertOutcome::Inserted => "insert",
                UpsertOutcome::Updated => "update",
                _ => unreachable!(),
            }.to_string(),
            entity_id: job.id.to_string(),
            loop_id: self.current_loop_id,
        });
    }
    Ok(outcome)
}
```

---

### Phase 6 — Heartbeat Watchdog and Reconnect

The TUI-side `RalphManager` spawns a watchdog task that:
1. Checks `client.is_healthy()` every 60 seconds.
2. If unhealthy for two consecutive checks (120s without an ack), emits `AppEvent::RalphUnhealthy`.
3. On `AppEvent::RalphUnhealthy`, the TUI shows a modal: "Ralph appears unresponsive. Restart?" — matching UX from the orphan cleanup plan.

```rust
// In RalphManager::start() after client is created
let client_ref = client.is_healthy_fn(); // returns Arc<dyn Fn() -> bool>
let tx = app_event_tx.clone();
tokio::spawn(async move {
    let mut missed = 0u8;
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if client_ref() {
            missed = 0;
        } else {
            missed += 1;
            if missed >= 2 {
                let _ = tx.send(AppEvent::RalphUnhealthy).await;
                break;
            }
        }
    }
});
```

Ralph sends `Heartbeat` every 30 seconds. TUI sends `HeartbeatAck` on receive. If the TUI
doesn't respond with `HeartbeatAck` within 90 seconds, Ralph logs a warning but does NOT
shut down — the TUI may be in a CPU-intensive render cycle.

---

## Key Crate APIs

| API | Usage |
|---|---|
| `tokio::net::UnixListener::bind(path)` | Daemon binds the socket |
| `tokio::net::UnixStream::connect(path)` | TUI client connects |
| `tokio::net::UnixStream::pair()` | In-memory pair for unit tests |
| `tokio_util::codec::LengthDelimitedCodec::builder().length_field_length(4).big_endian().new_framed(io)` | Frame codec |
| `futures_util::stream::StreamExt::next()` | Async frame read |
| `futures_util::sink::SinkExt::send(frame)` | Async frame write |
| `tokio::io::split(stream)` | Split into independent read/write halves |
| `tokio::sync::broadcast::channel(256)` | Event fan-out (DbChanged, progress events) |
| `tokio::sync::oneshot::channel()` | Request–reply correlation |
| `tokio::time::timeout(Duration, future)` | 30s request timeout |
| `tokio::time::interval(Duration)` | Heartbeat tick |
| `serde_json::to_vec(msg)` | JSON serialize before framing |
| `serde_json::from_slice(bytes)` | JSON deserialize after framing |

---

## SQLite Schema

The IPC layer itself does not need a schema (it is ephemeral). However, the `ralph_ipc_sessions` table is useful for debugging and post-mortem analysis:

```sql
-- Migration: 003_ralph_ipc_sessions.sql
CREATE TABLE IF NOT EXISTS ralph_ipc_sessions (
    id            TEXT PRIMARY KEY,          -- UUID
    started_at    TEXT NOT NULL,             -- ISO 8601
    ended_at      TEXT,
    socket_path   TEXT NOT NULL,
    tui_pid       INTEGER,
    ralph_pid     INTEGER,
    messages_sent INTEGER NOT NULL DEFAULT 0,
    messages_recv INTEGER NOT NULL DEFAULT 0,
    end_reason    TEXT                       -- "shutdown" | "crash" | "timeout"
) STRICT;
```

Written by `RalphIpcServer::bind()` (insert) and updated by `serve_connection()` on exit.
This table is purely informational — no query from the application logic reads it.

---

## Error Handling

```rust
// lazyjob-ralph/src/ipc/mod.rs

#[derive(thiserror::Error, Debug)]
pub enum IpcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialize error: {0}")]
    Serialize(serde_json::Error),

    #[error("JSON deserialize error: {0}")]
    Deserialize(serde_json::Error),

    #[error("protocol version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u8, got: u8 },

    #[error("connection closed by peer")]
    ConnectionClosed,

    #[error("Ralph IPC socket is already in use — is another instance running?")]
    AlreadyRunning,

    #[error("heartbeat timeout — Ralph did not respond within 90 seconds")]
    HeartbeatTimeout,

    #[error("send failed: {0}")]
    Send(String),

    #[error("unexpected message type: {0:?}")]
    UnexpectedMessage(MessageType),

    #[error("request timeout after 30s waiting for msg_id={0}")]
    RequestTimeout(Uuid),
}

pub type Result<T, E = IpcError> = std::result::Result<T, E>;
```

All IPC errors in `RalphManager` are converted to `anyhow::Error` via `?` and surfaced as
`AppEvent::RalphError` — they never crash the TUI. The TUI renders a dismissable error
banner with an option to reconnect or kill Ralph.

---

## Testing Strategy

### Unit Tests

**Codec round-trip** (`lazyjob-ralph/tests/ipc_codec.rs`):
```rust
#[tokio::test]
async fn roundtrip_start_loop_message() {
    let msg = IpcMessage {
        version: PROTOCOL_VERSION,
        msg_type: MessageType::StartLoop,
        msg_id: Uuid::new_v4(),
        payload: serde_json::to_value(StartLoopPayload {
            loop_type: LoopType::JobDiscovery,
            params: serde_json::json!({}),
            priority: 5,
        }).unwrap(),
        timestamp: Utc::now(),
    };
    let frame = encode(&msg).unwrap();
    let decoded = decode(BytesMut::from(frame.as_ref())).unwrap();
    assert_eq!(msg.msg_type, decoded.msg_type);
    assert_eq!(msg.msg_id, decoded.msg_id);
}
```

**Version mismatch rejection**:
```rust
#[tokio::test]
async fn rejects_wrong_version() {
    let mut msg = valid_message();
    msg.version = 99;
    let frame = encode(&msg).unwrap();
    let decoded = decode(BytesMut::from(frame.as_ref())).unwrap();
    // server calls validate_version
    assert!(matches!(
        decoded.validate_version(),
        Err(IpcError::VersionMismatch { .. })
    ));
}
```

### Integration Tests

**In-memory server/client** (`lazyjob-ralph/tests/ipc_integration.rs`):

```rust
#[tokio::test]
async fn start_loop_roundtrip() {
    // Use UnixStream::pair() for in-process test — no file system.
    let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();

    let pm = Arc::new(Mutex::new(MockProcessManager::new()));
    let (db_tx, db_rx) = broadcast::channel(16);
    let mut server = RalphIpcServer::from_stream(server_stream, pm, db_rx);

    tokio::spawn(async move { server.serve_stream().await });

    let client = RalphIpcClient::from_stream(client_stream).await.unwrap();
    let loop_id = client
        .start_loop(LoopType::JobDiscovery, serde_json::json!({}), 5)
        .await
        .unwrap();
    assert!(!loop_id.is_nil());
}
```

Add `from_stream` constructors to both `RalphIpcServer` and `RalphIpcClient` that accept a
`UnixStream` directly — same logic as the path-based constructors but without socket bind/connect.

**DbChanged relay test**:
```rust
#[tokio::test]
async fn db_changed_event_relayed_to_client() {
    let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();
    let (db_tx, db_rx) = broadcast::channel(16);
    let pm = Arc::new(Mutex::new(MockProcessManager::new()));
    let mut server = RalphIpcServer::from_stream(server_stream, pm, db_rx);
    tokio::spawn(async move { server.serve_stream().await });

    let client = RalphIpcClient::from_stream(client_stream).await.unwrap();
    let mut events = client.subscribe_events();

    db_tx.send(DbChangedPayload {
        table: "jobs".to_string(),
        operation: "insert".to_string(),
        entity_id: "job-123".to_string(),
        loop_id: None,
    }).unwrap();

    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await.unwrap().unwrap();
    assert_eq!(event.msg_type, MessageType::DbChanged);
}
```

### TUI Smoke Test

`RalphManager::start()` is tested with a real subprocess (the `lazyjob ralph` subcommand)
in `lazyjob-tui/tests/ralph_manager_smoke.rs`:
```rust
// Spawns the actual binary — marked #[ignore] in CI unless LAZYJOB_INTEGRATION=1
#[tokio::test]
#[ignore]
async fn ralph_manager_lifecycle() {
    let (tx, mut rx) = mpsc::channel(32);
    let mut manager = RalphManager::new();
    manager.start(tx).await.unwrap();
    let status = manager.client.as_ref().unwrap().get_status().await.unwrap();
    assert_eq!(status.active_loops.len(), 0);
    manager.shutdown().await;
    // Should receive no crash events
    assert!(rx.try_recv().is_err());
}
```

---

## Protocol Versioning

`PROTOCOL_VERSION: u8 = 1` is a compile-time constant. On receiving any message:

```rust
impl IpcMessage {
    pub fn validate_version(&self) -> Result<()> {
        if self.version != PROTOCOL_VERSION {
            return Err(IpcError::VersionMismatch {
                expected: PROTOCOL_VERSION,
                got: self.version,
            });
        }
        Ok(())
    }
}
```

When the protocol changes in an incompatible way:
- Increment `PROTOCOL_VERSION`.
- The server rejects older clients with `VersionMismatch`.
- The TUI shows: "Ralph daemon is outdated. Please restart LazyJob."
- All payload structs use `#[serde(default)]` on new optional fields to allow reading
  older messages without breaking deserialization — additive changes do not require a
  version bump.

---

## Open Questions

1. **Multiple TUI windows**: Should Ralph support multiple simultaneous TUI connections?
   Currently `RalphIpcServer::run_once()` accepts one connection; a second TUI would be
   silently queued in the kernel backlog. Decision: single-TUI model for MVP; a future
   `run_multi()` that broadcasts events to all connected clients can be added in Phase 7.

2. **Windows support**: `UnixStream` is not available on Windows pre-1803. If Windows becomes
   a target, the transport must switch to named pipes (`\\.\pipe\lazyjob-ralph`) behind a
   `#[cfg(windows)]` adapter. Stub out `resolve_socket_path()` for Windows now so compilation
   doesn't fail.

3. **Socket permissions**: should the socket be group-readable for multi-user systems?
   Currently `0o700` (owner-only). If team workspaces land (SaaS phase), change to `0o660`
   with a `lazyjob` group.

4. **Max frame size**: 16 MiB is generous for JSON messages. If `LoopProgress` payloads ever
   include large previews (resume DOCX preview), consider chunked streaming instead of
   embedding binary in the JSON payload.

5. **DbChanged granularity**: currently one event per row. High-frequency discovery runs
   (1000+ jobs) may flood the broadcast channel. Consider batching: aggregate `DbChanged`
   events for the same table within a 100ms window and send a single `DbChangedBatch` event.

6. **Stale socket on crash without cleanup**: if the Ralph process crashes before removing the
   socket file, the next startup calls `cleanup_stale_socket()` which tries to connect —
   if the kernel has not yet cleaned up the socket inode, the connect might block briefly.
   Enforce a 200ms connect timeout in `cleanup_stale_socket()`.

---

## Related Specs

- `specs/agentic-ralph-subprocess-protocol.md` — per-loop stdin/stdout NDJSON protocol
  (inner protocol, complementary to this one)
- `specs/agentic-ralph-orchestration.md` — loop queue, concurrency limits, priority
- `specs/XX-ralph-process-orphan-cleanup.md` — PID tracking, startup scan, SIGKILL escalation
- `specs/XX-llm-cost-budget-management.md` — cost attribution per loop (loop_id in DbChanged)
- `specs/09-tui-design-keybindings-implementation-plan.md` — AppEvent routing, TUI re-render
- `specs/16-privacy-security-implementation-plan.md` — socket permission model
