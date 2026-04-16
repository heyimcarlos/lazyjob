use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ApplicationId, JobId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStage {
    Interested,
    Applied,
    PhoneScreen,
    Technical,
    Onsite,
    Offer,
    Accepted,
    Rejected,
    Withdrawn,
}

impl ApplicationStage {
    pub fn all() -> &'static [ApplicationStage] {
        &[
            Self::Interested,
            Self::Applied,
            Self::PhoneScreen,
            Self::Technical,
            Self::Onsite,
            Self::Offer,
            Self::Accepted,
            Self::Rejected,
            Self::Withdrawn,
        ]
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected | Self::Withdrawn)
    }

    pub fn valid_transitions(self) -> &'static [ApplicationStage] {
        use ApplicationStage::*;
        match self {
            Interested => &[Applied, Rejected, Withdrawn],
            Applied => &[PhoneScreen, Rejected, Withdrawn],
            PhoneScreen => &[Technical, Rejected, Withdrawn],
            Technical => &[Onsite, Rejected, Withdrawn],
            Onsite => &[Offer, Rejected, Withdrawn],
            Offer => &[Accepted, Rejected, Withdrawn],
            Accepted | Rejected | Withdrawn => &[],
        }
    }

    pub fn can_transition_to(self, next: ApplicationStage) -> bool {
        self.valid_transitions().contains(&next)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Interested => "interested",
            Self::Applied => "applied",
            Self::PhoneScreen => "phone_screen",
            Self::Technical => "technical",
            Self::Onsite => "onsite",
            Self::Offer => "offer",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }
}

impl std::str::FromStr for ApplicationStage {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "interested" => Ok(Self::Interested),
            "applied" => Ok(Self::Applied),
            "phone_screen" => Ok(Self::PhoneScreen),
            "technical" => Ok(Self::Technical),
            "onsite" => Ok(Self::Onsite),
            "offer" => Ok(Self::Offer),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "withdrawn" => Ok(Self::Withdrawn),
            other => Err(format!("unknown application stage: {other}")),
        }
    }
}

impl std::fmt::Display for ApplicationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interested => write!(f, "Interested"),
            Self::Applied => write!(f, "Applied"),
            Self::PhoneScreen => write!(f, "Phone Screen"),
            Self::Technical => write!(f, "Technical"),
            Self::Onsite => write!(f, "Onsite"),
            Self::Offer => write!(f, "Offer"),
            Self::Accepted => write!(f, "Accepted"),
            Self::Rejected => write!(f, "Rejected"),
            Self::Withdrawn => write!(f, "Withdrawn"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Application {
    pub id: ApplicationId,
    pub job_id: JobId,
    pub stage: ApplicationStage,
    pub submitted_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub resume_version: Option<String>,
    pub cover_letter_version: Option<String>,
    pub notes: Option<String>,
}

impl Application {
    pub fn new(job_id: JobId) -> Self {
        Self {
            id: ApplicationId::new(),
            job_id,
            stage: ApplicationStage::Interested,
            submitted_at: None,
            updated_at: Utc::now(),
            resume_version: None,
            cover_letter_version: None,
            notes: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StageTransition {
    pub id: uuid::Uuid,
    pub application_id: ApplicationId,
    pub from_stage: ApplicationStage,
    pub to_stage: ApplicationStage,
    pub transitioned_at: DateTime<Utc>,
    pub notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_defaults_to_interested() {
        let app = Application::new(JobId::new());
        assert_eq!(app.stage, ApplicationStage::Interested);
        assert!(app.submitted_at.is_none());
    }

    #[test]
    fn application_serde_round_trip() {
        let mut app = Application::new(JobId::new());
        app.stage = ApplicationStage::Technical;
        app.resume_version = Some("v2".into());

        let json = serde_json::to_string(&app).unwrap();
        let deserialized: Application = serde_json::from_str(&json).unwrap();
        assert_eq!(app, deserialized);
    }

    #[test]
    fn application_stage_serializes_snake_case() {
        let json = serde_json::to_string(&ApplicationStage::PhoneScreen).unwrap();
        assert_eq!(json, "\"phone_screen\"");
    }

    #[test]
    fn all_stages_returns_nine_variants() {
        assert_eq!(ApplicationStage::all().len(), 9);
    }

    #[test]
    fn as_str_round_trips_all_stages() {
        for stage in ApplicationStage::all() {
            let s = stage.as_str();
            let parsed: ApplicationStage = s.parse().unwrap();
            assert_eq!(*stage, parsed);
        }
    }

    #[test]
    fn from_str_invalid_returns_error() {
        let result: std::result::Result<ApplicationStage, _> = "bogus".parse();
        assert!(result.is_err());
    }

    #[test]
    fn is_terminal_for_terminal_stages() {
        assert!(ApplicationStage::Accepted.is_terminal());
        assert!(ApplicationStage::Rejected.is_terminal());
        assert!(ApplicationStage::Withdrawn.is_terminal());
    }

    #[test]
    fn is_terminal_false_for_active_stages() {
        use ApplicationStage::*;
        for stage in [Interested, Applied, PhoneScreen, Technical, Onsite, Offer] {
            assert!(!stage.is_terminal(), "{stage:?} should not be terminal");
        }
    }

    #[test]
    fn valid_forward_transitions() {
        use ApplicationStage::*;
        assert!(Interested.can_transition_to(Applied));
        assert!(Applied.can_transition_to(PhoneScreen));
        assert!(PhoneScreen.can_transition_to(Technical));
        assert!(Technical.can_transition_to(Onsite));
        assert!(Onsite.can_transition_to(Offer));
        assert!(Offer.can_transition_to(Accepted));
    }

    #[test]
    fn any_non_terminal_to_withdrawn() {
        use ApplicationStage::*;
        for stage in [Interested, Applied, PhoneScreen, Technical, Onsite, Offer] {
            assert!(
                stage.can_transition_to(Withdrawn),
                "{stage:?} should transition to Withdrawn"
            );
        }
    }

    #[test]
    fn any_non_terminal_to_rejected() {
        use ApplicationStage::*;
        for stage in [Interested, Applied, PhoneScreen, Technical, Onsite, Offer] {
            assert!(
                stage.can_transition_to(Rejected),
                "{stage:?} should transition to Rejected"
            );
        }
    }

    #[test]
    fn terminal_stages_have_no_transitions() {
        use ApplicationStage::*;
        for stage in [Accepted, Rejected, Withdrawn] {
            assert!(
                stage.valid_transitions().is_empty(),
                "{stage:?} should have no valid transitions"
            );
        }
    }

    #[test]
    fn cannot_skip_stages() {
        use ApplicationStage::*;
        assert!(!Interested.can_transition_to(Technical));
        assert!(!Interested.can_transition_to(Onsite));
        assert!(!Interested.can_transition_to(Offer));
        assert!(!Interested.can_transition_to(Accepted));
        assert!(!Applied.can_transition_to(Onsite));
        assert!(!Applied.can_transition_to(Offer));
        assert!(!PhoneScreen.can_transition_to(Onsite));
    }

    #[test]
    fn cannot_go_backward() {
        use ApplicationStage::*;
        assert!(!Applied.can_transition_to(Interested));
        assert!(!PhoneScreen.can_transition_to(Applied));
        assert!(!Technical.can_transition_to(PhoneScreen));
        assert!(!Onsite.can_transition_to(Technical));
        assert!(!Offer.can_transition_to(Onsite));
    }

    #[test]
    fn cannot_transition_from_terminal() {
        use ApplicationStage::*;
        for terminal in [Accepted, Rejected, Withdrawn] {
            for target in ApplicationStage::all() {
                assert!(
                    !terminal.can_transition_to(*target),
                    "{terminal:?} should not transition to {target:?}"
                );
            }
        }
    }

    #[test]
    fn exhaustive_transition_matrix() {
        use ApplicationStage::*;
        let expected: &[(ApplicationStage, ApplicationStage, bool)] = &[
            (Interested, Applied, true),
            (Interested, PhoneScreen, false),
            (Interested, Technical, false),
            (Interested, Onsite, false),
            (Interested, Offer, false),
            (Interested, Accepted, false),
            (Interested, Rejected, true),
            (Interested, Withdrawn, true),
            (Applied, Interested, false),
            (Applied, PhoneScreen, true),
            (Applied, Technical, false),
            (Applied, Onsite, false),
            (Applied, Offer, false),
            (Applied, Accepted, false),
            (Applied, Rejected, true),
            (Applied, Withdrawn, true),
            (PhoneScreen, Interested, false),
            (PhoneScreen, Applied, false),
            (PhoneScreen, Technical, true),
            (PhoneScreen, Onsite, false),
            (PhoneScreen, Offer, false),
            (PhoneScreen, Accepted, false),
            (PhoneScreen, Rejected, true),
            (PhoneScreen, Withdrawn, true),
            (Technical, Interested, false),
            (Technical, Applied, false),
            (Technical, PhoneScreen, false),
            (Technical, Onsite, true),
            (Technical, Offer, false),
            (Technical, Accepted, false),
            (Technical, Rejected, true),
            (Technical, Withdrawn, true),
            (Onsite, Interested, false),
            (Onsite, Applied, false),
            (Onsite, PhoneScreen, false),
            (Onsite, Technical, false),
            (Onsite, Offer, true),
            (Onsite, Accepted, false),
            (Onsite, Rejected, true),
            (Onsite, Withdrawn, true),
            (Offer, Interested, false),
            (Offer, Applied, false),
            (Offer, PhoneScreen, false),
            (Offer, Technical, false),
            (Offer, Onsite, false),
            (Offer, Accepted, true),
            (Offer, Rejected, true),
            (Offer, Withdrawn, true),
        ];
        for &(from, to, allowed) in expected {
            assert_eq!(
                from.can_transition_to(to),
                allowed,
                "{from:?} -> {to:?} should be {allowed}"
            );
        }
    }

    #[test]
    fn stage_transition_serde_round_trip() {
        let t = StageTransition {
            id: uuid::Uuid::new_v4(),
            application_id: ApplicationId::new(),
            from_stage: ApplicationStage::Applied,
            to_stage: ApplicationStage::PhoneScreen,
            transitioned_at: Utc::now(),
            notes: Some("recruiter called".into()),
        };
        let json = serde_json::to_string(&t).unwrap();
        let deserialized: StageTransition = serde_json::from_str(&json).unwrap();
        assert_eq!(t, deserialized);
    }
}
