use super::error::{Result, TemplateError};
use super::types::TemplateVars;
use crate::prompts::sanitizer::sanitize_user_value;

pub struct CompanyResearchContext {
    pub company_name: String,
    pub target_role: String,
}

impl CompanyResearchContext {
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert(
            "company_name".into(),
            sanitize_user_value(&self.company_name),
        );
        vars.insert("target_role".into(), sanitize_user_value(&self.target_role));
        vars
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CompanyResearchOutput {
    pub company_name: String,
    pub industry: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub tech_stack: Vec<String>,
    #[serde(default)]
    pub culture_keywords: Vec<String>,
    #[serde(default)]
    pub recent_news: Vec<String>,
    #[serde(default)]
    pub hiring_signals: Option<String>,
}

pub fn system_prompt() -> &'static str {
    "You are a company research analyst. Given a company name and a target role, extract structured information about the company that would be useful for a job applicant."
}

pub fn user_prompt(context: &CompanyResearchContext) -> String {
    let vars = context.to_template_vars();
    format!(
        "Company: {}\nTarget role: {}\n\nResearch this company and return structured findings as JSON.",
        vars["company_name"], vars["target_role"]
    )
}

pub fn validate_output(raw: &str) -> Result<CompanyResearchOutput> {
    serde_json::from_str(raw).map_err(|e| TemplateError::ValidationFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_to_template_vars() {
        let ctx = CompanyResearchContext {
            company_name: "Anthropic".into(),
            target_role: "Staff Engineer".into(),
        };
        let vars = ctx.to_template_vars();
        assert_eq!(vars["company_name"], "Anthropic");
        assert_eq!(vars["target_role"], "Staff Engineer");
    }

    #[test]
    fn user_prompt_contains_context() {
        let ctx = CompanyResearchContext {
            company_name: "Stripe".into(),
            target_role: "Backend Engineer".into(),
        };
        let prompt = user_prompt(&ctx);
        assert!(prompt.contains("Stripe"));
        assert!(prompt.contains("Backend Engineer"));
    }

    #[test]
    fn validate_valid_output() {
        let json = r#"{
            "company_name": "Stripe",
            "industry": "Fintech",
            "size": "5000+",
            "tech_stack": ["Ruby", "Go", "Rust"],
            "culture_keywords": ["engineering excellence", "user-focused"],
            "recent_news": ["Launched new payment API"]
        }"#;
        let output = validate_output(json).unwrap();
        assert_eq!(output.company_name, "Stripe");
        assert_eq!(output.tech_stack.len(), 3);
    }

    #[test]
    fn validate_minimal_output() {
        let json = r#"{ "company_name": "Stripe", "industry": "Fintech" }"#;
        let output = validate_output(json).unwrap();
        assert_eq!(output.company_name, "Stripe");
        assert!(output.tech_stack.is_empty());
    }

    #[test]
    fn validate_invalid_output() {
        let err = validate_output("{}").unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }
}
