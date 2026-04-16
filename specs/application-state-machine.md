# Spec: Application State Machine

**JTBD**: A-3 — Track where I stand in every hiring process at a glance
**Topic**: Model each job application as a 10-stage state machine with validated transitions and a full history log.
**Domain**: application-tracking

---

## What

The application state machine represents the lifecycle of a single job application from first discovery through terminal resolution. It defines ten stages (`ApplicationStage` enum), a validated transition matrix, and an immutable history log (`application_transitions` SQLite table). Each transition stores who triggered it, when, and why. The machine is the canonical source of truth for where an application stands — all views, metrics, and workflow actions derive from it.

## Why

Job seekers actively managing 10–50+ applications in parallel quickly lose track of where each one stands. Without a structured state model, application status lives in email threads, calendar notes, and human memory — all unreliable. A properly modeled state machine enables the TUI kanban view, automated stale detection, follow-up reminders, and pipeline health metrics. It also provides the data foundation for learning what actions correlate with better outcomes.

The 10-stage model is calibrated to real hiring pipelines: it's granular enough to be meaningful (distinguishing Phone Screen from Technical from On-site matters for interview prep) but not so granular that users spend time managing stages instead of applying.

## How

### Stage Definitions

```
Discovered   — Job found via discovery loop; not yet reviewed by user
Interested   — User has reviewed and flagged as worth applying
Applied      — Application submitted; waiting for recruiter response
PhoneScreen  — Recruiter screen scheduled or completed
Technical    — Technical round scheduled or completed (async challenge or live coding)
OnSite       — Final rounds in progress (may be multiple loops)
Offer        — Offer letter received; negotiation window open
Accepted     — User accepted offer; application complete
Rejected     — Not moving forward (can arrive from any non-terminal stage)
Withdrawn    — User chose not to continue (can arrive from any non-terminal stage)
```

**Terminal stages**: `Accepted`, `Rejected`, `Withdrawn`. No transitions out of terminal stages.

### Transition Matrix

The `can_transition_to` function encodes allowed transitions explicitly rather than inferring from stage order. This prevents silent bugs where an invalid transition is accepted because the code only checked "is next stage > current stage."

Allowed transitions:
- **Forward progression** (standard path): Discovered→Interested, Interested→Applied, Applied→PhoneScreen, PhoneScreen→Technical, Technical→OnSite, OnSite→Offer, Offer→Accepted
- **Stage skipping** (real pipelines skip stages): Discovered→Applied, Applied→OnSite, Applied→Offer, PhoneScreen→Offer, Technical→Offer
- **Backward correction** (user enters wrong stage): Interested→Discovered, Applied→Interested, PhoneScreen→Applied, Technical→PhoneScreen, OnSite→Technical, Offer→OnSite
- **Any → Rejected** (from any non-terminal stage)
- **Any → Withdrawn** (from any non-terminal, non-Accepted stage)

Backward transitions exist not because pipelines go backward, but because users make data entry errors. The transition log records what really happened; the stage just reflects current user knowledge.

### Transition History

Every stage change is appended to an `application_transitions` table. This log is never modified — only appended. Transitions carry an optional `reason` field (user note or system message).

```sql
-- lazyjob-core/migrations/002_applications.sql
CREATE TABLE applications (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    stage TEXT NOT NULL DEFAULT 'Discovered',
    resume_version_id TEXT REFERENCES resume_versions(id),
    cover_letter_version_id TEXT REFERENCES cover_letter_versions(id),
    notes TEXT NOT NULL DEFAULT '',
    last_contact_at TEXT,
    next_follow_up_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE application_transitions (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    from_stage TEXT NOT NULL,
    to_stage TEXT NOT NULL,
    reason TEXT,
    transitioned_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE application_contacts (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT,          -- "recruiter", "hiring manager", "interviewer"
    email TEXT,
    linkedin_url TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE interviews (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type TEXT NOT NULL,  -- "PhoneScreen" | "Technical" | "OnSite" | "Panel" | "Async"
    scheduled_at TEXT,
    duration_minutes INTEGER,
    location TEXT,
    meeting_url TEXT,
    interviewers TEXT,  -- JSON array of names
    status TEXT NOT NULL DEFAULT 'Scheduled',  -- "Scheduled" | "Completed" | "Cancelled"
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE offers (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    base_salary INTEGER,
    equity_pct REAL,
    equity_cliff_months INTEGER,
    equity_vest_months INTEGER,
    signing_bonus INTEGER,
    annual_bonus_target_pct REAL,
    benefits_notes TEXT,
    offer_date TEXT,
    expiry_date TEXT,
    status TEXT NOT NULL DEFAULT 'Pending',  -- "Pending" | "Countered" | "Accepted" | "Declined"
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_applications_stage ON applications(stage);
CREATE INDEX idx_applications_job_id ON applications(job_id);
CREATE INDEX idx_application_transitions_application_id ON application_transitions(application_id);
```

### Naming Distinction

The `application_contacts` table above is distinct from `profile_contacts` (in the LifeSheet). `profile_contacts` stores the user's professional network (for JTBD A-4 — networking). `application_contacts` stores people encountered during a specific hiring process (recruiter, panel interviewers, hiring manager). The same person may appear in both tables; they serve different lookup patterns.

### Rust State Machine

```rust
// lazyjob-core/src/application/stage.rs

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
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected | Self::Withdrawn)
    }

    pub fn is_active(self) -> bool {
        !self.is_terminal()
    }

    /// Returns true if this transition is permitted.
    /// The matrix is explicit rather than inferred from stage ordering.
    pub fn can_transition_to(self, next: ApplicationStage) -> bool {
        use ApplicationStage::*;
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // Forward
            (Discovered, Interested)
            | (Discovered, Applied)
            | (Interested, Applied)
            | (Applied, PhoneScreen)
            | (PhoneScreen, Technical)
            | (Technical, OnSite)
            | (OnSite, Offer)
            | (Offer, Accepted) => true,
            // Skip stages (real hiring pipelines)
            (Applied, OnSite)
            | (Applied, Offer)
            | (PhoneScreen, Offer)
            | (Technical, Offer) => true,
            // Backward (user data correction)
            (Interested, Discovered)
            | (Applied, Interested)
            | (PhoneScreen, Applied)
            | (Technical, PhoneScreen)
            | (OnSite, Technical)
            | (Offer, OnSite) => true,
            // Any non-terminal → terminal
            (_, Rejected) => true,
            (_, Withdrawn) => !matches!(self, Accepted | Rejected | Withdrawn),
            _ => false,
        }
    }

    /// All stages in UI display order (used for kanban columns).
    pub fn display_order() -> &'static [ApplicationStage] {
        &[
            ApplicationStage::Discovered,
            ApplicationStage::Interested,
            ApplicationStage::Applied,
            ApplicationStage::PhoneScreen,
            ApplicationStage::Technical,
            ApplicationStage::OnSite,
            ApplicationStage::Offer,
            ApplicationStage::Accepted,
            ApplicationStage::Rejected,
            ApplicationStage::Withdrawn,
        ]
    }
}
```

```rust
// lazyjob-core/src/application/model.rs

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

pub struct StageTransition {
    pub id: Uuid,
    pub application_id: Uuid,
    pub from_stage: ApplicationStage,
    pub to_stage: ApplicationStage,
    pub reason: Option<String>,
    pub transitioned_at: DateTime<Utc>,
}

pub struct Interview {
    pub id: Uuid,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<u32>,
    pub location: Option<String>,
    pub meeting_url: Option<String>,
    pub interviewers: Vec<String>,
    pub status: InterviewStatus,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterviewType { PhoneScreen, Technical, OnSite, Panel, Async }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterviewStatus { Scheduled, Completed, Cancelled }

pub struct Offer {
    pub id: Uuid,
    pub application_id: Uuid,
    pub base_salary: Option<i64>,
    pub equity_pct: Option<f64>,
    pub equity_cliff_months: Option<u32>,
    pub equity_vest_months: Option<u32>,
    pub signing_bonus: Option<i64>,
    pub annual_bonus_target_pct: Option<f64>,
    pub benefits_notes: Option<String>,
    pub offer_date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub status: OfferStatus,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OfferStatus { Pending, Countered, Accepted, Declined }
```

```rust
// lazyjob-core/src/application/repository.rs

#[async_trait]
pub trait ApplicationRepository: Send + Sync {
    async fn insert(&self, app: &Application) -> Result<Uuid>;
    async fn get(&self, id: &Uuid) -> Result<Application>;
    async fn list(&self, filter: &ApplicationFilter) -> Result<Vec<Application>>;
    async fn update_stage(
        &self,
        id: &Uuid,
        new_stage: ApplicationStage,
        reason: Option<&str>,
    ) -> Result<()>;
    async fn update_notes(&self, id: &Uuid, notes: &str) -> Result<()>;
    async fn update_follow_up(&self, id: &Uuid, at: Option<DateTime<Utc>>) -> Result<()>;
    async fn list_transitions(&self, app_id: &Uuid) -> Result<Vec<StageTransition>>;
    async fn count_by_stage(&self) -> Result<HashMap<ApplicationStage, usize>>;
}

pub struct ApplicationFilter {
    pub stages: Option<Vec<ApplicationStage>>,
    pub job_id: Option<Uuid>,
    pub since: Option<DateTime<Utc>>,
    pub active_only: bool,
}
```

### Transition Enforcement

The `update_stage` repository method must enforce the transition matrix at the database-operation level — it calls `from_stage.can_transition_to(to_stage)` before writing. This prevents bypass through direct DB calls and keeps validation in one place (`ApplicationStage::can_transition_to`).

## Interface

See Rust types above. The repository trait is the entire public interface for this spec. Domain workflows (see `application-workflow-actions.md`) use the repository; they do not call SQLite directly.

## Open Questions

- **Async challenge tracking**: Some technical interviews are async coding challenges (HackerRank, Karat) with explicit deadlines. Should `Technical` have a sub-status for "challenge sent / challenge submitted"? Or is this a note + follow-up reminder?
- **Multi-role applications**: A user applying to two different roles at the same company should have two distinct `Application` records (different `job_id`). But they share `application_contacts`. Should contacts be linked to both applications or only the first?
- **Offer expiry deadlines**: The `offers.expiry_date` field needs to surface in the TUI's "Action Required" queue. This is a pipeline-metrics concern — note for the metrics spec.

## Implementation Tasks

- [ ] Define `ApplicationStage` enum with `can_transition_to` in `lazyjob-core/src/application/stage.rs`
- [ ] Define `Application`, `StageTransition`, `Interview`, `Offer` structs in `lazyjob-core/src/application/model.rs`
- [ ] Write `002_applications.sql` migration with `applications`, `application_transitions`, `application_contacts`, `interviews`, `offers` tables
- [ ] Implement `SqliteApplicationRepository` in `lazyjob-core/src/application/sqlite.rs` with transition validation enforced at `update_stage`
- [ ] Add `ApplicationRepository` trait to `lazyjob-core/src/application/repository.rs`
- [ ] Add `ApplicationFilter` struct supporting stage/job_id/since/active_only filters
- [ ] Expose `Application` module from `lazyjob-core/src/lib.rs`
