use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::theme::Theme;

pub struct ModalDialog<'a> {
    title: &'a str,
    body: &'a str,
    theme: &'a Theme,
    width_percent: u16,
    height: u16,
}

impl<'a> ModalDialog<'a> {
    pub fn new(title: &'a str, body: &'a str, theme: &'a Theme) -> Self {
        Self {
            title,
            body,
            theme,
            width_percent: 60,
            height: 10,
        }
    }

    pub fn width_percent(mut self, pct: u16) -> Self {
        self.width_percent = pct.clamp(1, 100);
        self
    }

    pub fn height(mut self, h: u16) -> Self {
        self.height = h.max(3);
        self
    }
}

impl<'a> Widget for ModalDialog<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup = centered_rect(self.width_percent, self.height, area);

        Clear.render(popup, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.focused_border_style())
            .title(Line::from(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(self.theme.primary),
            )));

        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height == 0 {
            return;
        }

        Paragraph::new(self.body)
            .style(Style::default().fg(self.theme.text_primary))
            .wrap(Wrap { trim: true })
            .render(inner, buf);
    }
}

pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let clamped_height = height.min(r.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(clamped_height),
            Constraint::Fill(1),
        ])
        .split(r);

    let w = (r.width * percent_x / 100).max(1).min(r.width);
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    Rect::new(x, vertical[1].y, w, clamped_height)
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
    fn modal_dialog_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ModalDialog::new("Alert", "Something happened.", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
    }

    #[test]
    fn modal_dialog_renders_title() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ModalDialog::new("MyTitle", "body text here", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("MyTitle"));
    }

    #[test]
    fn modal_dialog_renders_body() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ModalDialog::new("T", "UniqueBodyContent", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("UniqueBodyContent"));
    }

    #[test]
    fn centered_rect_is_centered_horizontally() {
        let area = Rect::new(0, 0, 100, 30);
        let popup = centered_rect(50, 10, area);
        let expected_w = 50u16;
        let expected_x = (100 - expected_w) / 2;
        assert_eq!(popup.width, expected_w);
        assert_eq!(popup.x, expected_x);
        assert_eq!(popup.height, 10);
    }

    #[test]
    fn centered_rect_is_centered_vertically() {
        let area = Rect::new(0, 0, 100, 30);
        let popup = centered_rect(50, 10, area);
        let expected_y = (30 - 10) / 2;
        assert_eq!(popup.y, expected_y);
    }

    #[test]
    fn centered_rect_full_width() {
        let area = Rect::new(0, 0, 80, 24);
        let popup = centered_rect(100, 5, area);
        assert_eq!(popup.width, 80);
        assert_eq!(popup.x, 0);
    }

    #[test]
    fn modal_dialog_small_area_does_not_panic() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ModalDialog::new("X", "y", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
    }
}
