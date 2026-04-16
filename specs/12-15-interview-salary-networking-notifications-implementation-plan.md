# Implementation Plan: Interview Prep, Salary Negotiation, Networking & Notifications

## Status
Draft

## Related Spec
`specs/12-15-interview-salary-networking-notifications.md`

## Overview

This plan covers four interrelated product features that extend the LazyJob pipeline beyond application submission: (1) AI-powered interview preparation with question generation and mock interview loops via Ralph subprocesses, (2) salary intelligence with market data aggregation and negotiation strategy generation, (3) networking and referral management with contact graph storage and outreach drafting, and (4) a notification system including morning briefs, reminders, and in-TUI alerts.

All four subsystems share a common architecture: domain types and repository operations live in `lazyjob-core`, LLM-backed generation logic lives in `lazyjob-llm` or service layers in `lazyjob-core`, Ralph subprocess orchestration lives in `lazyjob-ralph`, and TUI panels live in `lazyjob-tui`. Data is stored exclusively in SQLite, extending existing migrations introduced in earlier phases.

Each subsystem is phased so that a minimal but shippable slice is available after Phase 1 of each part. The subsystems are largely independent of each other and can be implemented in parallel by different engineers, though all depend on the core persistence layer (`04-sqlite-persistence-implementation-plan.md`) and the LLM provider abstraction (`02-llm-provider-abstraction-implementation-plan.md`).

## Prerequisites

### Specs/Plans That Must Be Implemented First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migration framework
- `specs/02-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` async trait
- `specs/10-application-workflow-implementation-plan.md` — `Application`, `ApplicationId`, workflow state machine
- `specs/08-cover-letter-generation-implementation-plan.md` — `CompanyResearcher`, `CompanyInsights`

### Crates to Add to Cargo.toml

```toml
# lazyjob-core
[dependencies]
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
thiserror = "1"
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
tracing = "0.1"
cron = "0.12"                # cron expression parsing for scheduler
notify-rust = "4"            # desktop notifications (Linux/macOS)

# lazyjob-ralph
tokio-util = { version = "0.7", features = ["codec"] }

# workspace-level (already present)
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

---

## Part 1: Interview Preparation

### Architecture

#### Crate Placement
- `lazyjob-core/src/interview/` — domain types, repositories, `InterviewPrepService`
- `lazyjob-ralph/src/loops/mock_interview.rs` — Ralph subprocess loop for mock interviews
- `lazyjob-tui/src/panels/interview_prep.rs` — TUI question bank browser and mock panel

#### Core Types

```rust
// lazyjob-core/src/interview/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type InterviewId = Uuid;
pub type QuestionId = Uuid;
pub type PrepSessionId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum InterviewType {
    PhoneScreen,
    TechnicalScreen,
    Behavioral,
    SystemDesign,
    OnSite,
    Executive,
    TakeHome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum QuestionType {
    Behavioral,
    Technical,
    SystemDesign,
    Coding,
    CultureFit,
    Situational,
    CandidateAsks,   // questions the candidate asks the interviewer
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewQuestion {
    pub id: QuestionId,
    pub application_id: Option<Uuid>,     // None = generic bank question
    pub interview_type: InterviewType,
    pub question_type: QuestionType,
    pub question: String,
    pub ideal_answer: Option<String>,     // LLM-generated guidance
    pub tips: Vec<String>,                // stored as JSON array in SQLite
    pub star_scaffold: Option<StarScaffold>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarScaffold {
    pub situation_prompt: String,
    pub task_prompt: String,
    pub action_prompt: String,
    pub result_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepSession {
    pub id: PrepSessionId,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub questions: Vec<InterviewQuestion>,
    pub company_insights: CompanyInsights,
    pub talking_points: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyInsights {
    pub company_name: String,
    pub tech_stack: Vec<String>,
    pub culture_notes: Vec<String>,
    pub recent_news: Vec<String>,
    pub interview_process_notes: Option<String>, // from Glassdoor/Blind
    pub suggested_questions_to_ask: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockInterviewSession {
    pub id: PrepSessionId,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub turns: Vec<InterviewTurn>,
    pub overall_feedback: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewTurn {
    pub question: InterviewQuestion,
    pub user_answer: String,
    pub llm_feedback: Option<TurnFeedback>,
    pub answered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFeedback {
    pub star_completeness: StarCompleteness,
    pub strengths: Vec<String>,
    pub improvements: Vec<String>,
    pub score: u8,  // 0-10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarCompleteness {
    pub situation: bool,
    pub task: bool,
    pub action: bool,
    pub result: bool,
}

#[derive(Debug, Clone)]
pub struct InterviewPrepRequest {
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub focus_areas: Vec<String>,
}
```

#### Trait Definitions

```rust
// lazyjob-core/src/interview/service.rs

use std::sync::Arc;
use crate::llm::LlmProvider;
use crate::interview::types::*;
use crate::interview::repository::InterviewRepository;

pub struct InterviewPrepService {
    llm: Arc<dyn LlmProvider>,
    repo: Arc<InterviewRepository>,
    company_researcher: Arc<dyn CompanyResearcherTrait>,
}

#[async_trait::async_trait]
pub trait CompanyResearcherTrait: Send + Sync {
    async fn research(&self, company_name: &str) -> anyhow::Result<CompanyInsights>;
}

impl InterviewPrepService {
    pub async fn generate_prep_session(
        &self,
        req: InterviewPrepRequest,
        job: &crate::jobs::Job,
    ) -> anyhow::Result<PrepSession>;

    pub async fn generate_questions(
        &self,
        job: &crate::jobs::Job,
        interview_type: InterviewType,
        n: usize,
    ) -> anyhow::Result<Vec<InterviewQuestion>>;

    pub async fn start_mock_session(
        &self,
        application_id: Uuid,
        interview_type: InterviewType,
    ) -> anyhow::Result<MockInterviewSession>;

    pub async fn submit_answer(
        &self,
        session_id: PrepSessionId,
        question_id: QuestionId,
        answer: String,
    ) -> anyhow::Result<TurnFeedback>;

    pub async fn finalize_session(
        &self,
        session_id: PrepSessionId,
    ) -> anyhow::Result<String>; // overall feedback text
}
```

#### SQLite Schema

```sql
-- Migration 010: interview_prep tables

CREATE TABLE IF NOT EXISTS interview_questions (
    id          TEXT PRIMARY KEY,
    application_id TEXT REFERENCES applications(id) ON DELETE SET NULL,
    interview_type TEXT NOT NULL,
    question_type TEXT NOT NULL,
    question    TEXT NOT NULL,
    ideal_answer TEXT,
    tips        TEXT NOT NULL DEFAULT '[]',   -- JSON array
    star_scaffold TEXT,                        -- JSON object or NULL
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_interview_questions_application
    ON interview_questions(application_id);

CREATE TABLE IF NOT EXISTS prep_sessions (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type  TEXT NOT NULL,
    questions_json  TEXT NOT NULL,            -- JSON array of question IDs + order
    company_insights_json TEXT NOT NULL,
    talking_points_json   TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS mock_interview_sessions (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type  TEXT NOT NULL,
    turns_json      TEXT NOT NULL DEFAULT '[]',  -- JSON array of InterviewTurn
    overall_feedback TEXT,
    started_at      TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_mock_sessions_application
    ON mock_interview_sessions(application_id);
```

#### Module Structure

```
lazyjob-core/
  src/
    interview/
      mod.rs        -- re-exports
      types.rs      -- all domain types above
      repository.rs -- InterviewRepository (CRUD, sqlx queries)
      service.rs    -- InterviewPrepService (orchestration)
      prompts.rs    -- prompt assembly helpers

lazyjob-ralph/
  src/
    loops/
      mock_interview.rs  -- MockInterviewLoop subprocess handler

lazyjob-tui/
  src/
    panels/
      interview_prep.rs  -- PrepPanel widget
      mock_session.rs    -- MockSessionPanel widget
```

### Implementation Phases

#### Phase 1 — Question Bank & Prep Session (MVP)

1. **Migration 010**: Add `interview_questions`, `prep_sessions` tables.
   - File: `lazyjob-core/migrations/010_interview_prep.sql`
   - Use `sqlx::migrate!()` macro on startup.

2. **`InterviewRepository`**: Implement CRUD.
   - `insert_question(q: &InterviewQuestion) -> Result<()>`
   - `list_questions_for_application(app_id: Uuid) -> Result<Vec<InterviewQuestion>>`
   - `insert_prep_session(s: &PrepSession) -> Result<()>`
   - `get_prep_session(id: PrepSessionId) -> Result<Option<PrepSession>>`
   - Use `sqlx::query_as!` with compile-time checks.

3. **`InterviewPrepService::generate_questions`**: Assemble a structured JSON prompt asking the LLM for N questions (2 behavioral, 2 technical, 1 culture fit, 1 candidate-asks). Parse response via `serde_json::from_str::<Vec<InterviewQuestion>>`. If parse fails, retry once with explicit schema hint in prompt.

4. **`InterviewPrepService::generate_prep_session`**: Call `generate_questions` + `company_researcher.research()` concurrently via `tokio::join!`. Persist both to SQLite. Return `PrepSession`.

5. **Verification**: `cargo test interview` — unit test with `MockLlmProvider` returning fixture JSON; assert question count and types.

#### Phase 2 — Mock Interview Loop (Ralph)

1. **`mock_interview_sessions` table** (migration 010 continuation): Add to migration SQL.

2. **`MockInterviewLoop`** in `lazyjob-ralph`:
   - Reads `application_id` and `interview_type` from stdin JSON envelope.
   - Calls `InterviewPrepService::generate_questions` (or loads existing prep session).
   - Emits each question to stdout as a Ralph `LoopMessage::Output` JSON frame.
   - Reads user answer from stdin (Ralph message `LoopMessage::Input`).
   - Calls `InterviewPrepService::submit_answer` to get `TurnFeedback`.
   - Emits feedback as another `LoopMessage::Output`.
   - After all questions, calls `finalize_session` and emits summary.

3. **`InterviewPrepService::submit_answer`**: Construct STAR completeness evaluation prompt. Parse JSON feedback. Persist turn to `mock_interview_sessions.turns_json` via append.

4. **TUI `MockSessionPanel`**: Shows question in top pane, scrollable answer input area, and feedback sidebar. Tracks session state: `Idle → Answering → Reviewing → Complete`.

5. **Verification**: `cargo test mock_interview` — drive mock loop with preset answers, assert feedback contains `score` in 0-10 range.

#### Phase 3 — Question Bank Browser & Analytics

1. **TUI `PrepPanel`**: Filterable list of saved questions by interview type and question type. Vim-style navigation. Press `Enter` to expand question + ideal answer. Press `p` to start mock session for current application.

2. **Question persistence across applications**: When generating for a new application, check if similar question text already exists (Jaro-Winkler similarity > 0.85 via `strsim`) and reuse to build a shared question bank.

3. **Session analytics**: After N mock sessions, display trend chart (sparkline via `ratatui::widgets::Sparkline`) of per-session average score.

---

## Part 2: Salary Negotiation

### Architecture

#### Crate Placement
- `lazyjob-core/src/salary/` — domain types, repositories, `SalaryService`
- `lazyjob-tui/src/panels/salary.rs` — offer comparison and negotiation TUI view

#### Core Types

```rust
// lazyjob-core/src/salary/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type OfferId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferedComp {
    pub base_salary: i64,        // annual, in cents (avoids float)
    pub bonus_percent: f32,
    pub signing_bonus: Option<i64>,
    pub equity_shares: Option<i64>,
    pub equity_cliff_months: u32,
    pub equity_vest_months: u32,
    pub equity_current_valuation: Option<i64>, // estimated $ per share * shares
    pub currency: String,        // "USD", "EUR", etc.
    pub location: String,
    pub remote: bool,
}

impl OfferedComp {
    pub fn total_comp_annual(&self) -> i64 {
        let bonus = (self.base_salary as f64 * self.bonus_percent as f64 / 100.0) as i64;
        let equity_annual = self
            .equity_current_valuation
            .map(|v| v / self.equity_vest_months as i64 * 12)
            .unwrap_or(0);
        self.base_salary + bonus + self.signing_bonus.unwrap_or(0) / 4 + equity_annual
        // signing spread over 4 years as convention
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id: OfferId,
    pub application_id: Uuid,
    pub company_name: String,
    pub job_title: String,
    pub comp: OfferedComp,
    pub offer_date: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
    pub status: OfferStatus,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum OfferStatus {
    Received,
    UnderEvaluation,
    Negotiating,
    Accepted,
    Declined,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalaryBand {
    pub role: String,
    pub company: Option<String>,
    pub location: String,
    pub low_annual: i64,
    pub median_annual: i64,
    pub high_annual: i64,
    pub p75_annual: Option<i64>,
    pub source: SalarySource,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SalarySource {
    LevelsFyi,
    Glassdoor,
    H1bLca,
    UserProvided,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferEvaluation {
    pub offer: Offer,
    pub market_band: SalaryBand,
    pub percentile: f32,      // 0.0-1.0 where offer sits in market band
    pub gap_annual: i64,      // offer - median (negative = below market)
    pub negotiation_points: Vec<NegotiationPoint>,
    pub llm_recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationPoint {
    pub component: String,    // "base", "equity", "signing"
    pub current: i64,
    pub suggested: i64,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationStrategy {
    pub offer_id: OfferId,
    pub opening_script: String,
    pub priority_order: Vec<String>,
    pub walk_away_threshold: Option<i64>,   // total comp, in cents
    pub expected_outcome: String,
    pub counter_offer: Option<OfferedComp>,
    pub created_at: DateTime<Utc>,
}
```

#### Trait Definitions

```rust
// lazyjob-core/src/salary/service.rs

#[async_trait::async_trait]
pub trait SalaryDataSource: Send + Sync {
    fn source_name(&self) -> SalarySource;
    async fn fetch_band(
        &self,
        role: &str,
        company: Option<&str>,
        location: &str,
    ) -> anyhow::Result<Option<SalaryBand>>;
}

pub struct SalaryService {
    sources: Vec<Box<dyn SalaryDataSource>>,
    llm: Arc<dyn LlmProvider>,
    repo: Arc<SalaryRepository>,
}

impl SalaryService {
    pub async fn get_market_band(
        &self,
        role: &str,
        company: Option<&str>,
        location: &str,
    ) -> anyhow::Result<SalaryBand>;

    pub async fn evaluate_offer(&self, offer: &Offer) -> anyhow::Result<OfferEvaluation>;

    pub async fn generate_negotiation_strategy(
        &self,
        offer: &Offer,
        evaluation: &OfferEvaluation,
        user_batna: Option<i64>,   // user's best alternative, total comp
    ) -> anyhow::Result<NegotiationStrategy>;
}
```

#### Data Sources

**Levels.fyi**: No official public API. Use the undocumented JSON endpoint at `https://www.levels.fyi/js/salaryData.json` (fetched with `reqwest`, parse with `serde_json`). Cache aggressively (TTL 7 days) because data changes slowly.

**H1B LCA Data**: Public DOL dataset (`h1bdata.info` or direct FY-{year}_H1B_FY{year}_Q{n}.xlsx). Fetch and parse CSV annually, cache locally in SQLite. This is accurate for specific company+role combinations.

**Glassdoor**: No public API. Scrape the mobile API endpoint (reverse-engineered, fragile). Wrap in a `GlassdoorClient` with graceful degradation if blocked.

**Fallback**: If all sources fail, use `UserProvided` band — the user enters their own market data manually in TUI.

#### SQLite Schema

```sql
-- Migration 011: salary tables

CREATE TABLE IF NOT EXISTS offers (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    company_name    TEXT NOT NULL,
    job_title       TEXT NOT NULL,
    comp_json       TEXT NOT NULL,         -- OfferedComp as JSON
    offer_date      TEXT NOT NULL,
    deadline        TEXT,
    status          TEXT NOT NULL DEFAULT 'received',
    notes           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_offers_application ON offers(application_id);
CREATE INDEX IF NOT EXISTS idx_offers_status ON offers(status);

CREATE TABLE IF NOT EXISTS salary_band_cache (
    id              TEXT PRIMARY KEY,
    role            TEXT NOT NULL,
    company         TEXT,
    location        TEXT NOT NULL,
    source          TEXT NOT NULL,
    band_json       TEXT NOT NULL,          -- SalaryBand as JSON
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at      TEXT NOT NULL           -- fetched_at + TTL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_salary_cache_key
    ON salary_band_cache(role, COALESCE(company, ''), location, source);

CREATE TABLE IF NOT EXISTS negotiation_strategies (
    id              TEXT PRIMARY KEY,
    offer_id        TEXT NOT NULL REFERENCES offers(id) ON DELETE CASCADE,
    strategy_json   TEXT NOT NULL,          -- NegotiationStrategy as JSON
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

#### Module Structure

```
lazyjob-core/
  src/
    salary/
      mod.rs
      types.rs
      repository.rs    -- OfferRepository, SalaryBandRepository
      service.rs       -- SalaryService
      sources/
        mod.rs
        levels_fyi.rs  -- LevelsFyiSource: SalaryDataSource
        h1b_lca.rs     -- H1bLcaSource: SalaryDataSource
        glassdoor.rs   -- GlassdoorSource: SalaryDataSource (fragile)
      prompts.rs       -- negotiation prompt builders

lazyjob-tui/
  src/
    panels/
      salary.rs        -- OfferDetailPanel, SalaryComparisonWidget
      offer_list.rs    -- OfferListPanel
```

### Implementation Phases

#### Phase 1 — Offer Storage & Basic Evaluation (MVP)

1. **Migration 011**: `offers`, `salary_band_cache`, `negotiation_strategies`.

2. **`OfferRepository`**:
   - `insert_offer(o: &Offer) -> Result<()>`
   - `get_offer(id: OfferId) -> Result<Option<Offer>>`
   - `list_offers_for_application(app_id: Uuid) -> Result<Vec<Offer>>`
   - `update_offer_status(id: OfferId, status: OfferStatus) -> Result<()>`
   - `upsert_salary_band(band: &SalaryBand, ttl_days: u32) -> Result<()>`
   - `get_cached_band(role: &str, company: Option<&str>, location: &str) -> Result<Option<SalaryBand>>`

3. **`LevelsFyiSource`**: Fetch `https://www.levels.fyi/js/salaryData.json` with `reqwest`. Filter by title and company. Compute p25/p50/p75 from raw data points. Implement with `#[async_trait]`.

4. **`SalaryService::evaluate_offer`**: Call `get_market_band` (cache-first), compute `percentile` = `(offer_tc - low) / (high - low)`, produce `NegotiationPoint`s for each comp component that is > 5% below market median.

5. **Verification**: Integration test fetching band from fixture JSON (injected via `MockSalaryDataSource`), asserting percentile calculation.

#### Phase 2 — Negotiation Strategy Generation

1. **`SalaryService::generate_negotiation_strategy`**: Build structured prompt including: offer breakdown in table format, market data, gap analysis, user's BATNA (if provided). Request JSON with fields: `opening_script`, `priority_order: Vec<String>`, `walk_away_threshold: Option<i64>`, `expected_outcome`, `counter_offer: OfferedComp`.

2. **`NegotiationStrategyRepository`**: Persist strategies; allow listing all strategies for an offer.

3. **TUI `OfferDetailPanel`**: Shows offer comp breakdown, market band bar chart (ratatui `BarChart`), percentile indicator, negotiation points. Keybind `n` triggers strategy generation via Ralph loop message (async).

#### Phase 3 — Multi-Offer Comparison

1. **`SalaryComparisonWidget`**: Side-by-side table of up to 4 offers. TC normalized to same currency. Color-code highest value per row green. Keybind `a`/`d` to cycle offers.

2. **`OfferedComp::scenario_model`**: Accept override fields, return modified TC for "what-if" scenarios (e.g., "what if I negotiate base to $200k?").

---

## Part 3: Networking & Referrals

### Architecture

#### Crate Placement
- `lazyjob-core/src/networking/` — domain types, contact repository, `NetworkingService`
- `lazyjob-tui/src/panels/networking.rs` — contact browser and outreach panel

#### Core Types

```rust
// lazyjob-core/src/networking/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ContactId = Uuid;
pub type OutreachId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ConnectionDegree {
    First,
    Second,
    Third,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: ContactId,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin_url: Option<String>,
    pub github_url: Option<String>,
    pub current_company: Option<String>,
    pub current_role: Option<String>,
    pub location: Option<String>,
    pub degree: ConnectionDegree,
    pub relationship_strength: f32,    // 0.0-1.0 derived from interaction count
    pub notes: Option<String>,
    pub tags: Vec<String>,              // stored as JSON array
    pub source: ContactSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactSource {
    ManualEntry,
    LinkedInCsvImport,
    VCardImport,
    GoogleContactsImport,
}

/// Links a contact to a target company (for warm path tracking)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCompanyLink {
    pub contact_id: ContactId,
    pub company_name: String,
    pub role_at_company: Option<String>,
    pub is_current: bool,
    pub warm_path_score: f32,   // 0.0-1.0; higher = warmer intro potential
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum OutreachType {
    ColdOutreach,
    ReferralRequest,
    ThankYou,
    FollowUp,
    IntroductionRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutreachDraft {
    pub id: OutreachId,
    pub contact_id: ContactId,
    pub application_id: Option<Uuid>,
    pub outreach_type: OutreachType,
    pub subject: Option<String>,
    pub body: String,
    pub tone: OutreachTone,
    pub status: OutreachStatus,
    pub sent_at: Option<DateTime<Utc>>,
    pub follow_up_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutreachTone {
    Casual,
    Professional,
    Warm,
    Formal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum OutreachStatus {
    Draft,
    Sent,
    Replied,
    NoResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ReferralStatus {
    Requested,
    Pending,
    Submitted,
    Declined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralRequest {
    pub id: Uuid,
    pub contact_id: ContactId,
    pub application_id: Uuid,
    pub status: ReferralStatus,
    pub requested_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}
```

#### Trait Definitions

```rust
// lazyjob-core/src/networking/service.rs

pub struct NetworkingService {
    llm: Arc<dyn LlmProvider>,
    repo: Arc<ContactRepository>,
}

impl NetworkingService {
    pub async fn find_contacts_for_company(
        &self,
        company_name: &str,
    ) -> anyhow::Result<Vec<Contact>>;

    pub async fn warm_path_score(
        &self,
        contact: &Contact,
        target_company: &str,
    ) -> f32;

    pub async fn generate_outreach(
        &self,
        contact: &Contact,
        outreach_type: OutreachType,
        context: &OutreachContext,
    ) -> anyhow::Result<OutreachDraft>;

    pub async fn import_contacts_csv(
        &self,
        csv_path: &std::path::Path,
        source: ContactSource,
    ) -> anyhow::Result<ImportResult>;
}

pub struct OutreachContext {
    pub your_name: String,
    pub job_title: String,
    pub company_name: String,
    pub shared_connection: Option<String>,
    pub specific_interest: Option<String>,  // "I saw your talk on X"
}

pub struct ImportResult {
    pub imported: usize,
    pub skipped_duplicates: usize,
    pub errors: Vec<String>,
}
```

#### Outreach Generation

The LLM prompt for outreach follows a structured template:

```
You are writing a {outreach_type} message from {your_name} to {contact_name} ({contact_role} at {company}).

Context:
- Connection degree: {degree}
- Shared connection: {shared_connection or "none"}
- Your goal: {goal}

Write a {tone} message under 150 words. Do NOT mention salary or ask for a job directly.
Output ONLY the message body, no subject line.
```

For `ReferralRequest`, append: "Ask for a referral for {job_title} at {company}. Make it easy to say no."

#### SQLite Schema

```sql
-- Migration 012: networking tables

CREATE TABLE IF NOT EXISTS contacts (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    email           TEXT,
    phone           TEXT,
    linkedin_url    TEXT,
    github_url      TEXT,
    current_company TEXT,
    current_role    TEXT,
    location        TEXT,
    degree          TEXT NOT NULL DEFAULT 'unknown',
    relationship_strength REAL NOT NULL DEFAULT 0.0,
    notes           TEXT,
    tags_json       TEXT NOT NULL DEFAULT '[]',
    source          TEXT NOT NULL DEFAULT 'manual_entry',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_contacts_company ON contacts(current_company);
CREATE INDEX IF NOT EXISTS idx_contacts_email ON contacts(email);

CREATE TABLE IF NOT EXISTS contact_company_links (
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    company_name    TEXT NOT NULL,
    role_at_company TEXT,
    is_current      INTEGER NOT NULL DEFAULT 1,
    warm_path_score REAL NOT NULL DEFAULT 0.0,
    PRIMARY KEY (contact_id, company_name)
);

CREATE INDEX IF NOT EXISTS idx_ccl_company ON contact_company_links(company_name);

CREATE TABLE IF NOT EXISTS outreach_drafts (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    application_id  TEXT REFERENCES applications(id) ON DELETE SET NULL,
    outreach_type   TEXT NOT NULL,
    subject         TEXT,
    body            TEXT NOT NULL,
    tone            TEXT NOT NULL DEFAULT 'professional',
    status          TEXT NOT NULL DEFAULT 'draft',
    sent_at         TEXT,
    follow_up_at    TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_outreach_contact ON outreach_drafts(contact_id);
CREATE INDEX IF NOT EXISTS idx_outreach_follow_up ON outreach_drafts(follow_up_at)
    WHERE follow_up_at IS NOT NULL AND status != 'replied';

CREATE TABLE IF NOT EXISTS referral_requests (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'requested',
    requested_at    TEXT NOT NULL DEFAULT (datetime('now')),
    submitted_at    TEXT,
    notes           TEXT,
    UNIQUE(contact_id, application_id)
);
```

#### Module Structure

```
lazyjob-core/
  src/
    networking/
      mod.rs
      types.rs
      repository.rs    -- ContactRepository, OutreachRepository, ReferralRepository
      service.rs       -- NetworkingService
      import/
        mod.rs
        linkedin_csv.rs -- parse LinkedIn CSV export format
        vcard.rs        -- parse .vcf files

lazyjob-tui/
  src/
    panels/
      networking.rs    -- ContactBrowserPanel
      outreach.rs      -- OutreachDraftPanel
```

### Implementation Phases

#### Phase 1 — Contact Storage & Import (MVP)

1. **Migration 012**: All networking tables.

2. **`ContactRepository`**:
   - `insert_contact(c: &Contact) -> Result<ContactId>`
   - `find_by_company(company: &str) -> Result<Vec<Contact>>`
   - `search(query: &str) -> Result<Vec<Contact>>` — SQLite FTS5 or LIKE
   - `upsert_company_link(link: &ContactCompanyLink) -> Result<()>`

3. **`NetworkingService::import_contacts_csv`**: Parse LinkedIn CSV export (columns: `First Name`, `Last Name`, `Email Address`, `Company`, `Position`). Normalize, dedup by email (if present) or `name + company`. Emit `ImportResult`. Wrap with `tokio::task::spawn_blocking` since CSV I/O is sync.

4. **TUI `ContactBrowserPanel`**: Searchable list, fuzzy filter by name/company. Show contact degree badge. Press `o` to open outreach draft panel.

5. **Verification**: Unit test `import_contacts_csv` with fixture CSV; assert deduplication works.

#### Phase 2 — Outreach Generation

1. **`NetworkingService::generate_outreach`**: Build prompt, call LLM, persist draft.

2. **`OutreachRepository`**:
   - `insert_draft(d: &OutreachDraft) -> Result<OutreachId>`
   - `mark_sent(id: OutreachId) -> Result<()>`
   - `list_pending_follow_ups(before: DateTime<Utc>) -> Result<Vec<OutreachDraft>>`

3. **TUI `OutreachDraftPanel`**: Editable text area with vim-mode bindings. Keybind `g` to generate (replace body with LLM output). Keybind `s` to mark as sent. Shows follow-up date if set.

4. **Referral tracking**: `ReferralRepository::upsert_request`, `update_status`. Show in application detail view.

#### Phase 3 — Warm Path & Analytics

1. **Warm path score**: `warm_path_score(contact, company)` = `(1.0 - degree_penalty) * relationship_strength`, where `degree_penalty` is 0.0 for First, 0.3 for Second, 0.6 for Third. Store result in `contact_company_links.warm_path_score`.

2. **Warm path finder**: Given `application_id`, query `contact_company_links` for `company_name = job.company_name`, order by `warm_path_score DESC`, return top 5.

3. **Networking dashboard**: Show contacts with pending follow-ups, referral pipeline stats (requested/pending/submitted), and top warm paths for active applications.

---

## Part 4: Notifications & Morning Brief

### Architecture

#### Crate Placement
- `lazyjob-core/src/notifications/` — domain types, scheduler, `NotificationService`
- `lazyjob-tui/src/panels/notifications.rs` — in-TUI notification overlay
- `lazyjob-tui/src/panels/morning_brief.rs` — morning brief dashboard view

#### Core Types

```rust
// lazyjob-core/src/notifications/types.rs

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type NotificationId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum NotificationType {
    MorningBrief,
    InterviewReminder,
    FollowUpReminder,
    ApplicationUpdate,
    NewJobMatch,
    OfferDeadline,
    WeeklySummary,
    OutreachFollowUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum Priority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: NotificationId,
    pub notification_type: NotificationType,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub source_entity_id: Option<Uuid>,   // e.g. application_id or contact_id
    pub source_entity_type: Option<String>,
    pub created_at: DateTime<Utc>,
    pub scheduled_for: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorningBrief {
    pub date: chrono::NaiveDate,
    pub pipeline_stats: PipelineStats,
    pub action_items: Vec<ActionItem>,
    pub new_matches: Vec<crate::jobs::Job>,
    pub upcoming_interviews: Vec<ScheduledInterview>,
    pub llm_summary: String,           // 2-3 sentence overview
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStats {
    pub total_applications: u32,
    pub active: u32,
    pub phone_screens: u32,
    pub technical: u32,
    pub onsites: u32,
    pub offers: u32,
    pub rejected: u32,
    pub response_rate: f32,            // applied / (applied + rejected)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub priority: Priority,
    pub title: String,
    pub description: String,
    pub due: Option<DateTime<Utc>>,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledInterview {
    pub application_id: Uuid,
    pub company_name: String,
    pub job_title: String,
    pub interview_type: InterviewType,
    pub scheduled_at: DateTime<Utc>,
    pub location: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub morning_brief_enabled: bool,
    pub morning_brief_time: NaiveTime,    // local time, e.g. 08:00
    pub desktop_notifications: bool,
    pub interview_reminder_hours_before: Vec<u32>,   // e.g. [24, 1]
    pub follow_up_reminder_days: u32,
}
```

#### Scheduler Design

Use the `cron` crate for expression parsing and `tokio::time::sleep_until` for scheduling. The scheduler runs as a background tokio task, not a subprocess:

```rust
// lazyjob-core/src/notifications/scheduler.rs

pub struct NotificationScheduler {
    config: NotificationConfig,
    service: Arc<NotificationService>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
}

impl NotificationScheduler {
    pub async fn run(mut self) {
        loop {
            let next_tick = self.compute_next_wake();
            tokio::select! {
                _ = tokio::time::sleep_until(next_tick) => {
                    self.service.process_due_notifications().await
                        .unwrap_or_else(|e| tracing::error!(?e, "notification tick failed"));
                }
                _ = self.shutdown.recv() => break,
            }
        }
    }

    fn compute_next_wake(&self) -> tokio::time::Instant {
        // Check all scheduled_for times in notifications table
        // Return earliest or 60s fallback
        todo!()
    }
}
```

#### Desktop Notifications

Use `notify-rust` crate for desktop notifications on Linux (libnotify) and macOS (NSUserNotification). Call from the scheduler when a notification comes due:

```rust
// Linux/macOS
notify_rust::Notification::new()
    .summary(&notification.title)
    .body(&notification.body)
    .timeout(notify_rust::Timeout::Milliseconds(8000))
    .show()?;
```

Wrap in `#[cfg(not(test))]` to skip in tests.

#### SQLite Schema

```sql
-- Migration 013: notifications tables

CREATE TABLE IF NOT EXISTS notifications (
    id                  TEXT PRIMARY KEY,
    notification_type   TEXT NOT NULL,
    title               TEXT NOT NULL,
    body                TEXT NOT NULL,
    priority            TEXT NOT NULL DEFAULT 'medium',
    source_entity_id    TEXT,
    source_entity_type  TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    scheduled_for       TEXT NOT NULL,
    delivered_at        TEXT,
    dismissed_at        TEXT
);

CREATE INDEX IF NOT EXISTS idx_notifications_scheduled
    ON notifications(scheduled_for)
    WHERE delivered_at IS NULL AND dismissed_at IS NULL;

CREATE TABLE IF NOT EXISTS notification_config (
    id                              INTEGER PRIMARY KEY CHECK (id = 1),
    morning_brief_enabled           INTEGER NOT NULL DEFAULT 1,
    morning_brief_time              TEXT NOT NULL DEFAULT '08:00',
    desktop_notifications           INTEGER NOT NULL DEFAULT 1,
    interview_reminder_hours_json   TEXT NOT NULL DEFAULT '[24, 1]',
    follow_up_reminder_days         INTEGER NOT NULL DEFAULT 3,
    updated_at                      TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO notification_config (id) VALUES (1);

CREATE TABLE IF NOT EXISTS morning_briefs (
    id          TEXT PRIMARY KEY,
    date        TEXT NOT NULL UNIQUE,
    brief_json  TEXT NOT NULL,              -- MorningBrief as JSON
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

#### Module Structure

```
lazyjob-core/
  src/
    notifications/
      mod.rs
      types.rs
      repository.rs        -- NotificationRepository
      service.rs           -- NotificationService
      scheduler.rs         -- NotificationScheduler (background tokio task)
      morning_brief.rs     -- MorningBriefGenerator

lazyjob-tui/
  src/
    panels/
      notifications.rs     -- NotificationOverlay (popup list)
      morning_brief.rs     -- MorningBriefView (full-screen dashboard)
```

### Implementation Phases

#### Phase 1 — Notification Storage & In-TUI Alerts (MVP)

1. **Migration 013**: `notifications`, `notification_config`, `morning_briefs`.

2. **`NotificationRepository`**:
   - `insert(n: &Notification) -> Result<NotificationId>`
   - `list_undelivered_due(now: DateTime<Utc>) -> Result<Vec<Notification>>`
   - `mark_delivered(id: NotificationId) -> Result<()>`
   - `mark_dismissed(id: NotificationId) -> Result<()>`

3. **`NotificationService::process_due_notifications`**: Called by scheduler every 60s. Queries `list_undelivered_due(Utc::now())`. For each: mark delivered, send to TUI via `tokio::sync::broadcast::Sender<Notification>`, optionally send desktop notification.

4. **TUI `NotificationOverlay`**: Floating panel (top-right corner, max 5 items). Populated from broadcast receiver. Each item shows title + age. Press `d` to dismiss, `Enter` to navigate to entity. Auto-dismisses after 30s for Medium/Low, never auto-dismisses High.

5. **Verification**: Test `process_due_notifications` with in-memory SQLite; assert broadcast receiver receives notification within 100ms of `scheduled_for`.

#### Phase 2 — Morning Brief Generator

1. **`MorningBriefGenerator::generate`**:
   - Query `PipelineStats` from `applications` table (COUNT + GROUP BY stage).
   - Query upcoming interviews from `mock_interview_sessions` and `applications` (filter `next_interview_at` within 48h).
   - Query pending follow-ups from `outreach_drafts` where `follow_up_at < now`.
   - Query new job matches from `jobs` where `discovered_at > now - 24h AND match_score > 0.7`.
   - Build `ActionItem`s from all the above, sorted by priority then due date.
   - Call LLM to generate `llm_summary` (2-3 sentences). Use prompt: "Summarize this job search status briefly and encouragingly: {stats_json}".
   - Persist to `morning_briefs`, schedule desktop notification.

2. **`NotificationScheduler`**: On startup, schedule morning brief according to `notification_config.morning_brief_time`. Use `cron::Schedule::from_str("0 {min} {hour} * * *")` (the `cron` crate uses 6-field format). Compute next occurrence relative to `Utc::now()` with local timezone conversion via `chrono_tz`.

3. **TUI `MorningBriefView`**: Full-screen view, triggered by keybind `b` from main view. Shows: stats bar at top (ratatui `Gauge` widgets for pipeline stages), action items list (scrollable, color-coded by priority), new matches carousel (max 5, press Enter to view job detail), LLM summary in a `Paragraph` block.

#### Phase 3 — Interview Reminders & Outreach Follow-ups

1. **Interview reminder scheduling**: When an interview is scheduled (from application workflow), insert `Notification` rows for each time in `interview_reminder_hours_before`. E.g., for a 9am interview: reminders at T-24h and T-1h.

2. **Outreach follow-up**: When an outreach is marked sent with `follow_up_at` set, insert a `Notification` of type `OutreachFollowUp` scheduled for that date.

3. **Offer deadline**: When offer deadline is within 48h, insert `Priority::High` notification.

4. **Weekly summary**: Every Sunday at `morning_brief_time`, generate a `WeeklySummary` notification with the week's stats diff.

---

## Key Crate APIs

### Interview Prep
- `sqlx::query_as!(InterviewQuestion, "SELECT ... FROM interview_questions WHERE application_id = ?", id)` — compile-time checked query
- `tokio::join!(generate_questions(job, type), company_researcher.research(&job.company_name))` — concurrent async calls
- `strsim::jaro_winkler(a, b) -> f64` — fuzzy question dedup
- `serde_json::from_str::<Vec<InterviewQuestion>>(&llm_response)` — parse LLM JSON output

### Salary
- `reqwest::Client::get(url).send().await?.json::<serde_json::Value>().await?` — fetch levels.fyi
- `sqlx::query!("INSERT OR REPLACE INTO salary_band_cache ...", ...)` — cache upsert
- Percentile: `(offer_tc - low) as f32 / (high - low) as f32` — simple linear percentile

### Networking
- `csv::Reader::from_path(path)` — LinkedIn CSV import (`csv` crate, sync, wrap in `spawn_blocking`)
- `sqlx::query!("INSERT OR IGNORE INTO contacts ...")` — dedup on email uniqueness

### Notifications
- `notify_rust::Notification::new().summary(title).body(body).show()?` — desktop notification
- `cron::Schedule::from_str("0 0 8 * * *").unwrap()` — cron expression parsing
- `chrono::Local::now().with_time(naive_time)` — compute next local-time occurrence
- `tokio::time::sleep_until(instant).await` — async sleep until next trigger
- `tokio::sync::broadcast::channel::<Notification>(64)` — TUI notification bus

## Error Handling

```rust
// lazyjob-core/src/interview/error.rs
#[derive(thiserror::Error, Debug)]
pub enum InterviewError {
    #[error("LLM failed to generate questions: {0}")]
    LlmGenerationFailed(#[from] anyhow::Error),
    #[error("failed to parse LLM question JSON: {0}")]
    QuestionParseFailed(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("session not found: {0}")]
    SessionNotFound(uuid::Uuid),
}

// lazyjob-core/src/salary/error.rs
#[derive(thiserror::Error, Debug)]
pub enum SalaryError {
    #[error("all salary data sources failed")]
    AllSourcesFailed,
    #[error("HTTP error fetching salary data: {0}")]
    Http(#[from] reqwest::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("offer not found: {0}")]
    OfferNotFound(uuid::Uuid),
}

// lazyjob-core/src/networking/error.rs
#[derive(thiserror::Error, Debug)]
pub enum NetworkingError {
    #[error("CSV import failed: {0}")]
    CsvImport(String),
    #[error("contact not found: {0}")]
    ContactNotFound(uuid::Uuid),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("LLM outreach generation failed: {0}")]
    OutreachGenerationFailed(#[from] anyhow::Error),
}

// lazyjob-core/src/notifications/error.rs
#[derive(thiserror::Error, Debug)]
pub enum NotificationError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("desktop notification failed: {0}")]
    DesktopNotification(String),
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),
}
```

## Testing Strategy

### Unit Tests

**Interview Prep**:
- `test_generate_questions_parses_llm_json`: Use `MockLlmProvider` returning fixture JSON string. Assert `Vec<InterviewQuestion>` has correct length and types.
- `test_star_scaffold_generated_for_behavioral`: Assert `star_scaffold` is `Some` for `QuestionType::Behavioral`.
- `test_question_dedup_reuses_similar`: Insert two near-identical questions, assert second uses existing ID.

**Salary**:
- `test_total_comp_calculation`: Assert `OfferedComp::total_comp_annual()` with known inputs.
- `test_percentile_calculation`: Market band low=$100k, high=$200k, offer TC=$150k → percentile=0.5.
- `test_salary_cache_hit_avoids_network`: Pre-insert band in SQLite, assert `SalaryDataSource::fetch_band` is never called.

**Networking**:
- `test_linkedin_csv_import`: Parse fixture CSV, assert 5 contacts imported with correct name/company.
- `test_import_deduplicates_by_email`: Same email twice → 1 inserted, 1 skipped.
- `test_warm_path_score_first_degree`: First-degree contact at target company → score ≈ 0.9.

**Notifications**:
- `test_morning_brief_action_items_sorted_by_priority`: High items first.
- `test_notification_broadcast_delivery`: Insert scheduled notification, call `process_due_notifications`, assert broadcast receiver gets it.
- `test_interview_reminder_creates_two_notifications`: Schedule interview 48h away, assert two reminders inserted (24h and 1h before).

### Integration Tests

- `test_full_interview_prep_flow`: Create application → generate prep session → start mock session → submit 3 answers → finalize → assert overall_feedback populated.
- `test_offer_evaluation_with_cached_band`: Insert band in cache → evaluate offer → assert `OfferEvaluation` contains non-zero `negotiation_points`.
- `test_outreach_generation_and_mark_sent`: Create contact → generate outreach → mark sent with follow-up → assert `OutreachFollowUp` notification inserted.

### TUI Tests

TUI panels are tested by driving `ratatui::Terminal::with_test_terminal` with simulated key events. Each panel test asserts:
- Panel renders without panic given a populated `AppState`.
- Key events trigger correct state transitions (e.g., `n` in offer detail panel changes panel mode to `GeneratingStrategy`).

## Open Questions

1. **Levels.fyi stability**: The undocumented JSON endpoint may change format. Should we build a fallback using manually scraped data from `h1b.io` or request user to enter salary data manually? Recommendation: always have `UserProvided` as fallback, never block evaluation on external data.

2. **LinkedIn ToS for contact scraping**: The spec mentions finding contacts on LinkedIn. Direct scraping violates ToS. The safest approach is CSV export import (LinkedIn allows this for your own connections). Automation via browser should be opt-in and clearly disclosed. Implement only CSV import in Phase 1.

3. **Mock interview recording**: The spec asks whether to allow self-review recordings. Audio recording requires platform permissions and adds complexity. Defer to a future spec. Text session logs are sufficient for MVP.

4. **Glassdoor API reliability**: Glassdoor's unofficial mobile API is fragile and changes without notice. Mark `GlassdoorSource` as `#[allow(dead_code)]` until a stable approach is identified. Do not block Phase 1 on this.

5. **Timezone handling for morning brief**: `NaiveTime` is stored in config; scheduler must convert to local timezone before computing next UTC trigger. Use `chrono_tz` crate with user-configured timezone (default: system local via `chrono::Local`). Add `user_timezone: String` to `notification_config`.

6. **Notification persistence vs. ephemeral**: Currently all notifications are persisted to SQLite. For high-frequency events (new job matches every hour), this can grow large. Add automatic cleanup: delete `dismissed_at IS NOT NULL AND dismissed_at < now - 7 days` on startup.

## Related Specs

- `specs/10-application-workflow.md` — provides `ApplicationStage`, interview scheduling
- `specs/08-cover-letter-generation.md` — `CompanyResearcher` shared by `InterviewPrepService`
- `specs/16-privacy-security.md` — all contact data and offer data must be covered by at-rest encryption
- `specs/17-ralph-prompt-templates.md` — mock interview loop will use Ralph subprocess protocol
- `specs/XX-interview-session-resumability.md` — extends the mock session with checkpoint/resume
- `specs/salary-negotiation-offers.md` — deeper spec for the offer/negotiation subsystem
- `specs/networking-outreach-drafting.md` — deeper spec for outreach generation
