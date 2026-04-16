use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ApplicationId, InterviewId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Interview {
    pub id: InterviewId,
    pub application_id: ApplicationId,
    pub interview_type: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub completed: bool,
}

impl Interview {
    pub fn new(application_id: ApplicationId, interview_type: impl Into<String>) -> Self {
        Self {
            id: InterviewId::new(),
            application_id,
            interview_type: interview_type.into(),
            scheduled_at: None,
            location: None,
            notes: None,
            completed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interview_construction() {
        let interview = Interview::new(ApplicationId::new(), "Technical");
        assert_eq!(interview.interview_type, "Technical");
        assert!(!interview.completed);
    }

    #[test]
    fn interview_serde_round_trip() {
        let mut interview = Interview::new(ApplicationId::new(), "Phone Screen");
        interview.location = Some("Zoom".into());
        interview.completed = true;

        let json = serde_json::to_string(&interview).unwrap();
        let deserialized: Interview = serde_json::from_str(&json).unwrap();
        assert_eq!(interview, deserialized);
    }
}
