use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::theme::Theme;

pub struct JobCard<'a> {
    title: &'a str,
    company: &'a str,
    match_score: f32,
    is_ghost: bool,
    selected: bool,
    theme: &'a Theme,
}

impl<'a> JobCard<'a> {
    pub fn new(title: &'a str, company: &'a str, theme: &'a Theme) -> Self {
        Self {
            title,
            company,
            match_score: 0.0,
            is_ghost: false,
            selected: false,
            theme,
        }
    }

    pub fn match_score(mut self, score: f32) -> Self {
        self.match_score = score.clamp(0.0, 1.0);
        self
    }

    pub fn ghost(mut self, is_ghost: bool) -> Self {
        self.is_ghost = is_ghost;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl<'a> Widget for JobCard<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.selected {
            Style::default()
                .fg(self.theme.border_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.border)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width < 4 {
            return;
        }

        let ghost_tag = if self.is_ghost { " [GHOST]" } else { "" };
        let available_title = (inner.width as usize).saturating_sub(ghost_tag.len());
        let truncated_title = truncate_str(self.title, available_title);

        let title_spans = if self.is_ghost {
            vec![
                Span::styled(
                    truncated_title,
                    Style::default()
                        .fg(self.theme.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(ghost_tag, Style::default().fg(self.theme.warning)),
            ]
        } else {
            vec![Span::styled(
                truncated_title,
                Style::default()
                    .fg(self.theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            )]
        };

        let score_color = match_score_color(self.match_score);
        let score_text = format!("{:.0}%", self.match_score * 100.0);
        let company_max = (inner.width as usize).saturating_sub(score_text.len() + 2);
        let company_truncated = truncate_str(self.company, company_max);

        let company_line = Line::from(vec![
            Span::styled(
                company_truncated,
                Style::default().fg(self.theme.text_secondary),
            ),
            Span::raw("  "),
            Span::styled(score_text, Style::default().fg(score_color)),
        ]);

        let lines = vec![Line::from(title_spans), company_line];
        Paragraph::new(lines).render(inner, buf);
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

fn match_score_color(score: f32) -> Color {
    if score >= 0.7 {
        Color::LightGreen
    } else if score >= 0.5 {
        Color::LightYellow
    } else {
        Color::LightRed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer().clone();
        let (width, height) = (buf.area.width, buf.area.height);
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn job_card_renders_without_panic() {
        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let card =
                    JobCard::new("Software Engineer", "Acme Corp", &Theme::DARK).match_score(0.85);
                f.render_widget(card, f.area());
            })
            .unwrap();
    }

    #[test]
    fn job_card_shows_title_and_company() {
        let backend = TestBackend::new(50, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let card = JobCard::new("Engineer", "Acme", &Theme::DARK);
                f.render_widget(card, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Engineer"));
        assert!(text.contains("Acme"));
    }

    #[test]
    fn job_card_shows_ghost_badge() {
        let backend = TestBackend::new(50, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let card = JobCard::new("Senior Engineer", "Unknown Co", &Theme::DARK)
                    .ghost(true)
                    .match_score(0.3);
                f.render_widget(card, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("GHOST"));
    }

    #[test]
    fn job_card_selected_renders() {
        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let card = JobCard::new("Dev", "Corp", &Theme::DARK).selected(true);
                f.render_widget(card, f.area());
            })
            .unwrap();
    }

    #[test]
    fn truncate_str_short_string() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exact_fit() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_long_string() {
        let result = truncate_str("hello world", 7);
        assert_eq!(result.chars().count(), 7);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncate_str_zero_max() {
        assert_eq!(truncate_str("anything", 0), "");
    }

    #[test]
    fn match_score_color_thresholds() {
        assert_eq!(match_score_color(0.8), Color::LightGreen);
        assert_eq!(match_score_color(0.7), Color::LightGreen);
        assert_eq!(match_score_color(0.6), Color::LightYellow);
        assert_eq!(match_score_color(0.5), Color::LightYellow);
        assert_eq!(match_score_color(0.4), Color::LightRed);
        assert_eq!(match_score_color(0.0), Color::LightRed);
    }
}
