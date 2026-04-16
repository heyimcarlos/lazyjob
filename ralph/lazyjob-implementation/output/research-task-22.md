# Research: Task 22 — ralph-tui-panel

## Existing Code State

### RalphPanelView (stub)
- Located at `crates/lazyjob-tui/src/views/ralph_panel.rs`
- Implements `View` trait with stub render (static placeholder text), no-op `handle_key`, and `name() -> "Ralph"`
- Zero-field unit struct — no state

### App (app.rs)
- `ralph_rx: broadcast::Receiver<RalphUpdate>` — receives TUI-side ralph events
- `handle_ralph_update(&mut self, _update: RalphUpdate)` — stub, does nothing
- No reference to `RalphProcessManager` (it's not wired to App yet)
- `views.ralph_panel: RalphPanelView` — accessed via `active_view_mut()` routing

### RalphUpdate enum (app.rs)
```rust
pub enum RalphUpdate {
    Progress { id: String, phase: String, percent: f64 },
    LogLine { id: String, line: String },
    Completed { id: String },
    Failed { id: String, reason: String },
}
```

### Action enum (action.rs)
Current variants: Quit, NavigateTo(ViewId), NavigateBack, ToggleHelp, Refresh, ScrollDown, ScrollUp, Select  
Missing: cancel-loop and detail-view actions for Ralph panel keybindings

### ProgressBar widget (widgets/progress_bar.rs)
`ProgressBar::new(ratio: f64, label: &str)` — renders `█` filled / `░` empty bar with `ratio` and a label suffix  
Color configurable via `.color(Color)` builder  
Implements `Widget` for ratatui

### WorkerEvent protocol (lazyjob-ralph/src/protocol.rs)
```rust
pub enum WorkerEvent {
    Status { phase: String, progress: f32, message: String },
    Results { data: serde_json::Value },
    Error { code: String, message: String },
    Done { success: bool },
}
```
The TUI-side `RalphUpdate` maps from these: Status→Progress/LogLine, Done→Completed, Error→Failed

### View trait (views/mod.rs)
```rust
pub trait View {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme);
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn name(&self) -> &'static str;
}
```
`render` takes `&mut self` — the view can mutate its own state during render (useful for cleanup of expired completed entries)

## Key Design Decisions

1. **State in the view struct**: `RalphPanelView` holds `Vec<ActiveEntry>` and `Vec<CompletedEntry>` and `selected: usize`. The App's `handle_ralph_update` dispatches to the view's `on_update()` method (not part of the `View` trait).

2. **Elapsed time**: Use `std::time::Instant` stored in `ActiveEntry::started_at`. Compute elapsed in render via `started_at.elapsed()`.

3. **5-second cleanup**: In `render()`, at the start, drain `completed` entries where `completed_at.elapsed() > Duration::from_secs(5)`.

4. **New Action variants**: Add `CancelRalphLoop(String)` and `RalphDetail(String)` to the `Action` enum. `handle_action` treats them as no-ops for now (full wiring in task 36 when process manager is integrated into App).

5. **No process manager integration yet**: The `RalphProcessManager` is not part of `App`. Cancel keybinding dispatches an Action that App ignores — infrastructure is ready for task 36.

6. **Layout**: Use `Layout::vertical` to split area: each active loop gets a 3-line row (title+elapsed line + progress bar line + separator). Completed entries get 1-line rows with ✓ or ✗.
