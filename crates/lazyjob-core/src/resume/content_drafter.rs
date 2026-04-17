use std::sync::Arc;

use async_trait::async_trait;

use crate::discovery::Completer;
use crate::error::Result;
use crate::life_sheet::LifeSheet;

use super::types::{
    ExperienceSection, GapReport, JobDescriptionAnalysis, ResumeContent, SkillsSection,
    TailoringOptions,
};

const SUMMARY_SYSTEM_PROMPT: &str = r#"You are a professional resume writer. Generate a 3-sentence professional summary for a resume.

Rules:
- Use active voice and strong action verbs
- Reference specific skills and experience levels
- Tailor the summary to the target job requirements
- Do NOT use cliches like "passionate about", "proven track record", "self-starter"
- Return ONLY the 3-sentence summary text, no JSON, no quotes"#;

const BULLET_REWRITE_SYSTEM_PROMPT: &str = r#"You rewrite resume bullet points to incorporate keywords from a target job description.

Rules:
1. Only rewrite based on the real achievement described — do not invent new facts
2. Use action verbs (Led, Built, Designed, Implemented, Reduced, etc.)
3. Quantify where the original has numbers — preserve exact metrics
4. Incorporate relevant target keywords naturally
5. Return ONLY a JSON array of strings, one per bullet
6. Keep the same number of bullets as the input"#;

#[async_trait]
pub trait ContentDrafter: Send + Sync {
    async fn draft(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapReport,
        options: &TailoringOptions,
    ) -> Result<ResumeContent>;
}

pub struct LlmContentDrafter {
    completer: Arc<dyn Completer>,
}

impl LlmContentDrafter {
    pub fn new(completer: Arc<dyn Completer>) -> Self {
        Self { completer }
    }
}

#[async_trait]
impl ContentDrafter for LlmContentDrafter {
    async fn draft(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapReport,
        options: &TailoringOptions,
    ) -> Result<ResumeContent> {
        let summary = self.generate_summary(life_sheet, jd, gaps).await?;

        let experience = self
            .rewrite_experience(life_sheet, jd, gaps, options)
            .await?;

        let skills = order_skills(life_sheet, jd, gaps);

        let education = life_sheet
            .education
            .iter()
            .map(|edu| super::types::EducationEntry {
                degree: edu.degree.clone().unwrap_or_default(),
                field: edu.field.clone().unwrap_or_default(),
                institution: edu.institution.clone(),
                graduation_year: edu
                    .end_date
                    .as_ref()
                    .and_then(|d| d.split('-').next())
                    .and_then(|y| y.parse().ok()),
                gpa: edu.score.as_ref().and_then(|s| s.parse().ok()),
            })
            .collect();

        let projects = life_sheet
            .projects
            .iter()
            .map(|proj| super::types::ProjectEntry {
                name: proj.name.clone(),
                description: proj.description.clone().unwrap_or_default(),
                technologies: proj.highlights.clone(),
                url: proj.url.clone(),
            })
            .collect();

        let certifications = life_sheet
            .certifications
            .iter()
            .map(|c| c.name.clone())
            .collect();

        Ok(ResumeContent {
            summary,
            experience,
            skills,
            education,
            projects,
            certifications,
        })
    }
}

impl LlmContentDrafter {
    async fn generate_summary(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapReport,
    ) -> Result<String> {
        let top_skills: Vec<&str> = gaps
            .matched_skills
            .iter()
            .take(5)
            .map(|s| s.skill_name.as_str())
            .collect();

        let years = life_sheet.work_experience.len().max(1) * 2;
        let current_title = life_sheet
            .work_experience
            .first()
            .map(|e| e.position.as_str())
            .unwrap_or("Professional");

        let user_msg = format!(
            "Profile: {current_title} with ~{years} years of experience.\n\
             Top matching skills: {}\n\
             Target job responsibilities: {}\n\
             Generate a 3-sentence professional summary.",
            top_skills.join(", "),
            jd.responsibilities.join("; "),
        );

        let response = self
            .completer
            .complete(SUMMARY_SYSTEM_PROMPT, &user_msg)
            .await?;

        Ok(response.trim().to_string())
    }

    async fn rewrite_experience(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        _gaps: &GapReport,
        options: &TailoringOptions,
    ) -> Result<Vec<ExperienceSection>> {
        let target_keywords: Vec<&str> = jd.keywords.iter().map(|s| s.as_str()).collect();
        let mut sections = Vec::new();

        for exp in &life_sheet.work_experience {
            let original_bullets: Vec<String> = exp
                .achievements
                .iter()
                .take(options.max_bullets_per_entry)
                .map(|a| a.description.clone())
                .collect();

            if original_bullets.is_empty() {
                let date_range = format_date_range(&exp.start_date, exp.end_date.as_deref());
                sections.push(ExperienceSection {
                    company: exp.company.clone(),
                    title: exp.position.clone(),
                    date_range,
                    bullets: vec![],
                    rewritten_indices: vec![],
                });
                continue;
            }

            let user_msg = format!(
                "Original bullets:\n{}\n\nTarget keywords: {}\n\n\
                 Rewrite these bullets to naturally incorporate the target keywords \
                 while preserving the original facts and metrics.",
                serde_json::to_string(&original_bullets).unwrap_or_default(),
                target_keywords.join(", "),
            );

            let rewritten = match self
                .completer
                .complete(BULLET_REWRITE_SYSTEM_PROMPT, &user_msg)
                .await
            {
                Ok(response) => parse_bullet_response(&response, &original_bullets),
                Err(_) => original_bullets.clone(),
            };

            let rewritten_indices: Vec<usize> = (0..rewritten.len())
                .filter(|&i| i < original_bullets.len() && rewritten[i] != original_bullets[i])
                .collect();

            let date_range = format_date_range(&exp.start_date, exp.end_date.as_deref());

            sections.push(ExperienceSection {
                company: exp.company.clone(),
                title: exp.position.clone(),
                date_range,
                bullets: rewritten,
                rewritten_indices,
            });
        }

        Ok(sections)
    }
}

fn parse_bullet_response(response: &str, originals: &[String]) -> Vec<String> {
    let trimmed = response.trim();
    let json_str = if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    match serde_json::from_str::<Vec<String>>(json_str) {
        Ok(bullets) if !bullets.is_empty() => bullets,
        _ => originals.to_vec(),
    }
}

fn order_skills(
    life_sheet: &LifeSheet,
    jd: &JobDescriptionAnalysis,
    gaps: &GapReport,
) -> SkillsSection {
    let matched_names: Vec<String> = gaps
        .matched_skills
        .iter()
        .map(|s| s.skill_name.clone())
        .collect();

    let all_skills: Vec<String> = life_sheet
        .skills
        .iter()
        .flat_map(|cat| cat.skills.iter().map(|s| s.name.clone()))
        .collect();

    let jd_keywords: Vec<String> = jd.keywords.iter().map(|k| k.to_lowercase()).collect();

    let mut primary: Vec<String> = Vec::new();
    let mut secondary: Vec<String> = Vec::new();

    for skill in &all_skills {
        let is_matched = matched_names
            .iter()
            .any(|m| m.to_lowercase() == skill.to_lowercase());
        let is_keyword = jd_keywords.contains(&skill.to_lowercase());

        if is_matched || is_keyword {
            if !primary
                .iter()
                .any(|p| p.to_lowercase() == skill.to_lowercase())
            {
                primary.push(skill.clone());
            }
        } else if !secondary
            .iter()
            .any(|s| s.to_lowercase() == skill.to_lowercase())
        {
            secondary.push(skill.clone());
        }
    }

    SkillsSection { primary, secondary }
}

fn format_date_range(start: &str, end: Option<&str>) -> String {
    match end {
        Some(e) => format!("{start} - {e}"),
        None => format!("{start} - Present"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use crate::life_sheet::{Achievement, Basics, Skill, SkillCategory, WorkExperience};
    use crate::resume::types::{MatchedSkill, SkillEvidenceSource, SkillRequirement};

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

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            let mut responses = self.responses.lock().unwrap();
            responses
                .pop()
                .ok_or(CoreError::Http("no more responses".into()))
        }
    }

    fn make_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Jane Doe".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "Acme Corp".into(),
                position: "Senior Engineer".into(),
                start_date: "2021-03".into(),
                end_date: None,
                location: None,
                url: None,
                summary: None,
                is_current: true,
                achievements: vec![
                    Achievement {
                        description: "Reduced API latency by 40%".into(),
                        metric_type: None,
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
                team_size: None,
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
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        }
    }

    fn make_jd() -> JobDescriptionAnalysis {
        JobDescriptionAnalysis {
            raw_text: "test".into(),
            required_skills: vec![SkillRequirement {
                name: "Rust".into(),
                canonical: "rust".into(),
                is_required: true,
            }],
            nice_to_have_skills: vec![],
            keywords: vec!["rust".into(), "backend".into(), "scalable".into()],
            responsibilities: vec!["Build backend services".into()],
        }
    }

    fn make_gaps() -> GapReport {
        GapReport {
            matched_skills: vec![MatchedSkill {
                skill_name: "Rust".into(),
                evidence_source: SkillEvidenceSource::ExplicitSkill,
                strength: 1.0,
            }],
            missing_required: vec![],
            missing_nice_to_have: vec![],
            match_score: 100.0,
            relevant_experience_order: vec![0],
        }
    }

    #[tokio::test]
    async fn draft_generates_resume_content() {
        let summary_response = "Experienced engineer with 4 years of Rust expertise. Built scalable backend systems at Acme Corp. Strong track record of reducing latency and mentoring engineers.";
        let bullets_response = r#"["Reduced API latency by 40% through Rust-based caching layer redesign for scalable backend services", "Mentored 3 junior engineers on backend best practices and Rust development"]"#;

        let completer = Arc::new(MockCompleter::new(vec![summary_response, bullets_response]));
        let drafter = LlmContentDrafter::new(completer);

        let sheet = make_life_sheet();
        let jd = make_jd();
        let gaps = make_gaps();
        let options = TailoringOptions::default();

        let content = drafter.draft(&sheet, &jd, &gaps, &options).await.unwrap();

        assert!(!content.summary.is_empty());
        assert_eq!(content.experience.len(), 1);
        assert_eq!(content.experience[0].company, "Acme Corp");
        assert_eq!(content.experience[0].bullets.len(), 2);
        assert!(!content.experience[0].rewritten_indices.is_empty());
    }

    #[tokio::test]
    async fn draft_falls_back_on_bullet_rewrite_failure() {
        let summary_response = "A summary.";
        let completer = Arc::new(MockCompleter::new(vec![summary_response]));
        let drafter = LlmContentDrafter::new(completer);

        let sheet = make_life_sheet();
        let jd = make_jd();
        let gaps = make_gaps();
        let options = TailoringOptions::default();

        let content = drafter.draft(&sheet, &jd, &gaps, &options).await.unwrap();
        assert_eq!(
            content.experience[0].bullets[0],
            "Reduced API latency by 40%"
        );
        assert!(content.experience[0].rewritten_indices.is_empty());
    }

    #[test]
    fn parse_bullet_response_valid_json() {
        let response = r#"["Bullet one", "Bullet two"]"#;
        let originals = vec!["Original one".into(), "Original two".into()];
        let result = parse_bullet_response(response, &originals);
        assert_eq!(result, vec!["Bullet one", "Bullet two"]);
    }

    #[test]
    fn parse_bullet_response_with_preamble() {
        let response = "Here are the rewritten bullets:\n[\"Bullet one\", \"Bullet two\"]";
        let originals = vec!["Original one".into(), "Original two".into()];
        let result = parse_bullet_response(response, &originals);
        assert_eq!(result, vec!["Bullet one", "Bullet two"]);
    }

    #[test]
    fn parse_bullet_response_invalid_falls_back() {
        let originals = vec!["Original".into()];
        let result = parse_bullet_response("not json", &originals);
        assert_eq!(result, originals);
    }

    #[test]
    fn order_skills_prioritizes_matched() {
        let sheet = make_life_sheet();
        let jd = make_jd();
        let gaps = make_gaps();
        let skills = order_skills(&sheet, &jd, &gaps);
        assert!(skills.primary.contains(&"Rust".to_string()));
        assert!(skills.secondary.contains(&"Python".to_string()));
    }

    #[test]
    fn format_date_range_with_end() {
        assert_eq!(
            format_date_range("2020-01", Some("2023-06")),
            "2020-01 - 2023-06"
        );
    }

    #[test]
    fn format_date_range_present() {
        assert_eq!(format_date_range("2020-01", None), "2020-01 - Present");
    }
}
