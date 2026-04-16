# Implementation Plan: Salary Negotiation and Offer Evaluation

## Status
Draft

## Related Spec
[specs/salary-negotiation-offers.md](salary-negotiation-offers.md)

## Overview

The salary negotiation and offer evaluation module gives candidates the quantitative and communication tools to evaluate multi-component offers accurately and negotiate effectively. It has three distinct responsibilities: (1) **Offer Evaluation Engine** — takes a full offer letter (base, equity grant, vesting schedule, signing bonus, cash bonus) and computes an annualized risk-adjusted total compensation figure, benchmarked against market data already in SQLite from the salary market intelligence module; (2) **BATNA and Negotiation Strategy** — computes the candidate's Best Alternative to a Negotiated Agreement across all active offers and produces a structured negotiation plan (target TC, walk-away TC, talking-point list, what-to-ask-for by component); (3) **Counter-Offer Drafting** — a Ralph async loop that produces a human-reviewable draft email and phone script, injecting market data, competing offer signals, and the candidate's stated priorities. The human always approves before sending — the agent drafts, the human owns.

This module is closely coupled with `lazyjob-core/src/salary/` (the market intelligence layer from the prior plan). All offer modeling types (`OfferRecord`, `EquityGrant`, `VestingSchedule`, `CompanyStage`) defined in `salary-market-intelligence-implementation-plan.md` are reused directly here rather than duplicated. This plan extends those types with negotiation-specific state (`NegotiationStatus`, `CounterOfferDraft`, `BatnaSnapshot`) and adds the negotiation service layer and TUI components.

The entire computation layer (`compute_total_comp`, `compute_batna`, `rank_offers`) is synchronous and pure — no I/O, testable with `cargo test` without a database. The async surface is limited to `NegotiationService::draft_counter_offer()` (calls the LLM via `lazyjob-llm`) and repository read/write operations.

## Prerequisites

### Must be implemented first
- `specs/salary-market-intelligence-implementation-plan.md` — `OfferRecord`, `OfferEvaluation`, `compute_total_comp`, `offers` SQLite table, `SalaryIntelligenceService`
- `specs/04-sqlite-persistence-implementation-plan.md` — Database connection pool, migration runner
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait, streaming chat
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — `RalphProcessManager`, `WorkerEvent`
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel system, `FormWidget`

### Crates to add to Cargo.toml
```toml
# lazyjob-core/Cargo.toml
[dependencies]
ordered-float = "4"   # NaN-free f64 wrapper for sorting risk-adjusted TC values
```

No new crates for `lazyjob-tui` — form widget, table, and bar chart already in `ratatui`.

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| Offer data model extensions | `lazyjob-core` | `src/salary/negotiation.rs` |
| Total comp computation | `lazyjob-core` | `src/salary/compute.rs` (extends existing) |
| BATNA calculator | `lazyjob-core` | `src/salary/batna.rs` |
| Offer comparison engine | `lazyjob-core` | `src/salary/comparison.rs` |
| Negotiation service (orchestrator) | `lazyjob-core` | `src/salary/negotiation_service.rs` |
| Counter-offer Ralph loop | `lazyjob-ralph` | `src/loops/counter_offer.rs` |
| TUI offer form | `lazyjob-tui` | `src/views/salary/offer_form.rs` |
| TUI comparison view | `lazyjob-tui` | `src/views/salary/comparison.rs` |
| TUI negotiation panel | `lazyjob-tui` | `src/views/salary/negotiation.rs` |

### Core Types

```rust
// lazyjob-core/src/salary/negotiation.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Wraps a UUID for a negotiation session (one session per application).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct NegotiationId(pub Uuid);

impl NegotiationId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// High-level status of negotiation for one application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum NegotiationStatus {
    /// Offer received, not yet evaluated.
    OfferReceived,
    /// User has entered full offer details; evaluation computed.
    Evaluated,
    /// Counter-offer draft generated, pending user review.
    DraftReady,
    /// User reviewed and sent counter-offer.
    CounterSent,
    /// Employer came back with a revised offer.
    RevisedOfferReceived,
    /// Negotiation closed — offer accepted.
    Accepted,
    /// Negotiation closed — offer declined.
    Declined,
}

/// User's stated priorities for negotiation. Stored as JSON in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationPriorities {
    /// Priority weights in [0, 10]. Higher = more important.
    pub base_salary_weight: u8,
    pub equity_weight: u8,
    pub signing_bonus_weight: u8,
    pub annual_bonus_weight: u8,
    pub start_date_weight: u8,
    pub remote_flexibility_weight: u8,
    /// TC floor below which the candidate will walk (cents/year, annualized).
    pub walk_away_tc_cents: i64,
    /// TC target the candidate wants to reach (cents/year, annualized).
    pub target_tc_cents: i64,
    /// Free text: anything the candidate wants the agent to know.
    pub additional_context: Option<String>,
}

/// A single counter-offer draft. Multiple drafts can exist per negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterOfferDraft {
    pub id: Uuid,
    pub negotiation_id: NegotiationId,
    pub created_at: DateTime<Utc>,
    /// Full draft email body (markdown, ready to paste).
    pub email_body: String,
    /// Phone script as a bulleted list of talking points.
    pub phone_script: String,
    /// Suggested specific ask per component (in cents where monetary).
    pub suggested_base_cents: Option<i64>,
    pub suggested_equity_grant_cents: Option<i64>,
    pub suggested_signing_cents: Option<i64>,
    pub suggested_bonus_pct: Option<f32>,
    pub suggested_start_date: Option<NaiveDate>,
    /// Which offer version this draft is based on (offer_id FK).
    pub offer_id: Uuid,
    /// Did the user mark this draft as "used" / sent?
    pub was_sent: bool,
}

/// Point-in-time snapshot of the BATNA computation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatnaSnapshot {
    pub computed_at: DateTime<Utc>,
    /// Best Alternative: the highest annualized TC from all other active offers.
    /// `None` if there are no competing offers.
    pub best_alternative_tc_cents: Option<i64>,
    /// The application_id of the best alternative offer.
    pub best_alternative_application_id: Option<Uuid>,
    /// The current offer's annualized risk-adjusted TC.
    pub current_offer_tc_cents: i64,
    /// Gap between current offer and BATNA (positive = current is better).
    pub tc_gap_cents: i64,
    /// Leverage signal: HIGH if best_alternative_tc >= current offer,
    /// MEDIUM if within 10%, LOW otherwise.
    pub leverage: LeverageSignal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeverageSignal {
    High,   // Competing offer ≥ current offer — strong negotiating position
    Medium, // Competing offer within 10% below
    Low,    // No competing offers or competing offer >10% below
}

/// Full negotiation record per application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationRecord {
    pub id: NegotiationId,
    pub application_id: Uuid,
    pub status: NegotiationStatus,
    pub priorities: NegotiationPriorities,
    /// Latest BATNA snapshot (recomputed on demand).
    pub batna: Option<BatnaSnapshot>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

```rust
// lazyjob-core/src/salary/comparison.rs

/// An offer ranked for side-by-side comparison.
#[derive(Debug, Clone)]
pub struct RankedOffer {
    pub offer_id: Uuid,
    pub application_id: Uuid,
    pub company_name: String,
    pub role_title: String,
    /// Annualized risk-adjusted TC in cents.
    pub total_comp_cents: i64,
    /// Raw (non-risk-adjusted) TC in cents.
    pub nominal_total_comp_cents: i64,
    /// Component breakdown (same struct as OfferEvaluation, reused from market intelligence).
    pub components: OfferComponentBreakdown,
    /// Market percentile from SalaryIntelligenceService (0..=100, None if no market data).
    pub market_percentile: Option<u8>,
    /// User-weighted score in [0.0, 1.0], combining TC and personal priorities.
    pub weighted_score: f32,
    /// Rank by weighted_score (1 = best).
    pub rank: usize,
}

/// Component-by-component breakdown (cents/year).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferComponentBreakdown {
    pub base_salary_cents: i64,
    pub equity_annualized_cents: i64,
    pub equity_risk_factor: f32,
    pub equity_risk_adjusted_cents: i64,
    pub signing_bonus_amortized_cents: i64, // Divided by expected tenure (default 2yr)
    pub annual_bonus_cents: i64,
    pub benefits_estimated_cents: i64, // From user-entered estimate
    pub total_cents: i64,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/salary/negotiation_service.rs

use async_trait::async_trait;

#[async_trait]
pub trait NegotiationRepository: Send + Sync {
    async fn upsert(&self, record: &NegotiationRecord) -> Result<(), NegotiationError>;
    async fn find_by_application(
        &self,
        application_id: Uuid,
    ) -> Result<Option<NegotiationRecord>, NegotiationError>;
    async fn list_active(&self) -> Result<Vec<NegotiationRecord>, NegotiationError>;
    async fn save_draft(&self, draft: &CounterOfferDraft) -> Result<(), NegotiationError>;
    async fn list_drafts(
        &self,
        negotiation_id: &NegotiationId,
    ) -> Result<Vec<CounterOfferDraft>, NegotiationError>;
    async fn mark_draft_sent(&self, draft_id: Uuid) -> Result<(), NegotiationError>;
}

/// Orchestrates the full negotiation lifecycle.
pub struct NegotiationService {
    repo: Arc<dyn NegotiationRepository>,
    offer_repo: Arc<dyn OfferRepository>,       // from market intelligence module
    salary_svc: Arc<SalaryIntelligenceService>, // from market intelligence module
    llm: Arc<dyn LlmProvider>,
}

impl NegotiationService {
    /// Compute (or recompute) the BATNA for a given offer by querying
    /// all other offers with status Evaluated/DraftReady/CounterSent/RevisedOfferReceived.
    pub async fn compute_batna(
        &self,
        application_id: Uuid,
    ) -> Result<BatnaSnapshot, NegotiationError>;

    /// Rank all active offers for side-by-side comparison, applying the
    /// user's NegotiationPriorities as weights.
    pub async fn rank_offers(
        &self,
        priorities: &NegotiationPriorities,
    ) -> Result<Vec<RankedOffer>, NegotiationError>;

    /// Dispatch a Ralph loop to draft a counter-offer email and phone script.
    /// Returns the draft ID; the caller polls `list_drafts()` for completion.
    pub async fn request_draft(
        &self,
        negotiation_id: &NegotiationId,
    ) -> Result<Uuid, NegotiationError>;

    /// Synchronous method: given priorities and an OfferEvaluation, return
    /// the suggested TC targets as a NegotiationTarget (no LLM needed).
    pub fn compute_target(
        evaluation: &OfferEvaluation,
        market_data: Option<&MarketDataRange>,
        priorities: &NegotiationPriorities,
        batna: &BatnaSnapshot,
    ) -> NegotiationTarget;
}

/// Pure synchronous struct produced by compute_target().
#[derive(Debug, Clone)]
pub struct NegotiationTarget {
    /// Annualized TC we want to reach. Computed as:
    /// max(target_tc_cents, p75_market_data, batna.best_alternative_tc_cents + 5%)
    pub target_tc_cents: i64,
    /// Floor below which the deal is rejected.
    pub walk_away_tc_cents: i64,
    /// Suggested per-component asks (prioritized by user weights).
    pub priority_order: Vec<NegotiationPriorityItem>,
}

#[derive(Debug, Clone)]
pub struct NegotiationPriorityItem {
    pub component: NegotiationComponent,
    pub current_cents: i64,
    pub target_cents: i64,
    /// Human-readable rationale for the specific ask.
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegotiationComponent {
    BaseSalary,
    EquityGrant,
    SigningBonus,
    AnnualBonus,
    StartDate,
}
```

### SQLite Schema

```sql
-- Migration 014: negotiation module

CREATE TABLE IF NOT EXISTS negotiation_records (
    id                   TEXT PRIMARY KEY,           -- UUID
    application_id       TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'offer_received',
    priorities_json      TEXT NOT NULL,              -- NegotiationPriorities as JSON
    batna_json           TEXT,                       -- BatnaSnapshot as JSON, nullable
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(application_id)                           -- one negotiation per application
);

CREATE INDEX IF NOT EXISTS idx_negotiation_status ON negotiation_records(status);

CREATE TABLE IF NOT EXISTS counter_offer_drafts (
    id                          TEXT PRIMARY KEY,    -- UUID
    negotiation_id              TEXT NOT NULL REFERENCES negotiation_records(id) ON DELETE CASCADE,
    offer_id                    TEXT NOT NULL REFERENCES offers(id),
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    email_body                  TEXT NOT NULL,
    phone_script                TEXT NOT NULL,
    suggested_base_cents        INTEGER,
    suggested_equity_grant_cents INTEGER,
    suggested_signing_cents     INTEGER,
    suggested_bonus_pct         REAL,
    suggested_start_date        TEXT,
    was_sent                    INTEGER NOT NULL DEFAULT 0  -- boolean: 0/1
);

CREATE INDEX IF NOT EXISTS idx_draft_negotiation ON counter_offer_drafts(negotiation_id);
CREATE INDEX IF NOT EXISTS idx_draft_created ON counter_offer_drafts(created_at);
```

### Module Structure

```
lazyjob-core/
  src/
    salary/
      mod.rs                  -- pub use negotiation::*, comparison::*, batna::*
      model.rs                -- OfferRecord, EquityGrant, VestingSchedule (prior plan)
      compute.rs              -- compute_total_comp (prior plan), extend with risk factor
      market.rs               -- MarketDataRange, SalaryIntelligenceService (prior plan)
      negotiation.rs          -- NegotiationRecord, CounterOfferDraft, BatnaSnapshot (NEW)
      batna.rs                -- compute_batna() pure logic (NEW)
      comparison.rs           -- RankedOffer, rank_offers() (NEW)
      negotiation_service.rs  -- NegotiationService orchestrator (NEW)
      repository.rs           -- SqliteNegotiationRepository (NEW)
      jurisdictions.rs        -- PAY_TRANSPARENT_JURISDICTIONS (prior plan)

lazyjob-ralph/
  src/
    loops/
      counter_offer.rs        -- CounterOfferLoop subprocess (NEW)

lazyjob-tui/
  src/
    views/
      salary/
        mod.rs
        offer_form.rs         -- OfferFormView (NEW)
        comparison.rs         -- OfferComparisonView (NEW)
        negotiation.rs        -- NegotiationPanel (NEW)
```

## Implementation Phases

### Phase 1 — Core Data Model and BATNA Computation (MVP foundation)

**Step 1.1 — Define negotiation types**

File: `lazyjob-core/src/salary/negotiation.rs`

Add `NegotiationRecord`, `NegotiationStatus`, `NegotiationPriorities`, `CounterOfferDraft`, `BatnaSnapshot`, `LeverageSignal`, and `NegotiationTarget` as shown in Core Types above.

Key implementation detail: `NegotiationPriorities` stores `walk_away_tc_cents` and `target_tc_cents` as `i64` (cents/year). The TUI enters these in dollars and converts. No floating-point in storage.

**Step 1.2 — Extend compute_total_comp for risk adjustment**

File: `lazyjob-core/src/salary/compute.rs`

The `compute_total_comp` function from the market intelligence plan already computes `equity_risk_adjusted_cents`. Verify it exposes:

```rust
pub fn compute_total_comp(offer: &OfferRecord) -> OfferComponentBreakdown {
    let equity_annualized = offer.equity_grant.as_ref()
        .map(|g| g.total_grant_usd_cents.unwrap_or(0) / g.vest_years as i64)
        .unwrap_or(0);

    let risk_factor = offer.equity_grant.as_ref()
        .map(|g| g.user_risk_factor.unwrap_or_else(|| offer.company_stage.default_risk_factor()))
        .unwrap_or(1.0);

    let equity_risk_adjusted = (equity_annualized as f64 * risk_factor as f64) as i64;

    // Signing bonus amortized over 2 years (standard tenure assumption).
    let signing_amortized = offer.signing_bonus_cents.unwrap_or(0) / 2;

    let annual_bonus = offer.target_bonus_cents.unwrap_or(0);
    let benefits = offer.benefits_estimated_value_cents.unwrap_or(0);

    let total = offer.base_salary_cents
        + equity_risk_adjusted
        + signing_amortized
        + annual_bonus
        + benefits;

    OfferComponentBreakdown {
        base_salary_cents: offer.base_salary_cents,
        equity_annualized_cents: equity_annualized,
        equity_risk_factor: risk_factor,
        equity_risk_adjusted_cents: equity_risk_adjusted,
        signing_bonus_amortized_cents: signing_amortized,
        annual_bonus_cents: annual_bonus,
        benefits_estimated_cents: benefits,
        total_cents: total,
    }
}
```

Verification: `cargo test -p lazyjob-core salary::compute` — all unit tests pass with known fixture values.

**Step 1.3 — Implement pure BATNA calculator**

File: `lazyjob-core/src/salary/batna.rs`

```rust
/// Pure function — no I/O. Takes all active offer evaluations (excluding the
/// current application) and returns a BatnaSnapshot.
pub fn compute_batna(
    current_eval: &OfferComponentBreakdown,
    current_application_id: Uuid,
    competing_offers: &[(Uuid, OfferComponentBreakdown)], // (application_id, breakdown)
) -> BatnaSnapshot {
    let best = competing_offers
        .iter()
        .max_by_key(|(_, b)| b.total_cents);

    let (best_alt_tc, best_alt_app_id) = best
        .map(|(id, b)| (Some(b.total_cents), Some(*id)))
        .unwrap_or((None, None));

    let tc_gap = current_eval.total_cents - best_alt_tc.unwrap_or(0);

    let leverage = match best_alt_tc {
        None => LeverageSignal::Low,
        Some(alt) if alt >= current_eval.total_cents => LeverageSignal::High,
        Some(alt) => {
            let pct_below = (current_eval.total_cents - alt) as f64
                / current_eval.total_cents as f64;
            if pct_below <= 0.10 { LeverageSignal::Medium } else { LeverageSignal::Low }
        }
    };

    BatnaSnapshot {
        computed_at: Utc::now(),
        best_alternative_tc_cents: best_alt_tc,
        best_alternative_application_id: best_alt_app_id,
        current_offer_tc_cents: current_eval.total_cents,
        tc_gap_cents: tc_gap,
        leverage,
    }
}
```

Verification: `cargo test -p lazyjob-core salary::batna` — test with 0 competing offers, 1 equal offer, 1 better offer, 1 worse offer.

**Step 1.4 — SQLite migration 014**

File: `lazyjob-core/migrations/014_negotiation.sql`

Apply the DDL from the SQLite Schema section above.

Verification: `cargo test -p lazyjob-core` — migration runner applies migration 014 cleanly to an in-memory test DB.

**Step 1.5 — SqliteNegotiationRepository**

File: `lazyjob-core/src/salary/repository.rs`

```rust
pub struct SqliteNegotiationRepository {
    pool: SqlitePool,
}

impl SqliteNegotiationRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl NegotiationRepository for SqliteNegotiationRepository {
    async fn upsert(&self, record: &NegotiationRecord) -> Result<(), NegotiationError> {
        let priorities_json = serde_json::to_string(&record.priorities)
            .map_err(NegotiationError::Serialization)?;
        let batna_json = record.batna.as_ref()
            .map(|b| serde_json::to_string(b))
            .transpose()
            .map_err(NegotiationError::Serialization)?;

        sqlx::query!(
            r#"
            INSERT INTO negotiation_records
                (id, application_id, status, priorities_json, batna_json, updated_at)
            VALUES (?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(id) DO UPDATE SET
                status         = excluded.status,
                priorities_json = excluded.priorities_json,
                batna_json     = excluded.batna_json,
                updated_at     = excluded.updated_at
            "#,
            record.id.0,
            record.application_id,
            record.status as NegotiationStatus,
            priorities_json,
            batna_json,
        )
        .execute(&self.pool)
        .await
        .map_err(NegotiationError::Database)?;
        Ok(())
    }

    // ... find_by_application, list_active, save_draft, list_drafts, mark_draft_sent
}
```

Verification: `#[sqlx::test(migrations = "migrations")]` integration tests for upsert + find + list round-trip.

---

### Phase 2 — Offer Comparison Engine

**Step 2.1 — Implement rank_offers with weighted scoring**

File: `lazyjob-core/src/salary/comparison.rs`

The weighted score formula combines TC percentile (primary) with component-level alignment to user priorities:

```rust
/// Pure function. Returns offers sorted by weighted_score descending.
pub fn rank_offers(
    offers: Vec<(OfferRecord, OfferComponentBreakdown, Option<u8>)>, // (offer, breakdown, market_pct)
    priorities: &NegotiationPriorities,
) -> Vec<RankedOffer> {
    // Normalize weights to sum to 1.0
    let total_weight = priorities.base_salary_weight as f32
        + priorities.equity_weight as f32
        + priorities.signing_bonus_weight as f32
        + priorities.annual_bonus_weight as f32;
    let w_base = priorities.base_salary_weight as f32 / total_weight;
    let w_equity = priorities.equity_weight as f32 / total_weight;
    let w_sign = priorities.signing_bonus_weight as f32 / total_weight;
    let w_bonus = priorities.annual_bonus_weight as f32 / total_weight;

    // Compute max of each component across all offers for normalization.
    let max_base = offers.iter().map(|(_, b, _)| b.base_salary_cents).max().unwrap_or(1);
    let max_equity = offers.iter().map(|(_, b, _)| b.equity_risk_adjusted_cents).max().unwrap_or(1);
    let max_signing = offers.iter().map(|(_, b, _)| b.signing_bonus_amortized_cents).max().unwrap_or(1);
    let max_bonus = offers.iter().map(|(_, b, _)| b.annual_bonus_cents).max().unwrap_or(1);

    let mut ranked: Vec<RankedOffer> = offers
        .into_iter()
        .map(|(offer, breakdown, market_pct)| {
            let score_base   = breakdown.base_salary_cents as f32 / max_base as f32;
            let score_equity = breakdown.equity_risk_adjusted_cents as f32 / max_equity.max(1) as f32;
            let score_sign   = breakdown.signing_bonus_amortized_cents as f32 / max_signing.max(1) as f32;
            let score_bonus  = breakdown.annual_bonus_cents as f32 / max_bonus.max(1) as f32;

            let weighted = w_base * score_base
                + w_equity * score_equity
                + w_sign * score_sign
                + w_bonus * score_bonus;

            RankedOffer {
                offer_id: offer.id,
                application_id: offer.application_id,
                company_name: offer.company_name.clone(),
                role_title: offer.role_title.clone(),
                total_comp_cents: breakdown.total_cents,
                nominal_total_comp_cents: breakdown.base_salary_cents
                    + breakdown.equity_annualized_cents
                    + breakdown.signing_bonus_amortized_cents
                    + breakdown.annual_bonus_cents,
                components: breakdown,
                market_percentile: market_pct,
                weighted_score: weighted,
                rank: 0, // filled in below
            }
        })
        .collect();

    // Sort descending by weighted_score, breaking ties by total_comp_cents.
    ranked.sort_by(|a, b| {
        b.weighted_score.partial_cmp(&a.weighted_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.total_comp_cents.cmp(&a.total_comp_cents))
    });

    // Assign ranks (1-based).
    for (i, offer) in ranked.iter_mut().enumerate() {
        offer.rank = i + 1;
    }
    ranked
}
```

**Step 2.2 — Implement NegotiationTarget computation**

File: `lazyjob-core/src/salary/negotiation_service.rs`

```rust
impl NegotiationService {
    pub fn compute_target(
        evaluation: &OfferComponentBreakdown,
        market_data: Option<&MarketDataRange>,
        priorities: &NegotiationPriorities,
        batna: &BatnaSnapshot,
    ) -> NegotiationTarget {
        // Target TC = max of:
        //   1. User's stated target
        //   2. Market p75 (if available)
        //   3. BATNA best alternative + 5% (only if leverage is High/Medium)
        let p75 = market_data.map(|m| m.p75_cents).unwrap_or(0);
        let batna_plus_5 = match batna.leverage {
            LeverageSignal::High | LeverageSignal::Medium =>
                batna.best_alternative_tc_cents.unwrap_or(0) * 105 / 100,
            LeverageSignal::Low => 0,
        };
        let target_tc = priorities.target_tc_cents
            .max(p75)
            .max(batna_plus_5);

        // Build priority-ordered list of component asks.
        // We compute a gap per component and weight by user priorities.
        let mut items: Vec<(u8, NegotiationPriorityItem)> = Vec::new();

        let current_tc = evaluation.total_cents;
        let tc_gap = target_tc - current_tc;

        if tc_gap > 0 && priorities.base_salary_weight > 0 {
            // Suggest directing gap proportionally to base.
            let base_share = (tc_gap as f64
                * priorities.base_salary_weight as f64
                / 10.0) as i64;
            let target_base = evaluation.base_salary_cents + base_share;
            // Round to nearest $1000 for realism.
            let target_base_rounded = (target_base / 100_000 + 1) * 100_000;
            items.push((
                priorities.base_salary_weight,
                NegotiationPriorityItem {
                    component: NegotiationComponent::BaseSalary,
                    current_cents: evaluation.base_salary_cents,
                    target_cents: target_base_rounded,
                    rationale: format!(
                        "Base salary represents a foundational, compounding component. \
                         Market p75 for this role is {}.",
                        market_data.map(|m| format_dollars(m.p75_cents))
                            .unwrap_or_else(|| "unknown".to_string())
                    ),
                },
            ));
        }

        // ... similar blocks for equity, signing bonus, annual bonus

        items.sort_by(|(w_a, _), (w_b, _)| w_b.cmp(w_a));

        NegotiationTarget {
            target_tc_cents: target_tc,
            walk_away_tc_cents: priorities.walk_away_tc_cents,
            priority_order: items.into_iter().map(|(_, item)| item).collect(),
        }
    }
}
```

Verification: Unit test with a mock `OfferComponentBreakdown`, two `MarketDataRange` values (one above target, one below), and three `LeverageSignal` variants.

---

### Phase 3 — Counter-Offer Ralph Loop

**Step 3.1 — Define the CounterOffer loop type**

File: `lazyjob-ralph/src/loops/counter_offer.rs`

`LoopType::CounterOfferDraft` is added to the enum (see `agentic-ralph-orchestration-implementation-plan.md` for `LoopType`). It is non-interactive (no bidirectional stdin), single-shot (one `Start` → multiple `Progress` → `Done`), and has a concurrency limit of 1 per negotiation (the orchestrator enforces this by checking `active_loops` before dispatching).

**Loop input JSON (passed as `params` in `WorkerCommand::Start`):**

```json
{
  "loop_type": "counter_offer_draft",
  "negotiation_id": "<uuid>",
  "offer": {
    "base_salary_cents": 20000000,
    "equity_grant_total_cents": 10000000,
    "vest_years": 4,
    "signing_bonus_cents": 3000000,
    "target_bonus_cents": 2000000,
    "company_stage": "public",
    "company_name": "Acme Corp",
    "role_title": "Senior Software Engineer"
  },
  "market_data": {
    "p25_cents": 18000000,
    "p50_cents": 21000000,
    "p75_cents": 25000000
  },
  "target": {
    "target_tc_cents": 24000000,
    "walk_away_tc_cents": 19000000,
    "priority_order": [
      { "component": "BaseSalary", "current_cents": 20000000, "target_cents": 22000000, "rationale": "..." },
      { "component": "EquityGrant", "current_cents": 2500000, "target_cents": 3000000, "rationale": "..." }
    ]
  },
  "batna": {
    "leverage": "High",
    "best_alternative_tc_cents": 23000000,
    "best_alternative_application_id": "<uuid>"
  },
  "priorities": {
    "additional_context": "I have 6 YOE in distributed systems. My current TC is $195k.",
    "base_salary_weight": 8,
    "equity_weight": 6
  }
}
```

**Step 3.2 — Prompt template for counter-offer drafting**

File: `lazyjob-ralph/src/loops/counter_offer.toml` (embedded via `include_str!`)

```toml
[system]
content = """
You are a salary negotiation coach for {company_name}. You help candidates
draft professional, firm, and warm counter-offer emails. You never fabricate
competing offers — you reference them only if the user confirms they exist.
You never write text that sounds AI-generated (avoid "I hope this email finds
you well", "I'm excited about this opportunity", "leverage").

Your job:
1. Draft a counter-offer email (150-250 words, professional, specific).
2. Draft a phone script (bullet list of talking points, 3-5 bullets max).
3. Output exactly this JSON and nothing else:
{
  "email_body": "...",
  "phone_script": "...",
  "suggested_base_cents": <integer or null>,
  "suggested_equity_grant_cents": <integer or null>,
  "suggested_signing_cents": <integer or null>,
  "suggested_bonus_pct": <float or null>,
  "suggested_start_date": "<YYYY-MM-DD or null>"
}
"""
cache_system_prompt = true

[user]
content = """
Offer details:
- Company: {company_name}
- Role: {role_title}
- Base salary: {base_salary}
- Equity grant: {equity_grant} over {vest_years} years (risk factor: {risk_factor})
- Signing bonus: {signing_bonus}
- Annual bonus target: {annual_bonus}
- Annualized risk-adjusted TC: {total_comp}

Market context:
- Market p25: {market_p25} | p50: {market_p50} | p75: {market_p75}
- Leverage signal: {leverage}
{batna_line}

Negotiation priorities (ordered):
{priority_list}

Additional context: {additional_context}

Draft the counter-offer email and phone script.
"""
```

The `batna_line` variable is only injected when `leverage != Low`. Template rendering skips the line (outputs empty string) when the variable is empty. This prevents fabricating a competing offer when the user has none.

**Step 3.3 — CounterOfferLoop worker**

File: `lazyjob-ralph/src/loops/counter_offer.rs`

```rust
pub struct CounterOfferLoop {
    llm: Arc<dyn LlmProvider>,
    repo: Arc<dyn NegotiationRepository>,
}

impl CounterOfferLoop {
    pub async fn run(
        &self,
        params: serde_json::Value,
        event_tx: broadcast::Sender<WorkerEvent>,
    ) -> Result<(), anyhow::Error> {
        let input: CounterOfferInput = serde_json::from_value(params)
            .context("invalid CounterOffer loop params")?;

        event_tx.send(WorkerEvent::Progress {
            message: "Analyzing offer and drafting counter-offer...".into(),
            progress_pct: Some(10),
        })?;

        let template = CounterOfferTemplate::render(&input);
        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: template.user_prompt,
        }];

        let response = self.llm
            .chat(messages, ChatOptions {
                system: Some(template.system_prompt),
                temperature: Some(0.4), // Slightly creative — needs human voice
                max_tokens: Some(1024),
            })
            .await
            .context("LLM call failed for counter-offer draft")?;

        let draft: CounterOfferDraftOutput = serde_json::from_str(&response.content)
            .context("LLM output was not valid JSON")?;

        let record = CounterOfferDraft {
            id: Uuid::new_v4(),
            negotiation_id: input.negotiation_id.clone(),
            created_at: Utc::now(),
            email_body: draft.email_body,
            phone_script: draft.phone_script,
            suggested_base_cents: draft.suggested_base_cents,
            suggested_equity_grant_cents: draft.suggested_equity_grant_cents,
            suggested_signing_cents: draft.suggested_signing_cents,
            suggested_bonus_pct: draft.suggested_bonus_pct,
            suggested_start_date: draft.suggested_start_date
                .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
            offer_id: input.offer_id,
            was_sent: false,
        };

        self.repo.save_draft(&record).await
            .context("failed to save counter-offer draft")?;

        event_tx.send(WorkerEvent::Done {
            result: serde_json::json!({ "draft_id": record.id }),
        })?;

        Ok(())
    }
}
```

Verification: Integration test using a `MockLlmProvider` that returns a hardcoded JSON string. Assert the draft is persisted to an in-memory SQLite DB.

---

### Phase 4 — TUI: Offer Entry Form

**Step 4.1 — OfferFormView**

File: `lazyjob-tui/src/views/salary/offer_form.rs`

A multi-step form with 4 sections navigated by `Tab`/`Shift-Tab`:

1. **Basic Compensation** — Base salary (dollars), target bonus % of base, signing bonus
2. **Equity** — Equity type (RSU/ISO/NSO via `<`/`>` selection), total grant value ($), vest years, cliff months, user risk factor override (slider 0.0–1.0)
3. **Company Stage** — radio selection: Public / Late Private / Mid Private / Early Private (auto-sets risk factor suggestion)
4. **Personal Priorities** — numeric weights (1–10) for each component, walk-away TC ($), target TC ($), free-text context area

All dollar fields parse `$200k`, `200000`, `200,000` formats via a shared `parse_dollar_input(s: &str) -> Option<i64>` helper (returns cents).

```rust
pub struct OfferFormView {
    app_id: Uuid,
    step: OfferFormStep,
    fields: OfferFormFields,
    validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OfferFormFields {
    pub base_salary_input: String,
    pub signing_bonus_input: String,
    pub target_bonus_pct_input: String,
    pub equity_type_idx: usize,
    pub equity_grant_input: String,
    pub vest_years_input: String,
    pub cliff_months_input: String,
    pub risk_factor_input: String,
    pub company_stage_idx: usize,
    pub walk_away_tc_input: String,
    pub target_tc_input: String,
    pub base_weight: u8,
    pub equity_weight: u8,
    pub signing_weight: u8,
    pub bonus_weight: u8,
    pub additional_context: String,
}
```

**Keybindings:**
- `Tab` / `Shift-Tab` — move between form fields
- `Enter` — advance to next section
- `Esc` — cancel (prompt if unsaved changes)
- `s` — save offer (only active when all required fields valid)
- `?` — open help overlay explaining equity types and risk factors

**Step 4.2 — Validation on submit**

Before saving, `OfferFormView::validate()` checks:
- `base_salary_cents > 0` (required)
- `vest_years in [1..=10]` for equity offers
- `risk_factor in [0.0..=1.0]`
- `walk_away_tc_cents < target_tc_cents`

Errors are displayed as a styled `Span` in red below the field. The form refuses to submit until all errors clear.

Verification: Manual TUI test — enter a complete offer for a public-company RSU grant; verify `OfferRecord` is written to SQLite and `OfferEvaluation` is computed and displayed in the status bar.

---

### Phase 5 — TUI: Comparison and Negotiation Views

**Step 5.1 — OfferComparisonView**

File: `lazyjob-tui/src/views/salary/comparison.rs`

Layout: full-width, two panes split vertically 30/70.

**Left pane (30%):** Ranked offer list using ratatui `List`. Each row shows:
```
[1] Acme Corp — Sr. SWE
    $247,500/yr risk-adj TC   [●●●●○] 78th pct
```
The 5-dot bar is a `Gauge`-style visual showing market percentile.

**Right pane (70%):** Component breakdown table for the selected offer. Uses ratatui `Table` with columns: Component | Current | Market p50 | Delta.

```
Component          Current     Mkt p50     Δ
─────────────────────────────────────────────
Base Salary        $200,000    $195,000    +$5,000 ↑
RSU (annualized)   $25,000     $30,000     -$5,000 ↓  (risk-adj: $25,000)
Signing (amort.)   $15,000     $10,000     +$5,000 ↑
Annual Bonus       $20,000     $18,000     +$2,000 ↑
─────────────────────────────────────────────
Risk-adj TC:       $247,500    $245,000    +$2,500
```

Cells are colored: green for positive delta, red for negative, white for zero.

**Priority sliders** at the bottom (ratatui `Gauge` repurposed as a non-interactive display). Show the user's weights visually so they can understand why the ranking order was produced.

**Keybindings:**
- `j`/`k` — navigate offer list
- `e` — edit priorities (opens OfferFormView in priorities-only mode)
- `n` — open NegotiationPanel for selected offer
- `d` — request counter-offer draft
- `Enter` — expand/collapse breakdown for selected offer

**Step 5.2 — NegotiationPanel**

File: `lazyjob-tui/src/views/salary/negotiation.rs`

A floating overlay (60% width, 80% height, centered) that shows:

1. **BATNA widget** (top): `LeverageSignal` displayed as colored badge (High=green, Medium=yellow, Low=red), BATNA TC, gap to current offer.
2. **NegotiationTarget widget** (middle): Walk-away TC, target TC, priority-ordered component asks as a bulleted list.
3. **Draft History** (bottom): Table of prior counter-offer drafts with timestamp and `[sent]` badge if `was_sent = true`. Press `Enter` on a draft row to view the full email body in a scrollable `Paragraph`.
4. **Actions** at bottom: `d` = request new draft, `s` = mark draft as sent, `Esc` = close.

When a draft is being generated (Ralph loop running), the BATNA widget area shows a `Gauge` spinner with "Drafting counter-offer..." text and progress_pct.

Verification: End-to-end TUI test — create two offers via the form, open comparison view, open negotiation panel, assert BATNA correctly identifies the competing offer as the best alternative.

---

## Key Crate APIs

- `sqlx::query!(...).execute(&pool)` — all SQLite writes (upsert, insert draft)
- `serde_json::to_string(&record)` / `serde_json::from_str(&s)` — JSON serialization for `priorities_json`, `batna_json`, `CounterOfferDraftOutput` parsing
- `ratatui::widgets::{Table, Row, Cell, List, ListState, Gauge, Paragraph, Block, Borders}` — TUI components
- `ratatui::layout::{Layout, Direction, Constraint}` — panel splitting
- `tokio::sync::broadcast::Sender<WorkerEvent>` — progress events from CounterOfferLoop to TUI
- `Arc<dyn LlmProvider>::chat(messages, options)` — LLM call in CounterOfferLoop
- `strsim::jaro_winkler` — NOT used in this module (comparison uses normalized float scoring, not string similarity)
- `once_cell::sync::Lazy<Regex>` — `parse_dollar_input` uses a single `Lazy<Regex>` for `$(\d[\d,]*)([kKmM]?)` pattern to parse dollar amounts from text fields

## Error Handling

```rust
// lazyjob-core/src/salary/negotiation.rs

#[derive(thiserror::Error, Debug)]
pub enum NegotiationError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("no offer found for application {application_id}")]
    OfferNotFound { application_id: Uuid },

    #[error("negotiation not found: {negotiation_id}")]
    NotFound { negotiation_id: String },

    #[error("walk-away TC ({walk_away}) must be less than target TC ({target})")]
    InvalidPriorities { walk_away: i64, target: i64 },

    #[error("LLM drafting failed: {0}")]
    DraftingFailed(#[from] anyhow::Error),

    #[error("LLM output was not valid JSON: {raw}")]
    MalformedDraft { raw: String },
}

pub type Result<T> = std::result::Result<T, NegotiationError>;
```

`MalformedDraft` is handled by `CounterOfferLoop::run` by retrying the LLM call once with an explicit "respond only with JSON" suffix appended to the user message. On second failure, `WorkerEvent::Error` is emitted.

## Testing Strategy

### Unit Tests

**`salary::batna` module:**
- `compute_batna_no_competing_offers` — leverage=Low, tc_gap=current_tc
- `compute_batna_equal_competing_offer` — leverage=High, tc_gap=0
- `compute_batna_10pct_below` — leverage=Medium (boundary case)
- `compute_batna_11pct_below` — leverage=Low

**`salary::comparison` module:**
- `rank_offers_single_offer` — rank=1, weighted_score=1.0
- `rank_offers_base_heavy_user` — high base_salary_weight, verifies offer with higher base wins despite lower total
- `rank_offers_equity_heavy_user` — inverted ranking from above
- `rank_offers_all_equal_offers` — stable sort by total_comp_cents as tiebreak

**`salary::negotiation_service` module:**
- `compute_target_no_market_data` — falls back to user's stated target_tc
- `compute_target_market_p75_wins` — p75 > user target, output = p75
- `compute_target_batna_wins` — batna_plus_5 > p75, output = batna_plus_5

**`parse_dollar_input` helper:**
- `"$200k"` → `Some(20_000_000)`
- `"200,000"` → `Some(20_000_000_000)` [i.e., 200000 dollars in cents]
- `"200"` → `Some(20_000)` (treated as dollars)
- `"not a number"` → `None`

### Integration Tests (sqlx::test)

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_negotiation_upsert_and_find(pool: SqlitePool) {
    let repo = SqliteNegotiationRepository::new(pool);
    let record = NegotiationRecord { /* ... */ };
    repo.upsert(&record).await.unwrap();
    let found = repo.find_by_application(record.application_id).await.unwrap();
    assert_eq!(found.unwrap().id, record.id);
}

#[sqlx::test(migrations = "migrations")]
async fn test_save_and_list_drafts(pool: SqlitePool) {
    // Save a negotiation, then a draft, then list drafts and verify count.
}

#[sqlx::test(migrations = "migrations")]
async fn test_mark_draft_sent(pool: SqlitePool) {
    // Save draft, mark sent, re-fetch and verify was_sent = true.
}
```

### CounterOfferLoop Test

```rust
#[tokio::test]
async fn test_counter_offer_loop_happy_path() {
    // MockLlmProvider returns a valid JSON string with email_body, phone_script, etc.
    // Assert: draft is saved to in-memory SQLite, WorkerEvent::Done is emitted.
}

#[tokio::test]
async fn test_counter_offer_loop_malformed_json_retry() {
    // MockLlmProvider returns invalid JSON on first call, valid on second.
    // Assert: one retry occurs, draft is saved successfully.
}

#[tokio::test]
async fn test_counter_offer_loop_two_failures() {
    // MockLlmProvider returns invalid JSON twice.
    // Assert: WorkerEvent::Error is emitted, no draft written to DB.
}
```

### TUI Tests

The TUI comparison view is tested manually (no automated TUI tests in MVP). Verification steps:
1. `lazyjob offer add` → fill form with sample values → assert saved to SQLite
2. `lazyjob offers compare` → assert ranked list shows the correct first-rank offer
3. Open NegotiationPanel → assert BATNA widget reflects correct leverage signal
4. Press `d` → assert Ralph loop starts and Draft History shows spinner
5. Wait for completion → assert draft appears in Draft History table

## Open Questions

1. **Signing bonus amortization period**: The plan uses 2 years as a fixed tenure assumption for amortization. Should this be configurable per offer? Some roles have 3-year cliffs. Suggest making it an optional field in `OfferRecord` (`expected_tenure_years: Option<u8>`) defaulting to 2 if unset.

2. **Benefits estimated value**: The spec mentions "health, 401k matching, PTO, etc." as part of total comp. The plan includes `benefits_estimated_value_cents` as a user-entered field. A future phase could auto-populate from a curated benefits cost table (health premium averages by state). For MVP, require user entry.

3. **Private company equity valuation**: The spec acknowledges no tool does this accurately for early-stage startups. The plan uses the `CompanyStage::default_risk_factor()` table (15% for Early Private). This is a coarse approximation. Phase 3 could add a `StartupEquityCalculator` using basic Black-Scholes inputs (if the user has strike price, FMV, volatility estimate). Deferred.

4. **Gender-aware coaching**: The spec references Babcock et al. research on gender negotiation dynamics. This plan intentionally excludes demographic-based coaching to avoid model bias and sensitive data collection. The prompts are written to be assertive but collaborative for all users by default. Future research needed before adding demographic-aware branching.

5. **Offer confidentiality**: The spec notes offer details may be confidential. The plan stores all data locally only (no cloud sync in MVP). The privacy mode from `specs/16-privacy-security.md` applies — in `Stealth` mode, company names in TUI views are redacted.

6. **Email draft quality detection**: "Draft sounds AI-generated" is listed as a failure mode in the spec. The counter-offer prompt explicitly instructs the model to avoid telltale phrases ("I hope this email finds you well", "I'm excited", "leverage"). A future phase could run the draft through a local perplexity check (using the same embedding model) to estimate naturalness before showing the user.

7. **Multi-round negotiation tracking**: The spec describes up to 2-3 rounds of back-and-forth. The plan stores multiple `CounterOfferDraft` records per negotiation and supports `RevisedOfferReceived` status for adding a new `OfferRecord` after revision. The link between rounds (draft → revised offer) is implicit via timestamps. A future phase could add an explicit `parent_offer_id` FK to `offers` for round tracking.

8. **Retention counter-offers**: When the user has an outside offer and the current employer counter-offers, this creates a different decision structure. Not modeled in this plan — would require a `RetentionCounterOffer` entity type and a different negotiation prompt context.

## Related Specs
- [specs/salary-market-intelligence.md](salary-market-intelligence.md) — Market data, OfferRecord, compute_total_comp
- [specs/salary-counter-offer-drafting.md](salary-counter-offer-drafting.md) — More detail on the counter-offer drafting workflow
- [specs/XX-multi-offer-comparison.md](XX-multi-offer-comparison.md) — Extended comparison scenarios including scenario modeling
- [specs/application-state-machine.md](application-state-machine.md) — ApplicationStage transitions that trigger NegotiationService
- [specs/16-privacy-security.md](16-privacy-security.md) — Privacy mode enforcement for offer data display
