# Plan: Task 11 — tui-views-stubs

## Files to Create
1. `lazyjob-tui/src/views/mod.rs` — View trait + Views container + re-exports
2. `lazyjob-tui/src/views/dashboard.rs` — DashboardView stub
3. `lazyjob-tui/src/views/jobs_list.rs` — JobsListView stub
4. `lazyjob-tui/src/views/job_detail.rs` — JobDetailView stub
5. `lazyjob-tui/src/views/applications.rs` — ApplicationsView stub
6. `lazyjob-tui/src/views/contacts.rs` — ContactsView stub
7. `lazyjob-tui/src/views/ralph_panel.rs` — RalphPanelView stub
8. `lazyjob-tui/src/views/settings.rs` — SettingsView stub
9. `lazyjob-tui/src/views/help_overlay.rs` — HelpOverlay

## Files to Modify
1. `lazyjob-tui/src/lib.rs` — add `pub mod views`
2. `lazyjob-tui/src/app.rs` — add Views struct to App, delegate handle_key to active view
3. `lazyjob-tui/src/render.rs` — dispatch body rendering to active view, render help overlay
4. `lazyjob-tui/src/event_loop.rs` — delegate unhandled keys to active view

## Types/Structs
- `View` trait: render(&mut self, frame, area, theme), handle_key(code, modifiers) -> Option<Action>
- `Views` struct: holds all view instances
- Each view struct: empty or minimal state, implements View
- HelpOverlay: render(frame, area, theme, active_view_name)

## Tests
- Each view renders without panic on TestBackend
- Each view's handle_key returns None for all keys (stubs)
- render.rs dispatches correctly to each view
- HelpOverlay renders with key hints visible
- Views container returns correct mutable reference for each ViewId

## Migrations
- None
