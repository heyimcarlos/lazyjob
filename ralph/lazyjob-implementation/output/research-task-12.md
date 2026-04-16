# Research: Task 12 — tui-keybindings

## Current State

- Global keys hardcoded in `event_loop.rs::map_global_key()` — a match statement
- Per-view keys handled by `View::handle_key()` trait method (all stubs return None)
- HelpOverlay has hardcoded keybinding text
- Config already has `keybindings: HashMap<String, String>` for overrides

## Key Design Decisions

### KeyCombo normalization
crossterm sends `?` as `KeyCode::Char('?')` with `KeyModifiers::SHIFT`. Must normalize: for `KeyCode::Char(_)`, strip SHIFT since the character already encodes it. This ensures `KeyCombo::plain(KeyCode::Char('?'))` matches both `(Char('?'), NONE)` and `(Char('?'), SHIFT)`.

### KeyMap resolution order
1. If help overlay open → route to help overlay
2. KeyMap.resolve(context, combo) → checks context-specific first, then Global fallback
3. If no match → fall through to View::handle_key() for future custom handling

### Action variants needed
- `ScrollDown` — j/Down arrow
- `ScrollUp` — k/Up arrow
- `Select` — Enter

`Esc→Back` already exists as `NavigateBack` in Global context.

### Config overrides format
Config.keybindings is `HashMap<String, String>` with `"action_name" = "key_combo_string"`. Applied as global overrides. Parse key strings like "ctrl+r", "j", "enter", "shift+tab".

### HelpOverlay
Render dynamically from KeyMap using `bindings_for_context()`. Group by Global + active context.

## Dependencies
No new crates needed. Uses existing crossterm, ratatui, std::collections.
