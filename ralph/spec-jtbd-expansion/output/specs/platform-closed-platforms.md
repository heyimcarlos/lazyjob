# Spec: Platform Closed Platforms

**JTBD**: Access all major job platforms from one tool without context switching
**Topic**: Handle platforms with restricted or zero public API access (LinkedIn, Workday, Indeed) using scraping, aggregation APIs, and manual-workflow alternatives
**Domain**: platform-integrations

---

## What

A specification for handling platforms that do not offer public APIs for job discovery or application submission — specifically LinkedIn, Workday, and Indeed. These platforms require scraping, third-party aggregation APIs, or human-assisted workflows. LazyJob's approach is explicitly **no automated access** for LinkedIn (ToS + legal risk), **browser-automation-first** for Workday (enterprise auth makes scraping hard), and **Adzuna API** for Indeed (free tier, no scraping required).

## Why

Even with Greenhouse and Lever covering ~30% of tech postings, users still need LinkedIn (77-80% of job seekers save jobs there), Workday (Fortune 500, 39% market share), and broad job boards (Indeed drives 66% of all applications). LazyJob cannot ignore these platforms without sacrificing real utility.

The key insight is that **LinkedIn cannot be automated** — ToS Section 8.2 explicitly bans all third-party automation, detection increased 340% from 2023-2025, and the Proxycurl/Apollo.io legal precedent shows that even B2B scraping tools get shut down. The right strategy for LazyJob is: CSV import for user-owned data, manual-approval for anything touching LinkedIn, and Adzuna/aggregation API for broad discovery.

## How

### Platform Access Tier Summary

| Platform | Discovery | Apply | Approach |
|----------|-----------|-------|----------|
| Greenhouse | Public API | Public API | Direct (Phase 1) |
| Lever | Public API | Public API | Direct (Phase 1) |
| Adzuna | Free API | No | Direct (Phase 2) |
| LinkedIn | Scraping (illegal) | Easy Apply (illegal) | CSV import + manual |
| Workday | Scraping | Browser automation | Apify + human review |
| Indeed | No public API | No | Adzuna covers discovery |
| Glassdoor | No public API | No | User clipboard import |

### LinkedIn: Manual-Only Access

**Hard constraint: No automated LinkedIn access — ever.** LinkedIn's ToS Section 8.2 explicitly prohibits all third-party automation, including browser extensions. LinkedIn has successfully sued Proxycurl, Apollo.io, and Seamless.ai (2025). Detection rates increased 340% from 2023-2025. Automating LinkedIn is both a ToS violation and an active legal risk.

**What LazyJob supports for LinkedIn:**

1. **CSV Export Import**: Users export their LinkedIn saved jobs via CSV (LinkedIn Sales Navigator or third-party export tools). LazyJob parses and imports into the job database. This is user-owned data.
2. **Job URL import**: User pastes a LinkedIn job URL. LazyJob stores it as a `Job` with `source = "linkedin"` and `url = <URL>`. No automated fetch — the user manually visits the URL or uses the browser extension approach.
3. **Manual apply workflow**: LazyJob cannot submit to LinkedIn. The user opens the URL in their browser. LazyJob tracks that an application was submitted via the `application_contacts` table linked to the `jobs` table.

```rust
// LinkedIn-specific: store as manual-entry job
#[derive(Debug, Clone)]
pub struct ManualJobEntry {
    pub url: String,
    pub source: &'static str = "linkedin",
    pub title: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
}

impl JobRepository {
    /// User pastes a LinkedIn job URL — we store it as a bookmark, no fetch attempted
    pub async fn import_linkedin_url(&self, url: &str, user_notes: Option<String>) -> Result<Job> {
        let job = Job {
            id: Uuid::new_v4(),
            title: "LinkedIn Job (imported manually)".to_string(),
            url: url.to_string(),
            source: "linkedin".to_string(),
            status: JobStatus::Discovered,
            notes: user_notes.unwrap_or_default(),
            ..Default::default()
        };
        self.insert(&job).await?;
        Ok(job)
    }
}
```

**What LazyJob explicitly does NOT do:**
- No automated job fetching from LinkedIn
- No automated profile scraping
- No browser extension for automated apply
- No InMail/messaging automation

### Workday: Browser Automation with Human Review

Workday has no public API for job seekers. Career sites are heavily JavaScript-rendered. This makes scraping difficult and automation fragile.

**Architecture: Apify Actors + Human Approval**

```
Workday URL → Apify Career Page Scraper Actor → Structured Job Data → TUI Preview → User Approves → Saved to JobRepository
```

```rust
// lazyjob-core/src/platforms/workday.rs

pub struct WorkdayIntegration {
    apify_client: reqwest::Client,
    api_key: String,
}

impl WorkdayIntegration {
    /// Fetches a Workday career page URL via Apify Actor
    /// Returns raw job listings that the TUI displays for user review before saving
    pub async fn fetch_career_page(&self, career_page_url: &str) -> Result<Vec<ApifyJobResult>> {
        let response = self.apify_client
            .post("https://api.apify.com/v2/acts/scrapepilot~career-page-job-scraper/run-sync-get-v1")
            .json(&ApifyRunInput {
                startUrls: vec![career_page_url.to_string()],
                maxConcurrency: 1,
            })
            .header("Authorization", format!("APIFY-API-TOKEN: {}", self.api_key))
            .send()
            .await?;
        let result: ApifyRunResponse = response.json().await?;
        Ok(result.items)
    }

    /// TUI previews results before calling this to persist selected jobs
    pub async fn save_jobs(&self, jobs: Vec<DiscoveredJob>) -> Result<()> {
        for job in jobs {
            self.job_repo.insert(&job).await?;
        }
        Ok(())
    }
}
```

**Key constraints:**
- Apify career scraper costs ~$0.005/result — reasonable for discovery use
- Results must go through human review before saving (anti-fabrication guardrail)
- Workday URL input is user-initiated only (no auto-discovery loop for Workday)

### Indeed: Via Adzuna API

Indeed has no public API for job search or application. Adzuna (free tier, 12 countries) provides a clean REST API for job discovery that covers many of the same listings.

```rust
// lazyjob-core/src/platforms/adzuna.rs

pub struct AdzunaClient {
    client: reqwest::Client,
    app_id: String,
    app_key: String,
    country: String, // 'gb', 'us', 'de', etc.
}

impl AdzunaClient {
    pub fn new(app_id: &str, app_key: &str, country: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            app_id: app_id.to_string(),
            app_key: app_key.to_string(),
            country: country.to_string(),
        }
    }

    pub async fn search(&self, query: &str, location: Option<&str>) -> Result<Vec<AdzunaJob>> {
        let mut url = format!(
            "https://api.adzuna.com/v1/api/jobs/{}/search/1?app_id={}&app_key={}&what={}",
            self.country, self.app_id, self.app_key, urlencoding(query)
        );
        if let Some(loc) = location {
            url.push_str(&format!("&where={}", urlencoding(loc)));
        }
        let response = self.client.get(&url).send().await?;
        let data: AdzunaResponse = response.json().await?;
        Ok(data.results)
    }
}
```

Adzuna free tier: 100 calls/day. Sufficient for passive monitoring (hourly checks during active search). Phase 2: paid tier for power users.

### User Configuration

```toml
[platforms.adzuna]
enabled = true
app_id = "your_adzuna_app_id"
app_key = "your_adzuna_app_key"
country = "us"  # gb, us, de, etc.

[platforms.apify]
enabled = true
api_key = "your_apify_api_key"

[platforms.linkedin]
enabled = false  # Manual import only, no automation
```

### Anti-Spam / Quality Gate

Since closed-platform integrations rely on scraping or third-party APIs, quality signals are weaker than direct API data. All scraped or aggregated jobs must be flagged with `source_quality = "scraped" | "aggregated"` so ghost detection heuristics give appropriate weight to the `source` signal.

## Open Questions

- **Apify reliability for Workday**: Apify actors handle anti-bot evasion. The cost is $0.005/result. What's the expected volume for Workday job discovery? For Phase 1, user-initiated fetch-on-demand is the right scope — not background polling.
- **Rebrowser for Workday automation**: The `rebrowser` project rebuilds automation outside CDP to avoid protocol-level detection. For Workday enterprise career sites (heavy JavaScript), this might be more reliable than Apify actors. Investigate in Phase 2.
- **Adzuna API rate limits**: Free tier is 100 calls/day. If ralph discovery loops run hourly, that's 24 calls/day — within limits. But for active job seekers doing full discovery sweeps, this could be exceeded. Phase 2 should consider a paid Adzuna plan or aggregator alternative.
- **LinkedIn Easy Apply via browser extension**: Products like Simplify Copilot (1M+ Chrome installs) auto-fill LinkedIn Easy Apply forms. This is a ToS violation for the user (not LazyJob directly), but LazyJob's positioning as "quality over volume" should explicitly not endorse auto-apply. Decision: no LinkedIn Easy Apply integration.

## Implementation Tasks

- [ ] Implement `AdzunaClient` in `lazyjob-core/src/platforms/adzuna.rs` with `search`, `job_detail` methods and `AdzunaJob` → `DiscoveredJob` normalization
- [ ] Add `[platforms.adzuna]` and `[platforms.apify]` TOML config sections for Adzuna app_id/app_key and Apify API key
- [ ] Build `WorkdayIntegration::fetch_career_page()` in `lazyjob-core/src/platforms/workday.rs` calling Apify Actor API with user-provided career page URL
- [ ] Add `Job.source_quality` field (`"api" | "scraped" | "aggregated"`) to the jobs table to flag scraped/aggregated sources for ghost detection weighting
- [ ] Implement `JobRepository::import_linkedin_url()` in `lazyjob-core/src/platforms/manual.rs` — stores LinkedIn URL as a manual bookmark, no fetch attempted
- [ ] Write TUI flow for Apify Workday scrape: user pastes Workday URL → Apify fetch → TUI preview table → user selects which jobs to save → save to repository
- [ ] Add `source_quality` to ghost detection scoring formula: scraped jobs get +0.1 additional ghost_score weight since quality signals are weaker
