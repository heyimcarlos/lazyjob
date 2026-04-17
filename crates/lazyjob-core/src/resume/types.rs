use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResumeVersionId(pub Uuid);

impl ResumeVersionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ResumeVersionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersion {
    pub id: ResumeVersionId,
    pub job_id: Uuid,
    pub application_id: Option<Uuid>,
    pub content: ResumeContent,
    pub gap_report: GapReport,
    pub fabrication_report: FabricationReport,
    pub tailoring_options: TailoringOptions,
    pub created_at: DateTime<Utc>,
    pub label: String,
    pub is_submitted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResumeContent {
    pub summary: String,
    pub experience: Vec<ExperienceSection>,
    pub skills: SkillsSection,
    pub education: Vec<EducationEntry>,
    pub projects: Vec<ProjectEntry>,
    pub certifications: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSection {
    pub company: String,
    pub title: String,
    pub date_range: String,
    pub bullets: Vec<String>,
    pub rewritten_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsSection {
    pub primary: Vec<String>,
    pub secondary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationEntry {
    pub degree: String,
    pub field: String,
    pub institution: String,
    pub graduation_year: Option<u16>,
    pub gpa: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub description: String,
    pub technologies: Vec<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailoringOptions {
    pub aggressiveness: f32,
    pub max_experience_years: u32,
    pub max_bullets_per_entry: usize,
    pub strict_fabrication: bool,
}

impl Default for TailoringOptions {
    fn default() -> Self {
        Self {
            aggressiveness: 0.6,
            max_experience_years: 10,
            max_bullets_per_entry: 4,
            strict_fabrication: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescriptionAnalysis {
    pub raw_text: String,
    pub required_skills: Vec<SkillRequirement>,
    pub nice_to_have_skills: Vec<SkillRequirement>,
    pub keywords: Vec<String>,
    pub responsibilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequirement {
    pub name: String,
    pub canonical: String,
    pub is_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GapReport {
    pub matched_skills: Vec<MatchedSkill>,
    pub missing_required: Vec<MissingSkill>,
    pub missing_nice_to_have: Vec<MissingSkill>,
    pub match_score: f32,
    pub relevant_experience_order: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedSkill {
    pub skill_name: String,
    pub evidence_source: SkillEvidenceSource,
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillEvidenceSource {
    ExplicitSkill,
    ExperienceBullet { company: String, index: usize },
    ProjectDescription { name: String },
    Certification { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingSkill {
    pub skill_name: String,
    pub is_required: bool,
    pub fabrication_risk: FabricationRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FabricationRisk {
    None,
    Low,
    High,
    Forbidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FabricationReport {
    pub items: Vec<FabricationItem>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub is_safe_to_submit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationItem {
    pub description: String,
    pub risk: FabricationRisk,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersionSummary {
    pub id: ResumeVersionId,
    pub label: String,
    pub match_score: f32,
    pub is_submitted: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressEvent {
    ParsingJd { pct: u8 },
    GapAnalysis { pct: u8 },
    FabricationPreCheck { pct: u8 },
    RewritingBullets { pct: u8 },
    GeneratingSummary { pct: u8 },
    Assembling { pct: u8 },
    Done { match_score: f32 },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_version_id_unique() {
        let a = ResumeVersionId::new();
        let b = ResumeVersionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn tailoring_options_default() {
        let opts = TailoringOptions::default();
        assert!((opts.aggressiveness - 0.6).abs() < f32::EPSILON);
        assert_eq!(opts.max_experience_years, 10);
        assert_eq!(opts.max_bullets_per_entry, 4);
        assert!(opts.strict_fabrication);
    }

    #[test]
    fn resume_content_serde_round_trip() {
        let content = ResumeContent {
            summary: "Experienced engineer".into(),
            experience: vec![ExperienceSection {
                company: "Acme".into(),
                title: "Engineer".into(),
                date_range: "2020 - Present".into(),
                bullets: vec!["Built systems".into()],
                rewritten_indices: vec![0],
            }],
            skills: SkillsSection {
                primary: vec!["Rust".into()],
                secondary: vec!["Python".into()],
            },
            education: vec![],
            projects: vec![],
            certifications: vec![],
        };
        let json = serde_json::to_string(&content).unwrap();
        let parsed: ResumeContent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.summary, "Experienced engineer");
        assert_eq!(parsed.experience[0].company, "Acme");
    }

    #[test]
    fn gap_report_serde_round_trip() {
        let report = GapReport {
            matched_skills: vec![MatchedSkill {
                skill_name: "Rust".into(),
                evidence_source: SkillEvidenceSource::ExplicitSkill,
                strength: 0.95,
            }],
            missing_required: vec![],
            missing_nice_to_have: vec![],
            match_score: 85.0,
            relevant_experience_order: vec![0, 1],
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: GapReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.matched_skills.len(), 1);
        assert!((parsed.match_score - 85.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fabrication_report_default_is_safe() {
        let report = FabricationReport::default();
        assert!(report.warnings.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn progress_event_serde() {
        let event = ProgressEvent::ParsingJd { pct: 10 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("ParsingJd"));
    }
}
