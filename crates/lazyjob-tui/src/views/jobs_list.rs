use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::theme::Theme;

use super::View;

#[derive(Default)]
pub struct JobsListView;

impl JobsListView {
    pub fn new() -> Self {
        Self
    }
}

impl View for JobsListView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let content = "Job listings from Greenhouse, Lever, and more\n\n\
                       j/k to navigate, Enter to view details\n\
                       / to search, f to filter\n\n\
                       No jobs loaded yet. Run a discovery loop to fetch jobs.";

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Jobs "),
            )
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, _code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        None
    }

    fn name(&self) -> &'static str {
        "Jobs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_without_panic() {
        let mut view = JobsListView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_title_in_buffer() {
        let mut view = JobsListView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let first_line: String = (0..80)
            .map(|x| buffer.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(first_line.contains("Jobs"));
    }
}
