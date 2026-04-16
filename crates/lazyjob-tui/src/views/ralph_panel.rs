use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::theme::Theme;

use super::View;

#[derive(Default)]
pub struct RalphPanelView;

impl RalphPanelView {
    pub fn new() -> Self {
        Self
    }
}

impl View for RalphPanelView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let content = "Ralph autonomous agent loops\n\n\
                       Active loops, progress, and results appear here.\n\
                       c = Cancel selected loop, Enter = Show detail\n\n\
                       No active loops.";

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Ralph "),
            )
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, _code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        None
    }

    fn name(&self) -> &'static str {
        "Ralph"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_without_panic() {
        let mut view = RalphPanelView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }
}
