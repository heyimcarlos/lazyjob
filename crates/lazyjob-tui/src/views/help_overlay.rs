use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::action::Action;
use crate::keybindings::{KeyContext, KeyMap};
use crate::theme::Theme;

use super::View;

#[derive(Default)]
pub struct HelpOverlay;

impl HelpOverlay {
    pub fn new() -> Self {
        Self
    }

    pub fn render_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        keymap: &KeyMap,
        active_context: &KeyContext,
    ) {
        let popup = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup);

        let mut lines = Vec::new();

        lines.push(Line::from(Span::styled(
            "Global Keybindings",
            theme.title_style(),
        )));
        lines.push(Line::from(""));

        for (key, action) in keymap.bindings_for_context(&KeyContext::Global) {
            lines.push(Line::from(format!("  {key:<12} {action}")));
        }

        if *active_context != KeyContext::Global {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("{} Keybindings", active_context.label()),
                theme.title_style(),
            )));
            lines.push(Line::from(""));

            for (key, action) in keymap.bindings_for_context(active_context) {
                lines.push(Line::from(format!("  {key:<12} {action}")));
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Help — press ? to close "),
            )
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, popup);
    }
}

impl View for HelpOverlay {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let popup = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup);

        let lines = vec![
            Line::from(Span::styled("Help", theme.title_style())),
            Line::from(""),
            Line::from("  Press ? or Esc to close"),
        ];

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Help "),
            )
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, popup);
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Char('?') | KeyCode::Esc => Some(Action::ToggleHelp),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Help"
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_overlay_without_panic() {
        let mut overlay = HelpOverlay::new();
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| overlay.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_dynamic_overlay_with_keymap() {
        let overlay = HelpOverlay::new();
        let keymap = KeyMap::default_keymap();
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                overlay.render_overlay(f, f.area(), &Theme::DARK, &keymap, &KeyContext::Jobs);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..40)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Quit"));
        assert!(all_text.contains("Scroll Down"));
        assert!(all_text.contains("Jobs Keybindings"));
    }

    #[test]
    fn dynamic_overlay_shows_global_section() {
        let overlay = HelpOverlay::new();
        let keymap = KeyMap::default_keymap();
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                overlay.render_overlay(f, f.area(), &Theme::DARK, &keymap, &KeyContext::Global);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..40)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Global Keybindings"));
        assert!(!all_text.contains("Jobs Keybindings"));
    }

    #[test]
    fn question_mark_closes_help() {
        let mut overlay = HelpOverlay::new();
        let action = overlay.handle_key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ToggleHelp));
    }

    #[test]
    fn esc_closes_help() {
        let mut overlay = HelpOverlay::new();
        let action = overlay.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ToggleHelp));
    }

    #[test]
    fn other_keys_return_none() {
        let mut overlay = HelpOverlay::new();
        assert_eq!(
            overlay.handle_key(KeyCode::Char('j'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn centered_rect_is_within_area() {
        let area = Rect::new(0, 0, 100, 40);
        let popup = centered_rect(60, 70, area);
        assert!(popup.x >= area.x);
        assert!(popup.y >= area.y);
        assert!(popup.right() <= area.right());
        assert!(popup.bottom() <= area.bottom());
        assert!(popup.width > 0);
        assert!(popup.height > 0);
    }
}
