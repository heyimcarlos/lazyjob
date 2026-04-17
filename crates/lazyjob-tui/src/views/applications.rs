use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use lazyjob_core::domain::{ApplicationId, ApplicationStage};

use crate::action::Action;
use crate::theme::Theme;
use crate::widgets::ConfirmDialog;

use super::View;

#[derive(Debug, Clone)]
pub struct ApplicationCard {
    pub application_id: ApplicationId,
    pub title: String,
    pub company: String,
    pub stage: ApplicationStage,
    pub updated_at: DateTime<Utc>,
}

impl ApplicationCard {
    fn days_in_stage(&self) -> i64 {
        (Utc::now() - self.updated_at).num_days()
    }
}

fn days_color(days: i64) -> Color {
    if days < 7 {
        Color::LightGreen
    } else if days < 14 {
        Color::LightYellow
    } else {
        Color::LightRed
    }
}

fn forward_stage(stage: ApplicationStage) -> Option<ApplicationStage> {
    let transitions = stage.valid_transitions();
    transitions
        .iter()
        .find(|s| {
            !s.is_terminal()
                && **s != ApplicationStage::Withdrawn
                && **s != ApplicationStage::Rejected
        })
        .or_else(|| transitions.first())
        .copied()
}

struct ConfirmState {
    application_id: ApplicationId,
    from_stage: ApplicationStage,
    to_stage: ApplicationStage,
    confirm_selected: bool,
}

pub struct ApplicationsView {
    cards: Vec<ApplicationCard>,
    focused_column: usize,
    focused_cards: [usize; 9],
    confirming: Option<ConfirmState>,
}

impl Default for ApplicationsView {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationsView {
    pub fn new() -> Self {
        Self {
            cards: Vec::new(),
            focused_column: 0,
            focused_cards: [0; 9],
            confirming: None,
        }
    }

    pub fn set_applications(&mut self, cards: Vec<ApplicationCard>) {
        self.cards = cards;
        self.focused_cards = [0; 9];
        if self.focused_column >= ApplicationStage::all().len() {
            self.focused_column = 0;
        }
    }

    fn cards_in_column(&self, stage: ApplicationStage) -> Vec<&ApplicationCard> {
        self.cards.iter().filter(|c| c.stage == stage).collect()
    }

    fn current_stage(&self) -> ApplicationStage {
        ApplicationStage::all()[self.focused_column]
    }

    fn selected_card(&self) -> Option<&ApplicationCard> {
        let stage = self.current_stage();
        let cards = self.cards_in_column(stage);
        let idx = self.focused_cards[self.focused_column];
        cards.get(idx).copied()
    }

    fn render_column(
        &self,
        frame: &mut Frame,
        area: Rect,
        stage: ApplicationStage,
        col_idx: usize,
        theme: &Theme,
    ) {
        let is_focused = col_idx == self.focused_column;
        let cards = self.cards_in_column(stage);
        let count = cards.len();
        let selected_idx = self.focused_cards[col_idx];

        let border_style = if is_focused {
            theme.focused_border_style()
        } else {
            Style::default().fg(theme.border)
        };

        let title = format!(" {} ({}) ", stage, count);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if cards.is_empty() {
            let empty = Paragraph::new("(empty)").style(Style::default().fg(theme.text_muted));
            frame.render_widget(empty, inner);
            return;
        }

        let mut y = inner.y;
        for (i, card) in cards.iter().enumerate() {
            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = is_focused && i == selected_idx;
            let days = card.days_in_stage();
            let day_color = days_color(days);

            let title_style = if is_selected {
                theme.selected_style()
            } else {
                Style::default().fg(theme.text_primary)
            };

            let title_text = truncate(&card.title, inner.width.saturating_sub(1) as usize);
            let title_line = Line::from(Span::styled(title_text, title_style));

            let company_line = Line::from(Span::styled(
                truncate(&card.company, inner.width.saturating_sub(1) as usize),
                Style::default().fg(theme.text_secondary),
            ));

            let days_text = format!("{}d", days);
            let days_line = Line::from(Span::styled(
                days_text,
                Style::default().fg(day_color).add_modifier(Modifier::BOLD),
            ));

            let card_height = 3u16;
            if y + card_height > inner.y + inner.height {
                break;
            }

            let card_area = Rect::new(inner.x, y, inner.width, card_height);

            let card_paragraph = Paragraph::new(vec![title_line, company_line, days_line]);
            frame.render_widget(card_paragraph, card_area);

            y += card_height + 1;
        }
    }

    fn render_confirm_overlay(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(confirm) = &self.confirming else {
            return;
        };
        let body = format!(
            "Move application from {} to {}?",
            confirm.from_stage, confirm.to_stage
        );
        let dialog = ConfirmDialog::new("Stage Transition", &body, theme)
            .confirm_selected(confirm.confirm_selected);
        frame.render_widget(Clear, area);
        frame.render_widget(dialog, area);
    }
}

impl View for ApplicationsView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let stages = ApplicationStage::all();
        let num_cols = stages.len() as u16;

        if area.width < num_cols * 2 || area.height < 5 {
            let msg = Paragraph::new("Terminal too small for kanban view")
                .style(Style::default().fg(theme.text_muted));
            frame.render_widget(msg, area);
            return;
        }

        let body_height = area.height.saturating_sub(2);
        let body_area = Rect::new(area.x, area.y, area.width, body_height);
        let help_area = Rect::new(area.x, area.y + body_height, area.width, 2.min(area.height));

        let constraints: Vec<Constraint> = (0..num_cols)
            .map(|_| Constraint::Ratio(1, num_cols as u32))
            .collect();

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(body_area);

        for (i, stage) in stages.iter().enumerate() {
            self.render_column(frame, columns[i], *stage, i, theme);
        }

        let help_text = Line::from(vec![
            Span::styled(
                " h/l",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":columns  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "j/k",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cards  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "m",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":advance  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "M",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":withdraw  ", Style::default().fg(theme.text_muted)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":back", Style::default().fg(theme.text_muted)),
        ]);
        let help = Paragraph::new(help_text);
        frame.render_widget(help, help_area);

        if self.confirming.is_some() {
            self.render_confirm_overlay(frame, area, theme);
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(confirm) = &mut self.confirming {
            match code {
                KeyCode::Left | KeyCode::Char('h') => {
                    confirm.confirm_selected = true;
                    None
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    confirm.confirm_selected = false;
                    None
                }
                KeyCode::Enter => {
                    let app_id = confirm.application_id;
                    let to_stage = confirm.to_stage;
                    let yes = confirm.confirm_selected;
                    self.confirming = None;
                    if yes {
                        Some(Action::TransitionApplication(app_id, to_stage))
                    } else {
                        None
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.confirming = None;
                    None
                }
                KeyCode::Char('y') => {
                    let app_id = confirm.application_id;
                    let to_stage = confirm.to_stage;
                    self.confirming = None;
                    Some(Action::TransitionApplication(app_id, to_stage))
                }
                _ => None,
            }
        } else {
            match (code, modifiers) {
                (KeyCode::Char('h') | KeyCode::Left, _) => {
                    if self.focused_column > 0 {
                        self.focused_column -= 1;
                    }
                    None
                }
                (KeyCode::Char('l') | KeyCode::Right, _) => {
                    let max = ApplicationStage::all().len().saturating_sub(1);
                    if self.focused_column < max {
                        self.focused_column += 1;
                    }
                    None
                }
                (KeyCode::Char('j') | KeyCode::Down, _) => {
                    let col = self.focused_column;
                    let count = self.cards_in_column(self.current_stage()).len();
                    if count > 0 && self.focused_cards[col] < count - 1 {
                        self.focused_cards[col] += 1;
                    }
                    None
                }
                (KeyCode::Char('k') | KeyCode::Up, _) => {
                    let col = self.focused_column;
                    if self.focused_cards[col] > 0 {
                        self.focused_cards[col] -= 1;
                    }
                    None
                }
                (KeyCode::Char('m'), KeyModifiers::NONE) => {
                    if let Some(card) = self.selected_card()
                        && let Some(next) = forward_stage(card.stage)
                    {
                        self.confirming = Some(ConfirmState {
                            application_id: card.application_id,
                            from_stage: card.stage,
                            to_stage: next,
                            confirm_selected: true,
                        });
                    }
                    None
                }
                (KeyCode::Char('M'), KeyModifiers::SHIFT) => {
                    if let Some(card) = self.selected_card()
                        && !card.stage.is_terminal()
                    {
                        self.confirming = Some(ConfirmState {
                            application_id: card.application_id,
                            from_stage: card.stage,
                            to_stage: ApplicationStage::Withdrawn,
                            confirm_selected: true,
                        });
                    }
                    None
                }
                (KeyCode::Esc, _) => Some(Action::NavigateBack),
                _ => None,
            }
        }
    }

    fn name(&self) -> &'static str {
        "Applications"
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_core::domain::ApplicationId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
        }
        s
    }

    fn make_card(
        title: &str,
        company: &str,
        stage: ApplicationStage,
        days_ago: i64,
    ) -> ApplicationCard {
        ApplicationCard {
            application_id: ApplicationId::new(),
            title: title.to_string(),
            company: company.to_string(),
            stage,
            updated_at: Utc::now() - chrono::Duration::days(days_ago),
        }
    }

    fn sample_cards() -> Vec<ApplicationCard> {
        vec![
            make_card("Software Engineer", "Stripe", ApplicationStage::Applied, 3),
            make_card("Backend Dev", "Google", ApplicationStage::Applied, 10),
            make_card("SRE", "Netflix", ApplicationStage::PhoneScreen, 2),
            make_card("Staff Eng", "Meta", ApplicationStage::Technical, 20),
            make_card("Frontend Dev", "Airbnb", ApplicationStage::Interested, 1),
            make_card("Data Eng", "Uber", ApplicationStage::Offer, 5),
        ]
    }

    #[test]
    fn new_creates_empty_view() {
        let view = ApplicationsView::new();
        assert!(view.cards.is_empty());
        assert_eq!(view.focused_column, 0);
        assert!(view.confirming.is_none());
    }

    #[test]
    fn set_applications_populates_cards() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        assert_eq!(view.cards.len(), 6);
        assert_eq!(view.cards_in_column(ApplicationStage::Applied).len(), 2);
        assert_eq!(view.cards_in_column(ApplicationStage::PhoneScreen).len(), 1);
        assert_eq!(view.cards_in_column(ApplicationStage::Interested).len(), 1);
        assert_eq!(view.cards_in_column(ApplicationStage::Offer).len(), 1);
        assert_eq!(view.cards_in_column(ApplicationStage::Technical).len(), 1);
    }

    #[test]
    fn handle_key_l_moves_column_right() {
        let mut view = ApplicationsView::new();
        assert_eq!(view.focused_column, 0);
        view.handle_key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(view.focused_column, 1);
        view.handle_key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(view.focused_column, 2);
    }

    #[test]
    fn handle_key_h_moves_column_left() {
        let mut view = ApplicationsView::new();
        view.focused_column = 3;
        view.handle_key(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(view.focused_column, 2);
        view.handle_key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(view.focused_column, 1);
    }

    #[test]
    fn handle_key_h_clamps_at_zero() {
        let mut view = ApplicationsView::new();
        view.handle_key(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(view.focused_column, 0);
    }

    #[test]
    fn handle_key_l_clamps_at_max() {
        let mut view = ApplicationsView::new();
        let max = ApplicationStage::all().len() - 1;
        view.focused_column = max;
        view.handle_key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(view.focused_column, max);
    }

    #[test]
    fn handle_key_j_moves_card_down() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1; // Applied column has 2 cards
        assert_eq!(view.focused_cards[1], 0);
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.focused_cards[1], 1);
    }

    #[test]
    fn handle_key_k_moves_card_up() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.focused_cards[1] = 1;
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.focused_cards[1], 0);
    }

    #[test]
    fn handle_key_k_clamps_at_zero() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.focused_cards[1], 0);
    }

    #[test]
    fn handle_key_j_clamps_at_max() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1; // Applied has 2 cards (idx 0,1)
        view.focused_cards[1] = 1;
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.focused_cards[1], 1);
    }

    #[test]
    fn handle_key_m_opens_confirm_dialog() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1; // Applied
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(view.confirming.is_some());
        let confirm = view.confirming.as_ref().unwrap();
        assert_eq!(confirm.from_stage, ApplicationStage::Applied);
        assert_eq!(confirm.to_stage, ApplicationStage::PhoneScreen);
    }

    #[test]
    fn handle_key_m_no_op_on_empty_column() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 4; // Onsite (index 4) — no cards in sample
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(view.confirming.is_none());
    }

    #[test]
    fn handle_key_m_no_op_on_terminal_stage() {
        let mut view = ApplicationsView::new();
        let card = make_card("Done", "Co", ApplicationStage::Accepted, 1);
        view.set_applications(vec![card]);
        view.focused_column = 6; // Accepted (index 6 in ApplicationStage::all())
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(view.confirming.is_none());
    }

    #[test]
    fn confirm_yes_returns_transition_action() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(view.confirming.is_some());
        let action = view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(view.confirming.is_none());
        assert!(matches!(
            action,
            Some(Action::TransitionApplication(
                _,
                ApplicationStage::PhoneScreen
            ))
        ));
    }

    #[test]
    fn confirm_no_closes_dialog() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        // Switch to No
        view.handle_key(KeyCode::Char('l'), KeyModifiers::NONE);
        let action = view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(view.confirming.is_none());
        assert!(action.is_none());
    }

    #[test]
    fn confirm_esc_closes_dialog() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(view.confirming.is_some());
        view.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(view.confirming.is_none());
    }

    #[test]
    fn confirm_y_shortcut() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        let action = view.handle_key(KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(matches!(action, Some(Action::TransitionApplication(_, _))));
    }

    #[test]
    fn shift_m_triggers_withdraw_confirm() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1; // Applied
        view.handle_key(KeyCode::Char('M'), KeyModifiers::SHIFT);
        assert!(view.confirming.is_some());
        let confirm = view.confirming.as_ref().unwrap();
        assert_eq!(confirm.to_stage, ApplicationStage::Withdrawn);
    }

    #[test]
    fn shift_m_no_op_on_terminal() {
        let mut view = ApplicationsView::new();
        let card = make_card("Done", "Co", ApplicationStage::Rejected, 1);
        view.set_applications(vec![card]);
        view.focused_column = 7; // Rejected (index 7 in ApplicationStage::all())
        view.handle_key(KeyCode::Char('M'), KeyModifiers::SHIFT);
        assert!(view.confirming.is_none());
    }

    #[test]
    fn esc_returns_navigate_back() {
        let mut view = ApplicationsView::new();
        let action = view.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::NavigateBack));
    }

    #[test]
    fn renders_without_panic() {
        let mut view = ApplicationsView::new();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_with_cards_without_panic() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_column_headers() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        let backend = TestBackend::new(180, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buf);
        assert!(content.contains("Interested"));
        assert!(content.contains("Applied"));
        assert!(content.contains("Phone Screen"));
    }

    #[test]
    fn renders_card_titles() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        let backend = TestBackend::new(180, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buf);
        assert!(content.contains("Stripe"));
        assert!(content.contains("Google"));
        assert!(content.contains("Netflix"));
    }

    #[test]
    fn renders_confirm_dialog() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 1;
        view.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buf);
        assert!(content.contains("Stage Transition"));
        assert!(content.contains("Yes"));
        assert!(content.contains("No"));
    }

    #[test]
    fn renders_small_terminal_gracefully() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        let backend = TestBackend::new(10, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn days_color_green_under_7() {
        assert_eq!(days_color(0), Color::LightGreen);
        assert_eq!(days_color(6), Color::LightGreen);
    }

    #[test]
    fn days_color_yellow_7_to_14() {
        assert_eq!(days_color(7), Color::LightYellow);
        assert_eq!(days_color(13), Color::LightYellow);
    }

    #[test]
    fn days_color_red_over_14() {
        assert_eq!(days_color(14), Color::LightRed);
        assert_eq!(days_color(100), Color::LightRed);
    }

    #[test]
    fn forward_stage_from_interested() {
        assert_eq!(
            forward_stage(ApplicationStage::Interested),
            Some(ApplicationStage::Applied)
        );
    }

    #[test]
    fn forward_stage_from_applied() {
        assert_eq!(
            forward_stage(ApplicationStage::Applied),
            Some(ApplicationStage::PhoneScreen)
        );
    }

    #[test]
    fn forward_stage_from_offer() {
        assert_eq!(
            forward_stage(ApplicationStage::Offer),
            Some(ApplicationStage::Accepted)
        );
    }

    #[test]
    fn forward_stage_from_accepted_is_none() {
        assert_eq!(forward_stage(ApplicationStage::Accepted), None);
    }

    #[test]
    fn set_applications_resets_focused_cards() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_cards[1] = 1;
        view.set_applications(sample_cards());
        assert_eq!(view.focused_cards[1], 0);
    }

    #[test]
    fn selected_card_returns_correct_card() {
        let mut view = ApplicationsView::new();
        let cards = sample_cards();
        let expected_id = cards[4].application_id; // Frontend Dev at Airbnb, Interested
        view.set_applications(cards);
        view.focused_column = 0; // Interested
        let selected = view.selected_card().unwrap();
        assert_eq!(selected.application_id, expected_id);
    }

    #[test]
    fn selected_card_returns_none_for_empty_column() {
        let mut view = ApplicationsView::new();
        view.set_applications(sample_cards());
        view.focused_column = 4; // Onsite (index 4) — no cards in sample
        assert!(view.selected_card().is_none());
    }
}
