use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

pub struct ProgressBar<'a> {
    ratio: f64,
    label: &'a str,
    color: Color,
}

impl<'a> ProgressBar<'a> {
    pub fn new(ratio: f64, label: &'a str) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            label,
            color: Color::LightBlue,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl<'a> Widget for ProgressBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let suffix = if self.label.is_empty() {
            format!(" {:.0}%", self.ratio * 100.0)
        } else {
            format!(" {} {:.0}%", self.label, self.ratio * 100.0)
        };

        let bar_width = (area.width as usize).saturating_sub(suffix.len());
        let filled = ((self.ratio * bar_width as f64).round() as usize).min(bar_width);
        let empty = bar_width - filled;

        let line = Line::from(vec![
            Span::styled("█".repeat(filled), Style::default().fg(self.color)),
            Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
            Span::styled(suffix, Style::default().fg(Color::Gray)),
        ]);

        Paragraph::new(line).render(area, buf);
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
    fn progress_bar_renders_without_panic() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let bar = ProgressBar::new(0.5, "Loading");
                f.render_widget(bar, f.area());
            })
            .unwrap();
    }

    #[test]
    fn progress_bar_label_appears() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let bar = ProgressBar::new(0.75, "Scanning");
                f.render_widget(bar, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Scanning"));
    }

    #[test]
    fn progress_bar_full_shows_percent() {
        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let bar = ProgressBar::new(1.0, "Done");
                f.render_widget(bar, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("100%"));
    }

    #[test]
    fn progress_bar_empty_shows_zero_percent() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let bar = ProgressBar::new(0.0, "");
                f.render_widget(bar, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("0%"));
    }

    #[test]
    fn progress_bar_clamps_ratio_above_one() {
        let bar = ProgressBar::new(1.5, "test");
        assert!((bar.ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn progress_bar_clamps_ratio_below_zero() {
        let bar = ProgressBar::new(-0.5, "test");
        assert!((bar.ratio - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn progress_bar_custom_color() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let bar = ProgressBar::new(0.5, "").color(Color::LightGreen);
                f.render_widget(bar, f.area());
            })
            .unwrap();
    }
}
