# Plan: Task 12 — tui-keybindings

## Files to Create
- `lazyjob-tui/src/keybindings.rs` — KeyCombo, KeyContext, KeyMap, parse_key_combo, parse_action

## Files to Modify
- `lazyjob-tui/src/lib.rs` — add `pub mod keybindings`
- `lazyjob-tui/src/action.rs` — add ScrollDown, ScrollUp, Select variants + action_name()
- `lazyjob-tui/src/app.rs` — add keymap field, handle new actions
- `lazyjob-tui/src/event_loop.rs` — replace map_global_key with keymap.resolve()
- `lazyjob-tui/src/views/help_overlay.rs` — render dynamically from KeyMap

## Types/Functions

### keybindings.rs
- `KeyCombo { code: KeyCode, modifiers: KeyModifiers }` with plain(), ctrl(), from_key_event(), Display
- `KeyContext` enum: Global, Dashboard, Jobs, Applications, Contacts, Ralph, Settings
- `KeyMap { bindings: HashMap<(KeyContext, KeyCombo), Action> }`
  - `default_keymap() -> Self`
  - `resolve(&self, ctx: &KeyContext, combo: &KeyCombo) -> Option<&Action>`
  - `with_overrides(self, overrides: &HashMap<String, String>) -> Self`
  - `bindings_for_context(&self, ctx: &KeyContext) -> Vec<(String, String)>` (key_display, action_name)
- `parse_key_combo(s: &str) -> Option<KeyCombo>`
- `parse_action(s: &str) -> Option<Action>`

### action.rs additions
- `ScrollDown`, `ScrollUp`, `Select` variants
- `Action::name(&self) -> &str` for display in help overlay

## Tests
- KeyCombo::from_key_event normalizes SHIFT for char keys
- KeyCombo Display formatting
- KeyMap::default_keymap resolves all expected bindings
- KeyMap::resolve falls back to Global
- KeyMap::with_overrides replaces a binding
- parse_key_combo parses "ctrl+r", "j", "enter", "shift+tab", etc.
- parse_action parses "quit", "scroll_down", etc.
- event_loop tests updated for new keymap-based dispatch
- HelpOverlay renders dynamic content from keymap

## No new crates needed
