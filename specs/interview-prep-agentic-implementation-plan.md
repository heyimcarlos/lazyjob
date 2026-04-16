# Implementation Plan: Agentic Interview Preparation

## Status
Draft

## Related Spec
[specs/interview-prep-agentic.md](./interview-prep-agentic.md)

## Overview

The agentic interview prep subsystem is a `lazyjob-ralph` autonomous research loop that assembles a complete, personalized interview dossier for a specific application. When triggered, it fans out across multiple research phases in sequence: company public web research, team/culture intelligence, product analysis, recent news aggregation, and job-description question prediction. It synthesizes these inputs — together with the candidate's `LifeSheet` and the target `InterviewPrepSession` question bank — into a structured `PrepDossier` persisted to SQLite and surfaced in the TUI.

The STAR story bank extraction pipeline is a separate, lighter-weight loop (`LoopType::StarBankExtraction`) that runs once after a LifeSheet import. It reads all experience entries and achievement bullets, clusters them by behavioral dimension (leadership, conflict, failure, impact, ambiguity, cross-team influence, technical complexity), scores each story's STAR completeness, and writes `StarStory` rows to SQLite. The story bank is then consumed by the question generation plan and the mock interview loop's answer scaffold.

Progress tracking ties both flows together: a `PrepProgressService` queries `prep_sessions`, `mock_interview_sessions`, `star_stories`, and `dossier_sections` to compute per-topic and per-company readiness scores, surfacing them in a `PrepDashboard` TUI view. Scheduling integration via `NotificationScheduler` fires a "prep checkpoint" reminder 48 hours before any interview event recorded in the `interviews` table.

## Prerequisites

### Specs/Plans that must precede this
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `WorkExperience`, `Achievement`, `LifeSheetRepository`
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, migration runner, `sqlx::Pool<Sqlite>`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `Arc<dyn LlmProvider>`, `ChatMessage`, `CompletionRequest`, streaming
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — `WorkerCommand`, `WorkerEvent`, NDJSON codec, `RalphProcessManager`, `CancelToken`
- `specs/agentic-ralph-orchestration-implementation-plan.md` — `LoopType`, `LoopQueue`, `LoopDispatch`, concurrency limits
- `specs/interview-prep-question-generation-implementation-plan.md` — `InterviewQuestion`, `InterviewPrepSession`, `PrepContextBuilder`
- `specs/interview-prep-mock-loop-implementation-plan.md` — `MockInterviewSession`, `MockResponse`
- `specs/job-search-company-research-implementation-plan.md` — `CompanyRecord`, `CompanyRepository`, `CompanyService`
- `specs/agentic-prompt-templates-implementation-plan.md` — `TemplateEngine`, `RenderedPrompt`, template TOML format
- `specs/09-tui-design-keybindings-implementation-plan.md` — `App`, `EventLoop`, panel system, `KeyContext`
- `specs/12-15-interview-salary-networking-notifications-implementation-plan.md` — `NotificationScheduler`, interview table schema

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
scraper     = "0.19"        # HTML parsing for company web research
ammonia     = "3"           # HTML sanitization before LLM input
once_cell   = "1"           # Lazy<Regex> for news/blog extraction patterns
regex       = "1"           # already used — news date/headline extraction
strsim      = "0.11"        # Jaro-Winkler for STAR story dedup
serde_json  = "1"           # dossier section JSON blob storage

# lazyjob-ralph/Cargo.toml
lazyjob-core  = { path = "../lazyjob-core" }
lazyjob-llm   = { path = "../lazyjob-llm" }
reqwest       = { workspace = true, default-features = false, features = ["rustls-tls", "json"] }
tokio         = { workspace = true }
serde         = { workspace = true }
serde_json    = { workspace = true }
uuid          = { workspace = true }
anyhow        = { workspace = true }
thiserror     = { workspace = true }
tracing       = { workspace = true }
futures       = "0.3"       # join_all for parallel page fetches
chrono        = { workspace = true }

# lazyjob-tui/Cargo.toml
ratatui       = { workspace = true }
crossterm     = { workspace = true }
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|---------------|
| `lazyjob-core` | All domain types: `PrepDossier`, `DossierSection`, `StarStory`, `BehavioralDimension`, `PrepProgress`, repositories, SQLite DDL, migrations 016-018 |
| `lazyjob-llm` | Prompt templates: `LoopType::InterviewDossier`, `LoopType::StarBankExtraction`; context structs `DossierResearchContext`, `StarExtractionContext` |
| `lazyjob-ralph` | `InterviewDossierLoop`, `StarBankExtractionLoop`, web research helpers, page fetchers |
| `lazyjob-tui` | `PrepDashboardView`, `DossierView`, `StarBankView`, `PrepProgressWidget` |
| `lazyjob-cli` | `lazyjob interview prep <application-id>` subcommand |

### Core Types

```rust
// lazyjob-core/src/interview/dossier.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepDossier {
    pub id:             Uuid,
    pub application_id: Uuid,
    pub company_id:     Uuid,
    pub created_at:     DateTime<Utc>,
    pub updated_at:     DateTime<Utc>,
    /// Ordered sections; rendered in sequence in the TUI dossier view.
    pub sections:       Vec<DossierSection>,
    /// Predicted questions extracted from the JD + company research.
    pub predicted_questions: Vec<PredictedQuestion>,
    /// Prep readiness score 0..100 recomputed on every dossier update.
    pub readiness_score: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DossierSectionKind {
    CompanyOverview,
    ProductAnalysis,
    TeamCultureNotes,
    RecentNews,
    TechStackSummary,
    QuestionPredictions,
    StarStoryMapping,
    NegotiationContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DossierSection {
    pub kind:       DossierSectionKind,
    pub title:      String,
    pub content_md: String,
    /// Source URLs scraped to produce this section.
    pub sources:    Vec<String>,
    pub generated_at: DateTime<Utc>,
    /// True while the section is being regenerated by a running loop.
    pub stale:      bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictedQuestion {
    pub id:              Uuid,
    pub dossier_id:      Uuid,
    pub question_text:   String,
    pub category:        QuestionCategory,  // re-use from interview-prep-question-generation
    pub confidence:      f32,               // 0.0–1.0, LLM-reported
    pub source_signal:   String,            // "job description: 'led cross-functional teams'"
    pub suggested_story: Option<Uuid>,      // FK → star_stories.id
}

// lazyjob-core/src/interview/star_bank.rs

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum BehavioralDimension {
    Leadership,
    Conflict,
    Failure,
    HighImpact,
    Ambiguity,
    CrossTeamInfluence,
    TechnicalComplexity,
    CustomerFocus,
    InitiativeOwnership,
}

impl BehavioralDimension {
    /// Trigger keywords used in offline classification (no LLM call).
    pub fn trigger_keywords(&self) -> &'static [&'static str] {
        match self {
            Self::Leadership        => &["led", "managed", "directed", "mentored", "grew"],
            Self::Conflict          => &["disagreed", "conflict", "pushback", "tension", "misalignment"],
            Self::Failure           => &["failed", "missed", "didn't work", "learned", "mistake"],
            Self::HighImpact        => &["million", "reduced", "increased", "saved", "launched"],
            Self::Ambiguity         => &["unclear", "ambiguous", "no roadmap", "undefined", "pioneered"],
            Self::CrossTeamInfluence=> &["stakeholder", "cross-functional", "partnered", "aligned", "collaborated"],
            Self::TechnicalComplexity => &["architecture", "refactored", "designed", "scalable", "performance"],
            Self::CustomerFocus     => &["user", "customer", "nps", "feedback", "retention"],
            Self::InitiativeOwnership => &["proactively", "own", "drove", "without being asked", "independently"],
        }
    }
}

/// A single STAR story extracted from the LifeSheet experience/achievements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarStory {
    pub id:              Uuid,
    pub life_sheet_id:   Uuid,  // canonical LifeSheet version ID
    pub experience_id:   Uuid,  // FK → experiences.id
    pub title:           String,
    /// Narrative assembled from the achievement bullet + context sentences.
    pub situation:       Option<String>,
    pub task:            Option<String>,
    pub action:          Option<String>,
    pub result:          Option<String>,
    /// Primary behavioral dimension(s); at most 3.
    pub dimensions:      Vec<BehavioralDimension>,
    /// STAR completeness: count of non-None fields / 4.
    pub completeness:    f32,
    /// If LLM was used to fill gaps: which fields it inferred.
    pub llm_inferred:    Vec<StarField>,
    pub created_at:      DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StarField { Situation, Task, Action, Result }

// lazyjob-core/src/interview/progress.rs

#[derive(Debug, Clone)]
pub struct PrepProgress {
    pub application_id:      Uuid,
    pub dossier_complete:    bool,
    pub question_bank_count: u32,
    pub star_stories_count:  u32,
    /// Fraction of questions that have a linked story in the bank.
    pub story_coverage:      f32,
    /// Mock sessions completed against this application.
    pub mock_sessions_count: u32,
    /// Average score across all completed mock sessions.
    pub avg_mock_score:      Option<f32>,
    /// Overall readiness 0..100.
    pub readiness_score:     u8,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/interview/dossier.rs

#[async_trait::async_trait]
pub trait PrepDossierRepository: Send + Sync {
    async fn upsert(&self, dossier: &PrepDossier) -> Result<(), DossierError>;
    async fn get_by_application(&self, application_id: Uuid)
        -> Result<Option<PrepDossier>, DossierError>;
    async fn update_section(
        &self,
        dossier_id: Uuid,
        section: DossierSection,
    ) -> Result<(), DossierError>;
    async fn upsert_predicted_question(
        &self,
        q: &PredictedQuestion,
    ) -> Result<(), DossierError>;
    async fn list_predicted_questions(
        &self,
        dossier_id: Uuid,
    ) -> Result<Vec<PredictedQuestion>, DossierError>;
}

#[async_trait::async_trait]
pub trait StarBankRepository: Send + Sync {
    async fn upsert_story(&self, story: &StarStory) -> Result<(), StarBankError>;
    async fn list_by_life_sheet(&self, life_sheet_id: Uuid)
        -> Result<Vec<StarStory>, StarBankError>;
    async fn find_best_match(
        &self,
        dimensions: &[BehavioralDimension],
        min_completeness: f32,
    ) -> Result<Vec<StarStory>, StarBankError>;
    /// Delete all stories for a given LifeSheet version (called on re-import).
    async fn delete_by_life_sheet(&self, life_sheet_id: Uuid)
        -> Result<u64, StarBankError>;
}
```

### SQLite Schema

```sql
-- Migration 016: prep_dossiers + dossier_sections + predicted_questions

CREATE TABLE prep_dossiers (
    id              TEXT PRIMARY KEY,           -- UUID
    application_id  TEXT NOT NULL UNIQUE,       -- FK → applications.id
    company_id      TEXT NOT NULL,              -- FK → companies.id
    readiness_score INTEGER NOT NULL DEFAULT 0, -- 0-100
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX prep_dossiers_company_idx ON prep_dossiers(company_id);

CREATE TABLE dossier_sections (
    id          TEXT PRIMARY KEY,               -- UUID
    dossier_id  TEXT NOT NULL
                REFERENCES prep_dossiers(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    content_md  TEXT NOT NULL DEFAULT '',
    sources     TEXT NOT NULL DEFAULT '[]',     -- JSON array of URLs
    stale       INTEGER NOT NULL DEFAULT 0,
    generated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX dossier_sections_kind_idx
    ON dossier_sections(dossier_id, kind);

CREATE TABLE predicted_questions (
    id              TEXT PRIMARY KEY,           -- UUID
    dossier_id      TEXT NOT NULL
                    REFERENCES prep_dossiers(id) ON DELETE CASCADE,
    question_text   TEXT NOT NULL,
    category        TEXT NOT NULL,
    confidence      REAL NOT NULL,
    source_signal   TEXT NOT NULL,
    suggested_story TEXT,                       -- FK → star_stories.id nullable
    created_at      TEXT NOT NULL
);

CREATE INDEX predicted_questions_dossier_idx ON predicted_questions(dossier_id);

-- Migration 017: star_stories

CREATE TABLE star_stories (
    id              TEXT PRIMARY KEY,           -- UUID
    life_sheet_id   TEXT NOT NULL,
    experience_id   TEXT NOT NULL,
    title           TEXT NOT NULL,
    situation       TEXT,
    task            TEXT,
    action          TEXT,
    result          TEXT,
    dimensions      TEXT NOT NULL DEFAULT '[]', -- JSON array of BehavioralDimension strings
    completeness    REAL NOT NULL DEFAULT 0.0,
    llm_inferred    TEXT NOT NULL DEFAULT '[]', -- JSON array of StarField strings
    created_at      TEXT NOT NULL
);

CREATE INDEX star_stories_life_sheet_idx ON star_stories(life_sheet_id);
CREATE INDEX star_stories_experience_idx ON star_stories(experience_id);

-- Covering index for find_best_match queries
CREATE INDEX star_stories_completeness_idx
    ON star_stories(completeness DESC);

-- Migration 018: prep_checkpoints (for scheduler integration)

CREATE TABLE prep_checkpoints (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL,
    interview_id    TEXT,                       -- FK → interviews.id nullable
    fire_at         TEXT NOT NULL,              -- ISO-8601 UTC
    fired_at        TEXT,
    checkpoint_type TEXT NOT NULL               -- '48h_reminder' | 'day_of' | 'post_interview'
);

CREATE INDEX prep_checkpoints_fire_idx
    ON prep_checkpoints(fire_at)
    WHERE fired_at IS NULL;
```

### Module Structure

```
lazyjob-core/
  src/
    interview/
      mod.rs                  # re-exports
      dossier.rs              # PrepDossier, DossierSection, PredictedQuestion, Repository trait
      star_bank.rs            # StarStory, BehavioralDimension, StarField, Repository trait
      progress.rs             # PrepProgress, PrepProgressService
      sqlite_dossier.rs       # SqlitePrepDossierRepository
      sqlite_star_bank.rs     # SqliteStarBankRepository
      progress_service.rs     # PrepProgressService impl

lazyjob-llm/
  src/
    prompts/
      interview_dossier.toml  # DossierResearchContext → research synthesis prompt
      star_extraction.toml    # StarExtractionContext → story JSON output prompt
      question_prediction.toml # QuestionPredictionContext → predicted_questions output

lazyjob-ralph/
  src/
    loops/
      interview_dossier/
        mod.rs                # InterviewDossierLoop entry point
        phases.rs             # phase_company_overview(), phase_product(), phase_news(), etc.
        web_fetcher.rs        # CompanyWebFetcher (scraper + ammonia)
        news_aggregator.rs    # GoogleNewsRssFetcher, parse_rss_items()
        question_predictor.rs # QuestionPredictor (LLM call → Vec<PredictedQuestion>)
      star_extraction/
        mod.rs                # StarBankExtractionLoop entry point
        keyword_classifier.rs # KeywordClassifier (offline BehavioralDimension tagging)
        star_scorer.rs        # StarScorer (LLM gap-fill for low-completeness stories)
        deduper.rs            # StoryDeduper (jaro_winkler title dedup)

lazyjob-tui/
  src/
    views/
      prep_dashboard.rs       # PrepDashboardView
      dossier_viewer.rs       # DossierView (section tabs + markdown rendering)
      star_bank_browser.rs    # StarBankView (list + detail panel)
```

---

## Implementation Phases

### Phase 1 — SQLite Schema + Core Types (MVP foundation)

**Step 1.1 — Migrations**

File: `lazyjob-core/migrations/016_prep_dossiers.sql`, `017_star_stories.sql`, `018_prep_checkpoints.sql`

Apply all three DDL blocks from the Schema section above. Verify using `sqlx migrate run` in the `lazyjob-core` directory and confirm all three migrations appear in `_sqlx_migrations`.

**Step 1.2 — Domain types**

File: `lazyjob-core/src/interview/dossier.rs`

Implement all structs from the Core Types section:
- `PrepDossier`, `DossierSection`, `DossierSectionKind`, `PredictedQuestion`
- All types `#[derive(Debug, Clone, Serialize, Deserialize)]`
- `DossierSectionKind`: additionally `#[derive(PartialEq, Eq, Hash)]` for HashMap keys

File: `lazyjob-core/src/interview/star_bank.rs`
- `StarStory`, `BehavioralDimension`, `StarField`
- `BehavioralDimension::trigger_keywords()` as a `match` returning `&'static [&'static str]`
- No wildcard arm — adding a new variant must be a compile error

**Step 1.3 — Repository traits**

File: `lazyjob-core/src/interview/dossier.rs` (append)
- `PrepDossierRepository` async trait
- `StarBankRepository` async trait
- Both marked `Send + Sync`

**Step 1.4 — Error enums**

File: `lazyjob-core/src/interview/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum DossierError {
    #[error("dossier not found for application {0}")]
    NotFound(Uuid),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum StarBankError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

**Verification:** `cargo test -p lazyjob-core` compiles with no errors; `clippy -- -D warnings` passes.

---

### Phase 2 — SQLite Repository Implementations

**Step 2.1 — `SqlitePrepDossierRepository`**

File: `lazyjob-core/src/interview/sqlite_dossier.rs`

```rust
pub struct SqlitePrepDossierRepository {
    pool: sqlx::Pool<Sqlite>,
}

impl SqlitePrepDossierRepository {
    pub fn new(pool: sqlx::Pool<Sqlite>) -> Self { Self { pool } }
}
```

Key implementation notes:
- `upsert`: `INSERT INTO prep_dossiers ... ON CONFLICT(application_id) DO UPDATE SET updated_at = excluded.updated_at, readiness_score = excluded.readiness_score`; sections stored separately
- `update_section`: `INSERT INTO dossier_sections ... ON CONFLICT(dossier_id, kind) DO UPDATE SET ...` — always safe to call repeatedly as sections are regenerated
- `list_predicted_questions`: `SELECT * FROM predicted_questions WHERE dossier_id = ? ORDER BY confidence DESC`

**Step 2.2 — `SqliteStarBankRepository`**

File: `lazyjob-core/src/interview/sqlite_star_bank.rs`

- `upsert_story`: `INSERT INTO star_stories ... ON CONFLICT(id) DO UPDATE SET ...` — idempotent re-runs safe
- `find_best_match`: fetch all stories for the current life_sheet_id, deserialize `dimensions` JSON, filter in Rust to stories where `dimensions` intersects the requested set, sort by `completeness DESC`, return top 5
- `delete_by_life_sheet`: `DELETE FROM star_stories WHERE life_sheet_id = ?`

**Verification:** `#[sqlx::test(migrations = "migrations")]` tests in `lazyjob-core/tests/interview_dossier.rs`:
- Insert a dossier, upsert a section, query back — section content matches
- Update same section kind — only one row remains (UNIQUE constraint)
- Insert 3 stories, `find_best_match([Leadership], 0.5)` returns all 3

---

### Phase 3 — Star Bank Extraction Loop

**Step 3.1 — Keyword classifier (offline)**

File: `lazyjob-ralph/src/loops/star_extraction/keyword_classifier.rs`

```rust
pub struct KeywordClassifier;

impl KeywordClassifier {
    /// Returns all dimensions whose trigger keywords appear in text (lowercased).
    pub fn classify(text: &str) -> Vec<BehavioralDimension> {
        let lower = text.to_lowercase();
        BehavioralDimension::ALL_VARIANTS
            .iter()
            .filter(|dim| {
                dim.trigger_keywords()
                   .iter()
                   .any(|kw| lower.contains(kw))
            })
            .cloned()
            .collect()
    }
}
```

Add `BehavioralDimension::ALL_VARIANTS: &[BehavioralDimension]` as a `const` array listing all 9 variants — updated whenever a new variant is added (enforced by the exhaustive `match` in `trigger_keywords`).

**Step 3.2 — Star scorer (LLM gap-fill)**

File: `lazyjob-ralph/src/loops/star_extraction/star_scorer.rs`

```rust
pub struct StarScorer {
    llm: Arc<dyn LlmProvider>,
    template_engine: TemplateEngine,
}

impl StarScorer {
    /// For stories with completeness < 0.75, ask the LLM to infer missing fields.
    /// Returns the same story with inferred fields filled in and llm_inferred updated.
    pub async fn fill_gaps(&self, story: StarStory, context: &LifeSheet)
        -> anyhow::Result<StarStory>
    {
        // Build StarExtractionContext from story + surrounding experience
        // Call LLM with temperature 0.1 for structured JSON output
        // Parse response as StarFieldsJson { situation, task, action, result }
        // Only replace None fields; never overwrite user-provided content
        // Append filled field names to story.llm_inferred
        todo!()
    }
}
```

LLM prompt template `star_extraction.toml`:
- System: "You are a career coach helping extract STAR components from professional experience bullets."
- User: Injects `experience_context`, `achievement_text`, `missing_fields` list
- Output: JSON object `{ "situation": "...", "task": "...", "action": "...", "result": "..." }` — empty string for still-unknown fields

**Step 3.3 — Story deduplication**

File: `lazyjob-ralph/src/loops/star_extraction/deduper.rs`

```rust
pub struct StoryDeduper;

impl StoryDeduper {
    /// Returns `true` if a story with similar title already exists in `existing`.
    /// Uses `strsim::jaro_winkler` threshold of 0.88.
    pub fn is_duplicate(candidate: &str, existing: &[StarStory]) -> bool {
        existing.iter().any(|s| strsim::jaro_winkler(&s.title, candidate) >= 0.88)
    }
}
```

**Step 3.4 — `StarBankExtractionLoop`**

File: `lazyjob-ralph/src/loops/star_extraction/mod.rs`

```rust
pub struct StarBankExtractionLoop {
    life_sheet_repo: Arc<dyn LifeSheetRepository>,
    star_bank_repo:  Arc<dyn StarBankRepository>,
    scorer:          StarScorer,
    events_tx:       broadcast::Sender<WorkerEvent>,
    cancel:          CancelToken,
}
```

Execution flow (called from `LoopRunner::run()`):

1. `life_sheet_repo.get_current()` → `LifeSheet`
2. `star_bank_repo.delete_by_life_sheet(life_sheet_id)` (full re-extract on each run)
3. For each `WorkExperience` entry in the life sheet:
   a. For each `Achievement` bullet:
      - `KeywordClassifier::classify(bullet.text)` → `Vec<BehavioralDimension>`
      - Skip if no dimensions detected (not a behavioral story)
      - Build a `StarStory` stub: title = first 80 chars of bullet, action = bullet.text, other fields None
      - `StoryDeduper::is_duplicate(&title, &accumulated_stories)` → skip if dupe
      - Emit `WorkerEvent::Progress { message: "Extracted story: {title}", percent: ... }`
   b. `check_cancelled()` after each experience entry
4. For each story with `completeness < 0.75`: call `scorer.fill_gaps(story, &life_sheet)`
5. `star_bank_repo.upsert_story(&story)` for each story
6. Emit `WorkerEvent::Complete { summary: "{n} stories extracted, {m} LLM-enhanced" }`

`LoopType::StarBankExtraction` gets `concurrency_limit() = 1` (only one extraction at a time) and `priority() = 2` (low — runs after question generation).

**Verification:**
- Unit test: `KeywordClassifier::classify("led a team of 5 engineers")` returns `[Leadership]`
- Integration test with `#[sqlx::test]`: seed a `LifeSheet` with 3 experiences, run the loop, confirm `star_bank_repo.list_by_life_sheet()` returns ≥ 3 stories

---

### Phase 4 — Interview Dossier Loop

**Step 4.1 — Company web fetcher**

File: `lazyjob-ralph/src/loops/interview_dossier/web_fetcher.rs`

```rust
pub struct CompanyWebFetcher {
    client: reqwest::Client,
}

impl CompanyWebFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (compatible; LazyJob/1.0)")
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client build"),
        }
    }

    /// Fetches URL, runs through ammonia allowlist sanitizer, returns plain text.
    pub async fn fetch_text(&self, url: &str) -> anyhow::Result<String> {
        let html = self.client.get(url).send().await?.text().await?;
        let clean_html = ammonia::Builder::new()
            .tags(std::collections::HashSet::new())   // strip all tags
            .clean(&html)
            .to_string();
        Ok(clean_html)
    }

    /// Fetches company about page URL from CompanyRecord.website, returns sanitized text.
    pub async fn fetch_company_about(&self, company: &CompanyRecord)
        -> anyhow::Result<Option<String>>
    {
        let Some(website) = &company.website else { return Ok(None) };
        let about_url = format!("{}/about", website.trim_end_matches('/'));
        Ok(Some(self.fetch_text(&about_url).await?))
    }

    pub fn new_with_base_url(_base: &str) -> Self { Self::new() }
}
```

All URLs come from `CompanyRecord.website` (stored in SQLite by `CompanyService`) — never from user input, never from LLM output. This prevents SSRF from prompt injection.

**Step 4.2 — News aggregator**

File: `lazyjob-ralph/src/loops/interview_dossier/news_aggregator.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static ITEM_PATTERN: Lazy<Regex> = Lazy::new(||
    Regex::new(r"(?s)<item>(.*?)</item>").unwrap()
);
static TITLE_PATTERN: Lazy<Regex> = Lazy::new(||
    Regex::new(r"<title><!\[CDATA\[(.*?)\]\]></title>").unwrap()
);
static PUBDATE_PATTERN: Lazy<Regex> = Lazy::new(||
    Regex::new(r"<pubDate>(.*?)</pubDate>").unwrap()
);

pub struct NewsItem {
    pub title: String,
    pub pub_date: Option<DateTime<Utc>>,
    pub link: Option<String>,
}

pub struct GoogleNewsRssFetcher {
    client: reqwest::Client,
}

impl GoogleNewsRssFetcher {
    pub async fn fetch_recent(
        &self,
        company_name: &str,
        days: u32,
    ) -> anyhow::Result<Vec<NewsItem>> {
        let query = urlencoding::encode(company_name);
        let url = format!(
            "https://news.google.com/rss/search?q={}&hl=en-US&gl=US&ceid=US:en",
            query
        );
        let rss = self.client.get(&url).send().await?.text().await?;
        let items = parse_rss_items(&rss, days);
        Ok(items)
    }
}

fn parse_rss_items(rss: &str, last_n_days: u32) -> Vec<NewsItem> {
    // Use ITEM_PATTERN to extract <item> blocks
    // For each block, extract title via TITLE_PATTERN, pubDate via PUBDATE_PATTERN
    // Parse pubDate with chrono::DateTime::parse_from_rfc2822, filter by last_n_days
    // Return at most 10 items sorted by pubDate DESC
    todo!()
}
```

**Step 4.3 — Question predictor**

File: `lazyjob-ralph/src/loops/interview_dossier/question_predictor.rs`

```rust
pub struct QuestionPredictor {
    llm: Arc<dyn LlmProvider>,
    template_engine: TemplateEngine,
}

#[derive(Serialize)]
struct QuestionPredictionContext {
    job_title:        String,
    job_description:  String,  // truncated to 3000 chars to control tokens
    company_overview: String,  // from CompanyRecord or freshly scraped
    candidate_role_history: Vec<String>,  // top 3 job titles from LifeSheet
    num_questions:    u32,     // always 10 in MVP
}

impl QuestionPredictor {
    pub async fn predict(
        &self,
        ctx: QuestionPredictionContext,
    ) -> anyhow::Result<Vec<PredictedQuestion>> {
        // Build RenderedPrompt from question_prediction.toml
        // Call LLM with temperature = 0.4
        // Parse response as Vec<PredictedQuestionRaw>:
        //   { "question": "...", "category": "...", "confidence": 0.8, "signal": "..." }
        // Validate: skip items where confidence < 0.3 or question.len() < 20
        // Return at most 10 items sorted by confidence DESC
        todo!()
    }
}
```

**Step 4.4 — `InterviewDossierLoop`**

File: `lazyjob-ralph/src/loops/interview_dossier/mod.rs`

```rust
pub struct InterviewDossierLoop {
    application_id:  Uuid,
    dossier_repo:    Arc<dyn PrepDossierRepository>,
    company_repo:    Arc<dyn CompanyRepository>,
    life_sheet_repo: Arc<dyn LifeSheetRepository>,
    star_bank_repo:  Arc<dyn StarBankRepository>,
    job_repo:        Arc<dyn JobRepository>,
    web_fetcher:     CompanyWebFetcher,
    news_fetcher:    GoogleNewsRssFetcher,
    predictor:       QuestionPredictor,
    llm:             Arc<dyn LlmProvider>,
    template_engine: TemplateEngine,
    events_tx:       broadcast::Sender<WorkerEvent>,
    cancel:          CancelToken,
}
```

Execution phases (each phase updates a dossier section and emits `WorkerEvent::Progress`):

```
Phase A — Bootstrap
  1. Load Application → job_id, company_id
  2. Load CompanyRecord from company_repo
  3. Load LifeSheet from life_sheet_repo
  4. Load Job (for description)
  5. Create PrepDossier stub in SQLite (upsert)
  
Phase B — Company Overview (DossierSectionKind::CompanyOverview)
  1. web_fetcher.fetch_company_about(company)  → about_text
  2. Build DossierResearchContext { company_name, about_text, tech_stack }
  3. LLM call (temperature 0.3): synthesize 3-paragraph company overview in Markdown
  4. dossier_repo.update_section(dossier_id, DossierSection { kind: CompanyOverview, ... })
  check_cancelled()

Phase C — Product Analysis (DossierSectionKind::ProductAnalysis)
  1. web_fetcher.fetch_text(company.website + "/product") if available
  2. web_fetcher.fetch_text(company.website + "/blog") first 5 posts (parallel via join_all)
  3. LLM call: summarize product landscape, key features, engineering challenges inferred
  4. dossier_repo.update_section(...)
  check_cancelled()

Phase D — Recent News (DossierSectionKind::RecentNews)
  1. news_fetcher.fetch_recent(company.name, 30) → Vec<NewsItem>
  2. If items.is_empty(): emit Progress("No recent news found"), skip LLM
  3. Else: LLM call: summarize news highlights relevant to the role
  4. dossier_repo.update_section(...)
  check_cancelled()

Phase E — Team Culture Notes (DossierSectionKind::TeamCultureNotes)
  1. Use CompanyRecord.culture_signals (populated by CompanyService in prior plan)
  2. LLM call: expand culture signals into interview implications + STAR story angles
  3. dossier_repo.update_section(...)
  check_cancelled()

Phase F — Question Predictions (DossierSectionKind::QuestionPredictions)
  1. predictor.predict(QuestionPredictionContext { ... })
  2. For each PredictedQuestion:
     a. Find best matching StarStory via star_bank_repo.find_best_match(question.category)
     b. Set predicted_question.suggested_story = story.id if found
  3. dossier_repo.upsert_predicted_question for each
  4. dossier_repo.update_section(QuestionPredictions, content = bulleted list in MD)
  check_cancelled()

Phase G — STAR Story Mapping (DossierSectionKind::StarStoryMapping)
  1. Load all star_stories for current life_sheet
  2. For each predicted question: recompute best story match
  3. Build Markdown table: | Question | Category | Suggested Story | Coverage |
  4. dossier_repo.update_section(StarStoryMapping, ...)

Phase H — Compute readiness_score
  1. story_coverage = predicted_questions with suggested_story / total predicted_questions
  2. readiness_score = clamp(story_coverage * 60 + (completeness_avg * 40), 0, 100) as u8
  3. dossier_repo.upsert(dossier with updated readiness_score)

Emit WorkerEvent::Complete { summary: "Dossier ready. Readiness: {score}/100" }
```

`LoopType::InterviewDossier` gets `concurrency_limit() = 2` (two companies in parallel) and `priority() = 5` (medium — same as question generation).

**Verification:**
- Unit test: `QuestionPredictor` with a `MockLlmProvider` returning 3 valid + 1 low-confidence question → only 3 returned
- Integration test: full loop run against a seeded application/company/job in in-memory SQLite → confirm all 7 section kinds present in `dossier_sections` table

---

### Phase 5 — Prep Progress Service + Scheduling

**Step 5.1 — `PrepProgressService`**

File: `lazyjob-core/src/interview/progress_service.rs`

```rust
pub struct PrepProgressService {
    pool: sqlx::Pool<Sqlite>,
}

const READINESS_QUERY: &str = r#"
    SELECT
        a.id AS application_id,
        (SELECT COUNT(*) FROM prep_dossiers pd WHERE pd.application_id = a.id) AS dossier_complete,
        (SELECT COUNT(*) FROM prep_sessions ps WHERE ps.application_id = a.id) AS question_bank_count,
        (SELECT COUNT(*) FROM star_stories ss
         WHERE ss.life_sheet_id = (
             SELECT id FROM life_sheet_meta ORDER BY created_at DESC LIMIT 1
         )) AS star_stories_count,
        (SELECT COUNT(*) FROM predicted_questions pq
         JOIN prep_dossiers pd ON pq.dossier_id = pd.id
         WHERE pd.application_id = a.id AND pq.suggested_story IS NOT NULL) AS covered_questions,
        (SELECT COUNT(*) FROM predicted_questions pq
         JOIN prep_dossiers pd ON pq.dossier_id = pd.id
         WHERE pd.application_id = a.id) AS total_predicted,
        (SELECT COUNT(*) FROM mock_interview_sessions mis
         WHERE mis.application_id = a.id AND mis.completed_at IS NOT NULL) AS mock_sessions_count,
        (SELECT AVG(overall_score) FROM mock_interview_sessions mis
         WHERE mis.application_id = a.id AND mis.completed_at IS NOT NULL) AS avg_mock_score,
        (SELECT readiness_score FROM prep_dossiers pd
         WHERE pd.application_id = a.id LIMIT 1) AS readiness_score
    FROM applications a
    WHERE a.id = ?
"#;

impl PrepProgressService {
    pub async fn compute(&self, application_id: Uuid)
        -> Result<PrepProgress, sqlx::Error>
    {
        // Execute READINESS_QUERY, map row to PrepProgress
        // story_coverage = covered_questions as f32 / max(total_predicted, 1) as f32
        todo!()
    }
}
```

**Step 5.2 — Prep checkpoint scheduler**

File: `lazyjob-core/src/interview/progress_service.rs` (extend)

```rust
impl PrepProgressService {
    /// Called when an interview is scheduled; inserts checkpoint rows.
    pub async fn schedule_checkpoints(
        &self,
        application_id: Uuid,
        interview_id:   Uuid,
        interview_at:   DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let checkpoints = vec![
            (interview_at - Duration::hours(48), "48h_reminder"),
            (interview_at - Duration::hours(2),  "day_of"),
            (interview_at + Duration::hours(2),  "post_interview"),
        ];
        for (fire_at, kind) in checkpoints {
            sqlx::query!(
                r#"INSERT OR IGNORE INTO prep_checkpoints
                   (id, application_id, interview_id, fire_at, checkpoint_type)
                   VALUES (?, ?, ?, ?, ?)"#,
                Uuid::new_v4().to_string(),
                application_id.to_string(),
                interview_id.to_string(),
                fire_at.to_rfc3339(),
                kind,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}
```

`NotificationScheduler` (from the 12-15 notifications plan) polls `prep_checkpoints WHERE fired_at IS NULL AND fire_at <= datetime('now')` on its existing 5-minute tick. For the `48h_reminder` type it fires a `notify-rust` desktop notification with body "Interview in 48 hours — prep dossier ready: {company_name}". For `post_interview` it triggers `LoopDispatch::dispatch(LoopType::InterviewDossier, ...)` to regenerate the dossier with a post-interview reflection section (Phase 3+).

**Verification:** Unit test `schedule_checkpoints` with `interview_at = now() + 72h` → 3 rows inserted; `fire_at` for the `48h_reminder` row is within 1 second of `now() + 24h`.

---

### Phase 6 — TUI Views

**Step 6.1 — `PrepDashboardView`**

File: `lazyjob-tui/src/views/prep_dashboard.rs`

Layout: full-width panel, vertical split into:
- Top row (20%): `PrepProgressWidget` — horizontal progress bars for story coverage, mock session count, readiness score (uses ratatui `Gauge`)
- Bottom row (80%): 3-column horizontal split:
  - Left (30%): application list (`List` + `ListState`), keybind `p` to open dossier for selected
  - Center (40%): `DossierSectionSummary` — section list with ✓/⚠/⏳ icons (Text::styled spans)
  - Right (30%): `StarBankSummaryWidget` — counts by BehavioralDimension as a ratatui `BarChart`

Keybindings (in `KeyContext::PrepDashboard`):
- `j`/`k` — navigate application list
- `d` — open full DossierView for selected application
- `s` — open StarBankView
- `r` — trigger `LoopDispatch::dispatch(LoopType::InterviewDossier, { application_id })`
- `q` — return to previous view

**Step 6.2 — `DossierView`**

File: `lazyjob-tui/src/views/dossier_viewer.rs`

Layout: tab bar at top (one tab per `DossierSectionKind`), full-width scrollable area below.

Tab rendering: use `ratatui::widgets::Tabs` with section titles. Active tab highlighted with `Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)`. Stale sections get `"⟳"` suffix in tab title.

Content rendering: `DossierSection.content_md` rendered as styled paragraphs using a minimal Markdown → `ratatui::text::Spans` converter (supporting `**bold**`, `*italic*`, `# heading`, `- list`). Full CommonMark is not required — handle just these four patterns via regex replacement into styled `Span` vectors.

Keybindings (in `KeyContext::DossierView`):
- `h`/`l` — previous/next tab
- `j`/`k` — scroll content
- `Ctrl-f`/`Ctrl-b` — page down/up
- `s` — open source URLs list popup (shows `sources` array for current section)
- `q` — return to PrepDashboard

**Step 6.3 — `StarBankView`**

File: `lazyjob-tui/src/views/star_bank_browser.rs`

Layout: 40/60 horizontal split.
- Left: `List` of story titles with completeness badge `[●●●○]` (4 blocks, each filled if ≥ 0.25, 0.5, 0.75, 1.0)
- Right: story detail panel showing STAR fields as labeled paragraphs, dimensions as colored tags

---

## Key Crate APIs

- `reqwest::Client::get(url).send().await?` — page fetch
- `reqwest::Response::text().await?` — response body as String
- `ammonia::Builder::new().tags(HashSet::new()).clean(&html).to_string()` — strip all HTML tags
- `scraper::Html::parse_document(&html)` — HTML parsing when structural extraction needed (team page)
- `scraper::Selector::parse("h2.team-member")` — CSS selector
- `strsim::jaro_winkler(a, b)` — story dedup threshold check
- `futures::future::join_all(page_fetches)` — parallel page fetching (up to 5 pages)
- `sqlx::query!(...)` — compile-time verified queries
- `tokio::sync::broadcast::Sender<WorkerEvent>` — progress event fan-out to TUI
- `once_cell::sync::Lazy<Regex>` — compiled regex patterns for RSS parsing
- `ratatui::widgets::Tabs::new(titles).select(active_idx)` — tab bar
- `ratatui::widgets::Gauge::default().percent(score as u16)` — progress bar
- `ratatui::widgets::BarChart::default().data(&data).bar_width(3)` — story dimension chart

---

## Error Handling

```rust
// lazyjob-ralph/src/loops/interview_dossier/error.rs

#[derive(thiserror::Error, Debug)]
pub enum DossierLoopError {
    #[error("application {0} not found")]
    ApplicationNotFound(Uuid),

    #[error("company record not found for application {0}")]
    CompanyNotFound(Uuid),

    #[error("web fetch failed for {url}: {source}")]
    WebFetch {
        url:    String,
        #[source]
        source: reqwest::Error,
    },

    #[error("LLM call failed: {0}")]
    LlmError(#[from] anyhow::Error),

    #[error("repository error: {0}")]
    Repository(#[from] DossierError),

    #[error("loop cancelled")]
    Cancelled,
}

// lazyjob-ralph/src/loops/star_extraction/error.rs

#[derive(thiserror::Error, Debug)]
pub enum StarExtractionError {
    #[error("life sheet not found")]
    LifeSheetNotFound,
    #[error("LLM error during gap fill: {0}")]
    LlmError(#[from] anyhow::Error),
    #[error("repository error: {0}")]
    Repository(#[from] StarBankError),
    #[error("loop cancelled")]
    Cancelled,
}
```

**Error degradation policy:**
- Web fetch failures in `DossierLoopError::WebFetch` are caught per-phase — the phase's section is written with `content_md = "*(research unavailable — fetch failed)*"` and the loop continues to the next phase.
- LLM errors in a section phase: same fallback — write a stub section and continue.
- `Cancelled` propagation: each `check_cancelled()` call returns `Err(DossierLoopError::Cancelled)` which bubbles up and causes `WorkerEvent::Cancelled` to be emitted.
- Phases B–G are individually wrapped in `match` so any single phase failure does not abort the rest.

---

## Testing Strategy

### Unit Tests

**`KeywordClassifier`** (`lazyjob-ralph/src/loops/star_extraction/keyword_classifier.rs`):
```rust
#[test]
fn classify_leadership() {
    let result = KeywordClassifier::classify("led a team of 5 through a platform migration");
    assert!(result.contains(&BehavioralDimension::Leadership));
    assert!(result.contains(&BehavioralDimension::TechnicalComplexity));
}

#[test]
fn classify_returns_empty_for_no_match() {
    let result = KeywordClassifier::classify("maintained the CI pipeline");
    assert!(result.is_empty());
}
```

**`StoryDeduper`**:
```rust
#[test]
fn deduplicates_similar_titles() {
    let existing = vec![StarStory { title: "Led migration of data warehouse".into(), .. }];
    assert!(StoryDeduper::is_duplicate("Led migration of the data warehouse", &existing));
    assert!(!StoryDeduper::is_duplicate("Designed new authentication system", &existing));
}
```

**`QuestionPredictor`** (with `MockLlmProvider`):
- Valid JSON with 5 questions → 5 returned
- LLM returns malformed JSON → `Err(DossierLoopError::LlmError(...))`
- 3 questions below confidence 0.3 threshold → filtered to 0 returned

### Integration Tests (`#[sqlx::test]`)

File: `lazyjob-core/tests/interview_dossier_repo.rs`:
- Upsert dossier, add 3 sections, verify all 3 exist
- Update a section via `update_section`, verify only one row for that `kind`
- `list_predicted_questions` returns questions sorted by `confidence DESC`

File: `lazyjob-core/tests/star_bank_repo.rs`:
- Upsert 4 stories (2 Leadership, 1 Conflict, 1 HighImpact), `find_best_match([Leadership], 0.5)` → 2 returned
- `delete_by_life_sheet` removes all stories for that life_sheet_id

### Loop Integration Tests

File: `lazyjob-ralph/tests/dossier_loop_integration.rs`:
- Seed: company with known `website = "http://localhost:{port}"`, application, job, LifeSheet
- Start `wiremock::MockServer` responding to `/about` with HTML "Acme Corp is a SaaS company"
- Run `InterviewDossierLoop` with a `MockLlmProvider` returning canned Markdown
- Assert: `CompanyOverview` and `RecentNews` sections present in SQLite with non-empty `content_md`
- Assert: `WorkerEvent::Complete` emitted before function returns

### TUI Tests

`DossierView` tab switching:
- Build an `App` with a `PrepDossier` containing 5 sections
- Simulate `h`/`l` keypresses against the `EventLoop`
- Assert `active_tab_index` cycles through 0..4 and wraps

---

## Open Questions

1. **STAR story LLM inference scope:** The plan gates LLM gap-fill at `completeness < 0.75`. Should this threshold be user-configurable in `~/.config/lazyjob/config.toml`, or kept as a code constant in MVP? User config adds surface area; a constant is simpler.

2. **Google News RSS reliability:** Google News RSS is unofficial and may change format or add geo-blocking. Should Phase 4 include a fallback to a DuckDuckGo news search (`https://html.duckduckgo.com/html/?q=site:reuters.com+{company}`) when the RSS returns 0 items?

3. **Story coverage threshold for readiness:** The current formula `story_coverage * 60 + completeness_avg * 40` weights coverage heavily. Should mock session score also factor in (`+ avg_mock_score * 20` and renormalize)? Deferred to Phase 5 iteration.

4. **Dossier regeneration on `update_section`:** When the loop regenerates a single stale section (user presses `r` on one section), should the entire loop re-run from Phase A, or should individual phase functions be callable independently? Independent phase re-run is more efficient but requires the `InterviewDossierLoop` phases to be refactored as standalone functions taking `&mut DossierLoopState`. Deferred to Phase 4 polish.

5. **SSRF via CompanyRecord.website:** The website URL is user-entered (imported from LifeSheet or set during job import). A validation step that restricts URLs to http/https schemes and rejects private IP ranges (10.x, 172.16.x, 192.168.x, localhost) should be added to `CompanyService::set_website()` before Phase 4 ships.

6. **Post-interview reflection:** The spec describes scheduling a `post_interview` checkpoint 2 hours after the interview. This checkpoint should trigger a new `LoopType::PostInterviewReflection` (not defined in this plan) that asks the candidate "how did it go?" via the TUI and stores their reflection. This loop type is deferred.

---

## Related Specs

- [specs/interview-prep-question-generation.md](./interview-prep-question-generation.md) — question bank and prep session types
- [specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md) — mock interview subprocess protocol
- [specs/job-search-company-research.md](./job-search-company-research.md) — `CompanyRecord`, `CompanyService`, web research pipeline
- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) — `LifeSheet`, `WorkExperience`, `Achievement`
- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — `LoopType`, `LoopDispatch`, concurrency model
- [specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) — `WorkerEvent`, `CancelToken`, NDJSON framing
- [specs/12-15-interview-salary-networking-notifications.md](./12-15-interview-salary-networking-notifications.md) — `NotificationScheduler`, `interviews` table
- [specs/profile-skills-gap-analysis.md](./profile-skills-gap-analysis.md) — `GapReport`, ESCO skill taxonomy referenced in question prediction
