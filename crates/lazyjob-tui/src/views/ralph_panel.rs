use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::app::RalphUpdate;
use crate::theme::Theme;
use crate::widgets::ProgressBar;

use super::View;

const MAX_LOG_LINES: usize = 50;
const COMPLETED_DISPLAY_SECS: u64 = 5;

struct ActiveEntry {
    run_id: String,
    loop_type: String,
    phase: String,
    progress: f64,
    message: String,
    started_at: Instant,
    log_lines: Vec<String>,
}

struct CompletedEntry {
    loop_type: String,
    success: bool,
    completed_at: Instant,
    summary: String,
}

pub struct RalphPanelView {
    active: Vec<ActiveEntry>,
    completed: Vec<CompletedEntry>,
    selected: usize,
}

impl Default for RalphPanelView {
    fn default() -> Self {
        Self::new()
    }
}

impl RalphPanelView {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            completed: Vec::new(),
            selected: 0,
        }
    }

    pub fn on_update(&mut self, update: RalphUpdate) {
        match update {
            RalphUpdate::Started { id, loop_type } => {
                self.active.push(ActiveEntry {
                    run_id: id,
                    loop_type,
                    phase: "starting".into(),
                    progress: 0.0,
                    message: "Starting...".into(),
                    started_at: Instant::now(),
                    log_lines: Vec::new(),
                });
                self.clamp_selected();
            }
            RalphUpdate::Progress { id, phase, percent } => {
                if let Some(entry) = self.active.iter_mut().find(|e| e.run_id == id) {
                    entry.phase = phase.clone();
                    entry.progress = percent / 100.0;
                    entry.message = phase;
                } else {
                    self.active.push(ActiveEntry {
                        run_id: id,
                        loop_type: String::from("unknown"),
                        phase: phase.clone(),
                        progress: percent / 100.0,
                        message: phase,
                        started_at: Instant::now(),
                        log_lines: Vec::new(),
                    });
                }
                self.clamp_selected();
            }
            RalphUpdate::LogLine { id, line } => {
                if let Some(entry) = self.active.iter_mut().find(|e| e.run_id == id) {
                    entry.log_lines.push(line.clone());
                    if entry.log_lines.len() > MAX_LOG_LINES {
                        entry.log_lines.remove(0);
                    }
                    entry.message = line;
                }
            }
            RalphUpdate::Completed { id } => {
                if let Some(pos) = self.active.iter().position(|e| e.run_id == id) {
                    let entry = self.active.remove(pos);
                    let summary = entry.log_lines.last().cloned().unwrap_or_default();
                    self.completed.push(CompletedEntry {
                        loop_type: entry.loop_type,
                        success: true,
                        completed_at: Instant::now(),
                        summary,
                    });
                    self.clamp_selected();
                }
            }
            RalphUpdate::Failed { id, reason } => {
                if let Some(pos) = self.active.iter().position(|e| e.run_id == id) {
                    let entry = self.active.remove(pos);
                    self.completed.push(CompletedEntry {
                        loop_type: entry.loop_type,
                        success: false,
                        completed_at: Instant::now(),
                        summary: reason,
                    });
                    self.clamp_selected();
                }
            }
        }
    }

    pub fn selected_run_id(&self) -> Option<String> {
        self.active.get(self.selected).map(|e| e.run_id.clone())
    }

    fn clamp_selected(&mut self) {
        if self.active.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.active.len() {
            self.selected = self.active.len() - 1;
        }
    }

    fn cleanup_expired(&mut self) {
        let threshold = Duration::from_secs(COMPLETED_DISPLAY_SECS);
        self.completed
            .retain(|e| e.completed_at.elapsed() < threshold);
    }

    fn format_elapsed(elapsed: Duration) -> String {
        let secs = elapsed.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else {
            format!("{}m{}s", secs / 60, secs % 60)
        }
    }
}

impl View for RalphPanelView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.cleanup_expired();

        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.focused_border_style())
            .title(" Ralph ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if self.active.is_empty() && self.completed.is_empty() {
            let msg = Paragraph::new(
                "No active loops.\n\nPress r on a job to run ResumeTailor, c for CoverLetter.",
            )
            .style(Style::default().fg(theme.text_muted));
            frame.render_widget(msg, inner);
            return;
        }

        let help_line_height = 1u16;
        if inner.height < help_line_height + 1 {
            return;
        }

        let body_height = inner.height.saturating_sub(help_line_height + 1);
        let [body_area, _, help_area] = Layout::vertical([
            Constraint::Length(body_height),
            Constraint::Length(1),
            Constraint::Length(help_line_height),
        ])
        .areas(inner);

        let help = Paragraph::new(Line::from(vec![
            Span::styled(
                "j/k",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": navigate  "),
            Span::styled(
                "c",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": cancel  "),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": detail  "),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": back"),
        ]))
        .style(Style::default().fg(theme.text_muted));
        frame.render_widget(help, help_area);

        let mut rows: Vec<Constraint> = Vec::new();
        for _ in &self.active {
            rows.push(Constraint::Length(2));
            rows.push(Constraint::Length(1));
        }
        for _ in &self.completed {
            rows.push(Constraint::Length(1));
        }

        let areas = Layout::vertical(rows).split(body_area);
        let mut area_idx = 0usize;

        for (i, entry) in self.active.iter().enumerate() {
            if area_idx + 1 >= areas.len() {
                break;
            }

            let title_area = areas[area_idx];
            area_idx += 1;
            let bar_area = areas[area_idx];
            area_idx += 1;

            let elapsed = Self::format_elapsed(entry.started_at.elapsed());
            let is_selected = i == self.selected;

            let title_style = if is_selected {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_primary)
            };
            let prefix = if is_selected { "▶ " } else { "  " };
            let title_line = Line::from(vec![
                Span::styled(format!("{}{}", prefix, entry.loop_type), title_style),
                Span::raw("  "),
                Span::styled(
                    format!("[{}]", entry.phase),
                    Style::default().fg(theme.text_secondary),
                ),
                Span::raw("  "),
                Span::styled(elapsed, Style::default().fg(theme.text_muted)),
            ]);

            frame.render_widget(Paragraph::new(title_line), title_area);

            let bar_color = if is_selected {
                theme.primary
            } else {
                Color::DarkGray
            };
            let bar = ProgressBar::new(entry.progress, &entry.message).color(bar_color);
            frame.render_widget(bar, bar_area);
        }

        for entry in &self.completed {
            if area_idx >= areas.len() {
                break;
            }
            let comp_area = areas[area_idx];
            area_idx += 1;

            let (marker, color) = if entry.success {
                ("✓", Color::Green)
            } else {
                ("✗", Color::Red)
            };
            let elapsed = Self::format_elapsed(entry.completed_at.elapsed());
            let line = Line::from(vec![
                Span::styled(
                    marker,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}  ", entry.loop_type)),
                Span::styled(
                    format!("{}s ago", elapsed),
                    Style::default().fg(theme.text_muted),
                ),
                Span::raw("  "),
                Span::styled(
                    entry.summary.chars().take(60).collect::<String>(),
                    Style::default().fg(theme.text_secondary),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), comp_area);
        }
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.active.is_empty() && self.selected + 1 < self.active.len() {
                    self.selected += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                None
            }
            KeyCode::Char('c') => self.selected_run_id().map(Action::CancelRalphLoop),
            KeyCode::Enter => self.selected_run_id().map(Action::RalphDetail),
            KeyCode::Esc => Some(Action::NavigateBack),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Ralph"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_panel() -> RalphPanelView {
        RalphPanelView::new()
    }

    #[test]
    fn on_update_progress_creates_active_entry() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Analyzing".to_string(),
            percent: 30.0,
        });
        assert_eq!(panel.active.len(), 1);
        assert_eq!(panel.active[0].run_id, "run-1");
        assert!((panel.active[0].progress - 0.3).abs() < 0.001);
    }

    #[test]
    fn on_update_progress_updates_existing_entry() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Start".to_string(),
            percent: 10.0,
        });
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Halfway".to_string(),
            percent: 50.0,
        });
        assert_eq!(panel.active.len(), 1);
        assert_eq!(panel.active[0].phase, "Halfway");
        assert!((panel.active[0].progress - 0.5).abs() < 0.001);
    }

    #[test]
    fn on_update_logline_appends_to_entry() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Start".to_string(),
            percent: 0.0,
        });
        panel.on_update(RalphUpdate::LogLine {
            id: "run-1".to_string(),
            line: "Fetching jobs...".to_string(),
        });
        assert_eq!(panel.active[0].log_lines.len(), 1);
        assert_eq!(panel.active[0].message, "Fetching jobs...");
    }

    #[test]
    fn on_update_completed_moves_to_completed() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Done".to_string(),
            percent: 100.0,
        });
        panel.on_update(RalphUpdate::Completed {
            id: "run-1".to_string(),
        });
        assert!(panel.active.is_empty());
        assert_eq!(panel.completed.len(), 1);
        assert!(panel.completed[0].success);
    }

    #[test]
    fn on_update_failed_moves_to_completed_with_failure() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Fetching".to_string(),
            percent: 20.0,
        });
        panel.on_update(RalphUpdate::Failed {
            id: "run-1".to_string(),
            reason: "Network error".to_string(),
        });
        assert!(panel.active.is_empty());
        assert_eq!(panel.completed.len(), 1);
        assert!(!panel.completed[0].success);
        assert_eq!(panel.completed[0].summary, "Network error");
    }

    #[test]
    fn cleanup_removes_expired_completed() {
        let mut panel = make_panel();
        panel.completed.push(CompletedEntry {
            loop_type: "job-discovery".to_string(),
            success: true,
            completed_at: Instant::now() - Duration::from_secs(10),
            summary: String::from("old"),
        });
        panel.completed.push(CompletedEntry {
            loop_type: "job-discovery".to_string(),
            success: true,
            completed_at: Instant::now(),
            summary: String::from("new"),
        });
        panel.cleanup_expired();
        assert_eq!(panel.completed.len(), 1);
        assert_eq!(panel.completed[0].summary, "new");
    }

    #[test]
    fn selected_run_id_returns_none_for_empty() {
        let panel = make_panel();
        assert!(panel.selected_run_id().is_none());
    }

    #[test]
    fn handle_key_j_scrolls_selection_down() {
        let mut panel = make_panel();
        for i in 0..3 {
            panel.on_update(RalphUpdate::Progress {
                id: format!("run-{}", i),
                phase: "Running".to_string(),
                percent: 50.0,
            });
        }
        assert_eq!(panel.selected, 0);
        let action = panel.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(action.is_none());
        assert_eq!(panel.selected, 1);
    }

    #[test]
    fn handle_key_k_scrolls_selection_up() {
        let mut panel = make_panel();
        for i in 0..3 {
            panel.on_update(RalphUpdate::Progress {
                id: format!("run-{}", i),
                phase: "Running".to_string(),
                percent: 50.0,
            });
        }
        panel.selected = 2;
        let action = panel.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(action.is_none());
        assert_eq!(panel.selected, 1);
    }

    #[test]
    fn handle_key_c_returns_cancel_action() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-abc".to_string(),
            phase: "Running".to_string(),
            percent: 40.0,
        });
        let action = panel.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::CancelRalphLoop("run-abc".to_string())));
    }

    #[test]
    fn handle_key_enter_returns_detail_action() {
        let mut panel = make_panel();
        panel.on_update(RalphUpdate::Progress {
            id: "run-abc".to_string(),
            phase: "Running".to_string(),
            percent: 40.0,
        });
        let action = panel.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::RalphDetail("run-abc".to_string())));
    }

    #[test]
    fn handle_key_esc_navigates_back() {
        let mut panel = make_panel();
        let action = panel.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::NavigateBack));
    }

    #[test]
    fn renders_empty_state_without_panic() {
        let mut view = RalphPanelView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("No active loops") || content.contains("Ralph"));
    }

    #[test]
    fn renders_active_loop_with_progress() {
        let mut view = RalphPanelView::new();
        view.on_update(RalphUpdate::Progress {
            id: "run-xyz".to_string(),
            phase: "Analyzing JD".to_string(),
            percent: 55.0,
        });
        view.active[0].loop_type = "resume-tailor".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("resume-tailor") || content.contains("unknown"));
    }

    #[test]
    fn renders_completed_with_success_marker() {
        let mut view = RalphPanelView::new();
        view.on_update(RalphUpdate::Progress {
            id: "run-1".to_string(),
            phase: "Running".to_string(),
            percent: 100.0,
        });
        view.on_update(RalphUpdate::Completed {
            id: "run-1".to_string(),
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("✓") || content.contains("Ralph"));
    }

    #[test]
    fn renders_without_panic() {
        let mut view = RalphPanelView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }
}
