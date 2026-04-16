pub mod applications;
pub mod contacts;
pub mod dashboard;
pub mod help_overlay;
pub mod job_detail;
pub mod jobs_list;
pub mod ralph_panel;
pub mod settings;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::theme::Theme;

pub trait View {
    fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme);
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn name(&self) -> &'static str;
}

pub struct Views {
    pub dashboard: dashboard::DashboardView,
    pub jobs_list: jobs_list::JobsListView,
    pub job_detail: job_detail::JobDetailView,
    pub applications: applications::ApplicationsView,
    pub contacts: contacts::ContactsView,
    pub ralph_panel: ralph_panel::RalphPanelView,
    pub settings: settings::SettingsView,
    pub help_overlay: help_overlay::HelpOverlay,
}

impl Views {
    pub fn new() -> Self {
        Self {
            dashboard: dashboard::DashboardView::new(),
            jobs_list: jobs_list::JobsListView::new(),
            job_detail: job_detail::JobDetailView::new(),
            applications: applications::ApplicationsView::new(),
            contacts: contacts::ContactsView::new(),
            ralph_panel: ralph_panel::RalphPanelView::new(),
            settings: settings::SettingsView::new(),
            help_overlay: help_overlay::HelpOverlay::new(),
        }
    }
}

impl Default for Views {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ViewId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn all_views_implement_view_trait() {
        let mut views = Views::new();
        let theme = &Theme::DARK;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let view_list: Vec<(&mut dyn View, &str)> = vec![
            (&mut views.dashboard, "Dashboard"),
            (&mut views.jobs_list, "Jobs"),
            (&mut views.applications, "Applications"),
            (&mut views.contacts, "Contacts"),
            (&mut views.ralph_panel, "Ralph"),
            (&mut views.settings, "Settings"),
        ];

        for (view, expected_name) in view_list {
            assert_eq!(view.name(), expected_name);
            terminal
                .draw(|f| {
                    view.render(f, f.area(), theme);
                })
                .unwrap();
        }
    }

    #[test]
    fn stub_views_return_none_for_all_keys() {
        let mut views = Views::new();
        let test_keys = [
            (KeyCode::Char('j'), KeyModifiers::NONE),
            (KeyCode::Char('k'), KeyModifiers::NONE),
            (KeyCode::Enter, KeyModifiers::NONE),
            (KeyCode::Char('x'), KeyModifiers::NONE),
        ];

        let view_list: Vec<&mut dyn View> = vec![
            &mut views.dashboard,
            &mut views.jobs_list,
            &mut views.applications,
            &mut views.contacts,
            &mut views.ralph_panel,
            &mut views.settings,
        ];

        for view in view_list {
            for (code, mods) in &test_keys {
                assert_eq!(view.handle_key(*code, *mods), None);
            }
        }
    }

    #[test]
    fn views_new_creates_all_views() {
        let views = Views::new();
        assert_eq!(views.dashboard.name(), "Dashboard");
        assert_eq!(views.jobs_list.name(), "Jobs");
        assert_eq!(views.job_detail.name(), "Job Detail");
        assert_eq!(views.applications.name(), "Applications");
        assert_eq!(views.contacts.name(), "Contacts");
        assert_eq!(views.ralph_panel.name(), "Ralph");
        assert_eq!(views.settings.name(), "Settings");
    }

    #[test]
    fn view_id_maps_to_correct_view_name() {
        let views = Views::new();
        let mappings: Vec<(ViewId, &str)> = vec![
            (ViewId::Dashboard, views.dashboard.name()),
            (ViewId::Jobs, views.jobs_list.name()),
            (ViewId::Applications, views.applications.name()),
            (ViewId::Contacts, views.contacts.name()),
            (ViewId::Ralph, views.ralph_panel.name()),
            (ViewId::Settings, views.settings.name()),
        ];

        for (view_id, view_name) in mappings {
            assert_eq!(view_id.label(), view_name);
        }
    }
}
