use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use sqlx::PgPool;
use tokio::sync::broadcast;

use lazyjob_core::config::Config;

use crate::action::{Action, ViewId};
use crate::keybindings::KeyMap;
use crate::theme::Theme;
use crate::views::Views;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,
    Search,
    Command,
}

#[derive(Debug, Clone)]
pub enum RalphUpdate {
    Progress {
        id: String,
        phase: String,
        percent: f64,
    },
    LogLine {
        id: String,
        line: String,
    },
    Completed {
        id: String,
    },
    Failed {
        id: String,
        reason: String,
    },
}

pub struct App {
    pub active_view: ViewId,
    pub prev_view: Option<ViewId>,
    pub should_quit: bool,
    pub help_open: bool,
    pub input_mode: InputMode,
    pub theme: &'static Theme,
    pub config: Arc<Config>,
    pub ralph_rx: broadcast::Receiver<RalphUpdate>,
    pub views: Views,
    pub keymap: KeyMap,
    pub pool: Option<PgPool>,
}

impl App {
    pub fn new(config: Arc<Config>, ralph_rx: broadcast::Receiver<RalphUpdate>) -> Self {
        let keymap = KeyMap::default_keymap().with_overrides(&config.keybindings);
        Self {
            active_view: ViewId::Dashboard,
            prev_view: None,
            should_quit: false,
            help_open: false,
            input_mode: InputMode::Normal,
            theme: &Theme::DARK,
            config,
            ralph_rx,
            views: Views::new(),
            keymap,
            pool: None,
        }
    }

    pub fn with_pool(mut self, pool: PgPool) -> Self {
        self.pool = Some(pool);
        self
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let (_tx, rx) = broadcast::channel(16);
        Self::new(Arc::new(Config::default()), rx)
    }

    pub fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.should_quit = true;
            }
            Action::NavigateTo(view) => {
                if self.active_view != view {
                    self.prev_view = Some(self.active_view);
                    self.active_view = view;
                }
            }
            Action::NavigateBack => {
                if let Some(prev) = self.prev_view.take() {
                    self.active_view = prev;
                }
            }
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
            }
            Action::Refresh => {}
            Action::ScrollDown => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Down, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::ScrollUp => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Up, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::Select => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Enter, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::CancelRalphLoop(_) | Action::RalphDetail(_) => {}
        }
    }

    pub fn active_view_mut(&mut self) -> &mut dyn crate::views::View {
        match self.active_view {
            ViewId::Dashboard => &mut self.views.dashboard,
            ViewId::Jobs => &mut self.views.jobs_list,
            ViewId::Applications => &mut self.views.applications,
            ViewId::Contacts => &mut self.views.contacts,
            ViewId::Ralph => &mut self.views.ralph_panel,
            ViewId::Settings => &mut self.views.settings,
        }
    }

    pub fn handle_ralph_update(&mut self, update: RalphUpdate) {
        self.views.ralph_panel.on_update(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        App::new_for_test()
    }

    #[test]
    fn action_quit_sets_should_quit() {
        let mut app = test_app();
        assert!(!app.should_quit);
        app.handle_action(Action::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn action_navigate_sets_view() {
        let mut app = test_app();
        assert_eq!(app.active_view, ViewId::Dashboard);
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        assert_eq!(app.active_view, ViewId::Jobs);
        assert_eq!(app.prev_view, Some(ViewId::Dashboard));
    }

    #[test]
    fn action_navigate_back_restores_prev() {
        let mut app = test_app();
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        app.handle_action(Action::NavigateBack);
        assert_eq!(app.active_view, ViewId::Dashboard);
        assert!(app.prev_view.is_none());
    }

    #[test]
    fn action_navigate_back_with_no_prev_does_nothing() {
        let mut app = test_app();
        let original = app.active_view;
        app.handle_action(Action::NavigateBack);
        assert_eq!(app.active_view, original);
    }

    #[test]
    fn action_toggle_help() {
        let mut app = test_app();
        assert!(!app.help_open);
        app.handle_action(Action::ToggleHelp);
        assert!(app.help_open);
        app.handle_action(Action::ToggleHelp);
        assert!(!app.help_open);
    }

    #[test]
    fn navigate_to_same_view_does_not_change_prev() {
        let mut app = test_app();
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        assert_eq!(app.prev_view, Some(ViewId::Dashboard));
    }

    #[test]
    fn default_input_mode_is_normal() {
        let app = test_app();
        assert_eq!(app.input_mode, InputMode::Normal);
    }
}
