use super::error::{Result, TemplateError};
use super::types::TemplateVars;
use crate::anti_fabrication::{
    GroundingReport, ProhibitedPhrase, check_grounding, prohibited_phrase_detector,
};
use crate::prompts::sanitizer::sanitize_user_value;
use lazyjob_core::life_sheet::LifeSheet;

pub struct OutreachContext {
    pub contact_name: String,
    pub contact_role: String,
    pub contact_company: String,
    pub job_title: String,
    pub company_name: String,
    pub tone: String,
    pub user_background: String,
    pub user_notes: String,
}

impl OutreachContext {
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert(
            "contact_name".into(),
            sanitize_user_value(&self.contact_name),
        );
        vars.insert(
            "contact_role".into(),
            sanitize_user_value(&self.contact_role),
        );
        vars.insert(
            "contact_company".into(),
            sanitize_user_value(&self.contact_company),
        );
        vars.insert("job_title".into(), sanitize_user_value(&self.job_title));
        vars.insert(
            "company_name".into(),
            sanitize_user_value(&self.company_name),
        );
        vars.insert("tone".into(), sanitize_user_value(&self.tone));
        vars.insert(
            "user_background".into(),
            sanitize_user_value(&self.user_background),
        );
        vars.insert("user_notes".into(), sanitize_user_value(&self.user_notes));
        vars
    }
}

pub fn system_prompt() -> &'static str {
    "You are a professional networking assistant. Generate a single, personalized outreach message.\n\n\
     RULES:\n\
     1. Every factual claim must be grounded in the user's actual background.\n\
     2. Never invent shared history, mutual connections, or experiences not mentioned.\n\
     3. Never reference salary, personal details, or internal company information.\n\
     4. For casual tone: do NOT include a job ask. Focus on relationship re-warming.\n\
     5. For referral-ask tone: make the referral ask specific to the role.\n\
     6. Keep the message concise (3-5 sentences for short messages, 100-200 words for email).\n\
     7. Do NOT use cliché phrases like 'passionate about', 'synergy', 'leverage my', etc.\n\n\
     OUTPUT: Return only the message body text. No meta-commentary. No subject line unless explicitly asked."
}

pub fn user_prompt(context: &OutreachContext) -> String {
    let vars = context.to_template_vars();
    format!(
        "Draft a {tone} outreach message to {contact_name} ({contact_role} at {contact_company}).\n\n\
         Target role: {job_title} at {company_name}\n\n\
         My background:\n{user_background}\n\n\
         Additional notes: {user_notes}\n\n\
         Write a concise, personalized message.",
        tone = vars["tone"],
        contact_name = vars["contact_name"],
        contact_role = vars["contact_role"],
        contact_company = vars["contact_company"],
        job_title = vars["job_title"],
        company_name = vars["company_name"],
        user_background = vars["user_background"],
        user_notes = vars["user_notes"],
    )
}

pub fn validate_output(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(TemplateError::ValidationFailed(
            "outreach draft body cannot be empty".into(),
        ));
    }
    if trimmed.split_whitespace().count() < 5 {
        return Err(TemplateError::ValidationFailed(
            "outreach draft must be at least 5 words".into(),
        ));
    }
    Ok(trimmed)
}

pub fn validate_grounding(
    body: &str,
    life_sheet: &LifeSheet,
) -> (GroundingReport, Vec<ProhibitedPhrase>) {
    let claims = body
        .split('.')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let grounding = check_grounding(&claims, life_sheet);
    let prohibited = prohibited_phrase_detector(body);
    (grounding, prohibited)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> OutreachContext {
        OutreachContext {
            contact_name: "Alice Smith".into(),
            contact_role: "Staff Engineer".into(),
            contact_company: "Acme Corp".into(),
            job_title: "Senior Engineer".into(),
            company_name: "Acme Corp".into(),
            tone: "professional".into(),
            user_background: "5 years Rust, distributed systems at TechCorp".into(),
            user_notes: "".into(),
        }
    }

    #[test]
    fn context_to_template_vars() {
        let ctx = make_context();
        let vars = ctx.to_template_vars();
        assert_eq!(vars.len(), 8);
        assert_eq!(vars["contact_name"], "Alice Smith");
        assert_eq!(vars["contact_company"], "Acme Corp");
    }

    #[test]
    fn user_prompt_contains_all_fields() {
        let ctx = make_context();
        let prompt = user_prompt(&ctx);
        assert!(prompt.contains("Alice Smith"));
        assert!(prompt.contains("Staff Engineer"));
        assert!(prompt.contains("Acme Corp"));
        assert!(prompt.contains("professional"));
    }

    #[test]
    fn validate_valid_output() {
        let body = "Hi Alice, I noticed we both work in distributed systems. I would love to connect and learn more about your work at Acme Corp.";
        let result = validate_output(body);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_empty_body_rejected() {
        let err = validate_output("").unwrap_err();
        matches!(err, TemplateError::ValidationFailed(_));
    }

    #[test]
    fn validate_too_short_rejected() {
        let err = validate_output("Hi there hello").unwrap_err();
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

        let body = "I am passionate about this opportunity and have a proven track record.";
        let (_report, prohibited) = validate_grounding(body, &sheet);
        assert!(!prohibited.is_empty());
        let phrases: Vec<&str> = prohibited.iter().map(|p| p.phrase.as_str()).collect();
        assert!(phrases.contains(&"passionate about"));
    }
}
