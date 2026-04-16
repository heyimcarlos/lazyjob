# Ralph Loop Integration — Implementation Plan

## Spec Reference
- **Spec file**: `specs/06-ralph-loop-integration.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
Ralph loops are autonomous AI agent subprocesses that power LazyJob's background job search tasks. This plan implements Option A: Stdio JSON Protocol, where Ralph is spawned as a CLI subprocess communicating via newline-delimited JSON over stdin/stdout. The TUI acts as the process orchestrator, managing loop lifecycle, progress tracking, and crash recovery.

## Problem Statement
LazyJob's TUI is long-running while Ralph loops are short-lived (minutes to hours). Key challenges:
1. **Lifecycle Management**: TUI must spawn, monitor, and terminate Ralph subprocesses
2. **IPC**: Bidirectional JSON messaging over stdin/stdout
3. **State Sharing**: Ralph accesses shared SQLite database
4. **Interruption**: User-initiated cancellation of running loops
5. **Crash Recovery**: Detection and recovery from Ralph crashes
6. **Parallelism**: Support for multiple concurrent loop types

## Implementation Phases

### Phase 1: Foundation — Ralph Binary CLI Scaffold
Create the `lazyjob-ralph` binary with clap CLI parsing and message protocol types.

**Steps:**
1. Create `lazyjob-ralph/` crate with `lazyjob-ralph` binary
2. Define `LoopType` enum with all loop variants (JobDiscovery, CompanyResearch, ResumeTailor, CoverLetterGeneration, InterviewPrep, SalaryNegotiation, Networking)
3. Implement CLI commands using clap with `#[derive(Parser)]`
4. Create `protocol.rs` with `IncomingMessage` and `OutgoingMessage` JSON enums
5. Build helper functions: `send_status()`, `send_results()`, `send_error()`, `send_done()`
6. Add basic stdin reader loop that logs received commands

**Types:**
```rust
// lazyjob-ralph/src/protocol.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingMessage {
    Start { loop_id: String, params: serde_json::Value },
    Cancel,
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    Status { phase: String, progress: f32, message: String },
    Results { data: serde_json::Value },
    Error { code: String, message: String },
    Done { success: bool },
}
```

### Phase 2: Core Implementation — RalphProcessManager in TUI
Implement the TUI-side process manager that spawns and orchestrates Ralph subprocesses.

**Steps:**
1. Add `lazyjob-ralph` as a dependency of `lazyjob-tui` (workspace member)
2. Create `lazyjob-ralph/src/process.rs` with `RalphProcessManager`
3. Implement `start_loop()`: spawn Ralph, set up stdout reader task, store `ChildHandle`
4. Implement `cancel_loop()`: send cancel message or kill process
5. Implement `wait_for_completion()`: timeout-aware event receiver
6. Implement `cleanup_dead_processes()`: detect crashed Ralph processes
7. Create broadcast channel for `RalphEvent` distribution to TUI views
8. Add `LoopId` newtype with `Uuid`

**Key Structs:**
```rust
// lazyjob-ralph/src/process.rs
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
pub enum RalphEvent {
    Started { loop_id: LoopId, loop_type: LoopType },
    Status { loop_id: LoopId, phase: String, progress: f32, message: String },
    Results { loop_id: LoopId, loop_type: LoopType, data: serde_json::Value },
    Error { loop_id: LoopId, code: String, message: String },
    Done { loop_id: LoopId, success: bool },
}
```

### Phase 3: Integration & Polish — TUI RalphPanel and Crash Recovery
Build the TUI view component and crash recovery logic.

**Steps:**
1. Create `lazyjob-tui/src/views/ralph_panel.rs` with `RalphPanel` struct
2. Implement `ActiveLoop` and `CompletedLoop` tracking structs
3. Build ratatui render logic with progress bars and status
4. Implement `handle_event()` to update UI state from RalphEvent stream
5. Add `restart_pending_loops()` to recover loops from previous TUI crash
6. Add SQL migration for `ralph_loops` table to track loop state
7. Implement periodic cleanup task in TUI event loop
8. Add keyboard shortcuts: `r` to start new loop, `c` to cancel selected

**RalphPanel render:**
- Header: "Ralph Loops" with icon
- Active loops list with progress bars (█░░░░ format)
- Status bar showing active count
- Error states displayed inline

### Phase 4: Per-Loop-Type Implementation
Implement actual loop logic for each `LoopType`. These are the actual AI tasks Ralph performs.

**Steps:**
1. **Job Discovery Loop** (`job-discovery`):
   - Load life sheet YAML
   - Fetch jobs from configured companies (Greenhouse/Lever APIs)
   - Generate embeddings for jobs
   - Match jobs against life sheet profile
   - Save results to SQLite

2. **Company Research Loop** (`company-research`):
   - Fetch company data from public APIs
   - Generate mission/culture/tech stack summary
   - Save to company table

3. **Resume Tailor Loop** (`resume-tailor`):
   - Read job description
   - Rewrite resume bullets with keywords
   - Output tailored resume file

4. **Cover Letter Generation** (`cover-letter`):
   - Read job and resume
   - Generate personalized cover letter
   - Save to output directory

5. **Interview Prep Loop** (`interview-prep`):
   - Generate practice questions
   - Create mock interview scenarios
   - Save to interview table

6. **Salary Negotiation Loop** (`salary-negotiation`):
   - Analyze market data
   - Generate negotiation strategy
   - Save to offers table

7. **Networking Loop** (`networking`):
   - Find warm contacts at target companies
   - Generate outreach templates
   - Save to contacts table

## Data Model

### New SQLite Tables

```sql
-- Tracks Ralph loop lifecycle for crash recovery
CREATE TABLE ralph_loops (
    id TEXT PRIMARY KEY,
    loop_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, in_progress, completed, failed, cancelled
    params TEXT,  -- JSON params
    started_at TEXT,
    updated_at TEXT,
    completed_at TEXT,
    exit_code INTEGER,
    result_data TEXT  -- JSON results
);

CREATE INDEX idx_ralph_loops_status ON ralph_loops(status);
CREATE INDEX idx_ralph_loops_updated ON ralph_loops(updated_at);

-- Activity log for TUI polling
CREATE TABLE activity_log (
    id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,  -- job, application, contact, etc.
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,  -- created, updated, deleted
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_activity_log_created ON activity_log(created_at);
```

### Ralph Configuration
```rust
// lazyjob-ralph/src/config.rs
#[derive(Debug, Clone, Deserialize)]
pub struct RalphConfig {
    pub path: PathBuf,  // Path to ralph binary
    pub db_path: PathBuf,
    pub life_sheet_path: PathBuf,
    pub max_concurrent_loops: usize,  // Default: 3
    pub loop_timeout: Duration,  // Default: 2 hours
}

impl Default for RalphConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("ralph"),
            db_path: dirs::data_dir().unwrap().join("lazyjob/lazyjob.db"),
            life_sheet_path: dirs::config_dir().unwrap().join("lazyjob/life-sheet.yaml"),
            max_concurrent_loops: 3,
            loop_timeout: Duration::from_secs(7200),
        }
    }
}
```

## API Surface

### lazyjob-ralph crate public API
```rust
// Main types
pub mod protocol;  // IncomingMessage, OutgoingMessage, send_* helpers
pub mod process;   // RalphProcessManager, RalphEvent, ChildHandle, LoopId
pub mod config;   // RalphConfig

// Re-exports for convenience
pub use protocol::{IncomingMessage, OutgoingMessage};
pub use process::{RalphProcessManager, RalphEvent, LoopId};
pub use config::RalphConfig;
```

### TUI Integration
```rust
// In lazyjob-tui/src/app.rs
pub struct App {
    ralph_manager: RalphProcessManager,
    ralph_events: broadcast::Receiver<RalphEvent>,
}

// Spawning a loop
let loop_id = app.ralph_manager.start_loop(LoopType::JobDiscovery, params).await?;
```

## Key Technical Decisions

### Stdio JSON over Unix Sockets
**Chosen:** Option A (Stdio JSON Protocol) for MVP
**Rationale:** Simpler implementation, Ralph is a pure CLI tool, easy to test, language-agnostic

**Rejected:** Unix Domain Sockets (Option B)
- More complexity (Ralph as daemon, socket server management)
- Overkill for MVP where Ralph is short-lived per loop

### Broadcast Channel for Events
**Chosen:** `tokio::sync::broadcast` with 100-event buffer
**Rationale:** Multiple TUI components may need Ralph event updates (panel, notifications, logs)

**Alternative:** `mpsc` channels per loop — rejected because we want broadcast semantics (all subscribers see events)

### Crash Recovery via SQLite
**Chosen:** Poll SQLite on startup for `in_progress` loops older than 1 hour
**Rationale:** Natural fit since Ralph writes directly to shared SQLite; no extra complexity

**Alternative:** Unix socket persistence — rejected as unnecessarily complex

## File Structure
```
lazyjob/
├── lazyjob-core/          # No changes
├── lazyjob-llm/           # No changes
├── lazyjob-tui/
│   └── src/
│       └── views/
│           └── ralph_panel.rs   # NEW: RalphPanel TUI component
├── lazyjob-cli/           # No changes
├── lazyjob-ralph/         # NEW crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs              # CLI entry, clap commands
│       ├── lib.rs               # Crate root, re-exports
│       ├── protocol.rs          # JSON message types
│       ├── process.rs           # RalphProcessManager
│       ├── config.rs            # RalphConfig
│       └── loops/
│           ├── mod.rs
│           ├── job_discovery.rs # JobDiscovery loop impl
│           ├── company_research.rs
│           ├── resume_tailor.rs
│           ├── cover_letter.rs
│           ├── interview_prep.rs
│           ├── salary_negotiation.rs
│           └── networking.rs
```

## Dependencies

### lazyjob-ralph/Cargo.toml
```toml
[package]
name = "lazyjob-ralph"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "1"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
futures = "0.3"
clap = { version = "4", features = ["derive"] }
dirs = "5"

# Database (Ralph binary accesses SQLite directly)
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }

# LLM integration (for actual loop implementations)
lazyjob-llm = { path = "../lazyjob-llm" }
```

### Changes to workspace Cargo.toml
Add `lazyjob-ralph` as a member crate.

## Testing Strategy

### Unit Tests
1. **protocol.rs**: Serialize/deserialize round-trip for all message types
2. **process.rs**: Mock child process behavior, cancellation signaling
3. **config.rs**: Default config, env var overrides

### Integration Tests
1. **Ralph CLI smoke test**: Spawn ralph binary, send commands, verify JSON output
2. **Process manager test**: Start loop, verify events emitted, cancel loop
3. **Crash recovery test**: Kill Ralph mid-loop, restart TUI, verify recovery UI

### TUI Tests
1. **RalphPanel render test**: Verify progress bar rendering with known state
2. **Event handling test**: Feed events, verify panel state updates

### Per-Loop Tests
Each loop type has integration tests that verify:
- Correct SQL writes to database
- Proper JSON result serialization
- Error handling (network failures, invalid data)

## Open Questions

1. **Ralph as Crate vs Separate Binary**: Should `lazyjob-ralph` be a workspace crate compiled as binary, or a completely separate project? — *Recommend: workspace crate for simpler dependency management*

2. **LLM API Keys**: How does Ralph access LLM credentials? — *Recommend: Ralph reads from shared config file or environment variables (same as TUI)*

3. **Progress Persistence**: Should Ralph periodically save checkpoint to SQLite so it can resume after crash? — *Recommend: Yes, checkpoint every 30 seconds for long loops*

4. **Concurrent Loop Limit**: Should we limit loops per type or total? — *Recommend: Max 3 total concurrent loops to avoid resource contention*

5. **Ralph Logging**: stdout/stderr redirection for debugging? — *Recommend: Stderr → log file, stdout is JSON protocol only*

## Effort Estimate

**Rough estimate: 2-3 weeks**

- Phase 1 (Foundation): 2-3 days — CLI scaffold, protocol types
- Phase 2 (ProcessManager): 3-4 days — Core spawning, event handling
- Phase 3 (TUI Integration): 2-3 days — RalphPanel, crash recovery
- Phase 4 (Loop Implementations): 5-7 days — One week to implement all 7 loop types
- Testing & Polish: 2-3 days

**Dependencies:**
- `04-sqlite-persistence` must be complete first (Ralph writes to same DB)
- `02-llm-provider-abstraction` needed for actual loop AI tasks
- `03-life-sheet-data-model` needed for job matching loop
