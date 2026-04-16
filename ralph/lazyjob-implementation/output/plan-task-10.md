# Plan: Task 10 — TUI App Loop

## Files to Create/Modify

### New Files
- `lazyjob-tui/src/app.rs` — App struct, ViewId enum, InputMode enum, App::new(), App::run()
- `lazyjob-tui/src/action.rs` — Action enum (Quit, NavigateTo, NavigateBack, ToggleHelp, Refresh)
- `lazyjob-tui/src/event_loop.rs` — run_event_loop() with tokio::select!, terminal setup/teardown
- `lazyjob-tui/src/theme.rs` — Theme struct with DARK constant
- `lazyjob-tui/src/layout.rs` — AppLayout with header/body/status_bar Rect computation
- `lazyjob-tui/src/render.rs` — Top-level render() function, header tabs, status bar

### Modified Files
- `Cargo.toml` — Add ratatui, crossterm, futures to workspace deps
- `lazyjob-tui/Cargo.toml` — Add ratatui, crossterm, futures deps
- `lazyjob-tui/src/lib.rs` — Declare modules, pub async fn run() entry point
- `lazyjob-cli/Cargo.toml` — May need lazyjob-core config import
- `lazyjob-cli/src/main.rs` — Wire `tui` subcommand to lazyjob_tui::run()

## Types/Functions/Structs

### app.rs
- `ViewId` enum: Dashboard, Jobs, Applications, Contacts, Ralph, Settings
- `InputMode` enum: Normal, Insert, Search, Command
- `App` struct: active_view, prev_view, should_quit, help_open, input_mode, config, db (PgPool), ralph_rx (broadcast::Receiver)
- `App::new(config, pool, ralph_rx)` — constructor
- `App::handle_action(action)` — process Action enum

### action.rs
- `Action` enum: Quit, NavigateTo(ViewId), NavigateBack, ToggleHelp, Refresh

### event_loop.rs
- `run_event_loop(app)` — tokio::select! over crossterm EventStream + tick interval + ralph broadcast
- `TerminalGuard` struct with Drop impl for cleanup

### theme.rs
- `Theme` struct with color constants
- `Theme::DARK` const
- Helper style methods: selected_style(), status_bar_style(), focused_border_style()

### layout.rs
- `AppLayout` struct: header Rect, body Rect, status_bar Rect
- `AppLayout::compute(frame_size: Rect) -> Self`
- Constants: HEADER_HEIGHT, STATUS_BAR_HEIGHT

### render.rs
- `render(frame, app)` — top-level render dispatching to header/body/status_bar
- `render_header(frame, area, app)` — Tabs widget
- `render_status_bar(frame, area, app)` — status line

## Tests

### Learning Tests
- `ratatui_test_backend_renders_paragraph` — proves TestBackend captures rendered text
- `ratatui_layout_splits_correctly` — proves Layout::default().constraints() produces expected Rect sizes
- `crossterm_key_event_constructible` — proves KeyEvent can be constructed for testing

### Unit Tests
- `view_id_tab_index_round_trips` — ViewId to tab index and back
- `app_layout_dimensions` — AppLayout::compute produces correct header/body/status_bar heights
- `theme_dark_has_colors` — Theme::DARK fields are populated
- `action_quit_sets_should_quit` — App::handle_action(Quit) sets should_quit
- `action_navigate_sets_view` — App::handle_action(NavigateTo) changes active_view
- `action_navigate_back_restores_prev` — NavigateBack restores prev_view
- `action_toggle_help` — ToggleHelp flips help_open

### Integration Tests
- `render_without_panic` — App renders to TestBackend without panicking

## Migrations
None needed.

## Dependencies to Add
- ratatui 0.29 (workspace)
- crossterm 0.28 with event-stream feature (workspace)
- futures 0.3 (workspace)
