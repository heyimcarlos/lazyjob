use async_trait::async_trait;
use serde::Deserialize;

use crate::domain::Job;
use crate::error::{CoreError, Result};

use super::{JobSource, RateLimiter, strip_html};

const LEVER_API_BASE: &str = "https://api.lever.co/v0/postings";

pub struct LeverClient {
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    base_url: String,
}

impl LeverClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            rate_limiter: RateLimiter::new(2),
            base_url: LEVER_API_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>> {
        self.do_fetch(company_id).await
    }

    async fn do_fetch(&self, company_id: &str) -> Result<Vec<Job>> {
        self.rate_limiter.wait().await;

        let url = format!("{}/{}?mode=json", self.base_url, company_id);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CoreError::Http(format!(
                "Lever API returned status {} for company {}",
                response.status(),
                company_id
            )));
        }

        let postings: Vec<LeverPosting> = response
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        Ok(postings
            .into_iter()
            .map(|p| p.into_job(company_id))
            .collect())
    }
}

impl Default for LeverClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobSource for LeverClient {
    fn name(&self) -> &'static str {
        "lever"
    }

    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>> {
        self.do_fetch(company_id).await
    }
}

#[derive(Deserialize)]
struct LeverPosting {
    id: String,
    text: String,
    description: Option<String>,
    categories: Option<LeverCategories>,
    #[serde(rename = "createdAt")]
    created_at: Option<i64>,
    #[serde(rename = "hostedUrl")]
    hosted_url: Option<String>,
}

impl LeverPosting {
    fn into_job(self, company_id: &str) -> Job {
        let description = self.description.as_deref().map(strip_html);
        let mut job = Job::new(self.text);
        job.source = Some("lever".to_string());
        job.source_id = Some(self.id);
        job.company_name = Some(company_id.to_string());
        job.url = self.hosted_url;
        job.description = description;
        if let Some(cats) = self.categories {
            job.location = cats.location;
        }
        let _ = self.created_at;
        job
    }
}

#[derive(Deserialize)]
struct LeverCategories {
    location: Option<String>,
    #[allow(dead_code)]
    team: Option<String>,
    #[allow(dead_code)]
    commitment: Option<String>,
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    static LEVER_FIXTURE: &str = r#"[
        {
            "id": "abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a",
            "text": "Senior Software Engineer",
            "description": "<p>We are looking for a <strong>skilled engineer</strong>.</p>",
            "categories": {
                "location": "San Francisco, CA",
                "team": "Engineering",
                "commitment": "Full-time"
            },
            "createdAt": 1706123456000,
            "hostedUrl": "https://jobs.lever.co/notion/abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a"
        },
        {
            "id": "bcd12345-abcd-1234-abcd-1234567890ab",
            "text": "Product Designer",
            "description": null,
            "categories": null,
            "createdAt": null,
            "hostedUrl": null
        }
    ]"#;

    #[tokio::test]
    async fn lever_parses_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/notion"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(LEVER_FIXTURE, "application/json"),
            )
            .mount(&mock_server)
            .await;

        let client = LeverClient::new().with_base_url(mock_server.uri());
        let jobs = client.fetch_jobs("notion").await.unwrap();

        assert_eq!(jobs.len(), 2);

        let first = &jobs[0];
        assert_eq!(first.title, "Senior Software Engineer");
        assert_eq!(first.source.as_deref(), Some("lever"));
        assert_eq!(
            first.source_id.as_deref(),
            Some("abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a")
        );
        assert_eq!(first.company_name.as_deref(), Some("notion"));
        assert_eq!(first.location.as_deref(), Some("San Francisco, CA"));
        assert_eq!(
            first.url.as_deref(),
            Some("https://jobs.lever.co/notion/abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a")
        );

        let desc = first.description.as_deref().unwrap();
        assert!(desc.contains("skilled engineer"));
        assert!(!desc.contains('<'));

        let second = &jobs[1];
        assert_eq!(second.title, "Product Designer");
        assert!(second.location.is_none());
        assert!(second.url.is_none());
        assert!(second.description.is_none());
    }

    #[tokio::test]
    async fn lever_returns_error_on_bad_status() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/unknown"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = LeverClient::new().with_base_url(mock_server.uri());
        let result = client.fetch_jobs("unknown").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"));
    }

    #[test]
    fn lever_posting_strips_html_from_description() {
        let posting = LeverPosting {
            id: "abc".to_string(),
            text: "Engineer".to_string(),
            description: Some("<p>Join our <em>team</em></p>".to_string()),
            categories: None,
            created_at: None,
            hosted_url: None,
        };
        let job = posting.into_job("acme");
        let desc = job.description.unwrap();
        assert!(desc.contains("Join our"));
        assert!(desc.contains("team"));
        assert!(!desc.contains('<'));
    }

    #[test]
    fn lever_posting_uses_categories_location() {
        let posting = LeverPosting {
            id: "xyz".to_string(),
            text: "PM".to_string(),
            description: None,
            categories: Some(LeverCategories {
                location: Some("New York, NY".to_string()),
                team: Some("Product".to_string()),
                commitment: Some("Full-time".to_string()),
            }),
            created_at: None,
            hosted_url: None,
        };
        let job = posting.into_job("company");
        assert_eq!(job.location.as_deref(), Some("New York, NY"));
    }
}
