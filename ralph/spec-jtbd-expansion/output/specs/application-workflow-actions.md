# Spec: Application Workflow Actions

**JTBD**: A-2 — Apply to jobs efficiently without repetitive manual work
**Topic**: Orchestrate the actions a user takes within a job application — submitting, scheduling interviews, advancing stages, and logging contact — while enforcing human-in-the-loop boundaries.
**Domain**: application-tracking

---

## What

The workflow actions layer orchestrates multi-step operations on `Application` records. It defines three principal workflows: `ApplyWorkflow` (create an application with optional tailored resume and cover letter), `MoveStageWorkflow` (advance or retreat an application's stage with pre/post-action side effects), and `ScheduleInterviewWorkflow` (record interview details, update last-contact timestamp, and create prep reminders). Each workflow runs through the `ApplicationRepository` and emits `WorkflowEvent`s that the TUI subscribes to for confirmation dialogs and progress display.

## Why

Users don't perform isolated CRUD operations on application records — they perform contextual actions with cascading effects. Clicking "Apply" should optionally trigger resume tailoring, cover letter generation, application record creation, and a follow-up reminder — all atomically from the user's perspective. Without workflow orchestration, each of these steps would be a separate user action, recreating the exact manual overhead the product is meant to eliminate.

The human-in-the-loop boundary is equally critical. The recruiter-side data is unambiguous: 91% of recruiters have spotted candidate AI deception, 34% spend half their week filtering spam applications, and companies are adding more screening steps in response. LazyJob's positioning must be "agent-assisted, human-authentic." That means: the AI prepares, the human approves, the human acts. The application submission itself always happens by the human — LazyJob never auto-submits.

## How

### Principal Workflows

#### ApplyWorkflow

Triggered when the user selects "Apply" on a job from the feed or job detail view.

```
Step 1: Duplicate check — query ApplicationRepository for existing application on this job_id
        → if found: surface existing application, offer "Update" not "Apply"
Step 2: Resume decision (TUI dialog)
        → "Tailor resume for this job" → spawn ralph resume-tailoring loop → await user approval
        → "Use latest resume" → fetch most recent ResumeVersion from SQLiteResumeVersionRepository
        → "Skip resume" → proceed without (warn: 89% of hiring managers expect a resume)
Step 3: Cover letter decision (TUI dialog, can be skipped)
        → "Generate cover letter" → spawn ralph cover-letter loop → await user approval
        → "Skip" → proceed without
Step 4: Screening questions check
        → if job has known_screening_questions (from platform metadata): surface them in TUI
        → user fills answers; answers stored in application record
Step 5: Create Application record (ApplicationStage::Applied, link resume/cover_letter version IDs)
Step 6: Update Job record: status = Applied, applied_at = now()
Step 7: Emit WorkflowEvent::ApplicationCreated → TUI confirms
Step 8: Schedule follow-up reminder at +7 days (configurable in user preferences)
```

**Critical design rule**: Step 5 creates the application record. Actual submission to the company's ATS is NOT part of this workflow — the user submits through the company's career portal or Greenhouse/Lever form directly. LazyJob stores the record of what was submitted and when.

**Exception — Greenhouse/Lever direct apply (Phase 2 only)**: For companies using Greenhouse or Lever with public job boards, LazyJob can optionally fill the application form via the API. This is gated behind a user setting (`platform.greenhouse.direct_apply = true`) and always requires a final TUI confirmation showing the complete data that will be submitted. Auto-submit without user review is permanently disabled.

#### MoveStageWorkflow

Triggered when the user advances an application's stage (kanban move, keyboard shortcut, or menu action).

```
Step 1: Validate transition via ApplicationStage::can_transition_to
        → if invalid: surface error with list of valid next stages
Step 2: Pre-transition side effects (by target stage):
        Rejected  → prompt: "Add rejection reason?" → store in StageTransition.reason
                    → cancel pending follow-up reminders for this application
        Withdrawn → prompt: "Why are you withdrawing?" → store reason
                    → cancel pending reminders
        Offer     → prompt: "Compensation details (optional)?" → create Offer record
Step 3: Execute transition: ApplicationRepository::update_stage
Step 4: Post-transition side effects:
        Applied      → create follow-up reminder at +7 days
        PhoneScreen  → suggest: "Generate interview prep questions?"
        Technical    → suggest: "Generate technical prep for [company]?"
        OnSite       → suggest: "Generate company cheat sheet for interview day?"
        Offer        → suggest: "Run salary market comparison?"
        Accepted     → archive all other applications at same stage or below (optional, user confirms)
Step 5: Update application.last_contact_at = now() if transitioning forward
Step 6: Emit WorkflowEvent::StageChanged → TUI updates kanban column
```

**Suggestion system**: Post-transition suggestions (step 4) are non-blocking. They are surfaced as soft prompts in the status bar ("Press `p` to generate interview prep"). The user can ignore them entirely. They are NOT modal dialogs — they must not interrupt flow.

#### ScheduleInterviewWorkflow

Triggered when the user logs an upcoming interview (typically after transitioning to PhoneScreen/Technical/OnSite).

```
Step 1: Validate application is in an active interview stage
        (PhoneScreen, Technical, or OnSite)
Step 2: Collect interview details via TUI form:
        - Date + time (freeform entry, parsed to DateTime<Utc>)
        - Duration in minutes (default: 60)
        - Location or meeting URL
        - Interviewer name(s) (optional, comma-separated)
        - Interview type (auto-inferred from stage, can override)
Step 3: Create Interview record
Step 4: Update application.last_contact_at = now()
Step 5: Create reminder N hours before interview (default: 24h; configurable)
Step 6: Optionally: create prep reminder N days before (default: 2 days)
Step 7: Emit WorkflowEvent::InterviewScheduled
```

#### LogContactWorkflow

A lightweight workflow for recording any contact event (email reply, recruiter call, rejection call) without a stage transition.

```
Step 1: User triggers "Log contact" (shortcut: `n` in application detail view)
Step 2: Collect: contact type, note text, optional contact name
Step 3: Update application.last_contact_at = now()
Step 4: Append note to application.notes (with timestamp prefix)
Step 5: Cancel next_follow_up_at if contact was received (reset stale clock)
```

### Human-in-the-Loop Boundaries

The following actions are **always automated** (no confirmation needed):
- Creating follow-up reminders
- Logging activity/transitions
- Updating `last_contact_at`
- Stale detection (passive background logic)

The following actions require **explicit user confirmation** before proceeding:
- Creating an application record (because it commits intent)
- Stage transitions (brief TUI confirmation dialog)
- Direct API submission via Greenhouse/Lever (full data preview required)
- Archiving sibling applications after accepting an offer

The following actions are **never automated** regardless of configuration:
- Sending any email or message (LazyJob generates drafts; user sends)
- Submitting to an ATS without review
- Withdrawing or declining an offer
- Deleting application records

This boundary map is explicit product policy, not just a safety feature. From the recruiter research: 34% of recruiters spend half their week filtering AI spam. LazyJob's quality signal depends on staying firmly on the human-authentic side of this line.

### Anti-Spam Architecture

LazyJob enforces quality-over-volume at the workflow level:

1. **Duplicate check gate**: `ApplyWorkflow` blocks re-application to the same job (same `job_id`). Prevents accidental re-sends.
2. **Ghost job gate**: `ApplyWorkflow` runs a ghost score check on the job before creating the application. If `ghost_score > 0.7`, it surfaces a warning: "This posting shows signs of being a ghost job (27-30% of listings are never filled). Apply anyway?" User still decides — the gate does not block.
3. **Daily application count metric**: `PipelineMetrics` tracks `applications_created_today`. No hard cap, but if the count exceeds a configurable threshold (default: 10/day), a non-blocking warning is shown: "Quality over volume: you've applied to N roles today. Consider refining your target list." This is informational, not a blocker.
4. **Tailoring gate (soft)**: `ApplyWorkflow` gently nudges users to tailor before applying. It does not block un-tailored applications — that would break the flow for jobs where the user's LifeSheet is a strong natural match.

### Workflow Event System

All workflow outcomes emit typed events over a broadcast channel. The TUI subscribes to these for real-time updates without polling SQLite.

```rust
pub enum WorkflowEvent {
    ApplicationCreated { application_id: Uuid, job_id: Uuid },
    StageChanged { application_id: Uuid, from: ApplicationStage, to: ApplicationStage },
    InterviewScheduled { interview_id: Uuid, application_id: Uuid, at: DateTime<Utc> },
    ContactLogged { application_id: Uuid },
    ReminderCreated { reminder_id: Uuid, application_id: Uuid, due_at: DateTime<Utc> },
    WorkflowError { workflow: &'static str, error: String },
}
```

The channel is tokio `broadcast` (multi-consumer, bounded at 64 events). The ralph subprocess can also emit `WorkflowEvent::ApplicationCreated` events when it auto-creates application drafts from discoveries — the TUI receives these on the same channel.

## Interface

```rust
// lazyjob-core/src/application/workflows.rs

pub struct ApplyWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub resume_repo: Arc<dyn ResumeVersionRepository>,
    pub cover_letter_repo: Arc<dyn CoverLetterVersionRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

impl ApplyWorkflow {
    pub async fn execute(&self, job_id: &Uuid, opts: ApplyOptions) -> Result<Application>;
    pub async fn check_duplicate(&self, job_id: &Uuid) -> Result<Option<Application>>;
}

pub struct ApplyOptions {
    pub resume_version_id: Option<Uuid>,   // None = prompt user
    pub cover_letter_version_id: Option<Uuid>,
    pub screening_answers: Option<serde_json::Value>,
    pub follow_up_days: u32,               // default: 7
}

pub struct MoveStageWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

impl MoveStageWorkflow {
    pub async fn execute(
        &self,
        app_id: &Uuid,
        target: ApplicationStage,
        reason: Option<&str>,
    ) -> Result<MoveStageResult>;
}

pub struct MoveStageResult {
    pub application: Application,
    pub suggestions: Vec<PostTransitionSuggestion>,
}

pub enum PostTransitionSuggestion {
    GenerateInterviewPrep { application_id: Uuid },
    RunSalaryComparison { application_id: Uuid },
    GenerateCompanyCheatSheet { application_id: Uuid },
}

pub struct ScheduleInterviewWorkflow {
    pub app_repo: Arc<dyn ApplicationRepository>,
    pub interview_repo: Arc<dyn InterviewRepository>,
    pub reminder_service: Arc<ReminderService>,
    pub events: broadcast::Sender<WorkflowEvent>,
}

impl ScheduleInterviewWorkflow {
    pub async fn execute(
        &self,
        app_id: &Uuid,
        details: InterviewDetails,
    ) -> Result<Interview>;
}

pub struct InterviewDetails {
    pub scheduled_at: DateTime<Utc>,
    pub duration_minutes: u32,
    pub location: Option<String>,
    pub meeting_url: Option<String>,
    pub interviewers: Vec<String>,
    pub interview_type: Option<InterviewType>,  // None = infer from stage
}

pub struct ReminderService {
    repo: Arc<dyn ReminderRepository>,
}

impl ReminderService {
    pub async fn create(&self, r: &Reminder) -> Result<Uuid>;
    pub async fn cancel_for_application(&self, app_id: &Uuid) -> Result<()>;
    pub async fn list_pending(&self, before: DateTime<Utc>) -> Result<Vec<Reminder>>;
}

pub struct Reminder {
    pub id: Uuid,
    pub application_id: Option<Uuid>,
    pub title: String,
    pub body: Option<String>,
    pub due_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
```

```sql
-- lazyjob-core/migrations/002_applications.sql (continuation)
CREATE TABLE reminders (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT REFERENCES applications(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT,
    due_at TEXT NOT NULL,
    fired_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_reminders_due_at ON reminders(due_at) WHERE fired_at IS NULL;
```

## Open Questions

- **Direct apply via Greenhouse/Lever API**: When is this safe to enable? Requires mapping LifeSheet fields to each platform's form schema. Should this be a Phase 2 feature requiring explicit opt-in per company, not a global setting?
- **Screening question pre-fill**: LazyJob could pre-fill common ATS screening questions ("Why do you want to work here?", "Are you authorized to work in the US?") from the LifeSheet. Is this a workflow step or a separate ralph loop?
- **Bulk stage transitions**: If a user is auto-rejected by an ATS from 5 applications in one day, should they be able to bulk-move them all to Rejected in one action? Or is per-application intentionality important for data quality?
- **Application templates**: Reusable ApplyOptions bundles for similar role types (e.g., "my standard fintech application: always tailor resume, always generate cover letter, 5-day follow-up"). Is this Phase 1 or Phase 2 scope?

## Implementation Tasks

- [ ] Implement `ApplyWorkflow::execute` and `check_duplicate` in `lazyjob-core/src/application/workflows.rs`
- [ ] Implement `MoveStageWorkflow::execute` with pre/post side-effect hooks in `lazyjob-core/src/application/workflows.rs`
- [ ] Implement `ScheduleInterviewWorkflow::execute` in `lazyjob-core/src/application/workflows.rs`
- [ ] Implement `LogContactWorkflow::execute` in `lazyjob-core/src/application/workflows.rs`
- [ ] Implement `ReminderService` and `SqliteReminderRepository` in `lazyjob-core/src/application/reminders.rs`
- [ ] Add `WorkflowEvent` enum and tokio broadcast channel wiring in `lazyjob-core/src/application/events.rs`
- [ ] Add `reminders` table to `lazyjob-core/migrations/002_applications.sql`
- [ ] Add ghost score check call in `ApplyWorkflow` (delegates to `GhostDetector` from job-search domain)
- [ ] Build TUI apply confirmation dialog in `lazyjob-tui/src/views/apply_confirm.rs` — shows job title, company, resume version, cover letter status, ghost score warning if applicable
- [ ] Build TUI stage transition dialog in `lazyjob-tui/src/views/stage_transition.rs` — shows current → next stage, optional reason field, confirm/cancel
