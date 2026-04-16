# Spec: Company Research Pipeline

**JTBD**: Find relevant job opportunities without wasting time on ghost jobs or mismatched roles (primary); Prepare for interviews systematically (secondary)
**Topic**: Enrich company records with culture, mission, tech stack, and recent news to support both application decisions and interview preparation
**Domain**: job-search

---

## What

The Company Research Pipeline builds and maintains a `CompanyRecord` for each company the user has configured or interacted with. It aggregates data from public sources (company website, Glassdoor summaries, Crunchbase funding, recent news) and stores a normalized, enriched record in SQLite. This record is consumed by three downstream features: (1) ghost job detection (headcount signals), (2) cover letter generation (tone matching, mission alignment), and (3) interview preparation (culture cheat sheet, recent news, tech stack).

The `CompanyRecord` struct lives in `lazyjob-core` — it is owned by this pipeline, not by any of its consumers.

## Why

Job seekers spend 1-3 hours researching a company before applying and 1-3 more hours before an interview. This research is repetitive (same company may appear in multiple job listings), unstructured (scattered across 5 browser tabs), and often forgotten by interview day. LazyJob should do this research once, store it locally, and surface it at the right moment — in the application review screen, in the cover letter prompt, and in the interview prep panel. Without centralized company data, each downstream feature must either re-fetch the same information or produce lower-quality output.

**Important**: `CompanyRecord` data must be clearly timestamped. Company culture, tech stack, and news go stale. The UI must show when data was last refreshed so users know whether to trust it for interview prep.

## How

### Data sources and collection strategy

| Data type | Source | Method | Freshness | Required? |
|---|---|---|---|---|
| Company description, mission, founding year | Company website (about page) | HTTP fetch + LLM extraction | On demand | Yes (Phase 1) |
| Tech stack | Stackshare.io, job descriptions | HTTP fetch + regex + LLM | On demand | Yes (Phase 1) |
| Employee count range | Greenhouse/Lever response metadata | Already in discovery | Real-time | Yes (Phase 1) |
| Recent news (last 90 days) | Google News RSS / Bing News API | RSS feed | Daily refresh | Phase 2 |
| Glassdoor summary (rating, pros/cons) | Glassdoor public pages | HTTP + LLM summarization | Weekly | Phase 2 |
| Funding history | Crunchbase public | HTTP fetch | Monthly | Phase 2 |
| Layoffs | layoffs.fyi | HTTP fetch | Weekly | Phase 2 |

**Phase 1 MVP**: Company website + tech stack + employee count (from discovery metadata). This is sufficient for basic cover letter personalization and ghost detection.

**Phase 2**: Add Glassdoor, news, Crunchbase, layoffs. These require either paid APIs or scraping — evaluated for ToS compliance before implementation.

### Pipeline flow

```
CompanyResearcher::enrich(company_name, website_url)
    → WebFetcher::fetch(about_url, careers_url)
        → LlmProvider::extract_structured(raw_html, CompanyExtractionSchema)
            → CompanyRecord { description, mission, values, tech_stack, size_signals }
                → CompanyRepository::upsert()
```

The pipeline is invoked:
1. When a new company is first discovered (discovery engine triggers it async)
2. When the user explicitly requests refresh (`r` key in TUI company view)
3. Pre-interview: triggered by the interview prep workflow, re-runs if data is > 7 days stale

### LLM extraction prompt

The `CompanyResearcher` sends the raw HTML of the company's About and Careers pages to the `LlmProvider` with a structured extraction prompt. Output schema:

```json
{
  "description": "...",
  "mission_statement": "...",
  "core_values": ["..."],
  "tech_stack": ["Rust", "Postgres", "Kubernetes", "..."],
  "product_areas": ["..."],
  "culture_signals": ["..."],
  "recent_hires_pattern": "..."
}
```

The prompt explicitly instructs the LLM: "Extract only factual information present in the source text. Do not invent or infer details not stated. Mark any field as null if not clearly present." This is the anti-fabrication constraint applied to company data.

### Tech stack inference from job descriptions

An additional tech stack inference pass reads all `DiscoveredJob.description` records for a given company and extracts technology mentions using a regex lexicon (same lexicon as ghost detection's vagueness scorer). This is a free, offline signal that works without any external API calls and improves tech stack coverage significantly.

### Storage

A `companies` table in SQLite holds the normalized company data. This is separate from the `CompanyConfig` used by the discovery engine (which is the user's configuration, not enriched research data).

## Interface

```rust
// lazyjob-core/src/companies/models.rs

pub struct CompanyRecord {
    pub id: Uuid,
    pub name: String,
    pub name_normalized: String,   // lowercase, stripped of Inc/LLC/Corp suffixes
    pub website_url: Option<String>,
    pub greenhouse_token: Option<String>,
    pub lever_id: Option<String>,
    
    // Enriched data
    pub description: Option<String>,
    pub mission_statement: Option<String>,
    pub core_values: Vec<String>,
    pub tech_stack: Vec<String>,
    pub product_areas: Vec<String>,
    pub culture_signals: Vec<String>,
    pub employee_count_range: Option<EmployeeCountRange>,
    pub funding_stage: Option<FundingStage>,
    pub founded_year: Option<u16>,
    pub hq_location: Option<String>,
    pub glassdoor_rating: Option<f32>,
    pub glassdoor_pros: Vec<String>,
    pub glassdoor_cons: Vec<String>,
    pub recent_news: Vec<NewsItem>,
    pub recent_layoffs: bool,
    
    // Metadata
    pub enriched_at: Option<DateTime<Utc>>,
    pub enrichment_source: Vec<EnrichmentSource>,
    pub is_stale: bool,   // true if enriched_at > 7 days ago
}

pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub snippet: String,
}

pub enum EmployeeCountRange {
    Tiny,       // 1–10
    Small,      // 11–50
    Medium,     // 51–200
    Large,      // 201–1000
    Enterprise, // 1000+
    Unknown,
}

pub enum FundingStage {
    PreSeed, Seed, SeriesA, SeriesB, SeriesC, SeriesD, 
    Public, Bootstrapped, Unknown,
}

pub enum EnrichmentSource {
    CompanyWebsite,
    JobDescriptionInference,
    Glassdoor,
    Crunchbase,
    LayoffsFyi,
    NewsRss,
}

// lazyjob-core/src/companies/researcher.rs

pub struct CompanyResearcher {
    llm: Arc<dyn LlmProvider>,
    http: reqwest::Client,
}

impl CompanyResearcher {
    /// Full enrichment pass for one company
    pub async fn enrich(&self, company: &CompanyRecord) -> Result<CompanyRecord>;

    /// Quick extraction from already-fetched HTML (used in tests)
    pub async fn extract_from_html(&self, html: &str) -> Result<CompanyExtractionResult>;

    /// Infer tech stack from a set of job descriptions (offline, no API calls)
    pub fn infer_tech_stack_from_jobs(&self, descriptions: &[&str]) -> Vec<String>;
}

pub struct CompanyExtractionResult {
    pub description: Option<String>,
    pub mission_statement: Option<String>,
    pub core_values: Vec<String>,
    pub tech_stack: Vec<String>,
    pub product_areas: Vec<String>,
    pub culture_signals: Vec<String>,
}

// lazyjob-core/src/companies/repository.rs

#[async_trait::async_trait]
pub trait CompanyRepository: Send + Sync {
    async fn get_by_name(&self, name_normalized: &str) -> Result<Option<CompanyRecord>>;
    async fn get_by_id(&self, id: Uuid) -> Result<Option<CompanyRecord>>;
    async fn upsert(&self, record: &CompanyRecord) -> Result<()>;
    async fn list_stale(&self, older_than_days: u32) -> Result<Vec<CompanyRecord>>;
}
```

## Open Questions

- **Glassdoor scraping legality**: Glassdoor's ToS prohibits automated data collection. Is it safer to (a) skip Glassdoor entirely, (b) offer it as a user-triggered manual step (they open the page, we parse the DOM via a bookmarklet/clipboard), or (c) use the official Glassdoor employer API (requires B2B contract)? **Recommendation**: Phase 1 skip; Phase 2 clipboard-paste approach as a power-user feature.
- **LLM cost per company enrichment**: A single company About page is ~2000-5000 tokens. At Claude Haiku rates (~$0.0008/1K tokens), enriching 50 companies = ~$0.20. Negligible. But if the user has 200 companies, and we re-run weekly, that's $16/year — fine. Confirm this is acceptable before defaulting enrichment to "always on".
- **`CompanyRecord` as the canonical company entity**: Three other specs reference company data (ghost detection needs `headcount_declining`, cover letters need `mission_statement` and `culture_signals`, interview prep needs the full record). All of them should query `CompanyRepository`, not maintain their own company data. This must be established as a hard architecture rule before implementation begins.
- **Company name normalization**: Many companies appear as "Stripe, Inc." in Greenhouse data and "stripe" in Lever data and "Stripe" in config. The `name_normalized` field must handle suffixes (Inc, LLC, Corp, Ltd), acronyms, and spacing. Should a regex normalization be sufficient, or do we need a fuzzy match step?
- **Stale data TTL by context**: "Is this company real?" (ghost detection) only needs monthly refresh. "Is this tech stack current?" (interview prep) needs weekly. "Any recent news?" (interview same-day) needs daily. A single `is_stale: bool` is too coarse — consider per-field staleness tracking.

## Implementation Tasks

- [ ] Define `CompanyRecord` struct and create `companies` table DDL and `CompanyRepository` trait in `lazyjob-core/src/companies/` — refs: `04-sqlite-persistence.md`, `company-pages.md`
- [ ] Implement `CompanyResearcher::infer_tech_stack_from_jobs()` as an offline regex-based extractor using the technical term lexicon — refs: `agentic-job-matching.md`
- [ ] Implement `CompanyResearcher::extract_from_html()` with an LLM extraction prompt targeting `CompanyExtractionResult` — refs: `08-cover-letter-generation.md`, `17-ralph-prompt-templates.md`
- [ ] Implement `CompanyResearcher::enrich()` that fetches the company website, runs extraction, merges with job-description inference, and upserts to `CompanyRepository` — refs: `05-job-discovery-layer.md`
- [ ] Wire `CompanyResearcher::enrich()` into the ralph discovery loop (triggered async after job ingestion for new companies) — refs: `06-ralph-loop-integration.md`
- [ ] Add a `CompanyView` panel to the TUI showing `CompanyRecord` fields with staleness indicators and a manual refresh keybind — refs: `09-tui-design-keybindings.md`
- [ ] Add `list_stale()` to the ralph daily refresh loop to re-enrich companies last updated > 7 days ago — refs: `06-ralph-loop-integration.md`
