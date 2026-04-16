use async_trait::async_trait;
use serde::Deserialize;

use crate::domain::Job;
use crate::error::{CoreError, Result};

use super::{JobSource, RateLimiter, strip_html};

const GREENHOUSE_API_BASE: &str = "https://boards-api.greenhouse.io/v1/boards";

pub struct GreenhouseClient {
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    base_url: String,
}

impl GreenhouseClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            rate_limiter: RateLimiter::new(1),
            base_url: GREENHOUSE_API_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub async fn fetch_jobs(&self, board_token: &str) -> Result<Vec<Job>> {
        self.do_fetch(board_token).await
    }

    async fn do_fetch(&self, board_token: &str) -> Result<Vec<Job>> {
        self.rate_limiter.wait().await;

        let url = format!("{}/{}/jobs?content=true", self.base_url, board_token);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CoreError::Http(format!(
                "Greenhouse API returned status {} for board {}",
                response.status(),
                board_token
            )));
        }

        let gh_response: GreenhouseResponse = response
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        Ok(gh_response
            .jobs
            .into_iter()
            .map(|j| j.into_job(board_token))
            .collect())
    }
}

impl Default for GreenhouseClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobSource for GreenhouseClient {
    fn name(&self) -> &'static str {
        "greenhouse"
    }

    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>> {
        self.do_fetch(company_id).await
    }
}

#[derive(Deserialize)]
struct GreenhouseResponse {
    jobs: Vec<GreenhouseJob>,
}

#[derive(Deserialize)]
struct GreenhouseJob {
    id: i64,
    title: String,
    content: Option<String>,
    location: Option<GreenhouseLocation>,
    #[allow(dead_code)]
    departments: Option<Vec<GreenhouseDepartment>>,
    #[allow(dead_code)]
    updated_at: Option<String>,
    absolute_url: Option<String>,
}

impl GreenhouseJob {
    fn into_job(self, board_token: &str) -> Job {
        let description = self.content.as_deref().map(strip_html);
        let mut job = Job::new(self.title);
        job.source = Some("greenhouse".to_string());
        job.source_id = Some(self.id.to_string());
        job.company_name = Some(board_token.to_string());
        job.location = self.location.and_then(|l| l.name);
        job.url = self.absolute_url;
        job.description = description;
        job
    }
}

#[derive(Deserialize)]
struct GreenhouseLocation {
    name: Option<String>,
}

#[derive(Deserialize)]
struct GreenhouseDepartment {
    #[allow(dead_code)]
    name: String,
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    static GREENHOUSE_FIXTURE: &str = r#"{
        "jobs": [
            {
                "id": 127817,
                "title": "Senior Software Engineer",
                "content": "<div><p>Build things with <b>Rust</b>.</p><ul><li>5+ years experience</li></ul></div>",
                "location": {"name": "San Francisco, CA"},
                "departments": [{"name": "Engineering", "id": 1}],
                "updated_at": "2024-01-15T10:30:00-05:00",
                "absolute_url": "https://boards.greenhouse.io/stripe/jobs/127817"
            },
            {
                "id": 127818,
                "title": "Product Manager",
                "content": null,
                "location": null,
                "departments": [],
                "updated_at": null,
                "absolute_url": null
            }
        ]
    }"#;

    // learning test: verifies wiremock MockServer intercepts HTTP and returns fixture body
    #[tokio::test]
    async fn wiremock_responds_with_json() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/boards/testco/jobs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"jobs":[]}"#, "application/json"),
            )
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/v1/boards/testco/jobs", mock_server.uri());
        let resp = client.get(&url).send().await.unwrap();
        assert!(resp.status().is_success());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["jobs"].is_array());
    }

    #[tokio::test]
    async fn greenhouse_parses_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stripe/jobs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(GREENHOUSE_FIXTURE, "application/json"),
            )
            .mount(&mock_server)
            .await;

        let client = GreenhouseClient::new().with_base_url(mock_server.uri());
        let jobs = client.fetch_jobs("stripe").await.unwrap();

        assert_eq!(jobs.len(), 2);

        let first = &jobs[0];
        assert_eq!(first.title, "Senior Software Engineer");
        assert_eq!(first.source.as_deref(), Some("greenhouse"));
        assert_eq!(first.source_id.as_deref(), Some("127817"));
        assert_eq!(first.company_name.as_deref(), Some("stripe"));
        assert_eq!(first.location.as_deref(), Some("San Francisco, CA"));
        assert_eq!(
            first.url.as_deref(),
            Some("https://boards.greenhouse.io/stripe/jobs/127817")
        );

        let desc = first.description.as_deref().unwrap();
        assert!(desc.contains("Build things with"));
        assert!(desc.contains("Rust"));
        assert!(!desc.contains('<'));

        let second = &jobs[1];
        assert_eq!(second.title, "Product Manager");
        assert!(second.location.is_none());
        assert!(second.url.is_none());
        assert!(second.description.is_none());
    }

    #[tokio::test]
    async fn greenhouse_returns_error_on_bad_status() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/unknown/jobs"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = GreenhouseClient::new().with_base_url(mock_server.uri());
        let result = client.fetch_jobs("unknown").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("404"));
    }

    #[test]
    fn greenhouse_job_strips_html_from_content() {
        let gh_job = GreenhouseJob {
            id: 1,
            title: "Engineer".to_string(),
            content: Some("<p>Hello <b>World</b></p>".to_string()),
            location: None,
            departments: None,
            updated_at: None,
            absolute_url: None,
        };
        let job = gh_job.into_job("acme");
        let desc = job.description.unwrap();
        assert!(desc.contains("Hello"));
        assert!(desc.contains("World"));
        assert!(!desc.contains('<'));
    }
}
