# Implementation Plan: LLM Cost Tracking and Budget Management

## Status
Draft

## Related Spec
[specs/XX-llm-cost-budget-management.md](./XX-llm-cost-budget-management.md)

## Overview

LazyJob's ralph loops make autonomous LLM API calls that can accumulate real billing costs
without the user noticing. This plan implements a cost tracking and budget management system
inside `lazyjob-llm` with a thin persistence layer in `lazyjob-core`.

The system has three responsibilities:
1. **Track every LLM call** — record `(provider, model, prompt_tokens, completion_tokens,
   cost_cents, operation_type, ralph_loop_id)` in an append-only SQLite table after each
   completed call, using the actual token counts returned by the provider's response object.
2. **Enforce budget limits** — before each call, the `BudgetEnforcer` computes the projected
   cost, checks it against configured daily/monthly limits and per-operation caps, and returns
   `CheckResult::Blocked` or `CheckResult::Throttled` when limits would be exceeded. Ralph
   workers and pipeline stages check this before calling `LlmProvider::chat()`.
3. **Display cost visibility** — the TUI exposes a dedicated `CostDashboardView` with a live
   monthly progress bar, per-operation breakdown table, and 7-day sparkline chart. The status
   bar also shows a compact `$X.XX/$Y.YY` cost pill updated on every usage record insertion.

All monetary values are stored and computed as `i64` microdollars (1 microdollar = 0.000001 USD)
— not cents — to avoid rounding errors at sub-cent precision for Haiku/gpt-4o-mini class models
that cost fractions of a cent per call. Display formatting converts to dollars with two decimal
places. The spec uses cents; this plan uses microdollars for higher precision (see below).

## Prerequisites

### Must be implemented first
- `specs/agentic-llm-provider-abstraction.md` — the `LlmProvider` trait and `TokenUsage` types
  must exist; cost tracking hooks into the provider call path.
- `specs/04-sqlite-persistence-implementation-plan.md` — `SqlitePool` pattern and migration
  runner must exist for the `llm_usage` table migrations.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
# already present from llm-provider-abstraction plan:
tokio             = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }
serde             = { version = "1", features = ["derive"] }
serde_json        = "1"
thiserror         = "1"
anyhow            = "1"
tracing           = "0.1"
uuid              = { version = "1", features = ["v4", "serde"] }
chrono            = { version = "0.4", features = ["serde"] }
sqlx              = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "chrono", "uuid"] }

# new for this plan:
once_cell         = "1"
```

No new binary dependencies. `once_cell` is needed for the `PRICING` static table in
`lazyjob-llm/src/cost/pricing.rs`.

---

## Architecture

### Crate Placement

`lazyjob-llm/src/cost/` owns cost estimation and the budget check interface.
`lazyjob-core/src/cost/` owns the `UsageRepository` (SQLite I/O) and `BudgetEnforcer`
(which reads from the DB). This split is load-bearing: `lazyjob-llm` has no SQLite
dependency; it only produces `UsageRecord` values which `lazyjob-core` persists.

`lazyjob-tui/src/views/cost_dashboard.rs` owns the TUI view.

Dependency direction: `lazyjob-tui` → `lazyjob-core` → `lazyjob-llm`.

### Module Structure

```
lazyjob-llm/
  src/
    cost/
      mod.rs            # re-exports Microdollars, ModelCost, CostTable, OperationType, UsageRecord
      pricing.rs        # PRICING: once_cell::sync::Lazy<CostTable> with hardcoded model rates
      types.rs          # Microdollars, ModelCost, OperationType, UsageRecord, CostEstimate

lazyjob-core/
  src/
    cost/
      mod.rs            # re-exports UsageRepository, BudgetEnforcer, BudgetConfig, BudgetEvent
      repository.rs     # SqliteUsageRepository: insert + aggregate queries
      budget.rs         # BudgetEnforcer: check_before_call(), record_actual()
      config.rs         # BudgetConfig, AlertThreshold, AlertAction, OperationLimit
      summary.rs        # CostSummary, BillingPeriod, PeriodRange, OperationBreakdown

lazyjob-tui/
  src/
    views/
      cost_dashboard.rs # CostDashboardView widget
    widgets/
      cost_pill.rs      # Compact status-bar cost display
```

### Core Types

```rust
// lazyjob-llm/src/cost/types.rs

/// All monetary values in microdollars (1 USD = 1_000_000 microdollars).
/// i64 gives up to ~$9.2 trillion before overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Microdollars(pub i64);

impl Microdollars {
    /// Construct from per-million-token rate and token count.
    /// `rate_per_million` is expressed in whole dollars (e.g. 3.0 = $3/M tokens).
    pub fn from_rate(rate_per_million: f64, token_count: u32) -> Self {
        // microdollars = (rate / 1_000_000) * tokens * 1_000_000
        // = rate * tokens (in microdollars)
        Self((rate_per_million * token_count as f64).round() as i64)
    }

    pub fn to_dollars_display(self) -> String {
        format!("${:.4}", self.0 as f64 / 1_000_000.0)
    }

    pub fn to_dollars_2dp(self) -> String {
        format!("${:.2}", self.0 as f64 / 1_000_000.0)
    }
}

impl std::ops::Add for Microdollars {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}

impl std::ops::AddAssign for Microdollars {
    fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
}

/// Per-model cost rates stored per 1M tokens.
#[derive(Clone, Debug)]
pub struct ModelCost {
    /// Input/prompt token rate, dollars per million.
    pub input_rate: f64,
    /// Output/completion token rate, dollars per million.
    pub output_rate: f64,
    /// Batch input rate (e.g. OpenAI Batch API, Anthropic Message Batches).
    pub batch_input_rate: Option<f64>,
    /// Batch output rate.
    pub batch_output_rate: Option<f64>,
}

impl ModelCost {
    pub fn estimate(&self, prompt_tokens: u32, completion_tokens: u32) -> Microdollars {
        Microdollars::from_rate(self.input_rate, prompt_tokens)
            + Microdollars::from_rate(self.output_rate, completion_tokens)
    }

    pub fn estimate_batch(&self, prompt_tokens: u32, completion_tokens: u32) -> Microdollars {
        let in_rate = self.batch_input_rate.unwrap_or(self.input_rate);
        let out_rate = self.batch_output_rate.unwrap_or(self.output_rate);
        Microdollars::from_rate(in_rate, prompt_tokens)
            + Microdollars::from_rate(out_rate, completion_tokens)
    }
}

/// Which product feature generated this LLM call.
/// Stored as TEXT in SQLite (to_db_str / from_db_str round-trips).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperationType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrep,
    MockInterview,
    SalaryNegotiation,
    NetworkingOutreach,
    SkillGapAnalysis,
    GhostJobDetection,
    BackgroundTask,
    UserChat,
}

impl OperationType {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::JobDiscovery         => "job_discovery",
            Self::CompanyResearch      => "company_research",
            Self::ResumeTailoring      => "resume_tailoring",
            Self::CoverLetterGeneration=> "cover_letter_generation",
            Self::InterviewPrep        => "interview_prep",
            Self::MockInterview        => "mock_interview",
            Self::SalaryNegotiation    => "salary_negotiation",
            Self::NetworkingOutreach   => "networking_outreach",
            Self::SkillGapAnalysis     => "skill_gap_analysis",
            Self::GhostJobDetection    => "ghost_job_detection",
            Self::BackgroundTask       => "background_task",
            Self::UserChat             => "user_chat",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "job_discovery"          => Self::JobDiscovery,
            "company_research"       => Self::CompanyResearch,
            "resume_tailoring"       => Self::ResumeTailoring,
            "cover_letter_generation"=> Self::CoverLetterGeneration,
            "interview_prep"         => Self::InterviewPrep,
            "mock_interview"         => Self::MockInterview,
            "salary_negotiation"     => Self::SalaryNegotiation,
            "networking_outreach"    => Self::NetworkingOutreach,
            "skill_gap_analysis"     => Self::SkillGapAnalysis,
            "ghost_job_detection"    => Self::GhostJobDetection,
            "user_chat"              => Self::UserChat,
            _                        => Self::BackgroundTask,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::JobDiscovery          => "Job Discovery",
            Self::CompanyResearch       => "Company Research",
            Self::ResumeTailoring       => "Resume Tailoring",
            Self::CoverLetterGeneration => "Cover Letter Gen",
            Self::InterviewPrep         => "Interview Prep",
            Self::MockInterview         => "Mock Interview",
            Self::SalaryNegotiation     => "Salary Negotiation",
            Self::NetworkingOutreach    => "Networking Outreach",
            Self::SkillGapAnalysis      => "Skill Gap Analysis",
            Self::GhostJobDetection     => "Ghost Job Detect",
            Self::BackgroundTask        => "Background Task",
            Self::UserChat              => "User Chat",
        }
    }
}

/// One completed LLM call. Produced by the provider call site; persisted by UsageRepository.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: uuid::Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub provider: String,
    pub model: String,
    pub operation: OperationType,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_microdollars: Microdollars,
    /// Which ralph loop triggered this call (None for non-loop operations).
    pub ralph_loop_id: Option<uuid::Uuid>,
    /// Optional caller-supplied tag for grouping (e.g. "gap_analysis_run_42").
    pub trace_id: Option<String>,
}

/// Pre-call cost estimate returned by CostTable::estimate_from_messages().
#[derive(Clone, Debug)]
pub struct CostEstimate {
    pub estimated_prompt_tokens: u32,
    pub estimated_completion_tokens: u32,
    pub estimated_cost: Microdollars,
    /// True when the model is unknown and cost is 0 (Ollama, unknown models).
    pub is_free_or_unknown: bool,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/cost/repository.rs (trait portion)

#[async_trait::async_trait]
pub trait UsageRepository: Send + Sync {
    async fn insert(&self, record: UsageRecord) -> Result<(), CostError>;
    async fn get_summary(
        &self,
        period: BillingPeriod,
    ) -> Result<CostSummary, CostError>;
    async fn get_by_operation(
        &self,
        period: BillingPeriod,
    ) -> Result<Vec<OperationBreakdown>, CostError>;
    async fn get_daily_totals(
        &self,
        last_n_days: u32,
    ) -> Result<Vec<DailyTotal>, CostError>;
    async fn get_operation_total(
        &self,
        operation: &OperationType,
        period: BillingPeriod,
    ) -> Result<Microdollars, CostError>;
}
```

### SQLite Schema

```sql
-- Migration: migrations/005_llm_usage.sql

CREATE TABLE IF NOT EXISTS llm_usage (
    id                  TEXT    PRIMARY KEY,    -- UUID v4 hyphenated
    timestamp           TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    provider            TEXT    NOT NULL,        -- "anthropic" | "openai" | "ollama"
    model               TEXT    NOT NULL,        -- e.g. "claude-3-5-sonnet-20241022"
    operation           TEXT    NOT NULL,        -- OperationType::to_db_str()
    prompt_tokens       INTEGER NOT NULL,
    completion_tokens   INTEGER NOT NULL,
    cost_microdollars   INTEGER NOT NULL,        -- Microdollars(i64)
    ralph_loop_id       TEXT,                    -- UUID, NULL for non-loop calls
    trace_id            TEXT
);

CREATE INDEX IF NOT EXISTS idx_llm_usage_timestamp
    ON llm_usage(timestamp);

CREATE INDEX IF NOT EXISTS idx_llm_usage_operation
    ON llm_usage(operation, timestamp);

CREATE INDEX IF NOT EXISTS idx_llm_usage_ralph_loop
    ON llm_usage(ralph_loop_id)
    WHERE ralph_loop_id IS NOT NULL;

-- Budget configuration: one row per user (single-user MVP: always user_id = 'default').
CREATE TABLE IF NOT EXISTS budget_config (
    user_id                 TEXT    PRIMARY KEY DEFAULT 'default',
    enabled                 INTEGER NOT NULL DEFAULT 0,
    monthly_limit_microdollars INTEGER NOT NULL DEFAULT 10_000_000_000, -- $10,000 default
    alert_thresholds_json   TEXT    NOT NULL DEFAULT '[]',  -- JSON AlertThreshold[]
    per_operation_json      TEXT    NOT NULL DEFAULT '{}',  -- JSON {op: OperationLimit}
    updated_at              TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Fired-alert log: prevents re-firing the same threshold in the same billing period.
CREATE TABLE IF NOT EXISTS budget_alert_log (
    id              TEXT    PRIMARY KEY,
    fired_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    threshold_pct   INTEGER NOT NULL,
    period_start    TEXT    NOT NULL,
    period_end      TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_budget_alert_period
    ON budget_alert_log(period_start, threshold_pct);
```

---

## Implementation Phases

### Phase 1 — Cost Estimation and Usage Recording (MVP)

**Goal:** Every LLM call records actual usage; pre-call estimates available for display.

**Step 1.1 — Implement `Microdollars`, `ModelCost`, `OperationType`, `UsageRecord`**
- File: `lazyjob-llm/src/cost/types.rs`
- All types as shown in the Core Types section.
- No external I/O. Pure value types.
- Verification: `cargo test -p lazyjob-llm cost::types` — test `Microdollars::from_rate`
  for known values (e.g. $3/M * 1000 tokens = 3000 microdollars), test `OperationType`
  round-trip via `to_db_str() / from_db_str()`.

**Step 1.2 — Implement `CostTable` with static `PRICING` table**
- File: `lazyjob-llm/src/cost/pricing.rs`
- Use `once_cell::sync::Lazy<HashMap<(String, String), ModelCost>>` keyed by
  `(provider.to_lowercase(), model_name.to_lowercase())`.
- Populate all models from the spec's cost table:
  - Anthropic: claude-3-5-sonnet-20241022 ($3/$15), claude-3-opus-20240229 ($15/$75),
    claude-3-haiku-20240307 ($0.25/$1.25), claude-sonnet-4-6 ($3/$15, same tier),
    claude-opus-4-6 ($15/$75)
  - OpenAI: gpt-4o ($2.5/$10), gpt-4o-mini ($0.15/$0.60), gpt-4-turbo ($10/$30)
  - Ollama models: always `Microdollars(0)` (local, no API cost).

```rust
// lazyjob-llm/src/cost/pricing.rs

use once_cell::sync::Lazy;
use std::collections::HashMap;
use crate::cost::types::ModelCost;

pub static PRICING: Lazy<HashMap<(&'static str, &'static str), ModelCost>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert(
        ("anthropic", "claude-3-5-sonnet-20241022"),
        ModelCost { input_rate: 3.0, output_rate: 15.0,
                    batch_input_rate: Some(1.5), batch_output_rate: Some(7.5) },
    );
    m.insert(
        ("anthropic", "claude-3-haiku-20240307"),
        ModelCost { input_rate: 0.25, output_rate: 1.25,
                    batch_input_rate: Some(0.125), batch_output_rate: Some(0.625) },
    );
    m.insert(
        ("openai", "gpt-4o"),
        ModelCost { input_rate: 2.5, output_rate: 10.0,
                    batch_input_rate: Some(1.0), batch_output_rate: Some(4.0) },
    );
    m.insert(
        ("openai", "gpt-4o-mini"),
        ModelCost { input_rate: 0.15, output_rate: 0.60,
                    batch_input_rate: Some(0.075), batch_output_rate: Some(0.30) },
    );
    // Ollama: omit — lookup miss → free/unknown path
    m
});

pub struct CostTable;

impl CostTable {
    pub fn estimate(
        provider: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> CostEstimate {
        let key = (
            provider.to_lowercase(),
            model.to_lowercase(),
        );
        // Static key lookup with owned string key requires a workaround:
        // iterate and compare, or use a secondary owned HashMap built at init.
        // See implementation note below.
        match PRICING.get(&(key.0.as_str(), key.1.as_str())) {
            Some(mc) => CostEstimate {
                estimated_prompt_tokens: prompt_tokens,
                estimated_completion_tokens: completion_tokens,
                estimated_cost: mc.estimate(prompt_tokens, completion_tokens),
                is_free_or_unknown: false,
            },
            None => CostEstimate {
                estimated_prompt_tokens: prompt_tokens,
                estimated_completion_tokens: completion_tokens,
                estimated_cost: Microdollars(0),
                is_free_or_unknown: true,
            },
        }
    }

    /// Quick estimate from raw message text (before sending, for pre-flight check).
    /// Uses 4 chars ≈ 1 token heuristic; assumes 500 completion tokens.
    pub fn estimate_from_chars(
        provider: &str,
        model: &str,
        total_prompt_chars: usize,
    ) -> CostEstimate {
        let estimated_prompt = (total_prompt_chars / 4) as u32;
        let estimated_completion = 500u32;
        Self::estimate(provider, model, estimated_prompt, estimated_completion)
    }
}
```

**Implementation note:** The `PRICING` map uses `&'static str` keys. The lookup
`PRICING.get(&(provider_str.as_str(), model_str.as_str()))` won't compile directly because
`HashMap<(&'static str, &'static str), V>` uses the Borrow trait through a fixed-lifetime.
Easiest fix: build a second `Lazy<HashMap<(String, String), &'static ModelCost>>` from `PRICING`,
or use a `Vec<((&'static str, &'static str), ModelCost)>` and linear scan (< 30 entries, fast).
Simplest production-grade approach: `PRICING` is a `Vec` sorted by (provider, model) with binary
search — or just use `HashMap<String, ModelCost>` keyed by `"{provider}/{model}"`.

- Verification: Unit tests checking that `estimate("anthropic", "claude-3-5-sonnet-20241022", 1_000_000, 1_000_000)` returns `Microdollars(18_000_000)` ($18).

**Step 1.3 — Wire usage recording into the LLM provider call path**
- File: `lazyjob-llm/src/providers/anthropic.rs`, `openai.rs`, `ollama.rs`
- Each `LlmProvider::chat()` implementation already returns a `ChatResponse` containing
  `TokenUsage { prompt_tokens, completion_tokens }` (from the provider abstraction plan).
- Add an optional `Arc<dyn Fn(UsageRecord) + Send + Sync>` callback to `AnthropicProvider`,
  `OpenAiProvider`, `OllamaProvider` structs — called after each successful response.
- The callback is `Option<_>` so the provider works standalone without a callback for tests.
- The `LlmBuilder` wires in the callback pointing to `UsageRepository::insert` in production.

```rust
// inside AnthropicProvider::chat() after receiving the full response:
if let Some(ref cb) = self.usage_callback {
    let record = UsageRecord {
        id: uuid::Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        provider: "anthropic".to_string(),
        model: request.model.clone(),
        operation: request.operation.unwrap_or(OperationType::BackgroundTask),
        prompt_tokens: response.usage.prompt_tokens,
        completion_tokens: response.usage.completion_tokens,
        cost_microdollars: CostTable::estimate(
            "anthropic", &request.model,
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        ).estimated_cost,
        ralph_loop_id: request.ralph_loop_id,
        trace_id: request.trace_id.clone(),
    };
    cb(record);
}
```

- The `ChatRequest` struct gains two optional fields:
  `operation: Option<OperationType>` and `ralph_loop_id: Option<uuid::Uuid>`.
- Verification: Run a test using `MockLlmProvider` that checks the callback fires with the
  correct `prompt_tokens`, `completion_tokens`, and `cost_microdollars`.

**Step 1.4 — Implement `SqliteUsageRepository`**
- File: `lazyjob-core/src/cost/repository.rs`
- Implements `UsageRepository` trait using `sqlx::SqlitePool`.
- `insert()`: `sqlx::query!("INSERT INTO llm_usage ...")` with all fields.
- `get_summary(period)`: single `SELECT SUM(...)` over the period date range.
- `get_by_operation(period)`: `GROUP BY operation` query returning `Vec<OperationBreakdown>`.
- `get_daily_totals(last_n_days)`: `GROUP BY date(timestamp)` returning `Vec<DailyTotal>`.
- `get_operation_total(op, period)`: single `SUM(cost_microdollars) WHERE operation = ?`.

```rust
// lazyjob-core/src/cost/summary.rs

#[derive(Clone, Debug)]
pub struct CostSummary {
    pub period: BillingPeriod,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: Microdollars,
    pub request_count: u64,
}

#[derive(Clone, Debug)]
pub struct OperationBreakdown {
    pub operation: OperationType,
    pub cost: Microdollars,
    pub request_count: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct DailyTotal {
    pub date: chrono::NaiveDate,
    pub cost: Microdollars,
    pub request_count: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum BillingPeriod {
    CurrentDay,
    CurrentWeek,
    CurrentMonth,
    Last30Days,
    Custom { start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc> },
}

impl BillingPeriod {
    pub fn date_range(self) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
        use chrono::{Datelike, TimeZone, Utc};
        let now = Utc::now();
        match self {
            Self::CurrentDay => {
                let start = Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                    .unwrap();
                (start, start + chrono::Duration::days(1))
            }
            Self::CurrentMonth => {
                let start = Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0).unwrap();
                let end = if now.month() == 12 {
                    Utc.with_ymd_and_hms(now.year() + 1, 1, 1, 0, 0, 0).unwrap()
                } else {
                    Utc.with_ymd_and_hms(now.year(), now.month() + 1, 1, 0, 0, 0).unwrap()
                };
                (start, end)
            }
            Self::Last30Days => (now - chrono::Duration::days(30), now),
            Self::CurrentWeek => {
                use chrono::Weekday;
                let days_since_monday = now.weekday().num_days_from_monday() as i64;
                let start = (now - chrono::Duration::days(days_since_monday))
                    .date_naive().and_hms_opt(0, 0, 0).unwrap()
                    .and_utc();
                (start, start + chrono::Duration::weeks(1))
            }
            Self::Custom { start, end } => (start, end),
        }
    }
}
```

- Verification: `#[sqlx::test(migrations = "migrations")]` with 5 inserted records —
  assert `get_summary(CurrentMonth).cost` equals sum of all microdollar values.

---

### Phase 2 — Budget Enforcement

**Goal:** Block/throttle LLM calls when limits would be exceeded; fire threshold events for TUI.

**Step 2.1 — `BudgetConfig` and `AlertThreshold`**
- File: `lazyjob-core/src/cost/config.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub enabled: bool,
    pub monthly_limit: Microdollars,
    pub daily_limit: Option<Microdollars>,
    pub alert_thresholds: Vec<AlertThreshold>,
    pub per_operation_limits: HashMap<OperationType, OperationLimit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlertThreshold {
    pub percentage: u8,   // 0–100
    pub action: AlertAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AlertAction {
    /// Write tracing::warn! only.
    Log,
    /// Send BudgetEvent to TUI broadcast channel.
    NotifyTui,
    /// Stop dispatching new ralph loops but allow in-flight calls to complete.
    PauseRalphLoops,
    /// Return CheckResult::Blocked for all new LLM calls.
    BlockNewRequests,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationLimit {
    pub per_day: Option<Microdollars>,
    pub per_month: Option<Microdollars>,
    pub max_requests_per_hour: Option<u32>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            enabled: false,    // opt-in; off by default for MVP
            monthly_limit: Microdollars(100_000_000_000), // $100,000 (effectively unlimited)
            daily_limit: None,
            alert_thresholds: vec![
                AlertThreshold { percentage: 50,  action: AlertAction::Log },
                AlertThreshold { percentage: 75,  action: AlertAction::NotifyTui },
                AlertThreshold { percentage: 90,  action: AlertAction::PauseRalphLoops },
                AlertThreshold { percentage: 100, action: AlertAction::BlockNewRequests },
            ],
            per_operation_limits: HashMap::new(),
        }
    }
}
```

- `BudgetConfig` is loaded from `budget_config` SQLite table on startup (deserialized from
  JSON columns) and cached in an `Arc<RwLock<BudgetConfig>>` for zero-copy reads by the
  enforcer. Updates are written back to SQLite and the `RwLock` refreshed.

**Step 2.2 — `SqliteBudgetConfigRepository`**
- File: `lazyjob-core/src/cost/repository.rs` (extend existing file)
- `load() -> Result<BudgetConfig, CostError>` — SELECT from `budget_config` WHERE
  `user_id = 'default'`, deserialize JSON columns.
- `save(config: &BudgetConfig) -> Result<(), CostError>` — upsert with `ON CONFLICT(user_id)
  DO UPDATE SET ...` updating all columns and `updated_at`.

**Step 2.3 — `BudgetEnforcer`**
- File: `lazyjob-core/src/cost/budget.rs`

```rust
pub struct BudgetEnforcer {
    usage_repo: Arc<dyn UsageRepository>,
    config: Arc<tokio::sync::RwLock<BudgetConfig>>,
    event_tx: tokio::sync::broadcast::Sender<BudgetEvent>,
    alert_log_repo: Arc<dyn AlertLogRepository>,
}

#[derive(Clone, Debug)]
pub enum BudgetEvent {
    ThresholdCrossed { threshold_pct: u8, current: Microdollars, limit: Microdollars },
    RequestBlocked   { operation: OperationType, estimated_cost: Microdollars, reason: String },
    RalphLoopPaused  { loop_id: uuid::Uuid },
    BudgetUpdated    { new_config: BudgetConfig },
}

pub enum CheckResult {
    Allowed,
    Blocked  { reason: String },
    Throttled { wait_seconds: u64 },
}

impl BudgetEnforcer {
    /// Call BEFORE making an LLM request.
    /// Returns Allowed/Blocked/Throttled.
    pub async fn check_before_call(
        &self,
        operation: &OperationType,
        estimate: &CostEstimate,
        ralph_loop_id: Option<uuid::Uuid>,
    ) -> Result<CheckResult, CostError> {
        let config = self.config.read().await;
        if !config.enabled || estimate.is_free_or_unknown {
            return Ok(CheckResult::Allowed);
        }

        // 1. Check global monthly limit
        let monthly = self.usage_repo
            .get_summary(BillingPeriod::CurrentMonth).await?;
        let projected_monthly = monthly.cost + estimate.estimated_cost;

        if projected_monthly > config.monthly_limit {
            let reason = format!(
                "Monthly budget {} exceeded (projected {})",
                config.monthly_limit.to_dollars_2dp(),
                projected_monthly.to_dollars_2dp(),
            );
            let _ = self.event_tx.send(BudgetEvent::RequestBlocked {
                operation: operation.clone(),
                estimated_cost: estimate.estimated_cost,
                reason: reason.clone(),
            });
            return Ok(CheckResult::Blocked { reason });
        }

        // 2. Fire threshold events (idempotent via alert_log_repo)
        let pct = (projected_monthly.0 * 100 / config.monthly_limit.0.max(1)) as u8;
        for threshold in config.alert_thresholds.iter().rev() {
            if pct >= threshold.percentage {
                let (period_start, period_end) =
                    BillingPeriod::CurrentMonth.date_range();
                if !self.alert_log_repo.was_fired(
                    threshold.percentage, period_start, period_end
                ).await? {
                    self.alert_log_repo.record_fired(
                        threshold.percentage, period_start, period_end
                    ).await?;
                    let _ = self.event_tx.send(BudgetEvent::ThresholdCrossed {
                        threshold_pct: threshold.percentage,
                        current: projected_monthly,
                        limit: config.monthly_limit,
                    });
                    match threshold.action {
                        AlertAction::BlockNewRequests => {
                            return Ok(CheckResult::Blocked {
                                reason: format!("{}% budget threshold reached", threshold.percentage),
                            });
                        }
                        AlertAction::PauseRalphLoops => {
                            if let Some(id) = ralph_loop_id {
                                let _ = self.event_tx.send(BudgetEvent::RalphLoopPaused { loop_id: id });
                            }
                            // Does not block the current call — ralph loop dispatcher
                            // subscribes to BudgetEvent::RalphLoopPaused and stops accepting
                            // new loop dispatches.
                        }
                        AlertAction::NotifyTui | AlertAction::Log => {}
                    }
                }
                break; // only fire the highest matched threshold
            }
        }

        // 3. Check per-operation daily limit
        if let Some(limit) = config.per_operation_limits.get(operation) {
            if let Some(daily_limit) = limit.per_day {
                let daily_op_cost = self.usage_repo
                    .get_operation_total(operation, BillingPeriod::CurrentDay).await?;
                if daily_op_cost + estimate.estimated_cost > daily_limit {
                    return Ok(CheckResult::Blocked {
                        reason: format!(
                            "Daily {} budget {} exceeded",
                            operation.display_name(),
                            daily_limit.to_dollars_2dp(),
                        ),
                    });
                }
            }
        }

        Ok(CheckResult::Allowed)
    }
}
```

- Verification: Unit test with a mocked `UsageRepository` returning a near-limit monthly
  total — assert `check_before_call` returns `Blocked` and a `BudgetEvent::ThresholdCrossed`
  is sent on the broadcast channel.

**Step 2.4 — Wire `BudgetEnforcer` into ralph worker dispatch**
- File: `lazyjob-ralph/src/manager.rs` and `lazyjob-core/src/pipeline/*.rs`
- Each call site that constructs a `ChatRequest` calls `enforcer.check_before_call()`
  first; on `Blocked` it returns a `WorkerEvent::Error { message }` without calling
  the LLM.
- The `BudgetEnforcer` is injected via `Arc<BudgetEnforcer>` at construction time;
  never constructed inside a worker.
- Verification: Integration test: set `monthly_limit = Microdollars(1)`, call
  `check_before_call` with a 1-token estimate, assert `Blocked`.

---

### Phase 3 — TUI Cost Dashboard

**Goal:** Users can see exactly what they're spending and where.

**Step 3.1 — `CostDashboardView` widget**
- File: `lazyjob-tui/src/views/cost_dashboard.rs`
- State struct:

```rust
pub struct CostDashboardState {
    pub summary: Option<CostSummary>,
    pub by_operation: Vec<OperationBreakdown>,
    pub daily_totals: Vec<DailyTotal>,      // last 7 days
    pub budget_config: BudgetConfig,
    pub selected_period: BillingPeriod,
    pub loading: bool,
    pub table_state: ratatui::widgets::TableState,
}
```

- Layout (full-screen view, `Direction::Vertical` split):
  - Row 0 (3 lines): header "LLM Usage & Budget" + period selector tabs
  - Row 1 (3 lines): monthly progress gauge — `ratatui::widgets::Gauge` from 0.0 to 1.0,
    label = "$X.XX / $Y.YY (Z%)"
  - Row 2 (remaining): horizontal split 60/40:
    - Left 60%: `ratatui::widgets::Table` — operation | cost | requests | avg_cost columns,
      sorted by cost desc, selected row highlighted
    - Right 40%: vertical split:
      - Top 50%: `ratatui::widgets::BarChart` — 7-day daily spend bars, bar width 3,
        gap 1, label = "Mon" etc., max value auto-scaled to highest day
      - Bottom 50%: `ratatui::widgets::Paragraph` — stats panel (avg/day, most expensive
        operation, estimated month-end, days remaining in period)
  - Bottom row (3 lines): keybind help `[Tab] period  [s] settings  [ESC] back`

- `render(f: &mut Frame, area: Rect, state: &CostDashboardState)` is a pure function
  with no async. Data is loaded separately via `CostDashboardLoader`.

**Step 3.2 — `CostDashboardLoader`**
- File: `lazyjob-tui/src/views/cost_dashboard.rs`
- Async method called on view activation and on `BudgetEvent` reception:

```rust
pub struct CostDashboardLoader {
    usage_repo: Arc<dyn UsageRepository>,
    budget_repo: Arc<dyn BudgetConfigRepository>,
}

impl CostDashboardLoader {
    pub async fn load(&self, period: BillingPeriod) -> Result<CostDashboardState, CostError> {
        let (summary, by_operation, daily_totals, budget_config) = tokio::join!(
            self.usage_repo.get_summary(period),
            self.usage_repo.get_by_operation(period),
            self.usage_repo.get_daily_totals(7),
            self.budget_repo.load(),
        );
        Ok(CostDashboardState {
            summary: Some(summary?),
            by_operation: by_operation?,
            daily_totals: daily_totals?,
            budget_config: budget_config?,
            selected_period: period,
            loading: false,
            table_state: Default::default(),
        })
    }
}
```

**Step 3.3 — `CostPill` status bar widget**
- File: `lazyjob-tui/src/widgets/cost_pill.rs`
- Renders a compact `Span`: `Cost: $X.XX/$Y.YY ██░░░░ 34%`
- Color: green < 50%, yellow 50–75%, red 75–90%, bold-red > 90%
- Updated via `tokio::sync::watch::Receiver<CostSummary>` — the `BudgetEnforcer` sends
  on this channel after every usage record insertion. The TUI event loop calls
  `watch_rx.borrow_and_update()` in `tokio::select!` to refresh.

**Step 3.4 — Budget settings inline edit**
- Pressing `[s]` in `CostDashboardView` opens a floating `Clear`-backed modal:
  - `monthly_limit` text field with dollar-amount parsing (`$100` / `100` / `100.00`)
  - `enabled` toggle with `[Space]`
  - `[Enter]` saves via `SqliteBudgetConfigRepository::save()` and broadcasts
    `BudgetEvent::BudgetUpdated`

---

### Phase 4 — Ollama Local Cost (Optional)

**Goal:** Track electricity/time cost for Ollama without a billing API.

- Add `OllamaLocalCost { tokens_per_second: f64, watts: f64, kwh_rate_cents: f64 }`
  to `BudgetConfig` (optional fields, None by default).
- `OllamaProvider` records actual latency from `Instant::now()` around the HTTP call;
  derive approximate tokens per second from `completion_tokens / latency_secs`.
- Compute `cost_microdollars` for Ollama calls as:
  `(completion_tokens / tokens_per_second) * (watts / 3600) * kwh_rate_cents * 10_000`
  (converting to microdollars).
- Display as "Local compute cost (est.)" in the dashboard, separate row from API costs.
- `is_free_or_unknown` remains true in `CostEstimate` for Ollama (budget enforcer ignores it;
  only informational display).
- Verification: Unit test `OllamaLocalCost::compute(completion_tokens=100, latency_secs=5.0,
  watts=200.0, kwh_rate_cents=15)` → deterministic microdollar value.

---

### Phase 5 — Export and Historical Analysis

**Goal:** CSV export of usage log and per-loop cost attribution.

**Step 5.1 — CSV export**
- `UsageRepository::export_csv(period, writer: &mut dyn Write)` — write header row +
  one row per `llm_usage` record using the `csv` crate.
- Exposed via `lazyjob cost export --period month --output usage.csv` CLI subcommand
  in `lazyjob-cli/src/commands/cost.rs`.

**Step 5.2 — Ralph loop cost attribution**
- Add `CostByLoop` query: `SELECT ralph_loop_id, SUM(cost_microdollars), COUNT(*) FROM
  llm_usage WHERE ralph_loop_id IS NOT NULL GROUP BY ralph_loop_id`.
- Join with `ralph_loop_runs.loop_type` to show "JobDiscovery loop #42: $0.83 (15 calls)".
- Surfaced in TUI `CostDashboardView` as an expandable "By Ralph Loop" table row.

**Step 5.3 — Retention cleanup**
- `UsageRepository::delete_older_than(cutoff: DateTime<Utc>)` for pruning old records.
- Default retention: 90 days, configurable in `BudgetConfig`.
- Run at startup (once, async) via `tokio::spawn`.

---

## Key Crate APIs

- `once_cell::sync::Lazy<T>` — static `PRICING` table initialized on first access.
- `sqlx::query!("INSERT INTO llm_usage ...")` — compile-time SQL verification.
- `sqlx::query!("SELECT SUM(cost_microdollars) FROM llm_usage WHERE timestamp BETWEEN ? AND ?")` — aggregate queries.
- `tokio::sync::broadcast::Sender<BudgetEvent>` — push events to TUI without blocking callers.
- `tokio::sync::watch::Sender<CostSummary>` — keep status bar current without polling.
- `tokio::sync::RwLock<BudgetConfig>` — shared budget config read by enforcer, written by settings save.
- `tokio::join!(a, b, c, d)` — parallel DB queries in `CostDashboardLoader::load()`.
- `ratatui::widgets::Gauge` — monthly budget progress bar.
- `ratatui::widgets::BarChart` — 7-day daily spend visualization.
- `ratatui::widgets::Table` + `TableState` — per-operation breakdown with selection.
- `chrono::Utc::now()`, `chrono::Duration::days(30)` — period range computation.
- `csv::Writer::from_writer(writer)` + `serialize()` — CSV export.

---

## Error Handling

```rust
// lazyjob-core/src/cost/mod.rs

#[derive(thiserror::Error, Debug)]
pub enum CostError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("budget config deserialization failed: {0}")]
    ConfigDeserialize(#[from] serde_json::Error),

    #[error("request blocked: {reason}")]
    RequestBlocked { reason: String },

    #[error("chrono out-of-range: {0}")]
    ChronoOutOfRange(#[from] chrono::OutOfRangeError),
}

pub type Result<T> = std::result::Result<T, CostError>;
```

`CostError::RequestBlocked` is returned to the caller when `BudgetEnforcer::check_before_call`
returns `CheckResult::Blocked`. Callers (ralph workers, pipeline stages) are expected to convert
this to a `WorkerEvent::Error` or surface it in the TUI action result.

---

## Testing Strategy

### Unit Tests

**`lazyjob-llm/src/cost/pricing.rs`**
- `test_estimate_known_model`: claude-3-5-sonnet $3 input + $15 output for 1M/1M tokens
  → `Microdollars(18_000_000)`.
- `test_estimate_unknown_model`: `is_free_or_unknown = true` for "ollama/llama3".
- `test_estimate_from_chars`: 400 chars → 100 prompt tokens, 500 completion assumed → correct
  microdollar value.

**`lazyjob-llm/src/cost/types.rs`**
- `test_microdollars_add`: overflow check at i64::MAX/2 + i64::MAX/2.
- `test_operation_type_roundtrip`: all 12 `OperationType` variants survive `to_db_str()` /
  `from_db_str()` round-trip with no panics.
- `test_microdollars_display`: `Microdollars(3_500_000).to_dollars_2dp()` == `"$3.50"`.

**`lazyjob-core/src/cost/budget.rs`**
- Mock `UsageRepository` returning zero monthly cost → `check_before_call` returns `Allowed`.
- Mock returning 99% of monthly limit + incoming estimate > 1% → `Blocked`.
- Mock returning 74% + small estimate → `ThresholdCrossed { 75 }` event NOT fired; mock
  returning 75% → event fired exactly once per period (idempotent alert log mock).

### Integration Tests (`#[sqlx::test(migrations = "migrations")]`)

**`tests/cost_repository.rs`**
- Insert 5 `UsageRecord`s with different operations and timestamps; assert:
  - `get_summary(CurrentMonth).cost` == sum of all microdollar values.
  - `get_by_operation(CurrentMonth)` groups correctly, sorted desc by cost.
  - `get_daily_totals(7)` returns one row per day with correct sums.
  - `get_operation_total(JobDiscovery, CurrentDay)` counts only `job_discovery` rows.

**`tests/budget_enforcer_integration.rs`**
- Full `BudgetEnforcer` with real `SqliteUsageRepository` and `budget_config` row with
  `monthly_limit = Microdollars(10_000)` (1 cent):
  - First call with `estimated_cost = Microdollars(5_000)` → `Allowed`.
  - Second call with `estimated_cost = Microdollars(6_000)` → `Blocked`.

### TUI Tests

- `cost_pill::test_render`: render with a `CostSummary` at 85% — assert output contains
  red-styled span and `85%` text.
- `cost_dashboard::test_render_empty`: render with `loading = true` — assert "Loading..."
  placeholder text present.
- `cost_dashboard::test_operation_table_sorted`: `by_operation` list with 3 entries —
  assert table rows appear in descending cost order.

---

## Open Questions

1. **Microdollars vs cents**: The spec uses `DollarCents` (i64 hundredths). This plan
   uses `Microdollars` (i64 millionths) for sub-cent precision. Decision: adopt microdollars
   since Haiku / gpt-4o-mini calls cost < 0.1 cents and would round to zero in cents.

2. **Currency**: MVP is USD-only. Future: store `currency_code TEXT NOT NULL DEFAULT 'USD'`
   in `llm_usage` and use a `Decimal` (from the `rust_decimal` crate) for multi-currency
   display. Deferred — no non-USD provider pricing in 2024 scope.

3. **Refund handling**: API providers can issue credits. No tracking in MVP; could add a
   `llm_credits` table with `credit_microdollars` and subtract from `get_summary` total.

4. **Ollama electricity cost**: Defaults to `is_free_or_unknown = true`, not enforced by
   `BudgetEnforcer`. Phase 4 adds opt-in local cost tracking but never blocks calls.

5. **Batch API pricing**: `ModelCost.batch_input_rate/batch_output_rate` fields are defined
   but not yet used. Wire in when `LlmProvider::chat_batch()` is implemented.

6. **Budget sync for SaaS**: `budget_config` table syncs via the standard outbox table
   (see spec 18). Offer/salary data exclusion from sync applies to `llm_usage` too since
   it reveals what features the user is using — mark as `sync_excluded` in the outbox writer.

7. **`OperationType::from_db_str` unknown values**: Currently falls through to `BackgroundTask`.
   Consider returning `Result<_, CostError>` instead to detect data corruption.

---

## Related Specs

- [specs/agentic-llm-provider-abstraction.md](./agentic-llm-provider-abstraction.md) — defines `LlmProvider`, `ChatRequest`, `TokenUsage`; cost recording hooks into its response path.
- [specs/04-sqlite-persistence-implementation-plan.md](./04-sqlite-persistence-implementation-plan.md) — migration runner applies `005_llm_usage.sql`.
- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — ralph loop dispatcher subscribes to `BudgetEvent::RalphLoopPaused`.
- [specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) — `ralph_loop_id` propagated from `WorkerCommand::Start` payload through all LLM calls.
- [specs/XX-llm-prompt-versioning.md](./XX-llm-prompt-versioning.md) — prompt versions correlate with cost deltas for A/B cost analysis.
- [specs/18-saas-migration-path.md](./18-saas-migration-path.md) — `budget_config` and `llm_usage` sync policy under SaaS.
