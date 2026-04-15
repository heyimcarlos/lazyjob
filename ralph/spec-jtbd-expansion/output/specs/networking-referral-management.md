# Spec: Networking Referral Management

**JTBD**: A-4 — Get warm introductions that beat cold applications
**Topic**: Track the relationship maintenance lifecycle for each networking contact to identify the right moment to request a referral
**Domain**: networking

---

## What

A lightweight CRM lifecycle tracker that models the relationship-warming arc for contacts at target companies, emits timely reminders to follow up or deepen relationships, identifies when a contact is ready to be asked for a referral, and records referral outcomes. It operates as a background poller (tokio task) that watches `profile_contacts` and emits `WorkflowEvent::NetworkingReminderDue` events to the TUI.

## Why

The timing of a referral ask determines whether it succeeds. Asking too early (before any interaction) produces near-zero conversion. Asking too late (role has closed, relationship has decayed) wastes goodwill on a dead opportunity. Most job seekers' professional networks decay because they can't track everyone simultaneously — relationships with people at target companies erode through inaction. An automated relationship-maintenance loop solves the tracking burden while keeping the human in every interaction.

The data supports investment here: referrals are 7–18x more likely to result in a hire. Sourced candidates (which includes referral-facilitated applications) are 5x more likely to be hired than inbound applicants. 44% of sourced hires in 2024 came from talent rediscovery — people already known to the company. Maintaining live relationships in a company's employee network keeps the user in this high-value pipeline.

## How

### Relationship Stage Machine

`RelationshipStage` is a linear state machine progressing from initial discovery to referral resolution:

```
Identified → Contacted → Replied → Warmed → ReferralAsked → ReferralResolved
```

| Stage | Description | Transition trigger |
|---|---|---|
| `Identified` | Contact found via connection mapping; no outreach yet | Auto on `warm_paths_for_job` result |
| `Contacted` | First outreach message drafted and marked sent | User action: marks message sent |
| `Replied` | Contact responded to outreach | User action: marks response received |
| `Warmed` | At least one substantive interaction (call, further exchange) | User action: logs interaction |
| `ReferralAsked` | User has asked for a referral for a specific job | User action: marks referral asked |
| `ReferralResolved` | Referral outcome known (succeeded, declined, no response) | User action or application stage event |

This is tracked per-contact, not per-job. A contact can be `Warmed` for multiple target companies. The `referral_asks` table tracks per-(contact, job) referral request state so one contact can be tracked for multiple roles without data collisions.

### Reminder Poller

`NetworkingReminderPoller` is a tokio background task in `lazyjob-ralph/src/networking_poller.rs` that runs on a configurable interval (default: once daily, configurable in `lazyjob.toml` as `networking.reminder_interval_hours`).

Each run:
1. Queries all `profile_contacts` in stages `Contacted`, `Warmed`.
2. For each contact, computes `days_since_last_interaction`.
3. Emits `WorkflowEvent::NetworkingReminderDue { contact_id, reason }` if:
   - Stage is `Contacted` and days_since > 7 (follow up on no response)
   - Stage is `Contacted` and days_since > 14 (second and final follow-up reminder)
   - Stage is `Replied` and no interaction logged in 21 days (relationship cooling)
   - Stage is `Warmed` and a fresh active job at their company exists and `ReferralReadinessChecker` returns ready
4. Anti-spam gate: no more than **2 reminder events** per contact per 30-day window. After 2 unanswered follow-ups, poller stops reminding for that contact (set `follow_up_exhausted = true`).

The TUI subscribes to `WorkflowEvent` via the shared broadcast channel (established in `application-workflow-actions.md`) and shows networking reminders in the morning digest alongside application reminders.

### Referral Readiness Checker

`ReferralReadinessChecker` in `lazyjob-core/src/networking/referral_readiness.rs` evaluates whether a contact is appropriate to ask for a referral for a specific job:

**Readiness criteria** (all must pass):
1. `contact.relationship_stage` is `Warmed` (at least one replied interaction)
2. `contact.last_interaction_at` ≥ 7 days ago and ≤ 180 days ago (relationship is warm but not stale)
3. Job is active: `job.status != Closed` and `ghost_score < 0.6` (ghost score from `GhostDetector`)
4. No previous referral ask for this (contact, job) pair in `referral_asks` table
5. User has not already applied to this job through another channel (`application.stage != Applied`)

**Readiness output**: `ReferralReadiness::Ready` with timing recommendation (ask now, wait N days, or skip with reason).

### Referral Outcome Integration

When an `Application.stage` transitions to `Offered` or `Rejected` (via `ApplicationStateMachine`), the system emits a `PostTransitionSuggestion::UpdateReferralOutcome { application_id }`. The TUI shows a prompt asking the user to record the referral outcome:

- `ReferralOutcome::Succeeded` — contact referred user; used to reinforce relationship importance
- `ReferralOutcome::Declined` — contact declined; relationship continues but referral track closed for this role
- `ReferralOutcome::NoResponse` — no answer to referral ask within 21 days
- `ReferralOutcome::NotApplicable` — application progressed without referral (e.g., applied direct)

This outcome is written to `referral_asks.outcome` and feeds future `ReferralReadinessChecker` calibration.

### Anti-Spam Guardrails (Product Policy)

These are product constraints, not implementation preferences:

1. **Never suggest asking for a referral before at least one replied interaction.** Asking someone who hasn't responded to your first message is not a referral ask — it's spam.
2. **Cap follow-up reminders at 2 per contact per role.** After 2 unanswered follow-ups, mark `follow_up_exhausted = true` and stop reminding.
3. **Never suggest a referral ask for a ghost-detected job.** Asking a contact to spend their social capital on a job that likely won't result in a hire damages the relationship.
4. **Never suggest outreach to more than 5 new contacts per week** (configurable cap in `lazyjob.toml` as `networking.max_new_outreach_per_week`). The cap protects both the user's professional reputation and the platform's quality-over-volume positioning.

## Interface

```rust
// lazyjob-core/src/networking/relationship.rs

pub enum RelationshipStage {
    Identified,
    Contacted,
    Replied,
    Warmed,
    ReferralAsked,
    ReferralResolved,
}

pub struct ReferralAsk {
    pub id: Uuid,
    pub contact_id: Uuid,
    pub job_id: Uuid,
    pub asked_at: NaiveDate,
    pub outcome: Option<ReferralOutcome>,
    pub outcome_recorded_at: Option<NaiveDate>,
}

pub enum ReferralOutcome {
    Succeeded,
    Declined,
    NoResponse,
    NotApplicable,
}

pub enum ReferralReadiness {
    Ready { recommended_ask_date: NaiveDate },
    NotYet { reason: NotYetReason, suggested_wait_days: u32 },
    Skip { reason: SkipReason },
}

pub enum NotYetReason {
    RelationshipTooNew,
    NeedMoreInteractions,
    TooSoonAfterLastContact,
}

pub enum SkipReason {
    GhostJobDetected,
    ReferralAlreadyAsked,
    AlreadyApplied,
    JobClosed,
    FollowUpExhausted,
}

pub struct NetworkingReminder {
    pub contact_id: Uuid,
    pub contact_name: String,
    pub company_name: String,
    pub current_stage: RelationshipStage,
    pub days_since_last_interaction: u32,
    pub suggested_action: NetworkingReminderAction,
}

pub enum NetworkingReminderAction {
    SendFollowUp,
    LogInteraction,
    AskForReferral { job_id: Uuid, job_title: String },
    RecordOutcome { referral_ask_id: Uuid },
}

// lazyjob-ralph/src/networking_poller.rs

pub struct NetworkingReminderPoller {
    contact_repo: Arc<dyn ContactRepository>,
    referral_readiness: Arc<ReferralReadinessChecker>,
    event_tx: broadcast::Sender<WorkflowEvent>,
    reminder_interval: Duration,
    max_reminders_per_contact_per_month: u8,  // default 2
}

impl NetworkingReminderPoller {
    pub fn spawn(self) -> JoinHandle<()>;
    async fn run_sweep(&self) -> Result<usize>;  // returns count of events emitted
}
```

```sql
-- Migration: relationship stage tracking on profile_contacts
ALTER TABLE profile_contacts
  ADD COLUMN relationship_stage TEXT NOT NULL DEFAULT 'identified',
  ADD COLUMN interaction_count  INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN follow_up_exhausted BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN reminder_count_this_month INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN reminder_window_start DATE;

-- New table: per-(contact, job) referral tracking
CREATE TABLE referral_asks (
    id                  TEXT PRIMARY KEY,    -- UUID
    contact_id          TEXT NOT NULL REFERENCES profile_contacts(id),
    job_id              TEXT NOT NULL REFERENCES jobs(id),
    asked_at            DATE NOT NULL,
    outcome             TEXT,                -- NULL until resolved
    outcome_recorded_at DATE,
    notes               TEXT,
    created_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(contact_id, job_id)
);
```

## Open Questions

- **Interaction logging granularity**: Should the user log every interaction (message, call, coffee chat) or just binary (yes I interacted / no I didn't)? Recommendation: simple interaction log with date + optional note. Avoid requiring detailed logging — friction kills adoption.
- **Relationship strength score**: Should LazyJob compute a numerical relationship strength score (like SSI) based on interaction frequency and recency? This would enable better `ReferralReadinessChecker` calibration. Deferred to Phase 2 — Phase 1 relies on stage machine only.
- **Ghost job check in referral readiness**: `ReferralReadinessChecker` calls `GhostDetector` to verify the job is real before suggesting the referral ask. Both are in `lazyjob-core`. If the ghost score is borderline (0.4–0.6), should the user be warned but not blocked? Recommendation: warn at 0.4–0.6, block at >0.6.
- **Contact duplication across import sources**: If a user manually enters a contact and later imports the same person from a LinkedIn CSV, they should be merged by email. If no email overlap, the duplicate is silent. Should the TUI show a deduplication suggestion? Phase 2.

## Implementation Tasks

- [ ] Add `relationship_stage`, `interaction_count`, `follow_up_exhausted`, `reminder_count_this_month` columns to `profile_contacts` DDL in `lazyjob-core/src/db/schema.sql`
- [ ] Create `referral_asks` table DDL with `(contact_id, job_id)` unique constraint
- [ ] Implement `ReferralReadinessChecker` in `lazyjob-core/src/networking/referral_readiness.rs` with all 5 readiness criteria and `GhostDetector` integration
- [ ] Implement `NetworkingReminderPoller` as a tokio background task in `lazyjob-ralph/src/networking_poller.rs` with configurable interval and 2-reminder anti-spam cap
- [ ] Wire `WorkflowEvent::NetworkingReminderDue` into the TUI's broadcast channel subscriber (same channel as `ReminderPoller` from `application-workflow-actions.md`)
- [ ] Wire `PostTransitionSuggestion::UpdateReferralOutcome` dispatch in `lazyjob-ralph/src/dispatch.rs` when application stage transitions to `Offered` or `Rejected`
- [ ] Add networking dashboard TUI view (`lazyjob-tui/src/views/networking/dashboard.rs`): contacts grouped by company with stage badge, reminder count, days-since-contact indicator
- [ ] Add interaction logging action to TUI contact detail view: `l` to log interaction (date + note), updates `interaction_count`, `last_contacted_at`, advances stage from `Contacted` → `Replied` or `Replied` → `Warmed`
