use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use lazyjob_core::domain::ApplicationStage;
use lazyjob_core::stats::{DashboardStats, StaleApplication};

use crate::action::Action;
use crate::theme::Theme;
use crate::widgets::StatBlock;

use super::View;

#[derive(Default)]
pub struct DashboardView {
    stats: DashboardStats,
    stale: Vec<StaleApplication>,
    selected_stale: usize,
}

impl DashboardView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_stats(&mut self, stats: DashboardStats, stale: Vec<StaleApplication>) {
        self.stats = stats;
        self.stale = stale;
        self.selected_stale = 0;
    }

    fn render_stat_blocks(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
            ])
            .split(area);

        frame.render_widget(
            StatBlock::new(
                "Total Jobs",
                &self.stats.total_jobs.to_string(),
                theme.primary,
            ),
            chunks[0],
        );
        frame.render_widget(
            StatBlock::new(
                "Applied This Week",
                &self.stats.applied_this_week.to_string(),
                theme.success,
            ),
            chunks[1],
        );
        frame.render_widget(
            StatBlock::new(
                "In Pipeline",
                &self.stats.in_pipeline.to_string(),
                theme.warning,
            ),
            chunks[2],
        );
        frame.render_widget(
            StatBlock::new(
                "Interviewing",
                &self.stats.interviewing.to_string(),
                Color::LightMagenta,
            ),
            chunks[3],
        );
    }

    fn render_kanban_counts(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let stages: &[ApplicationStage] = &[
            ApplicationStage::Interested,
            ApplicationStage::Applied,
            ApplicationStage::PhoneScreen,
            ApplicationStage::Technical,
            ApplicationStage::Onsite,
            ApplicationStage::Offer,
            ApplicationStage::Accepted,
            ApplicationStage::Rejected,
            ApplicationStage::Withdrawn,
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .title(" Pipeline ");

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let constraints: Vec<Constraint> = stages.iter().map(|_| Constraint::Ratio(1, 9)).collect();
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(inner);

        for (i, stage) in stages.iter().enumerate() {
            let count = self.stats.stage_counts.get(stage).copied().unwrap_or(0);
            let color = stage_color(*stage, theme);

            let lines = vec![
                Line::from(Span::styled(
                    stage_short_label(*stage),
                    Style::default().fg(theme.text_secondary),
                )),
                Line::from(Span::styled(
                    count.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
            ];

            frame.render_widget(
                Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
                cols[i],
            );
        }
    }

    fn render_stale_list(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .title(" Actions Required ");

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        if self.stale.is_empty() {
            let msg = Paragraph::new("No stale applications. You're on top of things!")
                .style(Style::default().fg(theme.text_muted))
                .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(msg, inner);
            return;
        }

        let items: Vec<ListItem> = self
            .stale
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let style = if i == self.selected_stale {
                    theme.selected_style()
                } else {
                    Style::default().fg(theme.text_primary)
                };

                let line = Line::from(vec![
                    Span::styled("⚠ ", Style::default().fg(theme.warning)),
                    Span::styled(&s.job_title, style),
                    Span::styled(
                        format!(" at {}", s.company),
                        Style::default().fg(theme.text_secondary),
                    ),
                    Span::styled(
                        format!(" — {}d stale", s.days_stale),
                        Style::default().fg(theme.error),
                    ),
                ]);

                ListItem::new(line)
            })
            .collect();

        frame.render_widget(List::new(items), inner);
    }
}

impl View for DashboardView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(5),
                Constraint::Fill(1),
            ])
            .split(area);

        self.render_stat_blocks(frame, chunks[0], theme);
        self.render_kanban_counts(frame, chunks[1], theme);
        self.render_stale_list(frame, chunks[2], theme);
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.stale.is_empty() {
                    self.selected_stale = (self.selected_stale + 1).min(self.stale.len() - 1);
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected_stale = self.selected_stale.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Dashboard"
    }
}

fn stage_color(stage: ApplicationStage, theme: &Theme) -> Color {
    match stage {
        ApplicationStage::Interested => theme.text_muted,
        ApplicationStage::Applied => theme.primary,
        ApplicationStage::PhoneScreen | ApplicationStage::Technical | ApplicationStage::Onsite => {
            Color::LightMagenta
        }
        ApplicationStage::Offer => theme.warning,
        ApplicationStage::Accepted => theme.success,
        ApplicationStage::Rejected => theme.error,
        ApplicationStage::Withdrawn => theme.text_secondary,
    }
}

fn stage_short_label(stage: ApplicationStage) -> &'static str {
    match stage {
        ApplicationStage::Interested => "INT",
        ApplicationStage::Applied => "APP",
        ApplicationStage::PhoneScreen => "PHN",
        ApplicationStage::Technical => "TEC",
        ApplicationStage::Onsite => "ONS",
        ApplicationStage::Offer => "OFR",
        ApplicationStage::Accepted => "ACC",
        ApplicationStage::Rejected => "REJ",
        ApplicationStage::Withdrawn => "WDR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_core::domain::ApplicationId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::HashMap;

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer().clone();
        let (width, height) = (buf.area.width, buf.area.height);
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    fn sample_stats() -> DashboardStats {
        let mut stage_counts = HashMap::new();
        stage_counts.insert(ApplicationStage::Applied, 5);
        stage_counts.insert(ApplicationStage::Technical, 2);
        stage_counts.insert(ApplicationStage::Rejected, 3);
        DashboardStats {
            total_jobs: 42,
            applied_this_week: 7,
            in_pipeline: 7,
            interviewing: 2,
            stage_counts,
        }
    }

    fn sample_stale() -> Vec<StaleApplication> {
        vec![
            StaleApplication {
                application_id: ApplicationId::new(),
                job_title: "Rust Engineer".into(),
                company: "StaleInc".into(),
                days_stale: 21,
            },
            StaleApplication {
                application_id: ApplicationId::new(),
                job_title: "Go Developer".into(),
                company: "OldCorp".into(),
                days_stale: 15,
            },
        ]
    }

    #[test]
    fn renders_without_panic() {
        let mut view = DashboardView::new();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_with_data_without_panic() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), sample_stale());
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_stat_block_values() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), vec![]);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("42"), "should contain total jobs count");
        assert!(
            text.contains("Total Jobs"),
            "should contain Total Jobs label"
        );
        assert!(text.contains("7"), "should contain applied this week count");
    }

    #[test]
    fn renders_pipeline_label() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), vec![]);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Pipeline"), "should contain Pipeline title");
    }

    #[test]
    fn renders_stage_counts() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), vec![]);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("APP"), "should contain APP stage label");
        assert!(text.contains("TEC"), "should contain TEC stage label");
        assert!(text.contains("5"), "should contain Applied count");
    }

    #[test]
    fn renders_stale_apps() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), sample_stale());
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("Rust Engineer"),
            "should contain stale job title"
        );
        assert!(text.contains("StaleInc"), "should contain stale company");
        assert!(text.contains("21d"), "should contain days stale");
    }

    #[test]
    fn renders_empty_stale_message() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), vec![]);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("No stale"),
            "should show no stale applications message"
        );
    }

    #[test]
    fn handle_key_j_scrolls_down() {
        let mut view = DashboardView::new();
        view.set_stats(DashboardStats::default(), sample_stale());
        assert_eq!(view.selected_stale, 0);
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.selected_stale, 1);
    }

    #[test]
    fn handle_key_k_scrolls_up() {
        let mut view = DashboardView::new();
        view.set_stats(DashboardStats::default(), sample_stale());
        view.selected_stale = 1;
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.selected_stale, 0);
    }

    #[test]
    fn handle_key_k_clamps_to_zero() {
        let mut view = DashboardView::new();
        view.set_stats(DashboardStats::default(), sample_stale());
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.selected_stale, 0);
    }

    #[test]
    fn handle_key_j_clamps_to_last() {
        let mut view = DashboardView::new();
        view.set_stats(DashboardStats::default(), sample_stale());
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.selected_stale, 1); // clamped to len-1
    }

    #[test]
    fn handle_key_j_empty_list_no_panic() {
        let mut view = DashboardView::new();
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.selected_stale, 0);
    }

    #[test]
    fn set_stats_resets_selection() {
        let mut view = DashboardView::new();
        view.set_stats(DashboardStats::default(), sample_stale());
        view.selected_stale = 1;
        view.set_stats(DashboardStats::default(), sample_stale());
        assert_eq!(view.selected_stale, 0);
    }

    #[test]
    fn name_returns_dashboard() {
        let view = DashboardView::new();
        assert_eq!(view.name(), "Dashboard");
    }

    #[test]
    fn renders_in_small_area() {
        let mut view = DashboardView::new();
        view.set_stats(sample_stats(), sample_stale());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn stage_short_labels_are_three_chars() {
        for stage in ApplicationStage::all() {
            assert_eq!(
                stage_short_label(*stage).len(),
                3,
                "stage_short_label for {stage:?} should be 3 chars"
            );
        }
    }
}
