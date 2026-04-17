use super::error::{Result, TemplateError};
use super::types::TemplateVars;
use crate::prompts::sanitizer::sanitize_user_value;

pub struct ResumeTailorContext {
    pub job_description: String,
    pub user_experience: String,
    pub requirements_analysis: String,
}

impl ResumeTailorContext {
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert(
            "job_description".into(),
            sanitize_user_value(&self.job_description),
        );
        vars.insert(
            "user_experience".into(),
            sanitize_user_value(&self.user_experience),
        );
        vars.insert(
            "requirements_analysis".into(),
            sanitize_user_value(&self.requirements_analysis),
        );
        vars
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ResumeTailorOutput {
    pub summary: String,
    pub experience: Vec<ExperienceEntry>,
    pub skills: Vec<SkillSection>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ExperienceEntry {
    pub company: String,
    pub position: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SkillSection {
    pub category: String,
    pub items: Vec<String>,
}

pub fn system_prompt() -> &'static str {
    "You are a professional resume writer. Given a job description, the user's work experience, and an analysis of requirements, produce a tailored resume optimized for this specific role."
}

pub fn user_prompt(context: &ResumeTailorContext) -> String {
    let vars = context.to_template_vars();
    format!(
        "Job description:\n{}\n\nUser's experience:\n{}\n\nRequirements analysis:\n{}\n\nProduce a tailored resume as structured JSON.",
        vars["job_description"], vars["user_experience"], vars["requirements_analysis"]
    )
}

pub fn validate_output(raw: &str) -> Result<ResumeTailorOutput> {
    serde_json::from_str(raw).map_err(|e| TemplateError::ValidationFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_to_template_vars() {
        let ctx = ResumeTailorContext {
            job_description: "We need a Rust engineer".into(),
            user_experience: "8 years in backend".into(),
            requirements_analysis: "Strong Rust match".into(),
        };
        let vars = ctx.to_template_vars();
        assert_eq!(vars.len(), 3);
        assert!(vars.contains_key("job_description"));
    }

    #[test]
    fn user_prompt_includes_all_fields() {
        let ctx = ResumeTailorContext {
            job_description: "Build distributed systems".into(),
            user_experience: "Led team of 5".into(),
            requirements_analysis: "90% skill match".into(),
        };
        let prompt = user_prompt(&ctx);
        assert!(prompt.contains("Build distributed systems"));
        assert!(prompt.contains("Led team of 5"));
        assert!(prompt.contains("90% skill match"));
    }

    #[test]
    fn validate_valid_output() {
        let json = r#"{
            "summary": "Experienced backend engineer with 8 years of Rust expertise.",
            "experience": [
                {
                    "company": "Acme Corp",
                    "position": "Senior Engineer",
                    "start_date": "2020-01",
                    "end_date": "2024-01",
                    "bullets": [
                        "Led migration from Python to Rust, reducing latency by 40%",
                        "Built distributed event processing pipeline handling 1M events/day"
                    ]
                }
            ],
            "skills": [
                {
                    "category": "Languages",
                    "items": ["Rust", "Python", "Go"]
                }
            ]
        }"#;
        let output = validate_output(json).unwrap();
        assert!(!output.summary.is_empty());
        assert_eq!(output.experience.len(), 1);
        assert_eq!(output.experience[0].bullets.len(), 2);
        assert_eq!(output.skills[0].items.len(), 3);
    }

    #[test]
    fn validate_invalid_output() {
        let err = validate_output(r#"{ "summary": "test" }"#).unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }
}
