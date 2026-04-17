use super::error::{Result, TemplateError};
use super::types::TemplateVars;
use crate::anti_fabrication::{
    GroundingReport, ProhibitedPhrase, check_grounding, prohibited_phrase_detector,
};
use crate::prompts::sanitizer::sanitize_user_value;
use lazyjob_core::life_sheet::LifeSheet;

pub struct CoverLetterContext {
    pub user_name: String,
    pub company_name: String,
    pub job_title: String,
    pub company_research: String,
    pub relevant_experience: String,
    pub job_description_summary: String,
}

impl CoverLetterContext {
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert("user_name".into(), sanitize_user_value(&self.user_name));
        vars.insert(
            "company_name".into(),
            sanitize_user_value(&self.company_name),
        );
        vars.insert("job_title".into(), sanitize_user_value(&self.job_title));
        vars.insert(
            "company_research".into(),
            sanitize_user_value(&self.company_research),
        );
        vars.insert(
            "relevant_experience".into(),
            sanitize_user_value(&self.relevant_experience),
        );
        vars.insert(
            "job_description_summary".into(),
            sanitize_user_value(&self.job_description_summary),
        );
        vars
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CoverLetterOutput {
    pub paragraphs: Vec<String>,
    pub template_type: String,
    #[serde(default)]
    pub subject_line: Option<String>,
    #[serde(default)]
    pub key_themes: Vec<String>,
}

pub fn system_prompt() -> &'static str {
    "You are a professional cover letter writer. Write compelling cover letters that connect the user's real experience to the job requirements."
}

pub fn user_prompt(context: &CoverLetterContext) -> String {
    let vars = context.to_template_vars();
    format!(
        "Applicant: {}\nCompany: {}\nPosition: {}\n\nCompany research:\n{}\n\nRelevant experience:\n{}\n\nJob description summary:\n{}\n\nWrite a cover letter as structured JSON with separate paragraphs.",
        vars["user_name"],
        vars["company_name"],
        vars["job_title"],
        vars["company_research"],
        vars["relevant_experience"],
        vars["job_description_summary"]
    )
}

pub fn validate_output(raw: &str) -> Result<CoverLetterOutput> {
    let output: CoverLetterOutput =
        serde_json::from_str(raw).map_err(|e| TemplateError::ValidationFailed(e.to_string()))?;
    if output.paragraphs.is_empty() {
        return Err(TemplateError::ValidationFailed(
            "cover letter must have at least one paragraph".into(),
        ));
    }
    Ok(output)
}

pub fn validate_grounding(
    output: &CoverLetterOutput,
    life_sheet: &LifeSheet,
) -> (GroundingReport, Vec<ProhibitedPhrase>) {
    let grounding = check_grounding(&output.paragraphs, life_sheet);

    let full_text = output.paragraphs.join(" ");
    let prohibited = prohibited_phrase_detector(&full_text);

    (grounding, prohibited)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> CoverLetterContext {
        CoverLetterContext {
            user_name: "Alice Smith".into(),
            company_name: "Anthropic".into(),
            job_title: "Staff Engineer".into(),
            company_research: "AI safety company, founded 2021".into(),
            relevant_experience: "8 years Rust, distributed systems".into(),
            job_description_summary: "Build safe AI systems".into(),
        }
    }

    #[test]
    fn context_to_template_vars() {
        let ctx = make_context();
        let vars = ctx.to_template_vars();
        assert_eq!(vars.len(), 6);
        assert_eq!(vars["user_name"], "Alice Smith");
        assert_eq!(vars["company_name"], "Anthropic");
    }

    #[test]
    fn user_prompt_contains_all_fields() {
        let ctx = make_context();
        let prompt = user_prompt(&ctx);
        assert!(prompt.contains("Alice Smith"));
        assert!(prompt.contains("Anthropic"));
        assert!(prompt.contains("Staff Engineer"));
    }

    #[test]
    fn validate_valid_output() {
        let json = r#"{
            "paragraphs": [
                "Dear Hiring Manager, I am writing to express my interest in the Staff Engineer position.",
                "With 8 years of experience in Rust and distributed systems, I bring deep expertise.",
                "I look forward to discussing how my background aligns with Anthropic's mission."
            ],
            "template_type": "standard_professional",
            "key_themes": ["AI safety", "systems engineering"]
        }"#;
        let output = validate_output(json).unwrap();
        assert_eq!(output.paragraphs.len(), 3);
        assert_eq!(output.template_type, "standard_professional");
    }

    #[test]
    fn validate_empty_paragraphs_rejected() {
        let json = r#"{ "paragraphs": [], "template_type": "standard_professional" }"#;
        let err = validate_output(json).unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }

    #[test]
    fn validate_invalid_json() {
        let err = validate_output("not json at all").unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }

    #[test]
    fn validate_grounding_detects_cliches() {
        use lazyjob_core::life_sheet::Basics;

        let sheet = LifeSheet {
            basics: Basics {
                name: "Test".into(),
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

        let output = CoverLetterOutput {
            paragraphs: vec![
                "I am passionate about this role and am a self-starter.".into(),
                "I have a proven track record and am highly motivated.".into(),
            ],
            template_type: "standard_professional".into(),
            subject_line: None,
            key_themes: vec![],
        };

        let (_report, prohibited) = validate_grounding(&output, &sheet);
        assert!(!prohibited.is_empty());
        let phrases: Vec<&str> = prohibited.iter().map(|p| p.phrase.as_str()).collect();
        assert!(phrases.contains(&"passionate about"));
        assert!(phrases.contains(&"self-starter"));
    }

    #[test]
    fn validate_grounding_with_real_experience() {
        use crate::anti_fabrication::FabricationLevel;
        use lazyjob_core::life_sheet::{Basics, Skill, SkillCategory, WorkExperience};

        let sheet = LifeSheet {
            basics: Basics {
                name: "Alice".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "Stripe".into(),
                position: "Backend Engineer".into(),
                start_date: "2021-01".into(),
                end_date: None,
                location: None,
                url: None,
                summary: None,
                is_current: true,
                achievements: vec![],
                tech_stack: vec!["Ruby".into(), "Go".into()],
                team_size: None,
                industry: None,
            }],
            education: vec![],
            skills: vec![SkillCategory {
                name: "Languages".into(),
                level: None,
                skills: vec![
                    Skill {
                        name: "Ruby".into(),
                        years_experience: Some(5),
                        proficiency: None,
                    },
                    Skill {
                        name: "Go".into(),
                        years_experience: Some(3),
                        proficiency: None,
                    },
                ],
            }],
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        };

        let output = CoverLetterOutput {
            paragraphs: vec![
                "As a Backend Engineer at Stripe, I built payment processing systems using Ruby and Go.".into(),
            ],
            template_type: "standard_professional".into(),
            subject_line: None,
            key_themes: vec![],
        };

        let (report, prohibited) = validate_grounding(&output, &sheet);
        assert_eq!(report.level, FabricationLevel::Grounded);
        assert!(prohibited.is_empty());
    }
}
