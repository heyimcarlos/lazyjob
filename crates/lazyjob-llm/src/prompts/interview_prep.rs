use super::error::{Result, TemplateError};
use super::types::TemplateVars;
use crate::prompts::sanitizer::sanitize_user_value;

pub struct InterviewPrepContext {
    pub interview_type: String,
    pub company_name: String,
    pub job_title: String,
    pub job_description: String,
    pub company_research: String,
    pub user_background: String,
}

impl InterviewPrepContext {
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert(
            "interview_type".into(),
            sanitize_user_value(&self.interview_type),
        );
        vars.insert(
            "company_name".into(),
            sanitize_user_value(&self.company_name),
        );
        vars.insert("job_title".into(), sanitize_user_value(&self.job_title));
        vars.insert(
            "job_description".into(),
            sanitize_user_value(&self.job_description),
        );
        vars.insert(
            "company_research".into(),
            sanitize_user_value(&self.company_research),
        );
        vars.insert(
            "user_background".into(),
            sanitize_user_value(&self.user_background),
        );
        vars
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InterviewPrepOutput {
    pub questions: Vec<InterviewQuestion>,
    #[serde(default)]
    pub preparation_tips: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InterviewQuestion {
    pub question_text: String,
    pub question_type: String,
    pub answer_framework: String,
    #[serde(default)]
    pub evidence_from_life_sheet: Option<String>,
    #[serde(default)]
    pub difficulty: Option<String>,
}

pub fn system_prompt() -> &'static str {
    "You are an interview preparation coach. Generate realistic interview questions with STAR framework guidance grounded in the user's actual experience."
}

pub fn user_prompt(context: &InterviewPrepContext) -> String {
    let vars = context.to_template_vars();
    format!(
        "Interview type: {}\nCompany: {}\nPosition: {}\n\nJob description:\n{}\n\nCompany research:\n{}\n\nUser's background:\n{}\n\nGenerate interview preparation questions and answer frameworks as structured JSON.",
        vars["interview_type"],
        vars["company_name"],
        vars["job_title"],
        vars["job_description"],
        vars["company_research"],
        vars["user_background"]
    )
}

pub fn validate_output(raw: &str) -> Result<InterviewPrepOutput> {
    let output: InterviewPrepOutput =
        serde_json::from_str(raw).map_err(|e| TemplateError::ValidationFailed(e.to_string()))?;
    if output.questions.is_empty() {
        return Err(TemplateError::ValidationFailed(
            "interview prep must have at least one question".into(),
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> InterviewPrepContext {
        InterviewPrepContext {
            interview_type: "technical".into(),
            company_name: "Anthropic".into(),
            job_title: "Staff Engineer".into(),
            job_description: "Design and build AI safety infrastructure".into(),
            company_research: "AI safety lab, 500+ employees".into(),
            user_background: "8 years Rust, distributed systems, ML infra".into(),
        }
    }

    #[test]
    fn context_to_template_vars() {
        let ctx = make_context();
        let vars = ctx.to_template_vars();
        assert_eq!(vars.len(), 6);
        assert_eq!(vars["interview_type"], "technical");
    }

    #[test]
    fn user_prompt_contains_all_fields() {
        let ctx = make_context();
        let prompt = user_prompt(&ctx);
        assert!(prompt.contains("technical"));
        assert!(prompt.contains("Anthropic"));
        assert!(prompt.contains("Staff Engineer"));
        assert!(prompt.contains("AI safety infrastructure"));
    }

    #[test]
    fn validate_valid_output() {
        let json = r#"{
            "questions": [
                {
                    "question_text": "Tell me about a time you designed a distributed system.",
                    "question_type": "behavioral",
                    "answer_framework": "Use STAR: Situation - at previous company, Task - needed to handle 10x traffic growth, Action - designed event-driven architecture, Result - system scaled to 1M events/day",
                    "evidence_from_life_sheet": "Built distributed event processing pipeline",
                    "difficulty": "medium"
                },
                {
                    "question_text": "How would you design a rate limiter?",
                    "question_type": "technical",
                    "answer_framework": "Discuss token bucket vs sliding window algorithms, trade-offs of distributed rate limiting",
                    "difficulty": "hard"
                }
            ],
            "preparation_tips": [
                "Review system design fundamentals",
                "Practice STAR method responses"
            ]
        }"#;
        let output = validate_output(json).unwrap();
        assert_eq!(output.questions.len(), 2);
        assert_eq!(output.questions[0].question_type, "behavioral");
        assert_eq!(output.preparation_tips.len(), 2);
    }

    #[test]
    fn validate_empty_questions_rejected() {
        let json = r#"{ "questions": [] }"#;
        let err = validate_output(json).unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }

    #[test]
    fn validate_invalid_json() {
        let err = validate_output("{invalid}").unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }
}
