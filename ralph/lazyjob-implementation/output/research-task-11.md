# Research: Task 11 — tui-views-stubs

## Current State
- TUI has App struct with basic state (active_view, should_quit, help_open, input_mode, theme, config, ralph_rx)
- ViewId enum: Dashboard, Jobs, Applications, Contacts, Ralph, Settings (6 tabs)
- render.rs has monolithic render() -> render_header + render_body (placeholder paragraph) + render_status_bar
- event_loop.rs handles global keybindings (q, ?, 1-6, Esc, Ctrl+R, Ctrl+C)
- No views/ directory, no View trait, no per-view state

## What Needs to Change
1. Define a View trait with render() and handle_key() methods
2. Create 8 stub view structs implementing View
3. App needs to own view instances and dispatch rendering/keys to active view
4. render.rs body section dispatches to the active view instead of a placeholder

## Design Decisions
- View trait takes `&mut self` for render (views may need to update scroll state etc.)
- handle_key returns Option<Action> — None means the key is unhandled, falls through to global bindings
- Views are stored in a Views struct on App, not in a HashMap — static dispatch, no allocation
- HelpOverlay is an overlay, not a tab view — rendered on top when help_open is true
- JobDetailView is not routed via ViewId (no detail variant yet) — exists as a type for future use
- Stubs render a centered message with view name and hint text

## Dependencies
- No new crates needed — ratatui, crossterm already available
- View trait uses types from ratatui (Frame, Rect) and crossterm (KeyCode, KeyModifiers)
