# Implementation Plan: Salary Market Intelligence

## Status
Draft

## Related Spec
[specs/salary-market-intelligence.md](salary-market-intelligence.md)

## Overview

`SalaryIntelligenceService` answers "what is this offer actually worth, and how does it compare to market?" It ingests a full offer (base salary, equity grant, signing bonus, cash bonus, vesting schedule, company stage) entered by the user, applies equity risk-adjustment factors to compute an annualized total compensation figure, and benchmarks the result against available market data. Market data is sourced from the U.S. DOL H1B LCA public disclosure CSVs (downloadable offline, loaded into SQLite FTS5), user-entered network reference points, and user-pasted levels.fyi salary table text — all approaches that respect platform ToS and keep data local.

The module is entirely contained in `lazyjob-core/src/salary/`. The heavy lifting (`compute_total_comp`) is a pure synchronous function with no I/O — it can be unit-tested exhaustively. The async surface is limited to `SalaryIntelligenceService::evaluate_offer()`, which queries the market data repository and returns a fully populated `OfferEvaluation` struct containing a per-component breakdown, market percentile comparison, pay transparency range check, and a list of natural-language negotiation signals.

This plan also specifies the `LevelsFyiParser` (paste-based import), the H1B LCA CSV importer (one-time setup CLI command), the `PAY_TRANSPARENT_JURISDICTIONS` static constant shared with the ghost detection module, and the TUI offer entry + comparison view. All monetary values are stored and computed as `i64` cents to avoid floating-point precision issues in comparisons and SQLite storage. The only `f32` values are the equity risk factor (a user-visible multiplier in the [0.0, 1.0] range) and gap percentages for display purposes.

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — Database connection, pool, migration infrastructure
- `specs/application-state-machine-implementation-plan.md` — `ApplicationId` newtype, `applications` table FK, `OfferRepository` base trait
- `specs/application-workflow-actions-implementation-plan.md` — `PostTransitionSuggestion::RunSalaryComparison` dispatch point
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel system, form widget

### Crates to add to Cargo.toml
```toml
# lazyjob-core/Cargo.toml
[dependencies]
csv        = "1.3"          # H1B LCA CSV parsing
encoding_rs = "0.8"        # BOM-tolerant UTF-8/Windows-1252 decoding for DOL files
zip        = "2"            # LCA disclosure ZIPs from DOL
tempfile   = "3"            # Scratch directory for ZIP extraction
```

No new crates are needed in `lazyjob-tui/Cargo.toml`; the form widget uses existing `ratatui` + `crossterm`.

## Architecture

### Crate Placement

All domain types and business logic live in `lazyjob-core/src/salary/`. The TUI form and comparison view live in `lazyjob-tui/src/views/salary/`. The CLI `lazyjob salary import-lca` subcommand lives in `lazyjob-cli/src/commands/salary.rs`.

The `PAY_TRANSPARENT_JURISDICTIONS` constant is declared in `lazyjob-core/src/salary/jurisdictions.rs` and re-exported from the crate root. The ghost detection module (`lazyjob-core/src/discovery/ghost.rs`) imports it from there — it is **not** duplicated.

### Core Types

```rust
// lazyjob-core/src/salary/model.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Wraps a UUID that identifies an offer record. Parse-don't-validate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct OfferId(pub Uuid);

impl OfferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum CompanyStage {
    Public,
    LatePrivate,  // Series D+, >$500M valuation
    MidPrivate,   // Series B/C
    EarlyPrivate, // Seed/Series A
}

impl CompanyStage {
    /// Default equity risk factor per stage.
    /// Returns a value in [0.0, 1.0] representing the expected realization fraction.
    /// Public = liquid at vesting (1.0); Early private = high dilution/illiquidity risk (0.15).
    pub fn default_risk_factor(&self) -> f32 {
        match self {
            CompanyStage::Public      => 1.00,
            CompanyStage::LatePrivate => 0.70,
            CompanyStage::MidPrivate  => 0.40,
            CompanyStage::EarlyPrivate => 0.15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EquityType {
    Rsu,  // Restricted Stock Units — value = current_price * shares
    Iso,  // Incentive Stock Options — intrinsic value only if FMV > strike
    Nso,  // Non-Qualified Stock Options
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityGrant {
    pub equity_type: EquityType,
    /// RSUs: current stock price × share count. Stored as cents.
    pub total_grant_usd_cents: Option<i64>,
    pub grant_shares: Option<i64>,
    pub vest_years: u8,
    /// Cliff in months before any vesting occurs (typically 12).
    pub cliff_months: u8,
    /// Options only: 409A FMV per share, in cents.
    pub fmv_per_share_cents: Option<i64>,
    /// Options only: exercise price per share, in cents.
    pub strike_price_cents: Option<i64>,
    /// "single trigger" | "double trigger" | None
    pub acceleration: Option<String>,
}

impl EquityGrant {
    /// Intrinsic value of the full grant in cents.
    /// - RSUs: total_grant_usd_cents (already a dollar value)
    /// - Options: (fmv - strike) * shares; returns 0 if underwater
    pub fn intrinsic_value_cents(&self) -> i64 {
        match self.equity_type {
            EquityType::Rsu => self.total_grant_usd_cents.unwrap_or(0),
            EquityType::Iso | EquityType::Nso => {
                let fmv = self.fmv_per_share_cents.unwrap_or(0);
                let strike = self.strike_price_cents.unwrap_or(0);
                let shares = self.grant_shares.unwrap_or(0);
                if fmv > strike {
                    (fmv - strike) * shares
                } else {
                    0
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferDetails {
    pub id: OfferId,
    pub application_id: crate::application::ApplicationId,
    pub company_name: String,
    pub job_title: String,
    /// Normalized location string used for pay transparency lookup.
    /// Expected format: "City, ST" or bare state abbreviation "CA".
    pub location: String,
    pub company_stage: CompanyStage,
    /// Annual base salary in cents.
    pub base_annual_cents: i64,
    /// Fixed annual cash bonus in cents. Exclusive of bonus_pct.
    pub bonus_annual_cents: Option<i64>,
    /// Target bonus as fraction of base (e.g. 0.15 = 15%). Exclusive of bonus_annual_cents.
    pub bonus_pct: Option<f32>,
    pub signing_bonus_cents: Option<i64>,
    pub equity: Option<EquityGrant>,
    /// User override for the equity risk factor; overrides CompanyStage::default_risk_factor().
    pub equity_risk_override: Option<f32>,
    pub received_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

/// Result of computing annualized total comp for a single offer.
/// All monetary fields are in cents. risk_factor_applied is the f32 used for equity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalCompBreakdown {
    pub offer_id: OfferId,
    pub base_annual: i64,
    pub bonus_annual: i64,
    pub equity_annual_risk_adjusted: i64,
    pub signing_amortized: i64,
    pub annualized_total: i64,
    pub risk_factor_applied: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketDataSource {
    H1bLca,
    UserProvided,
    LevelsFyiPaste,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDataPoint {
    pub id: Uuid,
    pub source: MarketDataSource,
    pub role: String,
    pub company: Option<String>,
    pub location: String,
    /// Percentile values in cents.
    pub base_p25_cents: i64,
    pub base_p50_cents: i64,
    pub base_p75_cents: i64,
    pub total_comp_p50_cents: Option<i64>,
    pub sample_count: Option<u32>,
    pub as_of_date: NaiveDate,
}

/// Full offer evaluation: breakdown + market comparison + negotiation signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferEvaluation {
    pub offer: OfferDetails,
    pub breakdown: TotalCompBreakdown,
    /// All market data points found for this role+location combination.
    pub market_data: Vec<MarketDataPoint>,
    /// Weighted p50 total comp across all market sources. None if fewer than 3 data points.
    pub market_p50_total_cents: Option<i64>,
    /// (offer_annualized_total - market_p50) / market_p50. Positive = above market.
    pub gap_vs_market_pct: Option<f32>,
    /// None if jurisdiction is not pay-transparent or role has no posted range.
    pub in_posted_range: Option<bool>,
    /// Breakdowns for competing offers, sorted descending by annualized_total.
    pub competing_offers: Vec<TotalCompBreakdown>,
    /// Human-readable negotiation signals, e.g. "Offer is 12% below H1B median".
    pub negotiation_signals: Vec<String>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/salary/repository.rs

#[async_trait::async_trait]
pub trait OfferRepository: Send + Sync {
    async fn save(&self, offer: &OfferDetails) -> Result<()>;
    async fn get(&self, id: &OfferId) -> Result<Option<OfferDetails>>;
    async fn list_for_application(
        &self,
        application_id: &crate::application::ApplicationId,
    ) -> Result<Vec<OfferDetails>>;
    async fn delete(&self, id: &OfferId) -> Result<()>;
}

#[async_trait::async_trait]
pub trait MarketDataRepository: Send + Sync {
    /// Find market data points for a (role, location) pair.
    /// `role` is a normalized job title; `location` is a state abbreviation or "City, ST".
    async fn find_market_data(
        &self,
        role: &str,
        location: &str,
    ) -> Result<Vec<MarketDataPoint>>;

    async fn save_market_data(&self, points: &[MarketDataPoint]) -> Result<usize>;

    /// Returns count of H1B LCA records imported per year.
    async fn lca_record_count(&self) -> Result<u64>;

    async fn save_salary_reference(&self, ref_point: &SalaryReference) -> Result<()>;
    async fn list_salary_references(&self) -> Result<Vec<SalaryReference>>;
}

/// User-entered reference point from their personal network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalaryReference {
    pub id: Uuid,
    pub source_note: String, // e.g. "Friend at Google L5 2024"
    pub role: String,
    pub company: Option<String>,
    pub location: String,
    pub base_annual_cents: i64,
    pub total_comp_cents: Option<i64>,
    pub as_of_date: NaiveDate,
}
```

### Service Definition

```rust
// lazyjob-core/src/salary/service.rs

pub struct SalaryIntelligenceService {
    market_repo: Arc<dyn MarketDataRepository>,
    offer_repo: Arc<dyn OfferRepository>,
}

impl SalaryIntelligenceService {
    pub fn new(
        market_repo: Arc<dyn MarketDataRepository>,
        offer_repo: Arc<dyn OfferRepository>,
    ) -> Self { ... }

    /// Pure computation — no I/O. Returns breakdown for a single offer.
    pub fn compute_total_comp(offer: &OfferDetails) -> TotalCompBreakdown { ... }

    /// Async: fetches market data, evaluates, returns full OfferEvaluation.
    pub async fn evaluate_offer(
        &self,
        offer: &OfferDetails,
        competing: &[OfferDetails],
    ) -> Result<OfferEvaluation> { ... }

    /// Checks whether `location` is in a pay-transparent jurisdiction.
    pub fn is_pay_transparent_jurisdiction(location: &str) -> bool { ... }

    /// Derives natural-language negotiation signals from an evaluation.
    fn derive_negotiation_signals(eval: &OfferEvaluation) -> Vec<String> { ... }
}
```

### SQLite Schema

Migration `014_salary_market_intelligence.sql`:

```sql
-- Detailed offer records. Privacy-sensitive: excluded from SaaS sync scope.
CREATE TABLE offer_details (
    id                   TEXT PRIMARY KEY,
    application_id       TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    company_name         TEXT NOT NULL,
    job_title            TEXT NOT NULL,
    location             TEXT NOT NULL,
    company_stage        TEXT NOT NULL,         -- 'public' | 'late_private' | 'mid_private' | 'early_private'
    base_annual_cents    INTEGER NOT NULL,
    bonus_annual_cents   INTEGER,
    bonus_pct            REAL,                  -- e.g. 0.15 for 15%
    signing_bonus_cents  INTEGER,
    equity_json          TEXT,                  -- EquityGrant serialized as JSON
    equity_risk_override REAL,                  -- user override in [0.0, 1.0]
    received_at          TEXT NOT NULL,
    expires_at           TEXT,
    notes                TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_offer_details_application ON offer_details(application_id);
CREATE INDEX idx_offer_details_expires ON offer_details(expires_at)
    WHERE expires_at IS NOT NULL;

-- Market data from H1B LCA, user-provided, levels.fyi paste imports.
CREATE TABLE market_data_references (
    id           TEXT PRIMARY KEY,
    source       TEXT NOT NULL,     -- 'h1b_lca' | 'user_provided' | 'levels_fyi_paste'
    role         TEXT NOT NULL,     -- job title, normalized to lowercase
    role_fts     TEXT NOT NULL,     -- FTS5 input (same as role, space-tokenized)
    company      TEXT,
    location     TEXT NOT NULL,     -- e.g. "CA" or "San Francisco, CA"
    base_p25     INTEGER NOT NULL,  -- cents
    base_p50     INTEGER NOT NULL,  -- cents
    base_p75     INTEGER NOT NULL,  -- cents
    total_p50    INTEGER,           -- cents; NULL for H1B LCA (no TC data)
    sample_count INTEGER,
    as_of_date   TEXT NOT NULL,     -- ISO date YYYY-MM-DD
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_mdr_role_location ON market_data_references(role, location);

-- FTS5 virtual table for fuzzy role search.
CREATE VIRTUAL TABLE market_data_fts USING fts5(
    role_fts,
    content='market_data_references',
    content_rowid='rowid'
);

-- Triggers to keep FTS in sync.
CREATE TRIGGER mdr_fts_insert AFTER INSERT ON market_data_references BEGIN
    INSERT INTO market_data_fts(rowid, role_fts) VALUES (new.rowid, new.role_fts);
END;

CREATE TRIGGER mdr_fts_delete AFTER DELETE ON market_data_references BEGIN
    INSERT INTO market_data_fts(market_data_fts, rowid, role_fts) VALUES ('delete', old.rowid, old.role_fts);
END;

-- User-entered salary reference points from personal network.
CREATE TABLE salary_references (
    id              TEXT PRIMARY KEY,
    source_note     TEXT NOT NULL,   -- e.g. "Friend at Google L5 2024"
    role            TEXT NOT NULL,
    company         TEXT,
    location        TEXT NOT NULL,
    base_annual     INTEGER NOT NULL, -- cents
    total_comp      INTEGER,          -- cents
    as_of_date      TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Module Structure

```
lazyjob-core/
  src/
    salary/
      mod.rs           # pub use re-exports, SalaryResult alias
      model.rs         # OfferDetails, EquityGrant, TotalCompBreakdown, OfferEvaluation, MarketDataPoint
      repository.rs    # OfferRepository + MarketDataRepository traits, SalaryReference
      service.rs       # SalaryIntelligenceService
      jurisdictions.rs # PAY_TRANSPARENT_JURISDICTIONS Lazy<HashSet>
      tc_calc.rs       # compute_total_comp (pure fn), risk_factor helpers
      lca_importer.rs  # H1bLcaImporter: CSV download + parse + upsert
      levels_fyi.rs    # LevelsFyiParser::parse_paste
      sqlite.rs        # SqliteOfferRepository + SqliteMarketDataRepository

lazyjob-tui/
  src/
    views/
      salary/
        mod.rs
        offer_form.rs  # OfferEntryForm widget (ratatui)
        comparison.rs  # OfferComparisonPanel widget
        breakdown.rs   # TotalCompBreakdownWidget

lazyjob-cli/
  src/
    commands/
      salary.rs        # `lazyjob salary import-lca [--year 2024]`
```

## Implementation Phases

### Phase 1 — Core Data Model and Pure Computation (MVP)

#### Step 1.1 — Domain types
**File:** `lazyjob-core/src/salary/model.rs`

Define all structs and enums from the Core Types section above. Key implementation notes:
- All monetary fields are `i64` (cents). No `f64` for money.
- `EquityGrant::intrinsic_value_cents()` is the only method on the type.
- `CompanyStage::default_risk_factor()` returns `f32` — this is display-only, never used in monetary arithmetic. Multiply as `(intrinsic_cents as f64 * risk_factor as f64) as i64` to avoid precision loss on large grants.
- Derive `sqlx::Type` for `CompanyStage` with `#[sqlx(rename_all = "snake_case")]` so it maps to `"public"`, `"late_private"`, etc. in SQLite TEXT columns.

**Verification:** `cargo check --package lazyjob-core` passes with all types visible.

#### Step 1.2 — Pay transparency jurisdictions
**File:** `lazyjob-core/src/salary/jurisdictions.rs`

```rust
use once_cell::sync::Lazy;
use std::collections::HashSet;

/// State abbreviations and DC where employers must post salary ranges.
/// Updated to include 2024 state laws: CA, CO, NY, WA, IL, NJ, MA, MD, RI, HI, NV, DC.
/// NOTE: Also consumed by lazyjob-core::discovery::ghost for salary_absent signal.
pub static PAY_TRANSPARENT_JURISDICTIONS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    HashSet::from([
        "CA", "CO", "NY", "WA", "IL", "NJ", "MA", "MD", "RI", "HI", "NV", "DC",
    ])
});

/// Extracts the state abbreviation from a location string like "San Francisco, CA" → "CA".
/// Returns None if no two-letter US state code can be found.
pub fn extract_state(location: &str) -> Option<&str> {
    location
        .split(',')
        .last()
        .map(|s| s.trim())
        .filter(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_uppercase()))
}
```

**Verification:** `use lazyjob_core::salary::jurisdictions::PAY_TRANSPARENT_JURISDICTIONS` compiles in both `lazyjob-core/src/discovery/ghost.rs` and `salary/service.rs`.

#### Step 1.3 — Total comp calculator
**File:** `lazyjob-core/src/salary/tc_calc.rs`

```rust
pub fn compute_total_comp(offer: &OfferDetails) -> TotalCompBreakdown {
    // 1. Bonus: prefer fixed bonus_annual_cents; fall back to bonus_pct * base.
    let bonus_annual = offer
        .bonus_annual_cents
        .or_else(|| {
            offer
                .bonus_pct
                .map(|pct| (offer.base_annual_cents as f64 * pct as f64) as i64)
        })
        .unwrap_or(0);

    // 2. Equity: risk-adjusted annualized value.
    let risk_factor = offer
        .equity_risk_override
        .unwrap_or_else(|| offer.company_stage.default_risk_factor());

    let equity_annual_risk_adjusted = offer
        .equity
        .as_ref()
        .map(|eq| {
            let intrinsic = eq.intrinsic_value_cents();
            let vest_years = eq.vest_years.max(1) as f64;
            let risk = risk_factor as f64;
            ((intrinsic as f64 / vest_years) * risk) as i64
        })
        .unwrap_or(0);

    // 3. Signing bonus amortized over min(2, vest_years).
    let amort_years = offer
        .equity
        .as_ref()
        .map(|eq| eq.vest_years.min(2).max(1) as f64)
        .unwrap_or(2.0);
    let signing_amortized = offer
        .signing_bonus_cents
        .map(|s| (s as f64 / amort_years) as i64)
        .unwrap_or(0);

    let annualized_total =
        offer.base_annual_cents + bonus_annual + equity_annual_risk_adjusted + signing_amortized;

    TotalCompBreakdown {
        offer_id: offer.id.clone(),
        base_annual: offer.base_annual_cents,
        bonus_annual,
        equity_annual_risk_adjusted,
        signing_amortized,
        annualized_total,
        risk_factor_applied: risk_factor,
    }
}
```

**Verification:**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_rsu_public_no_bonus() {
        // $200k base, $100k RSU/4yr, no bonus, Public → TC = $225k
        let offer = make_offer(200_000_00, None, None, None, Some(rsu_grant(100_000_00, 4)), None, CompanyStage::Public);
        let bd = compute_total_comp(&offer);
        assert_eq!(bd.annualized_total, 225_000_00);
        assert_eq!(bd.risk_factor_applied, 1.0);
    }

    #[test]
    fn test_option_underwater_gives_zero_equity() {
        // Strike > FMV → intrinsic = 0
        let grant = EquityGrant { equity_type: EquityType::Iso, strike_price_cents: Some(1500), fmv_per_share_cents: Some(1000), grant_shares: Some(10_000), vest_years: 4, cliff_months: 12, total_grant_usd_cents: None, acceleration: None };
        let offer = make_offer(150_000_00, None, None, None, Some(grant), None, CompanyStage::EarlyPrivate);
        let bd = compute_total_comp(&offer);
        assert_eq!(bd.equity_annual_risk_adjusted, 0);
    }

    #[test]
    fn test_early_private_risk_factor() {
        // $100k RSU/4yr at EarlyPrivate (0.15 risk) → equity_annual = $3,750
        let offer = make_offer(100_000_00, None, None, None, Some(rsu_grant(100_000_00, 4)), None, CompanyStage::EarlyPrivate);
        let bd = compute_total_comp(&offer);
        assert_eq!(bd.equity_annual_risk_adjusted, 3_750_00);
    }
}
```

#### Step 1.4 — SQLite migration
**File:** `migrations/014_salary_market_intelligence.sql`

Contains the full DDL from the SQLite Schema section above. Applied automatically on `Database::new()`.

**Verification:** `sqlx migrate run` applies cleanly against an empty test DB.

---

### Phase 2 — Repository Implementations

#### Step 2.1 — SqliteOfferRepository
**File:** `lazyjob-core/src/salary/sqlite.rs`

```rust
pub struct SqliteOfferRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait::async_trait]
impl OfferRepository for SqliteOfferRepository {
    async fn save(&self, offer: &OfferDetails) -> Result<()> {
        let equity_json = offer
            .equity
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serialize equity grant")?;
        sqlx::query!(
            r#"
            INSERT INTO offer_details
                (id, application_id, company_name, job_title, location, company_stage,
                 base_annual_cents, bonus_annual_cents, bonus_pct, signing_bonus_cents,
                 equity_json, equity_risk_override, received_at, expires_at, notes)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                base_annual_cents = excluded.base_annual_cents,
                bonus_annual_cents = excluded.bonus_annual_cents,
                bonus_pct = excluded.bonus_pct,
                signing_bonus_cents = excluded.signing_bonus_cents,
                equity_json = excluded.equity_json,
                equity_risk_override = excluded.equity_risk_override,
                expires_at = excluded.expires_at,
                notes = excluded.notes,
                updated_at = datetime('now')
            "#,
            offer.id.0.to_string(),
            offer.application_id.0.to_string(),
            offer.company_name,
            offer.job_title,
            offer.location,
            offer.company_stage as i32,  // mapped via sqlx::Type
            offer.base_annual_cents,
            offer.bonus_annual_cents,
            offer.bonus_pct,
            offer.signing_bonus_cents,
            equity_json,
            offer.equity_risk_override,
            offer.received_at.to_rfc3339(),
            offer.expires_at.map(|d| d.to_rfc3339()),
            offer.notes,
        )
        .execute(&self.pool)
        .await
        .context("save offer_details")?;
        Ok(())
    }

    async fn list_for_application(
        &self,
        application_id: &ApplicationId,
    ) -> Result<Vec<OfferDetails>> {
        let rows = sqlx::query!(
            "SELECT * FROM offer_details WHERE application_id = ? ORDER BY received_at DESC",
            application_id.0.to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|row| row_to_offer_details(&row)).collect()
    }
    // ... get, delete
}
```

Key note: `EquityGrant` is stored as `equity_json TEXT` and round-tripped via `serde_json::to_string` / `serde_json::from_str`. This avoids schema coupling to the equity struct's fields while keeping it fully queryable via `json_extract()` if needed.

#### Step 2.2 — SqliteMarketDataRepository
**File:** `lazyjob-core/src/salary/sqlite.rs` (same file)

```rust
#[async_trait::async_trait]
impl MarketDataRepository for SqliteMarketDataRepository {
    async fn find_market_data(&self, role: &str, location: &str) -> Result<Vec<MarketDataPoint>> {
        let state = jurisdictions::extract_state(location).unwrap_or(location);
        // First pass: exact role + state match from FTS5 index.
        let exact_rows = sqlx::query!(
            r#"
            SELECT mdr.*
            FROM market_data_references mdr
            JOIN market_data_fts fts ON fts.rowid = mdr.rowid
            WHERE market_data_fts MATCH ? AND (mdr.location = ? OR mdr.location LIKE ?)
            ORDER BY rank
            LIMIT 50
            "#,
            role,
            state,
            format!("%, {}", state),
        )
        .fetch_all(&self.pool)
        .await?;
        // Convert rows to MarketDataPoint, include salary_references as UserProvided points.
        // ...
        Ok(results)
    }

    async fn save_market_data(&self, points: &[MarketDataPoint]) -> Result<usize> {
        let mut count = 0usize;
        let mut tx = self.pool.begin().await?;
        for p in points {
            sqlx::query!(
                r#"
                INSERT OR IGNORE INTO market_data_references
                    (id, source, role, role_fts, company, location,
                     base_p25, base_p50, base_p75, total_p50, sample_count, as_of_date)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                p.id.to_string(),
                source_to_str(&p.source),
                p.role,
                p.role,  // role_fts = same as role; FTS5 tokenizes on insert
                p.company,
                p.location,
                p.base_p25_cents,
                p.base_p50_cents,
                p.base_p75_cents,
                p.total_comp_p50_cents,
                p.sample_count.map(|c| c as i64),
                p.as_of_date.to_string(),
            )
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        tx.commit().await?;
        Ok(count)
    }
}
```

---

### Phase 3 — H1B LCA CSV Importer

#### Step 3.1 — Importer struct
**File:** `lazyjob-core/src/salary/lca_importer.rs`

DOL publishes LCA disclosure data as annual ZIP files at https://www.dol.gov/agencies/eta/foreign-labor/performance — these URLs are stable and bookmarked in the spec. The importer downloads, extracts, and parses the CSV into `MarketDataPoint` records grouped by `(SOC_TITLE_normalized, WORKSITE_STATE)`.

```rust
pub struct H1bLcaImporter {
    market_repo: Arc<dyn MarketDataRepository>,
    http: reqwest::Client,
}

impl H1bLcaImporter {
    /// Downloads the DOL LCA disclosure ZIP for `year`, extracts the CSV,
    /// parses it into MarketDataPoint records (grouped by role+state), and
    /// upserts them into `market_data_references`.
    ///
    /// Progress callback receives (records_processed, total_estimated).
    pub async fn import_year(
        &self,
        year: u16,
        progress: impl Fn(u64, u64) + Send + 'static,
    ) -> Result<ImportReport> { ... }

    fn lca_csv_url(year: u16) -> String {
        format!(
            "https://www.dol.gov/sites/dolgov/files/ETA/oflc/pdfs/LCA_Disclosure_Data_FY{}_Q4.xlsx",
            year
        )
    }
}

pub struct ImportReport {
    pub year: u16,
    pub records_read: u64,
    pub records_imported: u64,
    pub roles_discovered: u64,
    pub errors: Vec<String>,
}
```

Key parsing decisions:
- DOL publishes as Excel XLSX (not CSV) for recent years. Use the `calamine` crate (`calamine = "0.24"`) for XLSX parsing in a `tokio::task::spawn_blocking` block.
- Aggregate `WAGE_RATE_OF_PAY_FROM` (base salary in the LCA) by `(SOC_TITLE, WORKSITE_STATE)` to compute p25/p50/p75 percentiles within Rust. A `BTreeMap<(String, String), Vec<i64>>` accumulates all wage values per group. After collecting, sort each Vec and compute percentiles with index arithmetic.
- Wage amounts are in USD/year. Convert to cents: `wage_usd * 100`.
- Filter `VISA_CLASS = "H-1B"` and `CASE_STATUS = "Certified"` only.
- Only include groups with ≥10 records to ensure statistical validity. Groups with fewer records are discarded silently (logged at `tracing::debug!`).
- `as_of_date` = last working day of the fiscal year quarter (hardcoded per year).

**Note:** Add `calamine = "0.24"` to `lazyjob-core/Cargo.toml`.

#### Step 3.2 — CLI command
**File:** `lazyjob-cli/src/commands/salary.rs`

```rust
/// lazyjob salary import-lca [--year 2024]
pub async fn import_lca(db: &Database, year: Option<u16>) -> anyhow::Result<()> {
    let year = year.unwrap_or(2024u16);
    println!("Importing H1B LCA data for FY{year}...");
    // Display progress bar via `indicatif` (already a dependency from ralph commands).
    let bar = ProgressBar::new_spinner();
    let report = importer.import_year(year, move |done, total| bar.set_position(done)).await?;
    println!(
        "Imported {} roles from {} records (FY{year})",
        report.roles_discovered,
        report.records_imported,
    );
    Ok(())
}
```

**Verification:** `lazyjob salary import-lca --year 2023` runs without panic, logs import count, and `SELECT count(*) FROM market_data_references WHERE source = 'h1b_lca'` returns a non-zero count.

---

### Phase 4 — LevelsFyi Paste Parser

**File:** `lazyjob-core/src/salary/levels_fyi.rs`

The user visits levels.fyi, selects the salary table (e.g. for "Software Engineer" at a company), and copies the visible text. The parser interprets the resulting plain-text block using a state machine, not regex — the table format has consistent column ordering but variable spacing.

Expected paste format (approximate):
```
Company   Level   Base    Stock   Bonus   Total
Google    L5      $220k   $50k    $40k    $310k
...
```

```rust
pub struct LevelsFyiParser;

impl LevelsFyiParser {
    /// Parse pasted levels.fyi table text into MarketDataPoint records.
    /// Returns an error only if the text contains no parseable rows.
    /// Individual unparseable rows are logged as warnings and skipped.
    pub fn parse_paste(text: &str, role: &str, location: &str) -> Result<Vec<MarketDataPoint>, LevelsFyiParseError> {
        let rows = Self::extract_data_rows(text)?;
        let base_values: Vec<i64> = rows.iter().filter_map(|r| r.base_cents).collect();
        let total_values: Vec<i64> = rows.iter().filter_map(|r| r.total_cents).collect();

        if base_values.is_empty() {
            return Err(LevelsFyiParseError::NoDataRows);
        }

        let point = MarketDataPoint {
            id: Uuid::new_v4(),
            source: MarketDataSource::LevelsFyiPaste,
            role: role.to_lowercase(),
            company: None, // paste may contain multiple companies; aggregate
            location: location.to_string(),
            base_p25_cents: percentile_cents(&base_values, 25),
            base_p50_cents: percentile_cents(&base_values, 50),
            base_p75_cents: percentile_cents(&base_values, 75),
            total_comp_p50_cents: if total_values.len() >= 3 {
                Some(percentile_cents(&total_values, 50))
            } else {
                None
            },
            sample_count: Some(base_values.len() as u32),
            as_of_date: chrono::Utc::now().date_naive(),
        };
        Ok(vec![point])
    }

    fn extract_data_rows(text: &str) -> Result<Vec<ParsedRow>, LevelsFyiParseError> { ... }

    fn parse_dollar_amount(s: &str) -> Option<i64> {
        // "220k" → 22_000_00 (cents), "$220,000" → 22_000_000 (cents)
        let s = s.trim().trim_start_matches('$').replace(',', "");
        if let Some(s) = s.strip_suffix('k') {
            s.parse::<f64>().ok().map(|v| (v * 1000.0 * 100.0) as i64)
        } else {
            s.parse::<f64>().ok().map(|v| (v * 100.0) as i64)
        }
    }
}

struct ParsedRow {
    base_cents: Option<i64>,
    total_cents: Option<i64>,
}

fn percentile_cents(values: &[i64], pct: usize) -> i64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((pct as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx]
}
```

**Verification:**
```rust
#[test]
fn test_parse_paste_k_suffix() {
    let text = "Company Level Base Stock Bonus Total\nGoogle L5 $220k $50k $40k $310k\nGoogle L5 $210k $45k $35k $290k";
    let pts = LevelsFyiParser::parse_paste(text, "software engineer", "CA").unwrap();
    assert_eq!(pts[0].sample_count, Some(2));
    assert_eq!(pts[0].base_p50_cents, 215_000_00); // median of 210k, 220k
}
```

---

### Phase 5 — SalaryIntelligenceService

**File:** `lazyjob-core/src/salary/service.rs`

```rust
impl SalaryIntelligenceService {
    pub async fn evaluate_offer(
        &self,
        offer: &OfferDetails,
        competing: &[OfferDetails],
    ) -> Result<OfferEvaluation> {
        let breakdown = Self::compute_total_comp(offer);

        // 1. Fetch market data (FTS5 search + state filter).
        let market_data = self
            .market_repo
            .find_market_data(&offer.job_title, &offer.location)
            .await?;

        // 2. Compute weighted market p50 (weight by sample_count; min 3 data points).
        let market_p50_total_cents = Self::compute_market_p50(&market_data);

        // 3. Gap vs market percentage.
        let gap_vs_market_pct = market_p50_total_cents.map(|p50| {
            (breakdown.annualized_total - p50) as f32 / p50 as f32 * 100.0
        });

        // 4. Pay transparency check.
        let in_posted_range = if Self::is_pay_transparent_jurisdiction(&offer.location) {
            // TODO Phase 5: fetch posted range from job record when available
            None // Not yet wired to job posting data
        } else {
            None
        };

        // 5. Competing offer breakdowns, sorted descending by TC.
        let mut competing_offers: Vec<TotalCompBreakdown> = competing
            .iter()
            .map(Self::compute_total_comp)
            .collect();
        competing_offers.sort_by_key(|b| std::cmp::Reverse(b.annualized_total));

        // 6. Build partial evaluation (before signals).
        let mut eval = OfferEvaluation {
            offer: offer.clone(),
            breakdown,
            market_data,
            market_p50_total_cents,
            gap_vs_market_pct,
            in_posted_range,
            competing_offers,
            negotiation_signals: vec![],
        };

        // 7. Derive signals after all fields are populated.
        eval.negotiation_signals = Self::derive_negotiation_signals(&eval);
        Ok(eval)
    }

    fn compute_market_p50(data: &[MarketDataPoint]) -> Option<i64> {
        let total_comp_points: Vec<i64> = data
            .iter()
            .filter_map(|p| p.total_comp_p50_cents)
            .collect();
        if total_comp_points.len() < 3 {
            // Fall back to base_p50 if total_comp not available.
            let base_points: Vec<i64> = data.iter().map(|p| p.base_p50_cents).collect();
            if base_points.len() < 3 {
                return None;
            }
            Some(percentile_cents(&base_points, 50))
        } else {
            Some(percentile_cents(&total_comp_points, 50))
        }
    }

    fn derive_negotiation_signals(eval: &OfferEvaluation) -> Vec<String> {
        let mut signals = Vec::new();

        if let Some(gap) = eval.gap_vs_market_pct {
            if gap < -10.0 {
                signals.push(format!(
                    "Offer is {:.0}% below market median — strong negotiation leverage.",
                    gap.abs()
                ));
            } else if gap < 0.0 {
                signals.push(format!(
                    "Offer is {:.0}% below market median — modest upside from negotiating.",
                    gap.abs()
                ));
            } else if gap > 15.0 {
                signals.push(format!(
                    "Offer is {:.0}% above market median — well-positioned.",
                    gap
                ));
            }
        }

        if !eval.competing_offers.is_empty() {
            let best_competing = eval.competing_offers[0].annualized_total;
            if best_competing > eval.breakdown.annualized_total {
                let delta = best_competing - eval.breakdown.annualized_total;
                signals.push(format!(
                    "Competing offer is ${:.0}k/yr higher — disclose to negotiate up.",
                    delta as f64 / 100.0 / 1000.0
                ));
            }
        }

        if eval.offer.equity_risk_override.is_none() {
            match eval.offer.company_stage {
                CompanyStage::EarlyPrivate => {
                    signals.push(
                        "Equity risk factor is 0.15 (early private). Consider negotiating for more RSUs or a higher base to offset illiquidity risk.".to_string(),
                    );
                }
                CompanyStage::MidPrivate => {
                    signals.push(
                        "Equity risk factor is 0.40 (mid-stage private). Review liquidation preference and secondary market options.".to_string(),
                    );
                }
                _ => {}
            }
        }

        signals
    }

    pub fn is_pay_transparent_jurisdiction(location: &str) -> bool {
        let state = jurisdictions::extract_state(location).unwrap_or(location.trim());
        PAY_TRANSPARENT_JURISDICTIONS.contains(state)
    }
}
```

---

### Phase 6 — TUI Offer Entry and Comparison View

**File:** `lazyjob-tui/src/views/salary/offer_form.rs`

The TUI offer entry form opens automatically when `PostTransitionSuggestion::RunSalaryComparison` is dispatched (triggered by `application-workflow-actions.md` when an application enters the `Offer` stage). It can also be opened manually from the application detail view with the `o` keybinding.

#### Layout

```
┌─────────────── Offer Details ─────────────────────────────────────────────────────┐
│ Company: [Acme Corp              ] Stage: [Mid Private ▾]                          │
│ Title:   [Senior Engineer        ] Location: [San Francisco, CA  ]                 │
│ Base:    [$220,000 /yr           ] Expires: [2026-05-01          ]                 │
│ Bonus:   [$30,000               ] or  [  %]                                        │
│ Signing: [$20,000               ]                                                  │
│                                                                                    │
│ Equity Type: [RSU ▾]   Total Grant: [$400,000] Vest: [4] yrs  Cliff: [12] mo     │
│ Risk Override: [     ] (leave blank to use stage default: 0.40)                   │
│                                                                                    │
│ ─── Live Breakdown ──────────────────────────────────────────────────────────────  │
│  Base:               $220,000                                                      │
│  Bonus:              $30,000                                                       │
│  Equity (risk-adj):  $40,000    (risk factor: 0.40)                               │
│  Signing (amort):    $10,000                                                       │
│  ─────────────────────────────                                                     │
│  Annualized Total:   $300,000                                                      │
│                                                                                    │
│ [s] Save    [e] Evaluate vs. Market    [Esc] Cancel                               │
└────────────────────────────────────────────────────────────────────────────────────┘
```

Implementation notes:
- The "Live Breakdown" panel updates on every keystroke using `SalaryIntelligenceService::compute_total_comp` (pure sync, called in the render function — no async).
- Field focus cycles with `Tab`/`Shift+Tab`. The form uses a `Vec<FormField>` with `focused_index: usize`.
- Monetary inputs accept natural language: `220000`, `220k`, `$220k`, `$220,000` — normalized on `Enter` or focus-leave via `parse_dollar_amount()`.
- `[e] Evaluate vs. Market` dispatches `AppAction::EvaluateOffer(offer_details)` to the async event handler, which calls `evaluate_offer()` and transitions to the comparison view.

**File:** `lazyjob-tui/src/views/salary/comparison.rs`

```
┌──── Offer Comparison ────────────────────────────────────────────────────────────┐
│                   Acme Corp SE    Google L5 (competing)   Market Median          │
│ Base              $220,000        $240,000                 $215,000               │
│ Bonus             $ 30,000        $ 36,000                 $ 28,000               │
│ Equity (risk-adj) $ 40,000        $ 62,500                 —                      │
│ Signing (amort)   $ 10,000        —                        —                      │
│ ─────────────────────────────────────────────────────────────────────────────────│
│ TOTAL             $300,000 ▲      $338,500 ▲               $243,000               │
│ vs. market        +23.5%          +39.3%                   —                      │
│                                                                                   │
│ Negotiation Signals:                                                              │
│  • Competing offer is $38.5k/yr higher — disclose to negotiate up.               │
│  • Equity risk factor is 0.40 (mid-stage private). Review liquidation terms.     │
│                                                                                   │
│ H1B LCA data: 127 records for "Software Engineer" in CA (FY2024)                 │
│ [r] Add Reference Point   [p] Paste levels.fyi Data   [Esc] Back                 │
└───────────────────────────────────────────────────────────────────────────────────┘
```

Uses ratatui `Table` widget with `WidgetRef` for column-aligned money formatting. Numbers formatted with `format_cents_k(cents)` helper: `$300k`, `$300,000` for values ≥ $10k.

---

## Key Crate APIs

- `sqlx::query!("SELECT...", ...).fetch_all(&pool).await` — compile-time checked queries
- `sqlx::SqlitePool::begin().await?` → `Transaction<'_, Sqlite>` for batch inserts
- `calamine::open_workbook_auto(path)` → `Xlsx<_>` for DOL XLSX parsing; `ws.rows()` for row iteration
- `tokio::task::spawn_blocking(|| { /* calamine parsing */ }).await?` — wraps sync XLSX parser
- `reqwest::Client::get(url).send().await?.bytes().await?` — download LCA ZIP to `Vec<u8>`
- `zip::ZipArchive::new(std::io::Cursor::new(bytes))` → extract XLSX from ZIP
- `once_cell::sync::Lazy<HashSet<&'static str>>` — zero-cost jurisdiction lookup
- `uuid::Uuid::new_v4()` — generate offer/market data IDs
- `chrono::Utc::now().date_naive()` — as_of_date for levels.fyi paste
- `serde_json::to_string(&equity_grant)` / `serde_json::from_str::<EquityGrant>(&row.equity_json)` — equity round-trip
- `ratatui::widgets::Table::new(rows, widths)` — comparison table rendering
- `ratatui::widgets::Clear` — erase background before modal offer form

## Error Handling

```rust
// lazyjob-core/src/salary/error.rs

#[derive(thiserror::Error, Debug)]
pub enum SalaryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("H1B LCA import failed: {0}")]
    LcaImport(String),

    #[error("LCA download failed: {0}")]
    Download(#[from] reqwest::Error),

    #[error("LCA XLSX parse error: {0}")]
    XlsxParse(String),

    #[error("levels.fyi paste contained no parseable rows")]
    LevelsFyiNoData,

    #[error("levels.fyi paste parse error: {reason}")]
    LevelsFyiParse { reason: String },

    #[error("offer not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SalaryError>;
```

`SalaryError::LcaImport` is non-fatal for individual rows — the importer collects per-row errors into `ImportReport::errors` (a `Vec<String>`) and continues. Only a catastrophic failure (download failure, unreadable XLSX) returns `Err(SalaryError::*)`.

## Testing Strategy

### Unit Tests — `lazyjob-core/src/salary/tc_calc.rs`

All combinations of the computation matrix must be covered:

| Test | CompanyStage | EquityType | Scenario |
|------|-------------|------------|----------|
| `test_rsu_public_no_bonus` | Public | RSU | baseline |
| `test_rsu_late_private_bonus_pct` | LatePrivate | RSU | bonus via pct |
| `test_iso_in_the_money` | MidPrivate | ISO | FMV > strike |
| `test_option_underwater` | EarlyPrivate | ISO | strike > FMV → 0 equity |
| `test_signing_amortized_2yr` | Public | RSU | vest=1yr → amort=1yr |
| `test_risk_override` | MidPrivate | RSU | user sets 0.8 override |
| `test_no_equity` | Public | — | no equity field |
| `test_bonus_priority` | Public | RSU | fixed bonus takes precedence over pct |

### Unit Tests — `LevelsFyiParser`

```rust
#[test]
fn test_parse_two_rows_k_suffix() { ... }
#[test]
fn test_parse_full_dollar() { ... }
#[test]
fn test_parse_underscore_format() { ... }
#[test]
fn test_no_data_rows_returns_error() { ... }
#[test]
fn test_percentile_p25_p50_p75() { ... }
```

### Integration Tests — `SqliteOfferRepository`

Use `#[sqlx::test(migrations = "migrations")]`:

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_save_and_retrieve_offer(pool: SqlitePool) {
    let repo = SqliteOfferRepository::new(pool.clone());
    let offer = make_test_offer();
    repo.save(&offer).await.unwrap();
    let retrieved = repo.get(&offer.id).await.unwrap().unwrap();
    assert_eq!(retrieved.base_annual_cents, offer.base_annual_cents);
    // Verify equity round-trip.
    assert_eq!(
        retrieved.equity.as_ref().unwrap().vest_years,
        offer.equity.as_ref().unwrap().vest_years
    );
}

#[sqlx::test(migrations = "migrations")]
async fn test_list_for_application_returns_multiple(pool: SqlitePool) { ... }

#[sqlx::test(migrations = "migrations")]
async fn test_fts_market_data_search(pool: SqlitePool) {
    // Insert a point for "software engineer" in CA, then search for "Software Engineer" — must match.
}
```

### Integration Tests — `H1bLcaImporter`

Use `wiremock` to serve a minimal synthetic XLSX with known wage values, verify `ImportReport::roles_discovered == expected` and the correct p50 is stored.

### TUI Tests — `OfferEntryForm`

Drive with synthetic key events and verify:
- `Tab` moves focus between fields.
- `$220k` input normalizes to `220_000_00` on `Enter`.
- Live breakdown updates after each Base field change.
- Empty equity fields produce zero equity in breakdown (no panic on `None`).

## Open Questions

1. **H1B LCA data bias disclaimer**: LCA data over-represents companies that sponsor H1B visas (large tech, finance, consulting). It significantly underrepresents: startup roles, companies that only hire domestic engineers, and roles below L3/L4 equivalent. A prominent disclaimer in the TUI is planned — exact wording TBD. Should the UI show this disclaimer once on first import, or on every `evaluate_offer()` call?

2. **Private equity complexity**: The current risk factor table (0.15–1.0 per stage) is a simplification. A more accurate model would incorporate expected IPO timeline, liquidation preference stack, and dilution rate from future funding rounds. Should Phase 5 add optional advanced fields (IPO timeline estimate in years, liquidation preference multiple) to `EquityGrant` that unlock a DCF-style calculation? Or is simplicity the right trade-off for MVP?

3. **Offer expiration alerts**: `offer_details.expires_at` is stored, but the existing `ReminderPoller` (from `application-workflow-actions-implementation-plan.md`) is not yet wired to query this table. Adding a `list_expiring_offers(horizon_hours: u32)` method to `OfferRepository` and querying it in `ReminderPoller::tick()` is low-effort. Should this be included in Phase 1 of this plan, or delegated to the pipeline metrics plan?

4. **Competing offer privacy**: Some users may be uncomfortable persisting a competing offer's details (paper trail for "do you have other offers?" conversations). A `session_only: bool` flag on `OfferDetails` that bypasses `offer_repo.save()` would address this, but adds UX complexity. Is this needed for MVP or deferred?

5. **levels.fyi paste format stability**: The parser is built against the current visible table structure. levels.fyi regularly redesigns their UI and the clipboard output format changes. Should the parser use a fuzzy column-header detection strategy (find column containing "Base", "Total", etc. regardless of position) rather than positional parsing? The spec's paste-based approach assumes the format is stable enough for the MVP.

## Related Specs

- [salary-negotiation-offers.md](salary-negotiation-offers.md) — builds directly on `OfferDetails` and `TotalCompBreakdown`
- [salary-counter-offer-drafting.md](salary-counter-offer-drafting.md) — uses `OfferEvaluation` as negotiation context for LLM drafting
- [application-state-machine-implementation-plan.md](application-state-machine-implementation-plan.md) — `ApplicationId` FK, `Offer` stage transition
- [application-workflow-actions-implementation-plan.md](application-workflow-actions-implementation-plan.md) — `PostTransitionSuggestion::RunSalaryComparison` dispatch
- [job-search-ghost-job-detection-implementation-plan.md](job-search-ghost-job-detection-implementation-plan.md) — shares `PAY_TRANSPARENT_JURISDICTIONS` constant
- [specs/12-15-interview-salary-networking-notifications-implementation-plan.md](12-15-interview-salary-networking-notifications-implementation-plan.md) — original combined plan covering salary at a coarser level
