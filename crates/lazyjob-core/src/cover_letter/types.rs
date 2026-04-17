use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoverLetterId(pub Uuid);

impl CoverLetterId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CoverLetterId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CoverLetterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTemplate {
    #[default]
    StandardProfessional,
    ProblemSolution,
    CareerChanger,
}

impl CoverLetterTemplate {
    pub fn description(&self) -> &'static str {
        match self {
            Self::StandardProfessional => {
                "Hook opening, company-specific paragraph, 1-2 achievement paragraphs, call to action"
            }
            Self::ProblemSolution => {
                "Open with company challenge, how you solved it, call to action"
            }
            Self::CareerChanger => {
                "Acknowledge pivot, transferable skills, enthusiasm for new direction"
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StandardProfessional => "standard_professional",
            Self::ProblemSolution => "problem_solution",
            Self::CareerChanger => "career_changer",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "standard_professional" | "standard" | "professional" => Self::StandardProfessional,
            "problem_solution" | "problem" => Self::ProblemSolution,
            "career_changer" | "career" | "changer" => Self::CareerChanger,
            _ => Self::StandardProfessional,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTone {
    #[default]
    Professional,
    Casual,
    Creative,
}

impl CoverLetterTone {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Professional => "professional",
            Self::Casual => "casual",
            Self::Creative => "creative",
        }
    }

    pub fn prompt_description(&self) -> &'static str {
        match self {
            Self::Professional => "professional and confident",
            Self::Casual => "warm, conversational, and direct",
            Self::Creative => "creative, memorable, and slightly unconventional",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "professional" => Self::Professional,
            "casual" => Self::Casual,
            "creative" => Self::Creative,
            _ => Self::Professional,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterLength {
    Short,
    #[default]
    Standard,
    Detailed,
}

impl CoverLetterLength {
    pub fn word_target(self) -> u32 {
        match self {
            Self::Short => 200,
            Self::Standard => 300,
            Self::Detailed => 400,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Standard => "standard",
            Self::Detailed => "detailed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "short" => Self::Short,
            "standard" => Self::Standard,
            "detailed" => Self::Detailed,
            _ => Self::Standard,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverLetterOptions {
    pub template: CoverLetterTemplate,
    pub tone: CoverLetterTone,
    pub length: CoverLetterLength,
    pub quick_mode: bool,
    pub custom_intro: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterVersion {
    pub id: CoverLetterId,
    pub job_id: Uuid,
    pub application_id: Option<Uuid>,
    pub version: i32,
    pub template: CoverLetterTemplate,
    pub content: String,
    pub plain_text: String,
    pub key_points: Vec<String>,
    pub tone: CoverLetterTone,
    pub length: CoverLetterLength,
    pub options: CoverLetterOptions,
    pub diff_from_previous: Option<String>,
    pub is_submitted: bool,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterVersionSummary {
    pub id: CoverLetterId,
    pub version: i32,
    pub template: CoverLetterTemplate,
    pub tone: CoverLetterTone,
    pub is_submitted: bool,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressEvent {
    Generating { pct: u8 },
    CheckingFabrication { pct: u8 },
    Persisting { pct: u8 },
    Done { version: i32 },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_letter_id_generates_unique() {
        let a = CoverLetterId::new();
        let b = CoverLetterId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn cover_letter_id_display() {
        let id = CoverLetterId::new();
        let s = id.to_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn template_description_non_empty() {
        assert!(
            !CoverLetterTemplate::StandardProfessional
                .description()
                .is_empty()
        );
        assert!(
            !CoverLetterTemplate::ProblemSolution
                .description()
                .is_empty()
        );
        assert!(!CoverLetterTemplate::CareerChanger.description().is_empty());
    }

    #[test]
    fn template_from_str_loose() {
        assert_eq!(
            CoverLetterTemplate::from_str_loose("standard"),
            CoverLetterTemplate::StandardProfessional
        );
        assert_eq!(
            CoverLetterTemplate::from_str_loose("problem_solution"),
            CoverLetterTemplate::ProblemSolution
        );
        assert_eq!(
            CoverLetterTemplate::from_str_loose("career-changer"),
            CoverLetterTemplate::CareerChanger
        );
        assert_eq!(
            CoverLetterTemplate::from_str_loose("unknown"),
            CoverLetterTemplate::StandardProfessional
        );
    }

    #[test]
    fn tone_prompt_description() {
        assert!(
            CoverLetterTone::Professional
                .prompt_description()
                .contains("professional")
        );
        assert!(
            CoverLetterTone::Casual
                .prompt_description()
                .contains("conversational")
        );
        assert!(
            CoverLetterTone::Creative
                .prompt_description()
                .contains("creative")
        );
    }

    #[test]
    fn length_word_target() {
        assert_eq!(CoverLetterLength::Short.word_target(), 200);
        assert_eq!(CoverLetterLength::Standard.word_target(), 300);
        assert_eq!(CoverLetterLength::Detailed.word_target(), 400);
    }

    #[test]
    fn options_default() {
        let opts = CoverLetterOptions::default();
        assert_eq!(opts.template, CoverLetterTemplate::StandardProfessional);
        assert_eq!(opts.tone, CoverLetterTone::Professional);
        assert_eq!(opts.length, CoverLetterLength::Standard);
        assert!(!opts.quick_mode);
        assert!(opts.custom_intro.is_none());
    }

    #[test]
    fn version_serde_roundtrip() {
        let version = CoverLetterVersion {
            id: CoverLetterId::new(),
            job_id: Uuid::new_v4(),
            application_id: None,
            version: 1,
            template: CoverLetterTemplate::ProblemSolution,
            content: "Dear Hiring Manager...".into(),
            plain_text: "Dear Hiring Manager...".into(),
            key_points: vec!["First point".into()],
            tone: CoverLetterTone::Casual,
            length: CoverLetterLength::Short,
            options: CoverLetterOptions::default(),
            diff_from_previous: None,
            is_submitted: false,
            label: Some("draft 1".into()),
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&version).unwrap();
        let back: CoverLetterVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, version.id);
        assert_eq!(back.version, 1);
        assert_eq!(back.template, CoverLetterTemplate::ProblemSolution);
    }

    #[test]
    fn progress_event_serde() {
        let event = ProgressEvent::Generating { pct: 50 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Generating"));
    }
}
