# Plan: Task 29 — Applications Kanban TUI

## Files to Create/Modify

### Modified
1. `crates/lazyjob-tui/src/action.rs` — Add `TransitionApplication(ApplicationId, ApplicationStage)` and `ScrollLeft`/`ScrollRight` variants
2. `crates/lazyjob-tui/src/views/applications.rs` — Full rewrite: kanban board implementation
3. `crates/lazyjob-tui/src/app.rs` — Add `load_applications()`, handle `TransitionApplication` action
4. `crates/lazyjob-tui/src/lib.rs` — Call `load_applications()` on startup
5. `crates/lazyjob-tui/src/event_loop.rs` — Call `load_applications()` on Refresh

### No new files needed

## Types/Structs to Define

### In applications.rs:
- `ApplicationCard` — denormalized card: application_id, job_id, title, company, stage, days_in_stage
- `KanbanColumn` — stage + vec of card indices
- `ConfirmState` — tracks pending confirmation: app_id, from_stage, to_stage, confirm_selected
- `ApplicationsView` — full kanban state: cards, focused_column, focused_card, confirming

### In action.rs:
- `Action::TransitionApplication(ApplicationId, ApplicationStage)` — emitted on confirmed stage transition
- `Action::ScrollLeft` / `Action::ScrollRight` — for h/l column navigation

## Tests to Write

### Unit tests (in applications.rs):
1. `new_creates_empty_view` — default state
2. `set_applications_populates_columns` — verify cards grouped by stage
3. `handle_key_h_moves_column_left` — column navigation
4. `handle_key_l_moves_column_right` — column navigation
5. `handle_key_j_moves_card_down` — card navigation within column
6. `handle_key_k_moves_card_up` — card navigation within column
7. `handle_key_m_opens_confirm_dialog` — forward stage transition
8. `handle_key_m_no_op_on_empty_column` — edge case
9. `confirm_yes_returns_transition_action` — confirm dialog flow
10. `confirm_no_closes_dialog` — cancel flow
11. `handle_key_esc_closes_confirm` — escape from confirm
12. `renders_without_panic` — basic render test
13. `renders_columns_with_stage_names` — verify column headers
14. `renders_cards_with_title_and_company` — verify card content
15. `days_color_green_under_7` — coloring
16. `days_color_yellow_7_to_14` — coloring
17. `days_color_red_over_14` — coloring
18. `focused_column_highlighted` — visual focus indicator

## Migrations
None needed — uses existing `applications` and `jobs` tables.

## Implementation Notes
- Kanban columns use `Constraint::Ratio(1, N)` for equal-width columns where N = number of non-empty stages + always show all 9
- Each column: header (stage name + count), scrollable card list
- Cards show: title (truncated), company, days badge with color
- Confirmation overlay uses existing `ConfirmDialog` widget
- Days in stage: `(Utc::now() - updated_at).num_days()`
- forward_stage() picks the first non-terminal entry from valid_transitions()
