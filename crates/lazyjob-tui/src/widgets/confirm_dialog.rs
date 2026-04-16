use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::theme::Theme;
use crate::widgets::modal_dialog::centered_rect;

pub struct ConfirmDialog<'a> {
    title: &'a str,
    body: &'a str,
    confirm_selected: bool,
    theme: &'a Theme,
    width_percent: u16,
    height: u16,
}

impl<'a> ConfirmDialog<'a> {
    pub fn new(title: &'a str, body: &'a str, theme: &'a Theme) -> Self {
        Self {
            title,
            body,
            confirm_selected: true,
            theme,
            width_percent: 50,
            height: 9,
        }
    }

    pub fn confirm_selected(mut self, selected: bool) -> Self {
        self.confirm_selected = selected;
        self
    }

    pub fn width_percent(mut self, pct: u16) -> Self {
        self.width_percent = pct.clamp(1, 100);
        self
    }
}

impl<'a> Widget for ConfirmDialog<'a> {
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

        if inner.height < 2 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(inner);

        Paragraph::new(self.body)
            .style(Style::default().fg(self.theme.text_primary))
            .wrap(Wrap { trim: true })
            .render(chunks[0], buf);

        let yes_style = if self.confirm_selected {
            Style::default()
                .fg(self.theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.text_muted)
        };

        let no_style = if !self.confirm_selected {
            Style::default()
                .fg(self.theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.text_muted)
        };

        let button_line = Line::from(vec![
            Span::styled("[ Yes ]", yes_style),
            Span::raw("   "),
            Span::styled("[ No ]", no_style),
        ]);

        Paragraph::new(button_line).render(chunks[1], buf);
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
    fn confirm_dialog_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("Confirm", "Are you sure?", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
    }

    #[test]
    fn confirm_dialog_shows_title() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("Delete Job", "Remove this job?", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Delete Job"));
    }

    #[test]
    fn confirm_dialog_shows_body() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("Confirm", "UniqueBodyText123", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("UniqueBodyText123"));
    }

    #[test]
    fn confirm_dialog_shows_yes_and_no_buttons() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("Q", "body", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Yes"));
        assert!(text.contains("No"));
    }

    #[test]
    fn confirm_dialog_default_selects_yes() {
        let dialog = ConfirmDialog::new("T", "b", &Theme::DARK);
        assert!(dialog.confirm_selected);
    }

    #[test]
    fn confirm_dialog_no_selected_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("Q", "body", &Theme::DARK).confirm_selected(false);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Yes"));
        assert!(text.contains("No"));
    }

    #[test]
    fn confirm_dialog_small_area_does_not_panic() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let dialog = ConfirmDialog::new("X", "y", &Theme::DARK);
                f.render_widget(dialog, f.area());
            })
            .unwrap();
    }
}
