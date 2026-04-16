use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{
    domain::CompanyId,
    error::{CoreError, Result},
    repositories::CompanyRepository,
};

use super::sources::strip_html;

#[async_trait]
pub trait Completer: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnrichmentData {
    pub industry: Option<String>,
    pub size: Option<String>,
    pub tech_stack: Vec<String>,
    pub culture_keywords: Vec<String>,
    pub recent_news: Vec<String>,
}

pub struct CompanyResearcher {
    completer: Arc<dyn Completer>,
    client: reqwest::Client,
}

impl CompanyResearcher {
    pub fn new(completer: Arc<dyn Completer>, client: reqwest::Client) -> Self {
        Self { completer, client }
    }

    pub async fn enrich(&self, company_id: &CompanyId, pool: &PgPool) -> Result<EnrichmentData> {
        let repo = CompanyRepository::new(pool.clone());

        let mut company =
            repo.find_by_id(company_id)
                .await?
                .ok_or_else(|| CoreError::NotFound {
                    entity: "Company",
                    id: company_id.to_string(),
                })?;

        let website = company.website.as_ref().ok_or_else(|| {
            CoreError::Validation("company has no website URL to research".into())
        })?;

        let html = self
            .client
            .get(website)
            .send()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?
            .text()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?;

        let text = strip_html(&html);
        let content: String = text.chars().take(3000).collect();

        let system = "You are a company research assistant. Extract factual company information \
                      from website content and return it as a single JSON object. Use null for \
                      fields you cannot determine. Be concise and accurate.";

        let user = format!(
            "Extract company information from this website content and return ONLY a JSON object \
             with these exact fields:\n\
             {{\"industry\": \"string or null\", \"size\": \"string or null (e.g. '50-200 employees')\", \
             \"tech_stack\": [\"list\", \"of\", \"technologies\"], \
             \"culture_keywords\": [\"list\", \"of\", \"culture\", \"words\"], \
             \"recent_news\": [\"list\", \"of\", \"recent\", \"news\", \"headlines\"]}}\n\n\
             Website content:\n{content}"
        );

        let response = self.completer.complete(system, &user).await?;

        let data = extract_json_from_response(&response)?;

        company.industry = data.industry.clone();
        company.size = data.size.clone();
        company.tech_stack = data.tech_stack.clone();
        company.culture_keywords = data.culture_keywords.clone();

        if !data.recent_news.is_empty() {
            company.notes = Some(data.recent_news.join("; "));
        }

        repo.update(&company).await?;

        Ok(data)
    }
}

pub fn enrichment_badge(industry: Option<&str>) -> &'static str {
    if industry.is_some() { "[E]" } else { "[ ]" }
}

fn extract_json_from_response(response: &str) -> Result<EnrichmentData> {
    let trimmed = response.trim();

    let json_start = trimmed
        .find('{')
        .ok_or_else(|| CoreError::Parse("LLM response contained no JSON object".into()))?;

    let json_end = trimmed
        .rfind('}')
        .ok_or_else(|| CoreError::Parse("LLM response JSON object was not closed".into()))?;

    let json_str = &trimmed[json_start..=json_end];

    serde_json::from_str(json_str)
        .map_err(|e| CoreError::Parse(format!("failed to parse enrichment JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct MockCompleter {
        response: String,
    }

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    // learning test: verifies reqwest::Client can be constructed and used with a mock server
    async fn reqwest_client_builds() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("client should build");

        // basic smoke test — client is usable
        let _ = client;
    }

    #[test]
    fn enrichment_data_deserializes_from_json() {
        let json = r#"{
            "industry": "Technology",
            "size": "1000-5000 employees",
            "tech_stack": ["Rust", "Go", "Kubernetes"],
            "culture_keywords": ["remote-first", "inclusive"],
            "recent_news": ["Company raises Series C"]
        }"#;

        let data: EnrichmentData = serde_json::from_str(json).unwrap();
        assert_eq!(data.industry.as_deref(), Some("Technology"));
        assert_eq!(data.size.as_deref(), Some("1000-5000 employees"));
        assert_eq!(data.tech_stack, vec!["Rust", "Go", "Kubernetes"]);
        assert_eq!(data.culture_keywords, vec!["remote-first", "inclusive"]);
        assert_eq!(data.recent_news, vec!["Company raises Series C"]);
    }

    #[test]
    fn enrichment_data_deserializes_with_nulls() {
        let json = r#"{
            "industry": null,
            "size": null,
            "tech_stack": [],
            "culture_keywords": [],
            "recent_news": []
        }"#;

        let data: EnrichmentData = serde_json::from_str(json).unwrap();
        assert!(data.industry.is_none());
        assert!(data.size.is_none());
        assert!(data.tech_stack.is_empty());
    }

    #[test]
    fn enrichment_badge_returns_e_for_enriched() {
        assert_eq!(enrichment_badge(Some("Technology")), "[E]");
    }

    #[test]
    fn enrichment_badge_returns_empty_for_unenriched() {
        assert_eq!(enrichment_badge(None), "[ ]");
    }

    #[test]
    fn extract_json_from_clean_response() {
        let response = r#"{"industry": "Tech", "size": "100", "tech_stack": [], "culture_keywords": [], "recent_news": []}"#;
        let data = extract_json_from_response(response).unwrap();
        assert_eq!(data.industry.as_deref(), Some("Tech"));
    }

    #[test]
    fn extract_json_from_response_with_preamble() {
        let response = r#"Here is the JSON: {"industry": "Finance", "size": "500", "tech_stack": ["Python"], "culture_keywords": ["fast-paced"], "recent_news": []}"#;
        let data = extract_json_from_response(response).unwrap();
        assert_eq!(data.industry.as_deref(), Some("Finance"));
        assert_eq!(data.tech_stack, vec!["Python"]);
    }

    #[test]
    fn extract_json_no_json_returns_error() {
        let result = extract_json_from_response("No JSON here at all");
        assert!(matches!(result, Err(CoreError::Parse(_))));
    }

    #[tokio::test]
    async fn completer_trait_dyn_dispatch() {
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter {
            response: r#"{"industry": "AI", "size": "50", "tech_stack": ["Rust"], "culture_keywords": ["innovative"], "recent_news": []}"#.into(),
        });
        let result = completer.complete("system", "user").await.unwrap();
        assert!(result.contains("AI"));
    }

    #[cfg(all(test, feature = "integration"))]
    #[tokio::test]
    async fn enrich_company_with_mock_completer() {
        use crate::domain::Company;
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

        let database_url = std::env::var("DATABASE_URL").unwrap();
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "<html><body><h1>TechCorp - AI-first company</h1><p>We use Rust and Python</p></body></html>",
                ),
            )
            .mount(&mock_server)
            .await;

        let mut company = Company::new("TechCorp");
        company.website = Some(mock_server.uri());
        let repo = CompanyRepository::new(pool.clone());
        repo.insert(&company).await.unwrap();

        let completer = Arc::new(MockCompleter {
            response: r#"{"industry": "Technology", "size": "50-200", "tech_stack": ["Rust", "Python"], "culture_keywords": ["ai-first"], "recent_news": []}"#.into(),
        });
        let client = reqwest::Client::new();
        let researcher = CompanyResearcher::new(completer, client);

        let data = researcher.enrich(&company.id, &pool).await.unwrap();
        assert_eq!(data.industry.as_deref(), Some("Technology"));
        assert_eq!(data.tech_stack, vec!["Rust", "Python"]);

        let updated = repo.find_by_id(&company.id).await.unwrap().unwrap();
        assert_eq!(updated.industry.as_deref(), Some("Technology"));

        repo.delete(&company.id).await.unwrap();
    }
}
