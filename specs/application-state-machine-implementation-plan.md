# Implementation Plan: Application State Machine

## Status
Draft

## Related Spec
[`specs/application-state-machine.md`](./application-state-machine.md)

## Overview

The application state machine models the lifecycle of a single job application across ten ordered stages — from first discovery through terminal resolution (Accepted, Rejected, or Withdrawn). It is the canonical source of truth for where each application stands and provides the data backbone that every other subsystem queries: the TUI kanban view, pipeline metrics, reminder poller, and Ralph AI triggers all read stage state from a single well-defined source.

The design encodes the full allowed transition matrix as a `match` expression in `ApplicationStage::can_transition_to()` rather than inferring validity from stage ordering. This prevents silent acceptance of invalid transitions, enables backward data-entry corrections without compromising forward integrity, and keeps all transition logic in one auditable place. Every stage change is atomically written to both the `applications.stage` column and an append-only `application_transitions` log — the log is never modified, only extended.

The `lazyjob-core` crate owns this domain entirely. The TUI consumes it through the repository trait; no rendering code has any knowledge of SQLite internals. The Ralph subprocess and workflow orchestrators (see `application-workflow-actions.md`) also operate through the same trait, enabling clean dependency injection and mockable test doubles.

## Prerequisites

### Specs / Plans That Must Be Implemented First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations infrastructure
- `specs/01-architecture-implementation-plan.md` — crate layout, workspace `Cargo.toml`, `lazyjob-core` skeleton

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
sqlx       = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
tokio      = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }
uuid       = { version = "1", features = ["v4", "serde"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
chrono     = { version = "0.4", features = ["serde"] }
thiserror  = "2"
anyhow     = "1"
tracing    = "0.1"

[dev-dependencies]
tokio      = { version = "1", features = ["full"] }
sqlx       = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
```

---

## Architecture

### Crate Placement

| Component | Crate | Reason |
|---|---|---|
| `ApplicationStage` enum + `can_transition_to` | `lazyjob-core` | Pure domain logic, no I/O |
| `Application`, `StageTransition`, `Interview`, `Offer` structs | `lazyjob-core` | Shared by TUI, CLI, Ralph loops |
| `ApplicationRepository` async trait | `lazyjob-core` | I/O boundary abstraction |
| `SqliteApplicationRepository` | `lazyjob-core` | Concrete SQLite implementation |
| `ApplicationFilter` | `lazyjob-core` | Query builder value object |
| `ApplicationError` enum | `lazyjob-core` | Public error boundary |
| `StageTransitionEvent` broadcast type | `lazyjob-core` | Cross-crate notification type |

### Core Types

```rust
// lazyjob-core/src/application/stage.rs

use serde::{Deserialize, Serialize};

/// The 10-stage hiring pipeline lifecycle.
/// Serializes to/from TEXT in SQLite via sqlx::Type derive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "PascalCase")]
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
    /// Terminal stages: no outgoing transitions are allowed.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected | Self::Withdrawn)
    }

    pub fn is_active(self) -> bool {
        !self.is_terminal()
    }

    /// Human-readable label for TUI display.
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

    /// Column index in the kanban view (Accepted/Rejected/Withdrawn share the last column).
    pub fn column_index(self) -> usize {
        match self {
            Self::Discovered  => 0,
            Self::Interested  => 1,
            Self::Applied     => 2,
            Self::PhoneScreen => 3,
            Self::Technical   => 4,
            Self::OnSite      => 5,
            Self::Offer       => 6,
            Self::Accepted    => 7,
            Self::Rejected    => 7,
            Self::Withdrawn   => 7,
        }
    }

    /// Canonical stage order used in kanban column headers and metrics breakdowns.
    pub fn display_order() -> &'static [Self] {
        use ApplicationStage::*;
        &[
            Discovered, Interested, Applied, PhoneScreen,
            Technical, OnSite, Offer, Accepted, Rejected, Withdrawn,
        ]
    }

    /// Returns true if this → next is a permitted transition.
    ///
    /// The matrix is explicit rather than inferred from ordinal ordering.
    /// Terminal stages never have allowed outgoing transitions.
    /// Backward corrections are permitted to support user data-entry fixes.
    pub fn can_transition_to(self, next: ApplicationStage) -> bool {
        use ApplicationStage::*;
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // ── Standard forward progression ────────────────────────────
            (Discovered, Interested)
            | (Interested, Applied)
            | (Applied, PhoneScreen)
            | (PhoneScreen, Technical)
            | (Technical, OnSite)
            | (OnSite, Offer)
            | (Offer, Accepted) => true,

            // ── Stage-skipping (real pipelines often bypass stages) ─────
            (Discovered, Applied)
            | (Applied, OnSite)
            | (Applied, Offer)
            | (PhoneScreen, Offer)
            | (Technical, Offer) => true,

            // ── Backward correction (data entry error recovery) ─────────
            (Interested, Discovered)
            | (Applied, Interested)
            | (PhoneScreen, Applied)
            | (Technical, PhoneScreen)
            | (OnSite, Technical)
            | (Offer, OnSite) => true,

            // ── Any non-terminal → Rejected or Withdrawn ─────────────────
            (_, Rejected) => true,
            // Accepted cannot be Withdrawn (you accepted; withdrawal not applicable)
            (_, Withdrawn) => !matches!(self, Accepted | Rejected | Withdrawn),

            _ => false,
        }
    }

    /// List all stages reachable from `self` in one step.
    pub fn reachable_from(self) -> Vec<ApplicationStage> {
        ApplicationStage::display_order()
            .iter()
            .copied()
            .filter(|&next| self.can_transition_to(next))
            .collect()
    }
}

impl std::fmt::Display for ApplicationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

impl std::str::FromStr for ApplicationStage {
    type Err = crate::application::error::ApplicationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
            other => Err(crate::application::error::ApplicationError::InvalidStage(
                other.to_owned(),
            )),
        }
    }
}
```

```rust
// lazyjob-core/src/application/model.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use super::stage::ApplicationStage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: Uuid,
    pub job_id: Uuid,
    pub stage: ApplicationStage,
    pub resume_version_id: Option<Uuid>,
    pub cover_letter_version_id: Option<Uuid>,
    pub notes: String,
    pub last_contact_at: Option<DateTime<Utc>>,
    pub next_follow_up_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageTransition {
    pub id: Uuid,
    pub application_id: Uuid,
    pub from_stage: ApplicationStage,
    pub to_stage: ApplicationStage,
    pub reason: Option<String>,
    pub transitioned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContact {
    pub id: Uuid,
    pub application_id: Uuid,
    pub name: String,
    pub role: Option<String>,  // "recruiter" | "hiring_manager" | "interviewer"
    pub email: Option<String>,
    pub linkedin_url: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "PascalCase")]
pub enum InterviewType {
    PhoneScreen,
    Technical,
    OnSite,
    Panel,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "PascalCase")]
pub enum InterviewStatus {
    Scheduled,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interview {
    pub id: Uuid,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<u32>,
    pub location: Option<String>,
    pub meeting_url: Option<String>,
    pub interviewers: Vec<String>,  // serialized as JSON in SQLite
    pub status: InterviewStatus,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "PascalCase")]
pub enum OfferStatus {
    Pending,
    Countered,
    Accepted,
    Declined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id: Uuid,
    pub application_id: Uuid,
    /// Annual base salary in cents to avoid float precision issues.
    pub base_salary_cents: Option<i64>,
    pub equity_pct: Option<f64>,
    pub equity_cliff_months: Option<u32>,
    pub equity_vest_months: Option<u32>,
    /// Signing bonus in cents.
    pub signing_bonus_cents: Option<i64>,
    pub annual_bonus_target_pct: Option<f64>,
    pub benefits_notes: Option<String>,
    pub offer_date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub status: OfferStatus,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

```rust
// lazyjob-core/src/application/filter.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;
use super::stage::ApplicationStage;

/// Query filter value object for listing applications.
/// All fields are additive ANDs when present.
#[derive(Debug, Clone, Default)]
pub struct ApplicationFilter {
    /// If set, only return applications in one of these stages.
    pub stages: Option<Vec<ApplicationStage>>,
    /// If set, only return the application for this job.
    pub job_id: Option<Uuid>,
    /// If set, only return applications updated after this time.
    pub since: Option<DateTime<Utc>>,
    /// If true, exclude Accepted/Rejected/Withdrawn.
    pub active_only: bool,
    /// Pagination: offset into results.
    pub offset: u32,
    /// Pagination: max results (default 200 if 0).
    pub limit: u32,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/application/repository.rs

use async_trait::async_trait;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use super::{
    error::ApplicationError,
    filter::ApplicationFilter,
    model::{Application, ApplicationContact, Interview, Offer, StageTransition},
    stage::ApplicationStage,
};

pub type Result<T> = std::result::Result<T, ApplicationError>;

#[async_trait]
pub trait ApplicationRepository: Send + Sync {
    // ── Application CRUD ───────────────────────────────────────────────

    /// Insert a new application record (stage defaults to Discovered).
    async fn insert(&self, app: &Application) -> Result<Uuid>;

    /// Fetch a single application by ID.
    async fn get(&self, id: Uuid) -> Result<Application>;

    /// List applications matching filter criteria.
    async fn list(&self, filter: &ApplicationFilter) -> Result<Vec<Application>>;

    /// Atomically update stage + append to application_transitions log.
    /// Returns `ApplicationError::InvalidTransition` if the transition is not permitted.
    async fn update_stage(
        &self,
        id: Uuid,
        new_stage: ApplicationStage,
        reason: Option<&str>,
    ) -> Result<StageTransition>;

    async fn update_notes(&self, id: Uuid, notes: &str) -> Result<()>;

    async fn update_follow_up(&self, id: Uuid, at: Option<DateTime<Utc>>) -> Result<()>;

    async fn update_last_contact(&self, id: Uuid, at: DateTime<Utc>) -> Result<()>;

    async fn delete(&self, id: Uuid) -> Result<()>;

    // ── Transition history ─────────────────────────────────────────────

    /// Fetch the full ordered transition log for an application.
    async fn list_transitions(&self, app_id: Uuid) -> Result<Vec<StageTransition>>;

    /// Fetch the most recent transition for an application.
    async fn latest_transition(&self, app_id: Uuid) -> Result<Option<StageTransition>>;

    // ── Aggregations ───────────────────────────────────────────────────

    /// Count of applications per stage for the kanban header and metrics.
    async fn count_by_stage(&self) -> Result<HashMap<ApplicationStage, usize>>;

    /// Applications with next_follow_up_at < now (overdue reminders).
    async fn list_overdue_follow_ups(&self, limit: u32) -> Result<Vec<Application>>;

    /// Applications in Offer stage with expiry_date < now + days.
    async fn list_expiring_offers(&self, within_days: u32) -> Result<Vec<(Application, Offer)>>;

    // ── Interview CRUD ─────────────────────────────────────────────────

    async fn insert_interview(&self, interview: &Interview) -> Result<Uuid>;
    async fn list_interviews(&self, app_id: Uuid) -> Result<Vec<Interview>>;
    async fn update_interview_status(
        &self,
        interview_id: Uuid,
        status: super::model::InterviewStatus,
    ) -> Result<()>;

    // ── Offer CRUD ─────────────────────────────────────────────────────

    async fn insert_offer(&self, offer: &Offer) -> Result<Uuid>;
    async fn get_offer(&self, app_id: Uuid) -> Result<Option<Offer>>;
    async fn update_offer(&self, offer: &Offer) -> Result<()>;

    // ── Contacts ───────────────────────────────────────────────────────

    async fn insert_contact(&self, contact: &ApplicationContact) -> Result<Uuid>;
    async fn list_contacts(&self, app_id: Uuid) -> Result<Vec<ApplicationContact>>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/002_applications.sql

CREATE TABLE IF NOT EXISTS applications (
    id                       TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id                   TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    stage                    TEXT NOT NULL DEFAULT 'Discovered',
    resume_version_id        TEXT REFERENCES resume_versions(id),
    cover_letter_version_id  TEXT REFERENCES cover_letter_versions(id),
    notes                    TEXT NOT NULL DEFAULT '',
    last_contact_at          TEXT,           -- ISO 8601 UTC
    next_follow_up_at        TEXT,           -- ISO 8601 UTC
    created_at               TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at               TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Immutable append-only transition log. Never UPDATE or DELETE rows here.
CREATE TABLE IF NOT EXISTS application_transitions (
    id               TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id   TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    from_stage       TEXT NOT NULL,
    to_stage         TEXT NOT NULL,
    reason           TEXT,                   -- optional user note
    transitioned_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS application_contacts (
    id               TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id   TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    role             TEXT,                   -- 'recruiter' | 'hiring_manager' | 'interviewer'
    email            TEXT,
    linkedin_url     TEXT,
    notes            TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS interviews (
    id               TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id   TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type   TEXT NOT NULL,          -- 'PhoneScreen' | 'Technical' | 'OnSite' | 'Panel' | 'Async'
    scheduled_at     TEXT,
    duration_minutes INTEGER,
    location         TEXT,
    meeting_url      TEXT,
    interviewers     TEXT NOT NULL DEFAULT '[]',  -- JSON array of names
    status           TEXT NOT NULL DEFAULT 'Scheduled',
    notes            TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS offers (
    id                       TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id           TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    base_salary_cents        INTEGER,
    equity_pct               REAL,
    equity_cliff_months      INTEGER,
    equity_vest_months       INTEGER,
    signing_bonus_cents      INTEGER,
    annual_bonus_target_pct  REAL,
    benefits_notes           TEXT,
    offer_date               TEXT,           -- ISO 8601 date (no time)
    expiry_date              TEXT,           -- ISO 8601 date
    status                   TEXT NOT NULL DEFAULT 'Pending',
    notes                    TEXT,
    created_at               TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes for common access patterns
CREATE INDEX IF NOT EXISTS idx_applications_stage
    ON applications(stage);

CREATE INDEX IF NOT EXISTS idx_applications_job_id
    ON applications(job_id);

CREATE INDEX IF NOT EXISTS idx_applications_next_follow_up
    ON applications(next_follow_up_at)
    WHERE next_follow_up_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_application_transitions_app_id
    ON application_transitions(application_id, transitioned_at);

CREATE INDEX IF NOT EXISTS idx_interviews_app_id
    ON interviews(application_id);

CREATE INDEX IF NOT EXISTS idx_interviews_scheduled_at
    ON interviews(scheduled_at)
    WHERE status = 'Scheduled';

CREATE INDEX IF NOT EXISTS idx_offers_expiry
    ON offers(expiry_date)
    WHERE expiry_date IS NOT NULL AND status = 'Pending';
```

### Module Structure

```
lazyjob-core/
  src/
    application/
      mod.rs          -- pub use re-exports; registers submodules
      error.rs        -- ApplicationError enum (thiserror)
      stage.rs        -- ApplicationStage enum + can_transition_to
      model.rs        -- Application, StageTransition, Interview, Offer structs
      filter.rs       -- ApplicationFilter value object
      repository.rs   -- ApplicationRepository async trait
      sqlite.rs       -- SqliteApplicationRepository (concrete impl)
      events.rs       -- StageTransitionEvent broadcast type
    lib.rs            -- pub mod application;
  migrations/
    002_applications.sql
```

---

## Implementation Phases

### Phase 1 — Domain Core (Stage Enum + Models)

**Step 1.1 — Create `lazyjob-core/src/application/` directory skeleton**

Create `mod.rs`:
```rust
// lazyjob-core/src/application/mod.rs
mod error;
mod events;
mod filter;
mod model;
mod repository;
mod stage;
mod sqlite;

pub use error::ApplicationError;
pub use events::StageTransitionEvent;
pub use filter::ApplicationFilter;
pub use model::{
    Application, ApplicationContact, Interview, InterviewStatus, InterviewType,
    Offer, OfferStatus, StageTransition,
};
pub use repository::ApplicationRepository;
pub use sqlite::SqliteApplicationRepository;
pub use stage::ApplicationStage;
```

Register in `lazyjob-core/src/lib.rs`:
```rust
pub mod application;
```

**Step 1.2 — Implement `ApplicationStage` in `stage.rs`**

Use the full type definition from the Core Types section above. Derive `sqlx::Type` with `rename_all = "PascalCase"` so the enum round-trips through SQLite TEXT columns without a custom mapping.

Verification: `cargo test -p lazyjob-core application::stage` — all 10 variants serialize/deserialize to their PascalCase string name via `serde_json::to_string` and `FromStr`.

**Step 1.3 — Implement models in `model.rs`**

All four model structs: `Application`, `StageTransition`, `ApplicationContact`, `Interview`, `Offer`. Note that `Interview::interviewers` is `Vec<String>` in Rust but stored as a JSON array TEXT in SQLite — the repository layer is responsible for `serde_json::to_string` / `serde_json::from_str` round-trips.

`Offer` uses `i64` cent-denominated fields instead of `f64` to avoid floating-point comparison bugs in cost calculations.

Verification: `cargo test -p lazyjob-core application::model` — round-trip serde_json encode/decode for each struct.

**Step 1.4 — Write `ApplicationError` in `error.rs`**

```rust
// lazyjob-core/src/application/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error("invalid stage string: {0}")]
    InvalidStage(String),

    #[error("transition from {from} to {to} is not permitted")]
    InvalidTransition {
        from: crate::application::ApplicationStage,
        to: crate::application::ApplicationStage,
    },

    #[error("application not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("application is in terminal stage {0} and cannot be modified")]
    TerminalStage(crate::application::ApplicationStage),

    #[error("interview not found: {0}")]
    InterviewNotFound(uuid::Uuid),

    #[error("offer not found for application {0}")]
    OfferNotFound(uuid::Uuid),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

Verification: Each variant compiles and produces a reasonable `.to_string()` output.

**Step 1.5 — Write `StageTransitionEvent` in `events.rs`**

```rust
// lazyjob-core/src/application/events.rs

use uuid::Uuid;
use super::{model::StageTransition, stage::ApplicationStage};

/// Broadcast on tokio::sync::broadcast when a stage change is committed.
/// The TUI subscribes to redraw the kanban column for the affected application.
#[derive(Debug, Clone)]
pub struct StageTransitionEvent {
    pub application_id: Uuid,
    pub from_stage: ApplicationStage,
    pub to_stage: ApplicationStage,
    pub transition: StageTransition,
}
```

Verification: The type is `Clone + Send + Sync` (required for `tokio::sync::broadcast::Sender<StageTransitionEvent>`).

---

### Phase 2 — SQLite Migration

**Step 2.1 — Write `002_applications.sql`**

Place the full DDL from the SQLite Schema section above into `lazyjob-core/migrations/002_applications.sql`. Do not combine with the jobs migration — each migration is a discrete file applied in numeric order by `sqlx::migrate!`.

**Step 2.2 — Verify migration applies cleanly**

```bash
cargo test -p lazyjob-core -- --include-ignored migration_002
```

Use `sqlx::migrate!("migrations")` in a `#[tokio::test]` to apply all migrations to an in-memory SQLite and confirm `sqlite_master` contains the expected tables.

```rust
#[sqlx::test(migrations = "migrations")]
async fn migration_002_creates_tables(pool: sqlx::SqlitePool) {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert!(names.contains(&"applications"));
    assert!(names.contains(&"application_transitions"));
    assert!(names.contains(&"application_contacts"));
    assert!(names.contains(&"interviews"));
    assert!(names.contains(&"offers"));
}
```

---

### Phase 3 — `SqliteApplicationRepository` Implementation

**Step 3.1 — Struct definition in `sqlite.rs`**

```rust
// lazyjob-core/src/application/sqlite.rs

use sqlx::SqlitePool;
use std::sync::Arc;

pub struct SqliteApplicationRepository {
    pool: Arc<SqlitePool>,
    /// Broadcast channel for stage transition events; the TUI subscribes to this.
    event_tx: tokio::sync::broadcast::Sender<super::events::StageTransitionEvent>,
}

impl SqliteApplicationRepository {
    pub fn new(
        pool: Arc<SqlitePool>,
        event_tx: tokio::sync::broadcast::Sender<super::events::StageTransitionEvent>,
    ) -> Self {
        Self { pool, event_tx }
    }
}
```

**Step 3.2 — `insert` method**

Uses `sqlx::query!` macro for compile-time query verification:

```rust
async fn insert(&self, app: &Application) -> Result<Uuid> {
    let id = app.id.to_string();
    let job_id = app.job_id.to_string();
    let stage = app.stage.to_string();
    sqlx::query!(
        "INSERT INTO applications
         (id, job_id, stage, notes, created_at, updated_at)
         VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
        id, job_id, stage, app.notes
    )
    .execute(self.pool.as_ref())
    .await?;
    Ok(app.id)
}
```

**Step 3.3 — `update_stage` method (core enforcement point)**

This method must:
1. Begin a SQLite transaction.
2. Fetch current stage to validate the transition.
3. Call `current_stage.can_transition_to(new_stage)` — return `InvalidTransition` on false.
4. Update `applications.stage` and `applications.updated_at`.
5. Insert a row into `application_transitions`.
6. Commit the transaction.
7. Broadcast `StageTransitionEvent` on success.

```rust
async fn update_stage(
    &self,
    id: Uuid,
    new_stage: ApplicationStage,
    reason: Option<&str>,
) -> Result<StageTransition> {
    let mut tx = self.pool.begin().await?;
    let id_str = id.to_string();

    // Fetch current stage within transaction to avoid TOCTOU race.
    let row = sqlx::query!("SELECT stage FROM applications WHERE id = ?", id_str)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApplicationError::NotFound(id))?;

    let current_stage: ApplicationStage = row.stage.parse()?;
    if !current_stage.can_transition_to(new_stage) {
        return Err(ApplicationError::InvalidTransition {
            from: current_stage,
            to: new_stage,
        });
    }

    let new_stage_str = new_stage.to_string();
    sqlx::query!(
        "UPDATE applications SET stage = ?, updated_at = datetime('now') WHERE id = ?",
        new_stage_str, id_str
    )
    .execute(&mut *tx)
    .await?;

    let transition_id = Uuid::new_v4().to_string();
    let from_str = current_stage.to_string();
    sqlx::query!(
        "INSERT INTO application_transitions
         (id, application_id, from_stage, to_stage, reason)
         VALUES (?, ?, ?, ?, ?)",
        transition_id, id_str, from_str, new_stage_str, reason
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let transition = StageTransition {
        id: Uuid::parse_str(&transition_id).unwrap(),
        application_id: id,
        from_stage: current_stage,
        to_stage: new_stage,
        reason: reason.map(ToOwned::to_owned),
        transitioned_at: chrono::Utc::now(),
    };

    // Best-effort broadcast; TUI may not be subscribed yet.
    let _ = self.event_tx.send(StageTransitionEvent {
        application_id: id,
        from_stage: current_stage,
        to_stage: new_stage,
        transition: transition.clone(),
    });

    Ok(transition)
}
```

**Step 3.4 — `count_by_stage` method**

```rust
async fn count_by_stage(&self) -> Result<HashMap<ApplicationStage, usize>> {
    let rows = sqlx::query!(
        "SELECT stage, COUNT(*) as cnt FROM applications GROUP BY stage"
    )
    .fetch_all(self.pool.as_ref())
    .await?;

    let mut map = HashMap::new();
    for row in rows {
        let stage: ApplicationStage = row.stage.parse()?;
        map.insert(stage, row.cnt as usize);
    }
    Ok(map)
}
```

**Step 3.5 — `list` with dynamic filter**

Since `sqlx::query!` requires a static query string, the list query uses `sqlx::QueryBuilder` for dynamic WHERE clause assembly:

```rust
async fn list(&self, filter: &ApplicationFilter) -> Result<Vec<Application>> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT id, job_id, stage, resume_version_id, cover_letter_version_id,
                notes, last_contact_at, next_follow_up_at, created_at, updated_at
         FROM applications WHERE 1=1"
    );

    if filter.active_only {
        qb.push(" AND stage NOT IN ('Accepted', 'Rejected', 'Withdrawn')");
    }
    if let Some(stages) = &filter.stages {
        qb.push(" AND stage IN (");
        let mut sep = qb.separated(", ");
        for s in stages {
            sep.push_bind(s.to_string());
        }
        qb.push(")");
    }
    if let Some(job_id) = filter.job_id {
        qb.push(" AND job_id = ").push_bind(job_id.to_string());
    }
    if let Some(since) = filter.since {
        qb.push(" AND updated_at >= ").push_bind(since.to_rfc3339());
    }
    let limit = if filter.limit == 0 { 200u32 } else { filter.limit };
    qb.push(" ORDER BY updated_at DESC LIMIT ").push_bind(limit as i64);
    qb.push(" OFFSET ").push_bind(filter.offset as i64);

    let rows = qb.build_query_as::<SqliteApplicationRow>()
        .fetch_all(self.pool.as_ref())
        .await?;

    rows.into_iter().map(Application::try_from).collect()
}
```

A `SqliteApplicationRow` private struct derives `sqlx::FromRow` for column mapping; a `TryFrom<SqliteApplicationRow>` implementation handles UUID parsing and datetime parsing.

**Step 3.6 — `list_overdue_follow_ups`**

```rust
async fn list_overdue_follow_ups(&self, limit: u32) -> Result<Vec<Application>> {
    let rows = sqlx::query_as!(
        SqliteApplicationRow,
        "SELECT * FROM applications
         WHERE next_follow_up_at < datetime('now')
           AND stage NOT IN ('Accepted', 'Rejected', 'Withdrawn')
         ORDER BY next_follow_up_at ASC
         LIMIT ?",
        limit as i64
    )
    .fetch_all(self.pool.as_ref())
    .await?;
    rows.into_iter().map(Application::try_from).collect()
}
```

**Step 3.7 — Interview and Offer methods**

`insert_interview`: Serialize `interviewers: Vec<String>` to JSON with `serde_json::to_string(&interview.interviewers)?` before binding. `list_interviews`: Deserialize from TEXT on read.

`insert_offer`: Store monetary fields as INTEGER cents, equity as REAL percent. `get_offer`: `SELECT ... WHERE application_id = ? ORDER BY created_at DESC LIMIT 1`.

**Step 3.8 — Expose via `AppState` in `lazyjob-cli`**

```rust
// lazyjob-cli/src/state.rs
pub struct AppState {
    pub application_repo: Arc<dyn ApplicationRepository>,
    pub stage_event_rx: tokio::sync::broadcast::Receiver<StageTransitionEvent>,
    // ... other repos
}
```

The broadcast channel is created at startup with capacity 64:
```rust
let (tx, rx) = tokio::sync::broadcast::channel::<StageTransitionEvent>(64);
let application_repo = Arc::new(SqliteApplicationRepository::new(pool.clone(), tx));
```

---

### Phase 4 — TUI Kanban Integration

**Step 4.1 — Stage transition in TUI key handler**

The TUI does not call `update_stage` directly. Instead it dispatches a `TuiAction::MoveApplicationStage { app_id, new_stage, reason }` which is handled by the workflow layer (see `application-workflow-actions.md`). The workflow calls the repository and the broadcast channel delivers the result back to the TUI.

**Step 4.2 — Kanban column rendering**

```rust
// lazyjob-tui/src/views/kanban.rs

pub struct KanbanView {
    /// Latest snapshot of count_by_stage output; refreshed on StageTransitionEvent.
    stage_counts: HashMap<ApplicationStage, usize>,
    /// Applications currently visible per stage column.
    columns: Vec<Vec<Application>>,
    focused_column: usize,
    focused_row: usize,
}
```

Column layout: `ApplicationStage::display_order()` drives the column order. Terminal stages (Accepted/Rejected/Withdrawn) are collapsed into a single "Done" column to save horizontal space. Column width = `total_width / 8` (7 active stages + 1 done column).

**Step 4.3 — Stage transition history overlay**

When the user presses `H` on an application, a popup renders the transition log:

```
┌─ Stage History ──────────────────────────────────────┐
│  2026-04-10 09:00  Discovered   → Applied            │
│  2026-04-11 14:30  Applied      → PhoneScreen        │
│  2026-04-13 11:00  PhoneScreen  → Technical          │
│  [reason: Moving to technical round, email confirmed] │
└──────────────────────────────────────────────────────┘
```

The overlay uses `ratatui::widgets::Clear` to blank the background area, then renders a `Table` widget with rows from `list_transitions()`. Scroll position state is maintained in `KanbanView`.

**Step 4.4 — Action Required queue**

A side panel renders applications needing immediate attention, populated by:
- `list_overdue_follow_ups(10)` — overdue reminders
- `list_expiring_offers(3)` — offers expiring within 3 days

Items are sorted by urgency (expiring offers first, then overdue follow-ups by days overdue). Each item shows company name, role title, and the type of action needed.

---

### Phase 5 — Reminder Integration

**Step 5.1 — `ReminderPoller` background task**

```rust
// lazyjob-core/src/application/reminder.rs

pub struct ReminderPoller {
    repo: Arc<dyn ApplicationRepository>,
    event_tx: tokio::sync::broadcast::Sender<ReminderDueEvent>,
    tick_interval: std::time::Duration,
}

#[derive(Debug, Clone)]
pub struct ReminderDueEvent {
    pub application_id: Uuid,
    pub kind: ReminderKind,
    pub overdue_by: chrono::Duration,
}

#[derive(Debug, Clone)]
pub enum ReminderKind {
    FollowUp,
    OfferExpiry { days_remaining: i64 },
}

impl ReminderPoller {
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.tick_interval);
            loop {
                interval.tick().await;
                if let Err(e) = self.check_reminders().await {
                    tracing::error!(error = %e, "reminder poller error");
                }
            }
        })
    }

    async fn check_reminders(&self) -> anyhow::Result<()> {
        let overdue = self.repo.list_overdue_follow_ups(50).await?;
        for app in overdue {
            let overdue_by = chrono::Utc::now()
                - app.next_follow_up_at.unwrap_or(chrono::Utc::now());
            let _ = self.event_tx.send(ReminderDueEvent {
                application_id: app.id,
                kind: ReminderKind::FollowUp,
                overdue_by,
            });
        }
        let expiring = self.repo.list_expiring_offers(3).await?;
        for (app, offer) in expiring {
            let days_remaining = offer
                .expiry_date
                .map(|d| (d - chrono::Utc::now().date_naive()).num_days())
                .unwrap_or(0);
            let _ = self.event_tx.send(ReminderDueEvent {
                application_id: app.id,
                kind: ReminderKind::OfferExpiry { days_remaining },
                overdue_by: chrono::Duration::zero(),
            });
        }
        Ok(())
    }
}
```

Default tick interval: 5 minutes in production, 100ms in tests (configurable via constructor).

---

## Key Crate APIs

```
sqlx::SqlitePool::connect("sqlite://lazyjob.db?mode=rwc")
  → for pool construction in lazyjob-cli/src/database.rs

sqlx::migrate!("migrations").run(&pool).await
  → applies 002_applications.sql in order

sqlx::query!("INSERT INTO applications ...", ...).execute(&pool)
  → compile-time verified DDL/DML

sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT ... WHERE 1=1")
  → dynamic WHERE clause assembly for ApplicationFilter

sqlx::query_as!(SqliteApplicationRow, "SELECT * FROM applications WHERE ...")
  → typed row mapping for list queries

tokio::sync::broadcast::channel::<StageTransitionEvent>(64)
  → event bus for TUI re-render notifications

uuid::Uuid::new_v4().to_string()
  → generates new IDs before insert (not relying on SQLite DEFAULT)

chrono::Utc::now().to_rfc3339()
  → timestamp serialization to ISO 8601 for SQLite TEXT columns

serde_json::to_string(&interview.interviewers)?
  → serialize Vec<String> to JSON TEXT for SQLite storage

ratatui::widgets::Clear
  → erase background before rendering stage history overlay popup

ratatui::widgets::Table::new(rows, widths)
  → render stage transition history in popup

ratatui::layout::Layout::default().constraints([...]).split(area)
  → split kanban area into equal-width columns
```

---

## Error Handling

```rust
// lazyjob-core/src/application/error.rs

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error("invalid stage string: '{0}'")]
    InvalidStage(String),

    #[error("transition from '{from}' to '{to}' is not permitted by the transition matrix")]
    InvalidTransition {
        from: crate::application::ApplicationStage,
        to: crate::application::ApplicationStage,
    },

    #[error("application not found: {0}")]
    NotFound(Uuid),

    #[error("application {0} is in terminal stage and cannot transition")]
    TerminalStage(Uuid),

    #[error("interview not found: {0}")]
    InterviewNotFound(Uuid),

    #[error("no offer found for application: {0}")]
    OfferNotFound(Uuid),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("UUID parse error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("datetime parse error: {0}")]
    DatetimeParse(String),
}
```

Callers (workflow layer, TUI) match on `InvalidTransition` to show a user-facing error message ("That transition is not allowed"). `Database` errors are propagated to the top-level error handler as internal failures.

---

## Testing Strategy

### Unit Tests — `ApplicationStage`

All tests in `lazyjob-core/src/application/stage.rs` under `#[cfg(test)]`:

```rust
#[test]
fn terminal_stages_cannot_transition() {
    use ApplicationStage::*;
    for terminal in [Accepted, Rejected, Withdrawn] {
        for any in ApplicationStage::display_order() {
            assert!(!terminal.can_transition_to(*any),
                "{terminal} is terminal but can_transition_to {any} returned true");
        }
    }
}

#[test]
fn forward_progression_allowed() {
    use ApplicationStage::*;
    let pairs = [
        (Discovered, Interested), (Interested, Applied),
        (Applied, PhoneScreen), (PhoneScreen, Technical),
        (Technical, OnSite), (OnSite, Offer), (Offer, Accepted),
    ];
    for (from, to) in pairs {
        assert!(from.can_transition_to(to), "{from} → {to} should be allowed");
    }
}

#[test]
fn backward_correction_allowed() {
    use ApplicationStage::*;
    let pairs = [
        (Applied, Interested), (PhoneScreen, Applied),
        (Technical, PhoneScreen), (OnSite, Technical), (Offer, OnSite),
    ];
    for (from, to) in pairs {
        assert!(from.can_transition_to(to), "{from} → {to} (backward) should be allowed");
    }
}

#[test]
fn any_to_rejected_allowed() {
    use ApplicationStage::*;
    for active in [Discovered, Interested, Applied, PhoneScreen, Technical, OnSite, Offer] {
        assert!(active.can_transition_to(Rejected), "{active} → Rejected should be allowed");
    }
}

#[test]
fn stage_roundtrip_serde() {
    use ApplicationStage::*;
    for stage in ApplicationStage::display_order() {
        let s = stage.to_string();
        let parsed: ApplicationStage = s.parse().unwrap();
        assert_eq!(*stage, parsed);
        let json = serde_json::to_string(stage).unwrap();
        let from_json: ApplicationStage = serde_json::from_str(&json).unwrap();
        assert_eq!(*stage, from_json);
    }
}
```

### Integration Tests — `SqliteApplicationRepository`

Use `#[sqlx::test(migrations = "migrations")]` which provides an in-memory `SqlitePool` with all migrations applied:

```rust
#[sqlx::test(migrations = "migrations")]
async fn update_stage_enforces_matrix(pool: SqlitePool) {
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let repo = SqliteApplicationRepository::new(Arc::new(pool), tx);

    // Insert a job first (FK constraint)
    let job_id = insert_test_job(&repo).await;
    let app_id = repo.insert(&make_application(job_id, ApplicationStage::Discovered)).await.unwrap();

    // Valid: Discovered → Applied (skip Interested)
    repo.update_stage(app_id, ApplicationStage::Applied, None).await.unwrap();

    // Invalid: Applied → Discovered (no such backward transition)
    let err = repo.update_stage(app_id, ApplicationStage::Discovered, None).await.unwrap_err();
    assert!(matches!(err, ApplicationError::InvalidTransition { .. }));
}

#[sqlx::test(migrations = "migrations")]
async fn transition_log_is_append_only(pool: SqlitePool) {
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let repo = SqliteApplicationRepository::new(Arc::new(pool), tx);
    let job_id = insert_test_job(&repo).await;
    let app_id = repo.insert(&make_application(job_id, ApplicationStage::Discovered)).await.unwrap();

    repo.update_stage(app_id, ApplicationStage::Applied, Some("applied online")).await.unwrap();
    repo.update_stage(app_id, ApplicationStage::PhoneScreen, None).await.unwrap();

    let transitions = repo.list_transitions(app_id).await.unwrap();
    assert_eq!(transitions.len(), 2);
    assert_eq!(transitions[0].from_stage, ApplicationStage::Discovered);
    assert_eq!(transitions[0].to_stage, ApplicationStage::Applied);
    assert_eq!(transitions[0].reason.as_deref(), Some("applied online"));
    assert_eq!(transitions[1].from_stage, ApplicationStage::Applied);
    assert_eq!(transitions[1].to_stage, ApplicationStage::PhoneScreen);
}

#[sqlx::test(migrations = "migrations")]
async fn count_by_stage_returns_correct_counts(pool: SqlitePool) {
    // Insert multiple applications in different stages...
    // Then assert count_by_stage returns the correct HashMap.
}

#[sqlx::test(migrations = "migrations")]
async fn terminal_stage_blocks_further_transitions(pool: SqlitePool) {
    // Move to Rejected, then try Rejected → Applied — must return TerminalStage error.
}
```

### TUI Tests — Kanban Widget

Ratatui provides `ratatui::backend::TestBackend` for deterministic rendering without a real terminal:

```rust
#[test]
fn kanban_renders_stage_counts() {
    let backend = ratatui::backend::TestBackend::new(200, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut kanban = KanbanView::new_with_counts(
        HashMap::from([
            (ApplicationStage::Applied, 3),
            (ApplicationStage::PhoneScreen, 1),
        ])
    );
    terminal.draw(|frame| kanban.render(frame, frame.area())).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Assert "Applied (3)" appears somewhere in the rendered buffer.
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Applied"));
}
```

### Mock Double

For workflow-layer tests that need an `ApplicationRepository` without SQLite:

```rust
// lazyjob-core/src/application/mock.rs (test-only, behind #[cfg(test)])

pub struct MockApplicationRepository {
    pub applications: std::sync::Mutex<HashMap<Uuid, Application>>,
    pub transitions: std::sync::Mutex<Vec<StageTransition>>,
}
```

Implements `ApplicationRepository` using in-memory `HashMap` operations.

---

## Open Questions

1. **Async challenge sub-status**: The spec notes that some `Technical` interviews are async coding challenges (HackerRank, Karat) with a deadline. The current `InterviewType::Async` variant handles the type, but there is no `deadline` field on `Interview`. A `challenge_deadline` column on `interviews` for `Async` type interviews could surface in the Action Required queue. Deferred until an actual use case arises.

2. **Multi-role applications sharing contacts**: A user applying to two roles at the same company gets two `Application` records with separate `application_contacts` entries. If the same recruiter appears in both, they are stored as two rows. There is no deduplication or cross-application contact linkage in Phase 1. The networking module (`profile_contacts`) is the right home for de-duplicated contact data if this becomes a pain point.

3. **SQLite concurrency**: `SqlitePool` in WAL mode allows concurrent readers with a single writer. The `update_stage` transaction holds a write lock for < 1ms on typical hardware. For Phase 1 (single user, local), this is fine. If multi-process access is needed (e.g., a Ralph subprocess and the TUI simultaneously), investigate `PRAGMA journal_mode=WAL` and `PRAGMA busy_timeout=5000`.

4. **Offer expiry notification ownership**: `list_expiring_offers` is defined on `ApplicationRepository` but the notification delivery (desktop toast via `notify-rust`) is outside this spec's scope. The `ReminderPoller` emits `ReminderDueEvent` and a separate notification subscriber in the TUI crate handles display.

5. **Deleted applications vs. archived**: The current spec has `delete()` which hard-deletes. A soft-delete `deleted_at` column may be preferable for audit/history purposes. Punted to the metrics spec (`application-pipeline-metrics.md`) which will need historical data.

---

## Related Specs

- [`specs/application-workflow-actions.md`](./application-workflow-actions.md) — workflow orchestrators that call this repository
- [`specs/10-application-workflow.md`](./10-application-workflow.md) — higher-level workflow spec (overlaps with this; this spec is the canonical state machine authority)
- [`specs/application-pipeline-metrics.md`](./application-pipeline-metrics.md) — metric aggregation queries over `applications` and `application_transitions`
- [`specs/09-tui-design-keybindings-implementation-plan.md`](./09-tui-design-keybindings-implementation-plan.md) — TUI `EventLoop` and widget rendering primitives
- [`specs/04-sqlite-persistence-implementation-plan.md`](./04-sqlite-persistence-implementation-plan.md) — SQLite pool, migration runner
- [`specs/12-15-interview-salary-networking-notifications-implementation-plan.md`](./12-15-interview-salary-networking-notifications-implementation-plan.md) — reminder/notification delivery layer
