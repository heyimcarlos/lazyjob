# Application Workflow

## Status
Researching

## Problem Statement

Job applications follow a complex state machine with many stages, multiple contacts, and rich metadata. Users need to:
1. Track where each application stands
2. Know what actions to take next
3. Maintain a history of all interactions
4. Get reminders for follow-ups
5. Understand their overall pipeline health

This spec defines the application state machine, the human-in-the-loop design, and the workflow for taking actions.

---

## Research Findings

### Application Tracking Patterns

**Linear Pipeline (Most ATS)**:
```
Applied → Phone Screen → Technical → On-site → Offer → Hired
              ↓            ↓          ↓
          Rejected      Rejected    Rejected
```

**Extended Pipeline**:
```
Discovered → Interested → Applied → Phone Screen → Technical → On-site → Offer → Accepted/Rejected
    ↓           ↓           ↓
 Withdrawn   Withdrawn   Withdrawn
```

### Key States

1. **Discovered**: Job found, not yet reviewed
2. **Interested**: User marked as interested
3. **Applied**: Application submitted
4. **Phone Screen**: Recruiter call scheduled/completed
5. **Technical**: Technical interview scheduled/completed
6. **On-site**: Final rounds scheduled/completed
7. **Offer**: Offer received
8. **Accepted**: Offer accepted
9. **Rejected**: Not moving forward (from any stage)
10. **Withdrawn**: User withdrew application

### Huntr / Linear Patterns

**Huntr** (Job tracking):
- Kanban board with drag-and-drop
- Each card shows: company, title, status, last contact, salary
- Quick actions on cards
- Notes per job

**Linear** (Issue tracking):
- Swimlanes by status
- Time-based filters
- Multiple views (board, list, calendar)
- Custom fields

---

## State Machine

### Application State

```rust
// lazyjob-core/src/application/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn to_string(&self) -> &'static str {
        match self {
            ApplicationStage::Discovered => "Discovered",
            ApplicationStage::Interested => "Interested",
            ApplicationStage::Applied => "Applied",
            ApplicationStage::PhoneScreen => "Phone Screen",
            ApplicationStage::Technical => "Technical",
            ApplicationStage::OnSite => "On-site",
            ApplicationStage::Offer => "Offer",
            ApplicationStage::Accepted => "Accepted",
            ApplicationStage::Rejected => "Rejected",
            ApplicationStage::Withdrawn => "Withdrawn",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ApplicationStage::Accepted | ApplicationStage::Rejected | ApplicationStage::Withdrawn
        )
    }

    pub fn can_transition_to(&self, next: &ApplicationStage) -> bool {
        use ApplicationStage::*;

        // Terminal states don't transition
        if self.is_terminal() {
            return false;
        }

        match (self, next) {
            // Forward progression
            (Discovered, Interested) => true,
            (Discovered, Applied) => true,  // Skip Interested
            (Interested, Applied) => true,
            (Applied, PhoneScreen) => true,
            (PhoneScreen, Technical) => true,
            (Technical, OnSite) => true,
            (OnSite, Offer) => true,
            (Offer, Accepted) => true,

            // Backward (re-evaluation)
            (Interested, Discovered) => true,
            (Applied, Interested) => true,
            (PhoneScreen, Applied) => true,
            (Technical, PhoneScreen) => true,
            (OnSite, Technical) => true,
            (Offer, OnSite) => true,

            // To terminal states
            (_, Rejected) => true,
            (_, Withdrawn) => !matches!(self, Accepted | Rejected | Withdrawn),

            // Direct to final
            (Applied, OnSite) => true,  // Skip intermediate
            (Applied, Offer) => true,  // Fast track
            (PhoneScreen, Offer) => true,
            (Technical, Offer) => true,

            _ => false,
        }
    }
}

pub struct Application {
    pub id: Uuid,
    pub job_id: Uuid,
    pub stage: ApplicationStage,
    pub stage_history: Vec<StageTransition>,
    pub resume_version: Option<String>,
    pub cover_letter_version: Option<String>,
    pub contacts: Vec<ApplicationContact>,
    pub interviews: Vec<Interview>,
    pub offers: Vec<Offer>,
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_contact_at: Option<DateTime<Utc>>,
    pub next_follow_up: Option<DateTime<Utc>>,
}

pub struct StageTransition {
    pub from: ApplicationStage,
    pub to: ApplicationStage,
    pub transitioned_at: DateTime<Utc>,
    pub reason: Option<String>,
}
```

### State Transitions

```rust
impl Application {
    pub fn transition_to(&mut self, new_stage: ApplicationStage, reason: Option<String>) -> Result<()> {
        if !self.stage.can_transition_to(&new_stage) {
            return Err(ApplicationError::InvalidTransition {
                from: self.stage,
                to: new_stage,
            });
        }

        let transition = StageTransition {
            from: self.stage,
            to: new_stage,
            transitioned_at: Utc::now(),
            reason,
        };

        self.stage_history.push(transition);
        self.stage = new_stage;
        self.updated_at = Utc::now();

        Ok(())
    }
}
```

---

## Workflow Actions

### 1. Apply to Job

```rust
pub struct ApplyWorkflow {
    pub steps: Vec<WorkflowStep>,
}

impl ApplyWorkflow {
    pub async fn execute(
        &self,
        job_id: &Uuid,
        options: ApplyOptions,
    ) -> Result<Application> {
        // Step 1: Check if job already has application
        if let Some(existing) = self.repo.find_by_job(job_id).await? {
            return Err(ApplicationError::AlreadyExists(existing.id));
        }

        // Step 2: Check if user has tailored resume
        let resume = if options.tailor_resume {
            self.tailor_resume(job_id).await?
        } else {
            self.get_latest_resume().await?
        };

        // Step 3: Check if user wants cover letter
        let cover_letter = if options.include_cover_letter {
            Some(self.generate_cover_letter(job_id).await?)
        } else {
            None
        };

        // Step 4: Update job status
        let mut job = self.job_repo.get(job_id).await?;
        job.status = JobStatus::Applied;
        job.applied_at = Some(Utc::now());
        self.job_repo.update(&job).await?;

        // Step 5: Create application
        let application = Application {
            id: Uuid::new_v4(),
            job_id: *job_id,
            stage: ApplicationStage::Applied,
            resume_version: resume.map(|r| r.version_id),
            cover_letter_version: cover_letter.as_ref().map(|c| c.version_id),
            ..Default::default()
        };

        self.repo.insert(&application).await?;

        // Step 6: Log activity
        self.activity_log.log(&Activity {
            entity_type: "application",
            entity_id: application.id,
            action: "created",
            details: serde_json::json!({
                "job_id": job_id,
                "has_resume": resume.is_some(),
                "has_cover_letter": cover_letter.is_some(),
            }),
        }).await?;

        Ok(application)
    }
}
```

### 2. Schedule Interview

```rust
pub struct ScheduleInterviewWorkflow {
    pub steps: Vec<WorkflowStep>,
}

impl ScheduleInterviewWorkflow {
    pub async fn execute(
        &self,
        application_id: &Uuid,
        details: InterviewDetails,
    ) -> Result<Interview> {
        // Step 1: Validate application can have interviews
        let app = self.repo.get(application_id).await?;
        if !matches!(
            app.stage,
            ApplicationStage::PhoneScreen | ApplicationStage::Technical | ApplicationStage::OnSite
        ) {
            return Err(ApplicationError::InvalidStageForAction {
                action: "schedule_interview",
                current_stage: app.stage,
            });
        }

        // Step 2: Determine interview type from stage
        let interview_type = match app.stage {
            ApplicationStage::PhoneScreen => InterviewType::PhoneScreen,
            ApplicationStage::Technical => InterviewType::Technical,
            ApplicationStage::OnSite => InterviewType::OnSite,
            _ => unreachable!(),
        };

        // Step 3: Create interview record
        let interview = Interview {
            id: Uuid::new_v4(),
            application_id: *application_id,
            interview_type,
            scheduled_at: details.scheduled_at,
            duration_minutes: details.duration,
            location: details.location,
            meeting_url: details.meeting_url,
            interviewers: details.interviewer_names,
            status: InterviewStatus::Scheduled,
            ..Default::default()
        };

        self.interview_repo.insert(&interview).await?;

        // Step 4: Update last_contact_at
        self.repo.update_last_contact(application_id, Utc::now()).await?;

        // Step 5: Create reminder
        if let Some(reminder_time) = details.reminder_time {
            self.reminder_service.create(&Reminder {
                title: format!("Interview prep: {}", app.job().title),
                due_at: reminder_time,
                application_id: Some(*application_id),
                ..Default::default()
            }).await?;
        }

        Ok(interview)
    }
}
```

### 3. Move to Next Stage

```rust
pub struct MoveStageWorkflow {
    pub steps: Vec<WorkflowStep>,
}

impl MoveStageWorkflow {
    pub async fn execute(
        &self,
        application_id: &Uuid,
        target_stage: ApplicationStage,
        reason: Option<String>,
    ) -> Result<Application> {
        let mut app = self.repo.get(application_id).await?;

        // Validate transition
        if !app.stage.can_transition_to(&target_stage) {
            return Err(ApplicationError::InvalidTransition {
                from: app.stage,
                to: target_stage,
            });
        }

        // Pre-transition actions
        match target_stage {
            ApplicationStage::Rejected => {
                self.handle_rejection_workflow(&app).await?;
            }
            ApplicationStage::Withdrawn => {
                self.handle_withdrawal_workflow(&app).await?;
            }
            ApplicationStage::Offer => {
                self.handle_offer_received_workflow(&app).await?;
            }
            _ => {}
        }

        // Execute transition
        app.transition_to(target_stage, reason)?;
        self.repo.update(&app).await?;

        // Post-transition actions
        match target_stage {
            ApplicationStage::Applied => {
                self.create_follow_up_reminder(&app, 7)?;  // 7 day follow up
            }
            ApplicationStage::PhoneScreen => {
                self.create_interview_prep_reminder(&app)?;
            }
            ApplicationStage::Offer => {
                self.create_negotiation_reminder(&app)?;
            }
            _ => {}
        }

        // Log activity
        self.activity_log.log_stage_change(&app).await?;

        Ok(app)
    }
}
```

---

## Human-in-the-Loop Design

### Automation Boundaries

**Automated (with confirmation)**:
- Stage transitions (user confirms)
- Reminder creation
- Activity logging
- Resume/cover letter generation

**Not Automated**:
- Actually sending emails/messages
- Scheduling interviews (just creates records)
- Accepting/rejecting offers
- Withdrawing applications

### Confirmation Dialogs

```
┌─────────────────────────────────────────────────┐
│                                                 │
│  Move to Technical Interview?                    │
│                                                 │
│  Current: Phone Screen                          │
│  Next:    Technical                             │
│                                                 │
│  Add notes (optional):                          │
│  [________________________]                     │
│                                                 │
│  [Cancel]              [Confirm Move]          │
│                                                 │
└─────────────────────────────────────────────────┘
```

### Action Required Queue

```
┌─────────────────────────────────────────────────┐
│  📋 Actions Required                             │
├─────────────────────────────────────────────────┤
│                                                 │
│  ⏰ Follow up with Stripe (3d overdue)          │
│     Applied 10 days ago                          │
│     [Send Follow-up]  [Snooze 2d]  [Dismiss]   │
│                                                 │
│  📅 Interview tomorrow: Google SRE               │
│     Technical round at 2pm                      │
│     [Prep Now]  [View Details]  [Reschedule]     │
│                                                 │
│  💼 Offer from Datadog                          │
│     Deadline: 5 days                            │
│     [Negotiate]  [Accept]  [Decline]           │
│                                                 │
└─────────────────────────────────────────────────┘
```

---

## Dashboard Metrics

```rust
pub struct PipelineMetrics {
    pub total_applications: usize,
    pub by_stage: HashMap<ApplicationStage, usize>,
    pub response_rate: f32,           // % of applications with any response
    pub interview_rate: f32,           // % that reached interview stage
    pub offer_rate: f32,              // % that received offers
    pub acceptance_rate: f32,          // % of offers accepted
    pub avg_time_in_stage: HashMap<ApplicationStage, Duration>,
    pub active_applications: usize,
    pub stale_applications: usize,    // No contact in 14+ days
}

impl PipelineMetrics {
    pub fn calculate(applications: &[Application]) -> Self {
        // Calculate all metrics from applications list
    }
}
```

---

## API Surface

```rust
// lazyjob-core/src/application/service.rs

#[cfg_attr(async_trait::async_trait, async_trait)]
pub trait ApplicationService {
    async fn create(&self, job_id: &Uuid) -> Result<Application>;
    async fn get(&self, id: &Uuid) -> Result<Application>;
    async fn list(&self, filter: &ApplicationFilter) -> Result<Vec<Application>>;
    async fn list_by_stage(&self, stage: ApplicationStage) -> Result<Vec<Application>>;
    async fn update_stage(&self, id: &Uuid, new_stage: ApplicationStage, reason: Option<String>) -> Result<Application>;
    async fn add_note(&self, id: &Uuid, note: &str) -> Result<()>;
    async fn archive(&self, id: &Uuid) -> Result<()>;
    async fn get_metrics(&self) -> Result<PipelineMetrics>;
}
```

---

## Failure Modes

1. **Invalid Transition**: Tried to skip stages; return error with valid transitions
2. **Missing Resume**: Warn user before applying without tailored resume
3. **Stale Application**: Flag applications with no contact in 14+ days
4. **Duplicate Application**: Prevent applying to same job twice
5. **Lost Interview Details**: Store all details, provide reminders

---

## Open Questions

1. **Application Deadline Tracking**: Should we track when company expects response?
2. **Offer Comparison**: Should we have a structured way to compare multiple offers?
3. **Bulk Actions**: Can users apply to multiple similar jobs at once?
4. **Application Templates**: Reusable application packages for similar roles?

---

## Sources

- [Huntr Job Search Tracker](https://huntr.com/)
- [Linear Issue Tracking](https://linear.app/)
- [Greenhouse Application Pipeline](https://www.greenhouse.io/)
