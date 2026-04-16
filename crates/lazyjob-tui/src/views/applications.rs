use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::theme::Theme;

use super::View;

#[derive(Default)]
pub struct ApplicationsView;

impl ApplicationsView {
    pub fn new() -> Self {
        Self
    }
}

impl View for ApplicationsView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let content = "Kanban board of your applications\n\n\
                       Columns: Interested > Applied > Phone Screen > Technical >\n\
                       Onsite > Offer > Accepted | Withdrawn | Rejected\n\n\
                       h/l to move between columns, j/k within column\n\
                       m to advance stage, M to move back";

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Applications "),
            )
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, _code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        None
    }

    fn name(&self) -> &'static str {
        "Applications"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_without_panic() {
        let mut view = ApplicationsView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }
}
