# Implementation Plan: Application Workflow Actions

## Status
Draft

## Related Spec
[`specs/application-workflow-actions.md`](./application-workflow-actions.md)

## Overview

The workflow actions layer is the orchestration heart of LazyJob's application tracking. It wraps the raw `ApplicationRepository` CRUD operations in domain-meaningful workflows — `ApplyWorkflow`, `MoveStageWorkflow`, `ScheduleInterviewWorkflow`, and `LogContactWorkflow` — each of which enforces side effects, emits typed events, and respects the human-in-the-loop boundaries that define LazyJob's product positioning.

Rather than requiring the TUI to manually sequence database writes, reminder creation, and event emission, each workflow bundles these into a single `execute()` call. The TUI provides user intent (which job to apply to, which stage to advance to) via options structs; the workflow handles atomicity, validation, and feedback. This keeps the TUI layer thin and testable: any workflow can be driven by a unit test without a running terminal.

The `ReminderService` is a companion to the workflow layer — it provides reminder creation and cancellation primitives that workflows delegate to. A background `ReminderPoller` tokio task queries overdue reminders and fires `ReminderDueEvent` values that the TUI and notification layer consume. All events flow over `tokio::sync::broadcast` channels, so the TUI receives real-time updates without polling SQLite.

## Prerequisites

### Specs / Plans That Must Be Implemented First
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `Application`, `ApplicationRepository` trait, and SQLite migrations
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations infrastructure
- `specs/01-architecture-implementation-plan.md` — workspace `Cargo.toml`, crate skeleton

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
sqlx       = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
tokio      = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }
uuid       = { version = "1", features = ["v4", "serde"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
chrono     = { version = "0.4", features = ["serde"] }
thiserror  = "2"
anyhow     = "1"
tracing    = "0.1"
async-trait = "0.1"

[dev-dependencies]
tokio      = { version = "1", features = ["full"] }
sqlx       = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }
```

---

## Architecture

### Crate Placement

| Component | Crate | Reason |
|---|---|---|
| `WorkflowEvent` enum | `lazyjob-core` | Consumed by TUI, Ralph, notification layer |
| `ApplyWorkflow` | `lazyjob-core` | Pure orchestration over repository traits |
| `MoveStageWorkflow` | `lazyjob-core` | Transition validation + side effects |
| `ScheduleInterviewWorkflow` | `lazyjob-core` | Interview creation + reminder scheduling |
| `LogContactWorkflow` | `lazyjob-core` | Lightweight contact log operation |
| `ReminderService` | `lazyjob-core` | Reminder creation/cancellation abstraction |
| `ReminderRepository` async trait | `lazyjob-core` | I/O boundary for reminder persistence |
| `SqliteReminderRepository` | `lazyjob-core` | Concrete SQLite implementation |
| `ReminderPoller` | `lazyjob-core` | Background tokio task for overdue reminder delivery |
| TUI apply confirmation dialog | `lazyjob-tui` | Rendering concern, not domain logic |
| TUI stage transition dialog | `lazyjob-tui` | Rendering concern, not domain logic |

### Core Types

```rust
// lazyjob-core/src/application/events.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::application::stage::ApplicationStage;

#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    ApplicationCreated {
        application_id: Uuid,
        job_id: Uuid,
    },
    StageChanged {
        application_id: Uuid,
        from: ApplicationStage,
        to: ApplicationStage,
    },
    InterviewScheduled {
        interview_id: Uuid,
        application_id: Uuid,
        at: DateTime<Utc>,
    },
    ContactLogged {
        application_id: Uuid,
    },
    ReminderCreated {
        reminder_id: Uuid,
        application_id: Uuid,
        due_at: DateTime<Utc>,
    },
    WorkflowError {
        workflow: &'static str,
        error: String,
    },
}

/// Emitted by ReminderPoller when a reminder's due_at has passed.
#[derive(Debug, Clone)]
pub struct ReminderDueEvent {
    pub reminder_id: Uuid,
    pub application_id: Option<Uuid>,
    pub title: String,
    pub body: Option<String>,
    pub due_at: DateTime<Utc>,
}
```

```rust
// lazyjob-core/src/application/reminder.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Reminder {
    pub id: Uuid,
    pub application_id: Option<Uuid>,
    pub title: String,
    pub body: Option<String>,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Reminder {
    pub fn new(
        application_id: Option<Uuid>,
        title: impl Into<String>,
        body: Option<String>,
        due_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            application_id,
            title: title.into(),
            body,
            due_at,
            fired_at: None,
            created_at: Utc::now(),
        }
    }
}
```

```rust
// lazyjob-core/src/application/workflows.rs

use std::sync::Arc;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::application::{
    events::WorkflowEvent,
    stage::ApplicationStage,
    reminder::ReminderService,
};
use crate::job::repository::JobRepository;
use crate::application::repository::{ApplicationRepository, InterviewRepository};
use crate::resume::repository::ResumeVersionRepository;
use crate::cover_letter::repository::CoverLetterVersionRepository;

// ─── ApplyWorkflow ────────────────────────────────────────────────────────────

pub struct ApplyWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub resume_repo: Arc<dyn ResumeVersionRepository>,
    pub cover_letter_repo: Arc<dyn CoverLetterVersionRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

pub struct ApplyOptions {
    pub resume_version_id: Option<Uuid>,
    pub cover_letter_version_id: Option<Uuid>,
    pub screening_answers: Option<serde_json::Value>,
    /// Days before follow-up reminder fires (default: 7)
    pub follow_up_days: u32,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            resume_version_id: None,
            cover_letter_version_id: None,
            screening_answers: None,
            follow_up_days: 7,
        }
    }
}

impl ApplyWorkflow {
    pub async fn check_duplicate(
        &self,
        job_id: &Uuid,
    ) -> Result<Option<Application>, WorkflowError> {
        self.app_repo
            .find_by_job_id(job_id)
            .await
            .map_err(WorkflowError::Repository)
    }

    pub async fn execute(
        &self,
        job_id: &Uuid,
        opts: ApplyOptions,
    ) -> Result<Application, WorkflowError> {
        // Step 1: Duplicate guard
        if let Some(existing) = self.check_duplicate(job_id).await? {
            return Err(WorkflowError::DuplicateApplication {
                existing_id: existing.id,
            });
        }

        // Step 2: Resolve resume version (caller must have resolved the TUI
        // dialog and pass the chosen ID here; None is valid — user opted out)
        let resume_version_id = opts.resume_version_id;
        let cover_letter_version_id = opts.cover_letter_version_id;

        // Step 3: Create Application record at Applied stage
        let app = Application {
            id: Uuid::new_v4(),
            job_id: *job_id,
            stage: ApplicationStage::Applied,
            resume_version_id,
            cover_letter_version_id,
            screening_answers: opts.screening_answers,
            notes: None,
            last_contact_at: Some(Utc::now()),
            next_follow_up_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.app_repo.create(&app).await.map_err(WorkflowError::Repository)?;

        // Step 4: Mark the Job as Applied
        self.job_repo
            .update_status(job_id, JobStatus::Applied)
            .await
            .map_err(WorkflowError::Repository)?;

        // Step 5: Schedule follow-up reminder
        let follow_up_due = Utc::now() + Duration::days(opts.follow_up_days as i64);
        let reminder = Reminder::new(
            Some(app.id),
            format!("Follow up on application"),
            Some(format!("No response after {} days — consider following up", opts.follow_up_days)),
            follow_up_due,
        );
        let reminder_id = self.reminder_service.create(&reminder).await.map_err(WorkflowError::ReminderFailed)?;

        // Step 6: Emit events
        let _ = self.events.send(WorkflowEvent::ApplicationCreated {
            application_id: app.id,
            job_id: *job_id,
        });
        let _ = self.events.send(WorkflowEvent::ReminderCreated {
            reminder_id,
            application_id: app.id,
            due_at: follow_up_due,
        });

        Ok(app)
    }
}

// ─── MoveStageWorkflow ────────────────────────────────────────────────────────

pub struct MoveStageWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

pub struct MoveStageInput {
    pub application_id: Uuid,
    pub target: ApplicationStage,
    /// Optional reason (required for Rejected, Withdrawn)
    pub reason: Option<String>,
    /// Compensation details when target is Offer
    pub offer_details: Option<OfferDetails>,
}

pub struct MoveStageResult {
    pub application: Application,
    pub suggestions: Vec<PostTransitionSuggestion>,
}

pub enum PostTransitionSuggestion {
    GenerateInterviewPrep { application_id: Uuid },
    GenerateTechnicalPrep { application_id: Uuid },
    GenerateCompanyCheatSheet { application_id: Uuid },
    RunSalaryComparison { application_id: Uuid },
    ArchiveSiblingApplications { application_id: Uuid },
}

impl MoveStageWorkflow {
    pub async fn execute(
        &self,
        input: MoveStageInput,
    ) -> Result<MoveStageResult, WorkflowError> {
        let app = self
            .app_repo
            .find_by_id(&input.application_id)
            .await
            .map_err(WorkflowError::Repository)?
            .ok_or(WorkflowError::NotFound(input.application_id))?;

        let from = app.stage;
        let to = input.target;

        // Step 1: Validate transition
        if !from.can_transition_to(to) {
            return Err(WorkflowError::InvalidTransition { from, to });
        }

        // Step 2: Pre-transition side effects
        match to {
            ApplicationStage::Rejected | ApplicationStage::Withdrawn => {
                // Cancel all pending reminders for this application
                self.reminder_service
                    .cancel_for_application(&input.application_id)
                    .await
                    .map_err(WorkflowError::ReminderFailed)?;
            }
            ApplicationStage::Offer => {
                // offer_details handling is the caller's responsibility
                // (TUI dialog collected them, passed in input)
            }
            _ => {}
        }

        // Step 3: Execute transition in the repository
        let updated = self
            .app_repo
            .update_stage(
                &input.application_id,
                to,
                input.reason.as_deref(),
            )
            .await
            .map_err(WorkflowError::Repository)?;

        // Step 4: Post-transition side effects
        if to.is_active() && from < to {
            // Moving forward: update last_contact_at
            self.app_repo
                .touch_last_contact(&input.application_id, Utc::now())
                .await
                .map_err(WorkflowError::Repository)?;
        }

        // Schedule follow-up for Applied (in case of re-apply from Interested)
        if to == ApplicationStage::Applied {
            let due = Utc::now() + Duration::days(7);
            let reminder = Reminder::new(
                Some(input.application_id),
                "Follow up on application".to_string(),
                None,
                due,
            );
            let rid = self.reminder_service.create(&reminder).await.map_err(WorkflowError::ReminderFailed)?;
            let _ = self.events.send(WorkflowEvent::ReminderCreated {
                reminder_id: rid,
                application_id: input.application_id,
                due_at: due,
            });
        }

        // Collect post-transition suggestions (non-blocking, user may ignore)
        let mut suggestions = Vec::new();
        match to {
            ApplicationStage::PhoneScreen => {
                suggestions.push(PostTransitionSuggestion::GenerateInterviewPrep {
                    application_id: input.application_id,
                });
            }
            ApplicationStage::Technical => {
                suggestions.push(PostTransitionSuggestion::GenerateTechnicalPrep {
                    application_id: input.application_id,
                });
            }
            ApplicationStage::OnSite => {
                suggestions.push(PostTransitionSuggestion::GenerateCompanyCheatSheet {
                    application_id: input.application_id,
                });
            }
            ApplicationStage::Offer => {
                suggestions.push(PostTransitionSuggestion::RunSalaryComparison {
                    application_id: input.application_id,
                });
            }
            ApplicationStage::Accepted => {
                suggestions.push(PostTransitionSuggestion::ArchiveSiblingApplications {
                    application_id: input.application_id,
                });
            }
            _ => {}
        }

        // Step 5: Emit event
        let _ = self.events.send(WorkflowEvent::StageChanged {
            application_id: input.application_id,
            from,
            to,
        });

        Ok(MoveStageResult {
            application: updated,
            suggestions,
        })
    }
}

// ─── ScheduleInterviewWorkflow ────────────────────────────────────────────────

pub struct ScheduleInterviewWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub interview_repo: Arc<dyn InterviewRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

pub struct InterviewDetails {
    pub scheduled_at: DateTime<Utc>,
    pub duration_minutes: u32,
    pub location: Option<String>,
    pub meeting_url: Option<String>,
    pub interviewers: Vec<String>,
    pub interview_type: Option<InterviewType>,
    /// Hours before interview to fire reminder (default: 24)
    pub reminder_hours_before: u32,
    /// Days before interview for prep reminder (default: 2)
    pub prep_reminder_days_before: Option<u32>,
}

impl Default for InterviewDetails {
    fn default() -> Self {
        Self {
            scheduled_at: Utc::now(),
            duration_minutes: 60,
            location: None,
            meeting_url: None,
            interviewers: Vec::new(),
            interview_type: None,
            reminder_hours_before: 24,
            prep_reminder_days_before: Some(2),
        }
    }
}

impl ScheduleInterviewWorkflow {
    pub async fn execute(
        &self,
        application_id: &Uuid,
        details: InterviewDetails,
    ) -> Result<Interview, WorkflowError> {
        // Step 1: Validate application is in an active interview stage
        let app = self
            .app_repo
            .find_by_id(application_id)
            .await
            .map_err(WorkflowError::Repository)?
            .ok_or(WorkflowError::NotFound(*application_id))?;

        let valid_stages = [
            ApplicationStage::PhoneScreen,
            ApplicationStage::Technical,
            ApplicationStage::OnSite,
        ];
        if !valid_stages.contains(&app.stage) {
            return Err(WorkflowError::InvalidStageForAction {
                current: app.stage,
                action: "schedule_interview",
            });
        }

        // Step 2: Infer interview type from stage if not provided
        let interview_type = details.interview_type.unwrap_or_else(|| {
            match app.stage {
                ApplicationStage::PhoneScreen => InterviewType::PhoneScreen,
                ApplicationStage::Technical => InterviewType::Technical,
                ApplicationStage::OnSite => InterviewType::OnSite,
                _ => InterviewType::Other,
            }
        });

        // Step 3: Create Interview record
        let interview = Interview {
            id: Uuid::new_v4(),
            application_id: *application_id,
            interview_type,
            scheduled_at: details.scheduled_at,
            duration_minutes: details.duration_minutes,
            location: details.location,
            meeting_url: details.meeting_url,
            interviewers: details.interviewers,
            notes: None,
            created_at: Utc::now(),
        };
        self.interview_repo.create(&interview).await.map_err(WorkflowError::Repository)?;

        // Step 4: Update last_contact_at
        self.app_repo
            .touch_last_contact(application_id, Utc::now())
            .await
            .map_err(WorkflowError::Repository)?;

        // Step 5: Create reminder N hours before interview
        let reminder_due =
            details.scheduled_at - Duration::hours(details.reminder_hours_before as i64);
        if reminder_due > Utc::now() {
            let r = Reminder::new(
                Some(*application_id),
                format!("Interview in {} hours", details.reminder_hours_before),
                Some(format!("Interview scheduled at {}", details.scheduled_at)),
                reminder_due,
            );
            let rid = self.reminder_service.create(&r).await.map_err(WorkflowError::ReminderFailed)?;
            let _ = self.events.send(WorkflowEvent::ReminderCreated {
                reminder_id: rid,
                application_id: *application_id,
                due_at: reminder_due,
            });
        }

        // Step 6: Create prep reminder N days before interview (optional)
        if let Some(prep_days) = details.prep_reminder_days_before {
            let prep_due = details.scheduled_at - Duration::days(prep_days as i64);
            if prep_due > Utc::now() {
                let rp = Reminder::new(
                    Some(*application_id),
                    "Interview prep reminder".to_string(),
                    Some(format!("Interview in {} days — review prep notes", prep_days)),
                    prep_due,
                );
                let rpid = self.reminder_service.create(&rp).await.map_err(WorkflowError::ReminderFailed)?;
                let _ = self.events.send(WorkflowEvent::ReminderCreated {
                    reminder_id: rpid,
                    application_id: *application_id,
                    due_at: prep_due,
                });
            }
        }

        // Step 7: Emit event
        let _ = self.events.send(WorkflowEvent::InterviewScheduled {
            interview_id: interview.id,
            application_id: *application_id,
            at: details.scheduled_at,
        });

        Ok(interview)
    }
}

// ─── LogContactWorkflow ───────────────────────────────────────────────────────

pub struct LogContactWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

pub struct LogContactInput {
    pub application_id: Uuid,
    pub contact_type: ContactType,
    pub note: String,
    pub contact_name: Option<String>,
    /// If true, cancel next_follow_up_at (we received contact, clock resets)
    pub reset_follow_up: bool,
}

pub enum ContactType {
    EmailReceived,
    EmailSent,
    PhoneCall,
    RecruiterMessage,
    Other,
}

impl LogContactWorkflow {
    pub async fn execute(
        &self,
        input: LogContactInput,
    ) -> Result<(), WorkflowError> {
        let timestamp = Utc::now();
        let note_with_timestamp = format!(
            "[{}] {}: {}{}",
            timestamp.format("%Y-%m-%d %H:%M"),
            contact_type_label(&input.contact_type),
            input.contact_name.as_deref().map(|n| format!("({}) ", n)).unwrap_or_default(),
            input.note
        );

        self.app_repo
            .append_note(&input.application_id, &note_with_timestamp)
            .await
            .map_err(WorkflowError::Repository)?;

        self.app_repo
            .touch_last_contact(&input.application_id, timestamp)
            .await
            .map_err(WorkflowError::Repository)?;

        if input.reset_follow_up {
            self.reminder_service
                .cancel_for_application(&input.application_id)
                .await
                .map_err(WorkflowError::ReminderFailed)?;
        }

        let _ = self.events.send(WorkflowEvent::ContactLogged {
            application_id: input.application_id,
        });

        Ok(())
    }
}

fn contact_type_label(ct: &ContactType) -> &'static str {
    match ct {
        ContactType::EmailReceived   => "Email received",
        ContactType::EmailSent       => "Email sent",
        ContactType::PhoneCall       => "Phone call",
        ContactType::RecruiterMessage => "Recruiter message",
        ContactType::Other           => "Contact",
    }
}
```

```rust
// lazyjob-core/src/application/reminder.rs (service layer)

use std::sync::Arc;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::application::reminder::{Reminder, ReminderRepository};

pub struct ReminderService {
    repo: Arc<dyn ReminderRepository>,
}

impl ReminderService {
    pub fn new(repo: Arc<dyn ReminderRepository>) -> Self {
        Self { repo }
    }

    pub async fn create(&self, r: &Reminder) -> Result<Uuid, anyhow::Error> {
        self.repo.insert(r).await?;
        Ok(r.id)
    }

    pub async fn cancel_for_application(&self, app_id: &Uuid) -> Result<(), anyhow::Error> {
        self.repo.cancel_by_application(app_id).await
    }

    pub async fn list_pending(&self, before: DateTime<Utc>) -> Result<Vec<Reminder>, anyhow::Error> {
        self.repo.list_unfired_before(before).await
    }

    pub async fn mark_fired(&self, reminder_id: &Uuid) -> Result<(), anyhow::Error> {
        self.repo.set_fired(reminder_id, Utc::now()).await
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/application/repository.rs (additional methods needed)

#[async_trait::async_trait]
pub trait ApplicationRepository: Send + Sync {
    // existing methods from application-state-machine plan ...

    async fn find_by_job_id(&self, job_id: &Uuid) -> Result<Option<Application>, anyhow::Error>;
    async fn touch_last_contact(
        &self,
        app_id: &Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), anyhow::Error>;
    async fn append_note(
        &self,
        app_id: &Uuid,
        note: &str,
    ) -> Result<(), anyhow::Error>;
}

#[async_trait::async_trait]
pub trait InterviewRepository: Send + Sync {
    async fn create(&self, interview: &Interview) -> Result<(), anyhow::Error>;
    async fn find_by_application(
        &self,
        app_id: &Uuid,
    ) -> Result<Vec<Interview>, anyhow::Error>;
    async fn delete(&self, interview_id: &Uuid) -> Result<(), anyhow::Error>;
}

#[async_trait::async_trait]
pub trait ReminderRepository: Send + Sync {
    async fn insert(&self, reminder: &Reminder) -> Result<(), anyhow::Error>;
    async fn cancel_by_application(&self, app_id: &Uuid) -> Result<(), anyhow::Error>;
    async fn list_unfired_before(
        &self,
        before: DateTime<Utc>,
    ) -> Result<Vec<Reminder>, anyhow::Error>;
    async fn set_fired(
        &self,
        reminder_id: &Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), anyhow::Error>;
}
```

```rust
// lazyjob-core/src/application/poller.rs

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use crate::application::reminder::ReminderService;
use crate::application::events::ReminderDueEvent;

/// Background task: polls for overdue reminders every 5 minutes and fires events.
pub struct ReminderPoller {
    service: Arc<ReminderService>,
    tx: broadcast::Sender<ReminderDueEvent>,
    interval: Duration,
}

impl ReminderPoller {
    pub fn new(
        service: Arc<ReminderService>,
        tx: broadcast::Sender<ReminderDueEvent>,
    ) -> Self {
        Self {
            service,
            tx,
            interval: Duration::from_secs(300), // 5 minutes
        }
    }

    pub async fn run(self) {
        let mut ticker = tokio::time::interval(self.interval);
        loop {
            ticker.tick().await;
            match self.service.list_pending(chrono::Utc::now()).await {
                Ok(reminders) => {
                    for r in reminders {
                        let event = ReminderDueEvent {
                            reminder_id: r.id,
                            application_id: r.application_id,
                            title: r.title.clone(),
                            body: r.body.clone(),
                            due_at: r.due_at,
                        };
                        let _ = self.tx.send(event);
                        if let Err(e) = self.service.mark_fired(&r.id).await {
                            tracing::error!(reminder_id = %r.id, err = %e, "failed to mark reminder fired");
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e, "reminder poller query failed");
                }
            }
        }
    }
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/003_reminders.sql

CREATE TABLE reminders (
    id              TEXT    PRIMARY KEY,
    application_id  TEXT    REFERENCES applications(id) ON DELETE CASCADE,
    title           TEXT    NOT NULL,
    body            TEXT,
    due_at          TEXT    NOT NULL,   -- ISO-8601 UTC
    fired_at        TEXT,               -- NULL = pending
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_reminders_pending ON reminders (due_at)
    WHERE fired_at IS NULL;

CREATE INDEX idx_reminders_app ON reminders (application_id)
    WHERE fired_at IS NULL;

-- interviews table (may be in 002 with applications, separated here for clarity)
CREATE TABLE interviews (
    id                  TEXT    PRIMARY KEY,
    application_id      TEXT    NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type      TEXT    NOT NULL,   -- 'PhoneScreen'|'Technical'|'OnSite'|'Other'
    scheduled_at        TEXT    NOT NULL,
    duration_minutes    INTEGER NOT NULL DEFAULT 60,
    location            TEXT,
    meeting_url         TEXT,
    interviewers        TEXT,              -- JSON array of strings
    notes               TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_interviews_app ON interviews (application_id);
CREATE INDEX idx_interviews_scheduled ON interviews (scheduled_at);

-- Extend applications table with last_contact_at, next_follow_up_at
-- (applied as ALTER TABLE if not in initial schema)
ALTER TABLE applications ADD COLUMN last_contact_at  TEXT;
ALTER TABLE applications ADD COLUMN next_follow_up_at TEXT;
ALTER TABLE applications ADD COLUMN notes            TEXT;
ALTER TABLE applications ADD COLUMN resume_version_id TEXT;
ALTER TABLE applications ADD COLUMN cover_letter_version_id TEXT;
ALTER TABLE applications ADD COLUMN screening_answers TEXT;  -- JSON
```

### Module Structure

```
lazyjob-core/
  src/
    application/
      mod.rs              ← re-exports
      stage.rs            ← ApplicationStage enum (existing)
      model.rs            ← Application, Interview, Offer structs
      repository.rs       ← ApplicationRepository, InterviewRepository, ReminderRepository traits
      sqlite/
        mod.rs
        application.rs    ← SqliteApplicationRepository (extended with find_by_job_id, touch_last_contact, append_note)
        interview.rs      ← SqliteInterviewRepository
        reminder.rs       ← SqliteReminderRepository
      events.rs           ← WorkflowEvent, ReminderDueEvent
      reminder.rs         ← Reminder struct + ReminderService
      workflows.rs        ← ApplyWorkflow, MoveStageWorkflow, ScheduleInterviewWorkflow, LogContactWorkflow
      poller.rs           ← ReminderPoller

lazyjob-tui/
  src/
    views/
      apply_confirm.rs    ← TUI confirmation dialog for ApplyWorkflow
      stage_transition.rs ← TUI stage transition dialog
      interview_form.rs   ← TUI form for ScheduleInterviewWorkflow
      log_contact.rs      ← TUI inline form for LogContactWorkflow
```

---

## Implementation Phases

### Phase 1 — Core Domain + SQLite (MVP)

**Goal**: All four workflows executable, reminder persistence, broadcast events.

#### 1.1 — Extend Application model

File: `lazyjob-core/src/application/model.rs`

Add fields to `Application` struct:
- `last_contact_at: Option<DateTime<Utc>>`
- `next_follow_up_at: Option<DateTime<Utc>>`
- `notes: Option<String>`
- `resume_version_id: Option<Uuid>`
- `cover_letter_version_id: Option<Uuid>`
- `screening_answers: Option<serde_json::Value>`

Add `Interview` struct with all fields from spec. Derive `sqlx::FromRow` on both.

Add `InterviewType` enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "PascalCase")]
pub enum InterviewType {
    PhoneScreen,
    Technical,
    OnSite,
    Other,
}
```

**Verification**: `cargo test -p lazyjob-core -- model` passes with serde round-trips.

#### 1.2 — SQLite migration

File: `lazyjob-core/migrations/003_reminders.sql`

Apply the DDL from the schema section above. Add `ALTER TABLE` statements as a migration fragment (SQLite allows `ALTER TABLE ADD COLUMN`).

**Verification**: `sqlx migrate run` succeeds; `sqlx migrate revert` and re-run succeeds.

#### 1.3 — `WorkflowEvent` and `ReminderDueEvent`

File: `lazyjob-core/src/application/events.rs`

Implement both enums exactly as shown in Core Types. Add `#[derive(Clone)]` — required by `broadcast::Sender<T>`.

**Verification**: `let (tx, mut rx) = broadcast::channel::<WorkflowEvent>(64);` compiles.

#### 1.4 — `Reminder` struct and `ReminderRepository` trait

File: `lazyjob-core/src/application/reminder.rs`

Implement `Reminder` struct with `new()` constructor. Implement `ReminderRepository` async trait with `insert`, `cancel_by_application`, `list_unfired_before`, `set_fired`.

**Verification**: Trait methods compile with mock impl.

#### 1.5 — `SqliteReminderRepository`

File: `lazyjob-core/src/application/sqlite/reminder.rs`

```rust
pub struct SqliteReminderRepository {
    pool: SqlitePool,
}

#[async_trait::async_trait]
impl ReminderRepository for SqliteReminderRepository {
    async fn insert(&self, r: &Reminder) -> Result<(), anyhow::Error> {
        sqlx::query!(
            "INSERT INTO reminders (id, application_id, title, body, due_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            r.id, r.application_id, r.title, r.body,
            r.due_at.to_rfc3339(), r.created_at.to_rfc3339()
        )
        .execute(&self.pool)
        .await
        .context("insert reminder")?;
        Ok(())
    }

    async fn cancel_by_application(&self, app_id: &Uuid) -> Result<(), anyhow::Error> {
        sqlx::query!(
            "UPDATE reminders SET fired_at = ?1 WHERE application_id = ?2 AND fired_at IS NULL",
            Utc::now().to_rfc3339(), app_id
        )
        .execute(&self.pool)
        .await
        .context("cancel reminders for application")?;
        Ok(())
    }

    async fn list_unfired_before(&self, before: DateTime<Utc>) -> Result<Vec<Reminder>, anyhow::Error> {
        let rows = sqlx::query_as!(
            ReminderRow,
            "SELECT * FROM reminders WHERE fired_at IS NULL AND due_at <= ?1
             ORDER BY due_at ASC",
            before.to_rfc3339()
        )
        .fetch_all(&self.pool)
        .await
        .context("list pending reminders")?;
        rows.into_iter().map(Reminder::try_from).collect()
    }

    async fn set_fired(&self, id: &Uuid, at: DateTime<Utc>) -> Result<(), anyhow::Error> {
        sqlx::query!(
            "UPDATE reminders SET fired_at = ?1 WHERE id = ?2",
            at.to_rfc3339(), id
        )
        .execute(&self.pool)
        .await
        .context("mark reminder fired")?;
        Ok(())
    }
}
```

**Verification**: `#[sqlx::test(migrations = "migrations")]` test inserts a reminder, lists it, marks fired, confirms it no longer appears in `list_unfired_before`.

#### 1.6 — `ReminderService`

File: `lazyjob-core/src/application/reminder.rs` (service section)

Thin wrapper over `ReminderRepository`. No business logic beyond delegation.

**Verification**: Unit test with `MockReminderRepository` (hand-written struct implementing the trait with `Vec<Reminder>` in-memory store).

#### 1.7 — Extend `SqliteApplicationRepository`

File: `lazyjob-core/src/application/sqlite/application.rs`

Add:
```rust
// find_by_job_id
async fn find_by_job_id(&self, job_id: &Uuid) -> Result<Option<Application>> {
    sqlx::query_as!(ApplicationRow, "SELECT * FROM applications WHERE job_id = ?1", job_id)
        .fetch_optional(&self.pool).await.context("find_by_job_id")?.map(Application::try_from).transpose()
}

// touch_last_contact
async fn touch_last_contact(&self, app_id: &Uuid, at: DateTime<Utc>) -> Result<()> {
    sqlx::query!("UPDATE applications SET last_contact_at = ?1, updated_at = ?2 WHERE id = ?3",
        at.to_rfc3339(), Utc::now().to_rfc3339(), app_id)
        .execute(&self.pool).await.context("touch_last_contact")?;
    Ok(())
}

// append_note (prepend with separator for readability)
async fn append_note(&self, app_id: &Uuid, note: &str) -> Result<()> {
    sqlx::query!("UPDATE applications SET
        notes = CASE WHEN notes IS NULL THEN ?1 ELSE notes || '\n---\n' || ?1 END,
        updated_at = ?2 WHERE id = ?3",
        note, Utc::now().to_rfc3339(), app_id)
        .execute(&self.pool).await.context("append_note")?;
    Ok(())
}
```

**Verification**: `#[sqlx::test]` test confirms `append_note` accumulates with separator.

#### 1.8 — `SqliteInterviewRepository`

File: `lazyjob-core/src/application/sqlite/interview.rs`

```rust
pub struct SqliteInterviewRepository {
    pool: SqlitePool,
}

#[async_trait::async_trait]
impl InterviewRepository for SqliteInterviewRepository {
    async fn create(&self, iv: &Interview) -> Result<(), anyhow::Error> {
        let interviewers_json = serde_json::to_string(&iv.interviewers)?;
        sqlx::query!(
            "INSERT INTO interviews (id, application_id, interview_type, scheduled_at,
             duration_minutes, location, meeting_url, interviewers, notes, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            iv.id, iv.application_id, iv.interview_type, iv.scheduled_at.to_rfc3339(),
            iv.duration_minutes, iv.location, iv.meeting_url, interviewers_json,
            iv.notes, iv.created_at.to_rfc3339()
        )
        .execute(&self.pool).await?;
        Ok(())
    }

    async fn find_by_application(&self, app_id: &Uuid) -> Result<Vec<Interview>, anyhow::Error> {
        // fetch rows, map to Interview via TryFrom<InterviewRow>
        todo!()
    }

    async fn delete(&self, interview_id: &Uuid) -> Result<(), anyhow::Error> {
        sqlx::query!("DELETE FROM interviews WHERE id = ?1", interview_id)
            .execute(&self.pool).await?;
        Ok(())
    }
}
```

**Verification**: `#[sqlx::test]` round-trip test.

#### 1.9 — Implement all four workflows

File: `lazyjob-core/src/application/workflows.rs`

Implement `ApplyWorkflow`, `MoveStageWorkflow`, `ScheduleInterviewWorkflow`, `LogContactWorkflow` as shown in Core Types. Focus on the happy path; edge cases in Phase 2.

**Verification**: Unit test each workflow with mock repositories and a broadcast channel. Assert that the correct `WorkflowEvent` variants are sent.

#### 1.10 — `ReminderPoller`

File: `lazyjob-core/src/application/poller.rs`

Implement as shown in Core Types. Use `tokio::time::interval` with the 5-minute default.

**Verification**: Test with a manual `interval` override (1ms) and a pre-seeded overdue reminder. Assert that `ReminderDueEvent` is received on the broadcast channel and `mark_fired` was called.

#### 1.11 — `WorkflowError` enum

File: `lazyjob-core/src/application/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum WorkflowError {
    #[error("application already exists for job {existing_id}")]
    DuplicateApplication { existing_id: Uuid },

    #[error("application {0} not found")]
    NotFound(Uuid),

    #[error("cannot transition from {from:?} to {to:?}")]
    InvalidTransition { from: ApplicationStage, to: ApplicationStage },

    #[error("stage {current:?} does not support action '{action}'")]
    InvalidStageForAction { current: ApplicationStage, action: &'static str },

    #[error("repository error: {0}")]
    Repository(#[from] anyhow::Error),

    #[error("reminder operation failed: {0}")]
    ReminderFailed(anyhow::Error),
}
```

**Verification**: All `WorkflowError` variants are used in workflows; `?` propagation compiles.

---

### Phase 2 — Anti-Spam Architecture + Ghost Score Integration

**Goal**: Enforce duplicate check gate, ghost score warning, daily application count metric.

#### 2.1 — Ghost score check in `ApplyWorkflow`

The `GhostDetector` service is defined in the job-search domain spec. Add an optional dependency injection point to `ApplyWorkflow`:

```rust
pub struct ApplyWorkflow {
    // ... existing fields
    pub ghost_detector: Option<Arc<dyn GhostDetector>>,
}
```

In `ApplyWorkflow::execute`, after the duplicate check:

```rust
if let Some(detector) = &self.ghost_detector {
    let score = detector.score(job_id).await.unwrap_or(0.0);
    if score > 0.7 {
        // Return a warning — the caller (TUI) decides whether to proceed
        return Err(WorkflowError::GhostJobWarning {
            job_id: *job_id,
            score,
        });
    }
}
```

Add `GhostJobWarning { job_id: Uuid, score: f64 }` to `WorkflowError`. The TUI maps this error to a dismissable confirmation dialog. If the user confirms, it calls `execute` again with a `bypass_ghost_check: bool` flag in `ApplyOptions`.

**Verification**: Test that `execute` returns `GhostJobWarning` when detector returns score > 0.7, and proceeds when `bypass_ghost_check = true`.

#### 2.2 — Daily application count metric

```rust
// lazyjob-core/src/application/metrics.rs

pub struct PipelineMetrics {
    app_repo: Arc<dyn ApplicationRepository>,
}

impl PipelineMetrics {
    pub async fn applications_created_today(&self) -> Result<u32, anyhow::Error> {
        self.app_repo.count_created_since(Utc::today().and_hms(0, 0, 0)).await
    }
}
```

Add `count_created_since(after: DateTime<Utc>) -> Result<u32>` to `ApplicationRepository` trait.

In `ApplyWorkflow::execute`, query the metric after the duplicate check. If `count >= threshold` (default 10, configurable), emit `WorkflowEvent::QualityWarning { applications_today: u32 }` alongside the result — not an error, the application still proceeds.

**Verification**: Seed 10 applications created today in test DB; assert warning event is sent on the 11th.

#### 2.3 — Tailoring soft nudge

`ApplyOptions` already carries `resume_version_id: Option<Uuid>`. When `None` is passed (user chose "Skip resume"), `ApplyWorkflow::execute` emits a `WorkflowEvent::NoResumeWarning { application_id }` event. The TUI may display this as a status-bar note ("Applied without a resume — consider tailoring for your next role") but does NOT block.

---

### Phase 3 — TUI Integration

**Goal**: Confirmation dialogs, stage transition dialog, interview form, log contact inline form.

#### 3.1 — `ApplyConfirmDialog`

File: `lazyjob-tui/src/views/apply_confirm.rs`

A modal overlay rendered on top of the job feed. Uses `ratatui::widgets::Clear` to erase the background within a centered `Rect`.

Layout:
```
┌─ Apply to [Company] - [Job Title] ──────────────────┐
│ Resume: [Tailored v2] / [Latest] / [None]           │
│ Cover letter: [Generated] / [None]                  │
│ Ghost score: ⚠ 72% — may be a ghost posting         │
│                                                      │
│       [Cancel]          [Apply]                      │
└──────────────────────────────────────────────────────┘
```

Key bindings: `Tab`/`Shift+Tab` to cycle fields, `Enter` to confirm, `Esc` to cancel.

State machine for the dialog:
```rust
pub enum ApplyConfirmState {
    SelectResume,
    SelectCoverLetter,
    Confirm,   // focused on [Apply] button
}
```

On `Enter` in `Confirm` state, the dialog emits a `TuiAction::ExecuteApply(ApplyOptions)`. The event loop receives this, calls `ApplyWorkflow::execute`, and handles `WorkflowError::GhostJobWarning` by showing a secondary inline warning with `[Apply Anyway]` / `[Cancel]` options.

**Verification**: Render the widget in a `TestBackend` and assert the rendered output contains "Ghost score" when the injected score > 0.7.

#### 3.2 — `StageTransitionDialog`

File: `lazyjob-tui/src/views/stage_transition.rs`

A compact modal confirming a stage change. Shows current stage → new stage with a styled arrow. For `Rejected`/`Withdrawn`, adds an optional text field for reason.

```
┌─ Move Stage ─────────────────────────────────┐
│ Applied  →  Phone Screen                     │
│                                              │
│  [Cancel]            [Confirm]               │
└──────────────────────────────────────────────┘
```

For terminal transitions (Rejected/Withdrawn), renders an additional row:
```
│ Reason (optional): [____________]            │
```

Uses `ratatui::widgets::Paragraph` with `ratatui::style::Style::default().fg(Color::Yellow)` for the arrow.

**Verification**: Snapshot test of rendered output for each transition type.

#### 3.3 — `InterviewFormView`

File: `lazyjob-tui/src/views/interview_form.rs`

A multi-field form dialog for `ScheduleInterviewWorkflow::execute`. Fields:

| Field | Widget |
|---|---|
| Date + time | `TextInput` — parsed via `chrono::NaiveDateTime::parse_from_str` |
| Duration (minutes) | `NumberInput` — increments with `+`/`-` keys |
| Location or URL | `TextInput` |
| Interviewers | `TextInput` — comma-separated |

On `Enter` in the last field, emits `TuiAction::ScheduleInterview(InterviewDetails)`.

**Verification**: Unit test that parsing "2026-05-01 14:00" in date field produces the correct `DateTime<Utc>`.

#### 3.4 — `LogContactForm`

File: `lazyjob-tui/src/views/log_contact.rs`

An inline (non-modal) bottom-panel form triggered by `n` in the application detail view. Shows a single text input for the note and a dropdown for contact type.

On `Enter`, emits `TuiAction::LogContact(LogContactInput)` and closes the form panel.

---

### Phase 4 — `PostTransitionSuggestion` Dispatch

**Goal**: Wire post-transition suggestions to Ralph loop spawning.

After `MoveStageWorkflow::execute` returns a `MoveStageResult`, the event loop inspects `suggestions`:

```rust
for suggestion in result.suggestions {
    match suggestion {
        PostTransitionSuggestion::GenerateInterviewPrep { application_id } => {
            // Display status-bar message: "Press `p` to generate interview prep"
            app_state.pending_suggestion = Some(PendingSuggestion::InterviewPrep(application_id));
        }
        PostTransitionSuggestion::RunSalaryComparison { application_id } => {
            app_state.pending_suggestion = Some(PendingSuggestion::SalaryComparison(application_id));
        }
        // ...
    }
}
```

`PendingSuggestion` is rendered as a non-blocking status-bar hint. Pressing the keybind (e.g., `p`) calls the Ralph orchestration layer to spawn the relevant loop. Pressing `Esc` or any other key dismisses it.

This dispatch is handled in `lazyjob-tui` and delegates to `lazyjob-ralph`. It must NOT be a modal dialog — it must not interrupt user navigation.

**Verification**: Integration test that simulates `MoveStageWorkflow::execute` returning `GenerateInterviewPrep`, then exercises the event loop to confirm the status-bar message appears.

---

### Phase 5 — Greenhouse/Lever Direct Apply (Phase 2 scope)

**Goal**: Optionally fill application forms via Greenhouse/Lever API, always requiring user preview.

This phase is intentionally deferred from the MVP. It requires:

1. A new `DirectApplyPlatform` trait with `prefill_form(job_id, life_sheet) -> Result<FormPreview>` and `submit(form_preview: FormPreview, confirmed: bool) -> Result<SubmissionReceipt>`.
2. Per-company opt-in gating via `platform.greenhouse.direct_apply = true` in user config.
3. A `DirectApplyPreviewDialog` in the TUI that renders every field that will be submitted, with a hard "REVIEW BEFORE SUBMIT" header.
4. `auto_submit = false` is enforced in the trait implementation with a compile-time assertion comment — the method does not exist.

`ApplyWorkflow::execute` checks `opts.direct_apply_platform` (an `Option<Arc<dyn DirectApplyPlatform>>`); if `Some`, it calls `prefill_form` and returns a `ApplyResult::AwaitingDirectApplyConfirmation(FormPreview)` variant rather than `Ok(Application)`, signalling the TUI to open the preview dialog before creating the application record.

---

## Key Crate APIs

```rust
// tokio broadcast channel (events)
let (tx, mut rx) = tokio::sync::broadcast::channel::<WorkflowEvent>(64);
let _ = tx.send(WorkflowEvent::ApplicationCreated { ... });
let event = rx.recv().await?;

// sqlx query macros
sqlx::query!("UPDATE reminders SET fired_at = ?1 WHERE id = ?2", at, id)
    .execute(&pool).await?;

sqlx::query_as!(ReminderRow, "SELECT * FROM reminders WHERE fired_at IS NULL AND due_at <= ?1", before)
    .fetch_all(&pool).await?;

// tokio time interval (poller)
let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
ticker.tick().await;  // first tick fires immediately

// chrono duration arithmetic
let follow_up = Utc::now() + chrono::Duration::days(7);
let prep_due  = scheduled_at - chrono::Duration::hours(24);

// uuid v4
let id = uuid::Uuid::new_v4();

// async-trait
#[async_trait::async_trait]
impl ReminderRepository for SqliteReminderRepository { ... }

// ratatui overlay (TUI dialogs)
frame.render_widget(ratatui::widgets::Clear, dialog_area);
frame.render_widget(dialog_widget, dialog_area);

// ratatui TestBackend for widget tests
let backend = ratatui::backend::TestBackend::new(80, 24);
let mut terminal = ratatui::Terminal::new(backend)?;
terminal.draw(|f| render_dialog(f, &state))?;
let buffer = terminal.backend().buffer().clone();
assert!(buffer_contains(&buffer, "Ghost score"));
```

---

## Error Handling

```rust
// lazyjob-core/src/application/error.rs

#[derive(thiserror::Error, Debug)]
pub enum WorkflowError {
    #[error("application already exists for job {existing_id}")]
    DuplicateApplication { existing_id: Uuid },

    #[error("application {0} not found")]
    NotFound(Uuid),

    #[error("cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: ApplicationStage,
        to: ApplicationStage,
    },

    #[error("stage {current:?} does not support action '{action}'")]
    InvalidStageForAction {
        current: ApplicationStage,
        action: &'static str,
    },

    #[error("ghost job warning: score {score:.2} exceeds threshold for job {job_id}")]
    GhostJobWarning {
        job_id: Uuid,
        score: f64,
    },

    #[error("repository operation failed: {0}")]
    Repository(#[from] anyhow::Error),

    #[error("reminder operation failed: {0}")]
    ReminderFailed(anyhow::Error),
}

pub type WorkflowResult<T> = Result<T, WorkflowError>;
```

`WorkflowError::GhostJobWarning` is NOT a fatal error — it is a user-interactive error that the TUI interprets as "show confirmation, then retry." All other variants are terminal errors that the TUI maps to error notifications.

---

## Testing Strategy

### Unit Tests (no database)

**MockApplicationRepository**: hand-written struct with `Vec<Application>` in memory, implements `ApplicationRepository`. Tests `ApplyWorkflow::execute` in isolation.

**MockReminderRepository**: `Vec<Reminder>`, implements `ReminderRepository`. Tests `ReminderService::create`, `cancel_for_application`, `list_pending`.

Key unit tests:
- `apply_workflow_creates_application_and_reminder`: feed a valid `ApplyOptions`, assert `ApplicationCreated` and `ReminderCreated` events are received.
- `apply_workflow_rejects_duplicate`: pre-seed an application for the same `job_id`, assert `DuplicateApplication` error.
- `move_stage_cancels_reminders_on_rejection`: seed two pending reminders, call `MoveStageWorkflow` with `target = Rejected`, assert `cancel_by_application` was called.
- `move_stage_produces_suggestions_on_phone_screen`: assert `GenerateInterviewPrep` in suggestions when transitioning to `PhoneScreen`.
- `schedule_interview_creates_two_reminders`: assert both the pre-interview and prep reminders are created when `prep_reminder_days_before = Some(2)`.
- `log_contact_appends_with_separator`: call `execute` twice, assert note contains `---` separator.
- `reminder_poller_fires_overdue_reminders`: inject a `MockReminderService` with one overdue reminder, run poller for 1ms interval, assert `ReminderDueEvent` received and `mark_fired` called.

### Integration Tests (real SQLite)

Use `#[sqlx::test(migrations = "migrations")]` macro. Each test gets a fresh in-memory SQLite with all migrations applied.

Key integration tests:
- `integration_apply_workflow_end_to_end`: insert a `Job` row, run `ApplyWorkflow::execute`, assert row in `applications` table, row in `reminders` table with correct `due_at`, correct events received.
- `integration_move_stage_rejected_cancels_reminders`: seed application + reminder, `MoveStageWorkflow` to `Rejected`, assert `reminders.fired_at IS NOT NULL`.
- `integration_schedule_interview_creates_row`: run `ScheduleInterviewWorkflow::execute`, query `interviews` table, assert row exists with correct `interview_type`.
- `integration_append_note_accumulates`: call `append_note` twice, query `applications.notes`, assert both notes present with separator.

### TUI Tests

Use `ratatui::backend::TestBackend` (fixed-size in-memory buffer). No terminal required.

- `apply_confirm_dialog_renders_ghost_warning`: construct dialog state with ghost score 0.75, render to buffer, assert "Ghost score" text present.
- `stage_transition_dialog_shows_reason_field_for_rejected`: construct dialog with `target = Rejected`, render, assert reason input field visible.
- `stage_transition_dialog_hides_reason_for_phone_screen`: render with `target = PhoneScreen`, assert no reason field.

---

## Open Questions

1. **Direct apply via Greenhouse/Lever**: The spec defers this to Phase 2. What is the exact Greenhouse API endpoint for submitting an application? The public Greenhouse Harvest API (`POST /v1/applications`) requires an `on_behalf_of` parameter that maps to a user in the ATS — this may not be available for job board listings. Needs investigation before Phase 5 implementation.

2. **Screening question pre-fill**: The spec mentions collecting screening answers in `ApplyOptions.screening_answers`. Should LazyJob auto-fill common questions ("Are you authorized to work in the US?") from the `LifeSheet`? If yes, what is the LifeSheet field path? This should be a Phase 2 feature to keep the MVP scope tight.

3. **Bulk stage transitions**: If the user is mass-rejected from 5 jobs (e.g., ATS auto-reject), should `MoveStageWorkflow` support a batch operation? The current API operates on a single `application_id`. A `Vec<Uuid>` batch mode would be efficient but must still emit one `StageChanged` event per application to keep the TUI synchronized correctly.

4. **Application templates**: `ApplyOptions` bundles (always tailor + cover letter + 5-day follow-up) are referenced in the spec as Phase 2. Where are these stored? A `application_templates` table or a TOML config section? The config section approach avoids schema complexity for MVP.

5. **Ghost detector coupling**: `ApplyWorkflow` takes `Option<Arc<dyn GhostDetector>>`. If `GhostDetector` is defined in the `job-search` crate and `ApplyWorkflow` is in `lazyjob-core`, there is a potential crate-level circular dependency. The `GhostDetector` trait should be defined in `lazyjob-core` (domain boundary) and implemented in the job-search layer, or moved to a shared `lazyjob-types` crate.

6. **`ReminderPoller` shutdown**: The current implementation runs a `loop {}` forever. It should accept a `CancellationToken` (from `tokio-util`) so the poller can be gracefully shut down when the application exits. Add `token: CancellationToken` to `ReminderPoller::run` signature in Phase 2.

---

## Related Specs

- [`specs/application-state-machine.md`](./application-state-machine.md) — defines `ApplicationStage`, `can_transition_to()`, and `ApplicationRepository` trait that this plan extends
- [`specs/10-application-workflow.md`](./10-application-workflow.md) — higher-level workflow spec covering kanban view integration
- [`specs/12-15-interview-salary-networking-notifications.md`](./12-15-interview-salary-networking-notifications.md) — `ReminderPoller` and desktop notification delivery
- [`specs/agentic-ralph-orchestration.md`](./agentic-ralph-orchestration.md) — `PostTransitionSuggestion` dispatch to Ralph loop spawning
- [`specs/job-search-ghost-job-detection.md`](./job-search-ghost-job-detection.md) — `GhostDetector` trait, score computation
- [`specs/09-tui-design-keybindings-implementation-plan.md`](./09-tui-design-keybindings-implementation-plan.md) — `Action` enum, modal dialog system, keybinding dispatch
