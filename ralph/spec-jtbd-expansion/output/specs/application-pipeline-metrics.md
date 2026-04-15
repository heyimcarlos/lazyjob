# Spec: Application Pipeline Metrics & Notifications

**JTBD**: A-3 — Track where I stand in every hiring process at a glance
**Topic**: Compute pipeline health metrics from application state data and surface actionable reminders to the user on demand and via a daily digest.
**Domain**: application-tracking

---

## What

The pipeline metrics layer aggregates application state data into health signals: response rate, interview rate, offer rate, average stage velocity, and stale application count. These metrics are computed on-demand from SQLite (no background aggregation job needed at MVP scale). A `ReminderPoller` surfaces due reminders in the TUI's action queue. An optional morning digest (a single terminal print at first-run of the day) summarizes overnight changes, due reminders, and pipeline health. No push notifications — LazyJob is a terminal tool, not a mobile app.

## Why

Most job seekers lack visibility into their own pipeline health. Without metrics, a user with 40 applications doesn't know that their response rate is 2% (below the 5.75% baseline for tailored resumes), or that 12 applications have been stale for 14+ days with no follow-up. This invisible data gap leads to continued ineffective behavior — applying more without understanding why current applications aren't converting.

The recruiter research is instructive: recruiters themselves track these conversion metrics (applications → screens → interviews → offers) because they reveal funnel health. Surfacing the same visibility to candidates is a meaningful information asymmetry reduction — and a core differentiator from passive trackers like Notion boards or Excel sheets.

## How

### PipelineMetrics Computation

Metrics are always computed fresh from `ApplicationRepository::count_by_stage` and a single pass over active applications. At MVP scale (1 user, <500 applications), there is no materialized view or background aggregation — just a single SQL query and a Rust computation pass.

```rust
// lazyjob-core/src/application/metrics.rs

pub struct PipelineMetrics {
    // Volume
    pub total_applications: usize,
    pub active_applications: usize,       // !is_terminal
    pub terminal_applications: usize,

    // By stage (kanban column counts)
    pub by_stage: HashMap<ApplicationStage, usize>,

    // Funnel conversion rates (exclude Discovered/Interested as "not yet applied")
    pub response_rate: f32,     // % of Applied+ that reached PhoneScreen+
    pub interview_rate: f32,    // % of Applied+ that reached Technical+
    pub offer_rate: f32,        // % of Applied+ that received an Offer+
    pub acceptance_rate: f32,   // % of Offer+ that were Accepted

    // Staleness
    pub stale_applications: usize,   // Active apps with last_contact_at > 14 days ago
    pub no_contact_ever: usize,       // Applied but never any contact logged

    // Velocity (average days spent in each stage before advancing)
    pub avg_days_in_stage: HashMap<ApplicationStage, f32>,
    pub median_days_to_response: Option<f32>,  // Discovered → PhoneScreen (if available)

    // Action required
    pub overdue_follow_ups: usize,    // Reminders past due_at
    pub expiring_offers: usize,       // Offers with expiry_date within 5 days

    // Computed at
    pub computed_at: DateTime<Utc>,
}
```

**Stale threshold**: 14 days without `last_contact_at` update on an active application. This is the product-defined threshold, configurable in user preferences (`tracking.stale_days`, default: 14). Applications in `Discovered` or `Interested` stage use a longer threshold (30 days) — they represent passive interest, not active tracking.

**Rate calculation denominator**: All conversion rates use `Applied + PhoneScreen + Technical + OnSite + Offer + Accepted + Rejected` (i.e., jobs where application was actually submitted). `Discovered` and `Interested` are excluded from funnel math.

### Stale Detection Query

```sql
-- lazyjob-core/src/application/queries.rs
SELECT id, job_id, stage, last_contact_at, next_follow_up_at
FROM applications
WHERE
    stage NOT IN ('Accepted', 'Rejected', 'Withdrawn')
    AND (
        last_contact_at IS NULL
        OR last_contact_at < datetime('now', '-14 days')
    )
    AND stage NOT IN ('Discovered', 'Interested')
ORDER BY last_contact_at ASC NULLS FIRST;
```

### TUI Action Required Queue

The action required queue is the "inbox" of the application tracking view. It surfaces:

1. **Overdue follow-ups**: Reminders where `due_at < now()` and `fired_at IS NULL`
2. **Stale applications**: Active apps with `last_contact_at` > stale threshold
3. **Expiring offers**: `offers.expiry_date` within 5 days
4. **Upcoming interviews**: `interviews.scheduled_at` within 48 hours and status = `Scheduled`
5. **New rejections** (informational): Applications that moved to `Rejected` since last TUI open

Ordering: expiring offers first (highest urgency), then upcoming interviews, then overdue follow-ups, then stale applications.

```
┌─────────────────────────────────────────────────────────────────┐
│  Action Required (4)                                             │
├─────────────────────────────────────────────────────────────────┤
│  URGENT  Offer expiring in 3 days — Datadog Staff SRE           │
│          Base $245K + equity. Deadline: Mon Apr 20              │
│          [Evaluate]  [Start Negotiation]  [Snooze]              │
├─────────────────────────────────────────────────────────────────┤
│  TODAY   Interview in 18h — Stripe Systems Engineer             │
│          Technical round at 2:00 PM PDT                         │
│          [Prep Now]  [View Details]                             │
├─────────────────────────────────────────────────────────────────┤
│  OVERDUE Follow up with Cloudflare (4d overdue)                 │
│          Applied 11 days ago, no response                       │
│          [Draft Follow-up]  [Mark Rejected]  [Snooze 2d]        │
├─────────────────────────────────────────────────────────────────┤
│  STALE   Figma — Product Engineer (19d stale)                   │
│          Applied → no contact since Apr 6                       │
│          [Log Contact]  [Mark Rejected]  [Dismiss]              │
└─────────────────────────────────────────────────────────────────┘
```

**Design rules**:
- Action items are never auto-dismissed. The user must explicitly act or snooze each one.
- "Snooze" defers the reminder by N days (configurable, default: 2d for follow-ups, 1d for interview prep).
- "Dismiss" removes the stale flag for 7 days (does NOT mark the application as rejected).
- The queue is not a push notification system — it surfaces only when the TUI is open.

### Morning Digest

The morning digest is printed once per day at the start of the first `lazyjob` invocation after a configurable time (default: 06:00 local). It is a plain-text summary printed to stdout before the TUI launches.

```
──────────────────────────────────────────
LazyJob daily digest — Tuesday, Apr 15
──────────────────────────────────────────
New matches since yesterday: 8 jobs (3 strong match)
Pipeline: 12 active applications | 4 action required
  → 1 offer expiring in 3 days (Datadog)
  → 1 interview tomorrow (Stripe)
  → 2 stale applications need follow-up
Response rate: 22.7% (pipeline: 5 of 22 applied)
Reminder: Applied to Figma 21 days ago — consider archiving
──────────────────────────────────────────
```

The digest is generated by `DigestService::generate_daily_digest()`, which reads from `ApplicationRepository`, `PipelineMetrics`, and `JobRepository` (for new matches). It is a read-only operation — no state changes. It respects `privacy_mode`: if `privacy_mode = true`, company names are omitted (job IDs shown instead).

### Velocity Metrics

Average days in each stage is computed from `application_transitions` timestamps:

```sql
SELECT
    from_stage,
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
GROUP BY from_stage;
```

This only computes correctly after accumulating several weeks of transition history. Before that, the TUI shows "Insufficient data" placeholders rather than misleading metrics computed on 1–2 data points.

### Metrics View in TUI

The metrics view is a secondary view accessible from the application tracking view (press `m` or navigate via status bar). It shows:

- **Funnel chart** (ratatui horizontal bars): Applied → Screen → Interview → Offer → Accepted
- **Stage distribution** (ratatui bar chart): Count per active stage
- **Stale list**: Sortable table of stale applications with last contact date
- **Velocity table**: Average days per stage

The funnel chart adapts to the user's personal data — it is not a benchmark comparison. However, the TUI may surface a contextual tip if response_rate < 5.0%: "Your response rate is below baseline. Consider: (1) increasing resume tailoring, (2) targeting companies where you have connections."

### Reminder Polling

`ReminderPoller` is a tokio task that wakes every 5 minutes and checks for due reminders:

```rust
// lazyjob-core/src/application/reminders.rs

pub struct ReminderPoller {
    repo: Arc<dyn ReminderRepository>,
    events: broadcast::Sender<WorkflowEvent>,
}

impl ReminderPoller {
    pub async fn run(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            let due = self.repo.list_pending(Utc::now()).await.unwrap_or_default();
            for reminder in due {
                let _ = self.events.send(WorkflowEvent::ReminderDue {
                    reminder_id: reminder.id,
                    application_id: reminder.application_id,
                    title: reminder.title.clone(),
                });
                self.repo.mark_fired(&reminder.id, Utc::now()).await.ok();
            }
        }
    }
}
```

The `WorkflowEvent::ReminderDue` event is consumed by the TUI's event loop and added to the action queue. The poller runs in the background as a tokio spawn — it does not block the main TUI loop.

## Interface

```rust
// lazyjob-core/src/application/metrics.rs

pub struct MetricsService {
    app_repo: Arc<dyn ApplicationRepository>,
    reminder_repo: Arc<dyn ReminderRepository>,
    offer_repo: Arc<dyn OfferRepository>,
    interview_repo: Arc<dyn InterviewRepository>,
}

impl MetricsService {
    pub async fn compute(&self) -> Result<PipelineMetrics>;
    pub async fn list_stale(&self, threshold_days: u32) -> Result<Vec<Application>>;
    pub async fn list_action_required(&self) -> Result<Vec<ActionItem>>;
}

pub enum ActionItem {
    OverdueFollowUp { reminder: Reminder, application: Application },
    StaleApplication { application: Application, days_stale: u32 },
    ExpiringOffer { offer: Offer, application: Application, days_remaining: u32 },
    UpcomingInterview { interview: Interview, application: Application, hours_until: i64 },
    NewRejection { application: Application },
}

// lazyjob-core/src/application/digest.rs

pub struct DigestService {
    metrics: Arc<MetricsService>,
    job_repo: Arc<dyn JobRepository>,
    preferences: UserPreferences,
}

impl DigestService {
    pub async fn generate_daily_digest(&self) -> Result<DailyDigest>;
    pub fn should_show_today(&self) -> bool;  // checks last digest date
    pub fn mark_shown(&self);
}

pub struct DailyDigest {
    pub date: NaiveDate,
    pub new_job_matches: usize,
    pub strong_matches: usize,
    pub active_applications: usize,
    pub action_items: Vec<ActionItem>,
    pub response_rate: f32,
    pub tips: Vec<String>,
}
```

```sql
-- lazyjob-core/migrations/002_applications.sql (continuation)

-- Track last digest shown to avoid showing multiple times per day
CREATE TABLE digest_log (
    date TEXT PRIMARY KEY,
    shown_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Soft snooze for stale applications
CREATE TABLE application_snoozes (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    snoozed_until TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (application_id, created_at)
);
```

## Open Questions

- **Benchmark comparison**: Should PipelineMetrics show the user how they compare to anonymized aggregate baselines (5.75% tailored response rate, 68.5-day median offer timeline)? This is motivating but risks demoralizing users with poor early metrics.
- **Notification delivery outside TUI**: LazyJob is terminal-only. For users who want reminders when the TUI isn't open, could we write due reminders to a file (`~/.lazyjob/reminders.txt`) that a cron job or shell integration reads? Or is this out of scope?
- **Privacy mode and digest**: If `privacy_mode = true`, company names are omitted from the digest. Should the digest also be suppressible entirely? (Some users may not want even aggregate stats echoed to their terminal.)
- **Offer comparison**: The `offers` table stores individual offers. When multiple offers exist, PipelineMetrics could compute a `best_offer_total_comp` summary. This may overlap with `salary-market-intelligence.md` scope.

## Implementation Tasks

- [ ] Implement `PipelineMetrics` struct and `MetricsService::compute` in `lazyjob-core/src/application/metrics.rs`
- [ ] Implement `MetricsService::list_stale` with configurable threshold in `lazyjob-core/src/application/metrics.rs`
- [ ] Implement `MetricsService::list_action_required` returning typed `ActionItem` variants in `lazyjob-core/src/application/metrics.rs`
- [ ] Implement `DigestService::generate_daily_digest` and `should_show_today` in `lazyjob-core/src/application/digest.rs`
- [ ] Implement `ReminderPoller` tokio background task in `lazyjob-core/src/application/reminders.rs`
- [ ] Add `digest_log` and `application_snoozes` tables to `lazyjob-core/migrations/002_applications.sql`
- [ ] Build TUI action required queue view in `lazyjob-tui/src/views/action_queue.rs` — ordered action list with per-item keybindings (act/snooze/dismiss)
- [ ] Build TUI metrics view in `lazyjob-tui/src/views/pipeline_metrics.rs` — funnel chart, stage distribution, stale list, velocity table
- [ ] Wire morning digest print-before-TUI in `lazyjob-cli/src/main.rs`
