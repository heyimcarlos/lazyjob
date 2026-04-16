# Implementation Plan: Interview Session Resumability

## Status
Draft

## Related Spec
[specs/XX-interview-session-resumability.md](./XX-interview-session-resumability.md)

## Overview

The `MockInterviewLoop` is a long-running, interactive Ralph subprocess. Users practice mock interviews over 15–30 minutes and may need to stop mid-session due to interruptions, fatigue, or time constraints. Without resumability, all partial progress — questions answered, feedback received, conversational context — is permanently lost.

This plan implements a full save/resume lifecycle for mock interview sessions. Auto-save triggers after every answered question and on a 5-minute background interval. Partial sessions are persisted to SQLite with an immutable `interview_session_checkpoints` table that stores the full Q&A history needed to reconstruct LLM context on resume. On TUI startup, a non-modal notification surfaces any valid partial sessions, offering Resume / New Session / Discard options with a transparent token cost estimate.

Sessions expire for resume 48 hours after their last checkpoint write. The session row itself persists indefinitely (with `is_partial = 1`) for analytics purposes. A cleanup routine at startup prunes checkpoint data older than 48 hours to keep the DB lean.

**Relationship to GAP-50 (05-gaps-cover-letter-interview-implementation-plan.md):** GAP-50 established the database schema (`interview_session_checkpoints` table, `is_partial` and `resumed_from_checkpoint_id` columns on `mock_interview_sessions`) and the core `SessionCheckpointer::save()` / `load()` operations. This plan builds on that foundation and specifies the full feature surface: auto-save scheduling, the `MockInterviewService` orchestrator, timeout handling, the session index browser TUI, and the complete resume flow through `MockInterviewLoop`.

## Prerequisites

### Specs/Plans that must be implemented first
- `specs/interview-prep-mock-loop-implementation-plan.md` — provides `MockInterviewSession`, `MockResponse`, `QuestionFeedback`, `MockInterviewRepository`, `MockInterviewLoop`, `mock_interview_sessions` table (migrations 014–015)
- `specs/05-gaps-cover-letter-interview-implementation-plan.md` — provides `SessionCheckpoint`, `CompletedTurn`, `SessionCheckpointer`, `interview_session_checkpoints` table (migration 021), `is_partial` column, `WorkerParams::Resume`
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — provides `WorkerCommand`, `WorkerEvent`, `CancelToken`, `RalphProcessManager`
- `specs/agentic-ralph-orchestration-implementation-plan.md` — provides `LoopType`, `LoopQueue`, `LoopDispatch`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel system, `KeyContext`, `Clear`-backed overlay dialogs
- `specs/XX-llm-cost-budget-management-implementation-plan.md` — provides `CostTable`, token counting (for resume cost estimation)

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
tokio        = { workspace = true, features = ["sync", "time"] }  # interval + watch
chrono       = { workspace = true, features = ["serde"] }
once_cell    = "1"

# lazyjob-tui/Cargo.toml — new additions
crossterm    = { workspace = true }
ratatui      = { workspace = true }
```

No new external crates are required; all building blocks are already in the workspace.

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|---------------|
| `lazyjob-core` | `MockInterviewService`, `SessionCheckpointer` (owner from GAP-50), `AutoSavePolicy`, `ResumeCheck`, `SessionTimeoutPolicy`, `TimeoutAction`, `PartialSessionSummary`, error types, migrations 021 (from GAP-50) |
| `lazyjob-ralph` | `MockInterviewLoop` resume path (`WorkerParams::Resume`), per-question checkpoint trigger, inactivity watch channel |
| `lazyjob-tui` | `SessionResumePicker` dialog (from GAP-50), `PartialSessionNotificationBanner`, `SessionIndexBrowser`, startup scan integration |
| `lazyjob-cli` | `lazyjob interview sessions` subcommand — lists partial and completed sessions |

`lazyjob-core` has no dependency on `lazyjob-ralph` or `lazyjob-tui`. All domain types flow upward.

### Core Types

```rust
// lazyjob-core/src/interview/resumability.rs

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Summary of a resumable partial session, derived at query time.
/// Not persisted; computed from the session row + latest checkpoint.
#[derive(Debug, Clone)]
pub struct PartialSessionSummary {
    pub session_id:          Uuid,
    pub application_id:      Uuid,
    /// Company name + role title for display, resolved from application.
    pub display_label:       String,
    pub questions_completed: u32,
    pub questions_total:     u32,
    /// When the last checkpoint was written.
    pub last_activity_at:    DateTime<Utc>,
    /// Checkpoint expires this time; None if already expired.
    pub expires_at:          DateTime<Utc>,
    /// Estimated tokens needed to reconstruct LLM context on resume.
    pub estimated_token_cost: u32,
    /// Human-readable cost string e.g. "~$0.02".
    pub estimated_cost_usd:  String,
}

/// Result of a can_resume() check.
#[derive(Debug)]
pub enum ResumeCheck {
    Resumable {
        summary: PartialSessionSummary,
    },
    TooStale {
        /// Hours since last checkpoint.
        hours_stale: i64,
        /// e.g. "Start a new session instead."
        suggestion:  String,
    },
    NoCheckpoint,
}

/// Outcome of a successful resume_session() call.
#[derive(Debug)]
pub struct ResumeResult {
    pub session_id:            Uuid,
    /// Questions already answered (to skip in the loop).
    pub resume_from_idx:       u32,
    /// Full Q&A history for LLM context reconstruction.
    pub completed_turns:       Vec<CompletedTurn>,
    pub estimated_token_cost:  u32,
}

/// Configuration for auto-save behavior. Read from config.toml at startup.
#[derive(Debug, Clone)]
pub struct AutoSavePolicy {
    /// Interval between time-triggered auto-saves (default: 5 minutes).
    pub interval_secs: u64,
    /// Trigger an additional save after every answered question (default: true).
    pub save_after_each_question: bool,
}

impl Default for AutoSavePolicy {
    fn default() -> Self {
        Self {
            interval_secs: 300,
            save_after_each_question: true,
        }
    }
}

/// Configuration for inactivity timeout. Read from config.toml.
#[derive(Debug, Clone)]
pub struct SessionTimeoutPolicy {
    /// Seconds of inactivity before the session is paused (default: 30 minutes).
    pub inactivity_threshold_secs: u32,
    /// Seconds before threshold at which the TUI warns the user (default: 60 seconds).
    pub warn_before_timeout_secs: u32,
    /// Always auto-save before pausing (default: true).
    pub auto_save_before_timeout: bool,
}

impl Default for SessionTimeoutPolicy {
    fn default() -> Self {
        Self {
            inactivity_threshold_secs: 1800,
            warn_before_timeout_secs:  60,
            auto_save_before_timeout:  true,
        }
    }
}

/// Returned by handle_inactivity_check() each tick.
#[derive(Debug)]
pub enum TimeoutAction {
    Continue,
    WarnImminent {
        /// Seconds until the session is paused.
        remaining_secs: u32,
        session_id:     Uuid,
    },
    SessionPaused {
        session_id:             Uuid,
        resume_available_until: DateTime<Utc>,
    },
}
```

```rust
// lazyjob-core/src/interview/mock_interview_service.rs

use crate::interview::{
    mock_session::{MockInterviewSession, SessionScore},
    mock_loop_checkpointing::{CompletedTurn, SessionCheckpoint, SessionCheckpointer},
    resumability::{
        AutoSavePolicy, PartialSessionSummary, ResumeCheck, ResumeResult,
        SessionTimeoutPolicy, TimeoutAction,
    },
};
use crate::llm::cost::CostTable;
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct MockInterviewService {
    pool:           SqlitePool,
    cost_table:     CostTable,
    auto_save:      AutoSavePolicy,
    timeout_policy: SessionTimeoutPolicy,
}

impl MockInterviewService {
    pub fn new(
        pool:           SqlitePool,
        cost_table:     CostTable,
        auto_save:      AutoSavePolicy,
        timeout_policy: SessionTimeoutPolicy,
    ) -> Self {
        Self { pool, cost_table, auto_save, timeout_policy }
    }

    /// Returns all partial sessions whose checkpoints have not yet expired.
    /// Called at TUI startup to surface the resume notification.
    pub async fn list_resumable_sessions(&self) -> Result<Vec<PartialSessionSummary>, InterviewError> {
        // Query: JOIN mock_interview_sessions + interview_session_checkpoints
        // WHERE sessions.is_partial = 1
        //   AND checkpoints.expires_at > datetime('now')
        // ORDER BY checkpoints.created_at DESC
        todo!()
    }

    /// Compute a ResumeCheck for a specific session.
    pub async fn can_resume(&self, session_id: Uuid) -> Result<ResumeCheck, InterviewError> {
        let checkpointer = SessionCheckpointer::new(self.pool.clone());
        match checkpointer.load(session_id.into()).await? {
            None => Ok(ResumeCheck::NoCheckpoint),
            Some(cp) => {
                let hours_stale = (Utc::now() - cp.created_at).num_hours();
                if cp.expires_at < Utc::now() {
                    return Ok(ResumeCheck::TooStale {
                        hours_stale,
                        suggestion: "Start a new session — your checkpoint has expired.".to_string(),
                    });
                }
                let summary = self.build_summary(&cp).await?;
                Ok(ResumeCheck::Resumable { summary })
            }
        }
    }

    /// Load the full CompletedTurn history for a session to reconstruct LLM context.
    pub async fn load_for_resume(&self, session_id: Uuid) -> Result<ResumeResult, InterviewError> {
        match self.can_resume(session_id).await? {
            ResumeCheck::TooStale { .. } => Err(InterviewError::CheckpointExpired(session_id)),
            ResumeCheck::NoCheckpoint    => Err(InterviewError::NoCheckpointFound(session_id)),
            ResumeCheck::Resumable { summary } => {
                let checkpointer = SessionCheckpointer::new(self.pool.clone());
                let cp = checkpointer.load(session_id.into()).await?.unwrap();
                let turns: Vec<CompletedTurn> = serde_json::from_str(&cp.qa_history_json)?;
                Ok(ResumeResult {
                    session_id,
                    resume_from_idx:      cp.checkpoint_idx + 1,
                    completed_turns:      turns,
                    estimated_token_cost: summary.estimated_token_cost,
                })
            }
        }
    }

    /// Handle inactivity check — called from the TUI event loop on a 1-second tick.
    pub fn handle_inactivity_check(
        &self,
        session_id:    Uuid,
        last_activity: chrono::DateTime<Utc>,
    ) -> TimeoutAction {
        let inactive_secs = (Utc::now() - last_activity).num_seconds() as u32;
        let threshold     = self.timeout_policy.inactivity_threshold_secs;
        let warn_at       = threshold.saturating_sub(self.timeout_policy.warn_before_timeout_secs);

        if inactive_secs >= threshold {
            TimeoutAction::SessionPaused {
                session_id,
                resume_available_until: Utc::now() + chrono::Duration::hours(48),
            }
        } else if inactive_secs >= warn_at {
            TimeoutAction::WarnImminent {
                remaining_secs: threshold - inactive_secs,
                session_id,
            }
        } else {
            TimeoutAction::Continue
        }
    }

    /// Prune checkpoints older than 48 hours. Called once at TUI startup.
    pub async fn prune_expired_checkpoints(&self) -> Result<u64, InterviewError> {
        let checkpointer = SessionCheckpointer::new(self.pool.clone());
        checkpointer.prune_expired().await
    }

    // Private helpers

    async fn build_summary(&self, cp: &SessionCheckpoint) -> Result<PartialSessionSummary, InterviewError> {
        let row = sqlx::query!(
            r#"
            SELECT s.application_id, s.questions_total,
                   a.company_name, a.role_title
            FROM   mock_interview_sessions s
            JOIN   applications a ON a.id = s.application_id
            WHERE  s.id = ?
            "#,
            cp.session_id.to_string()
        )
        .fetch_one(&self.pool)
        .await?;

        let token_cost    = cp.token_cost_estimate;
        let cost_usd      = self.cost_table.estimate_usd_display(token_cost);
        let display_label = format!("{} — {}", row.company_name, row.role_title);

        Ok(PartialSessionSummary {
            session_id:           cp.session_id.into(),
            application_id:       row.application_id.parse().unwrap_or_default(),
            display_label,
            questions_completed:  cp.checkpoint_idx + 1,
            questions_total:      row.questions_total as u32,
            last_activity_at:     cp.created_at,
            expires_at:           cp.expires_at,
            estimated_token_cost: token_cost,
            estimated_cost_usd:   cost_usd,
        })
    }
}
```

### Trait Definitions

```rust
// Extension to MockInterviewRepository (lazyjob-core)
// Adds methods needed for the session index browser.

#[async_trait]
pub trait MockInterviewRepository: Send + Sync {
    // --- existing methods from mock-loop plan ---
    async fn create_session(&self, session: &MockInterviewSession) -> Result<(), MockSessionError>;
    async fn save_response(&self, session_id: Uuid, response: &MockResponse) -> Result<(), MockSessionError>;
    async fn complete_session(&self, session_id: Uuid, score: &SessionScore) -> Result<(), MockSessionError>;
    async fn get_sessions_for_application(&self, application_id: Uuid) -> Result<Vec<MockInterviewSession>, MockSessionError>;
    async fn get_score_trend(&self, application_id: Uuid) -> Result<Vec<(DateTime<Utc>, f64)>, MockSessionError>;
    async fn get_session(&self, session_id: Uuid) -> Result<Option<MockInterviewSession>, MockSessionError>;

    // --- new methods for resumability ---

    /// Return all sessions for an application sorted newest-first,
    /// including partial sessions (is_partial = 1).
    async fn list_sessions_with_status(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<SessionListEntry>, MockSessionError>;

    /// Mark a session as discarded by the user.
    /// Soft-deletes: sets is_partial = 0, completed_at = now(), overall_score = NULL.
    async fn discard_partial_session(&self, session_id: Uuid) -> Result<(), MockSessionError>;

    /// Bulk-discard all partial sessions older than a cutoff timestamp.
    async fn discard_stale_partial_sessions(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<u64, MockSessionError>;
}

/// Row in the session list browser. Combines session row + checkpoint status.
#[derive(Debug, Clone)]
pub struct SessionListEntry {
    pub session_id:          Uuid,
    pub started_at:          DateTime<Utc>,
    pub completed_at:        Option<DateTime<Utc>>,
    pub is_partial:          bool,
    pub questions_answered:  u32,
    pub questions_total:     u32,
    pub overall_score:       Option<f64>,
    pub has_valid_checkpoint: bool,
}
```

### SQLite Schema

The core tables were established in GAP-50 (migration 021). This plan adds no new tables but specifies the queries that drive resumability features.

```sql
-- migration 021 (from GAP-50, included here for reference):
CREATE TABLE IF NOT EXISTS interview_session_checkpoints (
    id                  TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL REFERENCES mock_interview_sessions(id) ON DELETE CASCADE,
    checkpoint_idx      INTEGER NOT NULL,
    qa_history_json     TEXT NOT NULL,
    token_cost_estimate INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL
);

-- Unique constraint: only one live checkpoint per session (upsert semantics)
CREATE UNIQUE INDEX IF NOT EXISTS idx_isc_session_unique
    ON interview_session_checkpoints(session_id);

CREATE INDEX IF NOT EXISTS idx_isc_session
    ON interview_session_checkpoints(session_id, created_at DESC);

ALTER TABLE mock_interview_sessions ADD COLUMN is_partial INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mock_interview_sessions ADD COLUMN resumed_from_checkpoint_id TEXT
    REFERENCES interview_session_checkpoints(id) ON DELETE SET NULL;

-- Partial-session index for the startup scan:
-- Only partial, non-expired sessions appear in the resume offer.
CREATE INDEX IF NOT EXISTS idx_mis_partial
    ON mock_interview_sessions(is_partial, application_id)
    WHERE is_partial = 1;
```

**Key queries:**

```sql
-- Startup scan: find all resumable sessions across all applications
-- (used by list_resumable_sessions)
SELECT
    s.id            AS session_id,
    s.application_id,
    s.questions_answered,
    s.questions_total,
    c.checkpoint_idx,
    c.token_cost_estimate,
    c.created_at    AS last_activity_at,
    c.expires_at
FROM   mock_interview_sessions s
JOIN   interview_session_checkpoints c ON c.session_id = s.id
WHERE  s.is_partial = 1
  AND  c.expires_at > datetime('now')
ORDER  BY c.created_at DESC;

-- Discard partial session (soft-delete)
UPDATE mock_interview_sessions
SET    is_partial   = 0,
       completed_at = datetime('now')
WHERE  id = ?
  AND  is_partial = 1;

-- Bulk prune expired checkpoints
DELETE FROM interview_session_checkpoints
WHERE  expires_at <= datetime('now');

-- List sessions with checkpoint presence for session index browser
SELECT
    s.id, s.started_at, s.completed_at, s.is_partial,
    s.questions_answered, s.questions_total, s.overall_score,
    CASE WHEN c.id IS NOT NULL AND c.expires_at > datetime('now')
         THEN 1 ELSE 0 END AS has_valid_checkpoint
FROM mock_interview_sessions s
LEFT JOIN interview_session_checkpoints c ON c.session_id = s.id
WHERE s.application_id = ?
ORDER BY s.started_at DESC;
```

### Module Structure

```
lazyjob-core/
  src/
    interview/
      mod.rs                     -- pub use resumability::*, mock_interview_service::*
      mock_session.rs            -- (existing) MockInterviewSession, SessionScore, etc.
      mock_session_repository.rs -- (extended) list_sessions_with_status, discard_partial_session
      mock_loop_checkpointing.rs -- (from GAP-50) SessionCheckpointer, CompletedTurn
      resumability.rs            -- NEW: PartialSessionSummary, ResumeCheck, ResumeResult,
                                 --      AutoSavePolicy, SessionTimeoutPolicy, TimeoutAction
      mock_interview_service.rs  -- NEW: MockInterviewService orchestrator

lazyjob-ralph/
  src/
    loops/
      mock_interview.rs          -- (extended) resume path, auto-save trigger, inactivity timer

lazyjob-tui/
  src/
    interview/
      session_resume_picker.rs   -- (from GAP-50) confirm resume / new / discard dialog
      partial_session_banner.rs  -- NEW: startup notification banner for partial sessions
      session_index_browser.rs   -- NEW: full session list with resume actions

lazyjob-cli/
  src/
    cmd/
      interview.rs               -- (extended) `lazyjob interview sessions` subcommand
```

---

## Implementation Phases

### Phase 1 — Core Domain + Service Layer (MVP)

**Step 1.1 — `resumability.rs`: Define all new types**

File: `lazyjob-core/src/interview/resumability.rs`

Implement all types from the Core Types section:
- `PartialSessionSummary` (derived, never persisted)
- `ResumeCheck` enum (`Resumable`, `TooStale`, `NoCheckpoint`)
- `ResumeResult` struct
- `AutoSavePolicy` with `Default` impl (interval: 300s, save_after_each_question: true)
- `SessionTimeoutPolicy` with `Default` impl (1800s threshold, 60s warn window)
- `TimeoutAction` enum

Verification: `cargo test -p lazyjob-core -- interview::resumability` with unit tests for `AutoSavePolicy::default()` and `SessionTimeoutPolicy::default()` field values.

**Step 1.2 — `MockInterviewService` core methods**

File: `lazyjob-core/src/interview/mock_interview_service.rs`

Implement:
- `MockInterviewService::new()` — constructor taking `SqlitePool`, `CostTable`, `AutoSavePolicy`, `SessionTimeoutPolicy`
- `list_resumable_sessions()` — runs the startup scan query, joins to `applications` for display label, maps to `Vec<PartialSessionSummary>`. Returns empty vec (not error) when no sessions exist.
- `can_resume(session_id)` — delegates to `SessionCheckpointer::load()`, checks expiry, maps to `ResumeCheck`
- `load_for_resume(session_id)` — validates via `can_resume()`, deserializes `qa_history_json` via `serde_json::from_str::<Vec<CompletedTurn>>()`, returns `ResumeResult`
- `handle_inactivity_check(session_id, last_activity)` — pure sync function, no I/O; computes inactive seconds via `(Utc::now() - last_activity).num_seconds() as u32`, returns `TimeoutAction` variant
- `prune_expired_checkpoints()` — delegates to `SessionCheckpointer::prune_expired()`

Key API calls:
- `sqlx::query!("SELECT ... FROM mock_interview_sessions s JOIN ...")` — typed query macro
- `serde_json::from_str::<Vec<CompletedTurn>>(&cp.qa_history_json)` — deserialize checkpoint history
- `(cp.expires_at < Utc::now())` — expiry check using `chrono::DateTime` ordering

Verification: `#[sqlx::test(migrations = "src/db/migrations")]` tests:
- `can_resume_returns_resumable_when_checkpoint_fresh`
- `can_resume_returns_too_stale_when_checkpoint_expired` (manually insert checkpoint with `expires_at` in the past)
- `list_resumable_sessions_excludes_expired_checkpoints`
- `handle_inactivity_check_returns_continue_when_within_threshold`
- `handle_inactivity_check_returns_warn_imminent_at_warn_window`
- `handle_inactivity_check_returns_session_paused_at_threshold`

**Step 1.3 — `MockInterviewRepository` extension**

File: `lazyjob-core/src/interview/mock_session_repository.rs`

Add to the existing `MockInterviewRepository` trait and `SqliteMockInterviewRepository` impl:
- `list_sessions_with_status(application_id)` — LEFT JOIN sessions + checkpoints, map to `Vec<SessionListEntry>`, sorted `started_at DESC`
- `discard_partial_session(session_id)` — `UPDATE mock_interview_sessions SET is_partial = 0, completed_at = datetime('now') WHERE id = ? AND is_partial = 1`; returns `MockSessionError::NotFound` if the WHERE clause matches zero rows
- `discard_stale_partial_sessions(older_than)` — bulk UPDATE using the same clause plus `WHERE started_at < ?`; returns affected row count

Verification:
- `#[sqlx::test]` — insert 3 sessions (1 completed with score, 1 partial with valid checkpoint, 1 partial with expired checkpoint); assert `list_sessions_with_status` returns all 3 with correct `has_valid_checkpoint` values
- `discard_partial_session` — assert the row no longer appears in `list_resumable_sessions` after discard
- `discard_stale_partial_sessions` — returns correct count

---

### Phase 2 — Auto-Save and Inactivity Timer in MockInterviewLoop

**Step 2.1 — Auto-save trigger per question**

File: `lazyjob-ralph/src/loops/mock_interview.rs`

After each `repo.save_response()` call, trigger a checkpoint save if `policy.save_after_each_question` is true:

```rust
// Inside MockInterviewLoop::run() after save_response():
if self.auto_save_policy.save_after_each_question {
    let turns: Vec<CompletedTurn> = self.build_turns_snapshot(&responses_so_far);
    self.checkpointer.save(self.session_id.into(), &turns).await
        .unwrap_or_else(|e| tracing::warn!("auto-save failed: {e}"));
    // Emit a lightweight UI notification
    self.emit_event(&WorkerEvent::AutoSaved {
        session_id:          self.session_id,
        questions_completed: responses_so_far.len() as u32,
    })?;
}
```

Key decision: auto-save failure is **non-fatal** — it logs a `tracing::warn!` and the loop continues. Users lose at most one question's worth of progress on DB failure, not the full session.

**Step 2.2 — Time-interval auto-save task**

File: `lazyjob-ralph/src/loops/mock_interview.rs`

Spawn a background tokio task using `tokio::time::interval` to auto-save on the configured interval:

```rust
// Spawned before the question loop starts.
let interval_handle = {
    let checkpointer = self.checkpointer.clone();
    let session_id   = self.session_id;
    let policy       = self.auto_save_policy.clone();
    let turns_tx     = Arc::clone(&self.turns_snapshot);  // Arc<Mutex<Vec<CompletedTurn>>>
    let cancel       = self.cancel.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(
            std::time::Duration::from_secs(policy.interval_secs),
        );
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = cancel.changed() => break,
                _ = tick.tick() => {
                    let turns = turns_tx.lock().await.clone();
                    if turns.is_empty() { continue; }
                    checkpointer.save(session_id.into(), &turns).await
                        .unwrap_or_else(|e| tracing::warn!("interval auto-save: {e}"));
                }
            }
        }
    })
};
// Cancel the interval task when the loop exits:
drop(interval_handle); // tokio task drops on handle drop
```

The `turns_snapshot: Arc<Mutex<Vec<CompletedTurn>>>` is updated under the lock after each question completes, ensuring the interval task always saves a consistent snapshot.

**Step 2.3 — Inactivity monitoring channel**

The TUI sends a `last_activity` timestamp via a `tokio::sync::watch::Sender<DateTime<Utc>>` every time the user types. `MockInterviewLoop` owns the receiver:

```rust
// lazyjob-ralph/src/loops/mock_interview.rs

pub struct MockInterviewLoop {
    // ... existing fields ...
    /// Receives last-keystroke timestamps from the TUI event loop.
    pub activity_rx:    tokio::sync::watch::Receiver<DateTime<Utc>>,
    pub timeout_policy: SessionTimeoutPolicy,
}
```

Inside `read_user_response()`, the `tokio::select!` loop checks for inactivity every second:

```rust
async fn read_user_response(&mut self) -> anyhow::Result<Option<String>> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line   = String::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = self.cancel.changed() => {
                if *self.cancel.borrow() { return Ok(None); }
            }
            _ = ticker.tick() => {
                let last_activity = *self.activity_rx.borrow();
                match self.timeout_service.handle_inactivity_check(
                    self.session_id, last_activity
                ) {
                    TimeoutAction::WarnImminent { remaining_secs, session_id } => {
                        self.emit_event(&WorkerEvent::InactivityWarning {
                            session_id,
                            remaining_secs,
                        })?;
                    }
                    TimeoutAction::SessionPaused { session_id, resume_available_until } => {
                        if self.timeout_policy.auto_save_before_timeout {
                            // Trigger a final save synchronously
                            let turns = self.turns_snapshot.lock().await.clone();
                            self.checkpointer.save(session_id.into(), &turns).await
                                .unwrap_or_else(|e| tracing::warn!("timeout save: {e}"));
                        }
                        self.emit_event(&WorkerEvent::SessionTimedOut {
                            session_id,
                            resume_available_until,
                        })?;
                        return Ok(None);
                    }
                    TimeoutAction::Continue => {}
                }
            }
            n = reader.read_line(&mut line) => {
                if n? == 0 { return Ok(None); }
                let cmd: WorkerCommand = serde_json::from_str(line.trim())?;
                line.clear();
                match cmd {
                    WorkerCommand::UserInput { text } => return Ok(Some(text)),
                    WorkerCommand::Cancel             => return Ok(None),
                    _                                 => continue,
                }
            }
        }
    }
}
```

Add to `WorkerEvent`:
```rust
AutoSaved {
    session_id:          Uuid,
    questions_completed: u32,
},
InactivityWarning {
    session_id:     Uuid,
    remaining_secs: u32,
},
SessionTimedOut {
    session_id:             Uuid,
    resume_available_until: DateTime<Utc>,
},
```

Verification:
- Unit test `read_user_response_cancel_after_inactivity`: advance the `watch::Sender<DateTime<Utc>>` to a stale timestamp, assert `SessionTimedOut` event is emitted before `Ok(None)` is returned.
- Unit test `auto_save_interval_task_saves_on_tick`: mock `SessionCheckpointer`, advance time via `tokio::time::advance()` in a `#[tokio::test(start_paused = true)]`, assert `save()` was called.

**Step 2.4 — Resume path in `MockInterviewLoop`**

When the TUI dispatches `WorkerParams::Resume { qa_history, next_question_idx }`, the loop:

1. Skips questions `[0, next_question_idx)` without emitting them to the TUI
2. Injects the prior Q&A as system-context messages before the first new question prompt (not as user/assistant chat turns — as a context preamble in the system prompt)
3. Sets `is_partial = 0` on successful completion via `repo.complete_session()`

```rust
// lazyjob-ralph/src/loops/mock_interview.rs

pub async fn run(&mut self) -> anyhow::Result<()> {
    // Mark session as partial at the start
    sqlx::query!("UPDATE mock_interview_sessions SET is_partial = 1 WHERE id = ?",
        self.session_id.to_string())
        .execute(&self.pool).await?;

    let resume_context = self.resume_from.as_ref().map(|r| {
        build_resume_context_preamble(&r.completed_turns)
    });

    for (idx, question) in questions.iter().enumerate() {
        // Skip already-answered questions (resume path)
        if let Some(ref resume) = self.resume_from {
            if idx < resume.next_question_idx as usize {
                continue;
            }
        }

        // Emit question to TUI
        self.emit_event(&WorkerEvent::MockQuestion {
            question: question.clone(),
            question_number: idx as u32 + 1,
            total_questions:  total as u32,
        })?;

        // Read response (with inactivity polling)
        let Some(response_text) = self.read_user_response().await? else {
            // Cancelled or timed out — session stays is_partial = 1
            return Ok(());
        };

        // Evaluate
        let feedback = self.evaluate_response(question, &response_text, story_ref, resume_context.as_deref()).await?;

        // Persist
        let mock_response = MockResponse { ... };
        repo.save_response(self.session_id, &mock_response).await?;

        // Update turns snapshot for auto-save task
        self.turns_snapshot.lock().await.push(CompletedTurn {
            question:      question.clone(),
            user_response: response_text,
            feedback:      feedback.clone(),
        });

        // Per-question auto-save
        if self.auto_save_policy.save_after_each_question {
            let turns = self.turns_snapshot.lock().await.clone();
            self.checkpointer.save(self.session_id.into(), &turns).await
                .unwrap_or_else(|e| tracing::warn!("per-question save failed: {e}"));
        }
    }

    // All questions answered — finalize
    let score = SessionScore::compute(&responses, total as u32);
    repo.complete_session(self.session_id, &score).await?;

    // Clear is_partial and checkpoint
    sqlx::query!(
        "UPDATE mock_interview_sessions SET is_partial = 0 WHERE id = ?",
        self.session_id.to_string()
    ).execute(&self.pool).await?;

    self.emit_event(&WorkerEvent::MockSessionSummary { session: full_session })?;
    Ok(())
}
```

```rust
/// Build a context preamble from prior turns for injection into the eval prompt.
/// Token cost: ~500 tokens per prior Q&A pair (rough estimate).
fn build_resume_context_preamble(turns: &[CompletedTurn]) -> String {
    let mut parts = vec!["[SESSION RESUMED]\nPrevious Q&A:".to_string()];
    for (i, turn) in turns.iter().enumerate() {
        parts.push(format!(
            "Q{}: {}\nA: {}\n(Score: {})",
            i + 1,
            turn.question.question_text,
            turn.user_response,
            turn.feedback.score
        ));
    }
    parts.join("\n\n")
}
```

---

### Phase 3 — TUI Integration

**Step 3.1 — Startup notification banner**

File: `lazyjob-tui/src/interview/partial_session_banner.rs`

At TUI startup (before rendering the main view), `EventLoop::init()` calls `MockInterviewService::list_resumable_sessions()`. If the result is non-empty, a `PartialSessionBanner` is shown in a `ratatui::widgets::Clear`-backed overlay at the top of the terminal:

```rust
// lazyjob-tui/src/interview/partial_session_banner.rs

use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::style::{Color, Modifier, Style};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct PartialSessionBanner {
    pub sessions:    Vec<PartialSessionSummary>,
    pub list_state:  ListState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BannerAction {
    Resume(Uuid),
    NewSession,
    Discard(Uuid),
    DismissAll,
}

impl PartialSessionBanner {
    pub fn new(sessions: Vec<PartialSessionSummary>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { sessions, list_state }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        // Clear background before rendering banner
        frame.render_widget(Clear, area);

        let items: Vec<ListItem> = self.sessions.iter().map(|s| {
            let label = format!(
                "{} — {}/{} questions — {} ago — cost: {} (~{})",
                s.display_label,
                s.questions_completed,
                s.questions_total,
                format_time_ago(s.last_activity_at),
                s.estimated_token_cost,
                s.estimated_cost_usd,
            );
            ListItem::new(label)
        }).collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("  Partial sessions — press [r]esume / [n]ew / [d]iscard / [Esc]  ")
                    .style(Style::default().fg(Color::Yellow)),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> Option<BannerAction> {
        use crossterm::event::KeyCode;
        match key {
            KeyCode::Up   | KeyCode::Char('k') => { self.list_state.select_previous(); None }
            KeyCode::Down | KeyCode::Char('j') => { self.list_state.select_next(); None }
            KeyCode::Char('r') | KeyCode::Enter => {
                self.list_state.selected()
                    .and_then(|i| self.sessions.get(i))
                    .map(|s| BannerAction::Resume(s.session_id))
            }
            KeyCode::Char('n') => Some(BannerAction::NewSession),
            KeyCode::Char('d') => {
                self.list_state.selected()
                    .and_then(|i| self.sessions.get(i))
                    .map(|s| BannerAction::Discard(s.session_id))
            }
            KeyCode::Esc => Some(BannerAction::DismissAll),
            _ => None,
        }
    }
}
```

The banner is rendered **after** the main layout is drawn using `frame.render_widget(Clear, banner_area)` to clear the background. The area is a horizontally centered, vertically top-aligned `Rect` of height `min(sessions.len() + 4, 12)` to avoid covering the full screen.

**Step 3.2 — `SessionResumePicker` (from GAP-50)**

File: `lazyjob-tui/src/interview/session_resume_picker.rs`

This dialog (specified in GAP-50) is shown when the user explicitly navigates to the mock interview for a specific application that has a partial session. It renders a single-session confirmation:

```
┌─ Resume Session? ────────────────────────────────────────────────────┐
│  You have a partial session from 2h ago:                             │
│    Company: Stripe  •  Role: Backend Engineer  •  5 of 8 questions   │
│    Est. resume cost: ~520 tokens (~$0.02)                            │
│                                                                       │
│  [r] Resume   [n] New session   [d] Discard checkpoint               │
└───────────────────────────────────────────────────────────────────────┘
```

Use `ratatui::widgets::Clear` to erase background before rendering. Dispatch:
- `[r]` / `Enter` → `SessionAction::Resume(session_id)`
- `[n]` → `SessionAction::NewSession`
- `[d]` → `SessionAction::Discard(session_id)` (calls `discard_partial_session()`)

**Step 3.3 — `SessionIndexBrowser` — full session list view**

File: `lazyjob-tui/src/interview/session_index_browser.rs`

A full-panel session browser accessible via `lazyjob interview sessions` CLI or from the TUI's application detail panel:

```
┌─ Mock Interview Sessions — Stripe / Backend Engineer ───────────────────────┐
│  [ID]   Status       Questions  Score   Started         Actions             │
│  ────── ──────────── ─────────  ─────── ──────────────  ─────────────────  │
│  abc123  ● Partial   5 / 8      —       Apr 14 14:30    [r]esume  [d]iscard │
│  def456  ✓ Complete  8 / 8      7.2     Apr 13 10:00    [v]iew              │
│  ghi789  ✗ Discarded 3 / 8      —       Apr 10 09:00    [v]iew              │
└─────────────────────────────────────────────────────────────────────────────┘
│  [j/k] navigate  [r] resume  [d] discard  [v] view detail  [q] back        │
```

Implementation:
- Queries `MockInterviewRepository::list_sessions_with_status(application_id)`
- Renders via `ratatui::widgets::Table` with `TableState` for selection
- Status column uses `StatusSymbol::for_session_status()` (defined in accessibility plan) — `●` (Yellow) partial, `✓` (Green) complete, `✗` (DarkGray) discarded
- `[r]` only enabled when `has_valid_checkpoint` and `is_partial`; key handler returns `SessionIndexAction::Noop` otherwise with a status-bar flash "No valid checkpoint — start new session"

**Step 3.4 — Inactivity warning banner in active session**

File: `lazyjob-tui/src/views/mock_interview/mod.rs`

The `MockInterviewView` subscribes to `WorkerEvent::InactivityWarning` from the Ralph broadcast channel. When received:
- Render a one-line yellow warning bar at the bottom of the view: `"Session inactive — will pause in {remaining_secs}s — press any key to continue"`
- The bar disappears on the next `WorkerEvent::AutoSaved` or any user keypress forwarded via `WorkerCommand::UserInput`

When `WorkerEvent::SessionTimedOut` is received:
- The view transitions to `MockInterviewViewState::Paused`
- Show: `"Session paused — resume available for 48h from last save"`
- Offer `[r] Resume` / `[q] Quit` keys

---

### Phase 4 — Token Cost Transparency

**Step 4.1 — `estimate_resume_tokens()`**

File: `lazyjob-core/src/interview/mock_interview_service.rs`

```rust
impl MockInterviewService {
    /// Estimate the total token cost to resume a session from its latest checkpoint.
    /// Called by build_summary() and displayed in the UI before the user confirms.
    pub fn estimate_resume_tokens(turns: &[CompletedTurn]) -> u32 {
        turns.iter().map(|t| {
            let question_chars = t.question.question_text.len();
            let answer_chars   = t.user_response.len();
            let feedback_chars = serde_json::to_string(&t.feedback)
                .unwrap_or_default()
                .len();
            // 1 token ≈ 4 chars (rough estimate, consistent across providers)
            ((question_chars + answer_chars + feedback_chars) / 4) as u32
        }).sum::<u32>()
        // Add a fixed system prompt overhead (the resume context preamble header)
        + 50
    }

    /// Format estimated token cost as a human-readable USD string.
    /// Delegates to CostTable for the current provider/model.
    pub fn format_cost_usd(&self, tokens: u32) -> String {
        let microdollars = self.cost_table.estimate_input_cost_microdollars(tokens);
        if microdollars < 100 {
            "< $0.01".to_string()
        } else {
            format!("~${:.2}", microdollars as f64 / 1_000_000.0)
        }
    }
}
```

`CostTable::estimate_input_cost_microdollars(tokens: u32) -> i64` is defined in the LLM cost plan. The estimate is computed solely from input tokens because the resume context preamble is injected as input, not output.

**Step 4.2 — Token warning threshold**

In `SessionResumePicker` and `PartialSessionBanner`, when `estimated_token_cost > 5000`:

```rust
let cost_style = if summary.estimated_token_cost > 5000 {
    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
} else {
    Style::default().fg(Color::DarkGray)
};
```

At > 10,000 tokens add an explicit line: `"High context cost — long session history"` (Yellow bold).

---

### Phase 5 — Cleanup and CLI

**Step 5.1 — Startup prune**

File: `lazyjob-tui/src/app.rs` (or `lazyjob-cli/src/startup.rs`)

In `App::init()`, before rendering:

```rust
// Prune expired checkpoints silently at startup
service.prune_expired_checkpoints().await
    .unwrap_or_else(|e| tracing::warn!("checkpoint prune failed: {e}"));
```

Also call `discard_stale_partial_sessions(Utc::now() - chrono::Duration::days(30))` to clean up very old partial session rows that somehow escaped checkpoint pruning (e.g., checkpoint was never written).

**Step 5.2 — `lazyjob interview sessions` CLI subcommand**

File: `lazyjob-cli/src/cmd/interview.rs`

```
$ lazyjob interview sessions [--application <id>] [--status partial|complete|all]

  abc123  PARTIAL   5/8   Apr 14 14:30   Stripe — Backend Engineer
  def456  COMPLETE  8/8   Apr 13 10:00   Stripe — Backend Engineer  score: 7.2
```

Calls `MockInterviewRepository::list_sessions_with_status()` and formats as a plain text table with no TUI dependency. `--status partial` filters to resumable sessions only.

**Step 5.3 — `lazyjob interview discard <session-id>` CLI**

```rust
// lazyjob-cli/src/cmd/interview.rs

async fn cmd_discard(pool: &SqlitePool, session_id: Uuid) -> anyhow::Result<()> {
    let repo = SqliteMockInterviewRepository::new(pool.clone());
    repo.discard_partial_session(session_id).await
        .context("failed to discard session")?;
    // Also delete checkpoint if it exists
    sqlx::query!(
        "DELETE FROM interview_session_checkpoints WHERE session_id = ?",
        session_id.to_string()
    ).execute(pool).await?;
    println!("Session {session_id} discarded.");
    Ok(())
}
```

---

## Key Crate APIs

- `sqlx::query!("SELECT ... JOIN interview_session_checkpoints ...")` — typed query macro for startup scan
- `sqlx::query!("... ON CONFLICT(session_id) DO UPDATE SET ...")` — single-row checkpoint upsert (from GAP-50's `SessionCheckpointer::save()`)
- `serde_json::from_str::<Vec<CompletedTurn>>(&cp.qa_history_json)` — deserialize Q&A history for LLM context
- `chrono::Duration::hours(48)` — checkpoint expiry window; `cp.expires_at < Utc::now()` — expiry check
- `(Utc::now() - last_activity).num_seconds()` — inactivity duration for `handle_inactivity_check()`
- `tokio::time::interval(Duration::from_secs(300))` — 5-minute auto-save timer
- `tokio::time::MissedTickBehavior::Skip` — prevent burst saves after sleep/wake
- `tokio::select! { biased; _ = cancel.changed() => ..., _ = ticker.tick() => ..., n = reader.read_line() => ... }` — cancel-aware inactivity + stdin select
- `tokio::sync::watch::channel::<DateTime<Utc>>(Utc::now())` — last-activity broadcast channel
- `Arc<Mutex<Vec<CompletedTurn>>>` — turns snapshot shared between main loop and interval task
- `ratatui::widgets::Clear` — erase terminal background before rendering banner overlay
- `ratatui::widgets::List::new(items).highlight_style(...)` — session picker list with keyboard navigation
- `ratatui::widgets::Table` — session index browser with status, questions, score columns
- `ratatui::layout::Layout::default().direction(Direction::Horizontal).constraints([...]).split(area)` — session browser layout

---

## Error Handling

```rust
// lazyjob-core/src/interview/resumability.rs

#[derive(thiserror::Error, Debug)]
pub enum InterviewError {
    #[error("session checkpoint expired for session {0}")]
    CheckpointExpired(Uuid),

    #[error("no checkpoint found for session {0}")]
    NoCheckpointFound(Uuid),

    #[error("session not resumable: {reason}")]
    NotResumable { reason: String },

    #[error("session not found: {0}")]
    SessionNotFound(Uuid),

    #[error("partial session discard failed: session {0} not found or already finalized")]
    DiscardFailed(Uuid),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

All auto-save failures are **non-fatal** — they produce `tracing::warn!` and the loop continues. The session state is always valid from SQLite's perspective (responses are written row-by-row and committed individually), so at worst the user loses context-reconstruction fidelity on resume, not the responses themselves.

`InterviewError::CheckpointExpired` and `NotResumable` are surfaced as dismissable error dialogs in the TUI, not crashes. The user is offered "Start a new session" as the fallback action.

---

## Testing Strategy

### Unit Tests (`lazyjob-core`)

- `MockInterviewService::handle_inactivity_check_continue` — `last_activity = 5 mins ago`, `threshold = 30 mins` → `TimeoutAction::Continue`
- `MockInterviewService::handle_inactivity_check_warn_imminent` — `last_activity = 29 mins ago` → `TimeoutAction::WarnImminent { remaining_secs: 60 }`
- `MockInterviewService::handle_inactivity_check_paused` — `last_activity = 31 mins ago` → `TimeoutAction::SessionPaused { ... }`
- `estimate_resume_tokens_empty_turns` — zero turns → returns 50 (preamble overhead only)
- `estimate_resume_tokens_three_turns` — assert total is sum of character lengths / 4 + 50
- `format_cost_usd_under_threshold` — `microdollars = 50` → `"< $0.01"`
- `format_cost_usd_over_threshold` — `microdollars = 25_000` → `"~$0.03"`

### Integration Tests with `#[sqlx::test]` (`lazyjob-core`)

- `can_resume_returns_resumable` — insert session + fresh checkpoint; assert `ResumeCheck::Resumable` returned
- `can_resume_returns_too_stale` — insert expired checkpoint; assert `ResumeCheck::TooStale`
- `can_resume_no_checkpoint` — insert session with no checkpoint; assert `ResumeCheck::NoCheckpoint`
- `list_resumable_sessions_excludes_completed` — insert 1 partial + 1 complete; assert list returns only partial
- `list_resumable_sessions_excludes_expired_checkpoints` — partial session with expired checkpoint; assert empty list
- `discard_partial_session_clears_is_partial_flag` — insert partial; discard; assert `is_partial = 0`
- `discard_partial_session_returns_error_when_not_found` — assert `DiscardFailed` for unknown ID
- `prune_expired_checkpoints_removes_stale_rows` — insert 3 checkpoints: 2 expired, 1 fresh; prune; assert 1 row remains

### Integration Tests (`lazyjob-ralph`)

- `mock_interview_loop_saves_checkpoint_after_question` — after 1 question answered, assert `interview_session_checkpoints` has 1 row with `checkpoint_idx = 0`
- `mock_interview_loop_resume_skips_prior_questions` — start with `resume_from = { next_idx: 2, completed_turns: [...] }`; assert only questions 3–N are emitted as `WorkerEvent::MockQuestion`
- `mock_interview_loop_timeout_saves_before_exit` — advance `activity_rx` watch to `> threshold`; assert checkpoint is saved before `SessionTimedOut` is emitted
- `auto_save_interval_fires_on_tick` — `#[tokio::test(start_paused = true)]`; advance clock by 5 minutes; assert `checkpointer.save()` was called (mock checkpointer via `Arc<Mutex<Vec<_>>>` call log)
- `cancel_before_answer_leaves_is_partial_set` — send `WorkerCommand::Cancel` before first response; assert session row `is_partial = 1` and `completed_at IS NULL`

### TUI Tests (`lazyjob-tui`)

- `partial_session_banner_renders_session_label` — construct `PartialSessionBanner` with one summary; render to `ratatui::backend::TestBackend`; assert rendered buffer contains `display_label`
- `partial_session_banner_resume_action_on_r_key` — call `handle_key(KeyCode::Char('r'))`; assert `Some(BannerAction::Resume(session_id))`
- `partial_session_banner_discard_action_on_d_key` — assert `Some(BannerAction::Discard(session_id))`
- `session_resume_picker_renders_token_estimate` — assert rendered buffer contains `"~$0.02"` when `estimated_token_cost = 500`
- `session_index_browser_partial_row_shows_resume_key` — construct list with 1 partial `SessionListEntry` with `has_valid_checkpoint = true`; render; assert buffer contains `[r]esume`
- `session_index_browser_complete_row_shows_view_key` — render with complete session; assert buffer contains `[v]iew` but NOT `[r]esume`

---

## Open Questions

1. **Multiple concurrent partial sessions per application**: The current design allows multiple partial sessions for the same application (e.g., user started one, switched to a new session, both are partial). Decision pending: (a) allow multiple (current plan — simplest) or (b) enforce at most one partial per application at the DB level via a `UNIQUE(application_id) WHERE is_partial = 1` partial index. Option (b) requires `discard_partial_session()` to be called automatically on `create_session()`. Lean toward (a) for MVP, (b) post-MVP.

2. **Session merging**: Not planned. The `qa_history_json` blob is a complete snapshot per checkpoint; merging two sessions would require manually reordering questions. Deferred indefinitely.

3. **Checkpoint retention for completed sessions**: Currently, `prune_expired_checkpoints()` prunes any checkpoint older than 48 hours, including those belonging to completed sessions. This is safe because completed sessions don't need a checkpoint. Verify: assert checkpoint is deleted after `complete_session()` (call `DELETE FROM interview_session_checkpoints WHERE session_id = ?` at completion time).

4. **Token cost accuracy**: `estimate_resume_tokens()` uses 4 chars/token which is a rough average. Claude models use a subword tokenizer (tiktoken-style), and longer technical answers may have more tokens per character. Consider wrapping with a note: "estimated — actual cost may vary ±30%". No fix needed for MVP.

5. **Resume context injection strategy**: The current plan injects prior Q&A as a system-context preamble string (not as alternating chat turns). An alternative is injecting as `ChatMessage` history (`role: "user"/"assistant"` alternating). The preamble approach is simpler and avoids the risk of the LLM "continuing" old feedback. Revisit if resume fidelity is reported as low.

6. **Session expiry configurable vs. hardcoded**: `expires_at = created_at + 48h` is hardcoded in `SessionCheckpointer::save()`. Consider adding `checkpoint_ttl_hours: u32` to `AutoSavePolicy` for user configurability. Deferred to post-MVP to keep the config surface small.

---

## Related Specs

- [specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md) — base mock session types, `MockInterviewLoop`, `mock_interview_sessions` table
- [specs/interview-prep-mock-loop-implementation-plan.md](./interview-prep-mock-loop-implementation-plan.md) — Phase 5 (resumability stub) provides the starting design
- [specs/05-gaps-cover-letter-interview-implementation-plan.md](./05-gaps-cover-letter-interview-implementation-plan.md) — GAP-50 establishes `SessionCheckpoint`, `CompletedTurn`, `SessionCheckpointer`, migration 021
- [specs/agentic-ralph-subprocess-protocol-implementation-plan.md](./agentic-ralph-subprocess-protocol-implementation-plan.md) — `WorkerCommand`, `WorkerEvent`, `CancelToken`, `RalphProcessManager`
- [specs/XX-llm-cost-budget-management-implementation-plan.md](./XX-llm-cost-budget-management-implementation-plan.md) — `CostTable::estimate_input_cost_microdollars()`
- [specs/09-tui-design-keybindings-implementation-plan.md](./09-tui-design-keybindings-implementation-plan.md) — `App`, panel system, `Clear`-backed overlay dialogs
- [specs/XX-tui-accessibility-implementation-plan.md](./XX-tui-accessibility-implementation-plan.md) — `StatusSymbol` for session status icons
