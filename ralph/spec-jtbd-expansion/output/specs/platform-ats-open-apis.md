# Spec: Platform ATS Open APIs

**JTBD**: Access all major job platforms from one tool without context switching
**Topic**: Integrate with open ATS platforms (Greenhouse, Lever) via their public APIs for job discovery and application submission
**Domain**: platform-integrations

---

## What

A platform integration layer for public ATS APIs (Greenhouse Job Board API and Lever Postings API) that enables job discovery, listing enrichment, and application submission through a unified `PlatformClient` trait. Phase 1 covers Greenhouse and Lever as both are free, public, and have no authentication requirements.

## Why

Greenhouse and Lever together account for ~30% of tech company job postings. Their public APIs are explicitly sanctioned by the platforms — using them is the safest possible integration path. They provide structured, high-quality data (HTML-stripped descriptions, department/commitment metadata) that scraped data cannot match. Building a first-class integration now creates the foundation for a multi-platform aggregation layer later.

Without this, LazyJob users must manually check each company's Greenhouse/Lever page — destroying the ambient discovery experience that ralph loops are designed to provide.

## How

### Architecture

```
lazyjob-core/src/platforms/
├── mod.rs                    # PlatformRegistry, re-exports
├── traits.rs                 # PlatformClient trait, JobNormalizer
├── greenhouse.rs             # GreenhouseClient implementation
├── lever.rs                 # LeverClient implementation
└── rate_limiter.rs          # Shared RateLimiter
```

### PlatformClient Trait

```rust
// lazyjob-core/src/platforms/traits.rs

#[async_trait::async_trait]
pub trait PlatformClient: Send + Sync {
    fn platform_name(&self) -> &'static str;
    fn base_url(&self) -> &'static str;

    async fn fetch_jobs(&self, board_token: &str) -> Result<Vec<RawJob>>;
    async fn fetch_job(&self, board_token: &str, job_id: &str) -> Result<RawJob>;
    async fn submit_application(
        &self,
        board_token: &str,
        job_id: &str,
        application: &ApplicationSubmission,
    ) -> Result<ApplicationResponse>;
}

pub trait JobNormalizer: Send + Sync {
    fn normalize(&self, raw: RawJob, board_company_name: &str) -> Result<DiscoveredJob>;
}
```

### Greenhouse Implementation

Greenhouse's public Job Board API requires no authentication. The `board_token` is a public identifier visible in job URLs (e.g., `stripe` in `boards.greenhouse.io/stripe`).

```rust
// lazyjob-core/src/platforms/greenhouse.rs

pub struct GreenhouseClient {
    client: reqwest::Client,
}

impl GreenhouseClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("LazyJob/1.0 (job search tool)")
                .build()
                .unwrap(),
        }
    }

    pub fn board_url(&self, board_token: &str) -> String {
        format!("https://boards-api.greenhouse.io/v1/boards/{}/jobs", board_token)
    }
}

#[async_trait::async_trait]
impl PlatformClient for GreenhouseClient {
    fn platform_name(&self) -> &'static str { "greenhouse" }
    fn base_url(&self) -> &'static str { "https://boards-api.greenhouse.io" }

    async fn fetch_jobs(&self, board_token: &str) -> Result<Vec<RawJob>> {
        let url = format!("{}/v1/boards/{}/jobs?content=true", self.base_url(), board_token);
        let response = self.client.get(&url).send().await?;
        let data: GreenhouseResponse = response.json().await?;
        Ok(data.jobs)
    }

    async fn fetch_job(&self, board_token: &str, job_id: &str) -> Result<RawJob> {
        let url = format!("{}/v1/boards/{}/jobs/{}", self.base_url(), board_token, job_id);
        let response = self.client.get(&url).send().await?;
        let job: GreenhouseJob = response.json().await?;
        Ok(job.into())
    }

    async fn submit_application(
        &self,
        board_token: &str,
        job_id: &str,
        application: &ApplicationSubmission,
    ) -> Result<ApplicationResponse> {
        let url = format!(
            "{}/v1/boards/{}/jobs/{}/apply",
            self.base_url(), board_token, job_id
        );
        let response = self.client
            .post(&url)
            .json(&application.to_greenhouse_format())
            .send()
            .await?;
        // Greenhouse returns 200 even for validation errors — check body
        let body: serde_json::Value = response.json().await?;
        if body.get("error").is_some() {
            return Err(Error::ApplicationSubmission(body["error"].as_str().unwrap_or("unknown").into()));
        }
        Ok(ApplicationResponse { success: true, application_id: body["id"].as_str().map(String::from) })
    }
}
```

### Lever Implementation

Lever's public API uses the company ID (e.g., `stripe` for `lever.co/postings/stripe`). No authentication for public postings.

```rust
// lazyjob-core/src/platforms/lever.rs

pub struct LeverClient {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl PlatformClient for LeverClient {
    fn platform_name(&self) -> &'static str { "lever" }
    fn base_url(&self) -> &'static str { "https://api.lever.co" }

    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<RawJob>> {
        let url = format!("{}/v0/postings/{}?mode=json", self.base_url(), company_id);
        let response = self.client.get(&url).send().await?;
        let data: LeverResponse = response.json().await?;
        Ok(data.postings.into_iter().map(LeverPosting::into).collect())
    }

    async fn submit_application(
        &self,
        company_id: &str,
        job_id: &str,
        application: &ApplicationSubmission,
    ) -> Result<ApplicationResponse> {
        let url = format!("{}/v0/postings/{}/apply/{}", self.base_url(), company_id, job_id);
        let response = self.client
            .post(&url)
            .json(&application.to_lever_format())
            .send()
            .await?;
        if !response.status().is_success() {
            let body: serde_json::Value = response.json().await?;
            return Err(Error::ApplicationSubmission(body["message"].as_str().unwrap_or("unknown").into()));
        }
        Ok(ApplicationResponse { success: true, application_id: None })
    }
}
```

### Job Normalization

Both platforms produce a `RawJob` that must be normalized to LazyJob's canonical `DiscoveredJob`:

```rust
// lazyjob-core/src/platforms/traits.rs

pub struct DiscoveredJob {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub company_name: String,
    pub company_id: Option<String>,       // LazyJob internal CompanyRecord FK
    pub location: Option<String>,
    pub remote: Option<String>,            // 'yes', 'no', 'hybrid'
    pub url: String,
    pub description: String,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub salary_currency: Option<String>,
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub posted_at: Option<DateTime<Utc>>,
}

impl GreenhouseClient {
    fn normalize(&self, raw: RawJob, board_company_name: &str) -> Result<DiscoveredJob> {
        Ok(DiscoveredJob {
            source: "greenhouse".to_string(),
            source_id: raw.id.to_string(),
            title: raw.title,
            company_name: board_company_name.to_string(),
            location: raw.location.map(|l| l.name),
            url: raw.absolute_url,
            description: strip_html(raw.content.unwrap_or_default()),
            department: raw.departments.first().map(|d| d.name.clone()),
            employment_type: raw.levels.first().map(|l| l.name),
            ..Default::default()
        })
    }
}
```

### Rate Limiting

Both platforms share a single `RateLimiter` instance via `LazyLock`:

```rust
// lazyjob-core/src/platforms/rate_limiter.rs

use std::sync::LazyLock;
use tokio::sync::Mutex;

static RATE_LIMITER: LazyLock<Mutex<RateLimiter>> = LazyLock::new(|| {
    Mutex::new(RateLimiter::new(30)) // 30 req/min shared across all platform clients
});

pub struct RateLimiter {
    requests_per_minute: usize,
    last_request: Option<DateTime<Utc>>,
}

impl RateLimiter {
    pub fn new(rpm: usize) -> Self {
        Self { requests_per_minute: rpm, last_request: None }
    }

    pub async fn acquire(&self) {
        let mut lock = self.rate_limiter.lock().await;
        if let Some(last) = lock.last_request {
            let elapsed = Utc::now() - last;
            let min_interval = Duration::from_secs(60) / lock.requests_per_minute as u64;
            if elapsed < min_interval {
                tokio::time::sleep(min_interval - elapsed).await;
            }
        }
        lock.last_request = Some(Utc::now());
    }
}
```

### PlatformRegistry

```rust
// lazyjob-core/src/platforms/mod.rs

pub struct PlatformRegistry {
    clients: HashMap<&'static str, Box<dyn PlatformClient>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        let mut registry = Self { clients: HashMap::new() };
        registry.register(Box::new(GreenhouseClient::new()));
        registry.register(Box::new(LeverClient::new()));
        registry
    }

    pub fn register(&mut self, client: Box<dyn PlatformClient>) {
        self.clients.insert(client.platform_name(), client);
    }

    pub fn client(&self, name: &str) -> Option<&dyn PlatformClient> {
        self.clients.get(name).map(|b| b.as_ref())
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.clients.keys().copied().collect()
    }
}
```

### User Configuration

Users configure platform boards in `lazyjob.toml`:

```toml
[platforms.greenhouse]
enabled = true
board_tokens = ["stripe", "notion", "figma"]

[platforms.lever]
enabled = true
company_ids = ["stripe", "notion"]
```

### Application Submission Flow

Application submission is Phase 2 only. The flow requires human-in-the-loop:
1. User triggers `ApplyWorkflow` from TUI job detail view
2. TUI displays full data preview (tailored resume, cover letter, answers to custom questions)
3. User confirms submission
4. `ApplyWorkflow::execute` calls `submit_application` via the appropriate `PlatformClient`
5. Result is written to `applications` table; user sees confirmation

Greenhouse requires `first_name`, `last_name`, `email`, and optionally `resume` (multipart), `cover_letter`, and custom field answers. Custom fields vary per job — the job fetch returns a `questions` array that maps to the application form.

## Open Questions

- **Custom field handling**: Greenhouse job `questions` arrays describe form fields with types (input, select, textarea, file). For Phase 1, we can skip custom fields and submit name/email/resume only. Phase 2 should build a custom field renderer in the TUI that reads the `questions` array and renders a dynamic form.
- **Lever authentication**: Lever's application submission endpoint requires an API key (from Lever admin settings). Phase 1 uses read-only public posting data. Phase 2 adds write access with per-user API key storage.

## Implementation Tasks

- [ ] Define `PlatformClient` trait and `DiscoveredJob` struct in `lazyjob-core/src/platforms/traits.rs` — include `RawJob`, `ApplicationSubmission`, `ApplicationResponse` types
- [ ] Implement `GreenhouseClient` in `lazyjob-core/src/platforms/greenhouse.rs` — fetch_jobs, fetch_job, submit_application, job normalization with HTML stripping
- [ ] Implement `LeverClient` in `lazyjob-core/src/platforms/lever.rs` — fetch_jobs, fetch_job, submit_application, normalization
- [ ] Build `PlatformRegistry` in `lazyjob-core/src/platforms/mod.rs` with `register`, `client`, `names` methods
- [ ] Add `RateLimiter` with `LazyLock` singleton in `lazyjob-core/src/platforms/rate_limiter.rs`, apply to all client fetch calls
- [ ] Add `[platforms.greenhouse]` and `[platforms.lever]` TOML config sections, parse in `lazyjob-core/src/config.rs`
- [ ] Write integration tests with mock HTTP responses for both Greenhouse and Lever — verify normalization, rate limiting, error handling
