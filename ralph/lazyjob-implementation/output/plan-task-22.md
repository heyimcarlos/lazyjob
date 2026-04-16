# Plan: Task 22 — ralph-tui-panel

## Files to Create/Modify

1. **`crates/lazyjob-tui/src/views/ralph_panel.rs`** — full rewrite
2. **`crates/lazyjob-tui/src/action.rs`** — add `CancelRalphLoop(String)`, `RalphDetail(String)` variants
3. **`crates/lazyjob-tui/src/app.rs`** — fill in `handle_ralph_update`, handle new action variants

## Types/Functions to Define

### ralph_panel.rs
```rust
struct ActiveEntry {
    run_id: String,
    loop_type: String,
    phase: String,
    progress: f64,          // 0.0-1.0
    message: String,
    started_at: Instant,
    log_lines: Vec<String>, // last N lines capped at 50
}

struct CompletedEntry {
    run_id: String,
    loop_type: String,
    success: bool,
    completed_at: Instant,
    summary: String,        // last log line or reason
}

pub struct RalphPanelView {
    active: Vec<ActiveEntry>,
    completed: Vec<CompletedEntry>,
    selected: usize,
}

impl RalphPanelView {
    pub fn new() -> Self { ... }
    pub fn on_update(&mut self, update: RalphUpdate) { ... }
    fn cleanup_expired(&mut self) { ... }  // removes completed > 5s old
    fn selected_run_id(&self) -> Option<String> { ... }
}

impl View for RalphPanelView { ... }
```

### action.rs additions
```rust
Action::CancelRalphLoop(String),  // run_id
Action::RalphDetail(String),      // run_id
```

### app.rs changes
```rust
pub fn handle_ralph_update(&mut self, update: RalphUpdate) {
    self.views.ralph_panel.on_update(update);
}

// In handle_action:
Action::CancelRalphLoop(_) | Action::RalphDetail(_) => {}  // wired in task 36
```

## Render Layout

```
┌─ Ralph ────────────────────────────────────────────────────────┐
│ [Active: 2]  [Completed: 1]                                    │
│                                                                │
│ ▶ resume-tailor  [Analyzing JD]  0:42                         │
│   ████████████████░░░░░░░░░░░░░░ 55% Analyzing JD             │
│ ─────────────────────────────────────────────────────────────  │
│   job-discovery  [Fetching Lever]  1:23                       │
│   ████████░░░░░░░░░░░░░░░░░░░░░░ 28% Fetching jobs            │
│ ─────────────────────────────────────────────────────────────  │
│ ✓ cover-letter  Completed 3s ago                              │
│                                                                │
│ j/k: navigate  c: cancel  Enter: detail  Esc: back            │
└────────────────────────────────────────────────────────────────┘
```

## Tests to Write

### Learning tests
None required — no new external crates. `Instant` and `Duration` are std; ratatui is proven in tasks 10-13.

### Unit tests
1. `on_update_progress_creates_active_entry` — Progress update for unknown id creates new entry
2. `on_update_progress_updates_existing` — Progress update for known id updates phase/percent
3. `on_update_logline_appends_to_entry` — LogLine appended to matching active entry
4. `on_update_completed_moves_to_completed` — Completed removes from active, adds to completed
5. `on_update_failed_moves_to_completed` — Failed removes from active, adds completed with success=false
6. `cleanup_removes_expired_completed` — completed entries > 5s old are removed
7. `selected_run_id_returns_none_for_empty` — no panic on empty active list
8. `handle_key_j_scrolls_down` — j key returns ScrollDown (or None if at bottom)
9. `handle_key_c_returns_cancel_action` — c key returns Action::CancelRalphLoop with correct id
10. `renders_active_loops_with_progress_bar` — TestBackend test: active entries render progress bars
11. `renders_empty_state_message` — TestBackend test: empty view shows "No active loops" message
12. `renders_completed_with_checkmark` — TestBackend test: completed entry shows ✓
