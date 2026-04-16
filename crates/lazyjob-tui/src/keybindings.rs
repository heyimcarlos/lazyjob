use std::collections::HashMap;
use std::fmt;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::action::{Action, ViewId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyCombo {
    pub fn plain(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub fn from_key_event(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let normalized = match code {
            KeyCode::Char(_) => modifiers - KeyModifiers::SHIFT,
            _ => modifiers,
        };
        Self {
            code,
            modifiers: normalized,
        }
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        let key_str = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            _ => "?".to_string(),
        };
        parts.push(key_str);
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyContext {
    Global,
    Dashboard,
    Jobs,
    Applications,
    Contacts,
    Ralph,
    Settings,
}

impl KeyContext {
    pub fn from_view_id(view_id: ViewId) -> Self {
        match view_id {
            ViewId::Dashboard => Self::Dashboard,
            ViewId::Jobs => Self::Jobs,
            ViewId::Applications => Self::Applications,
            ViewId::Contacts => Self::Contacts,
            ViewId::Ralph => Self::Ralph,
            ViewId::Settings => Self::Settings,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Dashboard => "Dashboard",
            Self::Jobs => "Jobs",
            Self::Applications => "Applications",
            Self::Contacts => "Contacts",
            Self::Ralph => "Ralph",
            Self::Settings => "Settings",
        }
    }
}

pub struct KeyMap {
    bindings: HashMap<(KeyContext, KeyCombo), Action>,
}

impl KeyMap {
    pub fn default_keymap() -> Self {
        let mut m = HashMap::new();

        m.insert(
            (KeyContext::Global, KeyCombo::plain(KeyCode::Char('q'))),
            Action::Quit,
        );
        m.insert((KeyContext::Global, KeyCombo::ctrl('c')), Action::Quit);
        m.insert(
            (KeyContext::Global, KeyCombo::plain(KeyCode::Char('?'))),
            Action::ToggleHelp,
        );
        m.insert(
            (KeyContext::Global, KeyCombo::plain(KeyCode::Esc)),
            Action::NavigateBack,
        );
        m.insert((KeyContext::Global, KeyCombo::ctrl('r')), Action::Refresh);

        for i in 1u32..=6 {
            let c = char::from_digit(i, 10).unwrap();
            if let Some(view_id) = ViewId::from_tab_index((i - 1) as usize) {
                m.insert(
                    (KeyContext::Global, KeyCombo::plain(KeyCode::Char(c))),
                    Action::NavigateTo(view_id),
                );
            }
        }

        let view_contexts = [
            KeyContext::Dashboard,
            KeyContext::Jobs,
            KeyContext::Applications,
            KeyContext::Contacts,
            KeyContext::Ralph,
            KeyContext::Settings,
        ];

        for ctx in &view_contexts {
            m.insert(
                (ctx.clone(), KeyCombo::plain(KeyCode::Char('j'))),
                Action::ScrollDown,
            );
            m.insert(
                (ctx.clone(), KeyCombo::plain(KeyCode::Down)),
                Action::ScrollDown,
            );
            m.insert(
                (ctx.clone(), KeyCombo::plain(KeyCode::Char('k'))),
                Action::ScrollUp,
            );
            m.insert(
                (ctx.clone(), KeyCombo::plain(KeyCode::Up)),
                Action::ScrollUp,
            );
            m.insert(
                (ctx.clone(), KeyCombo::plain(KeyCode::Enter)),
                Action::Select,
            );
        }

        Self { bindings: m }
    }

    pub fn resolve(&self, ctx: &KeyContext, combo: &KeyCombo) -> Option<&Action> {
        self.bindings
            .get(&(ctx.clone(), combo.clone()))
            .or_else(|| self.bindings.get(&(KeyContext::Global, combo.clone())))
    }

    pub fn with_overrides(mut self, overrides: &HashMap<String, String>) -> Self {
        for (action_name, key_str) in overrides {
            if let (Some(action), Some(combo)) =
                (parse_action(action_name), parse_key_combo(key_str))
            {
                self.bindings.insert((KeyContext::Global, combo), action);
            }
        }
        self
    }

    pub fn bindings_for_context(&self, ctx: &KeyContext) -> Vec<(String, String)> {
        let mut result: Vec<(String, String)> = self
            .bindings
            .iter()
            .filter(|((c, _), _)| c == ctx)
            .map(|((_, combo), action)| (combo.to_string(), action.name().to_string()))
            .collect();
        result.sort_by(|(a, _), (b, _)| a.cmp(b));
        result
    }
}

pub fn parse_key_combo(s: &str) -> Option<KeyCombo> {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();

    let mut modifiers = KeyModifiers::NONE;
    let key_part = parts.last()?;

    for &part in &parts[..parts.len() - 1] {
        match part {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" => modifiers |= KeyModifiers::ALT,
            _ => return None,
        }
    }

    let code = match *key_part {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        s if s.starts_with('f') && s.len() >= 2 => {
            let n: u8 = s[1..].parse().ok()?;
            if n == 0 || n > 12 {
                return None;
            }
            KeyCode::F(n)
        }
        s if s.len() == 1 => KeyCode::Char(s.chars().next().unwrap()),
        _ => return None,
    };

    Some(KeyCombo { code, modifiers })
}

pub fn parse_action(s: &str) -> Option<Action> {
    match s.trim().to_lowercase().as_str() {
        "quit" => Some(Action::Quit),
        "toggle_help" | "help" => Some(Action::ToggleHelp),
        "navigate_back" | "back" => Some(Action::NavigateBack),
        "refresh" => Some(Action::Refresh),
        "scroll_down" | "down" => Some(Action::ScrollDown),
        "scroll_up" | "up" => Some(Action::ScrollUp),
        "select" => Some(Action::Select),
        "dashboard" => Some(Action::NavigateTo(ViewId::Dashboard)),
        "jobs" => Some(Action::NavigateTo(ViewId::Jobs)),
        "applications" => Some(Action::NavigateTo(ViewId::Applications)),
        "contacts" => Some(Action::NavigateTo(ViewId::Contacts)),
        "ralph" => Some(Action::NavigateTo(ViewId::Ralph)),
        "settings" => Some(Action::NavigateTo(ViewId::Settings)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_key_event_normalizes_shift_for_chars() {
        let combo = KeyCombo::from_key_event(KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(combo, KeyCombo::plain(KeyCode::Char('?')));
    }

    #[test]
    fn from_key_event_preserves_shift_for_non_chars() {
        let combo = KeyCombo::from_key_event(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(
            combo,
            KeyCombo {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::SHIFT
            }
        );
    }

    #[test]
    fn from_key_event_preserves_ctrl() {
        let combo = KeyCombo::from_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(combo, KeyCombo::ctrl('r'));
    }

    #[test]
    fn display_plain_char() {
        assert_eq!(KeyCombo::plain(KeyCode::Char('q')).to_string(), "q");
    }

    #[test]
    fn display_ctrl_combo() {
        assert_eq!(KeyCombo::ctrl('r').to_string(), "Ctrl+r");
    }

    #[test]
    fn display_special_keys() {
        assert_eq!(KeyCombo::plain(KeyCode::Enter).to_string(), "Enter");
        assert_eq!(KeyCombo::plain(KeyCode::Esc).to_string(), "Esc");
        assert_eq!(KeyCombo::plain(KeyCode::Down).to_string(), "Down");
        assert_eq!(KeyCombo::plain(KeyCode::F(1)).to_string(), "F1");
        assert_eq!(KeyCombo::plain(KeyCode::Char(' ')).to_string(), "Space");
    }

    #[test]
    fn default_keymap_quit_resolves_globally() {
        let km = KeyMap::default_keymap();
        let combo = KeyCombo::plain(KeyCode::Char('q'));
        assert_eq!(km.resolve(&KeyContext::Global, &combo), Some(&Action::Quit));
    }

    #[test]
    fn default_keymap_quit_resolves_from_any_context() {
        let km = KeyMap::default_keymap();
        let combo = KeyCombo::plain(KeyCode::Char('q'));
        assert_eq!(km.resolve(&KeyContext::Jobs, &combo), Some(&Action::Quit));
        assert_eq!(
            km.resolve(&KeyContext::Settings, &combo),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn default_keymap_tab_switching() {
        let km = KeyMap::default_keymap();
        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::plain(KeyCode::Char('1'))),
            Some(&Action::NavigateTo(ViewId::Dashboard))
        );
        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::plain(KeyCode::Char('2'))),
            Some(&Action::NavigateTo(ViewId::Jobs))
        );
        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::plain(KeyCode::Char('6'))),
            Some(&Action::NavigateTo(ViewId::Settings))
        );
    }

    #[test]
    fn default_keymap_per_view_scroll() {
        let km = KeyMap::default_keymap();
        let j = KeyCombo::plain(KeyCode::Char('j'));
        let k = KeyCombo::plain(KeyCode::Char('k'));
        let down = KeyCombo::plain(KeyCode::Down);
        let enter = KeyCombo::plain(KeyCode::Enter);

        assert_eq!(km.resolve(&KeyContext::Jobs, &j), Some(&Action::ScrollDown));
        assert_eq!(km.resolve(&KeyContext::Jobs, &k), Some(&Action::ScrollUp));
        assert_eq!(
            km.resolve(&KeyContext::Dashboard, &down),
            Some(&Action::ScrollDown)
        );
        assert_eq!(
            km.resolve(&KeyContext::Ralph, &enter),
            Some(&Action::Select)
        );
    }

    #[test]
    fn resolve_context_specific_before_global() {
        let mut m = HashMap::new();
        m.insert(
            (KeyContext::Global, KeyCombo::plain(KeyCode::Enter)),
            Action::Refresh,
        );
        m.insert(
            (KeyContext::Jobs, KeyCombo::plain(KeyCode::Enter)),
            Action::Select,
        );
        let km = KeyMap { bindings: m };

        assert_eq!(
            km.resolve(&KeyContext::Jobs, &KeyCombo::plain(KeyCode::Enter)),
            Some(&Action::Select)
        );
        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::plain(KeyCode::Enter)),
            Some(&Action::Refresh)
        );
    }

    #[test]
    fn resolve_falls_back_to_global() {
        let km = KeyMap::default_keymap();
        let combo = KeyCombo::plain(KeyCode::Char('?'));
        assert_eq!(
            km.resolve(&KeyContext::Applications, &combo),
            Some(&Action::ToggleHelp)
        );
    }

    #[test]
    fn resolve_unbound_returns_none() {
        let km = KeyMap::default_keymap();
        let combo = KeyCombo::plain(KeyCode::Char('z'));
        assert_eq!(km.resolve(&KeyContext::Global, &combo), None);
    }

    #[test]
    fn with_overrides_replaces_binding() {
        let mut overrides = HashMap::new();
        overrides.insert("quit".to_string(), "ctrl+q".to_string());

        let km = KeyMap::default_keymap().with_overrides(&overrides);

        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::ctrl('q')),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn with_overrides_ignores_invalid() {
        let mut overrides = HashMap::new();
        overrides.insert("nonexistent_action".to_string(), "x".to_string());
        overrides.insert("quit".to_string(), "".to_string());

        let km = KeyMap::default_keymap().with_overrides(&overrides);
        assert_eq!(
            km.resolve(&KeyContext::Global, &KeyCombo::plain(KeyCode::Char('q'))),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn parse_key_combo_simple_char() {
        assert_eq!(
            parse_key_combo("j"),
            Some(KeyCombo::plain(KeyCode::Char('j')))
        );
    }

    #[test]
    fn parse_key_combo_ctrl() {
        assert_eq!(parse_key_combo("ctrl+r"), Some(KeyCombo::ctrl('r')));
    }

    #[test]
    fn parse_key_combo_special_keys() {
        assert_eq!(
            parse_key_combo("enter"),
            Some(KeyCombo::plain(KeyCode::Enter))
        );
        assert_eq!(parse_key_combo("esc"), Some(KeyCombo::plain(KeyCode::Esc)));
        assert_eq!(
            parse_key_combo("space"),
            Some(KeyCombo::plain(KeyCode::Char(' ')))
        );
        assert_eq!(parse_key_combo("tab"), Some(KeyCombo::plain(KeyCode::Tab)));
        assert_eq!(parse_key_combo("f1"), Some(KeyCombo::plain(KeyCode::F(1))));
        assert_eq!(parse_key_combo("up"), Some(KeyCombo::plain(KeyCode::Up)));
    }

    #[test]
    fn parse_key_combo_shift_tab() {
        assert_eq!(
            parse_key_combo("shift+tab"),
            Some(KeyCombo {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::SHIFT,
            })
        );
    }

    #[test]
    fn parse_key_combo_case_insensitive() {
        assert_eq!(parse_key_combo("Ctrl+R"), Some(KeyCombo::ctrl('r')));
        assert_eq!(
            parse_key_combo("ENTER"),
            Some(KeyCombo::plain(KeyCode::Enter))
        );
    }

    #[test]
    fn parse_key_combo_invalid() {
        assert_eq!(parse_key_combo(""), None);
        assert_eq!(parse_key_combo("ctrl+"), None);
        assert_eq!(parse_key_combo("foo+bar"), None);
    }

    #[test]
    fn parse_action_valid() {
        assert_eq!(parse_action("quit"), Some(Action::Quit));
        assert_eq!(parse_action("toggle_help"), Some(Action::ToggleHelp));
        assert_eq!(parse_action("help"), Some(Action::ToggleHelp));
        assert_eq!(parse_action("back"), Some(Action::NavigateBack));
        assert_eq!(parse_action("refresh"), Some(Action::Refresh));
        assert_eq!(parse_action("scroll_down"), Some(Action::ScrollDown));
        assert_eq!(parse_action("scroll_up"), Some(Action::ScrollUp));
        assert_eq!(parse_action("select"), Some(Action::Select));
        assert_eq!(
            parse_action("dashboard"),
            Some(Action::NavigateTo(ViewId::Dashboard))
        );
        assert_eq!(parse_action("jobs"), Some(Action::NavigateTo(ViewId::Jobs)));
    }

    #[test]
    fn parse_action_case_insensitive() {
        assert_eq!(parse_action("QUIT"), Some(Action::Quit));
        assert_eq!(parse_action("Refresh"), Some(Action::Refresh));
    }

    #[test]
    fn parse_action_invalid() {
        assert_eq!(parse_action("nonexistent"), None);
        assert_eq!(parse_action(""), None);
    }

    #[test]
    fn bindings_for_context_returns_sorted() {
        let km = KeyMap::default_keymap();
        let bindings = km.bindings_for_context(&KeyContext::Global);
        assert!(!bindings.is_empty());

        let keys: Vec<&str> = bindings.iter().map(|(k, _)| k.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn bindings_for_context_contains_expected() {
        let km = KeyMap::default_keymap();
        let global = km.bindings_for_context(&KeyContext::Global);
        let action_names: Vec<&str> = global.iter().map(|(_, a)| a.as_str()).collect();
        assert!(action_names.contains(&"Quit"));
        assert!(action_names.contains(&"Toggle Help"));
        assert!(action_names.contains(&"Refresh"));
    }

    #[test]
    fn key_context_from_view_id() {
        assert_eq!(
            KeyContext::from_view_id(ViewId::Dashboard),
            KeyContext::Dashboard
        );
        assert_eq!(KeyContext::from_view_id(ViewId::Jobs), KeyContext::Jobs);
        assert_eq!(
            KeyContext::from_view_id(ViewId::Settings),
            KeyContext::Settings
        );
    }

    #[test]
    fn key_context_labels() {
        assert_eq!(KeyContext::Global.label(), "Global");
        assert_eq!(KeyContext::Jobs.label(), "Jobs");
    }
}
