# Research: Task 13 — tui-widgets

## Status of existing widgets

All five widgets from the task spec exist. Four are fully implemented; one is missing:

| Widget | File | Status |
|--------|------|--------|
| `JobCard` | `widgets/job_card.rs` | Complete |
| `ModalDialog` | `widgets/modal_dialog.rs` | Complete — includes `pub centered_rect()` helper |
| `StatBlock` | `widgets/stat_block.rs` | Complete |
| `ProgressBar` | `widgets/progress_bar.rs` | Complete |
| `ConfirmDialog` | `widgets/confirm_dialog.rs` | **Missing** — declared in mod.rs but file absent |

## Key gap: `lib.rs` missing `pub mod widgets`

`lib.rs` has no `pub mod widgets;`. All widget tests currently excluded from build.
Fix: add `pub mod widgets;` to lib.rs.

## Widget pattern (all four existing files follow this)

1. Private fields + `::new()` + builder methods
2. `impl Widget for T { fn render(self, area, buf) }` — consumes self
3. Guard zero-height/width early
4. Inner ratatui widgets write to buf
5. `#[cfg(test)]` block with TestBackend + `buffer_text()` helper

## ConfirmDialog design

- Title + body text (reuses centered_rect from modal_dialog)
- Two buttons: `[ Yes ]` / `[ No ]`
- `confirm_selected: bool` — which button is highlighted
- Selected button: `theme.primary` fg + BOLD; inactive: `theme.text_muted` fg
- Layout: body paragraph + 1-row button bar at bottom of inner area
