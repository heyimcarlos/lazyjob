# Plan: Task 30 — Dashboard Stats

## Files to Create/Modify

### New Files
1. `crates/lazyjob-core/src/stats.rs` — DashboardStats struct, StaleApplication struct, compute_dashboard_stats() async fn

### Modified Files
2. `crates/lazyjob-core/src/lib.rs` — add `pub mod stats`
3. `crates/lazyjob-tui/src/views/dashboard.rs` — full rewrite with StatBlocks, kanban counts, stale list, scroll
4. `crates/lazyjob-tui/src/app.rs` — add `load_dashboard_stats()` method
5. `crates/lazyjob-tui/src/event_loop.rs` — call `load_dashboard_stats()` on Refresh
6. `crates/lazyjob-tui/src/lib.rs` — call `load_dashboard_stats()` on startup

## Types/Functions/Structs

### lazyjob-core::stats
```rust
pub struct DashboardStats {
    pub total_jobs: i64,
    pub applied_this_week: i64,
    pub in_pipeline: i64,        // non-terminal, non-Interested
    pub interviewing: i64,       // PhoneScreen + Technical + Onsite
    pub stage_counts: HashMap<ApplicationStage, i64>,
}

pub struct StaleApplication {
    pub application_id: ApplicationId,
    pub job_title: String,
    pub company: String,
    pub days_stale: i64,
}

pub async fn compute_dashboard_stats(pool: &PgPool) -> Result<DashboardStats>
pub async fn find_stale_applications(pool: &PgPool) -> Result<Vec<StaleApplication>>
```

### lazyjob-tui::views::dashboard
```rust
pub struct DashboardView {
    stats: DashboardStats,
    stale: Vec<StaleApplication>,
    selected_stale: usize,
}

impl DashboardView {
    pub fn set_stats(&mut self, stats: DashboardStats, stale: Vec<StaleApplication>)
}
```

## Tests

### Unit Tests (lazyjob-core::stats)
- `default_stats_are_zero` — DashboardStats::default() has all zeros
- `stale_application_fields` — StaleApplication construction

### Integration Tests (lazyjob-core::stats)
- `compute_stats_empty_db` — returns all zeros on empty DB
- `compute_stats_with_data` — insert jobs + applications, verify counts
- `find_stale_applications_returns_old_apps` — insert old app, verify it appears
- `find_stale_applications_ignores_terminal` — terminal apps aren't stale

### TUI Tests (lazyjob-tui::views::dashboard)
- `renders_without_panic` (existing, update)
- `renders_title_in_buffer` (existing, update)
- `renders_stat_blocks` — set stats, verify values in buffer
- `renders_stage_counts` — verify kanban counts visible
- `renders_stale_list` — set stale apps, verify in buffer
- `renders_empty_stale_message` — no stale apps shows friendly message
- `handle_key_j_scrolls_stale_list` — j/Down scrolls stale list selection
- `handle_key_k_scrolls_up` — k/Up scrolls up
- `set_stats_updates_data` — verify setter stores data

## Migrations
None needed — uses existing jobs and applications tables.
