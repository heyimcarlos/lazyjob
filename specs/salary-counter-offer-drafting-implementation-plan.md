# Implementation Plan: Counter-Offer Drafting

## Status
Draft

## Related Spec
[specs/salary-counter-offer-drafting.md](salary-counter-offer-drafting.md)

## Overview

The counter-offer drafting module gives candidates a personalized, grounded counter-offer email and phone negotiation script at the moment they receive a job offer. It is the "last mile" of the salary pipeline: upstream modules (salary market intelligence, offer evaluation, BATNA calculator) compute the numbers; this module translates those numbers into communication the candidate can copy and send.

The system is built on three strict product constraints that are non-negotiable in the implementation: (1) **Grounding** — the LLM only receives data the user has explicitly entered or that LazyJob has computed from real sources; it never invents market ranges or competing offers; (2) **Human-in-the-loop** — drafts are shown with a `[DRAFT - NOT SENT]` header; there is no send button; the user copies the text themselves; (3) **Anti-fabrication** — if the user has not entered a competing offer, the draft must not reference one, and if insufficient market data is available the service degrades gracefully to a "principle-based" counter with an explicit data quality warning shown in the TUI.

The `CounterOfferDraftService` lives in `lazyjob-core/src/salary/counter_offer.rs`. It calls the LLM via the `LlmProvider` trait, persists drafts and negotiation history to SQLite, and emits events on a `broadcast::Sender<NegotiationEvent>` so the TUI can update without polling. All computation (context assembly, comp delta) is synchronous and pure; async is limited to LLM calls and SQLite I/O.

## Prerequisites

### Must be implemented first
- `specs/salary-market-intelligence-implementation-plan.md` — `OfferEvaluation`, `OfferRecord`, `MarketDataPoint`, `compute_total_comp`, `offers` SQLite table
- `specs/salary-negotiation-offers-implementation-plan.md` — `NegotiationStatus`, `NegotiationPriorities`, `BatnaSnapshot`, `NegotiationService`, `OfferRepository`
- `specs/04-sqlite-persistence-implementation-plan.md` — connection pool, `run_migrations`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider`, `ChatMessage`, `complete()`
- `specs/17-ralph-prompt-templates-implementation-plan.md` — `TemplateEngine`, TOML template loading
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel system, `FormWidget`
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `StageTransitionEvent`

### Crates to add to Cargo.toml
```toml
# No new crates required. All dependencies already present:
# lazyjob-core: uuid, chrono, serde, serde_json, thiserror, anyhow, tokio, sqlx
# lazyjob-llm: already has LlmProvider + prompt template infrastructure
# lazyjob-tui: ratatui, crossterm, tokio::sync::broadcast
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| Core types (request, draft, outcome, history, round) | `lazyjob-core` | `src/salary/counter_offer.rs` |
| Negotiation outcome + history types | `lazyjob-core` | `src/salary/outcome.rs` |
| Context builder (pure, grounding logic) | `lazyjob-core` | `src/salary/counter_offer.rs` |
| `CounterOfferDraftService` (async orchestrator) | `lazyjob-core` | `src/salary/counter_offer.rs` |
| SQLite repository | `lazyjob-core` | `src/salary/counter_offer_repo.rs` |
| Prompt template | `lazyjob-llm` | `src/prompts/salary_negotiation.rs` |
| Prompt template TOML | `lazyjob-llm` | `templates/salary_negotiation.toml` |
| TUI draft view | `lazyjob-tui` | `src/views/salary/counter_offer.rs` |
| TUI outcome recording form | `lazyjob-tui` | `src/views/salary/negotiation_outcome.rs` |

### Core Types

```rust
// lazyjob-core/src/salary/counter_offer.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// ID newtypes — parse-don't-validate pattern
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct CounterOfferDraftId(pub Uuid);
impl CounterOfferDraftId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Tone selected by the user before generating the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NegotiationTone {
    /// Formal, data-driven — large company with HR process.
    Professional,
    /// Warm, signals genuine enthusiasm — startups.
    Enthusiastic,
    /// Direct, low hedging — use when gap > 15%.
    Assertive,
}

/// Candidate's top priority component for this negotiation.
/// Ordered by importance (first = highest priority).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NegotiationPriority {
    Base,
    Equity,
    SigningBonus,
    StartDate,
    Title,
}

/// All input the user provides to generate a draft.
/// `offer_evaluation` is computed by `SalaryIntelligenceService` and is immutable here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterOfferRequest {
    pub offer_evaluation: OfferEvaluation,
    pub tone: NegotiationTone,
    /// Ordered list: index 0 is top priority.
    pub user_priorities: Vec<NegotiationPriority>,
    /// User's target base salary in cents, if known.
    pub target_base_cents: Option<i64>,
    /// User's target annualized TC in cents, if known.
    pub target_total_cents: Option<i64>,
}

/// The generated draft — persisted to SQLite and displayed in TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterOfferDraft {
    pub id: CounterOfferDraftId,
    /// FK to `offer_details`
    pub offer_id: Uuid,
    /// FK to `applications`
    pub application_id: Uuid,
    pub email_subject: String,
    /// Full draft email text — shown in TUI with [DRAFT - NOT SENT] header.
    pub email_body: String,
    /// Bullet points for phone negotiation.
    pub talking_points: Vec<String>,
    /// Warnings about negotiation tactics (e.g. round limit).
    pub negotiation_warnings: Vec<String>,
    /// Set when market data had < 3 samples.
    pub data_quality_warning: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub tone: NegotiationTone,
}

/// The pure grounding struct passed to the LLM. Constructed from CounterOfferRequest
/// without any LLM calls. The LLM sees this, not the raw request.
#[derive(Debug, Serialize)]
pub(crate) struct NegotiationContext {
    pub company_name: String,
    pub role_title: String,
    pub company_stage: CompanyStage,
    /// Computed annualized TC in cents (base + RSU/4yr + signing/4yr + bonus).
    pub offer_annualized_cents: i64,
    /// Human-readable string like "$185,000" for LLM consumption.
    pub offer_annualized_display: String,
    /// Market p50 in cents — only present if sample_count >= 3.
    pub market_p50_cents: Option<i64>,
    pub market_p50_display: Option<String>,
    /// Gap percentage: (market_p50 - offer) / market_p50. Positive = underpaid.
    pub gap_percentage: Option<f32>,
    /// ONLY set if user has explicitly entered a competing offer.
    pub competing_offer_annualized_cents: Option<i64>,
    pub competing_offer_annualized_display: Option<String>,
    pub tone: NegotiationTone,
    /// Priorities in display order.
    pub priority_list: Vec<String>,
    /// Negotiable components based on company stage (computed, not LLM).
    pub negotiable_components: Vec<String>,
    /// User target base in display form, if set.
    pub target_base_display: Option<String>,
    /// User target total comp in display form, if set.
    pub target_total_display: Option<String>,
    pub data_quality_warning: Option<String>,
}
```

### Outcome Types

```rust
// lazyjob-core/src/salary/outcome.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How the negotiation ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NegotiationOutcome {
    Accepted {
        /// The final offer details (may equal the initial if no counter was sent).
        final_offer_id: Uuid,
    },
    Rejected,
    OfferRevised {
        /// ID of the new revised offer record.
        revised_offer_id: Uuid,
        /// Which negotiation round this revision closes (1-indexed).
        round: u8,
    },
    Deferred,
}

/// A single round: a draft was generated, something happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationRound {
    pub round_number: u8,
    pub draft_id: Uuid,
    pub outcome: NegotiationOutcome,
    pub recorded_at: DateTime<Utc>,
}

/// Immutable history of one full negotiation thread (one application).
/// Built incrementally; finalized when `outcome` is Some.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationHistory {
    pub id: Uuid,
    pub application_id: Uuid,
    pub initial_offer_id: Uuid,
    pub final_offer_id: Option<Uuid>,
    pub rounds: Vec<NegotiationRound>,
    /// None until user explicitly records an outcome.
    pub outcome: Option<NegotiationOutcome>,
    /// Denormalized from initial offer for fast delta computation.
    pub initial_annualized_cents: i64,
    pub final_annualized_cents: Option<i64>,
    /// final - initial. Negative means final was lower (unusual; flag in TUI).
    pub comp_delta_cents: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}
```

### Service Interface

```rust
// lazyjob-core/src/salary/counter_offer.rs (continued)

pub struct CounterOfferDraftService {
    llm: Arc<dyn LlmProvider>,
    offer_repo: Arc<dyn OfferRepository>,
    draft_repo: Arc<dyn CounterOfferDraftRepository>,
    history_repo: Arc<dyn NegotiationHistoryRepository>,
    event_tx: tokio::sync::broadcast::Sender<NegotiationEvent>,
}

impl CounterOfferDraftService {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        offer_repo: Arc<dyn OfferRepository>,
        draft_repo: Arc<dyn CounterOfferDraftRepository>,
        history_repo: Arc<dyn NegotiationHistoryRepository>,
        event_tx: tokio::sync::broadcast::Sender<NegotiationEvent>,
    ) -> Self { /* ... */ }

    /// Primary entry point: generate a draft email + talking points.
    /// Called from TUI after user fills in the CounterOfferRequest form.
    pub async fn generate_draft(
        &self,
        request: &CounterOfferRequest,
    ) -> Result<CounterOfferDraft, CounterOfferError>;

    /// Record the outcome of a negotiation round.
    /// On Accepted/Rejected, finalizes NegotiationHistory and updates
    /// the application stage via StageTransitionEvent.
    pub async fn record_outcome(
        &self,
        application_id: Uuid,
        draft_id: Uuid,
        outcome: NegotiationOutcome,
    ) -> Result<NegotiationHistory, CounterOfferError>;

    /// Load all drafts for a given application, ordered by generated_at DESC.
    pub async fn list_drafts_for_application(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<CounterOfferDraft>, CounterOfferError>;

    /// Pure function — builds NegotiationContext from the request.
    /// Called synchronously before the LLM call. No I/O.
    pub(crate) fn build_context(request: &CounterOfferRequest) -> NegotiationContext;

    /// Determines which components are negotiable given CompanyStage.
    /// Returns display strings for the prompt.
    pub(crate) fn negotiable_components(stage: &CompanyStage) -> Vec<&'static str>;

    /// Formats cents to "$185,000" / "$185k" depending on magnitude.
    pub(crate) fn format_cents(cents: i64) -> String;

    /// Computes gap_percentage: (market_p50 - offer_tc) / market_p50.
    /// Returns None if market_p50 is None.
    pub(crate) fn compute_gap(
        offer_cents: i64,
        market_p50: Option<i64>,
    ) -> Option<f32>;
}
```

### Repository Trait Definitions

```rust
// lazyjob-core/src/salary/counter_offer_repo.rs

#[async_trait::async_trait]
pub trait CounterOfferDraftRepository: Send + Sync {
    async fn save(&self, draft: &CounterOfferDraft) -> Result<(), CounterOfferError>;
    async fn find_by_id(
        &self,
        id: &CounterOfferDraftId,
    ) -> Result<Option<CounterOfferDraft>, CounterOfferError>;
    async fn list_for_application(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<CounterOfferDraft>, CounterOfferError>;
}

#[async_trait::async_trait]
pub trait NegotiationHistoryRepository: Send + Sync {
    async fn save(&self, history: &NegotiationHistory) -> Result<(), CounterOfferError>;
    async fn find_by_application(
        &self,
        application_id: Uuid,
    ) -> Result<Option<NegotiationHistory>, CounterOfferError>;
    async fn update_outcome(
        &self,
        history_id: Uuid,
        outcome: &NegotiationOutcome,
        final_offer_id: Option<Uuid>,
        final_annualized_cents: Option<i64>,
        completed_at: DateTime<Utc>,
    ) -> Result<(), CounterOfferError>;
}
```

### Event Type

```rust
// lazyjob-core/src/salary/counter_offer.rs

#[derive(Debug, Clone)]
pub enum NegotiationEvent {
    DraftGenerated {
        application_id: Uuid,
        draft_id: Uuid,
    },
    OutcomeRecorded {
        application_id: Uuid,
        outcome: NegotiationOutcome,
        comp_delta_cents: Option<i64>,
    },
}
```

### SQLite Schema

```sql
-- Migration: 0015_counter_offer_drafts.sql

CREATE TABLE counter_offer_drafts (
    id                         TEXT PRIMARY KEY,
    offer_id                   TEXT NOT NULL REFERENCES offer_details(id) ON DELETE CASCADE,
    application_id             TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    email_subject              TEXT NOT NULL,
    email_body                 TEXT NOT NULL,
    talking_points_json        TEXT NOT NULL,      -- JSON array of strings
    negotiation_warnings_json  TEXT NOT NULL DEFAULT '[]',
    data_quality_warning       TEXT,
    tone                       TEXT NOT NULL,      -- NegotiationTone variant
    generated_at               TEXT NOT NULL       -- ISO8601 UTC
);

CREATE INDEX idx_counter_offer_drafts_application
    ON counter_offer_drafts(application_id, generated_at DESC);

CREATE TABLE negotiation_history (
    id                       TEXT PRIMARY KEY,
    application_id           TEXT NOT NULL UNIQUE REFERENCES applications(id) ON DELETE CASCADE,
    initial_offer_id         TEXT NOT NULL REFERENCES offer_details(id),
    final_offer_id           TEXT REFERENCES offer_details(id),
    rounds_json              TEXT NOT NULL DEFAULT '[]',  -- Vec<NegotiationRound> as JSON
    outcome                  TEXT,                        -- NegotiationOutcome variant name
    initial_annualized       INTEGER NOT NULL,            -- cents
    final_annualized         INTEGER,                     -- cents
    comp_delta               INTEGER,                     -- cents, computed on close
    started_at               TEXT NOT NULL,
    completed_at             TEXT
);

CREATE INDEX idx_negotiation_history_application
    ON negotiation_history(application_id);
```

### Module Structure

```
lazyjob-core/
  src/
    salary/
      mod.rs                    -- re-exports: CounterOfferDraftService, NegotiationHistory, etc.
      counter_offer.rs          -- CounterOfferDraftService + types + build_context
      counter_offer_repo.rs     -- Repository traits + SqliteCounterOfferDraftRepository
      outcome.rs                -- NegotiationOutcome, NegotiationHistory, NegotiationRound
  migrations/
    0015_counter_offer_drafts.sql

lazyjob-llm/
  src/
    prompts/
      salary_negotiation.rs     -- NegotiationPromptBuilder::build(ctx: &NegotiationContext)
  templates/
    salary_negotiation.toml     -- TOML template with tone variants

lazyjob-tui/
  src/
    views/
      salary/
        counter_offer.rs        -- CounterOfferRequestForm + DraftDisplayPanel
        negotiation_outcome.rs  -- OutcomeRecordingForm
```

## Implementation Phases

### Phase 1 — Core Types and Schema (MVP foundation)

**Step 1.1 — Define domain types**

File: `lazyjob-core/src/salary/counter_offer.rs`

Define `CounterOfferDraftId` (newtype around `Uuid`), `NegotiationTone`, `NegotiationPriority`, `CounterOfferRequest`, `CounterOfferDraft`, and the internal `NegotiationContext` struct. All types derive `Debug, Clone, Serialize, Deserialize`. `NegotiationTone` derives `sqlx::Type` for direct column mapping.

File: `lazyjob-core/src/salary/outcome.rs`

Define `NegotiationOutcome` (enum with `Accepted`, `Rejected`, `OfferRevised`, `Deferred` variants), `NegotiationRound`, and `NegotiationHistory`. `NegotiationOutcome` is stored as a JSON string in SQLite (not as individual columns) — use `#[serde(tag = "type")]` for the enum.

**Step 1.2 — Write and apply SQLite migration**

File: `lazyjob-core/migrations/0015_counter_offer_drafts.sql`

Create `counter_offer_drafts` and `negotiation_history` tables with the DDL above. Apply via the existing `run_migrations` infrastructure.

Key API: `sqlx::migrate!("migrations/").run(&pool).await`

**Verification:** `cargo test -p lazyjob-core -- salary` — compilation and migration tests pass.

---

### Phase 2 — Context Builder (pure, no I/O)

**Step 2.1 — Implement `build_context`**

File: `lazyjob-core/src/salary/counter_offer.rs`

`CounterOfferDraftService::build_context(request: &CounterOfferRequest) -> NegotiationContext` is a pure `fn` (not `async`). It does the following in order:

1. Extract `offer_annualized_cents` from `request.offer_evaluation.offer_breakdown.total_comp_annualized_cents`.
2. Call `format_cents(offer_annualized_cents)` → `offer_annualized_display`.
3. Extract `market_p50_cents` from `request.offer_evaluation.market_data` if `sample_count >= 3`, else `None`.
4. If `market_p50_cents.is_some()`, call `compute_gap()` → `gap_percentage`.
5. Construct `data_quality_warning` if `market_data.is_empty()` or `sample_count < 3`.
6. Extract `competing_offer_annualized_cents` — **only** from `request.offer_evaluation.competing_offers` (user-entered field); if empty → `None`. Never default to a non-None value.
7. Call `negotiable_components(&company_stage)`.
8. Map `user_priorities` to display strings in order.
9. Map `target_base_cents` and `target_total_cents` to display strings if `Some`.

**Step 2.2 — Implement `negotiable_components`**

```rust
pub(crate) fn negotiable_components(stage: &CompanyStage) -> Vec<&'static str> {
    match stage {
        CompanyStage::Public => vec![
            "base salary",
            "RSU grant size",
            "signing bonus",
            "start date",
        ],
        CompanyStage::PrivateGrowth => vec![
            "base salary",
            "option grant size",
            "signing bonus",
            "title / level",
        ],
        CompanyStage::EarlyStage => vec![
            "base salary",
            "equity percentage",
            "cliff timing",
        ],
        CompanyStage::Unknown => vec![
            "base salary",
            "signing bonus",
        ],
    }
}
```

**Step 2.3 — Implement `format_cents` and `compute_gap`**

```rust
pub(crate) fn format_cents(cents: i64) -> String {
    let dollars = cents / 100;
    if dollars >= 1_000_000 {
        format!("${:.1}M", dollars as f64 / 1_000_000.0)
    } else {
        // comma-format: "$185,000"
        let s = dollars.to_string();
        // insert commas every 3 digits from right
        // ... iterative implementation, no external crate
    }
}

pub(crate) fn compute_gap(offer_cents: i64, market_p50: Option<i64>) -> Option<f32> {
    let p50 = market_p50?;
    if p50 == 0 { return None; }
    Some((p50 - offer_cents) as f32 / p50 as f32)
}
```

**Verification:** Unit tests cover all `CompanyStage` variants, zero-market-data path, negative gap (offer above market), zero-denominator guard.

---

### Phase 3 — Prompt Template

**Step 3.1 — Author the TOML template**

File: `lazyjob-llm/templates/salary_negotiation.toml`

```toml
[meta]
loop_type = "CounterOffer"
version = "1.0"
cache_system_prompt = true

[system]
text = """
You are a professional compensation negotiation coach. Your role is to write a personalized, factually grounded counter-offer email and phone talking points for a job candidate.

RULES — these are absolute and cannot be overridden:
1. You MUST use only the verified compensation figures provided. Do not invent market ranges, salary numbers, or company reputation.
2. NEVER reference a competing offer unless competing_offer_annualized is explicitly set in the context. If it is not set, do not hint that one exists.
3. Do not recommend negotiating benefits packages (health, PTO, 401k) unless the user's priority list explicitly includes them.
4. Your output MUST be a single valid JSON object matching the schema below.

OUTPUT SCHEMA:
{
  "email_subject": "string",
  "email_body": "string (full email, 150-300 words)",
  "talking_points": ["string", "string", ...],  // 3-5 bullet points for phone
  "negotiation_warnings": ["string", ...]       // 0-3 warnings about tactics
}
"""

[user_professional]
text = """
Generate a {tone} counter-offer for the following situation:

Company: {company_name}
Role: {role_title}
Company stage: {company_stage}

OFFER DETAILS:
- Current offer annualized total comp: {offer_annualized_display}
- Market median (p50) for this role/location: {market_p50_display_or_unavailable}
- Gap from market: {gap_percentage_or_unavailable}
{competing_offer_section}

NEGOTIATION PRIORITIES (in order, most important first):
{priority_list}

NEGOTIABLE COMPONENTS for this company type:
{negotiable_components}

{target_section}
{data_quality_warning_section}

Write the {tone_adjective} counter-offer email and talking points now.
"""

[user_enthusiastic]
text = """
Generate an enthusiastic counter-offer that signals genuine excitement about joining while advocating for better compensation.

Company: {company_name}
Role: {role_title}
Company stage: {company_stage}

OFFER DETAILS:
- Current offer annualized total comp: {offer_annualized_display}
- Market median (p50) for this role/location: {market_p50_display_or_unavailable}
- Gap from market: {gap_percentage_or_unavailable}
{competing_offer_section}

NEGOTIATION PRIORITIES (in order):
{priority_list}

NEGOTIABLE COMPONENTS:
{negotiable_components}

{target_section}
{data_quality_warning_section}

The tone should be warm and collaborative — make it clear the candidate is excited about the opportunity.
"""

[user_assertive]
text = """
Generate a direct, assertive counter-offer. The candidate has a significant gap from market ({gap_percentage_or_unavailable}) and needs to advocate firmly.

Company: {company_name}
Role: {role_title}
Company stage: {company_stage}

OFFER DETAILS:
- Current offer annualized total comp: {offer_annualized_display}
- Market median (p50) for this role/location: {market_p50_display_or_unavailable}
- Gap from market: {gap_percentage_or_unavailable}
{competing_offer_section}

NEGOTIATION PRIORITIES (in order):
{priority_list}

NEGOTIABLE COMPONENTS:
{negotiable_components}

{target_section}
{data_quality_warning_section}

Write a confident, professional counter-offer. Less hedging, clear ask, maintain respect.
"""
```

**Step 3.2 — Implement `NegotiationPromptBuilder`**

File: `lazyjob-llm/src/prompts/salary_negotiation.rs`

```rust
use crate::prompts::template::{TemplateEngine, RenderedPrompt};
use lazyjob_core::salary::counter_offer::NegotiationContext;

pub struct NegotiationPromptBuilder {
    engine: TemplateEngine,
}

impl NegotiationPromptBuilder {
    pub fn new() -> Self {
        let toml_src = include_str!("../../templates/salary_negotiation.toml");
        Self { engine: TemplateEngine::from_toml(toml_src) }
    }

    pub fn build(&self, ctx: &NegotiationContext) -> RenderedPrompt {
        let vars = self.to_vars(ctx);
        let user_key = match ctx.tone {
            NegotiationTone::Professional => "user_professional",
            NegotiationTone::Enthusiastic => "user_enthusiastic",
            NegotiationTone::Assertive => "user_assertive",
        };
        self.engine.render("system", "meta", user_key, &vars)
    }

    fn to_vars(ctx: &NegotiationContext) -> HashMap<&'static str, String> {
        let mut vars = HashMap::new();
        vars.insert("company_name", ctx.company_name.clone());
        vars.insert("role_title", ctx.role_title.clone());
        vars.insert("company_stage", format!("{:?}", ctx.company_stage));
        vars.insert("offer_annualized_display", ctx.offer_annualized_display.clone());
        vars.insert(
            "market_p50_display_or_unavailable",
            ctx.market_p50_display.clone().unwrap_or_else(|| "not available".to_owned()),
        );
        vars.insert(
            "gap_percentage_or_unavailable",
            ctx.gap_percentage
                .map(|g| format!("{:.1}% below market", g * 100.0))
                .unwrap_or_else(|| "not available".to_owned()),
        );
        vars.insert(
            "competing_offer_section",
            ctx.competing_offer_annualized_display
                .as_ref()
                .map(|d| format!("- Competing offer: {d}"))
                .unwrap_or_default(),
        );
        vars.insert("priority_list", ctx.priority_list.join("\n- "));
        vars.insert("negotiable_components", ctx.negotiable_components.join(", "));
        vars.insert(
            "target_section",
            build_target_section(ctx.target_base_display.as_deref(), ctx.target_total_display.as_deref()),
        );
        vars.insert(
            "data_quality_warning_section",
            ctx.data_quality_warning.clone().unwrap_or_default(),
        );
        vars.insert("tone_adjective", tone_adjective(&ctx.tone).to_owned());
        vars
    }
}

fn build_target_section(base: Option<&str>, total: Option<&str>) -> String {
    match (base, total) {
        (Some(b), Some(t)) => format!("TARGET: base {b}, total comp {t}"),
        (Some(b), None) => format!("TARGET BASE: {b}"),
        (None, Some(t)) => format!("TARGET TOTAL COMP: {t}"),
        (None, None) => String::new(),
    }
}

fn tone_adjective(tone: &NegotiationTone) -> &'static str {
    match tone {
        NegotiationTone::Professional => "professional",
        NegotiationTone::Enthusiastic => "enthusiastic",
        NegotiationTone::Assertive => "assertive",
    }
}
```

**Verification:** Unit test `build_professional_prompt_omits_competing_offer_when_none` asserts the rendered system prompt does not contain "competing" in any form when `competing_offer_annualized_display` is `None`. Use `assert!(!prompt.user_message.contains("competing"))`.

---

### Phase 4 — Service Implementation

**Step 4.1 — Implement `generate_draft`**

File: `lazyjob-core/src/salary/counter_offer.rs`

```rust
impl CounterOfferDraftService {
    pub async fn generate_draft(
        &self,
        request: &CounterOfferRequest,
    ) -> Result<CounterOfferDraft, CounterOfferError> {
        // 1. Build grounding context (pure, no I/O)
        let ctx = Self::build_context(request);

        // 2. Build prompt
        let prompt_builder = NegotiationPromptBuilder::new();
        let rendered = prompt_builder.build(&ctx);

        // 3. Call LLM — single non-streaming call, temperature 0.3
        let messages = vec![
            ChatMessage::system(rendered.system_message),
            ChatMessage::user(rendered.user_message),
        ];
        let response = self.llm
            .complete(CompletionRequest {
                messages,
                temperature: Some(0.3),
                max_tokens: Some(1500),
                ..Default::default()
            })
            .await
            .map_err(CounterOfferError::LlmError)?;

        // 4. Parse JSON response
        let parsed: DraftResponse = serde_json::from_str(&response.content)
            .map_err(|e| CounterOfferError::MalformedLlmResponse(e.to_string()))?;

        // 5. Construct draft
        let draft = CounterOfferDraft {
            id: CounterOfferDraftId::new(),
            offer_id: request.offer_evaluation.offer_id,
            application_id: request.offer_evaluation.application_id,
            email_subject: parsed.email_subject,
            email_body: parsed.email_body,
            talking_points: parsed.talking_points,
            negotiation_warnings: parsed.negotiation_warnings,
            data_quality_warning: ctx.data_quality_warning,
            generated_at: Utc::now(),
            tone: request.tone.clone(),
        };

        // 6. Persist
        self.draft_repo.save(&draft).await?;

        // 7. Ensure negotiation history is started
        let history = self.history_repo
            .find_by_application(request.offer_evaluation.application_id)
            .await?;
        if history.is_none() {
            let new_history = NegotiationHistory {
                id: Uuid::new_v4(),
                application_id: request.offer_evaluation.application_id,
                initial_offer_id: request.offer_evaluation.offer_id,
                final_offer_id: None,
                rounds: vec![],
                outcome: None,
                initial_annualized_cents: request.offer_evaluation
                    .offer_breakdown
                    .total_comp_annualized_cents,
                final_annualized_cents: None,
                comp_delta_cents: None,
                started_at: Utc::now(),
                completed_at: None,
            };
            self.history_repo.save(&new_history).await?;
        }

        // 8. Broadcast event
        let _ = self.event_tx.send(NegotiationEvent::DraftGenerated {
            application_id: request.offer_evaluation.application_id,
            draft_id: draft.id.0,
        });

        Ok(draft)
    }
}
```

**Step 4.2 — LLM retry on malformed JSON**

If `serde_json::from_str` fails, retry once with an appended instruction: `"Your previous response was not valid JSON. Respond with ONLY the JSON object, no surrounding text."` concatenated to the user message. On second failure, return `CounterOfferError::MalformedLlmResponse`.

```rust
async fn call_llm_with_retry(
    &self,
    messages: Vec<ChatMessage>,
) -> Result<String, CounterOfferError> {
    let resp = self.llm.complete(/* ... */).await?;
    if serde_json::from_str::<DraftResponse>(&resp.content).is_ok() {
        return Ok(resp.content);
    }
    // Retry once
    let mut retry_messages = messages.clone();
    retry_messages.push(ChatMessage::assistant(resp.content));
    retry_messages.push(ChatMessage::user(
        "Your previous response was not valid JSON. Respond with ONLY the JSON object, no surrounding text."
    ));
    let resp2 = self.llm.complete(/* ... messages: retry_messages ... */).await?;
    if serde_json::from_str::<DraftResponse>(&resp2.content).is_ok() {
        Ok(resp2.content)
    } else {
        Err(CounterOfferError::MalformedLlmResponse(
            "LLM returned malformed JSON after retry".into(),
        ))
    }
}
```

**Step 4.3 — Implement `record_outcome`**

```rust
pub async fn record_outcome(
    &self,
    application_id: Uuid,
    draft_id: Uuid,
    outcome: NegotiationOutcome,
) -> Result<NegotiationHistory, CounterOfferError> {
    let mut history = self.history_repo
        .find_by_application(application_id)
        .await?
        .ok_or(CounterOfferError::NoHistoryFound(application_id))?;

    let round_number = history.rounds.len() as u8 + 1;
    history.rounds.push(NegotiationRound {
        round_number,
        draft_id,
        outcome: outcome.clone(),
        recorded_at: Utc::now(),
    });

    // Finalize if terminal outcome
    let (final_offer_id, final_annualized) = match &outcome {
        NegotiationOutcome::Accepted { final_offer_id } => {
            let offer = self.offer_repo.find_by_id(*final_offer_id).await?
                .ok_or(CounterOfferError::OfferNotFound(*final_offer_id))?;
            (Some(*final_offer_id), Some(offer.total_comp_annualized_cents))
        }
        NegotiationOutcome::OfferRevised { revised_offer_id, .. } => {
            let offer = self.offer_repo.find_by_id(*revised_offer_id).await?
                .ok_or(CounterOfferError::OfferNotFound(*revised_offer_id))?;
            (Some(*revised_offer_id), Some(offer.total_comp_annualized_cents))
        }
        _ => (None, None),
    };

    let is_terminal = matches!(outcome,
        NegotiationOutcome::Accepted { .. } | NegotiationOutcome::Rejected
    );

    let comp_delta = final_annualized.map(|f| f - history.initial_annualized_cents);
    let completed_at = if is_terminal { Some(Utc::now()) } else { None };

    self.history_repo.update_outcome(
        history.id,
        &outcome,
        final_offer_id,
        final_annualized,
        completed_at.unwrap_or_else(Utc::now),
    ).await?;

    history.outcome = Some(outcome.clone());
    history.final_offer_id = final_offer_id;
    history.final_annualized_cents = final_annualized;
    history.comp_delta_cents = comp_delta;
    history.completed_at = completed_at;

    let _ = self.event_tx.send(NegotiationEvent::OutcomeRecorded {
        application_id,
        outcome,
        comp_delta_cents: comp_delta,
    });

    Ok(history)
}
```

**Verification:** Integration test with in-memory SQLite: generate draft → record Accepted outcome → assert `comp_delta_cents` is correct.

---

### Phase 5 — SQLite Repository Implementation

**Step 5.1 — Implement `SqliteCounterOfferDraftRepository`**

File: `lazyjob-core/src/salary/counter_offer_repo.rs`

```rust
pub struct SqliteCounterOfferDraftRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait::async_trait]
impl CounterOfferDraftRepository for SqliteCounterOfferDraftRepository {
    async fn save(&self, draft: &CounterOfferDraft) -> Result<(), CounterOfferError> {
        sqlx::query!(
            r#"
            INSERT INTO counter_offer_drafts (
                id, offer_id, application_id, email_subject, email_body,
                talking_points_json, negotiation_warnings_json,
                data_quality_warning, tone, generated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                email_subject = excluded.email_subject,
                email_body    = excluded.email_body
            "#,
            draft.id.0,
            draft.offer_id,
            draft.application_id,
            draft.email_subject,
            draft.email_body,
            serde_json::to_string(&draft.talking_points)?,
            serde_json::to_string(&draft.negotiation_warnings)?,
            draft.data_quality_warning,
            serde_json::to_string(&draft.tone)?,
            draft.generated_at.to_rfc3339(),
        )
        .execute(&self.pool)
        .await
        .map_err(CounterOfferError::Database)?;
        Ok(())
    }

    async fn list_for_application(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<CounterOfferDraft>, CounterOfferError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, offer_id, application_id, email_subject, email_body,
                   talking_points_json, negotiation_warnings_json,
                   data_quality_warning, tone, generated_at
            FROM counter_offer_drafts
            WHERE application_id = ?
            ORDER BY generated_at DESC
            "#,
            application_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(CounterOfferError::Database)?;

        rows.into_iter().map(|row| {
            Ok(CounterOfferDraft {
                id: CounterOfferDraftId(Uuid::parse_str(&row.id)?),
                offer_id: Uuid::parse_str(&row.offer_id)?,
                application_id: Uuid::parse_str(&row.application_id)?,
                email_subject: row.email_subject,
                email_body: row.email_body,
                talking_points: serde_json::from_str(&row.talking_points_json)?,
                negotiation_warnings: serde_json::from_str(&row.negotiation_warnings_json)?,
                data_quality_warning: row.data_quality_warning,
                tone: serde_json::from_str(&row.tone)?,
                generated_at: DateTime::parse_from_rfc3339(&row.generated_at)?.with_timezone(&Utc),
            })
        })
        .collect()
    }
}
```

**Step 5.2 — Implement `SqliteNegotiationHistoryRepository`**

Similar pattern: `save` inserts with `rounds_json = '[]'`, `update_outcome` uses `UPDATE WHERE id = ?` for all mutable fields. `find_by_application` uses `SELECT ... WHERE application_id = ?` with `LIMIT 1` (UNIQUE constraint guarantees at most one row).

**Verification:** `#[sqlx::test(migrations = "migrations")]` tests for:
- `save` then `find_by_application` round-trips all fields
- `update_outcome` modifies `comp_delta` correctly
- `list_for_application` returns results ordered by `generated_at DESC`

---

### Phase 6 — TUI Counter-Offer View

**Step 6.1 — `CounterOfferRequestForm`**

File: `lazyjob-tui/src/views/salary/counter_offer.rs`

A multi-step form with 3 steps:
1. **Tone selection** — three radio-style items (`Professional` / `Enthusiastic` / `Assertive`) rendered as a `ratatui::widgets::List` with custom symbol (◉ for selected, ○ for unselected). `j`/`k` to move, `Enter` to confirm.
2. **Priority ordering** — ordered list of `NegotiationPriority` items. `j`/`k` to navigate, `Shift+j`/`Shift+k` to reorder, `Enter` to confirm.
3. **Target comp (optional)** — two `TextInput` widgets: "Target base (blank to skip)" and "Target total comp (blank to skip)". Input accepts `$185k`, `$185,000`, `185000` formats parsed via the same `parse_dollar_amount` helper used in the offer form.

State machine:
```rust
enum FormStep {
    ToneSelection,
    PriorityOrdering,
    TargetComp,
}
```

On `Enter` at the final step, calls `CounterOfferDraftService::generate_draft`. Shows a spinner (the `throbber-widgets-tui` crate or a manual rotating char) during LLM generation.

**Step 6.2 — `DraftDisplayPanel`**

A `Paragraph` widget wrapping the draft text with `Wrap::Trim`. Above the content, a `Block` with title `"[DRAFT - NOT SENT]"` rendered in yellow `Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)`.

Status bar at bottom: `[c] Copy to clipboard  [Enter] Record outcome  [Esc] Back`

Clipboard copy: use the `cli-clipboard` crate (cross-platform). If clipboard is unavailable, show a `Popup` widget with the text and instruction "Select all and copy manually".

**Step 6.3 — Talking Points Accordion**

A `List` widget below the email body, prefixed with `•`. Default: collapsed (hidden). Toggle with `t` key. Height computed dynamically from `talking_points.len()`.

**Step 6.4 — Data Quality Warning Banner**

If `draft.data_quality_warning.is_some()`, render a yellow `Paragraph` block above the draft: `"⚠  {warning_text}"`. This makes the degraded state visually salient.

**Step 6.5 — `OutcomeRecordingForm`**

File: `lazyjob-tui/src/views/salary/negotiation_outcome.rs`

Triggered when the user presses `r` in the `DraftDisplayPanel`. A floating modal (`Clear` + `Block`) with four options:
- `a` — `Accepted` (prompts for final offer; default = existing offer)
- `r` — `Rejected`
- `v` — `OfferRevised` (prompts to enter revised offer amount)
- `d` — `Deferred`

On confirm, calls `CounterOfferDraftService::record_outcome`. If `comp_delta_cents.is_some()` and outcome is `Accepted`, shows a brief notification: `"Negotiation closed. You gained {delta_display}."` / `"Negotiation closed. No change in comp."`.

Round-limit warning: if `history.rounds.len() >= 2`, before generating a new draft render a yellow dismissable banner: `"Round {n} of negotiation — most employers reach a limit around round 3. Consider whether this is your final ask."` Dismissable with `Enter` or `Esc`.

**Verification:** Manual TUI test: open form → select Enthusiastic → set priorities → enter $180k target → submit → view draft → copy → record Accepted → see comp delta notification.

---

## Key Crate APIs

| Crate | API | Usage |
|-------|-----|-------|
| `sqlx` | `sqlx::query!().execute(&pool).await` | DDL + DML on `counter_offer_drafts`, `negotiation_history` |
| `sqlx` | `#[sqlx::test(migrations = "migrations")]` | In-memory SQLite integration tests |
| `serde_json` | `serde_json::from_str::<DraftResponse>(&content)` | Parse LLM JSON output |
| `serde_json` | `serde_json::to_string(&vec)` | Serialize `talking_points_json`, `rounds_json` |
| `tokio::sync::broadcast` | `Sender<NegotiationEvent>::send()` | Notify TUI of draft ready / outcome recorded |
| `ratatui::widgets::List` | `List::new(items).block(b).highlight_style(s)` | Tone selection, priority ordering |
| `ratatui::widgets::Paragraph` | `Paragraph::new(text).wrap(Wrap { trim: true })` | Draft email body display |
| `ratatui::widgets::Clear` | rendered before modal block | Outcome recording modal background erase |
| `cli-clipboard` | `ClipboardContext::new()?.set_contents(text)` | Copy draft to clipboard |
| `uuid` | `Uuid::new_v4()`, `Uuid::parse_str()` | IDs for drafts, history, rounds |
| `chrono` | `Utc::now()`, `DateTime::parse_from_rfc3339()` | Timestamps |

---

## Error Handling

```rust
// lazyjob-core/src/salary/counter_offer.rs

#[derive(Debug, thiserror::Error)]
pub enum CounterOfferError {
    #[error("LLM provider error: {0}")]
    LlmError(#[from] LlmError),

    #[error("LLM returned malformed response: {0}")]
    MalformedLlmResponse(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Offer not found: {0}")]
    OfferNotFound(Uuid),

    #[error("No negotiation history found for application {0}")]
    NoHistoryFound(Uuid),

    #[error("Market data insufficient: {0} samples (minimum 3 required)")]
    InsufficientMarketData(usize),
}
```

`InsufficientMarketData` is **not fatal** — `generate_draft` handles it by setting `data_quality_warning` and continuing with a principle-based draft. The error variant exists for callers who want to check before calling.

---

## Testing Strategy

### Unit Tests (no I/O)

All pure functions in `counter_offer.rs` are unit-tested in a `#[cfg(test)] mod tests` block within the file:

- **`test_build_context_no_competing_offer`** — `CounterOfferRequest` with empty `competing_offers` → `NegotiationContext.competing_offer_annualized_cents` is `None` and `competing_offer_annualized_display` is `None`.
- **`test_build_context_with_competing_offer`** — Request with one competing offer → display string formatted correctly.
- **`test_build_context_insufficient_market_data`** — `offer_evaluation.market_data` empty → `data_quality_warning` is `Some(...)`, `market_p50_cents` is `None`.
- **`test_compute_gap_above_market`** — Offer > market p50 → negative gap percentage → format correctly.
- **`test_compute_gap_no_market_data`** — `market_p50 = None` → returns `None`, no panic.
- **`test_negotiable_components_all_stages`** — All `CompanyStage` variants return non-empty vectors, no duplicates.
- **`test_format_cents_millions`** — `200_000_000` (200k dollars in cents) → `"$200,000"`.
- **`test_format_cents_millions_over_1M`** — `1_000_000_00` → `"$1.0M"`.

### Prompt Tests

File: `lazyjob-llm/tests/salary_negotiation_prompt.rs`

- **`test_professional_prompt_no_competing_offer_section`** — Rendered user message for `Professional` tone with no competing offer does not contain the string "competing".
- **`test_assertive_prompt_includes_gap_percentage`** — Gap of 0.18 → rendered message contains "18.0% below market".
- **`test_data_quality_warning_in_prompt`** — When `data_quality_warning` is set, it appears in the rendered message.
- **`test_target_section_both_values`** — Both `target_base_cents` and `target_total_cents` set → rendered message contains both values.

### Integration Tests (in-memory SQLite)

File: `lazyjob-core/tests/counter_offer_integration.rs`

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_generate_draft_persists_and_event_fires(pool: SqlitePool) {
    // Setup: create application + offer records, build mock LlmProvider
    // that returns valid JSON
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let svc = CounterOfferDraftService::new(
        Arc::new(MockLlmProvider::fixed_json(DRAFT_JSON_FIXTURE)),
        Arc::new(SqliteOfferRepository::new(pool.clone())),
        Arc::new(SqliteCounterOfferDraftRepository::new(pool.clone())),
        Arc::new(SqliteNegotiationHistoryRepository::new(pool.clone())),
        tx,
    );
    let draft = svc.generate_draft(&request).await.unwrap();
    assert_eq!(draft.email_body.len() > 50, true);
    assert!(matches!(rx.recv().await.unwrap(),
        NegotiationEvent::DraftGenerated { .. }));
}

#[sqlx::test(migrations = "migrations")]
async fn test_record_outcome_accepted_computes_delta(pool: SqlitePool) {
    // Setup: initial offer TC = $150,000/yr, final = $165,000/yr
    // Expected delta = +$15,000/yr = +1_500_000 cents/yr
    let history = svc.record_outcome(app_id, draft_id, NegotiationOutcome::Accepted {
        final_offer_id: final_offer.id,
    }).await.unwrap();
    assert_eq!(history.comp_delta_cents, Some(1_500_000));
}

#[sqlx::test(migrations = "migrations")]
async fn test_malformed_llm_response_retries_once(pool: SqlitePool) {
    // MockLlmProvider returns invalid JSON first, valid JSON second
    // assert no error and draft is produced
}
```

### TUI Tests

No automated TUI snapshot tests for this widget in MVP. Manual verification checklist:
- [ ] `[DRAFT - NOT SENT]` header rendered in yellow bold
- [ ] Talking points hidden by default, visible after `t`
- [ ] Data quality warning banner rendered when `data_quality_warning.is_some()`
- [ ] Round-limit warning shown at round 3
- [ ] Clipboard copy succeeds (manual test)
- [ ] Outcome modal appears on `r`, records outcome correctly

---

## Open Questions

1. **Round-limit warning**: The spec raises whether a round-3 warning is paternalistic. Recommendation: implement the warning as a non-blocking banner (user can dismiss with `Enter`/`Esc` and proceed). Log the dismissal but never block generation. This respects user autonomy while delivering the coaching value.

2. **Offer letter parsing**: Parsing a pasted offer letter is a Phase 4+ feature. Implementation when added: LLM-based extraction into a structured `ParsedOfferFields` struct with an explicit user review screen before any values are written to `offer_details`. All extracted fields displayed with a "(parsed — please verify)" label.

3. **Gender-aware coaching**: This is a product values decision. Recommendation: do not implement in MVP. If added, it must be opt-in (`[negotiation] acknowledge_gender_coaching_disclaimer = true` in config) and phrased as "some candidates prefer a collaborative tone due to documented social dynamics" rather than a gender presumption.

4. **Negotiation analytics in SaaS mode**: `comp_delta` must be explicitly excluded from the default SaaS sync scope. The `sync_outbox` table must have a `sync_category` column; `negotiation_history` should be in the `private` category (sync only on explicit user opt-in). This is an implementation gate for the SaaS migration plan.

5. **Competing offer reference validation**: Currently the constraint is "only reference a competing offer if one is entered." Future hardening: validate that the referenced competing offer's annualized TC matches the `CompetingOffer.total_comp_cents` computed by `compute_total_comp` — ensure the LLM cannot inflate the competing offer number in its narrative even if it has the exact dollar figure.

---

## Related Specs

- [specs/salary-market-intelligence.md](salary-market-intelligence.md) — `OfferEvaluation`, market data
- [specs/salary-negotiation-offers.md](salary-negotiation-offers.md) — `OfferRecord`, BATNA, `NegotiationPriorities`
- [specs/agentic-llm-provider-abstraction.md](agentic-llm-provider-abstraction.md) — `LlmProvider::complete`
- [specs/17-ralph-prompt-templates.md](17-ralph-prompt-templates.md) — `TemplateEngine` infrastructure
- [specs/application-workflow-actions.md](application-workflow-actions.md) — `PostTransitionSuggestion::RunSalaryComparison`
- [specs/application-state-machine.md](application-state-machine.md) — `ApplicationStage` transitions on Accepted/Rejected
- [specs/09-tui-design-keybindings.md](09-tui-design-keybindings.md) — TUI event loop, modal system
