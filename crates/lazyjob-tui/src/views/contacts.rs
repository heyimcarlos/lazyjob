use crossterm::event::{KeyCode, KeyModifiers};
use lazyjob_core::domain::Contact;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use crate::action::Action;
use crate::theme::Theme;

use super::View;

pub struct ContactsView {
    contacts: Vec<Contact>,
    table_state: TableState,
}

impl Default for ContactsView {
    fn default() -> Self {
        Self::new()
    }
}

impl ContactsView {
    pub fn new() -> Self {
        Self {
            contacts: Vec::new(),
            table_state: TableState::default(),
        }
    }

    pub fn set_contacts(&mut self, contacts: Vec<Contact>) {
        self.contacts = contacts;
        if !self.contacts.is_empty() && self.table_state.selected().is_none() {
            self.table_state.select(Some(0));
        }
    }

    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let header = Row::new(vec![
            Cell::from("Name"),
            Cell::from("Company"),
            Cell::from("Role"),
            Cell::from("Email"),
            Cell::from("Source"),
        ])
        .style(
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        );

        let rows: Vec<Row> = self
            .contacts
            .iter()
            .enumerate()
            .map(|(i, contact)| {
                let selected = self.table_state.selected() == Some(i);
                let style = if selected {
                    Style::default()
                        .fg(theme.text_primary)
                        .bg(theme.bg_secondary)
                } else {
                    Style::default().fg(theme.text_secondary)
                };

                Row::new(vec![
                    Cell::from(contact.name.clone()),
                    Cell::from(
                        contact
                            .current_company
                            .clone()
                            .unwrap_or_else(|| "—".into()),
                    ),
                    Cell::from(contact.role.clone().unwrap_or_else(|| "—".into())),
                    Cell::from(contact.email.clone().unwrap_or_else(|| "—".into())),
                    Cell::from(contact.source.clone().unwrap_or_else(|| "manual".into())),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Percentage(22),
            Constraint::Percentage(22),
            Constraint::Percentage(22),
            Constraint::Percentage(26),
            Constraint::Percentage(8),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(format!(" Contacts ({}) ", self.contacts.len())),
            )
            .row_highlight_style(
                Style::default()
                    .bg(theme.bg_secondary)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_empty(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let content = "Your professional network\n\n\
                       No contacts imported yet.\n\n\
                       Use `lazyjob contacts import --file connections.csv`\n\
                       to import LinkedIn connections.";

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Contacts "),
            )
            .alignment(ratatui::layout::Alignment::Center);

        frame.render_widget(paragraph, area);
    }

    fn render_status_bar<'a>(contacts_count: usize, theme: &Theme) -> Line<'a> {
        Line::from(vec![
            Span::styled("j/k", Style::default().fg(theme.primary)),
            Span::styled(" Navigate  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                format!("{contacts_count} contacts"),
                Style::default().fg(theme.text_secondary),
            ),
        ])
    }
}

impl View for ContactsView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.contacts.is_empty() {
            self.render_empty(frame, area, theme);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);

        self.render_table(frame, chunks[0], theme);

        let status = Self::render_status_bar(self.contacts.len(), theme);
        frame.render_widget(Paragraph::new(status), chunks[1]);
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        if self.contacts.is_empty() {
            return None;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                let i = self.table_state.selected().unwrap_or(0);
                let next = if i >= self.contacts.len() - 1 {
                    0
                } else {
                    i + 1
                };
                self.table_state.select(Some(next));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.table_state.selected().unwrap_or(0);
                let next = if i == 0 {
                    self.contacts.len().saturating_sub(1)
                } else {
                    i - 1
                };
                self.table_state.select(Some(next));
                None
            }
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Contacts"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_core::domain::Contact;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_contacts() -> Vec<Contact> {
        let mut c1 = Contact::new("Alice Smith");
        c1.current_company = Some("Acme Corp".into());
        c1.role = Some("Engineer".into());
        c1.email = Some("alice@example.com".into());
        c1.source = Some("linkedin_csv".into());

        let mut c2 = Contact::new("Bob Jones");
        c2.current_company = Some("Widget Inc".into());
        c2.role = Some("Manager".into());
        c2.email = Some("bob@example.com".into());

        vec![c1, c2]
    }

    #[test]
    fn renders_without_panic_empty() {
        let mut view = ContactsView::new();
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_without_panic_with_contacts() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn set_contacts_selects_first() {
        let mut view = ContactsView::new();
        assert!(view.table_state.selected().is_none());
        view.set_contacts(make_contacts());
        assert_eq!(view.table_state.selected(), Some(0));
    }

    #[test]
    fn j_moves_selection_down() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        assert_eq!(view.table_state.selected(), Some(0));
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(1));
    }

    #[test]
    fn k_moves_selection_up() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        view.table_state.select(Some(1));
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(0));
    }

    #[test]
    fn j_wraps_to_top() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        view.table_state.select(Some(1));
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(0));
    }

    #[test]
    fn k_wraps_to_bottom() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        view.table_state.select(Some(0));
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(1));
    }

    #[test]
    fn empty_contacts_ignores_keys() {
        let mut view = ContactsView::new();
        let result = view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(result.is_none());
    }

    #[test]
    fn name_returns_contacts() {
        let view = ContactsView::new();
        assert_eq!(view.name(), "Contacts");
    }

    #[test]
    fn contacts_accessor_returns_slice() {
        let mut view = ContactsView::new();
        let contacts = make_contacts();
        view.set_contacts(contacts.clone());
        assert_eq!(view.contacts().len(), 2);
        assert_eq!(view.contacts()[0].name, "Alice Smith");
    }

    #[test]
    fn render_shows_contact_names() {
        let mut view = ContactsView::new();
        view.set_contacts(make_contacts());
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let mut found_alice = false;
        let mut found_bob = false;
        for y in 0..24 {
            let line: String = (0..100)
                .map(|x| {
                    buffer
                        .cell((x, y))
                        .unwrap()
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                })
                .collect();
            if line.contains("Alice Smith") {
                found_alice = true;
            }
            if line.contains("Bob Jones") {
                found_bob = true;
            }
        }
        assert!(found_alice, "Alice Smith should appear in rendered output");
        assert!(found_bob, "Bob Jones should appear in rendered output");
    }
}
