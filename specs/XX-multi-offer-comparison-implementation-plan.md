# Implementation Plan: Multi-Offer Comparison UI

## Status
Draft

## Related Spec
[specs/XX-multi-offer-comparison.md](XX-multi-offer-comparison.md)

## Overview

The Multi-Offer Comparison module provides a structured, data-driven view for candidates who have received concurrent offers and need to decide under time pressure. It extends the existing `OfferRecord` and `NegotiationRecord` types from the salary-negotiation-offers plan with comparison-specific logic: weighted scoring, year-over-year TC projection, scenario modeling ("what if I negotiate $20K more base?"), and expiry urgency tracking.

The core comparison engine is entirely synchronous and pure — `ComparisonEngine::rank()`, `ComparisonEngine::apply_scenario()`, and `tc_projection_years()` have no I/O, making them exhaustively unit-testable. The async surface is limited to loading offer records from SQLite and dispatching counter-offer Ralph loops from the comparison view. The export path (JSON/CSV) is also sync, using `serde_json` and `csv` crates directly.

This plan deliberately avoids re-defining types already established in `salary-market-intelligence-implementation-plan.md` and `salary-negotiation-offers-implementation-plan.md`. It imports `OfferRecord`, `EquityGrant`, `VestingSchedule`, `BenefitsEstimate`, and `compute_total_comp` from `lazyjob-core/src/salary/` and adds the comparison-specific surface on top.

## Prerequisites

### Must be implemented first
- `specs/salary-market-intelligence-implementation-plan.md` — `OfferRecord`, `EquityGrant`, `VestingSchedule`, `compute_total_comp`, SQLite `offers` table
- `specs/salary-negotiation-offers-implementation-plan.md` — `NegotiationRecord`, `NegotiationPriorities`, `OfferEvaluation`, `rank_offers()`
- `specs/04-sqlite-persistence-implementation-plan.md` — Database connection pool, migration runner
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, layout, `FormWidget`, `TableWidget`
- `specs/08-gaps-salary-tui-implementation-plan.md` — GAP-81 Black-Scholes valuation, GAP-82 offer letter parsing

### Crates to add to Cargo.toml
```toml
# lazyjob-core/Cargo.toml (no new crates — all dependencies already present)
# serde_json, serde, chrono, uuid, thiserror, anyhow — already in workspace

# lazyjob-tui/Cargo.toml
# ratatui, crossterm — already in workspace

# For CSV export (new):
csv = "1.3"
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| Comparison data types | `lazyjob-core` | `src/salary/comparison.rs` |
| TC projection engine | `lazyjob-core` | `src/salary/projection.rs` |
| Scenario modeling | `lazyjob-core` | `src/salary/scenario.rs` |
| Weighted scoring engine | `lazyjob-core` | `src/salary/comparison.rs` |
| Expiry urgency checker | `lazyjob-core` | `src/salary/expiry.rs` |
| Comparison repository | `lazyjob-core` | `src/salary/comparison_repo.rs` |
| Comparison export service | `lazyjob-core` | `src/salary/export.rs` |
| TUI: comparison table view | `lazyjob-tui` | `src/views/salary/offer_comparison.rs` |
| TUI: weighted score panel | `lazyjob-tui` | `src/views/salary/weighted_score.rs` |
| TUI: scenario editor | `lazyjob-tui` | `src/views/salary/scenario_editor.rs` |
| TUI: expiry urgency overlay | `lazyjob-tui` | `src/views/salary/expiry_overlay.rs` |

### Core Types

```rust
// lazyjob-core/src/salary/comparison.rs

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::salary::{OfferRecord, NegotiationPriorities};

/// Newtype for a comparison session ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComparisonId(pub Uuid);

impl ComparisonId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// A persisted comparison session referencing 2+ offers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonSession {
    pub id: ComparisonId,
    /// FK → `offer_evaluations.id` for each participating offer.
    pub offer_ids: Vec<Uuid>,
    /// Serialized as JSON in SQLite.
    pub priorities: ComparisonPriorities,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Priority weights for the comparison. All u8 in [0, 100].
/// The engine normalizes weights to sum to 1.0 before scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonPriorities {
    pub base_salary:      u8,
    pub total_comp:       u8,
    pub equity:           u8,
    pub signing_bonus:    u8,
    pub remote_policy:    u8,
    pub pto_and_benefits: u8,
    pub growth_stage:     u8,
    pub mission_fit:      u8,  // 0 = not weighted; user-entered 1-100
}

impl Default for ComparisonPriorities {
    fn default() -> Self {
        Self {
            base_salary:      60,
            total_comp:       20,
            equity:           10,
            signing_bonus:     5,
            remote_policy:     5,
            pto_and_benefits:  0,
            growth_stage:      0,
            mission_fit:       0,
        }
    }
}

/// A fully resolved snapshot of one offer ready for side-by-side display.
/// All monetary values are i64 cents (annual). Computed from `OfferRecord`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparableOffer {
    pub offer_id:           Uuid,
    pub application_id:     Uuid,
    pub company_name:       String,
    pub role_title:         String,

    // Compensation breakdown (cents/year)
    pub base_salary_cents:     i64,
    pub annual_bonus_cents:    i64,  // Expected Year 1
    pub signing_bonus_cents:   i64,  // Full signing, NOT annualized
    pub equity_annual_cents:   i64,  // Total grant / 4 (linear RSU) or BS value
    pub benefits_annual_cents: i64,  // Employer health + 401k match estimate

    // Derived totals
    pub total_cash_year1_cents:   i64,  // base + bonus + signing
    pub total_comp_year1_cents:   i64,  // cash + equity_annual + benefits

    // Equity details
    pub equity_shares:       u64,
    pub equity_grant_type:   String,   // "RSU" | "ISO" | "NSO"
    pub vest_years:          u8,
    pub vest_cliff_months:   u8,
    pub current_price_cents: Option<i64>,
    pub strike_price_cents:  Option<i64>,

    // Logistics
    pub remote_policy:       RemotePolicy,
    pub start_date:          Option<NaiveDate>,
    pub expiry_date:         Option<NaiveDate>,

    // Scoring
    pub weighted_score:      f32,   // Computed by ComparisonEngine::score()
    pub rank:                usize, // 1 = best
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemotePolicy {
    FullyRemote,
    Hybrid { days_in_office: u8 },
    OnSite,
    Flexible,
    Unknown,
}

impl RemotePolicy {
    /// Score in [0, 100] for weighting purposes.
    /// Higher = more remote-friendly.
    pub fn remote_score(&self) -> u8 {
        match self {
            RemotePolicy::FullyRemote      => 100,
            RemotePolicy::Flexible         => 80,
            RemotePolicy::Hybrid { days_in_office } => {
                // 1 day in office → 80, 5 days → 0
                100u8.saturating_sub(days_in_office * 16)
            }
            RemotePolicy::OnSite           => 0,
            RemotePolicy::Unknown          => 50,
        }
    }
}

/// Result of running the comparison engine over a set of offers.
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    pub offers:      Vec<ComparableOffer>,  // sorted by rank ascending
    pub top_pick_id: Uuid,
    pub priorities:  ComparisonPriorities,
}
```

```rust
// lazyjob-core/src/salary/projection.rs

/// Year-over-year TC projection for one offer.
/// Year 1 includes signing bonus; Years 2+ exclude it.
/// Equity is modeled linearly (vested tranche / vest_years per year).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcProjection {
    pub offer_id: Uuid,
    /// Length = years_to_project. Index 0 = Year 1.
    pub yearly_tc_cents: Vec<i64>,
}

/// Pure function. No I/O. Panics only on years_to_project == 0.
pub fn project_tc(offer: &ComparableOffer, years_to_project: u8) -> TcProjection {
    assert!(years_to_project > 0);
    let mut yearly = Vec::with_capacity(years_to_project as usize);
    for year in 1..=years_to_project {
        let equity = if year <= offer.vest_years as u8 {
            // Linear vest: each year receives 1/vest_years of total grant value.
            // Cliff: Year 1 gets normal share if year >= cliff (months converted).
            let cliff_years = (offer.vest_cliff_months as f32 / 12.0).ceil() as u8;
            if year < cliff_years {
                0
            } else {
                offer.equity_annual_cents
            }
        } else {
            0
        };
        let signing = if year == 1 { offer.signing_bonus_cents } else { 0 };
        let tc = offer.base_salary_cents
            + offer.annual_bonus_cents
            + signing
            + equity
            + offer.benefits_annual_cents;
        yearly.push(tc);
    }
    TcProjection { offer_id: offer.offer_id, yearly_tc_cents: yearly }
}
```

```rust
// lazyjob-core/src/salary/scenario.rs

/// What-if scenario: "if Stripe matches X more in base, does it beat Meta?"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationScenario {
    pub offer_id:      Uuid,
    pub delta_type:    ScenarioDeltaType,
    pub delta_cents:   i64,   // Positive = increase
    pub label:         String, // Human-readable: "Stripe +$20K base"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScenarioDeltaType {
    BaseSalary,
    SigningBonus,
    EquityGrantTotal,  // Total grant value delta; annualized automatically
    AnnualBonus,
}

/// Applies a scenario delta to a ComparableOffer, returning a modified clone.
/// Pure function — does not touch SQLite.
pub fn apply_scenario(
    offer: &ComparableOffer,
    scenario: &NegotiationScenario,
) -> ComparableOffer {
    let mut modified = offer.clone();
    match scenario.delta_type {
        ScenarioDeltaType::BaseSalary => {
            modified.base_salary_cents += scenario.delta_cents;
        }
        ScenarioDeltaType::SigningBonus => {
            modified.signing_bonus_cents += scenario.delta_cents;
        }
        ScenarioDeltaType::EquityGrantTotal => {
            // Annualized: delta / vest_years
            let annual_delta = scenario.delta_cents
                / offer.vest_years.max(1) as i64;
            modified.equity_annual_cents += annual_delta;
        }
        ScenarioDeltaType::AnnualBonus => {
            modified.annual_bonus_cents += scenario.delta_cents;
        }
    }
    // Recompute derived totals.
    modified.total_cash_year1_cents = modified.base_salary_cents
        + modified.annual_bonus_cents
        + modified.signing_bonus_cents;
    modified.total_comp_year1_cents = modified.total_cash_year1_cents
        + modified.equity_annual_cents
        + modified.benefits_annual_cents;
    modified
}
```

```rust
// lazyjob-core/src/salary/expiry.rs

use chrono::{NaiveDate, Local};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpiryUrgency {
    /// Expires in ≤ 2 calendar days.
    Critical { days_remaining: i64 },
    /// Expires in 3–5 calendar days.
    Warning  { days_remaining: i64 },
    /// Expires in 6–14 calendar days.
    Upcoming { days_remaining: i64 },
    /// Expires in > 14 days or no expiry set.
    Comfortable,
}

/// Pure function. Returns urgency for each offer keyed by offer_id.
/// `today` injected for testability.
pub fn classify_expiry(
    expiry_date: Option<NaiveDate>,
    today: NaiveDate,
) -> ExpiryUrgency {
    let Some(exp) = expiry_date else {
        return ExpiryUrgency::Comfortable;
    };
    let days = (exp - today).num_days();
    match days {
        d if d <= 2  => ExpiryUrgency::Critical { days_remaining: d },
        d if d <= 5  => ExpiryUrgency::Warning  { days_remaining: d },
        d if d <= 14 => ExpiryUrgency::Upcoming { days_remaining: d },
        _            => ExpiryUrgency::Comfortable,
    }
}

impl ExpiryUrgency {
    /// Returns true if this urgency should trigger a startup banner.
    pub fn is_urgent(&self) -> bool {
        matches!(self, Self::Critical { .. } | Self::Warning { .. })
    }
}
```

### Comparison Engine

```rust
// lazyjob-core/src/salary/comparison.rs (continued)

/// The comparison engine. Stateless — all pure functions.
pub struct ComparisonEngine;

impl ComparisonEngine {
    /// Resolve raw OfferRecords into ComparableOffers.
    /// Calls `compute_total_comp()` from the market intelligence module.
    pub fn resolve(
        records: &[OfferRecord],
        priorities: &ComparisonPriorities,
    ) -> ComparisonResult {
        let mut offers: Vec<ComparableOffer> = records
            .iter()
            .map(|r| Self::resolve_one(r))
            .collect();

        // Compute weighted scores.
        let min_max = MinMaxTable::build(&offers);
        for offer in &mut offers {
            offer.weighted_score = Self::score(offer, priorities, &min_max);
        }

        // Sort descending by weighted_score, then by total_comp for ties.
        offers.sort_by(|a, b| {
            b.weighted_score
                .partial_cmp(&a.weighted_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.total_comp_year1_cents.cmp(&a.total_comp_year1_cents))
        });

        let top_pick_id = offers[0].offer_id;
        for (i, o) in offers.iter_mut().enumerate() {
            o.rank = i + 1;
        }

        ComparisonResult { offers, top_pick_id, priorities: priorities.clone() }
    }

    /// Compute a weighted score in [0.0, 1.0] for one offer.
    /// Each component is normalized to [0, 1] against the min/max across all offers.
    fn score(
        offer: &ComparableOffer,
        p: &ComparisonPriorities,
        mm: &MinMaxTable,
    ) -> f32 {
        let total_weight = p.base_salary as f32
            + p.total_comp as f32
            + p.equity as f32
            + p.signing_bonus as f32
            + p.remote_policy as f32
            + p.pto_and_benefits as f32
            + p.growth_stage as f32
            + p.mission_fit as f32;

        if total_weight == 0.0 {
            return 0.0;
        }

        let norm = |val: i64, field: Field| -> f32 {
            let (min, max) = mm.range(field);
            if max == min { return 1.0; } // all offers equal on this dimension
            (val - min) as f32 / (max - min) as f32
        };

        let score =
            norm(offer.base_salary_cents, Field::Base)    * p.base_salary as f32 +
            norm(offer.total_comp_year1_cents, Field::Tc) * p.total_comp as f32 +
            norm(offer.equity_annual_cents, Field::Equity) * p.equity as f32 +
            norm(offer.signing_bonus_cents, Field::Signing) * p.signing_bonus as f32 +
            (offer.remote_policy.remote_score() as f32 / 100.0) * p.remote_policy as f32 +
            norm(offer.benefits_annual_cents, Field::Benefits) * p.pto_and_benefits as f32;
        // growth_stage and mission_fit are user-entered integers (0-100), not derived.
        // They will be added in Phase 3 when the UI allows direct entry.

        score / total_weight
    }

    fn resolve_one(r: &OfferRecord) -> ComparableOffer {
        // Delegates to compute_total_comp from market intelligence module.
        // Equity annualized as total_grant_cents / vest_years (linear).
        // Black-Scholes options valuation via bs_option_value() from GAP-81.
        todo!("implemented in Phase 1 Step 3")
    }
}

/// Min/max values across all offers, used for normalization.
struct MinMaxTable {
    base:     (i64, i64),
    tc:       (i64, i64),
    equity:   (i64, i64),
    signing:  (i64, i64),
    benefits: (i64, i64),
}

enum Field { Base, Tc, Equity, Signing, Benefits }

impl MinMaxTable {
    fn build(offers: &[ComparableOffer]) -> Self {
        let min_max = |f: fn(&ComparableOffer) -> i64| -> (i64, i64) {
            let vals: Vec<i64> = offers.iter().map(f).collect();
            (*vals.iter().min().unwrap(), *vals.iter().max().unwrap())
        };
        Self {
            base:     min_max(|o| o.base_salary_cents),
            tc:       min_max(|o| o.total_comp_year1_cents),
            equity:   min_max(|o| o.equity_annual_cents),
            signing:  min_max(|o| o.signing_bonus_cents),
            benefits: min_max(|o| o.benefits_annual_cents),
        }
    }

    fn range(&self, f: Field) -> (i64, i64) {
        match f {
            Field::Base     => self.base,
            Field::Tc       => self.tc,
            Field::Equity   => self.equity,
            Field::Signing  => self.signing,
            Field::Benefits => self.benefits,
        }
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/salary/comparison_repo.rs

#[async_trait::async_trait]
pub trait ComparisonSessionRepository: Send + Sync {
    async fn create(&self, session: &ComparisonSession) -> Result<(), ComparisonError>;
    async fn get(&self, id: &ComparisonId) -> Result<Option<ComparisonSession>, ComparisonError>;
    async fn update_priorities(
        &self,
        id: &ComparisonId,
        priorities: &ComparisonPriorities,
    ) -> Result<(), ComparisonError>;
    async fn list_active(&self) -> Result<Vec<ComparisonSession>, ComparisonError>;
    async fn delete(&self, id: &ComparisonId) -> Result<(), ComparisonError>;
}

/// Reads OfferRecord + OfferEvaluation rows needed to build ComparableOffer.
#[async_trait::async_trait]
pub trait OfferSnapshotReader: Send + Sync {
    async fn load_for_comparison(
        &self,
        offer_ids: &[Uuid],
    ) -> Result<Vec<OfferRecord>, ComparisonError>;
}
```

### SQLite Schema

```sql
-- Migration: 019_comparison_sessions.sql

CREATE TABLE IF NOT EXISTS offer_comparison_sessions (
    id          TEXT    NOT NULL PRIMARY KEY,       -- ComparisonId UUID
    offer_ids   TEXT    NOT NULL,                   -- JSON array of offer_evaluation UUIDs
    priorities  TEXT    NOT NULL,                   -- JSON ComparisonPriorities
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Scenarios saved by the user for one session.
CREATE TABLE IF NOT EXISTS offer_comparison_scenarios (
    id          TEXT    NOT NULL PRIMARY KEY,       -- UUID
    session_id  TEXT    NOT NULL REFERENCES offer_comparison_sessions(id) ON DELETE CASCADE,
    offer_id    TEXT    NOT NULL,                   -- which offer this scenario modifies
    delta_type  TEXT    NOT NULL,                   -- "base_salary" | "signing_bonus" | ...
    delta_cents INTEGER NOT NULL,
    label       TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_scenarios_session
    ON offer_comparison_scenarios(session_id);

-- Trigger to keep updated_at current.
CREATE TRIGGER IF NOT EXISTS trg_comparison_sessions_updated
    AFTER UPDATE ON offer_comparison_sessions
    FOR EACH ROW
BEGIN
    UPDATE offer_comparison_sessions
    SET updated_at = datetime('now')
    WHERE id = NEW.id;
END;
```

### Module Structure

```
lazyjob-core/
  src/
    salary/
      mod.rs              # Re-exports: pub use comparison::*, projection::*, ...
      comparison.rs       # ComparisonSession, ComparableOffer, ComparisonEngine, MinMaxTable
      projection.rs       # TcProjection, project_tc()
      scenario.rs         # NegotiationScenario, apply_scenario()
      expiry.rs           # ExpiryUrgency, classify_expiry()
      comparison_repo.rs  # ComparisonSessionRepository trait, SqliteComparisonRepo
      export.rs           # CsvExporter, JsonExporter

lazyjob-tui/
  src/
    views/
      salary/
        offer_comparison.rs    # ComparisonTableView (main 3-column table)
        weighted_score.rs      # WeightedScorePanel (sliders + ranked scores)
        scenario_editor.rs     # ScenarioEditorWidget (delta entry + live re-rank)
        expiry_overlay.rs      # ExpiryUrgencyBanner (startup modal + inline badge)
        projection_chart.rs    # TcProjectionChart (multi-year bar chart)
```

## Implementation Phases

### Phase 1 — Core Domain (MVP)

**Step 1 — SQLite migration**

File: `lazyjob-core/migrations/019_comparison_sessions.sql`

Apply the schema above. Verify:
```bash
sqlite3 ~/.local/share/lazyjob/lazyjob.db ".schema offer_comparison_sessions"
```

**Step 2 — `RemotePolicy`, `ExpiryUrgency`, `ComparisonPriorities` types**

File: `lazyjob-core/src/salary/comparison.rs`

Implement `RemotePolicy::remote_score()`, `ExpiryUrgency::is_urgent()`, and `ComparisonPriorities::default()` as shown in Core Types. Add `#[derive(Serialize, Deserialize)]` to all types.

Unit tests:
```rust
#[test]
fn remote_score_hybrid_3_days() {
    assert_eq!(RemotePolicy::Hybrid { days_in_office: 3 }.remote_score(), 52);
}

#[test]
fn expiry_critical() {
    let today = NaiveDate::from_ymd_opt(2026, 4, 16).unwrap();
    let exp   = NaiveDate::from_ymd_opt(2026, 4, 17).unwrap();
    assert!(matches!(
        classify_expiry(Some(exp), today),
        ExpiryUrgency::Critical { days_remaining: 1 }
    ));
}
```

**Step 3 — `ComparableOffer` resolver**

File: `lazyjob-core/src/salary/comparison.rs`

Implement `ComparisonEngine::resolve_one(r: &OfferRecord) -> ComparableOffer`:
- `base_salary_cents`: from `r.base_salary_cents`
- `equity_annual_cents`:
  - RSU: `r.equity_grant.total_grant_cents / r.equity_grant.vest_years as i64`
  - Options: call `bs_option_value()` from `lazyjob-core/src/salary/black_scholes.rs` (from GAP-81), then annualize: `bsv * r.equity_grant.shares / r.equity_grant.vest_years as i64`
- `benefits_annual_cents`: sum of `r.benefits.health_insurance_annual_cents + r.benefits.employer_401k_match_cents`
- `total_cash_year1_cents`: `base + annual_bonus + signing_bonus`
- `total_comp_year1_cents`: `total_cash + equity_annual + benefits`

Key APIs:
- `OfferRecord::base_salary_cents` — `i64`
- `EquityGrant::total_grant_cents` — `i64`
- `bs_option_value(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64` — from black_scholes module

Unit test:
```rust
#[test]
fn resolve_rsu_offer_no_options() {
    let offer = fixture_rsu_offer(base_cents: 18_500_000, shares: 5000, price_cents: 30000, vest_years: 4);
    let co = ComparisonEngine::resolve_one(&offer);
    assert_eq!(co.equity_annual_cents, 37_500_000); // 5000 * $300 / 4
    assert_eq!(co.total_comp_year1_cents, co.base_salary_cents + co.annual_bonus_cents
        + co.signing_bonus_cents + co.equity_annual_cents + co.benefits_annual_cents);
}
```

**Step 4 — `MinMaxTable` and weighted scoring**

File: `lazyjob-core/src/salary/comparison.rs`

Implement `MinMaxTable::build()`, `ComparisonEngine::score()`, and `ComparisonEngine::resolve()` as shown in Core Types.

Unit test:
```rust
#[test]
fn scoring_highest_base_wins_when_only_weight() {
    let mut p = ComparisonPriorities::default();
    // Only weight base_salary
    p.total_comp = 0; p.equity = 0; p.signing_bonus = 0;
    p.remote_policy = 0; p.pto_and_benefits = 0; p.growth_stage = 0; p.mission_fit = 0;
    p.base_salary = 100;

    let a = fixture_offer(base: 200_000_00, ..Default::default());
    let b = fixture_offer(base: 180_000_00, ..Default::default());
    let result = ComparisonEngine::resolve(&[a, b], &p);
    assert_eq!(result.top_pick_id, a.offer_id);
}
```

**Step 5 — `SqliteComparisonRepo`**

File: `lazyjob-core/src/salary/comparison_repo.rs`

```rust
pub struct SqliteComparisonRepo {
    pool: sqlx::SqlitePool,
}

impl SqliteComparisonRepo {
    pub fn new(pool: sqlx::SqlitePool) -> Self { Self { pool } }
}

#[async_trait::async_trait]
impl ComparisonSessionRepository for SqliteComparisonRepo {
    async fn create(&self, session: &ComparisonSession) -> Result<(), ComparisonError> {
        let offer_ids_json = serde_json::to_string(&session.offer_ids)?;
        let priorities_json = serde_json::to_string(&session.priorities)?;
        sqlx::query!(
            "INSERT INTO offer_comparison_sessions (id, offer_ids, priorities)
             VALUES (?, ?, ?)",
            session.id.0, offer_ids_json, priorities_json
        )
        .execute(&self.pool)
        .await
        .map_err(ComparisonError::Database)?;
        Ok(())
    }

    async fn update_priorities(
        &self,
        id: &ComparisonId,
        priorities: &ComparisonPriorities,
    ) -> Result<(), ComparisonError> {
        let json = serde_json::to_string(priorities)?;
        sqlx::query!(
            "UPDATE offer_comparison_sessions SET priorities = ? WHERE id = ?",
            json, id.0
        )
        .execute(&self.pool)
        .await
        .map_err(ComparisonError::Database)?;
        Ok(())
    }

    // list_active, get, delete follow same pattern.
}
```

**Step 6 — `TcProjection` and `project_tc()`**

File: `lazyjob-core/src/salary/projection.rs`

Implement `project_tc()` as shown in Core Types. Add cliff-year logic.

Unit test:
```rust
#[test]
fn cliff_year_gives_zero_equity_before_cliff() {
    let offer = fixture_offer_with_cliff(cliff_months: 12, vest_years: 4, equity_annual_cents: 40_000_00);
    let proj = project_tc(&offer, 4);
    // Year 1: base + bonus (no equity before cliff; cliff at 12 months is exactly end of year 1)
    // Spec: cliff_years = ceil(12/12) = 1; year 1 < cliff_years(1) is false → gets equity
    // If cliff is 18 months: cliff_years = ceil(18/12) = 2; year 1 < 2 → no equity
    let offer_18mo = fixture_offer_with_cliff(cliff_months: 18, vest_years: 4, equity_annual_cents: 40_000_00);
    let proj_18 = project_tc(&offer_18mo, 4);
    assert_eq!(proj_18.yearly_tc_cents[0],
        offer_18mo.base_salary_cents + offer_18mo.annual_bonus_cents
        + offer_18mo.signing_bonus_cents + offer_18mo.benefits_annual_cents);
}
```

### Phase 2 — Scenario Modeling

**Step 1 — `NegotiationScenario` and `apply_scenario()`**

File: `lazyjob-core/src/salary/scenario.rs`

Implement as shown. Key rule: `ScenarioDeltaType::EquityGrantTotal` annualizes by `delta_cents / vest_years.max(1)` — avoids divide-by-zero.

Unit tests:
```rust
#[test]
fn scenario_base_delta_updates_total_comp() {
    let offer = fixture_comparable_offer(base: 18_000_000, tc: 25_000_000);
    let scenario = NegotiationScenario {
        offer_id: offer.offer_id,
        delta_type: ScenarioDeltaType::BaseSalary,
        delta_cents: 2_000_000,  // +$20K
        label: "Stripe +$20K base".into(),
    };
    let modified = apply_scenario(&offer, &scenario);
    assert_eq!(modified.base_salary_cents, 20_000_000);
    assert_eq!(modified.total_comp_year1_cents, 27_000_000);
}

#[test]
fn scenario_equity_delta_is_annualized() {
    let offer = fixture_comparable_offer_with_vest_years(vest_years: 4);
    let scenario = NegotiationScenario {
        delta_type: ScenarioDeltaType::EquityGrantTotal,
        delta_cents: 40_000_000, // +$400K total grant
        ..
    };
    let modified = apply_scenario(&offer, &scenario);
    assert_eq!(
        modified.equity_annual_cents,
        offer.equity_annual_cents + 10_000_000 // +$400K / 4
    );
}
```

**Step 2 — Scenario persistence**

File: `lazyjob-core/src/salary/comparison_repo.rs`

Add `save_scenario()` and `list_scenarios()` to `SqliteComparisonRepo`:

```rust
async fn save_scenario(
    &self,
    session_id: &ComparisonId,
    scenario: &NegotiationScenario,
) -> Result<(), ComparisonError> {
    let id = Uuid::new_v4().to_string();
    let delta_type = serde_json::to_string(&scenario.delta_type)?;
    sqlx::query!(
        "INSERT INTO offer_comparison_scenarios
         (id, session_id, offer_id, delta_type, delta_cents, label)
         VALUES (?, ?, ?, ?, ?, ?)",
        id, session_id.0, scenario.offer_id,
        delta_type, scenario.delta_cents, scenario.label
    )
    .execute(&self.pool)
    .await
    .map_err(ComparisonError::Database)?;
    Ok(())
}
```

**Step 3 — `ComparisonService` orchestrator**

File: `lazyjob-core/src/salary/comparison_service.rs`

```rust
pub struct ComparisonService {
    repo:         Arc<dyn ComparisonSessionRepository>,
    offer_reader: Arc<dyn OfferSnapshotReader>,
}

impl ComparisonService {
    /// Load or create a comparison session for the given offer IDs.
    pub async fn get_or_create(
        &self,
        offer_ids: Vec<Uuid>,
    ) -> Result<(ComparisonSession, ComparisonResult), ComparisonError> {
        // Look for an active session with the same offer set.
        let sessions = self.repo.list_active().await?;
        let existing = sessions.into_iter().find(|s| {
            let mut a = s.offer_ids.clone(); a.sort();
            let mut b = offer_ids.clone(); b.sort();
            a == b
        });
        let session = match existing {
            Some(s) => s,
            None => {
                let s = ComparisonSession {
                    id: ComparisonId::new(),
                    offer_ids: offer_ids.clone(),
                    priorities: ComparisonPriorities::default(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                self.repo.create(&s).await?;
                s
            }
        };
        let records = self.offer_reader.load_for_comparison(&offer_ids).await?;
        let result = ComparisonEngine::resolve(&records, &session.priorities);
        Ok((session, result))
    }

    /// Update priorities and recompute rankings.
    pub async fn update_priorities(
        &self,
        session_id: &ComparisonId,
        priorities: ComparisonPriorities,
        offer_ids: &[Uuid],
    ) -> Result<ComparisonResult, ComparisonError> {
        self.repo.update_priorities(session_id, &priorities).await?;
        let records = self.offer_reader.load_for_comparison(offer_ids).await?;
        Ok(ComparisonEngine::resolve(&records, &priorities))
    }

    /// Apply a what-if scenario and return a re-ranked result (does NOT persist automatically).
    pub fn preview_scenario(
        &self,
        result: &ComparisonResult,
        scenario: &NegotiationScenario,
    ) -> ComparisonResult {
        let mut modified_offers: Vec<ComparableOffer> = result.offers
            .iter()
            .map(|o| {
                if o.offer_id == scenario.offer_id {
                    apply_scenario(o, scenario)
                } else {
                    o.clone()
                }
            })
            .collect();
        // Re-score with same priorities.
        let mm = MinMaxTable::build(&modified_offers);
        for offer in &mut modified_offers {
            offer.weighted_score = ComparisonEngine::score(offer, &result.priorities, &mm);
        }
        modified_offers.sort_by(|a, b| {
            b.weighted_score.partial_cmp(&a.weighted_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top_pick_id = modified_offers[0].offer_id;
        for (i, o) in modified_offers.iter_mut().enumerate() { o.rank = i + 1; }
        ComparisonResult {
            offers: modified_offers,
            top_pick_id,
            priorities: result.priorities.clone(),
        }
    }
}
```

### Phase 3 — Export

**Step 1 — `CsvExporter`**

File: `lazyjob-core/src/salary/export.rs`

```rust
use csv::Writer;
use std::io::Write;

pub struct CsvExporter;

impl CsvExporter {
    /// Writes comparison data to any `Write` impl (file, Vec<u8>, etc.).
    pub fn export<W: Write>(
        result: &ComparisonResult,
        writer: W,
    ) -> Result<(), ComparisonError> {
        let mut wtr = Writer::from_writer(writer);
        wtr.write_record(&[
            "Company", "Role", "Base ($)", "Annual Bonus ($)", "Signing ($)",
            "Equity Annual ($)", "Benefits Annual ($)", "Total Cash Year 1 ($)",
            "Total Comp Year 1 ($)", "Weighted Score", "Rank",
            "Remote Policy", "Start Date", "Expiry Date",
        ])?;
        for o in &result.offers {
            wtr.write_record(&[
                &o.company_name,
                &o.role_title,
                &format!("{:.2}", o.base_salary_cents as f64 / 100.0),
                &format!("{:.2}", o.annual_bonus_cents as f64 / 100.0),
                &format!("{:.2}", o.signing_bonus_cents as f64 / 100.0),
                &format!("{:.2}", o.equity_annual_cents as f64 / 100.0),
                &format!("{:.2}", o.benefits_annual_cents as f64 / 100.0),
                &format!("{:.2}", o.total_cash_year1_cents as f64 / 100.0),
                &format!("{:.2}", o.total_comp_year1_cents as f64 / 100.0),
                &format!("{:.4}", o.weighted_score),
                &o.rank.to_string(),
                &format!("{:?}", o.remote_policy),
                &o.start_date.map(|d| d.to_string()).unwrap_or_default(),
                &o.expiry_date.map(|d| d.to_string()).unwrap_or_default(),
            ])?;
        }
        wtr.flush()?;
        Ok(())
    }
}

pub struct JsonExporter;

impl JsonExporter {
    pub fn export(result: &ComparisonResult) -> Result<String, ComparisonError> {
        Ok(serde_json::to_string_pretty(result)?)
    }
}
```

**Step 2 — Multi-year projection export**

Extend `CsvExporter::export_projection()`:

```rust
pub fn export_projection<W: Write>(
    projections: &[TcProjection],
    offers: &[ComparableOffer],
    years: u8,
    writer: W,
) -> Result<(), ComparisonError> {
    let mut wtr = Writer::from_writer(writer);
    // Header: Company, Year 1, Year 2, ..., Year N
    let mut header = vec!["Company".to_string()];
    for y in 1..=years { header.push(format!("Year {} ($)", y)); }
    wtr.write_record(&header)?;
    for proj in projections {
        let company = offers.iter()
            .find(|o| o.offer_id == proj.offer_id)
            .map(|o| o.company_name.as_str())
            .unwrap_or("Unknown");
        let mut row = vec![company.to_string()];
        for cents in &proj.yearly_tc_cents {
            row.push(format!("{:.2}", *cents as f64 / 100.0));
        }
        wtr.write_record(&row)?;
    }
    wtr.flush()?;
    Ok(())
}
```

### Phase 4 — TUI

**Step 1 — `ComparisonTableView`**

File: `lazyjob-tui/src/views/salary/offer_comparison.rs`

Layout: full-width `ratatui::widgets::Table` with one column per offer (dynamic width, max 5 offers per viewport, horizontal scroll if > 5).

```rust
pub struct ComparisonTableView {
    pub result:       ComparisonResult,
    pub expiry_urgencies: Vec<(Uuid, ExpiryUrgency)>,
    pub selected_row: usize,
    pub sort_by:      SortBy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortBy { TotalComp, BaseSalary, WeightedScore, Expiry }

pub enum ComparisonAction {
    SortBy(SortBy),
    SelectOffer(Uuid),
    OpenScenarioEditor,
    OpenWeightedScorePanel,
    ExportCsv,
    ExportJson,
    NegotiateOffer(Uuid),
    Close,
}
```

Rendering logic:
- Each row is a metric (Base Salary, Annual Bonus, etc.).
- Column 0 = metric name (fixed 24 chars wide).
- Columns 1..N = values for each offer, formatted with `format_cents_k(cents: i64)` helper.
- `TOTAL COMP YEAR 1` row rendered in bold green.
- Expiry badges: `[⚠ 3d]` span in red for Critical, yellow for Warning, appended to the company name header.
- `[n]` keybind legend at bottom: `[s]ort [w]eights [e]xport [Enter]select`.

Key APIs:
- `ratatui::widgets::Table::new(rows, widths)` — column widths via `ratatui::layout::Constraint::Min(24)`
- `ratatui::widgets::Row::new(cells)` — each metric row
- `ratatui::style::Style::default().add_modifier(Modifier::BOLD)` — for totals row
- `ratatui::text::Span::styled(text, style)` — for colored urgency badges

**Step 2 — `WeightedScorePanel`**

File: `lazyjob-tui/src/views/salary/weighted_score.rs`

A floating overlay (uses `ratatui::widgets::Clear` to erase background, then renders a bordered block).

```rust
pub struct WeightedScorePanel {
    pub priorities:    ComparisonPriorities,
    pub scores:        Vec<(String, f32)>,  // (company, weighted_score)
    pub selected_field: WeightField,
    pub edit_mode:     bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightField {
    BaseSalary, TotalComp, Equity, SigningBonus,
    RemotePolicy, PtoAndBenefits, GrowthStage, MissionFit,
}
```

Rendering:
- Each priority is rendered as a label + `Gauge` widget showing the weight percentage.
- `[↑↓]` to select field, `[+/-]` to adjust by 5, `[r]eset` to restore defaults.
- Live re-rank: each keystroke calls `ComparisonEngine::resolve()` in-place (sync, instant).
- Bottom section: ranked list of offers with their weighted scores displayed as `$xxx,xxx (score: 0.82)`.

Key APIs:
- `ratatui::widgets::Gauge::default().percent(p)` — for weight slider
- `ratatui::layout::Layout::vertical([Constraint::Length(2); N])` — one row per priority

**Step 3 — `ScenarioEditorWidget`**

File: `lazyjob-tui/src/views/salary/scenario_editor.rs`

A side panel (40% width, right side) showing:
1. Drop-down to select which offer to modify.
2. Drop-down to select delta type (Base/Signing/Equity/Bonus).
3. Text input for delta amount (parses `$20k` → 2_000_000 cents, `20000` → 2_000_000 cents, `20` → 2_000 cents if < 10000 raw).
4. Live preview: re-renders the comparison table with the scenario applied but NOT persisted.
5. `[Enter]` = save scenario, `[Esc]` = discard.

Dollar amount parsing:
```rust
fn parse_dollar_input(s: &str) -> Option<i64> {
    let s = s.trim().trim_start_matches('$').replace(',', "");
    if let Some(k_str) = s.strip_suffix('k').or_else(|| s.strip_suffix('K')) {
        let k: f64 = k_str.parse().ok()?;
        Some((k * 100_000.0) as i64) // k * 1000 * 100 cents
    } else {
        let v: f64 = s.parse().ok()?;
        // Heuristic: if < 10000, treat as thousands
        if v < 10_000.0 {
            Some((v * 1_000.0 * 100.0) as i64)
        } else {
            Some((v * 100.0) as i64)
        }
    }
}
```

**Step 4 — `ExpiryUrgencyBanner`**

File: `lazyjob-tui/src/views/salary/expiry_overlay.rs`

Shown on app startup and when switching to the comparison view if any offer has `ExpiryUrgency::Critical` or `Warning`.

```rust
pub struct ExpiryUrgencyBanner {
    pub urgent_offers: Vec<(String, ExpiryUrgency)>,  // (company, urgency)
    pub dismissed:     bool,
}

pub enum ExpiryBannerAction {
    NegotiateExtension(Uuid),
    DeclineAndMove(Uuid),
    Dismiss,
}
```

Rendering: `ratatui::widgets::Clear` + `ratatui::widgets::Block` with red border for Critical, yellow for Warning. Lists each urgent offer with days remaining and keybind options.

Startup check in `App::on_start()`:
```rust
// lazyjob-tui/src/app.rs
async fn check_offer_expiry(&mut self) {
    let today = chrono::Local::now().date_naive();
    let urgent: Vec<_> = self.comparison_result.as_ref()
        .map(|r| r.offers.iter()
            .filter_map(|o| {
                let urgency = classify_expiry(o.expiry_date, today);
                if urgency.is_urgent() {
                    Some((o.company_name.clone(), urgency))
                } else {
                    None
                }
            })
            .collect()
        )
        .unwrap_or_default();
    if !urgent.is_empty() {
        self.expiry_banner = Some(ExpiryUrgencyBanner { urgent_offers: urgent, dismissed: false });
    }
}
```

**Step 5 — `TcProjectionChart`**

File: `lazyjob-tui/src/views/salary/projection_chart.rs`

A `ratatui::widgets::BarChart` rendering Year 1–4 grouped bars (one group per year, one bar per offer).

```rust
pub struct TcProjectionChart {
    pub projections: Vec<TcProjection>,
    pub offer_names: Vec<(Uuid, String)>, // For bar labels
    pub years: u8,
}
```

Rendering:
- `ratatui::widgets::BarChart` with `bar_width(4)`, one `BarGroup` per year.
- Each `Bar::default().value(cents / 100_000)` (in $100K units to fit the axis).
- Legend row beneath: one colored square per offer.

Key APIs:
- `ratatui::widgets::BarChart::default().data(groups).bar_width(4).group_gap(2)`
- `ratatui::widgets::BarGroup::default().bars(&bars).label(year_label)`

### Phase 5 — Polish and Extension

**Step 1 — Multi-year comparison in the main table**

Add a `[y]ear` toggle to the `ComparisonTableView` that switches between Year 1 (default) and a Year 1–4 scrollable panel using `TcProjectionChart`.

**Step 2 — Mission fit and growth stage scores**

Add two additional priority fields to `ComparisonPriorities` that accept user-entered integer scores (0–100) per offer, stored in `offer_comparison_scenarios` as `delta_type = "mission_fit"` with `delta_cents = score`. These are included in the `ComparisonEngine::score()` normalizer.

**Step 3 — Expiry extension negotiation link**

`ExpiryBannerAction::NegotiateExtension(offer_id)` opens the `CounterOfferLoop` with `delta_type: SigningBonus, delta_cents: 0` and a pre-filled note "Requesting a one-week deadline extension" — reusing the counter-offer drafting infrastructure.

**Step 4 — `lazyjob offers compare` CLI subcommand**

```
lazyjob offers compare --offer-ids <uuid,uuid,...> [--export csv|json] [--years 4]
```

Loads offers, runs `ComparisonEngine::resolve()` with default priorities, and prints the comparison table to stdout using `comfy-table` crate (already used for CLI output in other subcommands).

## Key Crate APIs

- `sqlx::query!()` macro — compile-time SQL verification for all DB ops
- `sqlx::SqlitePool` — connection pool passed as `Arc<SqlitePool>` to repositories
- `serde_json::to_string_pretty(&value)` — JSON export
- `csv::Writer::from_writer(w: W)` — CSV export without heap allocation for the writer
- `ratatui::widgets::Table::new(rows, widths)` — comparison table rendering
- `ratatui::widgets::BarChart::default().data(groups)` — projection chart
- `ratatui::widgets::Gauge::default().percent(p)` — priority sliders
- `ratatui::widgets::Clear` — clear background before overlays
- `ratatui::style::Modifier::BOLD` — total row emphasis
- `chrono::NaiveDate::signed_duration_since()` — expiry day calculation
- `strsim::jaro_winkler()` — NOT used in this module (comparison is by ID, not fuzzy name)

## Error Handling

```rust
// lazyjob-core/src/salary/comparison.rs

#[derive(thiserror::Error, Debug)]
pub enum ComparisonError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("CSV export error: {0}")]
    Csv(#[from] csv::Error),

    #[error("offer not found: {0}")]
    OfferNotFound(Uuid),

    #[error("comparison requires at least 2 offers, got {0}")]
    TooFewOffers(usize),

    #[error("invalid priority weights: total must be > 0")]
    ZeroWeights,
}

pub type ComparisonResult<T> = std::result::Result<T, ComparisonError>;
```

## Testing Strategy

### Unit Tests

All pure functions are exhaustively tested in `lazyjob-core/src/salary/comparison.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ComparisonEngine::resolve() tests
    #[test] fn three_offers_ranked_by_default_weights() { .. }
    #[test] fn single_dimension_weight_picks_highest_value() { .. }
    #[test] fn all_equal_offers_all_score_1_0() { .. }
    #[test] fn zero_weights_returns_zero_scores() { .. }

    // apply_scenario() tests
    #[test] fn base_delta_reflected_in_total_comp() { .. }
    #[test] fn equity_delta_annualized_by_vest_years() { .. }
    #[test] fn negative_delta_reduces_tc() { .. }

    // project_tc() tests
    #[test] fn cliff_blocks_equity_in_year_1_for_18mo_cliff() { .. }
    #[test] fn post_vest_years_equity_is_zero() { .. }
    #[test] fn signing_only_in_year_1() { .. }

    // classify_expiry() tests
    #[test] fn expiry_critical_at_1_day() { .. }
    #[test] fn expiry_comfortable_at_30_days() { .. }
    #[test] fn expiry_comfortable_when_no_date() { .. }

    // parse_dollar_input() tests
    #[test] fn parse_dollar_k_suffix() { .. }
    #[test] fn parse_raw_integer_large() { .. }
    #[test] fn parse_heuristic_small_integer_treated_as_thousands() { .. }
}
```

### Integration Tests (SQLite)

```rust
// lazyjob-core/tests/comparison_repo_test.rs

#[sqlx::test(migrations = "migrations")]
async fn create_and_reload_session(pool: sqlx::SqlitePool) {
    let repo = SqliteComparisonRepo::new(pool);
    let session = ComparisonSession {
        id: ComparisonId::new(),
        offer_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        priorities: ComparisonPriorities::default(),
        ..
    };
    repo.create(&session).await.unwrap();
    let loaded = repo.get(&session.id).await.unwrap().unwrap();
    assert_eq!(loaded.offer_ids, session.offer_ids);
}

#[sqlx::test(migrations = "migrations")]
async fn update_priorities_persisted(pool: sqlx::SqlitePool) { .. }

#[sqlx::test(migrations = "migrations")]
async fn save_and_list_scenarios(pool: sqlx::SqlitePool) { .. }
```

### TUI Tests

- `ComparisonTableView::render()` is tested by calling `ratatui::Terminal::new(TestBackend::new(80, 40))` and asserting cell content via `backend.buffer()`.
- `parse_dollar_input()` has dedicated unit tests with all three input formats.
- `WeightedScorePanel` is tested for boundary: weight sum overflow clamped to 100.

### Export Tests

```rust
#[test]
fn csv_export_round_trip() {
    let result = fixture_comparison_result_3_offers();
    let mut buf = Vec::new();
    CsvExporter::export(&result, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("Stripe"));
    assert!(s.contains("298500.00")); // $298,500 total comp
}

#[test]
fn json_export_is_valid_json() {
    let result = fixture_comparison_result_3_offers();
    let json = JsonExporter::export(&result).unwrap();
    let _: serde_json::Value = serde_json::from_str(&json).unwrap();
}
```

## Open Questions

1. **Tax estimation**: The spec asks about pre-tax vs post-tax. The plan defers this — showing pre-tax is accurate and universal. Post-tax would require jurisdiction + filing status input. Recommend adding a `lazyjob offers tax-estimate` subcommand in a follow-up spec rather than embedding it here.

2. **Multi-year equity appreciation**: The projection currently uses a fixed current price for all years. A `price_growth_pct_annual` user input could model expected appreciation (e.g., 15% CAGR). Deferred to Phase 5.

3. **Competitor offer privacy in SaaS mode**: When `SalaryPrivacyFilter::is_table_syncable()` is extended, `offer_comparison_sessions` must be excluded from cloud sync by default. This is enforced in `lazyjob-sync` as part of Cross-Spec S from `08-gaps-salary-tui-implementation-plan.md`.

4. **Source priority for `OfferRecord` resolver**: `ComparisonEngine::resolve_one()` calls `compute_total_comp()` which was defined in the market intelligence plan. If `OfferRecord.equity_grant.total_grant_cents` is not populated (user entered grant type but not value), the resolver must return `equity_annual_cents = 0` and log a `tracing::warn!` rather than panicking. Add a `DataQualityWarning::MissingEquityValue` variant to surface this in the TUI.

5. **`comfy-table` vs `tabled`**: The CLI subcommand uses one of these for stdout table rendering. Check which is already used in `lazyjob-cli` before adding a new dependency.

6. **`ComparisonPriorities::growth_stage` and `mission_fit`**: These are currently dead-weight in Phase 1 (always 0). They require the user to score each offer on a qualitative dimension (1–10) before they contribute to the weighted score. The UX for entering these scores (offer form vs comparison panel) is unresolved — defer to Phase 5 product design.

## Related Specs
- [salary-negotiation-offers.md](salary-negotiation-offers.md) — `OfferRecord`, `NegotiationPriorities`, `compute_total_comp`, `rank_offers()`
- [salary-market-intelligence.md](salary-market-intelligence.md) — `SalaryBenchmark`, market p25/p50/p75 data
- [salary-counter-offer-drafting.md](salary-counter-offer-drafting.md) — `CounterOfferLoop`, `NegotiationContext`
- [application-workflow-actions.md](application-workflow-actions.md) — `RecordOfferWorkflow`, `OfferReceivedEvent`
- [08-gaps-salary-tui.md](08-gaps-salary-tui.md) — GAP-81 Black-Scholes, GAP-82 offer letter parsing
- [09-tui-design-keybindings.md](09-tui-design-keybindings.md) — TUI event loop, panel overlays, `FormWidget`
