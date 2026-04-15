# Platform API Integrations

## Status
Researching

## Problem Statement

LazyJob integrates with job platforms to discover opportunities. Each platform has different APIs, authentication requirements, and data quality. This spec covers Greenhouse, Lever, Workday, LinkedIn, and browser automation approaches.

---

## Research Findings

### Greenhouse API

**Public Job Board API** (no authentication):
```
GET https://boards-api.greenhouse.io/v1/boards/{board_token}/jobs
GET https://boards-api.greenhouse.io/v1/boards/{board_token}/jobs/{job_id}
```

**Authentication**: None for public boards
**Rate Limit**: Reasonable (avoid > 60 req/min)
**Data Quality**: High (structured, HTML descriptions)

### Lever API

**Public API**:
```
GET https://api.lever.co/v0/postings/{company}?mode=json
```

**Authentication**: None for public postings
**Rate Limit**: Reasonable
**Data Quality**: High (structured categories)

### Workday

**Challenges**:
- No public API
- Requires enterprise authentication
- Heavily JavaScript-rendered

**Approaches**:
1. Browser automation (Playwright)
2. Credential-based scraping
3. Third-party scrapers (JobSpy, scrapinghub)

### LinkedIn

**Challenges**:
- Strictly against ToS
- Requires authentication
- Heavy JavaScript rendering
- CAPTCHA protection

**Note**: LinkedIn scraping is NOT recommended for production use.

### Browser Automation (Playwright)

For Workday and similar JavaScript-heavy sites:

```rust
use playwright::Playwright;

async fn scrape_workday(url: &str) -> Result<Vec<Job>> {
    let playwright = Playwright::new().await?;
    playwright.install_chromium().await?;

    let browser = playwright.chromium().launch().await?;
    let page = browser.new_page().await?;

    page.goto(url).await?;
    page.wait_for_selector("job listings", Duration::from_secs(10)).await?;

    // Extract job data
    let jobs = page.evaluate("...").await?;

    browser.close().await?;
    Ok(jobs)
}
```

---

## Integration Architecture

```rust
// lazyjob-core/src/platforms/mod.rs

pub trait PlatformClient: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;

    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<DiscoveredJob>>;
    async fn fetch_job(&self, company_id: &str, job_id: &str) -> Result<DiscoveredJob>;
    fn normalize_job(&self, raw: RawJob) -> Result<DiscoveredJob>;
}

pub struct PlatformRegistry {
    clients: HashMap<String, Box<dyn PlatformClient>>,
}
```

### Greenhouse Implementation

```rust
pub struct GreenhouseClient {
    client: reqwest::Client,
    board_token: String,
}

#[async_trait::async_trait]
impl PlatformClient for GreenhouseClient {
    fn name(&self) -> &str { "greenhouse" }
    fn base_url(&self) -> &str { "https://boards-api.greenhouse.io" }

    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>> {
        let url = format!(
            "{}/v1/boards/{}/jobs?content=true",
            self.base_url(),
            self.board_token
        );

        let response = self.client.get(&url).send().await?;
        let data: GreenhouseResponse = response.json().await?;

        data.jobs
            .into_iter()
            .map(|j| self.normalize_job(j))
            .collect()
    }

    fn normalize_job(&self, raw: RawJob) -> Result<DiscoveredJob> {
        Ok(DiscoveredJob {
            source: "greenhouse".to_string(),
            source_id: raw.id.to_string(),
            title: raw.title,
            company_name: self.board_token.to_string(),
            location: raw.location.map(|l| l.name),
            url: raw.absolute_url,
            description: strip_html(&raw.content.unwrap_or_default()),
            department: raw.departments.first().map(|d| d.name.clone()),
            ..Default::default()
        })
    }
}
```

### Lever Implementation

```rust
pub struct LeverClient {
    client: reqwest::Client,
    company_id: String,
}

#[async_trait::async_trait]
impl PlatformClient for LeverClient {
    fn name(&self) -> &str { "lever" }
    fn base_url(&self) -> &str { "https://api.lever.co" }

    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>> {
        let url = format!(
            "{}/v0/postings/{}?mode=json",
            self.base_url(),
            self.company_id
        );

        let response = self.client.get(&url).send().await?;
        let data: LeverResponse = response.json().await?;

        data.postings
            .into_iter()
            .map(|p| self.normalize_job(p))
            .collect()
    }

    fn normalize_job(&self, raw: LeverPosting) -> Result<DiscoveredJob> {
        Ok(DiscoveredJob {
            source: "lever".to_string(),
            source_id: raw.id,
            title: raw.title,
            company_name: self.company_id.to_string(),
            location: raw.location,
            url: raw.url,
            description: raw.description,
            department: raw.categories.and_then(|c| c.department),
            employment_type: raw.categories.and_then(|c| c.commitment),
            ..Default::default()
        })
    }
}
```

### Rate Limiting

```rust
pub struct RateLimiter {
    requests_per_minute: usize,
    last_request: Option<DateTime<Utc>>,
}

impl RateLimiter {
    pub async fn acquire(&self) -> Result<()> {
        if let Some(last) = self.last_request {
            let elapsed = Utc::now() - last;
            let min_interval = Duration::from_secs(60) / self.requests_per_minute as u64;

            if elapsed < min_interval {
                tokio::time::sleep(min_interval - elapsed).await;
            }
        }

        self.last_request = Some(Utc::now());
        Ok(())
    }
}
```

---

## Data Quality Matrix

| Platform | API Type | Auth Required | Data Quality | Update Frequency |
|----------|----------|---------------|--------------|------------------|
| Greenhouse | REST (public) | No | High | Real-time |
| Lever | REST (public) | No | High | Real-time |
| Workday | Scraping | Yes (enterprise) | High | Varies |
| LinkedIn | Scraping | Yes | Medium | Stale |
| Indeed | Scraping | No | Medium | Stale |
| Glassdoor | Scraping | No | Low | Very stale |

---

## Sources

- [Greenhouse Job Board API](https://developers.greenhouse.io/job-board)
- [Lever API Documentation](https://docs.lever.co/)
- [Playwright for Rust](https://github.com/kobzasr/playwright-rust)
