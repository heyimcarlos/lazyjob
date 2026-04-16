use crossterm::event::{KeyCode, KeyModifiers};
use lazyjob_core::discovery::enrichment_badge;
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

    pub fn format_company_badge(company_name: &str, industry: Option<&str>) -> String {
        format!("{} {}", enrichment_badge(industry), company_name)
    }
}

impl View for JobsListView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let enriched = enrichment_badge(Some("Technology"));
        let unenriched = enrichment_badge(None);
        let content = format!(
            "Job listings from Greenhouse, Lever, and more\n\n\
             j/k to navigate, Enter to view details\n\
             / to search, f to filter\n\n\
             Company enrichment: {enriched} = researched, {unenriched} = not yet researched\n\n\
             No jobs loaded yet. Run a discovery loop to fetch jobs."
        );

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

    #[test]
    fn enrichment_badge_shows_e_for_enriched_company() {
        let badge = enrichment_badge(Some("Technology"));
        assert_eq!(badge, "[E]");
    }

    #[test]
    fn enrichment_badge_shows_empty_for_unenriched_company() {
        let badge = enrichment_badge(None);
        assert_eq!(badge, "[ ]");
    }

    #[test]
    fn format_company_badge_with_industry() {
        let formatted = JobsListView::format_company_badge("Stripe", Some("Fintech"));
        assert_eq!(formatted, "[E] Stripe");
    }

    #[test]
    fn format_company_badge_without_industry() {
        let formatted = JobsListView::format_company_badge("Unknown Corp", None);
        assert_eq!(formatted, "[ ] Unknown Corp");
    }

    #[test]
    fn renders_enrichment_badge_legend_in_buffer() {
        let mut view = JobsListView::new();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("[E]"));
        assert!(all_text.contains("[ ]"));
    }
}
