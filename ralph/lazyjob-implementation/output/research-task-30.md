# Research: Task 30 — Dashboard Stats

## Summary

The DashboardView needs to show job search statistics, pipeline counts, and stale application alerts. The current dashboard is a stub rendering static text.

## Key Findings

### Existing Infrastructure
- **View trait**: `render(&mut self, Frame, Rect, &Theme)`, `handle_key(KeyCode, KeyModifiers) -> Option<Action>`, `name() -> &'static str`
- **StatBlock widget**: `StatBlock::new(label, value, color).subtitle(sub)` — renders a bordered box with label, bold value, optional subtitle. Needs 4+ rows height.
- **ProgressBar widget**: `ProgressBar::new(ratio, label).color(color)` — single-row horizontal bar.
- **Data loading pattern**: `App::load_*(&mut self)` methods guard on `self.pool`, construct repo, query, call `self.views.<view>.set_*(data)`.
- **App.pool**: `Option<PgPool>` — None if DB connection failed at startup.
- **Refresh**: `Action::Refresh` triggers `load_jobs().await` + `load_applications().await` in event_loop.

### ApplicationStage
9 variants: Interested, Applied, PhoneScreen, Technical, Onsite, Offer, Accepted, Rejected, Withdrawn.
- `ApplicationStage::all()` returns all 9.
- `is_terminal()` for Accepted/Rejected/Withdrawn.
- Application default stage is Interested.

### DB Schema
- `applications` table: id, job_id, stage (TEXT), submitted_at, updated_at, etc.
- `jobs` table: id, title, company_name, match_score, ghost_score, discovered_at, etc.
- Index `idx_applications_stage ON applications(stage)` exists.

### Stats to Compute
Task description says:
1. **Top row StatBlocks**: Total Jobs, Applied This Week, In Pipeline, Interviews Scheduled
2. **Middle**: mini kanban column counts (per-stage)
3. **Bottom**: Actions Required list (stale apps >14d)

### Reminders
The task mentions `ReminderService::check_due()` and `ReminderPoller`, but there is NO reminders table in the DB schema. The spec's Phase 3 mentions a reminders table but it was never created. I'll skip the reminder/poller parts and focus on stale application detection which can work with existing tables.

### "Interviews Scheduled"
There is no interviews table in the DB. The task mentions it but it doesn't exist. I'll show 0 or compute from stage counts (applications in PhoneScreen/Technical/Onsite stages).

## Design Decisions

1. **DashboardStats struct** in lazyjob-core: holds computed stats from SQL queries. Pure data struct, no DB dependency.
2. **compute_dashboard_stats(pool) -> DashboardStats** as a standalone async function — queries jobs count, application counts per stage, stale apps, applied this week.
3. **StaleApplication struct**: application_id, job_title, company, days_stale — for the Actions Required list.
4. **No reminders table/service** — not needed for MVP dashboard. Stale detection uses `applications.updated_at < now() - 14 days` for non-terminal stages.
5. **No ReminderPoller** — dashboard data refreshes on startup and Ctrl+R like other views.
6. **DashboardView** stores DashboardStats + Vec<StaleApplication> as fields; `set_stats()` setter follows existing pattern.
