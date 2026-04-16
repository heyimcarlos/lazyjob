# Research: Task 10 — TUI App Loop

## Task Description
Implement App struct, App::run() with tokio::select! event loop, crossterm EventStream, tick interval (250ms), ralph broadcast channel. Handle alternate screen enter/leave. Wire `tui` CLI subcommand to App::run().

## Spec Reference
- `specs/09-tui-design-keybindings-implementation-plan.md` — full TUI architecture

## Key Findings

### Dependencies Needed
- `ratatui = "0.29"` — TUI framework (widgets, layout, terminal)
- `crossterm = { version = "0.28", features = ["event-stream"] }` — terminal backend, async event stream
- `futures = "0.3"` — StreamExt for EventStream.next()
- `tokio-stream` not needed — crossterm's event-stream feature provides its own async stream

### Existing Codebase State
- lazyjob-tui currently only has a `version()` fn in lib.rs
- lazyjob-cli has a stub `tui` subcommand printing "coming soon"
- Database, Config, and all core types are fully implemented
- tokio::sync::broadcast will be used for Ralph events (task description says broadcast channel)

### Architecture Decisions
1. **App struct** holds: active_view (ViewId enum), config, should_quit flag, ralph broadcast receiver
2. **No database in App for now** — The task says "database handle" but we don't want to require a running PG just to launch the TUI. App will accept an optional database handle. For this task, we'll accept a PgPool parameter since future views need it.
3. **tokio::sync::broadcast** for Ralph events — The spec says crossbeam_channel but tokio broadcast is more natural in async context and matches the task description.
4. **250ms tick** as specified in the task (not 16ms/60fps from spec).
5. **Minimal views** — This task only sets up the App loop. Task 11 will add view stubs. We'll create a placeholder render that shows the active view name.
6. **Terminal cleanup on Drop** — Implement Drop on a TerminalGuard to ensure raw mode and alternate screen are always cleaned up.

### ratatui 0.29 API Notes
- `Terminal::new(CrosstermBackend::new(stdout()))` creates terminal
- `terminal.draw(|frame| { ... })` renders a frame
- `Frame::area()` returns full terminal Rect
- `ratatui::backend::TestBackend::new(w, h)` for testing
- `crossterm::event::EventStream` requires `event-stream` feature on crossterm
- `ListState::select_next()` and `select_previous()` are available in ratatui 0.29

### What This Task Does NOT Include
- View implementations (task 11)
- Keybinding system (task 12)
- Custom widgets (task 13)
- Data loading from DB in views
