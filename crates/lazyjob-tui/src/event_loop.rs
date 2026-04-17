use std::io::{self, Stdout};

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::time::{self, Duration};

use crate::action::Action;
use crate::app::{App, InputMode};
use crate::keybindings::{KeyCombo, KeyContext};
use crate::render;
use crate::views::View;

const TICK_RATE: Duration = Duration::from_millis(250);

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub async fn run_event_loop(mut app: App) -> anyhow::Result<()> {
    let mut guard = TerminalGuard::new()?;
    let mut event_reader = EventStream::new();
    let mut tick = time::interval(TICK_RATE);

    loop {
        guard.terminal.draw(|f| render::render(f, &mut app))?;

        tokio::select! {
            _ = tick.tick() => {}
            maybe_event = event_reader.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if let Some(action) = map_event_to_action(&mut app, &event) {
                            let is_refresh = matches!(action, Action::Refresh);
                            app.handle_action(action);
                            if is_refresh {
                                app.load_jobs().await;
                                app.load_applications().await;
                                app.load_dashboard_stats().await;
                            }
                        }
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            result = app.ralph_rx.recv() => {
                if let Ok(update) = result {
                    app.handle_ralph_update(update);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn map_event_to_action(app: &mut App, event: &Event) -> Option<Action> {
    match event {
        Event::Key(key) => {
            if key.kind != crossterm::event::KeyEventKind::Press {
                return None;
            }
            map_key_to_action(app, key.code, key.modifiers)
        }
        _ => None,
    }
}

fn map_key_to_action(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    if app.help_open {
        return app.views.help_overlay.handle_key(code, modifiers);
    }

    if app.input_mode == InputMode::Search {
        return app.active_view_mut().handle_key(code, modifiers);
    }

    let combo = KeyCombo::from_key_event(code, modifiers);
    let ctx = KeyContext::from_view_id(app.active_view);

    if let Some(action) = app.keymap.resolve(&ctx, &combo) {
        return Some(action.clone());
    }

    app.active_view_mut().handle_key(code, modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ViewId;

    #[test]
    fn key_q_maps_to_quit() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn key_ctrl_c_maps_to_quit() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn key_question_maps_to_toggle_help() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(action, Some(Action::ToggleHelp));
    }

    #[test]
    fn key_esc_maps_to_navigate_back() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::NavigateBack));
    }

    #[test]
    fn number_keys_map_to_views() {
        let mut app = App::new_for_test();
        let cases = [
            ('1', ViewId::Dashboard),
            ('2', ViewId::Jobs),
            ('3', ViewId::Applications),
            ('4', ViewId::Contacts),
            ('5', ViewId::Ralph),
            ('6', ViewId::Settings),
        ];

        for (ch, expected_view) in cases {
            let action = map_key_to_action(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
            assert_eq!(action, Some(Action::NavigateTo(expected_view)));
        }
    }

    #[test]
    fn ctrl_r_maps_to_refresh() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(action, Some(Action::Refresh));
    }

    #[test]
    fn unbound_key_returns_none() {
        let mut app = App::new_for_test();
        let action = map_key_to_action(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(action, None);
    }

    #[test]
    fn help_open_routes_to_help_overlay() {
        let mut app = App::new_for_test();
        app.help_open = true;
        let action = map_key_to_action(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ToggleHelp));
    }

    #[test]
    fn help_open_blocks_global_keys() {
        let mut app = App::new_for_test();
        app.help_open = true;
        let action = map_key_to_action(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(action, None);
    }

    #[test]
    fn j_maps_to_scroll_down_in_view() {
        let mut app = App::new_for_test();
        app.active_view = ViewId::Jobs;
        let action = map_key_to_action(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ScrollDown));
    }

    #[test]
    fn k_maps_to_scroll_up_in_view() {
        let mut app = App::new_for_test();
        app.active_view = ViewId::Applications;
        let action = map_key_to_action(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ScrollUp));
    }

    #[test]
    fn arrow_keys_map_to_scroll() {
        let mut app = App::new_for_test();
        app.active_view = ViewId::Jobs;
        assert_eq!(
            map_key_to_action(&mut app, KeyCode::Down, KeyModifiers::NONE),
            Some(Action::ScrollDown)
        );
        assert_eq!(
            map_key_to_action(&mut app, KeyCode::Up, KeyModifiers::NONE),
            Some(Action::ScrollUp)
        );
    }

    #[test]
    fn enter_maps_to_select_in_view() {
        let mut app = App::new_for_test();
        app.active_view = ViewId::Jobs;
        let action = map_key_to_action(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::Select));
    }
}
