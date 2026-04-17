use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use sqlx::PgPool;
use tokio::sync::{broadcast, mpsc};

use lazyjob_core::config::Config;
use lazyjob_core::networking;
use lazyjob_core::repositories::{
    ApplicationRepository, ContactRepository, JobRepository, Pagination,
};
use lazyjob_core::stats;

use crate::action::{Action, ViewId};
use crate::keybindings::KeyMap;
use crate::theme::Theme;
use crate::views::Views;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,
    Search,
    Command,
}

#[derive(Debug, Clone)]
pub enum RalphUpdate {
    Started {
        id: String,
        loop_type: String,
    },
    Progress {
        id: String,
        phase: String,
        percent: f64,
    },
    LogLine {
        id: String,
        line: String,
    },
    Completed {
        id: String,
    },
    Failed {
        id: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub enum RalphCommand {
    Spawn {
        loop_type: String,
        params: serde_json::Value,
    },
    Cancel {
        run_id: String,
    },
}

pub struct App {
    pub active_view: ViewId,
    pub prev_view: Option<ViewId>,
    pub viewing_job_detail: bool,
    pub should_quit: bool,
    pub help_open: bool,
    pub input_mode: InputMode,
    pub theme: &'static Theme,
    pub config: Arc<Config>,
    pub ralph_rx: broadcast::Receiver<RalphUpdate>,
    pub ralph_cmd_tx: mpsc::UnboundedSender<RalphCommand>,
    pub views: Views,
    pub keymap: KeyMap,
    pub pool: Option<PgPool>,
}

impl App {
    pub fn new(
        config: Arc<Config>,
        ralph_rx: broadcast::Receiver<RalphUpdate>,
        ralph_cmd_tx: mpsc::UnboundedSender<RalphCommand>,
    ) -> Self {
        let keymap = KeyMap::default_keymap().with_overrides(&config.keybindings);
        Self {
            active_view: ViewId::Dashboard,
            prev_view: None,
            viewing_job_detail: false,
            should_quit: false,
            help_open: false,
            input_mode: InputMode::Normal,
            theme: &Theme::DARK,
            config,
            ralph_rx,
            ralph_cmd_tx,
            views: Views::new(),
            keymap,
            pool: None,
        }
    }

    pub fn with_pool(mut self, pool: PgPool) -> Self {
        self.pool = Some(pool);
        self
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let (_tx, rx) = broadcast::channel(16);
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        Self::new(Arc::new(Config::default()), rx, cmd_tx)
    }

    pub fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.should_quit = true;
            }
            Action::NavigateTo(view) => {
                self.viewing_job_detail = false;
                if self.active_view != view {
                    self.prev_view = Some(self.active_view);
                    self.active_view = view;
                }
            }
            Action::NavigateBack => {
                if self.viewing_job_detail {
                    self.viewing_job_detail = false;
                    self.views.job_detail.clear();
                } else if let Some(prev) = self.prev_view.take() {
                    self.active_view = prev;
                }
            }
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
            }
            Action::Refresh => {}
            Action::ScrollDown => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Down, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::ScrollUp => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Up, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::Select => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Enter, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::OpenJob(id) => {
                let job = self
                    .views
                    .jobs_list
                    .jobs()
                    .iter()
                    .find(|j| j.id == id)
                    .cloned();
                if let Some(job) = job {
                    self.views.job_detail.set_job(job);
                    self.update_warm_paths();
                    self.viewing_job_detail = true;
                    if self.active_view != ViewId::Jobs {
                        self.prev_view = Some(self.active_view);
                        self.active_view = ViewId::Jobs;
                    }
                }
            }
            Action::ScrollLeft => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Left, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::ScrollRight => {
                if let Some(action) = self
                    .active_view_mut()
                    .handle_key(KeyCode::Right, KeyModifiers::NONE)
                {
                    self.handle_action(action);
                }
            }
            Action::TransitionApplication(_, _) => {}
            Action::ApplyToJob(_) => {}
            Action::TailorResume(job_id) => {
                let params = serde_json::json!({
                    "job_id": job_id.as_uuid().to_string(),
                    "database_url": self.config.database_url,
                });
                let _ = self.ralph_cmd_tx.send(RalphCommand::Spawn {
                    loop_type: "resume-tailor".into(),
                    params,
                });
            }
            Action::GenerateCoverLetter(job_id) => {
                let params = serde_json::json!({
                    "job_id": job_id.as_uuid().to_string(),
                    "database_url": self.config.database_url,
                });
                let _ = self.ralph_cmd_tx.send(RalphCommand::Spawn {
                    loop_type: "cover-letter".into(),
                    params,
                });
            }
            Action::OpenUrl(url) => {
                let _ = open::that(&url);
            }
            Action::CancelRalphLoop(run_id) => {
                let _ = self.ralph_cmd_tx.send(RalphCommand::Cancel { run_id });
            }
            Action::RalphDetail(_) => {}
            Action::EnterSearch => {
                self.input_mode = InputMode::Search;
            }
            Action::ExitSearch => {
                self.input_mode = InputMode::Normal;
            }
        }
    }

    pub fn active_view_mut(&mut self) -> &mut dyn crate::views::View {
        match self.active_view {
            ViewId::Dashboard => &mut self.views.dashboard,
            ViewId::Jobs => {
                if self.viewing_job_detail {
                    &mut self.views.job_detail
                } else {
                    &mut self.views.jobs_list
                }
            }
            ViewId::Applications => &mut self.views.applications,
            ViewId::Contacts => &mut self.views.contacts,
            ViewId::Ralph => &mut self.views.ralph_panel,
            ViewId::Settings => &mut self.views.settings,
        }
    }

    pub async fn load_jobs(&mut self) {
        let Some(pool) = &self.pool else { return };
        let repo = JobRepository::new(pool.clone());
        match repo.list(&Pagination::default()).await {
            Ok(jobs) => {
                self.views.jobs_list.set_jobs(jobs);
            }
            Err(err) => {
                tracing::warn!("Failed to load jobs: {err}");
            }
        }
    }

    pub async fn load_applications(&mut self) {
        use crate::views::applications::ApplicationCard;

        let Some(pool) = &self.pool else { return };
        let app_repo = ApplicationRepository::new(pool.clone());
        let job_repo = JobRepository::new(pool.clone());
        match app_repo.list(&Pagination::default()).await {
            Ok(applications) => {
                let mut cards = Vec::with_capacity(applications.len());
                for app in &applications {
                    let (title, company) = match job_repo.find_by_id(&app.job_id).await {
                        Ok(Some(job)) => (job.title, job.company_name.unwrap_or_default()),
                        _ => ("Unknown Job".to_string(), String::new()),
                    };
                    cards.push(ApplicationCard {
                        application_id: app.id,
                        title,
                        company,
                        stage: app.stage,
                        updated_at: app.updated_at,
                    });
                }
                self.views.applications.set_applications(cards);
            }
            Err(err) => {
                tracing::warn!("Failed to load applications: {err}");
            }
        }
    }

    pub async fn load_dashboard_stats(&mut self) {
        let Some(pool) = &self.pool else { return };
        let stats = match stats::compute_dashboard_stats(pool).await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("Failed to load dashboard stats: {err}");
                return;
            }
        };
        let stale = match stats::find_stale_applications(pool).await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("Failed to load stale applications: {err}");
                Vec::new()
            }
        };
        self.views.dashboard.set_stats(stats, stale);
    }

    pub async fn load_contacts(&mut self) {
        let Some(pool) = &self.pool else { return };
        let repo = ContactRepository::new(pool.clone());
        match repo
            .list(&Pagination {
                limit: 500,
                offset: 0,
            })
            .await
        {
            Ok(contacts) => {
                self.views.contacts.set_contacts(contacts);
            }
            Err(err) => {
                tracing::warn!("Failed to load contacts: {err}");
            }
        }
    }

    pub fn update_warm_paths(&mut self) {
        let job = match self.views.job_detail.job() {
            Some(j) => j.clone(),
            None => return,
        };
        let warm_paths = networking::warm_paths_for_job(self.views.contacts.contacts(), &job);
        self.views.job_detail.set_warm_paths(warm_paths);
    }

    pub fn handle_ralph_update(&mut self, update: RalphUpdate) {
        self.views.ralph_panel.on_update(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        App::new_for_test()
    }

    #[test]
    fn action_quit_sets_should_quit() {
        let mut app = test_app();
        assert!(!app.should_quit);
        app.handle_action(Action::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn action_navigate_sets_view() {
        let mut app = test_app();
        assert_eq!(app.active_view, ViewId::Dashboard);
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        assert_eq!(app.active_view, ViewId::Jobs);
        assert_eq!(app.prev_view, Some(ViewId::Dashboard));
    }

    #[test]
    fn action_navigate_back_restores_prev() {
        let mut app = test_app();
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        app.handle_action(Action::NavigateBack);
        assert_eq!(app.active_view, ViewId::Dashboard);
        assert!(app.prev_view.is_none());
    }

    #[test]
    fn action_navigate_back_with_no_prev_does_nothing() {
        let mut app = test_app();
        let original = app.active_view;
        app.handle_action(Action::NavigateBack);
        assert_eq!(app.active_view, original);
    }

    #[test]
    fn action_toggle_help() {
        let mut app = test_app();
        assert!(!app.help_open);
        app.handle_action(Action::ToggleHelp);
        assert!(app.help_open);
        app.handle_action(Action::ToggleHelp);
        assert!(!app.help_open);
    }

    #[test]
    fn navigate_to_same_view_does_not_change_prev() {
        let mut app = test_app();
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        app.handle_action(Action::NavigateTo(ViewId::Jobs));
        assert_eq!(app.prev_view, Some(ViewId::Dashboard));
    }

    #[test]
    fn default_input_mode_is_normal() {
        let app = test_app();
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn open_job_activates_detail_view() {
        use lazyjob_core::domain::Job;
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        let job = Job::new("Test Job");
        let job_id = job.id;
        app.views.jobs_list.set_jobs(vec![job]);
        app.handle_action(Action::OpenJob(job_id));
        assert!(app.viewing_job_detail);
    }

    #[test]
    fn open_job_with_unknown_id_does_nothing() {
        use lazyjob_core::domain::JobId;
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        app.handle_action(Action::OpenJob(JobId::new()));
        assert!(!app.viewing_job_detail);
    }

    #[test]
    fn navigate_back_from_detail_returns_to_jobs() {
        use lazyjob_core::domain::Job;
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        let job = Job::new("Test Job");
        let job_id = job.id;
        app.views.jobs_list.set_jobs(vec![job]);
        app.handle_action(Action::OpenJob(job_id));
        assert!(app.viewing_job_detail);
        app.handle_action(Action::NavigateBack);
        assert!(!app.viewing_job_detail);
        assert_eq!(app.active_view, ViewId::Jobs);
    }

    #[test]
    fn tab_switch_clears_detail_view() {
        use lazyjob_core::domain::Job;
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        let job = Job::new("Test Job");
        let job_id = job.id;
        app.views.jobs_list.set_jobs(vec![job]);
        app.handle_action(Action::OpenJob(job_id));
        assert!(app.viewing_job_detail);
        app.handle_action(Action::NavigateTo(ViewId::Dashboard));
        assert!(!app.viewing_job_detail);
        assert_eq!(app.active_view, ViewId::Dashboard);
    }

    #[test]
    fn active_view_mut_returns_job_detail_when_flag_set() {
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        app.viewing_job_detail = true;
        assert_eq!(app.active_view_mut().name(), "Job Detail");
    }

    #[test]
    fn active_view_mut_returns_jobs_list_when_flag_not_set() {
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        assert_eq!(app.active_view_mut().name(), "Jobs");
    }

    #[test]
    fn tailor_resume_sends_spawn_command() {
        use lazyjob_core::domain::JobId;

        let (_ralph_tx, ralph_rx) = broadcast::channel(16);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let mut app = App::new(Arc::new(Config::default()), ralph_rx, cmd_tx);

        let job_id = JobId::new();
        app.handle_action(Action::TailorResume(job_id));

        let cmd = cmd_rx.try_recv().unwrap();
        match cmd {
            RalphCommand::Spawn { loop_type, params } => {
                assert_eq!(loop_type, "resume-tailor");
                assert_eq!(params["job_id"], job_id.as_uuid().to_string());
            }
            _ => panic!("expected Spawn command"),
        }
    }

    #[test]
    fn generate_cover_letter_sends_spawn_command() {
        use lazyjob_core::domain::JobId;

        let (_ralph_tx, ralph_rx) = broadcast::channel(16);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let mut app = App::new(Arc::new(Config::default()), ralph_rx, cmd_tx);

        let job_id = JobId::new();
        app.handle_action(Action::GenerateCoverLetter(job_id));

        let cmd = cmd_rx.try_recv().unwrap();
        match cmd {
            RalphCommand::Spawn { loop_type, params } => {
                assert_eq!(loop_type, "cover-letter");
                assert_eq!(params["job_id"], job_id.as_uuid().to_string());
            }
            _ => panic!("expected Spawn command"),
        }
    }

    #[test]
    fn cancel_ralph_loop_sends_cancel_command() {
        let (_ralph_tx, ralph_rx) = broadcast::channel(16);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let mut app = App::new(Arc::new(Config::default()), ralph_rx, cmd_tx);

        app.handle_action(Action::CancelRalphLoop("run-123".into()));

        let cmd = cmd_rx.try_recv().unwrap();
        match cmd {
            RalphCommand::Cancel { run_id } => assert_eq!(run_id, "run-123"),
            _ => panic!("expected Cancel command"),
        }
    }

    #[test]
    fn handle_ralph_update_started_creates_entry() {
        let mut app = test_app();
        app.handle_ralph_update(RalphUpdate::Started {
            id: "run-1".into(),
            loop_type: "resume-tailor".into(),
        });
        assert_eq!(
            app.views.ralph_panel.selected_run_id(),
            Some("run-1".into())
        );
    }
}
