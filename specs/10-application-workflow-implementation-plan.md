# Implementation Plan: Application Workflow

## Status
Draft

## Related Spec
[`specs/10-application-workflow.md`](./10-application-workflow.md)

## Overview

The application workflow is the operational core of LazyJob — the system that tracks every job application from first discovery through terminal resolution (Accepted, Rejected, or Withdrawn). It implements a validated 10-stage state machine, an immutable append-only transition history log, and a set of workflow actions (Apply, MoveStage, ScheduleInterview, LogContact) that execute as side-effectful async operations. All state changes are persisted atomically to SQLite; no state lives only in memory.

The spec also defines four workflow orchestrators (`ApplyWorkflow`, `MoveStageWorkflow`, `ScheduleInterviewWorkflow`, `LogContactWorkflow`) that compose repository operations, LLM-triggered side effects (auto-tailor resume, generate cover letter), and reminder creation. Each workflow is pure business logic — it takes a db pool and domain inputs, executes against SQLite, and emits a `WorkflowEvent` on a tokio broadcast channel that the TUI listens to for re-rendering.

The TUI exposes a Kanban pipeline view (one column per active stage) plus an Action Required queue that surfaces overdue follow-ups, expiring offers, and upcoming interviews. Pipeline metrics (response rate, funnel ratios, avg time in stage) are computed by live SQL aggregation queries, not materialized views.

## Prerequisites

### Specs / Plans That Must Be Implemented First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations infrastructure
- `specs/01-architecture-implementation-plan.md` (or `01-gaps-core-architecture-implementation-plan.md`) — crate layout, `lazyjob-core`
- `specs/03-life-sheet-data-model-implementation-plan.md` — `ResumeVersion`, `LifeSheet` types
- `specs/09-tui-design-keybindings-implementation-plan.md` — `EventLoop`, `KeyContext`, widget rendering primitives

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
sqlx            = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
tokio           = { version = "1", features = ["full"] }
uuid            = { version = "1", features = ["v4", "serde"] }
serde           = { version = "1", features = ["derive"] }
serde_json      = "1"
chrono          = { version = "0.4", features = ["serde"] }
thiserror       = "2"
anyhow          = "1"
tracing         = "0.1"

# lazyjob-tui/Cargo.toml
[dependencies]
ratatui         = "0.29"
crossterm       = "0.28"
tokio           = { version = "1", features = ["full"] }
```

---

## Architecture

### Crate Placement

| Component | Crate | Reason |
|---|---|---|
| `ApplicationStage` enum | `lazyjob-core` | Domain type, no I/O dependencies |
| `Application`, `StageTransition`, `Interview`, `Offer` structs | `lazyjob-core` | Shared by TUI, CLI, and Ralph |
| `ApplicationRepository` trait + impl | `lazyjob-core` | Persistence boundary |
| `ApplyWorkflow`, `MoveStageWorkflow`, etc. | `lazyjob-core` | Business logic, async; testable without TUI |
| `PipelineMetrics`, metric queries | `lazyjob-core` | SQL aggregations |
| `ReminderPoller` background task | `lazyjob-core` | Tokio task; no TUI imports |
| Kanban board widget, Action Queue widget | `lazyjob-tui` | Pure rendering, receives `Arc<Database>` |
| Confirmation dialog overlay | `lazyjob-tui` | Ratatui overlay |

### Core Types

```rust
// lazyjob-core/src/application/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The 10-stage application lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum ApplicationStage {
    Discovered,
    Interested,
    Applied,
    PhoneScreen,
    Technical,
    OnSite,
    Offer,
    Accepted,
    Rejected,
    Withdrawn,
}

impl ApplicationStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovered  => "Discovered",
            Self::Interested  => "Interested",
            Self::Applied     => "Applied",
            Self::PhoneScreen => "Phone Screen",
            Self::Technical   => "Technical",
            Self::OnSite      => "On-site",
            Self::Offer       => "Offer",
            Self::Accepted    => "Accepted",
            Self::Rejected    => "Rejected",
            Self::Withdrawn   => "Withdrawn",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected | Self::Withdrawn)
    }

    pub fn active_stages() -> &'static [ApplicationStage] {
        use ApplicationStage::*;
        &[Discovered, Interested, Applied, PhoneScreen, Technical, OnSite, Offer]
    }

    /// Returns valid next stages from `self`.
    pub fn valid_transitions(self) -> &'static [ApplicationStage] {
        use ApplicationStage::*;
        match self {
            Discovered  => &[Interested, Applied, Rejected, Withdrawn],
            Interested  => &[Discovered, Applied, Rejected, Withdrawn],
            Applied     => &[Interested, PhoneScreen, Technical, OnSite, Offer, Rejected, Withdrawn],
            PhoneScreen => &[Applied, Technical, Offer, Rejected, Withdrawn],
            Technical   => &[PhoneScreen, OnSite, Offer, Rejected, Withdrawn],
            OnSite      => &[Technical, Offer, Rejected, Withdrawn],
            Offer       => &[OnSite, Accepted, Rejected, Withdrawn],
            Accepted    => &[],
            Rejected    => &[],
            Withdrawn   => &[],
        }
    }

    pub fn can_transition_to(self, next: ApplicationStage) -> bool {
        self.valid_transitions().contains(&next)
    }
}

impl std::str::FromStr for ApplicationStage {
    type Err = ApplicationError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Discovered"  => Ok(Self::Discovered),
            "Interested"  => Ok(Self::Interested),
            "Applied"     => Ok(Self::Applied),
            "PhoneScreen" => Ok(Self::PhoneScreen),
            "Technical"   => Ok(Self::Technical),
            "OnSite"      => Ok(Self::OnSite),
            "Offer"       => Ok(Self::Offer),
            "Accepted"    => Ok(Self::Accepted),
            "Rejected"    => Ok(Self::Rejected),
            "Withdrawn"   => Ok(Self::Withdrawn),
            other => Err(ApplicationError::UnknownStage(other.to_string())),
        }
    }
}

/// A single job application record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id:                      Uuid,
    pub job_id:                  Uuid,
    pub stage:                   ApplicationStage,
    pub resume_version_id:       Option<Uuid>,
    pub cover_letter_version_id: Option<Uuid>,
    pub notes:                   String,
    pub priority:                ApplicationPriority,
    pub last_contact_at:         Option<DateTime<Utc>>,
    pub next_follow_up_at:       Option<DateTime<Utc>>,
    pub archived_at:             Option<DateTime<Utc>>,
    pub created_at:              DateTime<Utc>,
    pub updated_at:              DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "INTEGER")]
#[repr(i32)]
pub enum ApplicationPriority {
    Low    = 1,
    Medium = 2,
    High   = 3,
    Urgent = 4,
}

impl Default for ApplicationPriority {
    fn default() -> Self { Self::Medium }
}

/// A single stage transition — immutable once inserted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageTransition {
    pub id:               Uuid,
    pub application_id:   Uuid,
    pub from_stage:       ApplicationStage,
    pub to_stage:         ApplicationStage,
    pub reason:           Option<String>,
    pub transitioned_at:  DateTime<Utc>,
}

/// A contact associated with an application (recruiter, HM, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContact {
    pub id:             Uuid,
    pub application_id: Uuid,
    pub name:           String,
    pub role:           Option<String>,   // "Recruiter", "Hiring Manager", "Engineer"
    pub email:          Option<String>,
    pub linkedin_url:   Option<String>,
    pub stage:          ApplicationStage, // Which stage they appeared in
    pub contacted_at:   DateTime<Utc>,
    pub notes:          Option<String>,
}

/// A scheduled or completed interview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interview {
    pub id:              Uuid,
    pub application_id:  Uuid,
    pub interview_type:  InterviewType,
    pub scheduled_at:    Option<DateTime<Utc>>,
    pub duration_mins:   Option<i32>,
    pub location:        Option<String>,
    pub meeting_url:     Option<String>,
    pub interviewer_names: Vec<String>,  // JSON array in SQLite
    pub status:          InterviewStatus,
    pub self_rating:     Option<i32>,    // 1–5 after interview
    pub feedback_notes:  Option<String>, // Recruiter or own feedback
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum InterviewType {
    PhoneScreen,
    Technical,
    SystemDesign,
    Behavioral,
    OnSite,
    AsyncChallenge,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum InterviewStatus {
    Scheduled,
    Completed,
    Cancelled,
    Rescheduled,
}

/// A compensation offer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id:                  Uuid,
    pub application_id:      Uuid,
    pub base_salary_cents:   i64,
    pub currency:            String,
    pub bonus_cents:         Option<i64>,
    pub equity_summary:      Option<String>, // free-form, e.g. "0.05% over 4yr"
    pub signing_bonus_cents: Option<i64>,
    pub benefits_notes:      Option<String>,
    pub start_date:          Option<chrono::NaiveDate>,
    pub expires_at:          Option<DateTime<Utc>>,
    pub status:              OfferStatus,
    pub raw_letter_text:     Option<String>,
    pub created_at:          DateTime<Utc>,
    pub updated_at:          DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum OfferStatus {
    Pending,
    Negotiating,
    Accepted,
    Declined,
    Expired,
}

/// A reminder or follow-up task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id:             Uuid,
    pub title:          String,
    pub body:           Option<String>,
    pub due_at:         DateTime<Utc>,
    pub completed:      bool,
    pub application_id: Option<Uuid>,
    pub job_id:         Option<Uuid>,
    pub created_at:     DateTime<Utc>,
}

/// Pipeline-level aggregated metrics.
#[derive(Debug, Clone, Default)]
pub struct PipelineMetrics {
    pub total_active:       usize,
    pub by_stage:           std::collections::HashMap<ApplicationStage, usize>,
    pub response_rate:      f32, // % with any recruiter response
    pub interview_rate:     f32, // % reaching PhoneScreen+
    pub offer_rate:         f32, // % receiving offer
    pub avg_days_in_stage:  std::collections::HashMap<ApplicationStage, f32>,
    pub stale_count:        usize, // no contact in 14d (configurable)
    pub expiring_offers:    usize, // offer expires within 5 days
}
```

### Trait Definitions

```rust
// lazyjob-core/src/application/repository.rs

#[async_trait::async_trait]
pub trait ApplicationRepository: Send + Sync {
    async fn insert(&self, app: &Application) -> Result<()>;
    async fn get(&self, id: Uuid) -> Result<Option<Application>>;
    async fn list(&self, filter: &ApplicationFilter) -> Result<Vec<Application>>;
    async fn update(&self, app: &Application) -> Result<()>;
    async fn archive(&self, id: Uuid) -> Result<()>;
    async fn delete(&self, id: Uuid) -> Result<()>;

    async fn update_stage(&self, id: Uuid, new_stage: ApplicationStage, reason: Option<&str>) -> Result<StageTransition>;
    async fn transition_history(&self, id: Uuid) -> Result<Vec<StageTransition>>;

    async fn insert_contact(&self, c: &ApplicationContact) -> Result<()>;
    async fn contacts(&self, application_id: Uuid) -> Result<Vec<ApplicationContact>>;

    async fn insert_interview(&self, i: &Interview) -> Result<()>;
    async fn update_interview(&self, i: &Interview) -> Result<()>;
    async fn interviews(&self, application_id: Uuid) -> Result<Vec<Interview>>;

    async fn insert_offer(&self, o: &Offer) -> Result<()>;
    async fn update_offer(&self, o: &Offer) -> Result<()>;
    async fn offers(&self, application_id: Uuid) -> Result<Vec<Offer>>;

    async fn insert_reminder(&self, r: &Reminder) -> Result<()>;
    async fn pending_reminders(&self) -> Result<Vec<Reminder>>;
    async fn complete_reminder(&self, id: Uuid) -> Result<()>;

    async fn metrics(&self) -> Result<PipelineMetrics>;
    async fn action_required(&self) -> Result<Vec<ActionRequired>>;
}

/// Filter for listing applications.
#[derive(Debug, Default)]
pub struct ApplicationFilter {
    pub stages:       Option<Vec<ApplicationStage>>,
    pub job_id:       Option<Uuid>,
    pub archived:     bool,
    pub stale_only:   bool,
    pub limit:        Option<i64>,
    pub offset:       Option<i64>,
}

/// An item surfaced in the "Action Required" queue.
#[derive(Debug, Clone)]
pub enum ActionRequired {
    OverdueFollowUp {
        application_id: Uuid,
        company:        String,
        title:          String,
        days_overdue:   i64,
    },
    UpcomingInterview {
        interview_id:   Uuid,
        application_id: Uuid,
        company:        String,
        interview_type: InterviewType,
        scheduled_at:   DateTime<Utc>,
    },
    ExpiringOffer {
        offer_id:       Uuid,
        application_id: Uuid,
        company:        String,
        expires_in_days: i64,
    },
    StaleApplication {
        application_id: Uuid,
        company:        String,
        title:          String,
        days_since_contact: i64,
    },
}
```

### WorkflowEvent Bus

```rust
// lazyjob-core/src/application/events.rs

use tokio::sync::broadcast;

/// Events emitted by workflow actions; consumed by TUI for re-render.
#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    ApplicationCreated(Uuid),
    StageChanged { application_id: Uuid, from: ApplicationStage, to: ApplicationStage },
    InterviewScheduled { application_id: Uuid, interview_id: Uuid },
    OfferReceived { application_id: Uuid, offer_id: Uuid },
    ReminderDue { reminder_id: Uuid, application_id: Option<Uuid> },
    MetricsInvalidated,
}

pub type EventTx = broadcast::Sender<WorkflowEvent>;
pub type EventRx = broadcast::Receiver<WorkflowEvent>;

pub fn event_bus() -> (EventTx, EventRx) {
    broadcast::channel(256)
}
```

### SQLite Schema

Migration `migrations/002_applications.sql` (creates application-related tables; jobs table from migration 001):

```sql
-- Applications core table
CREATE TABLE IF NOT EXISTS applications (
    id                      TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id                  TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    stage                   TEXT NOT NULL DEFAULT 'Discovered',
    resume_version_id       TEXT REFERENCES resume_versions(id) ON DELETE SET NULL,
    cover_letter_version_id TEXT REFERENCES cover_letter_versions(id) ON DELETE SET NULL,
    notes                   TEXT NOT NULL DEFAULT '',
    priority                INTEGER NOT NULL DEFAULT 2,
    last_contact_at         TEXT,
    next_follow_up_at       TEXT,
    archived_at             TEXT,
    created_at              TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_applications_job_id   ON applications(job_id);
CREATE INDEX IF NOT EXISTS idx_applications_stage    ON applications(stage);
CREATE INDEX IF NOT EXISTS idx_applications_archived ON applications(archived_at) WHERE archived_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_applications_followup ON applications(next_follow_up_at) WHERE next_follow_up_at IS NOT NULL;

-- Immutable transition history (append-only)
CREATE TABLE IF NOT EXISTS application_transitions (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    from_stage      TEXT NOT NULL,
    to_stage        TEXT NOT NULL,
    reason          TEXT,
    transitioned_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_transitions_application ON application_transitions(application_id, transitioned_at);

-- Contacts encountered during hiring process (not the networking contact graph)
CREATE TABLE IF NOT EXISTS application_contacts (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    role            TEXT,
    email           TEXT,
    linkedin_url    TEXT,
    stage           TEXT NOT NULL,
    contacted_at    TEXT NOT NULL DEFAULT (datetime('now')),
    notes           TEXT
);

CREATE INDEX IF NOT EXISTS idx_app_contacts_application ON application_contacts(application_id);

-- Interviews
CREATE TABLE IF NOT EXISTS interviews (
    id                 TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id     TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type     TEXT NOT NULL,
    scheduled_at       TEXT,
    duration_mins      INTEGER,
    location           TEXT,
    meeting_url        TEXT,
    interviewer_names  TEXT NOT NULL DEFAULT '[]',  -- JSON array
    status             TEXT NOT NULL DEFAULT 'Scheduled',
    self_rating        INTEGER,
    feedback_notes     TEXT,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at         TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_interviews_application ON interviews(application_id);
CREATE INDEX IF NOT EXISTS idx_interviews_scheduled   ON interviews(scheduled_at) WHERE scheduled_at IS NOT NULL;

-- Compensation offers
CREATE TABLE IF NOT EXISTS offers (
    id                   TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id       TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    base_salary_cents    INTEGER NOT NULL,
    currency             TEXT NOT NULL DEFAULT 'USD',
    bonus_cents          INTEGER,
    equity_summary       TEXT,
    signing_bonus_cents  INTEGER,
    benefits_notes       TEXT,
    start_date           TEXT,
    expires_at           TEXT,
    status               TEXT NOT NULL DEFAULT 'Pending',
    raw_letter_text      TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_offers_application ON offers(application_id);
CREATE INDEX IF NOT EXISTS idx_offers_expires     ON offers(expires_at) WHERE expires_at IS NOT NULL AND status = 'Pending';

-- Reminders and follow-up tasks
CREATE TABLE IF NOT EXISTS reminders (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title           TEXT NOT NULL,
    body            TEXT,
    due_at          TEXT NOT NULL,
    completed       INTEGER NOT NULL DEFAULT 0,
    application_id  TEXT REFERENCES applications(id) ON DELETE CASCADE,
    job_id          TEXT REFERENCES jobs(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_reminders_due       ON reminders(due_at) WHERE completed = 0;
CREATE INDEX IF NOT EXISTS idx_reminders_app       ON reminders(application_id);
```

### Module Structure

```
lazyjob-core/src/application/
├── mod.rs          # pub use re-exports; module registry
├── types.rs        # ApplicationStage, Application, StageTransition, Interview, Offer, Reminder, PipelineMetrics
├── error.rs        # ApplicationError, Result<T>
├── events.rs       # WorkflowEvent, EventTx, EventRx, event_bus()
├── repository.rs   # ApplicationRepository trait, ApplicationFilter, ActionRequired
├── sqlite_repo.rs  # SqliteApplicationRepository — sqlx impl
├── workflows/
│   ├── mod.rs
│   ├── apply.rs         # ApplyWorkflow
│   ├── move_stage.rs    # MoveStageWorkflow
│   ├── schedule.rs      # ScheduleInterviewWorkflow
│   ├── log_contact.rs   # LogContactWorkflow
│   └── offer.rs         # RecordOfferWorkflow
├── metrics.rs      # PipelineMetrics computation, SQL aggregations
└── reminder.rs     # ReminderPoller background task

lazyjob-tui/src/views/
├── kanban.rs       # KanbanView — kanban board; one column per active stage
├── action_queue.rs # ActionQueueWidget — overdue, expiring offers, interviews
└── app_detail.rs   # ApplicationDetailView — full application pane
```

---

## Implementation Phases

### Phase 1 — Core Domain and Repository (MVP)

**Goal**: The state machine, SQLite schema, and repository are functional. Tests pass.

#### Step 1.1 — Migration file `002_applications.sql`

- File: `lazyjob-core/migrations/002_applications.sql`
- Contains all DDL from the schema section above.
- Verify: `sqlx migrate run` succeeds against a fresh SQLite file.

#### Step 1.2 — Types module

- File: `lazyjob-core/src/application/types.rs`
- Implement `ApplicationStage` with `valid_transitions()`, `can_transition_to()`, `is_terminal()`, `label()`, `active_stages()`.
- Implement `FromStr` for `ApplicationStage` — used when reading TEXT from SQLite.
- Add `sqlx::Type` derives (encode/decode as TEXT).
- Implement `Application`, `StageTransition`, `ApplicationContact`, `Interview`, `Offer`, `Reminder` structs.
- All fields use `DateTime<Utc>` for timestamps; SQLite stores as ISO-8601 TEXT.
- Key crate APIs:
  - `uuid::Uuid::new_v4()` for ID generation
  - `chrono::Utc::now()` for timestamps
  - `serde::{Serialize, Deserialize}` for JSON export
- Verify: `cargo test application::types` passes with transition matrix unit tests.

#### Step 1.3 — Error type

- File: `lazyjob-core/src/application/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum ApplicationError {
    #[error("invalid stage transition: {from:?} → {to:?}")]
    InvalidTransition { from: ApplicationStage, to: ApplicationStage },

    #[error("application not found: {0}")]
    NotFound(Uuid),

    #[error("duplicate application: job {job_id} already has application {existing_id}")]
    DuplicateApplication { job_id: Uuid, existing_id: Uuid },

    #[error("action not valid in stage {current:?}: {action}")]
    InvalidStageForAction { current: ApplicationStage, action: &'static str },

    #[error("application is archived")]
    Archived,

    #[error("unknown stage: {0}")]
    UnknownStage(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, ApplicationError>;
```

#### Step 1.4 — `SqliteApplicationRepository`

- File: `lazyjob-core/src/application/sqlite_repo.rs`
- Hold `pool: sqlx::SqlitePool`.
- Implement `ApplicationRepository` trait.
- Use `sqlx::query!` macros where possible; fall back to `sqlx::query_as!` for complex types.

Key operations:

**insert**:
```rust
sqlx::query!(
    "INSERT INTO applications (id, job_id, stage, notes, priority, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)",
    id_str, job_id_str, stage_str, app.notes, priority_int, now, now
).execute(&self.pool).await?;
```

**update_stage** (atomic: update stage + insert transition in one transaction):
```rust
let mut tx = self.pool.begin().await?;
sqlx::query!(
    "UPDATE applications SET stage = ?, updated_at = ? WHERE id = ?",
    new_stage_str, now_str, id_str
).execute(&mut *tx).await?;
sqlx::query!(
    "INSERT INTO application_transitions (id, application_id, from_stage, to_stage, reason, transitioned_at)
     VALUES (?, ?, ?, ?, ?, ?)",
    transition_id_str, id_str, from_str, to_str, reason, now_str
).execute(&mut *tx).await?;
tx.commit().await?;
```

**list with filter** — build dynamic WHERE clause:
```rust
let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
    "SELECT * FROM applications WHERE archived_at IS NULL"
);
if let Some(stages) = &filter.stages {
    qb.push(" AND stage IN (");
    let mut sep = qb.separated(", ");
    for s in stages { sep.push_bind(s.label()); }
    qb.push(")");
}
if filter.stale_only {
    qb.push(" AND (last_contact_at IS NULL OR last_contact_at < datetime('now', '-14 days'))");
}
qb.push(" ORDER BY priority DESC, updated_at DESC");
```

**metrics** — single aggregation query:
```rust
sqlx::query_as!(MetricsRow,
    r#"SELECT
        stage,
        COUNT(*) AS count,
        AVG(JULIANDAY('now') - JULIANDAY(updated_at)) AS avg_days
     FROM applications
     WHERE archived_at IS NULL
     GROUP BY stage"#
).fetch_all(&self.pool).await?
```

**action_required** — composed from sub-queries:
- Overdue follow-ups: `WHERE next_follow_up_at < datetime('now') AND archived_at IS NULL`
- Expiring offers: JOIN offers where `expires_at < datetime('now', '+5 days') AND status = 'Pending'`
- Upcoming interviews: JOIN interviews where `scheduled_at BETWEEN datetime('now') AND datetime('now', '+1 day')`
- Stale: `WHERE last_contact_at < datetime('now', '-14 days') AND stage NOT IN ('Accepted','Rejected','Withdrawn','Discovered')`

Verify:
- `cargo test application::sqlite_repo` with an in-memory `SqlitePool` (`:memory:`).
- Test every transition in the valid_transitions matrix and several invalid ones.

---

### Phase 2 — Workflow Orchestrators

**Goal**: Business logic is encapsulated in workflow structs. Each workflow is independently unit-testable via a mock repository.

#### Step 2.1 — `ApplyWorkflow`

- File: `lazyjob-core/src/application/workflows/apply.rs`

```rust
pub struct ApplyWorkflow {
    pub repo:         Arc<dyn ApplicationRepository>,
    pub job_repo:     Arc<dyn JobRepository>,
    pub event_tx:     EventTx,
}

impl ApplyWorkflow {
    pub async fn execute(&self, input: ApplyInput) -> Result<Application> {
        // 1. Guard: job must exist
        let job = self.job_repo.get(input.job_id).await?
            .ok_or(ApplicationError::NotFound(input.job_id))?;

        // 2. Guard: no existing application for this job
        if let Some(existing) = self.repo.list(&ApplicationFilter {
            job_id: Some(input.job_id), ..Default::default()
        }).await?.into_iter().next() {
            return Err(ApplicationError::DuplicateApplication {
                job_id: input.job_id,
                existing_id: existing.id,
            });
        }

        // 3. Create application at Applied stage
        let app = Application {
            id:                      Uuid::new_v4(),
            job_id:                  input.job_id,
            stage:                   ApplicationStage::Applied,
            resume_version_id:       input.resume_version_id,
            cover_letter_version_id: input.cover_letter_version_id,
            notes:                   input.notes.unwrap_or_default(),
            priority:                ApplicationPriority::Medium,
            last_contact_at:         None,
            next_follow_up_at:       Some(Utc::now() + chrono::Duration::days(7)),
            archived_at:             None,
            created_at:              Utc::now(),
            updated_at:              Utc::now(),
        };
        self.repo.insert(&app).await?;

        // 4. Insert initial transition (None → Applied)
        // update_stage handles transition insert; here we insert a synthetic "created" transition
        let _ = self.repo.update_stage(app.id, ApplicationStage::Applied, Some("application created")).await;

        // 5. Emit event
        let _ = self.event_tx.send(WorkflowEvent::ApplicationCreated(app.id));

        Ok(app)
    }
}

pub struct ApplyInput {
    pub job_id:                  Uuid,
    pub resume_version_id:       Option<Uuid>,
    pub cover_letter_version_id: Option<Uuid>,
    pub notes:                   Option<String>,
}
```

#### Step 2.2 — `MoveStageWorkflow`

- File: `lazyjob-core/src/application/workflows/move_stage.rs`

```rust
pub struct MoveStageWorkflow {
    pub repo:     Arc<dyn ApplicationRepository>,
    pub event_tx: EventTx,
}

impl MoveStageWorkflow {
    pub async fn execute(&self, id: Uuid, target: ApplicationStage, reason: Option<String>) -> Result<Application> {
        let app = self.repo.get(id).await?
            .ok_or(ApplicationError::NotFound(id))?;

        if app.archived_at.is_some() {
            return Err(ApplicationError::Archived);
        }

        if !app.stage.can_transition_to(target) {
            return Err(ApplicationError::InvalidTransition { from: app.stage, to: target });
        }

        // Side effects before transition
        let next_follow_up = match target {
            ApplicationStage::Applied     => Some(Utc::now() + chrono::Duration::days(7)),
            ApplicationStage::PhoneScreen => Some(Utc::now() + chrono::Duration::days(3)),
            _                             => None,
        };

        // Persist transition atomically
        self.repo.update_stage(id, target, reason.as_deref()).await?;

        // Update follow-up if needed
        if let Some(due) = next_follow_up {
            self.repo.insert_reminder(&Reminder {
                id:             Uuid::new_v4(),
                title:          format!("Follow up on application"),
                body:           None,
                due_at:         due,
                completed:      false,
                application_id: Some(id),
                job_id:         None,
                created_at:     Utc::now(),
            }).await?;
        }

        let _ = self.event_tx.send(WorkflowEvent::StageChanged {
            application_id: id,
            from: app.stage,
            to: target,
        });

        self.repo.get(id).await?.ok_or(ApplicationError::NotFound(id))
    }
}
```

#### Step 2.3 — `ScheduleInterviewWorkflow`

- File: `lazyjob-core/src/application/workflows/schedule.rs`

```rust
pub struct ScheduleInterviewWorkflow {
    pub repo:     Arc<dyn ApplicationRepository>,
    pub event_tx: EventTx,
}

impl ScheduleInterviewWorkflow {
    pub async fn execute(&self, input: ScheduleInput) -> Result<Interview> {
        let app = self.repo.get(input.application_id).await?
            .ok_or(ApplicationError::NotFound(input.application_id))?;

        // Only allow if in an interview-capable stage
        if !matches!(app.stage,
            ApplicationStage::PhoneScreen |
            ApplicationStage::Technical   |
            ApplicationStage::OnSite
        ) {
            return Err(ApplicationError::InvalidStageForAction {
                current: app.stage,
                action:  "schedule_interview",
            });
        }

        let interview = Interview {
            id:               Uuid::new_v4(),
            application_id:   input.application_id,
            interview_type:   input.interview_type,
            scheduled_at:     input.scheduled_at,
            duration_mins:    input.duration_mins,
            location:         input.location,
            meeting_url:      input.meeting_url,
            interviewer_names: input.interviewer_names,
            status:           InterviewStatus::Scheduled,
            self_rating:      None,
            feedback_notes:   None,
            created_at:       Utc::now(),
            updated_at:       Utc::now(),
        };

        self.repo.insert_interview(&interview).await?;

        // Create prep reminder 24h before interview
        if let Some(scheduled) = input.scheduled_at {
            let reminder_due = scheduled - chrono::Duration::hours(24);
            if reminder_due > Utc::now() {
                self.repo.insert_reminder(&Reminder {
                    id:             Uuid::new_v4(),
                    title:          format!("Prep: {} interview", interview.interview_type.label()),
                    body:           Some("Review role, company, STAR stories".into()),
                    due_at:         reminder_due,
                    completed:      false,
                    application_id: Some(input.application_id),
                    job_id:         None,
                    created_at:     Utc::now(),
                }).await?;
            }
        }

        let _ = self.event_tx.send(WorkflowEvent::InterviewScheduled {
            application_id: input.application_id,
            interview_id:   interview.id,
        });

        Ok(interview)
    }
}
```

#### Step 2.4 — `LogContactWorkflow`

- File: `lazyjob-core/src/application/workflows/log_contact.rs`
- Records an `ApplicationContact`, updates `last_contact_at` on the application, and resets the `next_follow_up_at` clock.

```rust
pub struct LogContactWorkflow {
    pub repo:     Arc<dyn ApplicationRepository>,
    pub event_tx: EventTx,
}

impl LogContactWorkflow {
    pub async fn execute(&self, input: LogContactInput) -> Result<ApplicationContact> {
        let contact = ApplicationContact {
            id:             Uuid::new_v4(),
            application_id: input.application_id,
            name:           input.name,
            role:           input.role,
            email:          input.email,
            linkedin_url:   input.linkedin_url,
            stage:          input.stage,
            contacted_at:   input.contacted_at.unwrap_or_else(Utc::now),
            notes:          input.notes,
        };

        self.repo.insert_contact(&contact).await?;

        // Reset stale clock: update last_contact_at + schedule next follow-up
        let mut app = self.repo.get(input.application_id).await?
            .ok_or(ApplicationError::NotFound(input.application_id))?;
        app.last_contact_at = Some(contact.contacted_at);
        app.next_follow_up_at = input.follow_up_in_days
            .map(|d| contact.contacted_at + chrono::Duration::days(d as i64));
        app.updated_at = Utc::now();
        self.repo.update(&app).await?;

        let _ = self.event_tx.send(WorkflowEvent::MetricsInvalidated);
        Ok(contact)
    }
}
```

#### Step 2.5 — `RecordOfferWorkflow`

- File: `lazyjob-core/src/application/workflows/offer.rs`
- Validates application is in Offer stage, inserts `Offer` record, emits event.

---

### Phase 3 — ReminderPoller Background Task

**Goal**: Expired reminders surface in the TUI Action Queue without user having to poll.

- File: `lazyjob-core/src/application/reminder.rs`

```rust
pub struct ReminderPoller {
    repo:     Arc<dyn ApplicationRepository>,
    event_tx: EventTx,
    interval: std::time::Duration,
}

impl ReminderPoller {
    pub fn new(repo: Arc<dyn ApplicationRepository>, event_tx: EventTx) -> Self {
        Self { repo, event_tx, interval: std::time::Duration::from_secs(300) }
    }

    pub async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(self.interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.repo.pending_reminders().await {
                        Ok(reminders) => {
                            for r in reminders {
                                if r.due_at <= Utc::now() {
                                    let _ = self.event_tx.send(WorkflowEvent::ReminderDue {
                                        reminder_id:    r.id,
                                        application_id: r.application_id,
                                    });
                                }
                            }
                        }
                        Err(e) => tracing::error!("reminder poll failed: {e}"),
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    }
}
```

- Spawned from `lazyjob-tui/src/app.rs` via `tokio::spawn(poller.run(shutdown_rx))`.
- The TUI subscribes to `EventRx` and re-renders the Action Queue on `ReminderDue`.

---

### Phase 4 — TUI Kanban View

**Goal**: Users see all active applications as a kanban board with column-per-stage.

- File: `lazyjob-tui/src/views/kanban.rs`

```rust
pub struct KanbanView {
    apps_by_stage: IndexMap<ApplicationStage, Vec<ApplicationSummary>>,
    selected_stage: usize,   // column index
    selected_card:  usize,   // card index within column
    focus:          KanbanFocus,
}

#[derive(Clone)]
pub struct ApplicationSummary {
    pub id:         Uuid,
    pub company:    String,
    pub title:      String,
    pub stage:      ApplicationStage,
    pub priority:   ApplicationPriority,
    pub days_stale: Option<i64>,
}

pub enum KanbanFocus { Column, Card, Detail }
```

**Layout** (`ratatui::layout`):
- Horizontal split: one `Rect` per active stage column (7 columns).
- Column widths: `Constraint::Ratio(1, 7)` repeated 7 times.
- Each column: header bar (stage name + count), scrollable list of cards.
- Selected card: highlighted with `Style::default().add_modifier(Modifier::REVERSED)`.
- Stale cards: rendered with `Color::Yellow`.
- Priority indicators: `■` (urgent), `▲` (high), `●` (medium), `○` (low).

```rust
impl Widget for KanbanView {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, 7); 7])
            .split(area);

        for (i, stage) in ApplicationStage::active_stages().iter().enumerate() {
            let apps = self.apps_by_stage.get(stage).cloned().unwrap_or_default();
            let col = KanbanColumn {
                stage: *stage,
                apps,
                is_focused: self.selected_stage == i,
                selected_card: if self.selected_stage == i { Some(self.selected_card) } else { None },
            };
            col.render(cols[i], buf);
        }
    }
}
```

**Keybindings** (normal mode, `KeyContext::Kanban`):
| Key | Action |
|-----|--------|
| `h` / `←` | Move to left column |
| `l` / `→` | Move to right column |
| `j` / `↓` | Move to next card in column |
| `k` / `↑` | Move to prev card in column |
| `Enter` | Open application detail pane |
| `m` | Open move-stage modal |
| `n` | Create new application |
| `a` | Archive selected application |
| `/` | Enter search/filter mode |
| `?` | Show keybinding help |

**Move-stage modal**:
- Overlay rendered with `ratatui::widgets::Clear` covering center 50% of screen.
- Lists valid transitions for current stage with highlight.
- Pressing `Enter` on a transition calls `MoveStageWorkflow::execute`.
- Optional reason field: user can type a note before confirming.

---

### Phase 5 — Action Required Queue

- File: `lazyjob-tui/src/views/action_queue.rs`

```rust
pub struct ActionQueueWidget {
    items: Vec<ActionRequired>,
    selected: usize,
}

impl Widget for ActionQueueWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render as bordered list, each item with icon + description + actions
        // ⏰ Follow up with Stripe (3d overdue) — [f]ollow up  [s]nooze  [d]ismiss
        // 📅 Interview tomorrow: Google SRE — [p]rep  [v]iew
        // 💼 Offer from Datadog, 5d to decide — [n]egotiate  [a]ccept  [x]decline
    }
}
```

Populated by calling `repo.action_required()` on each `MetricsInvalidated` event or on a 60-second tick.

---

### Phase 6 — Bulk Actions

**Goal**: Users can select multiple applications and apply an action to all.

**TUI multi-select**: In `KanbanView`, pressing `Space` toggles a card into a selection set (`HashSet<Uuid>`). A `BulkActionBar` is rendered at the bottom when the set is non-empty.

```rust
pub struct BulkActionBar {
    count: usize,
    actions: Vec<BulkAction>,
}

pub enum BulkAction {
    MoveStage(ApplicationStage),
    Archive,
    Delete,
    ClearFollowUp,
}
```

**Confirmation dialog**: Bulk destructive actions (Delete) always show a confirmation overlay with item count before executing.

**Undo support**: Keep a `Vec<UndoEntry>` (max 10 entries) per session. Each `UndoEntry` stores the previous `Application` state as JSON. `Ctrl+Z` in normal mode applies the last undo entry.

---

## Key Crate APIs

```rust
// sqlx
sqlx::SqlitePool::connect("sqlite:~/.lazyjob/lazyjob.db").await?
sqlx::migrate!("./migrations").run(&pool).await?
sqlx::query!("UPDATE ...", params...).execute(&pool).await?
sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT ...").push_bind(val).build_query_as::<Row>().fetch_all(&pool).await?
let mut tx = pool.begin().await?; tx.commit().await?

// uuid
uuid::Uuid::new_v4()           // ID generation
uuid::Uuid::parse_str(s)?      // from TEXT column

// chrono
chrono::Utc::now()
chrono::Duration::days(7)
dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()  // SQLite TEXT encoding

// tokio
tokio::sync::broadcast::channel(256)          // WorkflowEvent bus
tokio::sync::watch::channel(false)            // shutdown signal
tokio::time::interval(Duration::from_secs(300)) // ReminderPoller

// ratatui
ratatui::layout::Layout::default().direction(Direction::Horizontal).constraints(...)
ratatui::widgets::Clear                        // overlay background erase
ratatui::text::{Line, Span}                    // rich text cells
ratatui::style::{Style, Color, Modifier}
```

---

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum ApplicationError {
    #[error("invalid stage transition: {from:?} → {to:?}")]
    InvalidTransition { from: ApplicationStage, to: ApplicationStage },

    #[error("application not found: {0}")]
    NotFound(Uuid),

    #[error("duplicate application: job {job_id} already has application {existing_id}")]
    DuplicateApplication { job_id: Uuid, existing_id: Uuid },

    #[error("action '{action}' not valid in stage {current:?}")]
    InvalidStageForAction { current: ApplicationStage, action: &'static str },

    #[error("application is archived")]
    Archived,

    #[error("unknown stage string: {0}")]
    UnknownStage(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("job not found: {0}")]
    JobNotFound(Uuid),
}

pub type Result<T> = std::result::Result<T, ApplicationError>;
```

The TUI catches `ApplicationError` at the action dispatch site and renders it as an inline status-bar error message for 3 seconds. It never panics on workflow errors.

---

## Testing Strategy

### Unit Tests — `lazyjob-core`

**Transition matrix** (`application/types.rs`):
```rust
#[test]
fn valid_transitions_cover_all_expected_paths() {
    use ApplicationStage::*;
    assert!(Discovered.can_transition_to(Interested));
    assert!(Applied.can_transition_to(Offer));      // fast track
    assert!(!Accepted.can_transition_to(Applied));  // terminal
    assert!(!Discovered.can_transition_to(Technical)); // skipping too many
}
```

**Repository** — use `sqlx::sqlite::SqlitePoolOptions::new().connect(":memory:").await?` + run migrations:
```rust
#[sqlx::test(migrations = "./migrations")]
async fn apply_workflow_prevents_duplicate(pool: SqlitePool) {
    let repo = SqliteApplicationRepository::new(pool.clone());
    let (tx, _rx) = event_bus();
    let workflow = ApplyWorkflow { repo: Arc::new(repo), job_repo: ..., event_tx: tx };
    let first = workflow.execute(ApplyInput { job_id: JOB_ID, ... }).await.unwrap();
    let err   = workflow.execute(ApplyInput { job_id: JOB_ID, ... }).await.unwrap_err();
    assert!(matches!(err, ApplicationError::DuplicateApplication { .. }));
}
```

**Stage transition atomicity**:
```rust
#[sqlx::test(migrations = "./migrations")]
async fn update_stage_inserts_transition_record(pool: SqlitePool) {
    // Insert application, call update_stage, verify transition row exists
}
```

**Stale detection query**:
```rust
#[sqlx::test(migrations = "./migrations")]
async fn stale_filter_excludes_recently_contacted(pool: SqlitePool) {
    // Insert two apps: one contacted 20 days ago, one 5 days ago
    // list(stale_only: true) returns only the 20-day app
}
```

**PipelineMetrics computation**:
```rust
#[sqlx::test(migrations = "./migrations")]
async fn metrics_counts_by_stage(pool: SqlitePool) {
    // Insert 3 Applied, 1 Technical, 1 Rejected
    // metrics() returns by_stage[Applied]=3, by_stage[Technical]=1
    // response_rate = 2/4 (Applied+Technical out of active)
}
```

### Integration Tests — `lazyjob-core/tests/`

- `apply_and_move_full_pipeline.rs`: Apply → PhoneScreen → Technical → Offer → Accepted end-to-end.
- `bulk_archive.rs`: Insert 10 applications, archive all by filter, verify list returns 0 active.
- `reminder_poller.rs`: Insert overdue reminder, run poller one tick, verify event emitted.

### TUI Tests — `lazyjob-tui`

- `kanban_render.rs`: Construct `KanbanView` with synthetic data, call `render()` into a `TestBackend`, assert cell content.
- `move_stage_modal.rs`: Simulate `m` keypress, assert overlay appears, simulate `Enter`, assert `MoveStageWorkflow::execute` called.

---

## Open Questions

1. **Cross-source deduplication**: When the same job is posted on Greenhouse and LinkedIn (same company, title, location), should LazyJob merge them into one `Application` or keep two linked records? The gap analysis (GAP-59) calls this critical. Decision needed before implementing `ApplyWorkflow` duplicate check fully.

2. **Undo depth and durability**: Is the 10-entry in-memory undo stack sufficient, or should undo history persist to SQLite across sessions? Session-only is simpler and probably fine for v1.

3. **`ApplicationStage::Discovered` as default**: The spec says `Discovered` is set when a job is first found. Should `Application` creation be separate from job creation, or should discovering a job implicitly create an `Application` at `Discovered`? Current plan: `ApplyWorkflow` creates at `Applied`; a separate `SaveForLaterWorkflow` creates at `Interested`. Discovery auto-creates nothing until user acts.

4. **Multi-offer comparison**: GAP-60 identifies this as critical. The `offers` table is designed here, but the comparison view (`XX-multi-offer-comparison.md`) is a separate spec/plan task and should cross-link to this schema.

5. **Async coding challenge sub-state**: GAP-64. The `Technical` stage doesn't currently model the sent/pending/submitted sub-state for async challenges. For v1, users record challenge details in `interview.meeting_url` + `interview.notes`. A dedicated `challenges` table is deferred.

6. **`sqlx::Type` for enums stored as TEXT**: SQLite doesn't enforce column value constraints for enum TEXT columns. We rely on `ApplicationStage::from_str` during reads to catch stale/invalid data. An alternative is a CHECK constraint on the column, but sqlx migrations don't validate this unless we explicitly add it.

---

## Related Specs

- [`specs/application-state-machine.md`](./application-state-machine.md) — extended state machine with full transition matrix and `application_transitions` DDL
- [`specs/application-workflow-actions.md`](./application-workflow-actions.md) — detailed workflow design, human-in-the-loop boundaries, anti-spam architecture
- [`specs/application-pipeline-metrics.md`](./application-pipeline-metrics.md) — pipeline metrics queries, stale detection, morning digest, action required queue
- [`specs/06-gaps-application-workflow.md`](./06-gaps-application-workflow.md) — gap analysis: cross-source dedup, multi-offer comparison, bulk operations, deadline tracking
- [`specs/04-sqlite-persistence-implementation-plan.md`](./04-sqlite-persistence-implementation-plan.md) — SQLite foundation this plan builds on
- [`specs/09-tui-design-keybindings-implementation-plan.md`](./09-tui-design-keybindings-implementation-plan.md) — TUI widget primitives used by kanban view
- [`specs/XX-application-cross-source-deduplication.md`](./XX-application-cross-source-deduplication.md) — cross-source dedup spec (separate plan task #43)
- [`specs/XX-multi-offer-comparison.md`](./XX-multi-offer-comparison.md) — multi-offer comparison spec (separate plan task #50)
