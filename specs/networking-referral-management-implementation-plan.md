# Implementation Plan: Networking Referral Management

## Status
Draft

## Related Spec
[specs/networking-referral-management.md](networking-referral-management.md)

## Overview

The networking referral management module implements a lightweight CRM lifecycle tracker that
models the relationship-warming arc from initial contact identification through referral resolution.
It augments the existing `profile_contacts` table with a `RelationshipStage` state machine and
creates a per-(contact, job) `referral_asks` table for tracking referral request state. The module
is designed around two invariants: (1) every user action to advance the relationship is explicit —
no stage advances automatically without the user marking it, and (2) every suggestion to ask for a
referral passes all five readiness gates before surfacing in the TUI.

The background engine is `NetworkingReminderPoller`, a tokio task in `lazyjob-ralph` that runs on
a configurable interval (default 24 h), queries contacts in actionable stages, applies the
anti-spam gate (≤2 reminders per contact per rolling 30-day window), and broadcasts
`WorkflowEvent::NetworkingReminderDue` on the shared channel consumed by the TUI. The poller
integrates directly with `GhostDetector` so it never suggests spending a contact's social capital
on a job with `ghost_score ≥ 0.6`.

The TUI exposes a networking dashboard view that groups contacts by company with stage badges,
and a contact detail panel with an interaction log (`l` to log interaction). The module integrates
with the `ApplicationStateMachine` via `PostTransitionSuggestion::UpdateReferralOutcome` so that
when an application reaches `Offered` or `Rejected`, the user is prompted to record the referral
outcome that fed the application.

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — `run_migrations`, connection pool
- `specs/networking-connection-mapping-implementation-plan.md` — `ProfileContact`, `ContactRepository`, `ConnectionTier`, `normalize_company_name()`, migration creating `profile_contacts`
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobRecord`, `JobRepository`, `JobId`
- `specs/job-search-ghost-job-detection-implementation-plan.md` — `GhostDetector`, `ghost_score` field on `JobRecord`
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `StageTransitionEvent`
- `specs/application-workflow-actions-implementation-plan.md` — `WorkflowEvent`, `PostTransitionSuggestion`, broadcast channel wiring
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel system, broadcast subscriber

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# No new crates — all required crates are already declared:
# uuid, chrono, serde, serde_json, sqlx, thiserror, anyhow, tokio,
# async-trait, once_cell, tracing — from prior modules
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `RelationshipStage`, `ReferralAsk`, `ReferralOutcome`, `ReferralReadiness`, `NetworkingReminder`, `NetworkingReminderAction` | `lazyjob-core` | `src/networking/referral/types.rs` |
| `ReferralRepository` trait + `SqliteReferralRepository` | `lazyjob-core` | `src/networking/referral/repo.rs` |
| `ReferralReadinessChecker` | `lazyjob-core` | `src/networking/referral/readiness.rs` |
| `InteractionLog` + `InteractionRepository` | `lazyjob-core` | `src/networking/referral/interaction.rs` |
| `RelationshipService` (orchestrator) | `lazyjob-core` | `src/networking/referral/service.rs` |
| SQLite migration (017) | `lazyjob-core` | `migrations/017_referral_management.sql` |
| `NetworkingReminderPoller` | `lazyjob-ralph` | `src/networking_poller.rs` |
| TUI networking dashboard | `lazyjob-tui` | `src/views/networking/dashboard.rs` |
| TUI contact detail panel | `lazyjob-tui` | `src/views/networking/contact_detail.rs` |
| Module re-export facade | `lazyjob-core` | `src/networking/referral/mod.rs` |

### Core Types

```rust
// lazyjob-core/src/networking/referral/types.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::networking::ContactId;
use crate::discovery::JobId;

// ── Newtype IDs ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct ReferralAskId(pub Uuid);
impl ReferralAskId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct InteractionLogId(pub Uuid);
impl InteractionLogId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

// ── Relationship state machine ───────────────────────────────────────────────

/// Linear state machine representing how far the relationship with a contact
/// has progressed. Stored as TEXT in SQLite via sqlx::Type derive.
///
/// Transitions:
///   Identified → Contacted  (user marks outreach sent)
///   Contacted  → Replied    (user marks response received)
///   Replied    → Warmed     (user logs substantive interaction: call/coffee chat)
///   Warmed     → ReferralAsked (user marks referral ask sent for a specific job)
///   ReferralAsked → ReferralResolved (user records referral outcome)
///
/// Note: a contact can be Warmed and have zero ReferralAsked rows — the stage
/// tracks the contact's *relationship depth*, not any specific referral ask.
/// Multiple per-(contact, job) referral asks are in the `referral_asks` table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "snake_case")]
pub enum RelationshipStage {
    Identified,
    Contacted,
    Replied,
    Warmed,
    ReferralAsked,
    ReferralResolved,
}

impl RelationshipStage {
    /// Returns true if this stage is eligible for outreach reminders.
    pub fn is_active_for_reminders(&self) -> bool {
        matches!(self, Self::Contacted | Self::Replied | Self::Warmed)
    }

    /// The stage that results from logging an interaction in the current stage.
    /// Returns None if the stage doesn't advance on interaction.
    pub fn advance_on_interaction(&self) -> Option<RelationshipStage> {
        match self {
            Self::Contacted => Some(Self::Replied),
            Self::Replied   => Some(Self::Warmed),
            _               => None,
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Identified      => "identified",
            Self::Contacted       => "contacted",
            Self::Replied         => "replied",
            Self::Warmed          => "warmed",
            Self::ReferralAsked   => "referral_asked",
            Self::ReferralResolved => "referral_resolved",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, ReferralError> {
        match s {
            "identified"        => Ok(Self::Identified),
            "contacted"         => Ok(Self::Contacted),
            "replied"           => Ok(Self::Replied),
            "warmed"            => Ok(Self::Warmed),
            "referral_asked"    => Ok(Self::ReferralAsked),
            "referral_resolved" => Ok(Self::ReferralResolved),
            other => Err(ReferralError::InvalidStage(other.to_string())),
        }
    }
}

// ── Referral ask ─────────────────────────────────────────────────────────────

/// Per-(contact, job) referral request record. A single contact can have
/// referral asks for multiple roles; the UNIQUE(contact_id, job_id) constraint
/// prevents duplicate asks for the same pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralAsk {
    pub id:                  ReferralAskId,
    pub contact_id:          ContactId,
    pub job_id:              JobId,
    pub asked_at:            NaiveDate,
    pub outcome:             Option<ReferralOutcome>,
    pub outcome_recorded_at: Option<NaiveDate>,
    pub notes:               Option<String>,
    pub created_at:          DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "snake_case")]
pub enum ReferralOutcome {
    /// Contact referred the user; reinforces relationship value.
    Succeeded,
    /// Contact declined the referral ask.
    Declined,
    /// No response to the referral ask within 21 days.
    NoResponse,
    /// Application progressed without a referral (e.g., applied direct).
    NotApplicable,
}

// ── Referral readiness ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferralReadiness {
    Ready {
        /// Date on which the ask is recommended.
        recommended_ask_date: NaiveDate,
    },
    NotYet {
        reason:              NotYetReason,
        suggested_wait_days: u32,
    },
    Skip {
        reason: SkipReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotYetReason {
    /// Stage is below Warmed — relationship not deep enough.
    RelationshipTooNew,
    /// Stage is Warmed but last interaction was < 7 days ago (too soon after last touch).
    TooSoonAfterLastContact,
    /// Stage is Warmed but last interaction was > 180 days ago (relationship stale).
    RelationshipStale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Ghost score ≥ 0.6 — don't spend social capital on a likely-ghost job.
    GhostJobDetected,
    /// A referral ask already exists in `referral_asks` for this (contact, job) pair.
    ReferralAlreadyAsked,
    /// User has already applied to this job directly.
    AlreadyApplied,
    /// Job status is Closed.
    JobClosed,
    /// `follow_up_exhausted = true` — poller will no longer remind for this contact.
    FollowUpExhausted,
}

// ── Networking reminder ──────────────────────────────────────────────────────

/// Emitted by `NetworkingReminderPoller` and broadcast as
/// `WorkflowEvent::NetworkingReminderDue`. The TUI renders these in the
/// morning digest panel alongside application reminders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkingReminder {
    pub contact_id:                  ContactId,
    pub contact_name:                String,
    pub company_name:                String,
    pub current_stage:               RelationshipStage,
    pub days_since_last_interaction: u32,
    pub suggested_action:            NetworkingReminderAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkingReminderAction {
    /// Send a follow-up message to a contact who hasn't responded.
    SendFollowUp,
    /// Log a substantive interaction to advance from Replied → Warmed.
    LogInteraction,
    /// Contact is ready — suggest asking for a referral for this job.
    AskForReferral { job_id: JobId, job_title: String },
    /// Application reached Offered/Rejected — record the referral outcome.
    RecordOutcome { referral_ask_id: ReferralAskId },
}

// ── Interaction log ──────────────────────────────────────────────────────────

/// A single interaction event logged by the user. Used for computing
/// `days_since_last_interaction` and for driving stage advancement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionLog {
    pub id:            InteractionLogId,
    pub contact_id:    ContactId,
    pub interacted_at: NaiveDate,
    pub note:          Option<String>,
    pub created_at:    DateTime<Utc>,
}

// ── New outreach weekly cap ──────────────────────────────────────────────────

/// Config-driven cap for new contacts contacted per week.
/// Enforced in `RelationshipService::can_send_new_outreach()`.
/// Configurable in lazyjob.toml as `networking.max_new_outreach_per_week`.
pub const DEFAULT_MAX_NEW_OUTREACH_PER_WEEK: u32 = 5;

/// Reminder anti-spam cap: max reminders per contact per 30-day rolling window.
pub const DEFAULT_MAX_REMINDERS_PER_CONTACT_PER_MONTH: u8 = 2;
```

### Trait Definitions

```rust
// lazyjob-core/src/networking/referral/repo.rs

use async_trait::async_trait;

/// Repository for `referral_asks` table operations.
#[async_trait]
pub trait ReferralRepository: Send + Sync {
    /// Insert a new referral ask. Returns `ReferralError::AlreadyAsked` if a row
    /// with the same (contact_id, job_id) already exists.
    async fn create_referral_ask(&self, ask: &ReferralAsk) -> Result<(), ReferralError>;

    /// Record the outcome of a referral ask.
    async fn record_outcome(
        &self,
        ask_id:   &ReferralAskId,
        outcome:  ReferralOutcome,
        recorded_at: NaiveDate,
    ) -> Result<(), ReferralError>;

    /// Find all open referral asks (outcome IS NULL) for a given contact.
    async fn list_open_asks_for_contact(
        &self,
        contact_id: &ContactId,
    ) -> Result<Vec<ReferralAsk>, ReferralError>;

    /// Find all open referral asks linked to an application job.
    async fn list_open_asks_for_job(
        &self,
        job_id: &JobId,
    ) -> Result<Vec<ReferralAsk>, ReferralError>;

    /// Check if a referral ask exists for (contact_id, job_id).
    async fn ask_exists(
        &self,
        contact_id: &ContactId,
        job_id:     &JobId,
    ) -> Result<bool, ReferralError>;
}

/// Repository for `contact_interaction_log` table.
#[async_trait]
pub trait InteractionRepository: Send + Sync {
    async fn log_interaction(&self, log: &InteractionLog) -> Result<(), ReferralError>;

    /// Returns the most recent interaction date for a contact, or None.
    async fn last_interaction_date(
        &self,
        contact_id: &ContactId,
    ) -> Result<Option<NaiveDate>, ReferralError>;

    /// Returns the count of interactions logged for a contact.
    async fn interaction_count(
        &self,
        contact_id: &ContactId,
    ) -> Result<u32, ReferralError>;
}

/// Extends `ContactRepository` (from connection-mapping plan) with
/// relationship-stage mutation methods. Placed here to avoid a cyclic
/// dependency between the connection-mapping and referral-management modules.
#[async_trait]
pub trait RelationshipRepository: Send + Sync {
    /// Update the relationship stage column.
    async fn update_stage(
        &self,
        contact_id: &ContactId,
        new_stage:  RelationshipStage,
    ) -> Result<(), ReferralError>;

    /// Mark `follow_up_exhausted = true` for a contact.
    async fn mark_follow_up_exhausted(
        &self,
        contact_id: &ContactId,
    ) -> Result<(), ReferralError>;

    /// Increment `reminder_count_this_month`. Resets if `reminder_window_start`
    /// is > 30 days ago (rolling window).
    async fn increment_reminder_count(
        &self,
        contact_id: &ContactId,
        today:      NaiveDate,
    ) -> Result<u8, ReferralError>;  // returns new count after increment

    /// Return all contacts in stages eligible for reminders, with their
    /// current reminder counts.
    async fn list_contacts_for_sweep(&self) -> Result<Vec<ContactReminderRow>, ReferralError>;
}

/// Projected row for the poller sweep query — avoids loading full ProfileContact.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContactReminderRow {
    pub contact_id:               String,   // UUID as TEXT
    pub contact_name:             String,
    pub company_name:             String,
    pub relationship_stage:       String,   // as db str
    pub last_contacted_at:        Option<NaiveDate>,
    pub follow_up_exhausted:      bool,
    pub reminder_count_this_month: u8,
    pub reminder_window_start:    Option<NaiveDate>,
}
```

### SQLite Schema

```sql
-- Migration: 017_referral_management.sql

-- ── Extend profile_contacts with relationship tracking columns ──────────────

ALTER TABLE profile_contacts
  ADD COLUMN relationship_stage     TEXT NOT NULL DEFAULT 'identified';

ALTER TABLE profile_contacts
  ADD COLUMN follow_up_exhausted    INTEGER NOT NULL DEFAULT 0;  -- BOOLEAN

ALTER TABLE profile_contacts
  ADD COLUMN reminder_count_this_month INTEGER NOT NULL DEFAULT 0;

ALTER TABLE profile_contacts
  ADD COLUMN reminder_window_start  DATE;

-- Index for the poller sweep — only active-for-reminders stages.
CREATE INDEX IF NOT EXISTS idx_profile_contacts_stage_active
  ON profile_contacts (relationship_stage)
  WHERE relationship_stage IN ('contacted', 'replied', 'warmed')
    AND follow_up_exhausted = 0;

-- ── Per-(contact, job) referral ask tracking ────────────────────────────────

CREATE TABLE IF NOT EXISTS referral_asks (
    id                   TEXT PRIMARY KEY,          -- UUID
    contact_id           TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    job_id               TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    asked_at             DATE NOT NULL,
    outcome              TEXT,                      -- NULL until resolved
    outcome_recorded_at  DATE,
    notes                TEXT,
    created_at           DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,

    UNIQUE(contact_id, job_id)
);

CREATE INDEX IF NOT EXISTS idx_referral_asks_contact
  ON referral_asks (contact_id);

CREATE INDEX IF NOT EXISTS idx_referral_asks_job
  ON referral_asks (job_id);

-- Open asks only — used by `list_open_asks_for_job`.
CREATE INDEX IF NOT EXISTS idx_referral_asks_open
  ON referral_asks (job_id)
  WHERE outcome IS NULL;

-- ── Contact interaction log ──────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS contact_interaction_log (
    id             TEXT PRIMARY KEY,                -- UUID
    contact_id     TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    interacted_at  DATE NOT NULL,
    note           TEXT,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_contact_interaction_log_contact
  ON contact_interaction_log (contact_id, interacted_at DESC);
```

### Module Structure

```
lazyjob-core/
  src/
    networking/
      referral/
        mod.rs          # pub use re-exports
        types.rs        # all domain types (above)
        repo.rs         # trait definitions
        readiness.rs    # ReferralReadinessChecker
        interaction.rs  # InteractionLog helpers, SqliteInteractionRepository
        service.rs      # RelationshipService (orchestrator)
        sqlite.rs       # SqliteReferralRepository + SqliteRelationshipRepository impls
  migrations/
    017_referral_management.sql

lazyjob-ralph/
  src/
    networking_poller.rs   # NetworkingReminderPoller

lazyjob-tui/
  src/
    views/
      networking/
        dashboard.rs       # NetworkingDashboardView
        contact_detail.rs  # ContactDetailPanel (interaction log, stage badges)
```

## Implementation Phases

### Phase 1 — Domain Types, Schema, Repositories (MVP Foundation)

#### Step 1.1 — SQLite migration

File: `lazyjob-core/migrations/017_referral_management.sql`

Apply the DDL block from the schema section above. Register the migration in the
`run_migrations` function by adding `include_str!("../migrations/017_referral_management.sql")`
to the ordered migration array.

**Verification**: `cargo test -p lazyjob-core -- migration` runs all migration tests. The schema
tests should create a fresh in-memory SQLite, apply all migrations, and assert all new columns
and tables exist via `SELECT * FROM sqlite_master WHERE type='table'`.

#### Step 1.2 — Domain types

File: `lazyjob-core/src/networking/referral/types.rs`

Implement all types from the Core Types section above. Key points:
- `RelationshipStage` derives `PartialOrd + Ord` so `assert!(Warmed > Replied)` works in tests.
- All `sqlx::Type` derives use `#[sqlx(type_name = "TEXT")]` + `#[sqlx(rename_all = "snake_case")]`.
- `InteractionLog` does not need an `interaction_count` field — count is derived at query time via
  `SELECT COUNT(*) FROM contact_interaction_log WHERE contact_id = ?`.

**Verification**: `cargo test -p lazyjob-core -- referral::types` runs type unit tests verifying
`RelationshipStage::from_db_str(stage.as_db_str()) == Ok(stage)` for all variants.

#### Step 1.3 — Repository traits

File: `lazyjob-core/src/networking/referral/repo.rs`

Define the three `async_trait` traits: `ReferralRepository`, `InteractionRepository`,
`RelationshipRepository`. Export `ContactReminderRow` as a `sqlx::FromRow` struct.

#### Step 1.4 — SQLite repository implementations

File: `lazyjob-core/src/networking/referral/sqlite.rs`

```rust
pub struct SqliteReferralRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteReferralRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl ReferralRepository for SqliteReferralRepository {
    async fn create_referral_ask(&self, ask: &ReferralAsk) -> Result<(), ReferralError> {
        sqlx::query!(
            r#"
            INSERT INTO referral_asks (id, contact_id, job_id, asked_at, notes, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            ask.id.0.to_string(),
            ask.contact_id.0.to_string(),
            ask.job_id.0.to_string(),
            ask.asked_at,
            ask.notes,
            ask.created_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db_err)
                if db_err.message().contains("UNIQUE constraint failed") =>
            {
                ReferralError::AlreadyAsked {
                    contact_id: ask.contact_id.clone(),
                    job_id: ask.job_id.clone(),
                }
            }
            other => ReferralError::Database(other),
        })?;
        Ok(())
    }

    async fn ask_exists(
        &self,
        contact_id: &ContactId,
        job_id: &JobId,
    ) -> Result<bool, ReferralError> {
        let count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM referral_asks WHERE contact_id = ? AND job_id = ?",
            contact_id.0.to_string(),
            job_id.0.to_string(),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(ReferralError::Database)?;
        Ok(count > 0)
    }

    // ... other methods follow the same pattern
}
```

`SqliteRelationshipRepository` is a second struct that wraps the same pool but implements
`RelationshipRepository`. The `increment_reminder_count` method uses a single SQL statement
to handle the rolling-window reset atomically:

```sql
UPDATE profile_contacts
SET
  reminder_count_this_month = CASE
    WHEN reminder_window_start IS NULL
      OR julianday('now') - julianday(reminder_window_start) > 30
    THEN 1
    ELSE reminder_count_this_month + 1
  END,
  reminder_window_start = CASE
    WHEN reminder_window_start IS NULL
      OR julianday('now') - julianday(reminder_window_start) > 30
    THEN date('now')
    ELSE reminder_window_start
  END
WHERE id = ?
RETURNING reminder_count_this_month
```

**Verification**: `#[sqlx::test(migrations = "migrations")]` attribute on integration tests
auto-creates an in-memory SQLite with all migrations applied. Tests assert:
- `create_referral_ask` returns `AlreadyAsked` on duplicate (contact_id, job_id).
- `increment_reminder_count` resets to 1 when window is > 30 days old.
- `list_contacts_for_sweep` returns only contacts with active stages and `follow_up_exhausted = false`.

### Phase 2 — Referral Readiness Checker

#### Step 2.1 — `ReferralReadinessChecker`

File: `lazyjob-core/src/networking/referral/readiness.rs`

```rust
use crate::discovery::{JobRecord, JobRepository};
use crate::job_search::ghost::GhostDetector;
use super::repo::{InteractionRepository, ReferralRepository, RelationshipRepository};
use super::types::*;

pub struct ReferralReadinessChecker {
    referral_repo:    Arc<dyn ReferralRepository>,
    interaction_repo: Arc<dyn InteractionRepository>,
    job_repo:         Arc<dyn JobRepository>,
    ghost_detector:   Arc<GhostDetector>,
}

impl ReferralReadinessChecker {
    pub fn new(
        referral_repo:    Arc<dyn ReferralRepository>,
        interaction_repo: Arc<dyn InteractionRepository>,
        job_repo:         Arc<dyn JobRepository>,
        ghost_detector:   Arc<GhostDetector>,
    ) -> Self {
        Self { referral_repo, interaction_repo, job_repo, ghost_detector }
    }

    /// Evaluate all 5 readiness gates for a (contact, job) pair.
    /// Gates are evaluated in order — the first failing gate returns immediately.
    pub async fn check(
        &self,
        contact_id: &ContactId,
        contact_stage: &RelationshipStage,
        last_interaction_at: Option<NaiveDate>,
        follow_up_exhausted: bool,
        job_id: &JobId,
    ) -> Result<ReferralReadiness, ReferralError> {
        use ReferralReadiness::*;
        use SkipReason::*;
        use NotYetReason::*;

        let today = chrono::Local::now().date_naive();

        // Gate 1: Follow-up exhausted — hard skip.
        if follow_up_exhausted {
            return Ok(Skip { reason: FollowUpExhausted });
        }

        // Gate 2: Referral already asked for this (contact, job) pair.
        if self.referral_repo.ask_exists(contact_id, job_id).await? {
            return Ok(Skip { reason: ReferralAlreadyAsked });
        }

        // Gate 3: Job must be active and not a ghost (score ≥ 0.6 blocks).
        let job = self.job_repo
            .find_by_id(job_id)
            .await?
            .ok_or(ReferralError::JobNotFound(job_id.clone()))?;

        if matches!(job.status, JobStatus::Closed) {
            return Ok(Skip { reason: JobClosed });
        }

        // Re-use cached ghost_score from JobRecord (set during discovery loop).
        // Ghost score is refreshed by LoopType::GhostRescore daily at 2 AM.
        if let Some(score) = job.ghost_score {
            if score >= 0.6 {
                return Ok(Skip { reason: GhostJobDetected });
            }
        }

        // Gate 4: Stage must be Warmed.
        if *contact_stage < RelationshipStage::Warmed {
            return Ok(NotYet {
                reason: RelationshipTooNew,
                suggested_wait_days: match contact_stage {
                    RelationshipStage::Identified => 30,
                    RelationshipStage::Contacted  => 21,
                    RelationshipStage::Replied    => 14,
                    _ => 7,
                },
            });
        }

        // Gate 5: Last interaction timing (7–180 day window).
        let days_since = last_interaction_at
            .map(|d| (today - d).num_days() as u32)
            .unwrap_or(u32::MAX);

        if days_since < 7 {
            return Ok(NotYet {
                reason: TooSoonAfterLastContact,
                suggested_wait_days: 7 - days_since,
            });
        }

        if days_since > 180 {
            return Ok(NotYet {
                reason: RelationshipStale,
                suggested_wait_days: 0,   // Stale — needs fresh interaction first.
            });
        }

        // All gates passed. Ghost score 0.4–0.59 is a warning, not a block.
        // The caller surfaces a yellow warning badge on the TUI suggestion.
        let ghost_warning = job.ghost_score.map_or(false, |s| s >= 0.4);
        let _ = ghost_warning; // Used by caller, not encoded in the return value here.

        Ok(Ready {
            recommended_ask_date: today,
        })
    }
}
```

**Ghost score borderline handling (0.4–0.59)**: `ReferralReadinessChecker::check()` passes
the job but the caller (`NetworkingReminderPoller::run_sweep`) reads `job.ghost_score` and
annotates the emitted `NetworkingReminder` suggestion with a `ghost_warning: bool` flag.
The TUI renders the suggestion with a yellow `[?]` badge next to the job title when this flag
is set. This is a product decision: warn but do not block at 0.4–0.59.

**Verification**: Unit tests use mock implementations of all four injected repositories.
Key test cases:
- `check()` with `ghost_score = 0.65` → `Skip { reason: GhostJobDetected }`.
- `check()` with `ghost_score = 0.45` → `Ready { recommended_ask_date: today }` (passes with warning).
- `check()` with `contact_stage = Contacted` → `NotYet { reason: RelationshipTooNew, suggested_wait_days: 21 }`.
- `check()` with `last_interaction_at = today - 181 days` → `NotYet { reason: RelationshipStale }`.
- `check()` with existing `ReferralAsk` row → `Skip { reason: ReferralAlreadyAsked }`.

### Phase 3 — Relationship Service (Orchestrator)

#### Step 3.1 — `RelationshipService`

File: `lazyjob-core/src/networking/referral/service.rs`

`RelationshipService` is the high-level API consumed by the TUI and the poller. It orchestrates
the interaction between repositories, the readiness checker, and the `WorkflowEvent` broadcast
channel.

```rust
pub struct RelationshipService {
    referral_repo:         Arc<dyn ReferralRepository>,
    interaction_repo:      Arc<dyn InteractionRepository>,
    relationship_repo:     Arc<dyn RelationshipRepository>,
    readiness_checker:     Arc<ReferralReadinessChecker>,
    event_tx:              broadcast::Sender<WorkflowEvent>,
    max_new_outreach_week: u32,
}

impl RelationshipService {
    pub fn new(/* ... */ ) -> Self { /* ... */ }

    /// Log a user interaction with a contact. Advances stage when applicable:
    ///   Contacted → Replied (first interaction)
    ///   Replied   → Warmed  (second+ interaction)
    ///
    /// The stage advance logic uses `RelationshipStage::advance_on_interaction()`.
    #[tracing::instrument(skip(self))]
    pub async fn log_interaction(
        &self,
        contact_id:     ContactId,
        interacted_at:  NaiveDate,
        note:           Option<String>,
        current_stage:  RelationshipStage,
    ) -> Result<Option<RelationshipStage>, ReferralError> {
        let log = InteractionLog {
            id:            InteractionLogId::new(),
            contact_id:    contact_id.clone(),
            interacted_at,
            note,
            created_at:    Utc::now(),
        };
        self.interaction_repo.log_interaction(&log).await?;

        if let Some(next_stage) = current_stage.advance_on_interaction() {
            self.relationship_repo.update_stage(&contact_id, next_stage.clone()).await?;
            Ok(Some(next_stage))
        } else {
            Ok(None)
        }
    }

    /// Create a referral ask record. Returns `AlreadyAsked` if one exists.
    #[tracing::instrument(skip(self))]
    pub async fn record_referral_ask(
        &self,
        contact_id: ContactId,
        job_id:     JobId,
        notes:      Option<String>,
    ) -> Result<ReferralAsk, ReferralError> {
        let ask = ReferralAsk {
            id:                  ReferralAskId::new(),
            contact_id:          contact_id.clone(),
            job_id:              job_id.clone(),
            asked_at:            chrono::Local::now().date_naive(),
            outcome:             None,
            outcome_recorded_at: None,
            notes,
            created_at:          Utc::now(),
        };
        self.referral_repo.create_referral_ask(&ask).await?;

        // Advance stage to ReferralAsked.
        self.relationship_repo
            .update_stage(&contact_id, RelationshipStage::ReferralAsked)
            .await?;

        Ok(ask)
    }

    /// Record the outcome of a referral ask. Advances stage to ReferralResolved.
    #[tracing::instrument(skip(self))]
    pub async fn record_outcome(
        &self,
        ask_id:     ReferralAskId,
        contact_id: ContactId,
        outcome:    ReferralOutcome,
    ) -> Result<(), ReferralError> {
        let today = chrono::Local::now().date_naive();
        self.referral_repo
            .record_outcome(&ask_id, outcome, today)
            .await?;
        self.relationship_repo
            .update_stage(&contact_id, RelationshipStage::ReferralResolved)
            .await?;
        self.event_tx
            .send(WorkflowEvent::ReferralOutcomeRecorded { ask_id, contact_id })
            .ok(); // ignore send error if no subscribers
        Ok(())
    }

    /// Returns the weekly new-outreach count (contacts transitioned from Identified → Contacted
    /// in the last 7 days) and whether the weekly cap has been reached.
    pub async fn can_send_new_outreach(&self) -> Result<bool, ReferralError> {
        let count = self.relationship_repo.new_outreach_count_this_week().await?;
        Ok(count < self.max_new_outreach_week)
    }
}
```

**Verification**: Unit tests inject mock repos and assert:
- `log_interaction` on a `Contacted` contact writes the log row and calls `update_stage(Replied)`.
- `log_interaction` on a `Warmed` contact writes the log row but does NOT call `update_stage`.
- `record_referral_ask` on a contact that already has a row returns `AlreadyAsked`.
- `record_outcome` on an `ask_id` transitions stage to `ReferralResolved` and broadcasts event.

### Phase 4 — Networking Reminder Poller

#### Step 4.1 — `NetworkingReminderPoller`

File: `lazyjob-ralph/src/networking_poller.rs`

```rust
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info, warn};

use lazyjob_core::networking::referral::{
    ContactReminderRow, InteractionRepository, ReferralReadinessChecker,
    RelationshipRepository, ReferralRepository,
};
use lazyjob_core::networking::referral::types::*;
use lazyjob_core::discovery::JobRepository;
use lazyjob_core::workflow::WorkflowEvent;

pub struct NetworkingReminderPoller {
    relationship_repo:   Arc<dyn RelationshipRepository>,
    interaction_repo:    Arc<dyn InteractionRepository>,
    referral_repo:       Arc<dyn ReferralRepository>,
    job_repo:            Arc<dyn JobRepository>,
    readiness_checker:   Arc<ReferralReadinessChecker>,
    event_tx:            broadcast::Sender<WorkflowEvent>,
    reminder_interval:   Duration,
    max_reminders_per_contact_per_month: u8,
}

impl NetworkingReminderPoller {
    /// Spawn the poller as an independent tokio task. Returns a `JoinHandle`
    /// the caller can use to join or abort.
    pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(self.reminder_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                match self.run_sweep().await {
                    Ok(count) => info!(reminders_emitted = count, "networking sweep complete"),
                    Err(e)    => warn!(error = %e, "networking sweep error"),
                }
            }
        })
    }

    /// Single sweep iteration. Returns the count of `WorkflowEvent` messages emitted.
    pub async fn run_sweep(&self) -> Result<usize, ReferralError> {
        let today = chrono::Local::now().date_naive();
        let rows = self.relationship_repo.list_contacts_for_sweep().await?;
        let mut emitted = 0usize;

        for row in rows {
            // Anti-spam gate: skip if monthly cap reached.
            if row.reminder_count_this_month >= self.max_reminders_per_contact_per_month {
                continue;
            }

            let contact_id = ContactId(uuid::Uuid::parse_str(&row.contact_id)
                .map_err(|_| ReferralError::InvalidUuid(row.contact_id.clone()))?);
            let stage = RelationshipStage::from_db_str(&row.relationship_stage)?;

            let days_since = row.last_contacted_at
                .map(|d| (today - d).num_days() as u32)
                .unwrap_or(u32::MAX);

            let action = self.determine_action(&row, &contact_id, &stage, days_since, today).await?;

            if let Some(action) = action {
                let reminder = NetworkingReminder {
                    contact_id:                  contact_id.clone(),
                    contact_name:                row.contact_name.clone(),
                    company_name:                row.company_name.clone(),
                    current_stage:               stage,
                    days_since_last_interaction: days_since,
                    suggested_action:            action,
                };

                // Increment monthly counter before emitting.
                self.relationship_repo
                    .increment_reminder_count(&contact_id, today)
                    .await?;

                self.event_tx
                    .send(WorkflowEvent::NetworkingReminderDue(reminder))
                    .ok();
                emitted += 1;
            }
        }

        Ok(emitted)
    }

    /// Determine the appropriate action for a single contact row.
    /// Returns None if no action is warranted.
    async fn determine_action(
        &self,
        row:        &ContactReminderRow,
        contact_id: &ContactId,
        stage:      &RelationshipStage,
        days_since: u32,
        today:      NaiveDate,
    ) -> Result<Option<NetworkingReminderAction>, ReferralError> {
        match stage {
            RelationshipStage::Contacted => {
                // Follow-up on no response: 7 days.
                // Second and final follow-up: 14 days.
                if days_since >= 7 {
                    return Ok(Some(NetworkingReminderAction::SendFollowUp));
                }
            }
            RelationshipStage::Replied => {
                // Relationship cooling — encourage a substantive interaction.
                if days_since >= 21 {
                    return Ok(Some(NetworkingReminderAction::LogInteraction));
                }
            }
            RelationshipStage::Warmed => {
                // Find active jobs at the contact's company, check referral readiness.
                let jobs = self.job_repo
                    .list_active_by_company_name(&row.company_name)
                    .await?;

                for job in jobs {
                    let readiness = self.readiness_checker.check(
                        contact_id,
                        stage,
                        row.last_contacted_at,
                        row.follow_up_exhausted,
                        &job.id,
                    ).await?;

                    if let ReferralReadiness::Ready { .. } = readiness {
                        return Ok(Some(NetworkingReminderAction::AskForReferral {
                            job_id:    job.id,
                            job_title: job.title,
                        }));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }
}
```

**`MissedTickBehavior::Skip`** is required (same as `ReminderPoller` in the application workflow
plan) to prevent a burst of ticks if the system clock jumps or the process was suspended.

**Verification**: Integration tests use a `MockRelationshipRepository` and a `MockJobRepository`
(in-memory `Vec`-backed). Key test cases:
- Contact with `Contacted` stage, `days_since = 8` → `SendFollowUp` emitted.
- Contact with `Contacted` stage, `days_since = 3` → no action.
- Contact with `reminder_count_this_month = 2` → no action (anti-spam gate).
- Contact with `Warmed` stage, one active job at company, readiness = Ready → `AskForReferral` emitted.
- `run_sweep` returns count of events emitted.

#### Step 4.2 — Wire into application stage transition dispatch

File: `lazyjob-ralph/src/dispatch.rs`

Add `PostTransitionSuggestion::UpdateReferralOutcome` variant handling to the exhaustive `match`
in `LoopDispatch::dispatch_suggestion()`:

```rust
PostTransitionSuggestion::UpdateReferralOutcome { application_id } => {
    // Find open referral asks for the job linked to this application.
    let asks = referral_repo
        .list_open_asks_for_job(&job_id_from_application(application_id))
        .await?;
    if !asks.is_empty() {
        event_tx.send(WorkflowEvent::ReferralOutcomePromptRequired {
            application_id,
            ask_ids: asks.iter().map(|a| a.id.clone()).collect(),
        }).ok();
    }
}
```

The `ApplicationStateMachine::transition()` emits `PostTransitionSuggestion::UpdateReferralOutcome`
when the new stage is `Offered` or `Rejected`. This wiring is in `lazyjob-ralph/src/dispatch.rs`
alongside the existing `ResumeAutoTailor` and other post-transition suggestions.

### Phase 5 — TUI Networking Dashboard

#### Step 5.1 — Networking dashboard view

File: `lazyjob-tui/src/views/networking/dashboard.rs`

The dashboard is a two-panel layout: left panel is a company-grouped contact list; right panel is
the contact detail with interaction log.

**Layout** (ratatui `Layout::default().direction(Horizontal).constraints([38%, 62%])`):

```
┌─ Networking (42 contacts) ──────────────────────────────────────────────────┐
│ Google (3)                │ Alice Chen — Google                              │
│  ● Alice Chen   [Warmed]  │ Stage: Warmed  ·  Last contact: 12 days ago     │
│  ○ Bob Kim      [Replied] │                                                  │
│  ○ Carol Wu     [Contacted]│ Interaction Log                                 │
│ Meta (1)                  │  2026-04-04  Coffee chat at SFO                 │
│  ○ David Li     [Identified]│ 2026-03-20  LinkedIn DM - replied to post    │
│ Stripe (2)                │                                                  │
│  ● Emma Park    [Warmed]  │ Referral Asks                                   │
│  ○ Frank Cho    [Contacted]│  ○ SWE II - Applied (pending)                  │
│                           │                                                  │
│ [r]eferral  [l]og  [↑↓] nav │ [l]og interaction  [r]eferral  [o]utreach    │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Stage badge color coding**:

| Stage | Color | Symbol |
|-------|-------|--------|
| Identified | Gray | `○` |
| Contacted | Yellow | `○` |
| Replied | Cyan | `○` |
| Warmed | Green | `●` |
| ReferralAsked | Magenta | `●` |
| ReferralResolved | Dark Gray | `✓` |

**Keybindings** (in `KeyContext::Networking`):

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate contacts in left panel |
| `l` | Open "Log Interaction" form dialog |
| `r` | Open "Record Referral Ask" confirmation dialog |
| `o` | Open outreach draft panel for selected contact |
| `Enter` | Focus right panel (detail) |
| `Escape` | Return focus to left panel |
| `?` | Show ghost score explanation popup for focused job |

#### Step 5.2 — "Log Interaction" dialog

File: `lazyjob-tui/src/views/networking/contact_detail.rs`

A modal dialog (rendered over `ratatui::widgets::Clear`) with:
- A date field pre-populated with today (editable, YYYY-MM-DD format).
- An optional note text area (single line, max 200 chars).
- `Enter` to submit → calls `RelationshipService::log_interaction()`.
- On success: interaction log in right panel updates; stage badge in left panel advances if applicable.
- The dialog shows the new stage in a confirmation line: "Stage advanced to [Warmed]" in green.

#### Step 5.3 — Morning digest integration

The TUI subscribes to `WorkflowEvent::NetworkingReminderDue(reminder)` via the shared broadcast
channel (already established in the application workflow actions plan). Networking reminders are
rendered as a section in the morning digest panel below application reminders:

```
╔ Networking Reminders ════════════════════════════════════════╗
║  ● Alice Chen (Google) — Ask for referral: SWE II           ║
║  ○ Bob Kim (Meta) — Send follow-up (8 days no response)     ║
╚═════════════════════════════════════════════════════════════ ╝
```

Ghost-warning referral suggestions have a `[?]` badge rendered in `Color::Yellow` next to the
job title.

## Key Crate APIs

### sqlx
- `sqlx::query!()` macro for compile-time checked queries — used in all repository methods.
- `sqlx::SqlitePool` — connection pool injected into repository structs.
- `#[sqlx::test(migrations = "migrations")]` — integration test attribute that applies all
  migrations to a fresh in-memory SQLite.
- `sqlx::FromRow` derive — used on `ContactReminderRow` for the sweep query.
- `sqlx::Error::Database` match arm with `message().contains("UNIQUE constraint failed")` —
  maps the SQLite UNIQUE violation to `ReferralError::AlreadyAsked`.

### tokio
- `tokio::time::interval(Duration::from_secs(86400))` — 24-hour poller tick.
- `tokio::time::MissedTickBehavior::Skip` — prevents burst ticks after sleep/wake.
- `tokio::sync::broadcast::Sender<WorkflowEvent>` — event channel for TUI subscribers.
- `tokio::task::JoinHandle` — returned by `NetworkingReminderPoller::spawn()`.

### chrono
- `chrono::Local::now().date_naive()` — local date for interaction timestamps and day-delta math.
- `(today - last_interaction).num_days() as u32` — days-since calculation in poller sweep.
- `NaiveDate` — all date fields in SQLite (stored as `DATE` / `TEXT`).

### uuid
- `Uuid::new_v4()` — ID generation for all new entities.
- `Uuid::parse_str(&str)` — deserialization from SQLite TEXT column.

### ratatui
- `Layout::default().direction(Horizontal).constraints([Percentage(38), Percentage(62)])` — dashboard split.
- `List::new(items).block(Block::default().title("Contacts"))` — contact list panel.
- `Clear` widget rendered before the "Log Interaction" modal to erase background.
- `Span::styled("[Warmed]", Style::default().fg(Color::Green))` — stage badge rendering.

### tracing
- `#[tracing::instrument(skip(self))]` on all async service methods.
- `tracing::info!(reminders_emitted = count, "networking sweep complete")` — structured log.
- `tracing::warn!(error = %e, "networking sweep error")` — sweep error without panic.

## Error Handling

```rust
// lazyjob-core/src/networking/referral/error.rs

#[derive(thiserror::Error, Debug)]
pub enum ReferralError {
    #[error("referral ask already exists for contact {contact_id:?} and job {job_id:?}")]
    AlreadyAsked { contact_id: ContactId, job_id: JobId },

    #[error("job not found: {0:?}")]
    JobNotFound(JobId),

    #[error("contact not found: {0:?}")]
    ContactNotFound(ContactId),

    #[error("referral ask not found: {0:?}")]
    AskNotFound(ReferralAskId),

    #[error("invalid relationship stage string: {0}")]
    InvalidStage(String),

    #[error("invalid UUID: {0}")]
    InvalidUuid(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ReferralError>;
```

**Error escalation rules:**
- `ReferralError::AlreadyAsked` — TUI shows a dismissable inline info message ("You've already
  asked this contact for a referral for this role"). Not an error dialog.
- `ReferralError::JobNotFound` / `ContactNotFound` — TUI shows an error toast. Logged at
  `tracing::error!` level.
- `ReferralError::Database` — TUI shows "Something went wrong — check logs". Logged at
  `tracing::error!` with full `sqlx::Error` chain.
- `ReferralError::InvalidStage` — panic in debug, log+skip in release during migration (should
  never occur once migration is applied).

## Testing Strategy

### Unit Tests

File: `lazyjob-core/src/networking/referral/readiness.rs` (module-level `#[cfg(test)]` block)

```rust
struct MockReferralRepo { existing_asks: Vec<(ContactId, JobId)> }
struct MockInteractionRepo { last_date: Option<NaiveDate> }
struct MockJobRepo { jobs: Vec<JobRecord> }
struct MockGhostDetector;

#[tokio::test]
async fn test_readiness_blocks_ghost_job() {
    let job = job_with_ghost_score(0.65);
    let checker = checker_with_job(job);
    let result = checker.check(
        &contact_id(), &RelationshipStage::Warmed,
        Some(chrono::Local::now().date_naive() - chrono::Duration::days(30)),
        false, &job_id(),
    ).await.unwrap();
    assert_eq!(result, ReferralReadiness::Skip { reason: SkipReason::GhostJobDetected });
}

#[tokio::test]
async fn test_readiness_warns_borderline_ghost() {
    let job = job_with_ghost_score(0.45);
    let checker = checker_with_job(job);
    let result = checker.check(
        &contact_id(), &RelationshipStage::Warmed,
        Some(chrono::Local::now().date_naive() - chrono::Duration::days(30)),
        false, &job_id(),
    ).await.unwrap();
    // Passes but caller should surface ghost_warning = true in TUI.
    assert!(matches!(result, ReferralReadiness::Ready { .. }));
}

#[tokio::test]
async fn test_stage_too_new_suggested_wait() {
    // ...
}
```

### Integration Tests (SQLite)

File: `lazyjob-core/tests/referral_integration.rs`

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_create_referral_ask_duplicate_returns_error(pool: SqlitePool) {
    let repo = SqliteReferralRepository::new(pool);
    let ask = make_referral_ask();
    repo.create_referral_ask(&ask).await.unwrap();
    let result = repo.create_referral_ask(&ask).await;
    assert!(matches!(result, Err(ReferralError::AlreadyAsked { .. })));
}

#[sqlx::test(migrations = "migrations")]
async fn test_increment_reminder_count_rolling_window_resets(pool: SqlitePool) {
    // Insert a contact with reminder_window_start = 31 days ago.
    // increment_reminder_count should reset count to 1.
    // ...
}

#[sqlx::test(migrations = "migrations")]
async fn test_log_interaction_advances_contacted_to_replied(pool: SqlitePool) {
    // Insert contact with stage = 'contacted'.
    // Call RelationshipService::log_interaction.
    // Assert profile_contacts.relationship_stage = 'replied'.
    // ...
}
```

### Poller Tests

File: `lazyjob-ralph/src/networking_poller.rs` (`#[cfg(test)]` module)

- `run_sweep` with 3 contacts (2 actionable, 1 at cap) → asserts `emitted == 2`.
- `run_sweep` with a contact at cap → asserts `emitted == 0`.
- `determine_action` for `Warmed` contact with `ghost_score = 0.65` job → returns `None`.

### TUI Tests

`ContactDetailPanel` renders via `ratatui::backend::TestBackend` with a fixed 80×24 terminal.
Assert that:
- `[Warmed]` badge renders in `Color::Green`.
- Log interaction dialog renders `Clear` widget (first widget drawn in the frame).
- After `log_interaction`, the stage badge updates without a full view rebuild.

## Open Questions

1. **Relationship strength score (Phase 2)**: The spec defers a numeric SSI-like score to Phase 2.
   When added, it should live in `ContactReminderRow` and be used to weight referral suggestion
   ordering in the TUI (higher score = shown first). The existing `is_active_for_reminders()`
   partial index and sweep query should remain unchanged — the score is purely for ordering.

2. **Ghost score borderline threshold configuration**: The 0.4 (warn) and 0.6 (block) thresholds
   for ghost score in `ReferralReadinessChecker` are hardcoded constants in Phase 1. Phase 3 should
   move them to `lazyjob.toml` as `networking.ghost_warn_threshold` and `networking.ghost_block_threshold`.

3. **Contact deduplication on re-import** (spec Open Question 4): If the same person is added
   manually and later imported from LinkedIn CSV, `email` is the dedup key. The silent duplicate
   (no email overlap) is deferred to Phase 2. Phase 2 should add a `possible_duplicate_of TEXT`
   column on `profile_contacts` populated by the `LinkedInCsvImporter` post-import sweep.

4. **Interaction logging granularity** (spec Open Question 1): Phase 1 uses a flat interaction log
   (date + note). Phase 2 can add an `interaction_type` enum (`Message`, `PhoneCall`, `VideoCall`,
   `InPersonMeeting`, `SocialMedia`) to enable richer analytics and stage-advance triggers
   (e.g., only advance to `Warmed` on `PhoneCall` or `InPersonMeeting`).

5. **`list_active_by_company_name` index**: The poller calls `JobRepository::list_active_by_company_name`
   in a loop over all warmed contacts. If the user has many warmed contacts, this can produce
   many DB queries. Phase 2 should batch this into a single `WHERE company_name IN (...)` query
   with the list of warmed contacts' company names.

6. **PostTransitionSuggestion variant exhaustiveness**: `LoopDispatch::dispatch_suggestion` uses
   an exhaustive `match` with no wildcard arm. Adding `UpdateReferralOutcome` variant in Phase 4
   will cause a compile error in all existing callsites that need updating — this is intentional.
   All arms must be updated before the code compiles.

## Related Specs

- [specs/networking-connection-mapping.md](networking-connection-mapping.md) — `ProfileContact`, `ContactRepository`, `ConnectionTier`
- [specs/networking-outreach-drafting.md](networking-outreach-drafting.md) — outreach draft pipeline
- [specs/application-state-machine.md](application-state-machine.md) — `ApplicationStage` transitions
- [specs/application-workflow-actions.md](application-workflow-actions.md) — `WorkflowEvent`, `PostTransitionSuggestion`, broadcast channel
- [specs/job-search-ghost-job-detection.md](job-search-ghost-job-detection.md) — `GhostDetector`, `ghost_score`
- [specs/job-search-discovery-engine.md](job-search-discovery-engine.md) — `JobRecord`, `JobRepository`, `JobStatus`
- [specs/networking-referrals-agentic.md](networking-referrals-agentic.md) — autonomous contact discovery (Phase 3+)
