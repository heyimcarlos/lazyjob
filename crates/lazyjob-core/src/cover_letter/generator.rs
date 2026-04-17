use std::collections::HashSet;
use std::sync::Arc;

use crate::discovery::Completer;
use crate::domain::Job;
use crate::error::Result;
use crate::life_sheet::LifeSheet;

use super::types::{CoverLetterLength, CoverLetterTemplate, CoverLetterTone};

pub struct CoverLetterGenerator {
    completer: Arc<dyn Completer>,
}

impl CoverLetterGenerator {
    pub fn new(completer: Arc<dyn Completer>) -> Self {
        Self { completer }
    }

    pub async fn generate(
        &self,
        job: &Job,
        life_sheet: &LifeSheet,
        template: CoverLetterTemplate,
        tone: CoverLetterTone,
        length: CoverLetterLength,
        custom_intro: Option<&str>,
    ) -> Result<String> {
        let system = self.build_system_prompt(template, tone);
        let user = self.build_user_prompt(job, life_sheet, template, tone, length, custom_intro);
        self.completer.complete(&system, &user).await
    }

    fn build_system_prompt(&self, template: CoverLetterTemplate, tone: CoverLetterTone) -> String {
        let structure = match template {
            CoverLetterTemplate::StandardProfessional => {
                "Structure: hook opening paragraph, company-specific paragraph connecting your background to their mission, \
                 1-2 achievement paragraphs with concrete metrics, closing call to action."
            }
            CoverLetterTemplate::ProblemSolution => {
                "Structure: open with a specific challenge the company faces (inferred from the job description), \
                 explain how you have solved a similar problem with measurable results, \
                 bridge to how you would apply that experience here, closing call to action."
            }
            CoverLetterTemplate::CareerChanger => {
                "Structure: briefly acknowledge the career pivot with confidence (not apology), \
                 highlight 2-3 transferable skills with evidence from your previous career, \
                 express genuine enthusiasm for the new direction with specific reasons, closing call to action."
            }
        };

        format!(
            "You are an expert cover letter writer. Write cover letters that are {tone_desc} in tone.\n\n\
             {structure}\n\n\
             Rules:\n\
             - Use concrete metrics from the candidate's background wherever available\n\
             - Do NOT use cliches: \"I am writing to express my interest\", \"passionate about\", \
               \"synergy\", \"leverage\", \"team player\", \"detail-oriented\", \"proven track record\"\n\
             - Do NOT include header/address block — body only\n\
             - Do NOT fabricate achievements, certifications, or metrics not in the candidate's background\n\
             - Return ONLY the letter body text, no formatting labels or markdown headers",
            tone_desc = tone.prompt_description(),
            structure = structure,
        )
    }

    fn build_user_prompt(
        &self,
        job: &Job,
        life_sheet: &LifeSheet,
        _template: CoverLetterTemplate,
        _tone: CoverLetterTone,
        length: CoverLetterLength,
        custom_intro: Option<&str>,
    ) -> String {
        let company = job.company_name.as_deref().unwrap_or("the company");
        let title = &job.title;
        let jd = job
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(1500)
            .collect::<String>();
        let experience = format_relevant_experience(life_sheet, job);
        let skills = format_skills(life_sheet);

        let custom = custom_intro
            .map(|s| format!("\nUse this as the opening sentence: \"{s}\""))
            .unwrap_or_default();

        format!(
            "Write a cover letter for this job application.\n\n\
             ## Role\n\
             Company: {company}\n\
             Title: {title}\n\
             Job description:\n{jd}\n\n\
             ## Candidate Background\n\
             Name: {name}\n\
             {experience}\n\n\
             ## Key Skills\n\
             {skills}\n\n\
             ## Instructions\n\
             - Target length: ~{word_target} words\n\
             - Return ONLY the letter body text\
             {custom}",
            company = company,
            title = title,
            jd = jd,
            name = life_sheet.basics.name,
            experience = experience,
            skills = skills,
            word_target = length.word_target(),
            custom = custom,
        )
    }

    pub fn extract_key_points(content: &str) -> Vec<String> {
        content
            .split("\n\n")
            .take(4)
            .filter_map(|p| {
                let trimmed = p.trim();
                if trimmed.len() > 20 {
                    Some(trimmed.lines().next().unwrap_or(trimmed).trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn to_plain_text(content: &str) -> String {
        content
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                l.trim_start_matches('*')
                    .trim_start_matches('_')
                    .trim_start_matches('`')
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_relevant_experience(life_sheet: &LifeSheet, job: &Job) -> String {
    let jd_lower = job.description.as_deref().unwrap_or("").to_lowercase();
    let jd_words: HashSet<&str> = jd_lower.split_whitespace().collect();

    let mut scored: Vec<_> = life_sheet
        .work_experience
        .iter()
        .map(|exp| {
            let text = format!(
                "{} {} {}",
                exp.position,
                exp.company,
                exp.achievements
                    .iter()
                    .map(|a| a.description.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .to_lowercase();
            let score = text
                .split_whitespace()
                .filter(|w| jd_words.contains(*w))
                .count();
            (exp, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    scored
        .iter()
        .take(3)
        .map(|(exp, _)| {
            let top_achievement = exp
                .achievements
                .first()
                .map(|a| a.description.as_str())
                .unwrap_or("");
            let end = exp.end_date.as_deref().unwrap_or("present");
            format!(
                "- {} at {} ({}–{}): {}",
                exp.position, exp.company, exp.start_date, end, top_achievement,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_skills(life_sheet: &LifeSheet) -> String {
    life_sheet
        .skills
        .iter()
        .map(|cat| {
            let names: Vec<&str> = cat.skills.iter().map(|s| s.name.as_str()).collect();
            format!("{}: {}", cat.name, names.join(", "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::{Achievement, Basics, Skill, SkillCategory, WorkExperience};

    fn mock_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Alice Smith".into(),
                label: None,
                email: Some("alice@example.com".into()),
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "TechCorp".into(),
                position: "Senior Engineer".into(),
                location: None,
                url: None,
                start_date: "2020".into(),
                end_date: None,
                is_current: true,
                summary: None,
                achievements: vec![Achievement {
                    description: "Reduced API latency by 40% serving 10M requests/day".into(),
                    metric_type: None,
                    metric_value: None,
                    metric_unit: None,
                }],
                team_size: None,
                industry: None,
                tech_stack: vec!["Rust".into(), "PostgreSQL".into()],
            }],
            education: vec![],
            skills: vec![SkillCategory {
                name: "Languages".into(),
                level: None,
                skills: vec![
                    Skill {
                        name: "Rust".into(),
                        years_experience: Some(4),
                        proficiency: None,
                    },
                    Skill {
                        name: "Python".into(),
                        years_experience: Some(6),
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

    fn mock_job() -> Job {
        let mut job = Job::new("Backend Engineer");
        job.company_name = Some("Acme Inc".into());
        job.description = Some("We need a Rust backend engineer with API experience".into());
        job
    }

    #[test]
    fn extract_key_points_returns_first_lines() {
        let content = "First paragraph with enough text to pass the filter.\n\n\
                        Second paragraph also long enough to be included here.\n\n\
                        Third paragraph is here too with sufficient length.";
        let points = CoverLetterGenerator::extract_key_points(content);
        assert_eq!(points.len(), 3);
        assert!(points[0].contains("First paragraph"));
        assert!(points[1].contains("Second paragraph"));
    }

    #[test]
    fn extract_key_points_skips_short_paragraphs() {
        let content = "Ok.\n\nThis is a sufficiently long paragraph for extraction.";
        let points = CoverLetterGenerator::extract_key_points(content);
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn to_plain_text_strips_markdown() {
        let md = "# Header\n*Bold text*\n_Italic text_\n`Code text`\nPlain text";
        let plain = CoverLetterGenerator::to_plain_text(md);
        assert!(!plain.contains("# Header"));
        assert!(plain.contains("Bold text"));
        assert!(plain.contains("Plain text"));
    }

    #[test]
    fn format_relevant_experience_ranks_by_relevance() {
        let sheet = mock_life_sheet();
        let job = mock_job();
        let result = format_relevant_experience(&sheet, &job);
        assert!(result.contains("Senior Engineer at TechCorp"));
        assert!(result.contains("Reduced API latency"));
    }

    #[test]
    fn format_skills_lists_categories() {
        let sheet = mock_life_sheet();
        let result = format_skills(&sheet);
        assert!(result.contains("Languages: Rust, Python"));
    }

    #[test]
    fn build_system_prompt_standard_professional() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let prompt = generator.build_system_prompt(
            CoverLetterTemplate::StandardProfessional,
            CoverLetterTone::Professional,
        );
        assert!(prompt.contains("hook opening"));
        assert!(prompt.contains("professional and confident"));
    }

    #[test]
    fn build_system_prompt_problem_solution() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let prompt = generator.build_system_prompt(
            CoverLetterTemplate::ProblemSolution,
            CoverLetterTone::Casual,
        );
        assert!(prompt.contains("challenge"));
        assert!(prompt.contains("warm, conversational"));
    }

    #[test]
    fn build_system_prompt_career_changer() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let prompt = generator.build_system_prompt(
            CoverLetterTemplate::CareerChanger,
            CoverLetterTone::Creative,
        );
        assert!(prompt.contains("career pivot"));
        assert!(prompt.contains("creative, memorable"));
    }

    #[test]
    fn build_user_prompt_includes_job_and_candidate() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let job = mock_job();
        let sheet = mock_life_sheet();
        let prompt = generator.build_user_prompt(
            &job,
            &sheet,
            CoverLetterTemplate::StandardProfessional,
            CoverLetterTone::Professional,
            CoverLetterLength::Standard,
            None,
        );
        assert!(prompt.contains("Acme Inc"));
        assert!(prompt.contains("Backend Engineer"));
        assert!(prompt.contains("Alice Smith"));
        assert!(prompt.contains("~300 words"));
    }

    #[test]
    fn build_user_prompt_with_custom_intro() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let job = mock_job();
        let sheet = mock_life_sheet();
        let prompt = generator.build_user_prompt(
            &job,
            &sheet,
            CoverLetterTemplate::StandardProfessional,
            CoverLetterTone::Professional,
            CoverLetterLength::Standard,
            Some("I recently saw your talk at RustConf"),
        );
        assert!(prompt.contains("I recently saw your talk at RustConf"));
    }

    struct MockCompleter;

    #[async_trait::async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> crate::error::Result<String> {
            Ok("Dear Hiring Manager, I am excited to apply for this role. \
                My experience at TechCorp reduced API latency by 40%. \
                I would love to discuss how I can contribute to your team."
                .into())
        }
    }

    #[tokio::test]
    async fn generate_calls_completer_and_returns_content() {
        let generator = CoverLetterGenerator::new(Arc::new(MockCompleter));
        let job = mock_job();
        let sheet = mock_life_sheet();
        let result = generator
            .generate(
                &job,
                &sheet,
                CoverLetterTemplate::StandardProfessional,
                CoverLetterTone::Professional,
                CoverLetterLength::Standard,
                None,
            )
            .await
            .unwrap();
        assert!(result.contains("Dear Hiring Manager"));
        assert!(result.contains("40%"));
    }
}
