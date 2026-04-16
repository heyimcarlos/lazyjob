# Plan: Task 13 — tui-widgets

## Files to create/modify

1. `crates/lazyjob-tui/src/widgets/confirm_dialog.rs` — NEW: ConfirmDialog widget
2. `crates/lazyjob-tui/src/lib.rs` — ADD `pub mod widgets;`

## Types/functions to define

### `confirm_dialog.rs`

```
pub struct ConfirmDialog<'a> {
    title: &'a str,
    body: &'a str,
    confirm_selected: bool,   // true = Yes highlighted, false = No highlighted
    theme: &'a Theme,
    width_percent: u16,       // default 50
    height: u16,              // default 9
}

impl ConfirmDialog::new(title, body, theme) -> Self
impl ConfirmDialog::confirm_selected(bool) -> Self
impl ConfirmDialog::width_percent(u16) -> Self
impl Widget for ConfirmDialog<'_>
```

### Layout inside the dialog

```
block(title) → inner rect
inner: split vertically
  - Fill(1): body Paragraph
  - Length(1): button row "  [ Yes ]   [ No ]  "
```

## Tests to write

- `confirm_dialog_renders_without_panic` — basic smoke test
- `confirm_dialog_shows_title` — title appears in buffer
- `confirm_dialog_shows_body` — body text appears
- `confirm_dialog_shows_yes_no_buttons` — "Yes" and "No" appear
- `confirm_dialog_default_selects_yes` — default `confirm_selected = true`
- `confirm_dialog_no_selected_renders` — can select No without panic

## No migrations needed.
