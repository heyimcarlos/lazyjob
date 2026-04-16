# Implementation Plan: Cover Letter Generation

## Status
Draft

## Related Spec
[specs/08-cover-letter-generation.md](./08-cover-letter-generation.md)

## Overview

The cover letter generation subsystem gives users AI-powered, company-research-backed cover letters in seconds. Unlike generic letter generators, this pipeline first runs a company research agent (scraping the company's website, Careers page, and optional public signals) and injects that research into the generation prompt — producing letters with specific company references, culture alignment, and recent news hooks that feel genuinely researched.

The pipeline is fully async and lives in `lazyjob-core`. It composes three independent stages: (1) **company research** — HTTP scraping + LLM synthesis → `CompanyResearch`; (2) **content generation** — life sheet + job description + research → raw cover letter text; (3) **version persistence** — stores the result in SQLite tied to a `job_id`. Ralph can trigger generation as a background subprocess loop; the TUI receives progress events and surfaces the final draft in an editor panel with version history.

Version management is a first-class concern: every generated or edited draft is stored with a monotonic version number and a diff against the previous version (similar to git). The user can restore any prior version at any time and pin a specific version to their job application before submitting.

## Prerequisites

### Specs/plans that must be implemented first
- `specs/03-life-sheet-data-model.md` — provides `LifeSheet`, `WorkExperience`, `Skill` types
- `specs/04-sqlite-persistence.md` / `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, `SqlitePool`, migration infrastructure
- `specs/02-llm-provider-abstraction.md` / `specs/02-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>` trait

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
# HTML scraping
scraper       = "0.20"                          # CSS-selector HTML parser
ammonia       = "4"                             # HTML sanitization/text extraction

# Document generation
docx-rs       = "0.4"                           # DOCX generation

# Text diffing (for version diffs)
similar       = "2"                             # Diff library (similar to diff-match-patch)

# Already expected from other plans:
reqwest       = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
chrono        = { version = "0.4", features = ["serde"] }
uuid          = { version = "1", features = ["v4", "serde"] }
tokio         = { version = "1", features = ["full"] }
thiserror     = "2"
anyhow        = "1"
tracing       = "0.1"
async-trait   = "0.1"
sqlx          = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono"] }
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|---------------|
| `lazyjob-core` | All domain logic: `CoverLetterService`, `CompanyResearcher`, `CoverLetterGenerator`, `CoverLetterVersionRepository`, all core types |
| `lazyjob-llm` | No new code; the existing `Arc<dyn LlmProvider>` trait is used |
| `lazyjob-tui` | `CoverLetterEditorPanel`, `VersionHistoryWidget`, progress indicators |
| `lazyjob-ralph` | Spawns research + generation as a background loop subprocess |

`lazyjob-core` owns all state and logic. The TUI and Ralph crates are thin adapters that call into `lazyjob-core` services.

### Core Types

```rust
// lazyjob-core/src/cover_letter/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque newtype for cover letter version IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoverLetterId(pub Uuid);

impl CoverLetterId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// A single stored cover letter version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterVersion {
    pub id: CoverLetterId,
    /// FK into the jobs table.
    pub job_id: Uuid,
    /// FK into applications (nullable until application is created).
    pub application_id: Option<Uuid>,
    /// Monotonic version counter per (job_id).
    pub version: u32,
    /// Full markdown text of the letter.
    pub content: String,
    /// Plain text for ATS/copy-paste; stripped of markdown.
    pub plain_text: String,
    /// Serialized `CompanyResearch` JSON — null if research was skipped.
    pub research_json: Option<String>,
    /// First sentences from each paragraph, extracted for preview.
    pub key_points: Vec<String>,
    /// Tone used at generation time.
    pub tone: CoverLetterTone,
    /// Length target used at generation time.
    pub length: CoverLetterLength,
    /// Unified diff (similar::TextDiff) versus previous version, if any.
    pub diff_from_previous: Option<String>,
    /// Whether this version was pinned to a submitted application.
    pub is_submitted: bool,
    /// Human label set by user ("after company research", "v2 tighten opening").
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTone {
    #[default]
    Professional,
    Casual,
    Creative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterLength {
    Short,      // ~200 words
    #[default]
    Standard,   // ~300 words
    Detailed,   // ~400 words
}

impl CoverLetterLength {
    pub fn word_target(self) -> u32 {
        match self {
            Self::Short    => 200,
            Self::Standard => 300,
            Self::Detailed => 400,
        }
    }
}

/// Options passed in at generation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterOptions {
    pub tone: CoverLetterTone,
    pub length: CoverLetterLength,
    /// Skip deep company research (faster but less personalized).
    pub quick_mode: bool,
    /// User-provided custom opening sentence (overrides AI opening).
    pub custom_intro: Option<String>,
    /// If true, generate 3 variants for the user to choose from.
    pub generate_variants: bool,
}

impl Default for CoverLetterOptions {
    fn default() -> Self {
        Self {
            tone: CoverLetterTone::Professional,
            length: CoverLetterLength::Standard,
            quick_mode: false,
            custom_intro: None,
            generate_variants: false,
        }
    }
}
```

### Company Research Types

```rust
// lazyjob-core/src/cover_letter/company_research.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Synthesized company intelligence used to personalize cover letters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompanyResearch {
    pub company_name: String,
    pub mission: Option<String>,
    /// Up to 5 culture/values signals extracted from website.
    pub values: Vec<String>,
    /// Key products or services.
    pub products: Vec<String>,
    /// Recent news items ordered newest first.
    pub recent_news: Vec<NewsItem>,
    /// Words describing work culture: "fast-paced", "remote-first", etc.
    pub culture_signals: Vec<String>,
    pub team_size: Option<CompanySize>,
    /// Funding stage ("Series B", "Public", "Bootstrapped").
    pub funding_stage: Option<String>,
    /// Tech stack hints gleaned from job descriptions or engineering blog.
    pub tech_stack: Vec<String>,
    /// 2-3 specific hooks the cover letter author should mention.
    pub personalization_hooks: Vec<String>,
    /// When this research was gathered.
    pub researched_at: DateTime<Utc>,
    /// Cache TTL in hours; research is stale after this.
    pub ttl_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub date: DateTime<Utc>,
    pub url: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompanySize {
    Startup,        // 1–50
    SmallMid,       // 51–500
    MidMarket,      // 501–5000
    Enterprise,     // 5000+
    Unknown,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/cover_letter/traits.rs

use async_trait::async_trait;
use uuid::Uuid;
use crate::life_sheet::LifeSheet;
use crate::jobs::Job;
use super::types::{CoverLetterVersion, CoverLetterOptions};
use super::company_research::CompanyResearch;

#[async_trait]
pub trait CoverLetterGeneratorTrait: Send + Sync {
    /// Generate a cover letter and persist it as a new version.
    async fn generate(
        &self,
        job_id: Uuid,
        life_sheet: &LifeSheet,
        options: CoverLetterOptions,
    ) -> Result<CoverLetterVersion, CoverLetterError>;

    /// Research a company. Results are cached in SQLite.
    async fn research_company(
        &self,
        company_name: &str,
    ) -> Result<CompanyResearch, CoverLetterError>;

    /// List all versions for a job, newest first.
    async fn list_versions(
        &self,
        job_id: Uuid,
    ) -> Result<Vec<CoverLetterVersion>, CoverLetterError>;

    /// Restore a prior version as the new "current" for that job (creates a copy).
    async fn restore_version(
        &self,
        version_id: CoverLetterId,
    ) -> Result<CoverLetterVersion, CoverLetterError>;

    /// Pin a version to a submitted application.
    async fn pin_to_application(
        &self,
        version_id: CoverLetterId,
        application_id: Uuid,
    ) -> Result<(), CoverLetterError>;

    /// Export a specific version as DOCX bytes.
    async fn export_docx(
        &self,
        version_id: CoverLetterId,
    ) -> Result<Vec<u8>, CoverLetterError>;
}

#[async_trait]
pub trait CompanyResearcherTrait: Send + Sync {
    async fn research(&self, company_name: &str) -> Result<CompanyResearch, CoverLetterError>;
}
```

### SQLite Schema

Migration `009_cover_letters.sql`:

```sql
-- Cover letter versions, one-to-many per job
CREATE TABLE IF NOT EXISTS cover_letter_versions (
    id                  TEXT NOT NULL PRIMARY KEY,  -- UUID v4
    job_id              TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id      TEXT REFERENCES applications(id) ON DELETE SET NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    content             TEXT NOT NULL,              -- markdown
    plain_text          TEXT NOT NULL,              -- stripped for ATS
    research_json       TEXT,                       -- JSON blob of CompanyResearch
    key_points_json     TEXT NOT NULL DEFAULT '[]', -- JSON array of strings
    tone                TEXT NOT NULL DEFAULT 'professional',
    length              TEXT NOT NULL DEFAULT 'standard',
    diff_from_previous  TEXT,                       -- unified diff string
    is_submitted        INTEGER NOT NULL DEFAULT 0, -- boolean
    label               TEXT,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_clv_job_id      ON cover_letter_versions(job_id);
CREATE INDEX IF NOT EXISTS idx_clv_app_id      ON cover_letter_versions(application_id);
CREATE INDEX IF NOT EXISTS idx_clv_job_version ON cover_letter_versions(job_id, version DESC);

-- Cached company research; keyed by normalized company name
CREATE TABLE IF NOT EXISTS company_research_cache (
    id              TEXT NOT NULL PRIMARY KEY,    -- UUID v4
    company_name    TEXT NOT NULL,               -- normalized lowercase
    research_json   TEXT NOT NULL,               -- full CompanyResearch JSON
    researched_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ttl_hours       INTEGER NOT NULL DEFAULT 72
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_crc_name ON company_research_cache(company_name);
```

### Module Structure

```
lazyjob-core/
  src/
    cover_letter/
      mod.rs            # pub re-exports; CoverLetterService construction
      types.rs          # CoverLetterVersion, CoverLetterOptions, tone/length enums
      traits.rs         # CoverLetterGeneratorTrait, CompanyResearcherTrait
      company_research.rs  # CompanyResearch, NewsItem, CompanySize
      researcher.rs     # CompanyResearcher: HTTP scraping + LLM synthesis
      generator.rs      # CoverLetterGenerator: prompt assembly + LLM call
      service.rs        # CoverLetterService: orchestrates researcher + generator + repo
      repository.rs     # CoverLetterVersionRepository: SQLite CRUD
      docx.rs           # DOCX export via docx-rs
      error.rs          # CoverLetterError enum

lazyjob-ralph/
  src/
    loops/
      cover_letter.rs   # CoverLetterLoop: Ralph subprocess handler

lazyjob-tui/
  src/
    panels/
      cover_letter/
        mod.rs
        editor.rs       # CoverLetterEditorPanel: text view + edit mode
        version_list.rs # VersionHistoryWidget: scrollable version list
        diff_view.rs    # DiffViewWidget: show unified diff between two versions
```

---

## Implementation Phases

### Phase 1 — Core Domain + Generation MVP

#### Step 1.1 — Error type (`lazyjob-core/src/cover_letter/error.rs`)

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoverLetterError {
    #[error("LLM call failed: {0}")]
    LlmFailed(#[from] crate::llm::LlmError),

    #[error("HTTP request failed: {0}")]
    HttpFailed(#[from] reqwest::Error),

    #[error("HTML scraping produced empty content for {url}")]
    EmptyScrapedContent { url: String },

    #[error("Failed to parse company research from LLM response: {0}")]
    ResearchParseFailed(#[from] serde_json::Error),

    #[error("Job not found: {0}")]
    JobNotFound(uuid::Uuid),

    #[error("Cover letter version not found: {0}")]
    VersionNotFound(uuid::Uuid),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("DOCX generation failed: {0}")]
    DocxFailed(String),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoverLetterError>;
```

**Verification**: `cargo check -p lazyjob-core` compiles with zero errors.

---

#### Step 1.2 — Company Researcher (`lazyjob-core/src/cover_letter/researcher.rs`)

The researcher scrapes the company's website and careers page, then passes the raw text to the LLM for structured extraction.

```rust
use std::sync::Arc;
use anyhow::Context as _;
use scraper::{Html, Selector};
use crate::llm::LlmProvider;
use super::{company_research::CompanyResearch, error::Result};

pub struct CompanyResearcher {
    http: reqwest::Client,
    llm: Arc<dyn LlmProvider>,
}

impl CompanyResearcher {
    pub fn new(http: reqwest::Client, llm: Arc<dyn LlmProvider>) -> Self {
        Self { http, llm }
    }

    pub async fn research(&self, company_name: &str) -> Result<CompanyResearch> {
        // 1. Attempt a DuckDuckGo-style search to find the company URL
        //    (or derive it heuristically from the name).
        let base_url = self.infer_company_url(company_name).await;

        // 2. Scrape about + careers pages concurrently.
        let (about_text, careers_text) = if let Some(url) = &base_url {
            tokio::join!(
                self.scrape_page(&format!("{url}/about")),
                self.scrape_page(&format!("{url}/careers")),
            )
        } else {
            (Ok(String::new()), Ok(String::new()))
        };

        let about_text   = about_text.unwrap_or_default();
        let careers_text = careers_text.unwrap_or_default();

        // 3. Synthesize with LLM.
        self.synthesize(company_name, &about_text, &careers_text).await
    }

    async fn scrape_page(&self, url: &str) -> Result<String> {
        let html = self.http
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (LazyJob; research bot)")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .text()
            .await?;

        // Extract visible text from <p>, <li>, <h1>–<h3> tags.
        let doc = Html::parse_document(&html);
        let sel = Selector::parse("p, li, h1, h2, h3, h4").unwrap();
        let text: String = doc
            .select(&sel)
            .map(|el| el.text().collect::<String>())
            .filter(|t| t.trim().len() > 20)
            .collect::<Vec<_>>()
            .join(" ");

        Ok(ammonia::clean_text(&text))
    }

    async fn synthesize(
        &self,
        company_name: &str,
        about: &str,
        careers: &str,
    ) -> Result<CompanyResearch> {
        let prompt = format!(
            r#"You are a business analyst. Extract structured company intelligence for personalizing job application cover letters.

Company name: {company_name}

About page text (truncated to 3000 chars):
{about}

Careers page text (truncated to 2000 chars):
{careers}

Return ONLY valid JSON matching this schema:
{{
  "mission": "<string or null>",
  "values": ["<string>", ...],         // up to 5
  "products": ["<string>", ...],       // up to 3 key products/services
  "culture_signals": ["<string>", ...], // adjectives/phrases: "fast-paced", "remote-first"
  "tech_stack": ["<string>", ...],     // inferred technology hints
  "personalization_hooks": ["<string>", ...] // 2-3 SPECIFIC things worth mentioning in a cover letter
}}
"#,
            company_name = company_name,
            about = about.chars().take(3000).collect::<String>(),
            careers = careers.chars().take(2000).collect::<String>(),
        );

        let raw = self.llm.complete(&prompt).await
            .context("LLM synthesis failed")?;

        #[derive(serde::Deserialize)]
        struct LlmOutput {
            mission: Option<String>,
            values: Vec<String>,
            products: Vec<String>,
            culture_signals: Vec<String>,
            tech_stack: Vec<String>,
            personalization_hooks: Vec<String>,
        }

        // Strip markdown code fences if present.
        let json_str = raw.trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: LlmOutput = serde_json::from_str(json_str)?;

        Ok(CompanyResearch {
            company_name: company_name.to_string(),
            mission: parsed.mission,
            values: parsed.values,
            products: parsed.products,
            recent_news: vec![],
            culture_signals: parsed.culture_signals,
            team_size: None,
            funding_stage: None,
            tech_stack: parsed.tech_stack,
            personalization_hooks: parsed.personalization_hooks,
            researched_at: chrono::Utc::now(),
            ttl_hours: 72,
        })
    }

    /// Heuristic: derive a likely base URL from company name.
    /// e.g. "Stripe" → "https://stripe.com"
    async fn infer_company_url(&self, company_name: &str) -> Option<String> {
        let slug = company_name
            .to_lowercase()
            .replace(' ', "");
        let candidate = format!("https://{slug}.com");
        // HEAD request to verify it resolves.
        let ok = self.http
            .head(&candidate)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);
        if ok { Some(candidate) } else { None }
    }
}
```

**Verification**: Unit test that passes mock HTML returns a populated `CompanyResearch` with `mission` set.

---

#### Step 1.3 — Cover Letter Generator (`lazyjob-core/src/cover_letter/generator.rs`)

```rust
use std::sync::Arc;
use anyhow::Context as _;
use crate::llm::LlmProvider;
use crate::life_sheet::{LifeSheet, WorkExperience};
use crate::jobs::Job;
use super::{
    company_research::CompanyResearch,
    types::{CoverLetterOptions, CoverLetterTone, CoverLetterLength},
    error::Result,
};

pub struct CoverLetterGenerator {
    llm: Arc<dyn LlmProvider>,
}

impl CoverLetterGenerator {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self { Self { llm } }

    pub async fn generate(
        &self,
        job: &Job,
        life_sheet: &LifeSheet,
        research: &CompanyResearch,
        options: &CoverLetterOptions,
    ) -> Result<String> {
        let tone_desc = match options.tone {
            CoverLetterTone::Professional => "professional and confident",
            CoverLetterTone::Casual       => "warm, conversational, and direct",
            CoverLetterTone::Creative     => "creative, memorable, and slightly unconventional",
        };

        let top_experience = self.format_relevant_experience(life_sheet, job);
        let hooks = research.personalization_hooks.join("\n- ");
        let culture = research.culture_signals.join(", ");
        let product_hook = research.products.first().cloned().unwrap_or_default();
        let custom_intro = options.custom_intro
            .as_deref()
            .map(|s| format!("Use this as the opening sentence: \"{s}\""))
            .unwrap_or_default();

        let prompt = format!(
            r#"Write a cover letter for this job application. Follow the instructions exactly.

## Role
Company: {company}
Title: {title}
Job description (first 600 chars):
{jd}

## Company Intelligence
Mission: {mission}
Culture: {culture}
Key product/service: {product_hook}
Personalization hooks:
- {hooks}

## Candidate background
{experience}

## Instructions
- Tone: {tone_desc}
- Target length: ~{word_target} words
- Structure: hook opening → company-specific paragraph → 1-2 achievement paragraphs → call to action
- Use concrete metrics wherever the candidate background provides them
- Mention at least ONE specific item from the personalization hooks
- Do NOT use any of these clichés: "I am writing to express my interest", "passionate about", "synergy", "leverage", "team player", "detail-oriented"
- Do NOT include the header/address block — body only
- Return ONLY the letter body text, no formatting labels
{custom_intro}
"#,
            company      = &job.company_name,
            title        = &job.title,
            jd           = job.description.chars().take(600).collect::<String>(),
            mission      = research.mission.as_deref().unwrap_or("not stated"),
            culture      = culture,
            product_hook = product_hook,
            hooks        = hooks,
            experience   = top_experience,
            tone_desc    = tone_desc,
            word_target  = options.length.word_target(),
            custom_intro = custom_intro,
        );

        self.llm.complete(&prompt).await.context("LLM cover letter generation failed")
    }

    /// Extract the 2-3 most relevant work experience entries from the life sheet
    /// relative to the job's description. Uses simple keyword overlap scoring.
    fn format_relevant_experience(&self, life_sheet: &LifeSheet, job: &Job) -> String {
        let jd_lower = job.description.to_lowercase();
        let jd_words: std::collections::HashSet<&str> = jd_lower.split_whitespace().collect();

        let mut scored: Vec<(&WorkExperience, usize)> = life_sheet
            .work_experience
            .iter()
            .map(|exp| {
                let text = format!(
                    "{} {} {}",
                    exp.title,
                    exp.company,
                    exp.achievements.join(" "),
                ).to_lowercase();
                let score = text
                    .split_whitespace()
                    .filter(|w| jd_words.contains(*w))
                    .count();
                (exp, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));

        scored
            .iter()
            .take(3)
            .map(|(exp, _)| {
                let top_achievement = exp.achievements.first().cloned().unwrap_or_default();
                format!(
                    "- {} at {} ({}–{}): {}",
                    exp.title,
                    exp.company,
                    exp.start_date.format("%Y"),
                    exp.end_date.map(|d| d.format("%Y").to_string()).unwrap_or_else(|| "present".to_string()),
                    top_achievement,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract first sentence of each paragraph as key points for preview.
    pub fn extract_key_points(&self, content: &str) -> Vec<String> {
        content
            .split("\n\n")
            .take(4)
            .filter_map(|p| {
                p.lines()
                    .find(|l| l.trim().len() > 20)
                    .map(|l| l.trim().to_string())
            })
            .collect()
    }

    /// Strip markdown formatting to produce ATS-friendly plain text.
    pub fn to_plain_text(&self, markdown: &str) -> String {
        markdown
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.trim_start_matches(&['*', '_', '`'][..]))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
```

**Verification**: Integration test with a mock `LlmProvider` that returns a canned letter confirms the generator constructs the prompt correctly and maps the result into `key_points`.

---

#### Step 1.4 — Repository (`lazyjob-core/src/cover_letter/repository.rs`)

```rust
use sqlx::SqlitePool;
use uuid::Uuid;
use super::{
    types::{CoverLetterId, CoverLetterVersion},
    error::Result,
};

pub struct CoverLetterVersionRepository {
    pool: SqlitePool,
}

impl CoverLetterVersionRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn next_version_number(&self, job_id: Uuid) -> Result<u32> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) FROM cover_letter_versions WHERE job_id = ?"
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(n,)| n as u32 + 1).unwrap_or(1))
    }

    pub async fn save(&self, version: &CoverLetterVersion) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO cover_letter_versions
                (id, job_id, application_id, version, content, plain_text,
                 research_json, key_points_json, tone, length,
                 diff_from_previous, is_submitted, label, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            version.id.0.to_string(),
            version.job_id.to_string(),
            version.application_id.map(|u| u.to_string()),
            version.version,
            version.content,
            version.plain_text,
            version.research_json,
            serde_json::to_string(&version.key_points).unwrap(),
            format!("{:?}", version.tone).to_lowercase(),
            format!("{:?}", version.length).to_lowercase(),
            version.diff_from_previous,
            version.is_submitted as i64,
            version.label,
            version.created_at.to_rfc3339(),
            version.updated_at.to_rfc3339(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_for_job(&self, job_id: Uuid) -> Result<Vec<CoverLetterVersion>> {
        // Fetch rows ordered newest first; deserialize JSON fields.
        let rows = sqlx::query!(
            "SELECT * FROM cover_letter_versions WHERE job_id = ? ORDER BY version DESC",
            job_id.to_string(),
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|r| Self::row_to_version(r))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub async fn get(&self, id: CoverLetterId) -> Result<Option<CoverLetterVersion>> {
        let row = sqlx::query!(
            "SELECT * FROM cover_letter_versions WHERE id = ?",
            id.0.to_string(),
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(Self::row_to_version).transpose().map_err(Into::into)
    }

    pub async fn pin_to_application(
        &self,
        id: CoverLetterId,
        application_id: Uuid,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE cover_letter_versions SET application_id = ?, is_submitted = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            application_id.to_string(),
            id.0.to_string(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn row_to_version(
        r: /* sqlx query result row */ impl /* row type */ std::any::Any,
    ) -> anyhow::Result<CoverLetterVersion> {
        // Implemented by extracting each field from the sqlx row.
        // (Exact sqlx macro types abbreviated here for readability.)
        todo!("map sqlx record fields → CoverLetterVersion")
    }
}
```

**Note**: The `row_to_version` function maps the raw sqlx record to the domain type, parsing UUIDs, JSON fields, and chrono datetimes from strings.

**Verification**: Integration test against an in-memory SQLite (`SqlitePool::connect(":memory:")`) — save a version, list it, confirm fields round-trip correctly.

---

#### Step 1.5 — Service (`lazyjob-core/src/cover_letter/service.rs`)

The `CoverLetterService` wires together `CompanyResearcher`, `CoverLetterGenerator`, and `CoverLetterVersionRepository`. It also manages the company research cache in SQLite.

```rust
use std::sync::Arc;
use similar::{ChangeTag, TextDiff};
use uuid::Uuid;
use crate::{jobs::JobRepository, life_sheet::LifeSheetRepository, llm::LlmProvider};
use super::{
    company_research::CompanyResearch,
    error::{CoverLetterError, Result},
    generator::CoverLetterGenerator,
    repository::CoverLetterVersionRepository,
    researcher::CompanyResearcher,
    types::{CoverLetterId, CoverLetterOptions, CoverLetterVersion},
};

pub struct CoverLetterService {
    researcher: CompanyResearcher,
    generator: CoverLetterGenerator,
    repo: CoverLetterVersionRepository,
    jobs: Arc<JobRepository>,
    life_sheet: Arc<LifeSheetRepository>,
}

impl CoverLetterService {
    pub fn new(
        pool: sqlx::SqlitePool,
        llm: Arc<dyn LlmProvider>,
        jobs: Arc<JobRepository>,
        life_sheet: Arc<LifeSheetRepository>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client");
        Self {
            researcher: CompanyResearcher::new(http, Arc::clone(&llm)),
            generator:  CoverLetterGenerator::new(Arc::clone(&llm)),
            repo:       CoverLetterVersionRepository::new(pool.clone()),
            jobs,
            life_sheet,
        }
    }

    pub async fn generate_for_job(
        &self,
        job_id: Uuid,
        options: CoverLetterOptions,
    ) -> Result<CoverLetterVersion> {
        let job = self.jobs.get(job_id).await
            .map_err(|_| CoverLetterError::JobNotFound(job_id))?
            .ok_or(CoverLetterError::JobNotFound(job_id))?;

        let life_sheet = self.life_sheet.get().await
            .context("Failed to load life sheet")?;

        // Research company (with cache check).
        let research = if options.quick_mode {
            CompanyResearch {
                company_name: job.company_name.clone(),
                ..Default::default()
            }
        } else {
            self.research_with_cache(&job.company_name).await?
        };

        // Generate letter text.
        let content = self.generator.generate(&job, &life_sheet, &research, &options).await?;
        let plain_text = self.generator.to_plain_text(&content);
        let key_points = self.generator.extract_key_points(&content);

        // Compute diff from previous version if one exists.
        let prev_versions = self.repo.list_for_job(job_id).await?;
        let diff = prev_versions.first().map(|prev| {
            let diff = TextDiff::from_lines(&prev.content, &content);
            diff.unified_diff()
                .context_radius(3)
                .header("previous", "current")
                .to_string()
        });

        let version_number = prev_versions
            .first()
            .map(|v| v.version + 1)
            .unwrap_or(1);

        let version = CoverLetterVersion {
            id:                   CoverLetterId::new(),
            job_id,
            application_id:       None,
            version:              version_number,
            content,
            plain_text,
            research_json:        Some(serde_json::to_string(&research)?),
            key_points,
            tone:                 options.tone,
            length:               options.length,
            diff_from_previous:   diff,
            is_submitted:         false,
            label:                None,
            created_at:           chrono::Utc::now(),
            updated_at:           chrono::Utc::now(),
        };

        self.repo.save(&version).await?;
        Ok(version)
    }

    async fn research_with_cache(&self, company_name: &str) -> Result<CompanyResearch> {
        let key = company_name.to_lowercase();

        // Check cache table.
        let cached: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT research_json, researched_at, ttl_hours FROM company_research_cache WHERE company_name = ?"
        )
        .bind(&key)
        .fetch_optional(/* pool */)
        .await
        .ok()
        .flatten();

        if let Some((json, researched_at_str, ttl)) = cached {
            let researched_at = chrono::DateTime::parse_from_rfc3339(&researched_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_default();
            let age_hours = (chrono::Utc::now() - researched_at).num_hours();
            if age_hours < ttl {
                if let Ok(r) = serde_json::from_str::<CompanyResearch>(&json) {
                    return Ok(r);
                }
            }
        }

        // Cache miss or stale — run research.
        let research = self.researcher.research(company_name).await?;

        // Upsert cache.
        let json = serde_json::to_string(&research)?;
        sqlx::query!(
            "INSERT INTO company_research_cache (id, company_name, research_json, ttl_hours)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(company_name) DO UPDATE SET research_json = excluded.research_json, researched_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            uuid::Uuid::new_v4().to_string(),
            key,
            json,
            research.ttl_hours,
        )
        .execute(/* pool */)
        .await
        .ok(); // Cache write failure is non-fatal.

        Ok(research)
    }

    pub async fn list_versions(&self, job_id: Uuid) -> Result<Vec<CoverLetterVersion>> {
        self.repo.list_for_job(job_id).await
    }

    pub async fn restore_version(&self, version_id: CoverLetterId) -> Result<CoverLetterVersion> {
        let original = self.repo
            .get(version_id)
            .await?
            .ok_or(CoverLetterError::VersionNotFound(version_id.0))?;

        // Create a new version copying content from the original.
        let latest = self.repo.list_for_job(original.job_id).await?;
        let new_version_number = latest.first().map(|v| v.version + 1).unwrap_or(1);

        let restored = CoverLetterVersion {
            id: CoverLetterId::new(),
            version: new_version_number,
            label: Some(format!("Restored from v{}", original.version)),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..original
        };

        self.repo.save(&restored).await?;
        Ok(restored)
    }
}
```

**Verification**: End-to-end test: create a mock job + life sheet → generate → assert `version == 1`. Generate again → assert `version == 2` and `diff_from_previous.is_some()`.

---

### Phase 2 — DOCX Export + Ralph Loop

#### Step 2.1 — DOCX Export (`lazyjob-core/src/cover_letter/docx.rs`)

```rust
use docx_rs::{Docx, Paragraph, Run, Tab};
use super::types::CoverLetterVersion;
use super::error::Result;
use crate::life_sheet::PersonalInfo;

pub fn export_docx(version: &CoverLetterVersion, personal: &PersonalInfo) -> Result<Vec<u8>> {
    let header_line = format!(
        "{} | {} | {}",
        personal.name,
        personal.email,
        personal.location.as_deref().unwrap_or(""),
    );

    let mut doc = Docx::new();

    // Header: name + contact
    doc = doc.add_paragraph(
        Paragraph::new().add_run(
            Run::new()
                .add_text(&header_line)
                .bold()
                .size(22), // 11pt
        )
    );

    // Date
    doc = doc.add_paragraph(
        Paragraph::new().add_run(
            Run::new()
                .add_text(&chrono::Utc::now().format("%B %d, %Y").to_string())
                .size(22),
        )
    );

    // Spacer
    doc = doc.add_paragraph(Paragraph::new());

    // Letter body — one paragraph per double-newline block
    for para in version.plain_text.split("\n\n") {
        let trimmed = para.trim();
        if !trimmed.is_empty() {
            doc = doc.add_paragraph(
                Paragraph::new().add_run(
                    Run::new().add_text(trimmed).size(22),
                )
            );
        }
    }

    let mut buf = Vec::new();
    doc.build()
        .pack(&mut buf)
        .map_err(|e| super::error::CoverLetterError::DocxFailed(e.to_string()))?;
    Ok(buf)
}
```

**Key APIs**:
- `docx_rs::Docx::new()` → builder
- `docx_rs::Paragraph::new().add_run(Run::new().add_text(s).size(22))` for body text
- `Docx::build().pack(&mut Vec<u8>)` to serialize

**Verification**: Export a version, write to `/tmp/test.docx`, open in LibreOffice and confirm formatting.

---

#### Step 2.2 — Ralph Loop (`lazyjob-ralph/src/loops/cover_letter.rs`)

Ralph runs the cover letter generation as a child subprocess. Communication follows the existing JSON-over-stdio protocol.

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{LoopContext, RalphResult};

#[derive(Debug, Deserialize)]
pub struct CoverLetterRequest {
    pub job_id: Uuid,
    pub tone: String,
    pub length: String,
    pub quick_mode: bool,
    pub custom_intro: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CoverLetterResult {
    pub version_id: Uuid,
    pub version_number: u32,
    pub key_points: Vec<String>,
    pub word_count: usize,
}

pub struct CoverLetterLoop {
    ctx: LoopContext,
}

impl CoverLetterLoop {
    pub async fn run(&self, req: CoverLetterRequest) -> RalphResult<()> {
        self.ctx.send_progress(0.05, "Loading job and life sheet").await?;

        let options = build_options(&req);

        self.ctx.send_progress(0.15, "Researching company").await?;

        let version = self.ctx
            .cover_letter_service()
            .generate_for_job(req.job_id, options)
            .await?;

        self.ctx.send_progress(0.90, "Persisting version").await?;

        let result = CoverLetterResult {
            version_id: version.id.0,
            version_number: version.version,
            key_points: version.key_points.clone(),
            word_count: version.plain_text.split_whitespace().count(),
        };

        self.ctx.send_result(serde_json::to_value(result)?).await?;
        self.ctx.send_done().await
    }
}
```

**Progress events** are emitted at 5%, 15%, 50% (company research done), 80% (generation done), 90% (saving), 100%.

**Verification**: Run `cargo test -p lazyjob-ralph cover_letter` — assert the loop handles a mock service and emits well-formed JSON events on stdout.

---

### Phase 3 — TUI Integration

#### Step 3.1 — Cover Letter Editor Panel (`lazyjob-tui/src/panels/cover_letter/editor.rs`)

The editor panel is a full-screen modal that shows:
- Current cover letter text (scrollable, read-only by default)
- Status bar: version number, word count, tone, length
- Keybinds: `e` — open $EDITOR for manual editing; `g` — re-generate; `d` — toggle diff view; `h` — version history; `x` — export DOCX

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarState, Wrap},
    Frame,
};
use crate::state::AppState;

pub struct CoverLetterEditorPanel {
    scroll_offset: u16,
    show_diff: bool,
}

impl CoverLetterEditorPanel {
    pub fn render(&mut self, frame: &mut Frame, state: &AppState) {
        let area = frame.size();

        // Split: letter content (main) + status bar (1 line)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        // Render letter body or diff.
        if let Some(version) = state.current_cover_letter() {
            let content = if self.show_diff {
                version.diff_from_previous.as_deref().unwrap_or("(no previous version)")
            } else {
                &version.content
            };

            let paragraph = Paragraph::new(Text::raw(content))
                .block(Block::default()
                    .title(format!(" Cover Letter v{} ", version.version))
                    .borders(Borders::ALL))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll_offset, 0));

            frame.render_widget(paragraph, chunks[0]);

            // Status bar
            let status = Line::from(vec![
                Span::styled(" e:edit ", Style::default().fg(Color::Yellow)),
                Span::raw("| "),
                Span::styled("g:regenerate ", Style::default().fg(Color::Green)),
                Span::raw("| "),
                Span::styled("d:diff ", Style::default().fg(Color::Cyan)),
                Span::raw("| "),
                Span::styled("h:history ", Style::default().fg(Color::Magenta)),
                Span::raw("| "),
                Span::styled("x:export DOCX", Style::default().fg(Color::Blue)),
                Span::raw(format!(
                    "  [{} words | {} | {}]",
                    version.plain_text.split_whitespace().count(),
                    format!("{:?}", version.tone).to_lowercase(),
                    format!("{:?}", version.length).to_lowercase(),
                )),
            ]);
            frame.render_widget(
                Paragraph::new(status).style(Style::default().bg(Color::DarkGray)),
                chunks[1],
            );
        } else {
            let placeholder = Paragraph::new("No cover letter yet. Press 'g' to generate.")
                .block(Block::default().title(" Cover Letter ").borders(Borders::ALL));
            frame.render_widget(placeholder, chunks[0]);
        }
    }

    pub fn scroll_down(&mut self) { self.scroll_offset = self.scroll_offset.saturating_add(3); }
    pub fn scroll_up(&mut self)   { self.scroll_offset = self.scroll_offset.saturating_sub(3); }
    pub fn toggle_diff(&mut self) { self.show_diff = !self.show_diff; }
}
```

**Verification**: Render snapshot test using `ratatui::backend::TestBackend` — assert the panel renders a block titled "Cover Letter v1" when a version is loaded into state.

---

#### Step 3.2 — Version History Widget (`lazyjob-tui/src/panels/cover_letter/version_list.rs`)

A scrollable list showing all versions for the selected job. Each row:
```
v3  2026-04-15  professional / standard  [submitted]  "After company research"
v2  2026-04-14  casual / short
v1  2026-04-13  professional / standard
```

```rust
use ratatui::widgets::{List, ListItem, ListState};
use crate::state::AppState;

pub struct VersionHistoryWidget {
    pub list_state: ListState,
}

impl VersionHistoryWidget {
    pub fn render(&mut self, frame: &mut ratatui::Frame, state: &AppState, area: ratatui::layout::Rect) {
        let versions = state.cover_letter_versions();
        let items: Vec<ListItem> = versions
            .iter()
            .map(|v| {
                let label = v.label.as_deref().unwrap_or("");
                let submitted = if v.is_submitted { " [submitted]" } else { "" };
                let text = format!(
                    "v{:>2}  {}  {} / {}{}  {}",
                    v.version,
                    v.created_at.format("%Y-%m-%d"),
                    format!("{:?}", v.tone).to_lowercase(),
                    format!("{:?}", v.length).to_lowercase(),
                    submitted,
                    label,
                );
                ListItem::new(text)
            })
            .collect();

        let list = List::new(items)
            .block(ratatui::widgets::Block::default()
                .title(" Version History ")
                .borders(ratatui::widgets::Borders::ALL))
            .highlight_style(ratatui::style::Style::default()
                .fg(ratatui::style::Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD));

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    pub fn select_next(&mut self, total: usize) {
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some((i + 1).min(total.saturating_sub(1))));
    }

    pub fn select_prev(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(1)));
    }
}
```

**Verification**: Render snapshot with 3 mock versions → assert all 3 rows appear and selected row is highlighted.

---

## Key Crate APIs

| Crate | API | Usage |
|-------|-----|-------|
| `scraper` | `Html::parse_document(&str)`, `Selector::parse("p, li, h1")`, `doc.select(&sel)`, `el.text().collect::<String>()` | Parse company page HTML |
| `ammonia` | `ammonia::clean_text(&str) → String` | Strip unsafe HTML, get readable text |
| `reqwest` | `Client::get(url).timeout(Duration).send().await?.text().await?` | HTTP scraping |
| `similar` | `TextDiff::from_lines(old, new)`, `.unified_diff().context_radius(3).to_string()` | Version diff computation |
| `docx_rs` | `Docx::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text(s)))`, `.build().pack(&mut Vec<u8>)` | DOCX export |
| `sqlx` | `sqlx::query!("...", bind).execute(&pool)`, `query_as::<_, Row>(...)`.fetch_all` | SQLite persistence |
| `ratatui` | `Paragraph`, `List`, `ListState`, `Block`, `Scrollbar` | TUI rendering |
| `serde_json` | `to_string(&v)`, `from_str::<T>(s)` | JSON serialization for DB blob fields |
| `chrono` | `Utc::now()`, `DateTime::parse_from_rfc3339`, `.num_hours()` | TTL and timestamps |
| `uuid` | `Uuid::new_v4()`, `.to_string()` | ID generation |

---

## Error Handling

```rust
// lazyjob-core/src/cover_letter/error.rs

#[derive(Debug, thiserror::Error)]
pub enum CoverLetterError {
    #[error("LLM call failed: {0}")]
    LlmFailed(#[from] crate::llm::LlmError),

    #[error("HTTP request failed: {0}")]
    HttpFailed(#[from] reqwest::Error),

    #[error("Empty scraped content at {url}")]
    EmptyScrapedContent { url: String },

    #[error("Failed to parse LLM response as company research JSON: {0}")]
    ResearchParseFailed(#[from] serde_json::Error),

    #[error("Job not found: {0}")]
    JobNotFound(uuid::Uuid),

    #[error("Cover letter version not found: {0}")]
    VersionNotFound(uuid::Uuid),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("DOCX export failed: {0}")]
    DocxFailed(String),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, CoverLetterError>;
```

**Graceful degradation rules:**
- Company research HTTP failure → fall back to `CompanyResearch::default()` (empty), mark `quick_mode` in version metadata, emit `tracing::warn!`.
- LLM JSON parse failure → retry once with stricter prompt instruction; if still fails, surface `ResearchParseFailed` to the TUI for user notification.
- Company URL inference failure → no URL found, skip scraping, continue with empty context.

---

## Testing Strategy

### Unit Tests

**`researcher.rs`**
```rust
#[tokio::test]
async fn test_synthesize_extracts_mission() {
    let mock_llm = MockLlmProvider::returning(r#"{"mission":"Build the future","values":["bold"],"products":["Acme SDK"],"culture_signals":["fast-paced"],"tech_stack":["Rust"],"personalization_hooks":["engineering blog on memory safety"]}"#);
    let researcher = CompanyResearcher::new(reqwest::Client::new(), Arc::new(mock_llm));
    let result = researcher.synthesize("Acme", "about page text", "careers text").await.unwrap();
    assert_eq!(result.mission.as_deref(), Some("Build the future"));
    assert_eq!(result.products, vec!["Acme SDK"]);
}
```

**`generator.rs`**
```rust
#[tokio::test]
async fn test_generate_injects_personalization_hook() {
    let mock_llm = MockLlmProvider::capturing_prompts();
    let gen = CoverLetterGenerator::new(Arc::new(mock_llm.clone()));
    let research = CompanyResearch {
        personalization_hooks: vec!["engineering blog on Rust async".to_string()],
        ..Default::default()
    };
    gen.generate(&mock_job(), &mock_life_sheet(), &research, &Default::default()).await.unwrap();
    let prompt = mock_llm.last_prompt();
    assert!(prompt.contains("engineering blog on Rust async"));
}
```

**`generator.rs` — extract_key_points**
```rust
#[test]
fn test_extract_key_points_returns_first_sentences() {
    let content = "First sentence of para one.\n\nSecond para starts here.\n\nThird para.";
    let gen = CoverLetterGenerator::new(/* unused */ todo!());
    let points = gen.extract_key_points(content);
    assert_eq!(points[0], "First sentence of para one.");
    assert_eq!(points[1], "Second para starts here.");
}
```

**`repository.rs`**
```rust
#[sqlx::test]
async fn test_save_and_list_versions(pool: SqlitePool) {
    let repo = CoverLetterVersionRepository::new(pool);
    let v1 = make_version(job_id, 1);
    repo.save(&v1).await.unwrap();
    let v2 = make_version(job_id, 2);
    repo.save(&v2).await.unwrap();
    let versions = repo.list_for_job(job_id).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 2); // newest first
}
```

### Integration Tests

**`service.rs`**
```rust
#[sqlx::test]
async fn test_generate_creates_version_with_diff(pool: SqlitePool) {
    let svc = build_test_service(pool);
    let v1 = svc.generate_for_job(JOB_ID, Default::default()).await.unwrap();
    assert_eq!(v1.version, 1);
    assert!(v1.diff_from_previous.is_none());
    let v2 = svc.generate_for_job(JOB_ID, Default::default()).await.unwrap();
    assert_eq!(v2.version, 2);
    assert!(v2.diff_from_previous.is_some());
}
```

**Company research cache**
```rust
#[sqlx::test]
async fn test_research_cache_avoids_second_http_call(pool: SqlitePool) {
    let (svc, http_call_count) = build_test_service_with_call_counter(pool);
    svc.generate_for_job(JOB_ID, Default::default()).await.unwrap();
    svc.generate_for_job(JOB_ID, Default::default()).await.unwrap();
    assert_eq!(http_call_count.load(Ordering::SeqCst), 1); // cached on second call
}
```

### TUI Tests

```rust
#[test]
fn test_editor_panel_renders_version_number() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::with_cover_letter(make_version(1));
    let mut panel = CoverLetterEditorPanel::default();
    terminal.draw(|f| panel.render(f, &state)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Cover Letter v1"));
}
```

---

## Open Questions

1. **ATS scanning compatibility**: Should the generated letter avoid markdown entirely in `content`, or is markdown acceptable since `plain_text` is always computed? Current decision: keep `content` as plain text (no markdown) to keep the diff view clean; the `plain_text` field becomes redundant and can be removed in a later cleanup pass.

2. **"No cover letter needed" detection**: The spec asks whether we should skip generation if the job description says "no cover letter required". This can be implemented as a lightweight regex check in `CoverLetterService::generate_for_job` before invoking the pipeline. Not blocking for Phase 1.

3. **A/B variants**: The `generate_variants: bool` option is scaffolded but not implemented. When implemented, the generator would make 3 parallel LLM calls with different tone prompts and return all 3 as separate versions for the user to choose from. Deferred to Phase 3.

4. **Company URL database**: The current heuristic (`{slug}.com`) fails for many companies (e.g. "Stripe" → stripe.com is correct, but "JP Morgan Chase" → jpmorgan.com is ambiguous). A curated data source or DuckDuckGo Instant Answer API integration would improve coverage. Track as a follow-up issue.

5. **News integration**: `recent_news: Vec<NewsItem>` is in the schema but the researcher doesn't yet fetch news. Phase 2 or 3 extension: integrate with a free RSS search endpoint or a web scraper for TechCrunch headlines.

6. **Edit in $EDITOR**: Phase 2 work. When the user presses `e` in the TUI, LazyJob should write `plain_text` to a temp file, `exec` $EDITOR, and on return diff the file and save the changes as a new version.

---

## Related Specs

- `specs/07-resume-tailoring-pipeline.md` — companion pipeline; cover letter should reference tailored resume bullet points when available
- `specs/03-life-sheet-data-model.md` — source of candidate background data
- `specs/04-sqlite-persistence.md` — migration infrastructure
- `specs/09-tui-design-keybindings.md` — keybinding conventions for editor panel
- `specs/12-15-interview-salary-networking-notifications.md` — company research data also used for interview prep
- `specs/XX-cover-letter-version-management.md` — deeper version management (restore, export, diff browser)
