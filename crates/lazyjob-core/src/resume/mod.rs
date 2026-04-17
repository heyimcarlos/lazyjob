pub mod content_drafter;
pub mod docx;
pub mod fabrication;
pub mod gap_analyzer;
pub mod jd_parser;
pub mod repository;
pub mod types;

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;

use crate::discovery::Completer;
use crate::domain::Job;
use crate::error::{CoreError, Result};
use crate::life_sheet::LifeSheet;

pub use self::types::*;

use self::content_drafter::{ContentDrafter, LlmContentDrafter};
use self::fabrication::DefaultFabricationAuditor;
use self::gap_analyzer::DefaultGapAnalyzer;
use self::jd_parser::{JobDescriptionParser, LlmJdParser, RegexJdParser};

pub struct ResumeTailor {
    jd_parser: LlmJdParser,
    fallback_parser: RegexJdParser,
    gap_analyzer: DefaultGapAnalyzer,
    content_drafter: LlmContentDrafter,
    fabrication_auditor: DefaultFabricationAuditor,
}

impl ResumeTailor {
    pub fn new(completer: Arc<dyn Completer>) -> Self {
        Self {
            jd_parser: LlmJdParser::new(Arc::clone(&completer)),
            fallback_parser: RegexJdParser,
            gap_analyzer: DefaultGapAnalyzer,
            content_drafter: LlmContentDrafter::new(completer),
            fabrication_auditor: DefaultFabricationAuditor,
        }
    }

    pub async fn tailor(
        &self,
        job: &Job,
        life_sheet: &LifeSheet,
        options: TailoringOptions,
        progress_tx: Option<mpsc::Sender<ProgressEvent>>,
    ) -> Result<(ResumeContent, GapReport, FabricationReport)> {
        if life_sheet.work_experience.is_empty() && life_sheet.skills.is_empty() {
            return Err(CoreError::Validation(
                "life sheet is empty — cannot tailor without profile data".into(),
            ));
        }

        let raw_jd = job
            .description
            .as_deref()
            .ok_or_else(|| CoreError::Validation("job has no description".into()))?;

        send_progress(&progress_tx, ProgressEvent::ParsingJd { pct: 10 }).await;

        // Stage 1: Parse JD
        let jd = match self.jd_parser.parse(raw_jd).await {
            Ok(jd) => jd,
            Err(_) => self.fallback_parser.parse_sync(raw_jd)?,
        };

        send_progress(&progress_tx, ProgressEvent::GapAnalysis { pct: 25 }).await;

        // Stage 2: Gap analysis
        let gap_report = self.gap_analyzer.analyze(life_sheet, &jd);

        send_progress(&progress_tx, ProgressEvent::FabricationPreCheck { pct: 35 }).await;

        // Stage 3: Fabrication pre-check on life sheet bullets
        let pre_check_content = build_pre_check_content(life_sheet);
        let pre_report = self
            .fabrication_auditor
            .audit(&pre_check_content, life_sheet);
        if options.strict_fabrication && !pre_report.is_safe_to_submit {
            return Err(CoreError::Validation(format!(
                "life sheet fabrication pre-check failed: {:?}",
                pre_report.errors
            )));
        }

        send_progress(&progress_tx, ProgressEvent::RewritingBullets { pct: 50 }).await;

        // Stages 4 & 5: Content drafting (bullet rewriting + summary generation)
        let content = self
            .content_drafter
            .draft(life_sheet, &jd, &gap_report, &options)
            .await?;

        send_progress(&progress_tx, ProgressEvent::Assembling { pct: 85 }).await;

        // Stage 6: Final fabrication audit
        let fabrication_report = self.fabrication_auditor.audit(&content, life_sheet);

        if options.strict_fabrication && !fabrication_report.is_safe_to_submit {
            return Err(CoreError::Validation(format!(
                "resume contains unsupported claims: {:?}",
                fabrication_report.errors
            )));
        }

        send_progress(
            &progress_tx,
            ProgressEvent::Done {
                match_score: gap_report.match_score,
            },
        )
        .await;

        Ok((content, gap_report, fabrication_report))
    }

    pub fn build_resume_version(
        job: &Job,
        content: ResumeContent,
        gap_report: GapReport,
        fabrication_report: FabricationReport,
        options: TailoringOptions,
        label: String,
    ) -> ResumeVersion {
        ResumeVersion {
            id: ResumeVersionId::new(),
            job_id: *job.id.as_uuid(),
            application_id: None,
            content,
            gap_report,
            fabrication_report,
            tailoring_options: options,
            created_at: Utc::now(),
            label,
            is_submitted: false,
        }
    }
}

fn build_pre_check_content(life_sheet: &LifeSheet) -> ResumeContent {
    let experience = life_sheet
        .work_experience
        .iter()
        .map(|exp| ExperienceSection {
            company: exp.company.clone(),
            title: exp.position.clone(),
            date_range: String::new(),
            bullets: exp
                .achievements
                .iter()
                .map(|a| a.description.clone())
                .collect(),
            rewritten_indices: vec![],
        })
        .collect();

    let skills = SkillsSection {
        primary: life_sheet
            .skills
            .iter()
            .flat_map(|cat| cat.skills.iter().map(|s| s.name.clone()))
            .collect(),
        secondary: vec![],
    };

    let certifications = life_sheet
        .certifications
        .iter()
        .map(|c| c.name.clone())
        .collect();

    ResumeContent {
        summary: String::new(),
        experience,
        skills,
        education: vec![],
        projects: vec![],
        certifications,
    }
}

async fn send_progress(tx: &Option<mpsc::Sender<ProgressEvent>>, event: ProgressEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::{
        Achievement, Basics, Certification, Skill, SkillCategory, WorkExperience,
    };

    struct MockCompleter {
        responses: std::sync::Mutex<Vec<String>>,
    }

    impl MockCompleter {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: std::sync::Mutex::new(
                    responses.into_iter().rev().map(String::from).collect(),
                ),
            }
        }
    }

    #[async_trait::async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            let mut responses = self.responses.lock().unwrap();
            responses
                .pop()
                .ok_or(CoreError::Http("no more mock responses".into()))
        }
    }

    fn make_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Jane Doe".into(),
                label: Some("Senior Engineer".into()),
                email: Some("jane@example.com".into()),
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "Acme Corp".into(),
                position: "Senior Software Engineer".into(),
                start_date: "2021-03".into(),
                end_date: None,
                location: None,
                url: None,
                summary: None,
                is_current: true,
                achievements: vec![
                    Achievement {
                        description: "Reduced API latency by 40% through caching layer redesign"
                            .into(),
                        metric_type: Some("percentage".into()),
                        metric_value: Some("40".into()),
                        metric_unit: None,
                    },
                    Achievement {
                        description: "Mentored 3 junior engineers".into(),
                        metric_type: None,
                        metric_value: None,
                        metric_unit: None,
                    },
                ],
                tech_stack: vec!["Rust".into(), "PostgreSQL".into()],
                team_size: Some(8),
                industry: None,
            }],
            education: vec![],
            skills: vec![SkillCategory {
                name: "Backend".into(),
                level: None,
                skills: vec![
                    Skill {
                        name: "Rust".into(),
                        years_experience: Some(4),
                        proficiency: None,
                    },
                    Skill {
                        name: "Python".into(),
                        years_experience: Some(8),
                        proficiency: None,
                    },
                ],
            }],
            certifications: vec![Certification {
                name: "AWS Solutions Architect".into(),
                authority: None,
                issue_date: None,
                expiry_date: None,
                url: None,
            }],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        }
    }

    fn make_job() -> Job {
        let mut job = Job::new("Senior Rust Backend Engineer");
        job.company_name = Some("TechCo".into());
        job.description = Some(
            "Requirements:\n- 5+ years Rust\n- PostgreSQL experience\n\
             Nice to Have:\n- Kubernetes\n\
             Responsibilities:\n- Build backend services\n- Mentor team members"
                .into(),
        );
        job
    }

    const CANNED_JD_JSON: &str = r#"{
        "required_skills": [
            {"name": "Rust", "canonical": "rust"},
            {"name": "PostgreSQL", "canonical": "postgresql"}
        ],
        "nice_to_have_skills": [
            {"name": "Kubernetes", "canonical": "kubernetes"}
        ],
        "keywords": ["rust", "backend", "postgresql"],
        "responsibilities": ["Build backend services", "Mentor team"]
    }"#;

    const CANNED_SUMMARY: &str = "Senior Software Engineer with 4+ years of Rust experience and deep PostgreSQL expertise. Proven ability to reduce API latency through innovative caching solutions while leading backend teams. Skilled in building scalable systems and mentoring engineering talent.";

    const CANNED_BULLETS: &str = r#"["Reduced API latency by 40% through Rust-based caching layer redesign for scalable backend systems", "Mentored 3 junior engineers on backend development best practices and Rust patterns"]"#;

    #[tokio::test]
    async fn full_pipeline_with_mock() {
        let completer = Arc::new(MockCompleter::new(vec![
            CANNED_JD_JSON,
            CANNED_SUMMARY,
            CANNED_BULLETS,
        ]));
        let tailor = ResumeTailor::new(completer);
        let job = make_job();
        let sheet = make_life_sheet();

        let (content, gap_report, fab_report) = tailor
            .tailor(&job, &sheet, TailoringOptions::default(), None)
            .await
            .unwrap();

        assert!(!content.summary.is_empty());
        assert_eq!(content.experience.len(), 1);
        assert_eq!(content.experience[0].company, "Acme Corp");
        assert!(!content.experience[0].bullets.is_empty());
        assert!(!content.skills.primary.is_empty());
        assert!(gap_report.match_score > 0.0);
        assert!(fab_report.is_safe_to_submit);
    }

    #[tokio::test]
    async fn pipeline_falls_back_to_regex_on_llm_failure() {
        let completer = Arc::new(MockCompleter::new(vec![
            "invalid json that will fail parsing",
            CANNED_SUMMARY,
            CANNED_BULLETS,
        ]));
        let tailor = ResumeTailor::new(completer);
        let job = make_job();
        let sheet = make_life_sheet();

        let result = tailor
            .tailor(&job, &sheet, TailoringOptions::default(), None)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pipeline_rejects_empty_life_sheet() {
        let completer = Arc::new(MockCompleter::new(vec![]));
        let tailor = ResumeTailor::new(completer);
        let job = make_job();
        let empty_sheet = LifeSheet {
            basics: Basics {
                name: "Empty".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![],
            education: vec![],
            skills: vec![],
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        };

        let result = tailor
            .tailor(&job, &empty_sheet, TailoringOptions::default(), None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pipeline_rejects_job_without_description() {
        let completer = Arc::new(MockCompleter::new(vec![]));
        let tailor = ResumeTailor::new(completer);
        let job = Job::new("No Description Job");
        let sheet = make_life_sheet();

        let result = tailor
            .tailor(&job, &sheet, TailoringOptions::default(), None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pipeline_emits_progress_events() {
        let completer = Arc::new(MockCompleter::new(vec![
            CANNED_JD_JSON,
            CANNED_SUMMARY,
            CANNED_BULLETS,
        ]));
        let tailor = ResumeTailor::new(completer);
        let job = make_job();
        let sheet = make_life_sheet();

        let (tx, mut rx) = mpsc::channel(16);
        tailor
            .tailor(&job, &sheet, TailoringOptions::default(), Some(tx))
            .await
            .unwrap();

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(events.len() >= 5);
        assert!(matches!(events[0], ProgressEvent::ParsingJd { .. }));
        assert!(matches!(events.last().unwrap(), ProgressEvent::Done { .. }));
    }

    #[test]
    fn build_resume_version_creates_valid_version() {
        let job = make_job();
        let content = ResumeContent::default();
        let gap = GapReport::default();
        let fab = FabricationReport::default();
        let opts = TailoringOptions::default();

        let version =
            ResumeTailor::build_resume_version(&job, content, gap, fab, opts, "v1".into());
        assert_eq!(version.label, "v1");
        assert_eq!(version.job_id, *job.id.as_uuid());
        assert!(!version.is_submitted);
    }
}
