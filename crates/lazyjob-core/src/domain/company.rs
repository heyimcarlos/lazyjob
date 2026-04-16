use serde::{Deserialize, Serialize};

use super::ids::CompanyId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Company {
    pub id: CompanyId,
    pub name: String,
    pub website: Option<String>,
    pub industry: Option<String>,
    pub size: Option<String>,
    pub tech_stack: Vec<String>,
    pub culture_keywords: Vec<String>,
    pub notes: Option<String>,
}

impl Company {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: CompanyId::new(),
            name: name.into(),
            website: None,
            industry: None,
            size: None,
            tech_stack: Vec::new(),
            culture_keywords: Vec::new(),
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_construction() {
        let company = Company::new("Acme Corp");
        assert_eq!(company.name, "Acme Corp");
        assert!(company.tech_stack.is_empty());
    }

    #[test]
    fn company_serde_round_trip() {
        let mut company = Company::new("TechStartup");
        company.website = Some("https://techstartup.io".into());
        company.tech_stack = vec!["Rust".into(), "PostgreSQL".into()];
        company.industry = Some("SaaS".into());

        let json = serde_json::to_string(&company).unwrap();
        let deserialized: Company = serde_json::from_str(&json).unwrap();
        assert_eq!(company, deserialized);
    }
}
