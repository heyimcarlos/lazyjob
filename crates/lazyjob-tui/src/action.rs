use lazyjob_core::domain::{ApplicationId, ApplicationStage, ContactId, JobId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    NavigateTo(ViewId),
    NavigateBack,
    ToggleHelp,
    Refresh,
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
    Select,
    OpenJob(JobId),
    ApplyToJob(JobId),
    TailorResume(JobId),
    GenerateCoverLetter(JobId),
    DraftOutreach(ContactId),
    OpenUrl(String),
    CancelRalphLoop(String),
    RalphDetail(String),
    TransitionApplication(ApplicationId, ApplicationStage),
    EnterSearch,
    ExitSearch,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::NavigateTo(view) => view.label(),
            Self::NavigateBack => "Back",
            Self::ToggleHelp => "Toggle Help",
            Self::Refresh => "Refresh",
            Self::ScrollDown => "Scroll Down",
            Self::ScrollUp => "Scroll Up",
            Self::ScrollLeft => "Scroll Left",
            Self::ScrollRight => "Scroll Right",
            Self::Select => "Select",
            Self::OpenJob(_) => "Open Job",
            Self::ApplyToJob(_) => "Apply",
            Self::TailorResume(_) => "Tailor Resume",
            Self::GenerateCoverLetter(_) => "Cover Letter",
            Self::DraftOutreach(_) => "Draft Outreach",
            Self::OpenUrl(_) => "Open URL",
            Self::CancelRalphLoop(_) => "Cancel Loop",
            Self::RalphDetail(_) => "Loop Detail",
            Self::TransitionApplication(_, _) => "Transition Stage",
            Self::EnterSearch => "Search",
            Self::ExitSearch => "Exit Search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    Dashboard,
    Jobs,
    Applications,
    Contacts,
    Ralph,
    Settings,
}

impl ViewId {
    pub const ALL: [ViewId; 6] = [
        ViewId::Dashboard,
        ViewId::Jobs,
        ViewId::Applications,
        ViewId::Contacts,
        ViewId::Ralph,
        ViewId::Settings,
    ];

    pub fn tab_index(self) -> usize {
        match self {
            ViewId::Dashboard => 0,
            ViewId::Jobs => 1,
            ViewId::Applications => 2,
            ViewId::Contacts => 3,
            ViewId::Ralph => 4,
            ViewId::Settings => 5,
        }
    }

    pub fn from_tab_index(index: usize) -> Option<Self> {
        ViewId::ALL.get(index).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            ViewId::Dashboard => "Dashboard",
            ViewId::Jobs => "Jobs",
            ViewId::Applications => "Applications",
            ViewId::Contacts => "Contacts",
            ViewId::Ralph => "Ralph",
            ViewId::Settings => "Settings",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_id_tab_index_round_trips() {
        for view in ViewId::ALL {
            let index = view.tab_index();
            let recovered = ViewId::from_tab_index(index).unwrap();
            assert_eq!(view, recovered);
        }
    }

    #[test]
    fn from_tab_index_out_of_range_returns_none() {
        assert!(ViewId::from_tab_index(6).is_none());
        assert!(ViewId::from_tab_index(100).is_none());
    }

    #[test]
    fn all_views_have_labels() {
        for view in ViewId::ALL {
            assert!(!view.label().is_empty());
        }
    }
}
