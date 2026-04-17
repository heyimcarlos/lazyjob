use std::collections::HashMap;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyModifiers};
use lazyjob_core::discovery::enrichment_badge;
use lazyjob_core::domain::{Job, JobId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use crate::action::Action;
use crate::theme::Theme;

use super::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobFilter {
    #[default]
    All,
    New,
    HighMatch,
    Applied,
}

impl JobFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::New => "New (≤7d)",
            Self::HighMatch => "Match ≥70%",
            Self::Applied => "Applied",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::New,
            Self::New => Self::HighMatch,
            Self::HighMatch => Self::Applied,
            Self::Applied => Self::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Date,
    Match,
    Company,
}

impl SortBy {
    fn label(self) -> &'static str {
        match self {
            Self::Date => "Date",
            Self::Match => "Match",
            Self::Company => "Company",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Date => Self::Match,
            Self::Match => Self::Company,
            Self::Company => Self::Date,
        }
    }
}

pub struct JobsListView {
    jobs: Vec<Job>,
    filtered: Vec<usize>,
    table_state: TableState,
    filter: JobFilter,
    sort: SortBy,
    search_query: String,
    search_active: bool,
    application_stages: HashMap<JobId, String>,
}

impl Default for JobsListView {
    fn default() -> Self {
        Self::new()
    }
}

impl JobsListView {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            filtered: Vec::new(),
            table_state: TableState::default(),
            filter: JobFilter::All,
            sort: SortBy::Date,
            search_query: String::new(),
            search_active: false,
            application_stages: HashMap::new(),
        }
    }

    pub fn set_jobs(&mut self, jobs: Vec<Job>) {
        self.jobs = jobs;
        self.apply_filter_sort();
        if !self.filtered.is_empty() {
            self.table_state.select(Some(0));
        }
    }

    pub fn set_application_stages(&mut self, stages: HashMap<JobId, String>) {
        self.application_stages = stages;
        self.apply_filter_sort();
    }

    pub fn selected_job(&self) -> Option<&Job> {
        let sel = self.table_state.selected()?;
        let idx = self.filtered.get(sel)?;
        self.jobs.get(*idx)
    }

    fn apply_filter_sort(&mut self) {
        let query = self.search_query.to_lowercase();

        let mut indices: Vec<usize> = (0..self.jobs.len())
            .filter(|&i| {
                let job = &self.jobs[i];
                self.match_filter_idx(job, &query)
            })
            .collect();

        match self.sort {
            SortBy::Date => {
                indices
                    .sort_by(|&a, &b| self.jobs[b].discovered_at.cmp(&self.jobs[a].discovered_at));
            }
            SortBy::Match => {
                indices.sort_by(|&a, &b| {
                    let sa = self.jobs[a].match_score.unwrap_or(0.0);
                    let sb = self.jobs[b].match_score.unwrap_or(0.0);
                    sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            SortBy::Company => {
                indices.sort_by(|&a, &b| {
                    let ca = self.jobs[a]
                        .company_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase();
                    let cb = self.jobs[b]
                        .company_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase();
                    ca.cmp(&cb)
                });
            }
        }

        let prev_sel = self.table_state.selected().unwrap_or(0);
        self.filtered = indices;

        if !self.filtered.is_empty() {
            let clamped = prev_sel.min(self.filtered.len() - 1);
            self.table_state.select(Some(clamped));
        } else {
            self.table_state.select(None);
        }
    }

    fn match_filter_idx(&self, job: &Job, query: &str) -> bool {
        let passes_filter = match self.filter {
            JobFilter::All => true,
            JobFilter::New => {
                let age = Utc::now() - job.discovered_at;
                age.num_days() <= 7
            }
            JobFilter::HighMatch => job.match_score.is_some_and(|s| s >= 0.70),
            JobFilter::Applied => self.application_stages.contains_key(&job.id),
        };

        if !passes_filter {
            return false;
        }

        if query.is_empty() {
            return true;
        }

        let title = job.title.to_lowercase();
        let company = job.company_name.as_deref().unwrap_or("").to_lowercase();
        title.contains(query) || company.contains(query)
    }

    fn scroll_down(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = (current + 1).min(self.filtered.len() - 1);
        self.table_state.select(Some(next));
    }

    fn scroll_up(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let prev = current.saturating_sub(1);
        self.table_state.select(Some(prev));
    }

    pub fn format_company_badge(company_name: &str, industry: Option<&str>) -> String {
        format!("{} {}", enrichment_badge(industry), company_name)
    }
}

fn relative_date(dt: &DateTime<Utc>) -> String {
    let age = Utc::now() - *dt;
    let days = age.num_days();
    if days < 1 {
        "today".to_string()
    } else if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", age.num_weeks())
    } else {
        format!("{}m", days / 30)
    }
}

fn match_pct_cell(score: Option<f64>, theme: &Theme) -> Cell<'static> {
    match score {
        Some(s) => {
            let pct = (s * 100.0).round() as u32;
            let color = if s >= 0.70 {
                theme.success
            } else if s >= 0.40 {
                theme.warning
            } else {
                theme.error
            };
            Cell::from(format!("{pct:>3}%")).style(Style::default().fg(color))
        }
        None => Cell::from("  ─  ").style(Style::default().fg(theme.text_muted)),
    }
}

fn ghost_cell(score: Option<f64>) -> Cell<'static> {
    if score.is_some_and(|s| s >= 5.0) {
        Cell::from("⚠").style(Style::default().fg(Color::LightRed))
    } else {
        Cell::from(" ").style(Style::default())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}

impl View for JobsListView {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);

        let body_area = chunks[0];
        let status_area = chunks[1];

        if self.filtered.is_empty() && self.jobs.is_empty() {
            let msg = Paragraph::new(
                "No jobs loaded. Run a discovery loop:\n  lazyjob ralph job-discovery --source greenhouse --company-id <id>",
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Jobs "),
            );
            frame.render_widget(msg, body_area);
        } else if self.filtered.is_empty() {
            let msg = Paragraph::new("No jobs match the current filter.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(" Jobs "),
            );
            frame.render_widget(msg, body_area);
        } else {
            let header = Row::new(vec![
                Cell::from("Title").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from("Company").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from("Match").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from("G").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from("Stage").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from("Posted").style(
                    Style::default()
                        .fg(theme.text_secondary)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
            .height(1)
            .bottom_margin(0);

            let rows: Vec<Row> = self
                .filtered
                .iter()
                .map(|&idx| {
                    let job = &self.jobs[idx];
                    let title = truncate(&job.title, 32);
                    let company_raw = job.company_name.as_deref().unwrap_or("─");
                    let company = truncate(company_raw, 25);
                    let posted = relative_date(&job.discovered_at);

                    let stage = self
                        .application_stages
                        .get(&job.id)
                        .map(|s| s.as_str())
                        .unwrap_or("─");

                    Row::new(vec![
                        Cell::from(title).style(Style::default().fg(theme.text_primary)),
                        Cell::from(company).style(Style::default().fg(theme.primary)),
                        match_pct_cell(job.match_score, theme),
                        ghost_cell(job.ghost_score),
                        Cell::from(stage.to_string())
                            .style(Style::default().fg(theme.text_secondary)),
                        Cell::from(posted).style(Style::default().fg(theme.text_secondary)),
                    ])
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Fill(3),
                    Constraint::Fill(2),
                    Constraint::Length(5),
                    Constraint::Length(1),
                    Constraint::Length(11),
                    Constraint::Length(6),
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.focused_border_style())
                    .title(format!(
                        " Jobs ({}/{}) ",
                        self.filtered.len(),
                        self.jobs.len()
                    )),
            )
            .row_highlight_style(theme.selected_style())
            .highlight_symbol("▶ ");

            frame.render_stateful_widget(table, body_area, &mut self.table_state);
        }

        let status_line = if self.search_active {
            Line::from(vec![
                Span::styled(
                    " Search: ",
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    self.search_query.clone(),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled("█", Style::default().fg(theme.primary)),
                Span::raw("  "),
                Span::styled(
                    "[Esc] cancel  [Enter] confirm",
                    Style::default().fg(theme.text_muted),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" Filter: {} ", self.filter.label()),
                    Style::default().fg(theme.text_secondary),
                ),
                Span::styled("│", Style::default().fg(theme.border)),
                Span::styled(
                    format!(" Sort: {} ", self.sort.label()),
                    Style::default().fg(theme.text_secondary),
                ),
                Span::styled("│", Style::default().fg(theme.border)),
                Span::styled(
                    "  [/] search  [f] filter  [s] sort  [j/k] navigate  [Enter] open",
                    Style::default().fg(theme.text_muted),
                ),
            ])
        };

        let status = Paragraph::new(status_line).style(Style::default().bg(theme.bg_secondary));
        frame.render_widget(status, status_area);
    }

    fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        if self.search_active {
            return match code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.apply_filter_sort();
                    Some(Action::ExitSearch)
                }
                KeyCode::Enter => {
                    self.search_active = false;
                    Some(Action::ExitSearch)
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.apply_filter_sort();
                    None
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.apply_filter_sort();
                    None
                }
                _ => None,
            };
        }

        match code {
            KeyCode::Char('/') => {
                self.search_active = true;
                Some(Action::EnterSearch)
            }
            KeyCode::Char('f') => {
                self.filter = self.filter.next();
                self.apply_filter_sort();
                None
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.apply_filter_sort();
                None
            }
            KeyCode::Down => {
                self.scroll_down();
                None
            }
            KeyCode::Up => {
                self.scroll_up();
                None
            }
            KeyCode::Enter => self.selected_job().map(|job| Action::OpenJob(job.id)),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        "Jobs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use chrono::Duration;
    use lazyjob_core::domain::Job;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_job(title: &str, company: &str, match_score: Option<f64>, days_old: i64) -> Job {
        let mut job = Job::new(title);
        job.company_name = Some(company.to_string());
        job.match_score = match_score;
        job.discovered_at = Utc::now() - Duration::days(days_old);
        job
    }

    fn make_ghost_job(title: &str) -> Job {
        let mut job = Job::new(title);
        job.ghost_score = Some(6.0);
        job
    }

    fn three_jobs() -> Vec<Job> {
        vec![
            make_job("Senior Rust Engineer", "Stripe", Some(0.85), 1),
            make_job("Backend Developer", "Acme Corp", Some(0.45), 10),
            make_job("Golang Engineer", "Widgets LLC", Some(0.30), 3),
        ]
    }

    #[test]
    fn set_jobs_populates_list() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        assert_eq!(view.jobs.len(), 3);
        assert_eq!(view.filtered.len(), 3);
    }

    #[test]
    fn filter_all_shows_everything() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.filter = JobFilter::All;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 3);
    }

    #[test]
    fn filter_high_match_excludes_low_scores() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.filter = JobFilter::HighMatch;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 1);
        let idx = view.filtered[0];
        assert!(view.jobs[idx].match_score.unwrap() >= 0.70);
    }

    #[test]
    fn filter_new_excludes_old_jobs() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.filter = JobFilter::New;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 2);
        for &idx in &view.filtered {
            let age = Utc::now() - view.jobs[idx].discovered_at;
            assert!(age.num_days() <= 7);
        }
    }

    #[test]
    fn sort_by_match_orders_highest_first() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.sort = SortBy::Match;
        view.apply_filter_sort();
        let scores: Vec<f64> = view
            .filtered
            .iter()
            .map(|&i| view.jobs[i].match_score.unwrap_or(0.0))
            .collect();
        for w in scores.windows(2) {
            assert!(w[0] >= w[1]);
        }
    }

    #[test]
    fn sort_by_company_orders_alphabetically() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.sort = SortBy::Company;
        view.apply_filter_sort();
        let companies: Vec<String> = view
            .filtered
            .iter()
            .map(|&i| {
                view.jobs[i]
                    .company_name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
            })
            .collect();
        for w in companies.windows(2) {
            assert!(w[0] <= w[1]);
        }
    }

    #[test]
    fn search_filters_by_title() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.search_query = "rust".to_string();
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 1);
        let idx = view.filtered[0];
        assert!(view.jobs[idx].title.to_lowercase().contains("rust"));
    }

    #[test]
    fn search_case_insensitive() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.search_query = "STRIPE".to_string();
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 1);
    }

    #[test]
    fn search_matches_company() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.search_query = "acme".to_string();
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 1);
        let idx = view.filtered[0];
        assert_eq!(view.jobs[idx].company_name.as_deref(), Some("Acme Corp"));
    }

    #[test]
    fn scroll_down_moves_selection() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        assert_eq!(view.table_state.selected(), Some(0));
        view.scroll_down();
        assert_eq!(view.table_state.selected(), Some(1));
    }

    #[test]
    fn scroll_down_clamps_at_end() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.scroll_down();
        view.scroll_down();
        view.scroll_down();
        assert_eq!(view.table_state.selected(), Some(2));
    }

    #[test]
    fn scroll_up_at_top_clamps() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.scroll_up();
        assert_eq!(view.table_state.selected(), Some(0));
    }

    #[test]
    fn handle_key_slash_returns_enter_search() {
        let mut view = JobsListView::new();
        let action = view.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(action, Some(Action::EnterSearch));
        assert!(view.search_active);
    }

    #[test]
    fn handle_key_f_cycles_filter() {
        let mut view = JobsListView::new();
        assert_eq!(view.filter, JobFilter::All);
        view.handle_key(KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(view.filter, JobFilter::New);
        view.handle_key(KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(view.filter, JobFilter::HighMatch);
        view.handle_key(KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(view.filter, JobFilter::Applied);
        view.handle_key(KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(view.filter, JobFilter::All);
    }

    #[test]
    fn handle_key_s_cycles_sort() {
        let mut view = JobsListView::new();
        assert_eq!(view.sort, SortBy::Date);
        view.handle_key(KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(view.sort, SortBy::Match);
        view.handle_key(KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(view.sort, SortBy::Company);
        view.handle_key(KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(view.sort, SortBy::Date);
    }

    #[test]
    fn handle_key_search_char_appends() {
        let mut view = JobsListView::new();
        view.search_active = true;
        view.handle_key(KeyCode::Char('r'), KeyModifiers::NONE);
        view.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert_eq!(view.search_query, "ru");
    }

    #[test]
    fn handle_key_backspace_removes_char() {
        let mut view = JobsListView::new();
        view.search_active = true;
        view.search_query = "rust".to_string();
        view.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(view.search_query, "rus");
    }

    #[test]
    fn handle_key_esc_in_search_exits_and_clears() {
        let mut view = JobsListView::new();
        view.search_active = true;
        view.search_query = "rust".to_string();
        let action = view.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ExitSearch));
        assert!(!view.search_active);
        assert!(view.search_query.is_empty());
    }

    #[test]
    fn handle_key_enter_in_search_exits_keeps_query() {
        let mut view = JobsListView::new();
        view.search_active = true;
        view.search_query = "rust".to_string();
        let action = view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::ExitSearch));
        assert!(!view.search_active);
        assert_eq!(view.search_query, "rust");
    }

    #[test]
    fn handle_key_down_scrolls_selection() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(1));
    }

    #[test]
    fn handle_key_up_scrolls_selection() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.scroll_down();
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(view.table_state.selected(), Some(0));
    }

    #[test]
    fn renders_jobs_table_without_panic() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_empty_state_without_panic() {
        let mut view = JobsListView::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
    }

    #[test]
    fn renders_job_title_in_buffer() {
        let mut view = JobsListView::new();
        view.set_jobs(vec![make_job(
            "Senior Rust Engineer",
            "Stripe",
            Some(0.85),
            1,
        )]);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Senior Rust Engineer"));
    }

    #[test]
    fn renders_search_status_when_active() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.search_active = true;
        view.search_query = "rust".to_string();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Search"));
        assert!(all_text.contains("rust"));
    }

    #[test]
    fn renders_filter_status_in_normal_mode() {
        let mut view = JobsListView::new();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Filter"));
        assert!(all_text.contains("All"));
    }

    #[test]
    fn relative_date_today() {
        let dt = Utc::now();
        assert_eq!(relative_date(&dt), "today");
    }

    #[test]
    fn relative_date_days() {
        let dt = Utc::now() - Duration::days(3);
        assert_eq!(relative_date(&dt), "3d");
    }

    #[test]
    fn relative_date_weeks() {
        let dt = Utc::now() - Duration::days(14);
        assert_eq!(relative_date(&dt), "2w");
    }

    #[test]
    fn relative_date_months() {
        let dt = Utc::now() - Duration::days(45);
        assert_eq!(relative_date(&dt), "1m");
    }

    #[test]
    fn selected_job_returns_correct_job() {
        let mut view = JobsListView::new();
        let jobs = three_jobs();
        let first_title = jobs[0].title.clone();
        view.set_jobs(jobs);
        let selected = view.selected_job();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().title, first_title);
    }

    #[test]
    fn selected_job_returns_none_for_empty_list() {
        let view = JobsListView::new();
        assert!(view.selected_job().is_none());
    }

    #[test]
    fn ghost_job_not_excluded_by_default() {
        let mut view = JobsListView::new();
        view.set_jobs(vec![make_ghost_job("Ghost Job")]);
        assert_eq!(view.filtered.len(), 1);
    }

    #[test]
    fn handle_key_enter_returns_open_job() {
        let mut view = JobsListView::new();
        let jobs = three_jobs();
        let expected_id = jobs[0].id;
        view.set_jobs(jobs);
        let action = view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(action, Some(Action::OpenJob(expected_id)));
    }

    #[test]
    fn handle_key_enter_returns_none_when_empty() {
        let mut view = JobsListView::new();
        let action = view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(action, None);
    }

    #[test]
    fn filter_applied_shows_only_applied_jobs() {
        let mut view = JobsListView::new();
        let jobs = three_jobs();
        let applied_id = jobs[0].id;
        view.set_jobs(jobs);

        let mut stages = HashMap::new();
        stages.insert(applied_id, "applied".to_string());
        view.set_application_stages(stages);

        view.filter = JobFilter::Applied;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 1);
        assert_eq!(view.jobs[view.filtered[0]].id, applied_id);
    }

    #[test]
    fn filter_applied_empty_when_no_applications() {
        let mut view = JobsListView::new();
        view.set_jobs(three_jobs());
        view.filter = JobFilter::Applied;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 0);
    }

    #[test]
    fn set_application_stages_updates_filter() {
        let mut view = JobsListView::new();
        let jobs = three_jobs();
        let id0 = jobs[0].id;
        let id1 = jobs[1].id;
        view.set_jobs(jobs);

        let mut stages = HashMap::new();
        stages.insert(id0, "applied".to_string());
        stages.insert(id1, "phone_screen".to_string());
        view.set_application_stages(stages);

        view.filter = JobFilter::Applied;
        view.apply_filter_sort();
        assert_eq!(view.filtered.len(), 2);
    }

    #[test]
    fn stage_column_renders_in_table() {
        let mut view = JobsListView::new();
        let jobs = three_jobs();
        let id = jobs[0].id;
        view.set_jobs(jobs);

        let mut stages = HashMap::new();
        stages.insert(id, "applied".to_string());
        view.set_application_stages(stages);

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &Theme::DARK))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Stage"));
        assert!(all_text.contains("applied"));
    }

    #[test]
    fn format_company_badge_with_industry() {
        let formatted = JobsListView::format_company_badge("Stripe", Some("Fintech"));
        assert_eq!(formatted, "[E] Stripe");
    }

    #[test]
    fn format_company_badge_without_industry() {
        let formatted = JobsListView::format_company_badge("Unknown Corp", None);
        assert_eq!(formatted, "[ ] Unknown Corp");
    }
}
