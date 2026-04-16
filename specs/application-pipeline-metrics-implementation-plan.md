# Implementation Plan: Application Pipeline Metrics & Notifications

## Status
Draft

## Related Spec
[`specs/application-pipeline-metrics.md`](./application-pipeline-metrics.md)

## Overview

The pipeline metrics layer transforms raw application state data stored in SQLite into health signals that a job seeker can act on. Rather than building a background aggregation job (unnecessary at single-user scale), all metrics are computed fresh on demand via SQL queries and a lightweight Rust computation pass over `ApplicationRepository`. This keeps the implementation simple, avoids cache invalidation bugs, and is fast enough for <500 applications on any modern machine.

The system has four interlocking pieces: (1) `MetricsService` computes and returns the full `PipelineMetrics` value type on each call; (2) `ActionItemService` aggregates the action-required queue — overdue follow-ups, stale applications, expiring offers, upcoming interviews, and new rejections — sorted by urgency; (3) `ReminderPoller` runs as a background tokio task that wakes every 5 minutes to fire due reminders as `WorkflowEvent::ReminderDue` broadcast messages; and (4) `DigestService` generates a daily plain-text summary printed to stdout before the TUI launches on the first invocation of each day.

Two TUI views complement the services: an action-required queue view (`ActionQueueView`) that shows a prioritized, interactive inbox with per-item keybindings, and a metrics dashboard view (`PipelineMetricsView`) showing a funnel chart, stage distribution bar chart, stale application table, and velocity table — all rendered via ratatui widgets.

## Prerequisites

### Specs / Plans That Must Be Implemented First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, migrations, `sqlx::Pool<Sqlite>`
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `Application`, `ApplicationRepository`, `application_transitions` table
- `specs/application-workflow-actions-implementation-plan.md` — `WorkflowEvent`, `Reminder`, `ReminderRepository`, `Offer`, `OfferRepository`, `Interview`, `InterviewRepository`
- `specs/09-tui-design-keybindings-implementation-plan.md` — `App`, `EventLoop`, `KeyContext`, `Action`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml (additions)
[dependencies]
sqlx      = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono"] }
tokio     = { version = "1", features = ["time", "sync", "macros"] }
chrono    = { version = "0.4", features = ["serde"] }
serde     = { version = "1", features = ["derive"] }
thiserror = "2"
anyhow    = "1"
tracing   = "0.1"

# lazyjob-tui/Cargo.toml (additions)
[dependencies]
ratatui = "0.29"
```

---

## Architecture

### Crate Placement

| Component | Crate | Reason |
|---|---|---|
| `PipelineMetrics` struct | `lazyjob-core` | Pure domain value, shared with CLI digest |
| `ActionItem` enum | `lazyjob-core` | Shared by TUI action queue and digest |
| `MetricsService` | `lazyjob-core` | I/O-bearing computation, aggregates multiple repos |
| `ActionItemService` | `lazyjob-core` | Derives action queue from repos + metrics |
| `DigestService` | `lazyjob-core` | Read-only morning digest generation |
| `ReminderPoller` | `lazyjob-core` | Background tokio task for firing reminders |
| `MetricsError` | `lazyjob-core` | Public error enum for this module |
| `ActionQueueView` widget | `lazyjob-tui` | TUI action queue interactive panel |
| `PipelineMetricsView` widget | `lazyjob-tui` | TUI metrics dashboard |

### Core Types

```rust
// lazyjob-core/src/application/metrics.rs

use std::collections::HashMap;
use chrono::{DateTime, NaiveDate, Utc};
use crate::application::stage::ApplicationStage;

pub struct PipelineMetrics {
    // Volume
    pub total_applications: usize,
    pub active_applications: usize,    // stages that are NOT terminal
    pub terminal_applications: usize,  // Accepted | Rejected | Withdrawn

    // Per-stage counts (all stages, including terminal)
    pub by_stage: HashMap<ApplicationStage, usize>,

    // Funnel conversion rates
    // Denominator: Applied + PhoneScreen + Technical + OnSite + Offer + Accepted + Rejected
    // (Discovered and Interested excluded — application not yet submitted)
    pub response_rate: f32,     // (PhoneScreen + Technical + OnSite + Offer + Accepted) / denom
    pub interview_rate: f32,    // (Technical + OnSite + Offer + Accepted) / denom
    pub offer_rate: f32,        // (Offer + Accepted) / denom
    pub acceptance_rate: f32,   // Accepted / (Offer + Accepted); 0.0 if denom is 0

    // Staleness
    pub stale_applications: usize,  // active apps with last_contact_at > stale_threshold
    pub no_contact_ever: usize,     // Applied+ but last_contact_at IS NULL

    // Velocity — None until ≥3 data points for that stage
    pub avg_days_in_stage: HashMap<ApplicationStage, Option<f32>>,
    pub median_days_to_response: Option<f32>,  // Applied → PhoneScreen median

    // Action items (derived counts, authoritative in ActionItemService)
    pub overdue_follow_ups: usize,    // reminders past due_at and not fired
    pub expiring_offers: usize,       // offers with expiry_date within 5 days

    pub computed_at: DateTime<Utc>,
}

// lazyjob-core/src/application/action_item.rs

use crate::application::{Application, Interview, Offer, Reminder};

#[derive(Debug, Clone)]
pub enum ActionItem {
    ExpiringOffer {
        offer: Offer,
        application: Application,
        days_remaining: u32,
    },
    UpcomingInterview {
        interview: Interview,
        application: Application,
        hours_until: i64,
    },
    OverdueFollowUp {
        reminder: Reminder,
        application: Application,
    },
    StaleApplication {
        application: Application,
        days_stale: u32,
    },
    NewRejection {
        application: Application,
    },
}

impl ActionItem {
    /// Sort priority: lower = more urgent. Used for stable ordering in the action queue.
    pub fn priority(&self) -> u8 {
        match self {
            Self::ExpiringOffer { .. }     => 0,
            Self::UpcomingInterview { .. } => 1,
            Self::OverdueFollowUp { .. }   => 2,
            Self::StaleApplication { .. }  => 3,
            Self::NewRejection { .. }      => 4,
        }
    }
}
```

```rust
// lazyjob-core/src/application/digest.rs

use chrono::NaiveDate;
use crate::application::action_item::ActionItem;

pub struct DailyDigest {
    pub date: NaiveDate,
    pub new_job_matches: usize,
    pub strong_matches: usize,   // relevance_score >= 0.75
    pub active_applications: usize,
    pub action_items: Vec<ActionItem>,
    pub response_rate: f32,
    pub tips: Vec<String>,       // contextual tips if response_rate < 5.0%
    pub privacy_mode: bool,      // if true, company names redacted
}

impl DailyDigest {
    /// Renders a human-readable multi-line string for stdout.
    pub fn render(&self) -> String { /* ... */ }
}

pub struct DigestService {
    metrics_svc: Arc<MetricsService>,
    action_svc: Arc<ActionItemService>,
    job_repo: Arc<dyn JobRepository>,
    digest_repo: Arc<dyn DigestRepository>,
    preferences: UserPreferences,
}

impl DigestService {
    /// Returns true if digest should be shown today (not yet shown since midnight local).
    pub fn should_show_today(&self) -> bool;

    /// Generates the digest. Does NOT call mark_shown() — caller does that after printing.
    pub async fn generate_daily_digest(&self) -> Result<DailyDigest, MetricsError>;

    /// Records that the digest was shown today. Idempotent (INSERT OR IGNORE).
    pub async fn mark_shown(&self) -> Result<(), MetricsError>;
}
```

```rust
// lazyjob-core/src/application/reminders.rs

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use crate::application::repository::ReminderRepository;
use crate::application::workflow::WorkflowEvent;

pub struct ReminderPoller {
    repo: Arc<dyn ReminderRepository>,
    sender: broadcast::Sender<WorkflowEvent>,
}

impl ReminderPoller {
    pub fn new(
        repo: Arc<dyn ReminderRepository>,
        sender: broadcast::Sender<WorkflowEvent>,
    ) -> Self;

    /// Spawns itself as a background tokio task. Returns the JoinHandle.
    pub fn spawn(self) -> tokio::task::JoinHandle<()>;

    async fn run(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            // fire due reminders
        }
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/application/repository.rs (additions)

#[async_trait::async_trait]
pub trait ReminderRepository: Send + Sync + 'static {
    async fn list_pending(&self, before: DateTime<Utc>) -> Result<Vec<Reminder>, MetricsError>;
    async fn mark_fired(&self, id: &ReminderId, at: DateTime<Utc>) -> Result<(), MetricsError>;
    async fn snooze(
        &self,
        application_id: &ApplicationId,
        until: DateTime<Utc>,
        reason: Option<&str>,
    ) -> Result<(), MetricsError>;
}

#[async_trait::async_trait]
pub trait OfferRepository: Send + Sync + 'static {
    async fn list_expiring(&self, within_days: u32) -> Result<Vec<Offer>, MetricsError>;
    async fn get_for_application(&self, application_id: &ApplicationId) -> Result<Option<Offer>, MetricsError>;
}

#[async_trait::async_trait]
pub trait InterviewRepository: Send + Sync + 'static {
    async fn list_upcoming(&self, within_hours: i64) -> Result<Vec<Interview>, MetricsError>;
    async fn get_for_application(&self, application_id: &ApplicationId) -> Result<Vec<Interview>, MetricsError>;
}

#[async_trait::async_trait]
pub trait DigestRepository: Send + Sync + 'static {
    async fn has_shown_today(&self) -> Result<bool, MetricsError>;
    async fn record_shown(&self, date: chrono::NaiveDate) -> Result<(), MetricsError>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/007_metrics.sql

-- Track last digest shown per day. INSERT OR IGNORE prevents double-show.
CREATE TABLE IF NOT EXISTS digest_log (
    date     TEXT PRIMARY KEY,          -- 'YYYY-MM-DD' local date
    shown_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Soft snooze for stale applications.
-- An application is NOT stale if its most recent snooze has snoozed_until > now().
CREATE TABLE IF NOT EXISTS application_snoozes (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    snoozed_until  TEXT NOT NULL,        -- ISO8601 datetime
    reason         TEXT,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (application_id, created_at)
);

-- Partial index: efficiently find active snoozes
CREATE INDEX IF NOT EXISTS idx_snoozes_active
    ON application_snoozes(application_id, snoozed_until)
    WHERE snoozed_until > datetime('now');
```

The `reminders` and `interviews` tables are defined in `006_workflow_actions.sql` (see `application-workflow-actions-implementation-plan.md`).

### Module Structure

```
lazyjob-core/
  src/
    application/
      mod.rs           -- re-exports: Application, ApplicationStage, PipelineMetrics, ActionItem, ...
      stage.rs         -- ApplicationStage enum + can_transition_to
      model.rs         -- Application, Interview, Offer, Reminder structs
      repository.rs    -- ApplicationRepository + extension traits
      metrics.rs       -- PipelineMetrics struct + MetricsService
      action_item.rs   -- ActionItem enum + ActionItemService
      digest.rs        -- DailyDigest + DigestService
      reminders.rs     -- ReminderPoller
      queries.rs       -- raw SQL constants (stale query, velocity query, etc.)
  migrations/
    007_metrics.sql    -- digest_log, application_snoozes

lazyjob-tui/
  src/
    views/
      action_queue.rs       -- ActionQueueView widget
      pipeline_metrics.rs   -- PipelineMetricsView widget (funnel + bar chart + tables)
    widgets/
      funnel_chart.rs       -- custom ratatui Widget for horizontal funnel visualization
      sparkline_row.rs      -- optional: trend sparkline per stage

lazyjob-cli/
  src/
    main.rs            -- digest check + print before TUI launch
```

---

## Implementation Phases

### Phase 1 — Core Metrics Computation (MVP)

**Goal:** `MetricsService::compute()` returns a fully populated `PipelineMetrics` from SQLite.

#### Step 1.1 — Add `migrations/007_metrics.sql`

Create `lazyjob-core/migrations/007_metrics.sql` with the DDL above. Apply via the existing `sqlx::migrate!` macro call in `Database::connect()`.

**Verification:** `sqlx migrate run` succeeds; tables appear in `sqlite3 ~/.lazyjob/data.db .tables`.

#### Step 1.2 — Add SQL constants in `queries.rs`

```rust
// lazyjob-core/src/application/queries.rs

pub const STALE_APPLICATIONS: &str = r#"
SELECT a.*
FROM applications a
LEFT JOIN (
    SELECT application_id, MAX(snoozed_until) as max_snooze
    FROM application_snoozes
    GROUP BY application_id
) s ON s.application_id = a.id
WHERE
    a.stage NOT IN ('Accepted', 'Rejected', 'Withdrawn', 'Discovered', 'Interested')
    AND (
        a.last_contact_at IS NULL
        OR a.last_contact_at < datetime('now', '-' || :stale_days || ' days')
    )
    AND (s.max_snooze IS NULL OR s.max_snooze < datetime('now'))
ORDER BY a.last_contact_at ASC NULLS FIRST
"#;

pub const COUNT_BY_STAGE: &str = r#"
SELECT stage, COUNT(*) as cnt
FROM applications
GROUP BY stage
"#;

pub const VELOCITY_BY_STAGE: &str = r#"
SELECT
    from_stage,
    COUNT(*) as transitions,
    AVG(julianday(transitioned_at) - julianday(prev_transitioned_at)) as avg_days
FROM (
    SELECT
        application_id,
        from_stage,
        transitioned_at,
        LAG(transitioned_at) OVER (
            PARTITION BY application_id ORDER BY transitioned_at
        ) as prev_transitioned_at
    FROM application_transitions
)
WHERE prev_transitioned_at IS NOT NULL
GROUP BY from_stage
"#;

pub const MEDIAN_DAYS_APPLIED_TO_PHONE: &str = r#"
WITH response_times AS (
    SELECT
        a.application_id,
        julianday(a.transitioned_at) - julianday(b.transitioned_at) as days
    FROM application_transitions a
    JOIN application_transitions b ON a.application_id = b.application_id
    WHERE a.to_stage = 'PhoneScreen'
      AND b.to_stage = 'Applied'
)
SELECT days
FROM response_times
ORDER BY days
LIMIT 1 OFFSET (SELECT COUNT(*)/2 FROM response_times)
"#;
```

**Verification:** Each query runs with `sqlx::query!` in a `#[sqlx::test]` harness.

#### Step 1.3 — Implement `MetricsService::compute()`

```rust
// lazyjob-core/src/application/metrics.rs

use std::collections::HashMap;
use std::sync::Arc;
use chrono::Utc;
use sqlx::{Pool, Sqlite};
use crate::application::stage::ApplicationStage;
use crate::application::queries::{COUNT_BY_STAGE, VELOCITY_BY_STAGE, MEDIAN_DAYS_APPLIED_TO_PHONE};
use super::{PipelineMetrics, MetricsError};

pub struct MetricsService {
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) app_repo: Arc<dyn ApplicationRepository>,
    pub(crate) reminder_repo: Arc<dyn ReminderRepository>,
    pub(crate) offer_repo: Arc<dyn OfferRepository>,
    pub(crate) interview_repo: Arc<dyn InterviewRepository>,
    pub(crate) stale_days: u32,  // from UserPreferences, default 14
}

impl MetricsService {
    pub async fn compute(&self) -> Result<PipelineMetrics, MetricsError> {
        // 1. Count by stage
        let rows = sqlx::query!(COUNT_BY_STAGE)
            .fetch_all(&self.pool)
            .await?;
        let by_stage: HashMap<ApplicationStage, usize> = rows
            .iter()
            .filter_map(|r| {
                let stage: ApplicationStage = r.stage.parse().ok()?;
                Some((stage, r.cnt as usize))
            })
            .collect();

        // 2. Total / active / terminal counts
        let terminal = [ApplicationStage::Accepted, ApplicationStage::Rejected, ApplicationStage::Withdrawn];
        let terminal_applications = terminal.iter().map(|s| by_stage.get(s).copied().unwrap_or(0)).sum();
        let total_applications: usize = by_stage.values().sum();
        let active_applications = total_applications - terminal_applications;

        // 3. Funnel rates
        let applied_stages = [
            ApplicationStage::Applied,
            ApplicationStage::PhoneScreen,
            ApplicationStage::Technical,
            ApplicationStage::OnSite,
            ApplicationStage::Offer,
            ApplicationStage::Accepted,
            ApplicationStage::Rejected,
        ];
        let denom: usize = applied_stages.iter().map(|s| by_stage.get(s).copied().unwrap_or(0)).sum();
        let responded = [ApplicationStage::PhoneScreen, ApplicationStage::Technical, ApplicationStage::OnSite, ApplicationStage::Offer, ApplicationStage::Accepted]
            .iter().map(|s| by_stage.get(s).copied().unwrap_or(0)).sum::<usize>();

        let response_rate  = if denom > 0 { responded as f32 / denom as f32 } else { 0.0 };
        let interview_count = [ApplicationStage::Technical, ApplicationStage::OnSite, ApplicationStage::Offer, ApplicationStage::Accepted]
            .iter().map(|s| by_stage.get(s).copied().unwrap_or(0)).sum::<usize>();
        let interview_rate = if denom > 0 { interview_count as f32 / denom as f32 } else { 0.0 };
        let offer_count = [ApplicationStage::Offer, ApplicationStage::Accepted]
            .iter().map(|s| by_stage.get(s).copied().unwrap_or(0)).sum::<usize>();
        let offer_rate = if denom > 0 { offer_count as f32 / denom as f32 } else { 0.0 };
        let accepted = by_stage.get(&ApplicationStage::Accepted).copied().unwrap_or(0);
        let acceptance_rate = if offer_count > 0 { accepted as f32 / offer_count as f32 } else { 0.0 };

        // 4. Staleness
        let stale_list = self.list_stale(self.stale_days).await?;
        let stale_applications = stale_list.len();
        let no_contact_ever = self.count_no_contact_ever().await?;

        // 5. Velocity
        let avg_days_in_stage = self.compute_velocity().await?;
        let median_days_to_response = self.compute_median_response().await?;

        // 6. Action-required counts
        let overdue_follow_ups = self.reminder_repo
            .list_pending(Utc::now()).await?.len();
        let expiring_offers = self.offer_repo
            .list_expiring(5).await?.len();

        Ok(PipelineMetrics {
            total_applications,
            active_applications,
            terminal_applications,
            by_stage,
            response_rate,
            interview_rate,
            offer_rate,
            acceptance_rate,
            stale_applications,
            no_contact_ever,
            avg_days_in_stage,
            median_days_to_response,
            overdue_follow_ups,
            expiring_offers,
            computed_at: Utc::now(),
        })
    }

    pub async fn list_stale(&self, threshold_days: u32) -> Result<Vec<Application>, MetricsError> {
        // Execute STALE_APPLICATIONS query with :stale_days = threshold_days
        // Return deserialized Application rows
        todo!()
    }

    async fn count_no_contact_ever(&self) -> Result<usize, MetricsError> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM applications WHERE stage NOT IN ('Discovered','Interested','Accepted','Rejected','Withdrawn') AND last_contact_at IS NULL"
        ).fetch_one(&self.pool).await?;
        Ok(count as usize)
    }

    async fn compute_velocity(&self) -> Result<HashMap<ApplicationStage, Option<f32>>, MetricsError> {
        // Execute VELOCITY_BY_STAGE; stages with < 3 transitions → None
        todo!()
    }

    async fn compute_median_response(&self) -> Result<Option<f32>, MetricsError> {
        // Execute MEDIAN_DAYS_APPLIED_TO_PHONE; None if no rows
        todo!()
    }
}
```

**Verification:**
- `#[sqlx::test(migrations = "migrations")]` test with 10 seeded applications across stages returns correct counts.
- `response_rate` is `0.0` when only `Discovered` applications exist.

#### Step 1.4 — Implement `ActionItemService::list_action_required()`

```rust
// lazyjob-core/src/application/action_item.rs

pub struct ActionItemService {
    app_repo: Arc<dyn ApplicationRepository>,
    reminder_repo: Arc<dyn ReminderRepository>,
    offer_repo: Arc<dyn OfferRepository>,
    interview_repo: Arc<dyn InterviewRepository>,
    pool: Pool<Sqlite>,
}

impl ActionItemService {
    pub async fn list_action_required(&self) -> Result<Vec<ActionItem>, MetricsError> {
        let now = Utc::now();
        let mut items: Vec<ActionItem> = Vec::new();

        // 1. Expiring offers (within 5 days)
        for offer in self.offer_repo.list_expiring(5).await? {
            let app = self.app_repo.get(&offer.application_id).await?;
            let days_remaining = offer.expiry_date
                .map(|d| (d - now.date_naive()).num_days().max(0) as u32)
                .unwrap_or(0);
            items.push(ActionItem::ExpiringOffer { offer, application: app, days_remaining });
        }

        // 2. Upcoming interviews (within 48 hours)
        for interview in self.interview_repo.list_upcoming(48).await? {
            let app = self.app_repo.get(&interview.application_id).await?;
            let hours = (interview.scheduled_at - now).num_hours();
            items.push(ActionItem::UpcomingInterview { interview, application: app, hours_until: hours });
        }

        // 3. Overdue follow-up reminders
        for reminder in self.reminder_repo.list_pending(now).await? {
            let app = self.app_repo.get(&reminder.application_id).await?;
            items.push(ActionItem::OverdueFollowUp { reminder, application: app });
        }

        // 4. Stale applications (14d default, respects snoozes)
        for app in self.list_stale_excluding_snoozed(14).await? {
            let days = Self::days_stale(&app, now);
            items.push(ActionItem::StaleApplication { application: app, days_stale: days });
        }

        // 5. New rejections (since last TUI open, tracked in local state)
        for app in self.list_new_rejections_since(self.last_opened_at()).await? {
            items.push(ActionItem::NewRejection { application: app });
        }

        // Sort by priority (stable sort preserves relative ordering within category)
        items.sort_by_key(|a| a.priority());

        Ok(items)
    }

    fn days_stale(app: &Application, now: DateTime<Utc>) -> u32 {
        app.last_contact_at
            .map(|lc| (now - lc).num_days().max(0) as u32)
            .unwrap_or_else(|| {
                (now - app.created_at).num_days().max(0) as u32
            })
    }
}
```

**Verification:** Unit test with mocked repos verifies ordering: ExpiringOffer appears before StaleApplication.

---

### Phase 2 — ReminderPoller and DigestService

#### Step 2.1 — Implement `ReminderPoller`

```rust
// lazyjob-core/src/application/reminders.rs

use std::sync::Arc;
use std::time::Duration;
use chrono::Utc;
use tokio::sync::broadcast;
use tracing::{info, error};
use crate::application::repository::ReminderRepository;
use crate::application::workflow::WorkflowEvent;

pub struct ReminderPoller {
    repo: Arc<dyn ReminderRepository>,
    sender: broadcast::Sender<WorkflowEvent>,
}

impl ReminderPoller {
    pub fn new(
        repo: Arc<dyn ReminderRepository>,
        sender: broadcast::Sender<WorkflowEvent>,
    ) -> Self {
        Self { repo, sender }
    }

    /// Spawn as a detached tokio task. The handle is returned for cancellation.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match self.repo.list_pending(Utc::now()).await {
                Ok(due) => {
                    for reminder in due {
                        let event = WorkflowEvent::ReminderDue {
                            reminder_id: reminder.id.clone(),
                            application_id: reminder.application_id.clone(),
                            title: reminder.title.clone(),
                        };
                        // Ignore SendError — no active TUI subscribers is not a failure
                        let _ = self.sender.send(event);
                        if let Err(e) = self.repo.mark_fired(&reminder.id, Utc::now()).await {
                            error!(?e, "failed to mark reminder fired");
                        }
                    }
                }
                Err(e) => error!(?e, "ReminderPoller: failed to list pending reminders"),
            }
        }
    }
}
```

Key crate APIs:
- `tokio::time::interval(Duration)` — creates an interval stream
- `interval.set_missed_tick_behavior(MissedTickBehavior::Skip)` — prevents burst tick catch-up after sleep/wake
- `broadcast::Sender::send()` — returns `Err(SendError)` when zero receivers; must be ignored here

**Verification:**
- Unit test: seed a reminder due in the past, mock `mark_fired`, run one poll cycle, verify `mark_fired` was called.
- Integration test: full tokio runtime, 300ms interval mocked via `tokio::time::pause()` + `advance()`.

#### Step 2.2 — Implement `DigestService`

```rust
// lazyjob-core/src/application/digest.rs

use std::sync::Arc;
use chrono::{Local, NaiveDate, Utc};
use crate::application::action_item::ActionItemService;
use crate::application::metrics::MetricsService;
use crate::jobs::repository::JobRepository;

pub struct DigestService {
    metrics_svc: Arc<MetricsService>,
    action_svc: Arc<ActionItemService>,
    job_repo: Arc<dyn JobRepository>,
    digest_repo: Arc<dyn DigestRepository>,
    pub preferences: UserPreferences,
}

impl DigestService {
    pub fn should_show_today(&self) -> bool {
        // Call digest_repo.has_shown_today() synchronously via tokio::task::block_in_place
        // or require callers to be async.
        // In practice, called from CLI main() inside tokio::main, so async is fine.
        todo!()
    }

    pub async fn generate_daily_digest(&self) -> Result<DailyDigest, MetricsError> {
        let metrics = self.metrics_svc.compute().await?;
        let action_items = self.action_svc.list_action_required().await?;

        // New job matches since yesterday
        let yesterday = (Utc::now() - chrono::Duration::days(1)).into();
        let (new_matches, strong_matches) = self.job_repo
            .count_discovered_since(yesterday).await
            .unwrap_or((0, 0));

        let tips = self.build_tips(&metrics);

        Ok(DailyDigest {
            date: Local::now().date_naive(),
            new_job_matches: new_matches,
            strong_matches,
            active_applications: metrics.active_applications,
            action_items,
            response_rate: metrics.response_rate,
            tips,
            privacy_mode: self.preferences.privacy_mode,
        })
    }

    pub async fn mark_shown(&self) -> Result<(), MetricsError> {
        self.digest_repo.record_shown(Local::now().date_naive()).await
    }

    fn build_tips(&self, m: &PipelineMetrics) -> Vec<String> {
        let mut tips = Vec::new();
        if m.response_rate < 0.05 && m.total_applications >= 10 {
            tips.push(
                "Response rate below 5%. Consider: (1) targeting roles where you have connections, (2) increasing resume tailoring.".to_string()
            );
        }
        if m.stale_applications > 5 {
            tips.push(format!(
                "{} applications are stale. Batch-archive or send follow-ups.",
                m.stale_applications
            ));
        }
        tips
    }
}

impl DailyDigest {
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let line = "─".repeat(44);
        writeln!(out, "{}", line).unwrap();
        writeln!(out, "LazyJob daily digest — {}", self.date.format("%A, %b %-d")).unwrap();
        writeln!(out, "{}", line).unwrap();
        writeln!(out, "New matches since yesterday: {} jobs ({} strong match)", self.new_job_matches, self.strong_matches).unwrap();
        writeln!(out, "Pipeline: {} active applications | {} action required", self.active_applications, self.action_items.len()).unwrap();
        for item in &self.action_items {
            match item {
                ActionItem::ExpiringOffer { application, days_remaining, .. } => {
                    let company = if self.privacy_mode { "[redacted]" } else { &application.company_name };
                    writeln!(out, "  → Offer expiring in {days_remaining}d ({company})").unwrap();
                }
                ActionItem::UpcomingInterview { application, hours_until, .. } => {
                    let company = if self.privacy_mode { "[redacted]" } else { &application.company_name };
                    writeln!(out, "  → Interview in {hours_until}h ({company})").unwrap();
                }
                ActionItem::OverdueFollowUp { application, .. } => {
                    let company = if self.privacy_mode { "[redacted]" } else { &application.company_name };
                    writeln!(out, "  → Overdue follow-up ({company})").unwrap();
                }
                ActionItem::StaleApplication { application, days_stale } => {
                    let company = if self.privacy_mode { "[redacted]" } else { &application.company_name };
                    writeln!(out, "  → Stale {days_stale}d ({company})").unwrap();
                }
                ActionItem::NewRejection { application } => {
                    let company = if self.privacy_mode { "[redacted]" } else { &application.company_name };
                    writeln!(out, "  → Rejected ({company})").unwrap();
                }
            }
        }
        writeln!(out, "Response rate: {:.1}%", self.response_rate * 100.0).unwrap();
        for tip in &self.tips {
            writeln!(out, "Tip: {tip}").unwrap();
        }
        writeln!(out, "{}", line).unwrap();
        out
    }
}
```

**Verification:** Unit test calls `render()` on a known `DailyDigest` and asserts output contains expected lines.

#### Step 2.3 — Wire digest into `lazyjob-cli/src/main.rs`

```rust
// lazyjob-cli/src/main.rs (simplified structure)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let db = Database::connect(&config.database_path).await?;
    let services = Services::build(&config, &db).await?;

    // Morning digest: print before TUI launches, at most once per day
    if services.digest.should_show_today().await? {
        let digest = services.digest.generate_daily_digest().await?;
        print!("{}", digest.render());
        services.digest.mark_shown().await?;
    }

    // Launch TUI
    services.tui.run().await?;

    Ok(())
}
```

**Verification:** Running `lazyjob` twice in the same day: digest prints on first invocation only.

---

### Phase 3 — TUI: Action Queue View

#### Step 3.1 — `ActionQueueView` widget

The action queue is a scrollable list of `ActionItem` entries. Each item shows:
- A severity badge (URGENT / TODAY / OVERDUE / STALE)
- One-line description (company name, stage, days)
- Per-item action hints in the status bar

```rust
// lazyjob-tui/src/views/action_queue.rs

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget},
    Frame,
};
use lazyjob_core::application::action_item::ActionItem;

pub struct ActionQueueState {
    pub items: Vec<ActionItem>,
    pub list_state: ListState,
}

impl ActionQueueState {
    pub fn new(items: Vec<ActionItem>) -> Self {
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }
        Self { items, list_state }
    }

    pub fn selected(&self) -> Option<&ActionItem> {
        self.list_state.selected().and_then(|i| self.items.get(i))
    }

    pub fn move_down(&mut self) {
        let i = self.list_state.selected().map_or(0, |i| (i + 1).min(self.items.len().saturating_sub(1)));
        self.list_state.select(Some(i));
    }

    pub fn move_up(&mut self) {
        let i = self.list_state.selected().map_or(0, |i| i.saturating_sub(1));
        self.list_state.select(Some(i));
    }
}

pub struct ActionQueueView;

impl StatefulWidget for ActionQueueView {
    type State = ActionQueueState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items: Vec<ListItem> = state.items.iter().map(|item| {
            let (badge, badge_color) = match item {
                ActionItem::ExpiringOffer { .. }      => ("URGENT", Color::Red),
                ActionItem::UpcomingInterview { .. }  => ("TODAY ", Color::Yellow),
                ActionItem::OverdueFollowUp { .. }    => ("OVERDUE", Color::Magenta),
                ActionItem::StaleApplication { .. }   => ("STALE ", Color::Cyan),
                ActionItem::NewRejection { .. }       => ("INFO  ", Color::DarkGray),
            };
            let description = action_description(item);
            let line = Line::from(vec![
                Span::styled(format!(" {badge} "), Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::raw(description),
            ]);
            ListItem::new(line)
        }).collect();

        let title = format!(" Action Required ({}) ", state.items.len());
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        StatefulWidget::render(list, area, buf, &mut state.list_state);
    }
}

fn action_description(item: &ActionItem) -> String {
    match item {
        ActionItem::ExpiringOffer { application, days_remaining, .. } =>
            format!("Offer expiring in {}d — {} {}", days_remaining, application.company_name, application.job_title),
        ActionItem::UpcomingInterview { application, hours_until, .. } =>
            format!("Interview in {}h — {} {}", hours_until, application.company_name, application.job_title),
        ActionItem::OverdueFollowUp { application, reminder } =>
            format!("Follow up with {} ({} overdue)", application.company_name, reminder.title),
        ActionItem::StaleApplication { application, days_stale } =>
            format!("{} — {} ({}d stale)", application.company_name, application.job_title, days_stale),
        ActionItem::NewRejection { application } =>
            format!("Rejected — {} {}", application.company_name, application.job_title),
    }
}
```

**Key bindings** (handled in `EventLoop`, not the widget):
- `j`/`k` or `↓`/`↑` — navigate list
- `a` — act on selected item (context-dependent action)
- `s` — snooze (2 days for follow-ups, 1 day for interviews)
- `d` — dismiss stale flag (7 days)
- `q` — close action queue

**Verification:** Render test using `ratatui::backend::TestBackend`:
```rust
#[test]
fn renders_with_three_items() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let items = vec![/* mock ActionItem::ExpiringOffer, StaleApplication */];
    let mut state = ActionQueueState::new(items);
    terminal.draw(|f| f.render_stateful_widget(ActionQueueView, f.area(), &mut state)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // assert "URGENT" appears in buffer
}
```

---

### Phase 4 — TUI: Pipeline Metrics Dashboard

#### Step 4.1 — `PipelineMetricsView` layout

The metrics view occupies the full frame. Layout:

```
┌──────────────────────────────────────────────────────────────┐
│ Funnel (Applied → Accepted)        │ Stage Distribution       │
│ ████████████ Applied   22          │ ██ PhoneScreen 5        │
│ ██████ Phone   5  22.7%            │ █  Technical  2         │
│ ████ Technical 2  9.1%             │ ██ OnSite     2         │
│ ██ Offer       2  9.1%             │ █  Offer      2         │
│ █ Accepted     1  4.5%             │ ██ Stale      3         │
├──────────────────────────────────────────────────────────────┤
│ Stage Velocity (avg days)          │ Stale Applications       │
│ Applied       → Phone: 8.3d        │ Figma — 19d stale       │
│ Phone         → Tech:  5.1d        │ Notion — 14d stale      │
│ Technical     → OnSite: 4.2d       │ Ramp — 14d stale        │
└──────────────────────────────────────────────────────────────┘
```

```rust
// lazyjob-tui/src/views/pipeline_metrics.rs

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, BarChart, Table, Row, Cell},
    style::Style,
    Frame,
};
use lazyjob_core::application::metrics::PipelineMetrics;

pub struct PipelineMetricsView<'a> {
    pub metrics: &'a PipelineMetrics,
    pub stale: &'a [Application],
}

impl<'a> Widget for PipelineMetricsView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let top_bottom = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(top_bottom[0]);

        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(top_bottom[1]);

        self.render_funnel(top[0], buf);
        self.render_stage_bar(top[1], buf);
        self.render_velocity_table(bottom[0], buf);
        self.render_stale_table(bottom[1], buf);
    }
}

impl<'a> PipelineMetricsView<'a> {
    fn render_funnel(&self, area: Rect, buf: &mut Buffer) {
        // ratatui BarChart with horizontal=true, max derived from Applied count
        // Each bar represents one funnel stage with percentage label
        let m = self.metrics;
        let denom = m.by_stage.get(&ApplicationStage::Applied).copied().unwrap_or(1).max(1);
        let data: Vec<(&str, u64)> = vec![
            ("Applied",    m.by_stage.get(&ApplicationStage::Applied).copied().unwrap_or(0) as u64),
            ("PhoneScr",  m.by_stage.get(&ApplicationStage::PhoneScreen).copied().unwrap_or(0) as u64),
            ("Technical", m.by_stage.get(&ApplicationStage::Technical).copied().unwrap_or(0) as u64),
            ("OnSite",    m.by_stage.get(&ApplicationStage::OnSite).copied().unwrap_or(0) as u64),
            ("Offer",     m.by_stage.get(&ApplicationStage::Offer).copied().unwrap_or(0) as u64),
            ("Accepted",  m.by_stage.get(&ApplicationStage::Accepted).copied().unwrap_or(0) as u64),
        ];
        let chart = BarChart::default()
            .block(Block::default().borders(Borders::ALL).title(" Funnel "))
            .data(&data)
            .bar_width(1)
            .bar_gap(0)
            .max(denom as u64);
        Widget::render(chart, area, buf);
    }

    fn render_velocity_table(&self, area: Rect, buf: &mut Buffer) {
        let stages = [
            (ApplicationStage::Applied, "Applied → Screen"),
            (ApplicationStage::PhoneScreen, "Screen → Tech"),
            (ApplicationStage::Technical, "Tech → OnSite"),
            (ApplicationStage::OnSite, "OnSite → Offer"),
        ];
        let rows: Vec<Row> = stages.iter().map(|(stage, label)| {
            let days = self.metrics.avg_days_in_stage.get(stage).copied().flatten();
            let cell = match days {
                Some(d) => Cell::from(format!("{:.1}d", d)),
                None    => Cell::from("—  "),
            };
            Row::new(vec![Cell::from(*label), cell])
        }).collect();

        let table = Table::new(rows, [Constraint::Percentage(70), Constraint::Percentage(30)])
            .block(Block::default().borders(Borders::ALL).title(" Stage Velocity "))
            .header(Row::new(["Transition", "Avg Days"]));
        Widget::render(table, area, buf);
    }

    fn render_stale_table(&self, area: Rect, buf: &mut Buffer) {
        let rows: Vec<Row> = self.stale.iter().take(10).map(|app| {
            Row::new(vec![
                Cell::from(app.company_name.as_str()),
                Cell::from(app.job_title.as_str()),
            ])
        }).collect();

        let table = Table::new(rows, [Constraint::Percentage(50), Constraint::Percentage(50)])
            .block(Block::default().borders(Borders::ALL).title(format!(" Stale ({}) ", self.stale.len())))
            .header(Row::new(["Company", "Role"]));
        Widget::render(table, area, buf);
    }

    fn render_stage_bar(&self, area: Rect, buf: &mut Buffer) {
        let active_stages = [
            ApplicationStage::PhoneScreen,
            ApplicationStage::Technical,
            ApplicationStage::OnSite,
            ApplicationStage::Offer,
        ];
        let data: Vec<(&str, u64)> = active_stages.iter().map(|s| {
            let label = match s {
                ApplicationStage::PhoneScreen => "PhoneScr",
                ApplicationStage::Technical => "Techncl",
                ApplicationStage::OnSite => "OnSite",
                ApplicationStage::Offer => "Offer",
                _ => "Other",
            };
            (label, self.metrics.by_stage.get(s).copied().unwrap_or(0) as u64)
        }).collect();
        let chart = BarChart::default()
            .block(Block::default().borders(Borders::ALL).title(" Active Stages "))
            .data(&data)
            .bar_width(2);
        Widget::render(chart, area, buf);
    }
}
```

**Key bindings** for this view (handled in `EventLoop`):
- `m` — toggle metrics view (from application list)
- `q` / `Esc` — return to application list
- No interactive state needed in Phase 4 (view-only)

**Contextual tip rendering**: After the funnel, if `response_rate < 0.05` and `total_applications >= 10`, the view renders a highlighted tip block below the funnel using `ratatui::widgets::Paragraph` with a yellow border.

**Verification:** `TestBackend` render test with a `PipelineMetrics` that has known values; assert bar chart text contains expected stage names.

---

### Phase 5 — Integration and Polish

#### Step 5.1 — Snooze flow in action queue

When the user presses `s` on a `StaleApplication` item:

```rust
// lazyjob-tui/src/views/action_queue.rs (event handler)

Action::Snooze => {
    if let Some(ActionItem::StaleApplication { application, .. }) = state.selected() {
        let until = Utc::now() + chrono::Duration::days(7);
        ctx.services.reminder_repo.snooze(&application.id, until, None).await?;
        // Reload action items
        state.items = ctx.services.action_svc.list_action_required().await?;
        state.list_state.select(state.list_state.selected().map(|i| i.min(state.items.len().saturating_sub(1))));
    }
}
```

The `application_snoozes` table's partial index (`WHERE snoozed_until > datetime('now')`) ensures the stale query efficiently excludes recently snoozed applications without a full table scan.

#### Step 5.2 — `UserPreferences` stale threshold

```rust
// lazyjob-core/src/config.rs

pub struct TrackingPreferences {
    pub stale_days: u32,           // default: 14
    pub digest_enabled: bool,      // default: true
    pub privacy_mode: bool,        // default: false
    pub snooze_follow_up_days: u32,  // default: 2
    pub snooze_stale_days: u32,    // default: 7
}
```

Loaded from `~/.config/lazyjob/config.toml` via the `config` crate or `serde_yaml`. `MetricsService` and `ActionItemService` accept `TrackingPreferences` at construction.

#### Step 5.3 — Metrics refresh on broadcast events

The TUI's `EventLoop` subscribes to `broadcast::Receiver<WorkflowEvent>`. When `WorkflowEvent::StageTransitionEvent` is received while the metrics view is active, it re-calls `MetricsService::compute()` and updates the view state. This ensures the metrics dashboard reflects the latest state without requiring a full restart.

```rust
// lazyjob-tui/src/event_loop.rs (pseudocode)

tokio::select! {
    Some(event) = workflow_rx.recv() => {
        match event {
            WorkflowEvent::ReminderDue { .. } => {
                app.action_queue_state.items = action_svc.list_action_required().await?;
            }
            WorkflowEvent::StageTransitionEvent { .. } => {
                if app.current_view == View::Metrics {
                    app.metrics = metrics_svc.compute().await?;
                }
            }
            _ => {}
        }
    }
    // ... other branches
}
```

---

## Key Crate APIs

| API | Usage |
|---|---|
| `sqlx::query!(SQL).fetch_all(&pool).await` | Execute stage count query |
| `sqlx::query_scalar!(SQL).fetch_one(&pool).await` | Single scalar (count, median) |
| `tokio::time::interval(Duration::from_secs(300))` | ReminderPoller 5-minute tick |
| `interval.set_missed_tick_behavior(MissedTickBehavior::Skip)` | Prevent burst ticks after sleep |
| `broadcast::Sender<WorkflowEvent>::send(event)` | Fire reminder event to TUI |
| `ratatui::widgets::BarChart::default().data(&data).max(n)` | Funnel and distribution charts |
| `ratatui::widgets::List::new(items).highlight_style(...)` | Action queue scrollable list |
| `ratatui::widgets::StatefulWidget` impl for `ActionQueueView` | Stateful selection tracking |
| `ratatui::backend::TestBackend::new(w, h)` | Widget rendering tests |
| `chrono::Duration::days(n)` | Snooze duration calculation |
| `Local::now().date_naive()` | Morning digest date check |

---

## Error Handling

```rust
// lazyjob-core/src/application/metrics.rs

#[derive(thiserror::Error, Debug)]
pub enum MetricsError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("application not found: {0}")]
    ApplicationNotFound(ApplicationId),

    #[error("reminder not found: {0}")]
    ReminderNotFound(ReminderId),

    #[error("digest already shown today")]
    DigestAlreadyShown,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, MetricsError>;
```

`MetricsError::DigestAlreadyShown` is only returned from `DigestRepository::record_shown` if the caller inserts on a day already present — callers must check `has_shown_today()` first to avoid this (or use `INSERT OR IGNORE`).

---

## Testing Strategy

### Unit Tests

All in `lazyjob-core/src/application/metrics.rs` and `digest.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Test rate calculations with known stage counts
    #[test]
    fn response_rate_correct() {
        // 10 Applied + 5 PhoneScreen → 50%
    }

    // Test rate returns 0.0 when only Discovered apps exist
    #[test]
    fn rates_zero_when_only_discovered() {
        let mut by_stage = HashMap::new();
        by_stage.insert(ApplicationStage::Discovered, 10);
        // verify response_rate = 0.0
    }

    // Test DailyDigest::render() output
    #[test]
    fn digest_render_includes_action_items() { }

    // Test privacy_mode redacts company names in render()
    #[test]
    fn digest_redacts_company_in_privacy_mode() { }

    // Test ActionItem::priority() ordering
    #[test]
    fn action_item_ordering_expiring_offer_first() { }

    // Test days_stale calculation from last_contact_at
    #[test]
    fn days_stale_uses_last_contact_at() { }
}
```

### Integration Tests with `#[sqlx::test]`

```rust
// lazyjob-core/tests/metrics_integration.rs

#[sqlx::test(migrations = "migrations")]
async fn compute_with_seeded_applications(pool: Pool<Sqlite>) {
    // Seed 20 applications across various stages
    // Assert by_stage counts match seeded data
    // Assert response_rate is computed correctly
}

#[sqlx::test(migrations = "migrations")]
async fn stale_query_excludes_snoozed(pool: Pool<Sqlite>) {
    // Seed 3 stale applications, snooze 1 for 7 days
    // Assert list_stale() returns 2 (not 3)
}

#[sqlx::test(migrations = "migrations")]
async fn reminder_poller_fires_and_marks_done(pool: Pool<Sqlite>) {
    // Seed a reminder with due_at = 1 hour ago
    // Run one poll cycle (tokio::time::pause + advance)
    // Assert fired_at is now set
}

#[sqlx::test(migrations = "migrations")]
async fn digest_shown_only_once_per_day(pool: Pool<Sqlite>) {
    // Call mark_shown() twice
    // Assert has_shown_today() returns true on second call
    // Assert INSERT OR IGNORE prevents duplicate rows
}
```

### TUI Tests

```rust
// lazyjob-tui/tests/action_queue_widget.rs

#[test]
fn action_queue_renders_item_count_in_title() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let items = vec![mock_expiring_offer(), mock_stale_application()];
    let mut state = ActionQueueState::new(items);
    terminal.draw(|f| f.render_stateful_widget(ActionQueueView, f.area(), &mut state)).unwrap();
    let content: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Action Required (2)"));
    assert!(content.contains("URGENT"));
}
```

---

## Open Questions

1. **Benchmark baselines**: Should `PipelineMetrics` include a `response_rate_baseline: f32` field showing the anonymized industry 5.75% figure? Motivating for some users, demoralizing for others. Deferred to a configuration option (`tracking.show_benchmarks`, default: true).

2. **Out-of-TUI reminders**: For users who want reminders when the TUI is closed, could `ReminderPoller` write to `~/.lazyjob/reminders.txt` that a shell alias or cron job reads? Deferred — LazyJob is terminal-only in MVP.

3. **Digest suppression**: Should `privacy_mode = true` also suppress the digest entirely? Current plan: it only redacts company names. Full suppression can be a separate flag (`tracking.digest_enabled`).

4. **Multi-offer comparison in metrics**: When `offers.expiry_date` is tracked per application, `PipelineMetrics` could expose `best_offer_total_comp`. Deferred to `salary-market-intelligence.md` scope.

5. **Velocity data insufficiency threshold**: Currently "< 3 data points → None". Should this be configurable? For now, 3 is a hardcoded constant `MIN_VELOCITY_SAMPLES: usize = 3`.

6. **`last_opened_at` tracking for NewRejection**: The `ActionItemService::list_new_rejections_since()` needs to know when the TUI was last opened. This could be tracked in a `tui_sessions` table or a local file. The simplest approach: write a timestamp to `~/.lazyjob/last_opened` on TUI startup, read it in `ActionItemService`. Deferred to a small follow-up task.

---

## Related Specs

- [`specs/application-state-machine.md`](./application-state-machine.md) — provides `ApplicationStage`, `Application`, `application_transitions` table
- [`specs/application-workflow-actions.md`](./application-workflow-actions.md) — provides `WorkflowEvent`, `Reminder`, `Interview`, `Offer`, `ReminderRepository`
- [`specs/09-tui-design-keybindings.md`](./09-tui-design-keybindings.md) — provides `EventLoop`, `App`, `KeyContext`, `Action`
- [`specs/04-sqlite-persistence.md`](./04-sqlite-persistence.md) — provides `Database`, migrations
- [`specs/salary-market-intelligence.md`](./salary-market-intelligence.md) — may extend `PipelineMetrics` with offer comp analysis
