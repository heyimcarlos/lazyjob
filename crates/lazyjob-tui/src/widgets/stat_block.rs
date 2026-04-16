use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

pub struct StatBlock<'a> {
    label: &'a str,
    value: &'a str,
    subtitle: Option<&'a str>,
    color: Color,
}

impl<'a> StatBlock<'a> {
    pub fn new(label: &'a str, value: &'a str, color: Color) -> Self {
        Self {
            label,
            value,
            subtitle: None,
            color,
        }
    }

    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }
}

impl<'a> Widget for StatBlock<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.color));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 {
            return;
        }

        let mut lines = vec![
            Line::from(Span::styled(self.label, Style::default().fg(Color::Gray))),
            Line::from(Span::styled(
                self.value,
                Style::default().fg(self.color).add_modifier(Modifier::BOLD),
            )),
        ];

        if let Some(sub) = self.subtitle {
            lines.push(Line::from(Span::styled(
                sub,
                Style::default().fg(Color::DarkGray),
            )));
        }

        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .render(inner, buf);
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
    fn stat_block_renders_without_panic() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let block = StatBlock::new("Total Jobs", "42", Color::LightBlue);
                f.render_widget(block, f.area());
            })
            .unwrap();
    }

    #[test]
    fn stat_block_renders_value() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let block = StatBlock::new("Total Jobs", "42", Color::LightBlue);
                f.render_widget(block, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("42"));
        assert!(text.contains("Total Jobs"));
    }

    #[test]
    fn stat_block_renders_subtitle() {
        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let block = StatBlock::new("Applied", "7", Color::LightGreen).subtitle("this week");
                f.render_widget(block, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("this week"));
    }

    #[test]
    fn stat_block_renders_in_tiny_area() {
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let block = StatBlock::new("X", "0", Color::White);
                f.render_widget(block, f.area());
            })
            .unwrap();
    }
}
