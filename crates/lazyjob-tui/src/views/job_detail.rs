use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use lazyjob_core::domain::{Application, Job, StageTransition};
use lazyjob_core::networking::WarmPath;

use crate::action::Action;
use crate::theme::Theme;

use super::View;

pub struct JobDetailView {
    job: Option<Job>,
    application: Option<Application>,
    transitions: Vec<StageTransition>,
    warm_paths: Vec<WarmPath>,
    scroll_offset: u16,
}

impl Default for JobDetailView {
    fn default() -> Self {
        Self::new()
    }
}

impl JobDetailView {
    pub fn new() -> Self {
        Self {
            job: None,
            application: None,
            transitions: Vec::new(),
            warm_paths: Vec::new(),
            scroll_offset: 0,
        }
    }

    pub fn set_job(&mut self, job: Job) {
        self.job = Some(job);
        self.scroll_offset = 0;
    }

    pub fn set_application(
        &mut self,
        application: Option<Application>,
        transitions: Vec<StageTransition>,
    ) {
        self.application = application;
        self.transitions = transitions;
    }

    pub fn job(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    pub fn set_warm_paths(&mut self, paths: Vec<WarmPath>) {
        self.warm_paths = paths;
    }

    pub fn clear(&mut self) {
        self.job = None;
        self.application = None;
        self.transitions.clear();
        self.warm_paths.clear();
        self.scroll_offset = 0;
    }

    fn render_metadata<'a>(
        job: &'a Job,
        application: &Option<Application>,
        warm_paths: &[WarmPath],
        theme: &Theme,
    ) -> Text<'a> {
        let mut lines = Vec::new();

        lines.push(Line::from(vec![
            Span::styled("Company: ", Style::default().fg(theme.text_muted)),
            Span::styled(
                job.company_name.as_deref().unwrap_or("—"),
                Style::default().fg(theme.text_primary),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("Location: ", Style::default().fg(theme.text_muted)),
            Span::styled(
                job.location.as_deref().unwrap_or("—"),
                Style::default().fg(theme.text_primary),
            ),
        ]));

        let salary = format_salary(job.salary_min, job.salary_max);
        lines.push(Line::from(vec![
            Span::styled("Salary: ", Style::default().fg(theme.text_muted)),
            Span::styled(salary, Style::default().fg(theme.text_primary)),
        ]));

        let posted = format_relative_time(job.discovered_at);
        lines.push(Line::from(vec![
            Span::styled("Posted: ", Style::default().fg(theme.text_muted)),
            Span::styled(posted, Style::default().fg(theme.text_primary)),
        ]));

        lines.push(Line::from(""));

        if let Some(score) = job.match_score {
            let pct = (score * 100.0) as u8;
            let color = match pct {
                70.. => theme.success,
                40..=69 => theme.warning,
                _ => theme.error,
            };
            lines.push(Line::from(vec![
                Span::styled("Match: ", Style::default().fg(theme.text_muted)),
                Span::styled(
                    format!("{pct}%"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if let Some(ghost) = job.ghost_score {
            let (label, color) = if ghost >= 5.0 {
                ("High", theme.error)
            } else if ghost >= 3.0 {
                ("Medium", theme.warning)
            } else {
                ("Low", theme.success)
            };
            lines.push(Line::from(vec![
                Span::styled("Ghost: ", Style::default().fg(theme.text_muted)),
                Span::styled(label, Style::default().fg(color)),
            ]));
        }

        if let Some(source) = &job.source {
            lines.push(Line::from(vec![
                Span::styled("Source: ", Style::default().fg(theme.text_muted)),
                Span::styled(source.as_str(), Style::default().fg(theme.text_primary)),
            ]));
        }

        lines.push(Line::from(""));

        if let Some(app) = application {
            lines.push(Line::from(Span::styled(
                format!("Stage: {}", app.stage),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "Not applied",
                Style::default().fg(theme.text_muted),
            )));
        }

        if let Some(url) = &job.url {
            lines.push(Line::from(""));
            let display = if url.len() > 35 {
                format!("{}…", &url[..34])
            } else {
                url.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("URL: ", Style::default().fg(theme.text_muted)),
                Span::styled(display, Style::default().fg(theme.primary)),
            ]));
        }

        if let Some(notes) = &job.notes {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Notes:",
                Style::default().fg(theme.text_muted),
            )));
            lines.push(Line::from(Span::styled(
                notes.as_str(),
                Style::default().fg(theme.text_primary),
            )));
        }

        if !warm_paths.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Warm Paths ({}):", warm_paths.len()),
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )));
            for wp in warm_paths.iter().take(5) {
                let role_str = wp
                    .contact_role
                    .as_deref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(theme.success)),
                    Span::styled(
                        format!("{}{role_str}", wp.contact_name),
                        Style::default().fg(theme.text_primary),
                    ),
                ]));
            }
            if warm_paths.len() > 5 {
                lines.push(Line::from(Span::styled(
                    format!("  +{} more", warm_paths.len() - 5),
                    Style::default().fg(theme.text_muted),
                )));
            }
        }

        Text::from(lines)
    }

    fn render_history<'a>(transitions: &[StageTransition], theme: &Theme) -> Text<'a> {
        if transitions.is_empty() {
            return Text::from(Line::from(Span::styled(
                "No transition history",
                Style::default().fg(theme.text_muted),
            )));
        }

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "History:",
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::BOLD),
        )));

        for t in transitions.iter().rev() {
            let time = format_relative_time(t.transitioned_at);
            let note = t
                .notes
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(theme.primary)),
                Span::styled(
                    format!("{} → {}", t.from_stage, t.to_stage),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled(
                    format!(" ({time}{note})"),
                    Style::default().fg(theme.text_muted),
                ),
            ]));
        }

        Text::from(lines)
    }
}

impl View for JobDetailView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some(job) = &self.job else {
            let paragraph = Paragraph::new("Select a job from the Jobs list to view details.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme.focused_border_style())
                        .title(" Job Detail "),
                );
            frame.render_widget(paragraph, area);
            return;
        };

        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.focused_border_style())
            .title(format!(" {} ", job.title));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(inner);

        let body = chunks[0];
        let action_bar_area = chunks[1];

        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(body);

        let meta_area = body_chunks[0];
        let desc_area = body_chunks[1];

        let meta_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Min(0)])
            .split(meta_area);

        let metadata = Self::render_metadata(job, &self.application, &self.warm_paths, theme);
        let history = Self::render_history(&self.transitions, theme);

        let meta_height = metadata.height() as u16;
        let history_height = history.height() as u16;
        let available = meta_chunks[0].height;

        let (meta_constraint, hist_constraint) = if meta_height + history_height < available {
            (
                Constraint::Length(meta_height),
                Constraint::Length(history_height),
            )
        } else {
            (Constraint::Percentage(60), Constraint::Percentage(40))
        };

        let meta_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([meta_constraint, hist_constraint])
            .split(meta_chunks[0]);

        let meta_para = Paragraph::new(metadata).block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(theme.border)),
        );
        frame.render_widget(meta_para, meta_split[0]);

        let hist_para = Paragraph::new(history).block(
            Block::default()
                .borders(Borders::RIGHT | Borders::TOP)
                .border_style(Style::default().fg(theme.border)),
        );
        frame.render_widget(hist_para, meta_split[1]);

        let description = job
            .description
            .as_deref()
            .unwrap_or("No description available.");
        let desc_para = Paragraph::new(description)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0))
            .block(
                Block::default()
                    .borders(Borders::NONE)
                    .title(" Description ")
                    .title_style(
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
            );
        frame.render_widget(desc_para, desc_area);

        let has_application = self.application.is_some();
        let action_text = if has_application {
            " j/k=Scroll  r=Resume  c=Cover Letter  o=Open URL  Esc=Back "
        } else {
            " j/k=Scroll  a=Apply  r=Resume  c=Cover Letter  o=Open URL  Esc=Back "
        };
        let action_bar = Paragraph::new(action_text).style(Style::default().fg(theme.text_muted));
        frame.render_widget(action_bar, action_bar_area);
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        let job = self.job.as_ref()?;

        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                None
            }
            KeyCode::Char('o') => job.url.as_ref().map(|url| Action::OpenUrl(url.clone())),
            KeyCode::Char('a') => {
                if self.application.is_none() {
                    Some(Action::ApplyToJob(job.id))
                } else {
                    None
                }
            }
            KeyCode::Char('r') => Some(Action::TailorResume(job.id)),
            KeyCode::Char('c') => Some(Action::GenerateCoverLetter(job.id)),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Job Detail"
    }
}

fn format_salary(min: Option<i64>, max: Option<i64>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("${} – ${}", format_k(lo), format_k(hi)),
        (Some(lo), None) => format!("${}+", format_k(lo)),
        (None, Some(hi)) => format!("Up to ${}", format_k(hi)),
        (None, None) => "—".to_string(),
    }
}

fn format_k(cents: i64) -> String {
    if cents >= 1000 {
        format!("{}k", cents / 1000)
    } else {
        format!("{cents}")
    }
}

fn format_relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_days() > 365 {
        format!("{}y ago", diff.num_days() / 365)
    } else if diff.num_days() > 30 {
        format!("{}mo ago", diff.num_days() / 30)
    } else if diff.num_days() > 0 {
        format!("{}d ago", diff.num_days())
    } else if diff.num_hours() > 0 {
        format!("{}h ago", diff.num_hours())
    } else if diff.num_minutes() > 0 {
        format!("{}m ago", diff.num_minutes())
    } else {
        "just now".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_core::domain::{ApplicationId, ApplicationStage, JobId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sample_job() -> Job {
        let mut job = Job::new("Senior Rust Engineer");
        job.company_name = Some("Acme Corp".into());
        job.location = Some("Remote".into());
        job.salary_min = Some(120_000);
        job.salary_max = Some(180_000);
        job.url = Some("https://example.com/jobs/123".into());
        job.description = Some("We are looking for a Rust developer...".into());
        job.match_score = Some(0.85);
        job.ghost_score = Some(2.0);
        job.source = Some("greenhouse".into());
        job
    }

    fn sample_application(job_id: JobId) -> Application {
        let mut app = Application::new(job_id);
        app.stage = ApplicationStage::Applied;
        app
    }

    fn sample_transitions(app_id: ApplicationId) -> Vec<StageTransition> {
        vec![StageTransition {
            id: uuid::Uuid::new_v4(),
            application_id: app_id,
            from_stage: ApplicationStage::Interested,
            to_stage: ApplicationStage::Applied,
            transitioned_at: chrono::Utc::now(),
            notes: Some("Submitted online".into()),
        }]
    }

    #[test]
    fn set_job_stores_data() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let title = job.title.clone();
        view.set_job(job);
        assert_eq!(view.job.as_ref().unwrap().title, title);
    }

    #[test]
    fn set_job_resets_scroll() {
        let mut view = JobDetailView::new();
        view.scroll_offset = 10;
        view.set_job(sample_job());
        assert_eq!(view.scroll_offset, 0);
    }

    #[test]
    fn clear_removes_data() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        view.clear();
        assert!(view.job.is_none());
        assert!(view.application.is_none());
        assert!(view.transitions.is_empty());
        assert_eq!(view.scroll_offset, 0);
    }

    #[test]
    fn handle_key_returns_none_when_no_job() {
        let mut view = JobDetailView::new();
        assert_eq!(
            view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE),
            None
        );
        assert_eq!(
            view.handle_key(KeyCode::Char('o'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn handle_key_j_scrolls_down() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        assert_eq!(view.scroll_offset, 0);
        view.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 1);
        view.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 2);
    }

    #[test]
    fn handle_key_k_scrolls_up() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        view.scroll_offset = 5;
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 4);
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 3);
    }

    #[test]
    fn handle_key_k_clamps_to_zero() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        view.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 0);
    }

    #[test]
    fn handle_key_o_returns_open_url() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        let action = view.handle_key(KeyCode::Char('o'), KeyModifiers::NONE);
        assert_eq!(
            action,
            Some(Action::OpenUrl("https://example.com/jobs/123".into()))
        );
    }

    #[test]
    fn handle_key_o_returns_none_when_no_url() {
        let mut view = JobDetailView::new();
        let mut job = sample_job();
        job.url = None;
        view.set_job(job);
        assert_eq!(
            view.handle_key(KeyCode::Char('o'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn handle_key_a_returns_apply_when_not_applied() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let job_id = job.id;
        view.set_job(job);
        let action = view.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ApplyToJob(job_id)));
    }

    #[test]
    fn handle_key_a_returns_none_when_already_applied() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let app = sample_application(job.id);
        view.set_job(job);
        view.set_application(Some(app), vec![]);
        assert_eq!(
            view.handle_key(KeyCode::Char('a'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn handle_key_r_returns_tailor_resume() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let job_id = job.id;
        view.set_job(job);
        assert_eq!(
            view.handle_key(KeyCode::Char('r'), KeyModifiers::NONE),
            Some(Action::TailorResume(job_id))
        );
    }

    #[test]
    fn handle_key_c_returns_cover_letter() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let job_id = job.id;
        view.set_job(job);
        assert_eq!(
            view.handle_key(KeyCode::Char('c'), KeyModifiers::NONE),
            Some(Action::GenerateCoverLetter(job_id))
        );
    }

    #[test]
    fn renders_empty_state() {
        let mut view = JobDetailView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_with_job_data() {
        let mut view = JobDetailView::new();
        view.set_job(sample_job());
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..40)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Acme Corp"));
        assert!(all_text.contains("Remote"));
        assert!(all_text.contains("85%"));
    }

    #[test]
    fn renders_with_application_history() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let app = sample_application(job.id);
        let transitions = sample_transitions(app.id);
        view.set_job(job);
        view.set_application(Some(app), transitions);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..40)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Applied"));
    }

    #[test]
    fn renders_salary_formatting() {
        assert_eq!(format_salary(Some(120_000), Some(180_000)), "$120k – $180k");
        assert_eq!(format_salary(Some(120_000), None), "$120k+");
        assert_eq!(format_salary(None, Some(180_000)), "Up to $180k");
        assert_eq!(format_salary(None, None), "—");
        assert_eq!(format_salary(Some(500), Some(999)), "$500 – $999");
    }

    #[test]
    fn set_application_stores_data() {
        let mut view = JobDetailView::new();
        let job = sample_job();
        let app = sample_application(job.id);
        let transitions = sample_transitions(app.id);
        view.set_job(job);
        view.set_application(Some(app), transitions);
        assert!(view.application.is_some());
        assert_eq!(view.transitions.len(), 1);
    }
}
