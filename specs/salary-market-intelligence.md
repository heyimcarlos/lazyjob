# Spec: Salary Market Intelligence

**JTBD**: A-6 — Negotiate the best possible compensation offer
**Topic**: Compute a candidate's personalized market compensation range for a specific role, level, and location by aggregating available public data and calculating total comp across all components.
**Domain**: salary-negotiation

---

## What

`SalaryIntelligenceService` answers: "What is this offer actually worth, and how does it compare to market?" It takes a full offer (base, equity, signing, bonus, benefits) entered by the user, computes an annualized total comp value, and compares it against aggregated market data from public sources (levels.fyi data, H1B LCA public filings, user-provided reference points). The output is a structured `OfferEvaluation`: gap vs. market, per-component breakdown, equity risk-adjusted value, and whether the offer has negotiation room.

## Why

Research from Payscale (31,000 respondents): 40–50% of candidates who negotiate receive a better offer; negotiation yields 5–15% base salary increase on average. Yet most candidates either don't negotiate (leaving money on table) or negotiate only on base salary, ignoring 20–40% of their actual compensation (equity, signing, bonus). The core problem: candidates can't evaluate multi-component offers accurately. They compare base salaries when they should compare annualized total comp with risk-adjusted equity. No existing tool handles this for private-company equity. LazyJob solves this by giving users a structured calculator integrated with their application record.

## How

**Data access reality check (privacy-first):**

No major salary data source has a public API. levels.fyi, Glassdoor, Blind, and Payscale are all web-only. This spec does NOT propose scraping any of these platforms — it would violate ToS and be brittle. Instead:

**Phase 1 — User-provided + H1B LCA data:**
- User manually enters offer details (base, equity, signing, bonus, vest schedule, company stage)
- H1B LCA disclosure data (public, ~1M records/year from DOL) is the primary market data source — downloadable CSV, queryable offline with SQLite FTS, updated annually
- User can enter reference comp points from their own network ("friend at Google L5 makes $X") — stored in `salary_references` table, private to the user
- User can paste raw levels.fyi table data as text — system parses and caches it

**Phase 2 — Levels.fyi HTML import:**
The user visits levels.fyi in their browser, copies the salary table for a role, and pastes it into LazyJob. A structured parser (`LevelsFyiParser`) extracts the data. No scraping, no automation — user action required. This follows the same clipboard-import pattern as the LinkedIn CSV import in the networking domain.

**Total comp calculation:**

```
Annualized Total Comp = base_annual
    + bonus_annual  (base * bonus_pct OR fixed bonus / vest_years)
    + equity_annual (grant_total / vest_years * risk_factor)
    + signing_amortized (signing / MIN(2, vest_years))
```

**Equity risk factors by company stage:**
- Public company: `risk_factor = 1.0` (liquid at vesting)
- Late private (Series D+, >$500M valuation): `risk_factor = 0.7` (illiquidity discount)
- Mid private (Series B/C): `risk_factor = 0.4`
- Early private (Seed/A): `risk_factor = 0.15`
- User can override risk factor manually — default is suggestive, not authoritative

**RSU vs. Options distinction:**
- RSUs (public/late private): value = `grant_shares * current_price`
- Options (private): value = `(current_fmv_per_share - strike_price) * grant_options` — requires user to enter 409A FMV and strike. If underwater (strike > FMV), intrinsic value = 0.

**Pay transparency jurisdictions:** Roles in CA, CO, NY, WA, IL, NJ, and MA require salary ranges in job postings. For jobs in these states, the system checks whether the offer's base salary falls within the posted range. The `pay_transparency_jurisdictions` HashSet lives in `lazyjob-core/src/salary/jurisdictions.rs` — the same module referenced by the ghost detection spec's salary-absent signal. This is a shared constant, not duplicated.

**Competing offer comparison:** Users can enter multiple offers. The system sorts by `annualized_total_comp` and highlights per-component differences. Competing offers are the single strongest negotiation lever — the system should make this visible.

**Data storage — privacy requirement:** Offer details are sensitive. They are stored in SQLite (local-only, never synced or exported by default). `offer_details` table has no cloud sync layer — it is explicitly excluded from SaaS sync scope (see `saas-migration-path.md`). User can export to JSON manually for their own records.

**Crate placement:** `SalaryIntelligenceService`, `OfferDetails`, `OfferEvaluation`, and `LevelsFyiParser` live in `lazyjob-core/src/salary/`. The `OfferRepository` trait is already defined in `lazyjob-core/src/application/model.rs` (established in task 5). This salary spec ADDS `save_offer_details` and `get_offers_for_application` to `OfferRepository` — it does NOT create a separate repository.

## Interface

```rust
// lazyjob-core/src/salary/model.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompanyStage {
    Public,
    LatePrivate,   // Series D+, >$500M
    MidPrivate,    // Series B/C
    EarlyPrivate,  // Seed/Series A
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EquityType {
    Rsu,
    Iso,   // Incentive Stock Options
    Nso,   // Non-Qualified Stock Options
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityGrant {
    pub equity_type: EquityType,
    pub total_grant_usd: Option<i64>,    // RSUs: current price * shares
    pub grant_shares: Option<i64>,
    pub vest_years: u8,
    pub cliff_months: u8,                // usually 12
    pub strike_price_cents: Option<i64>, // Options only
    pub fmv_per_share_cents: Option<i64>,// Options only — 409A FMV
    pub acceleration: Option<String>,    // "single trigger", "double trigger"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferDetails {
    pub id: Uuid,
    pub application_id: Uuid,
    pub company_name: String,
    pub job_title: String,
    pub location: String,
    pub company_stage: CompanyStage,
    pub base_annual_cents: i64,
    pub bonus_annual_cents: Option<i64>,
    pub bonus_pct: Option<f32>,           // alternative to fixed bonus
    pub signing_bonus_cents: Option<i64>,
    pub equity: Option<EquityGrant>,
    pub equity_risk_override: Option<f32>,// user override for risk factor
    pub received_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalCompBreakdown {
    pub base_annual: i64,
    pub bonus_annual: i64,
    pub equity_annual_risk_adjusted: i64,
    pub signing_amortized: i64,
    pub annualized_total: i64,
    pub risk_factor_applied: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDataPoint {
    pub source: MarketDataSource,         // H1bLca, UserProvided, LevelsFyiPaste
    pub role: String,
    pub company: Option<String>,
    pub location: String,
    pub base_p25_cents: i64,
    pub base_p50_cents: i64,
    pub base_p75_cents: i64,
    pub total_comp_p50_cents: Option<i64>,
    pub sample_count: Option<u32>,
    pub as_of_date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketDataSource {
    H1bLca,
    UserProvided,
    LevelsFyiPaste,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferEvaluation {
    pub offer: OfferDetails,
    pub breakdown: TotalCompBreakdown,
    pub market_data: Vec<MarketDataPoint>,
    pub market_p50_total: Option<i64>,    // None if insufficient data
    pub gap_vs_market_pct: Option<f32>,   // positive = offer above market
    pub in_posted_range: Option<bool>,    // None if jurisdiction not pay-transparent
    pub competing_offers: Vec<TotalCompBreakdown>, // for comparison
    pub negotiation_signals: Vec<String>, // e.g. "Offer is 12% below H1B median"
}

pub struct SalaryIntelligenceService {
    market_repo: Arc<dyn MarketDataRepository>,
    offer_repo: Arc<dyn OfferRepository>,
    jurisdictions: &'static HashSet<&'static str>,
}

impl SalaryIntelligenceService {
    pub fn evaluate_offer(&self, offer: &OfferDetails, competing: &[OfferDetails]) -> OfferEvaluation;
    pub fn compute_total_comp(offer: &OfferDetails) -> TotalCompBreakdown;
    fn is_pay_transparent_jurisdiction(location: &str) -> bool;
}

// lazyjob-core/src/salary/jurisdictions.rs
pub static PAY_TRANSPARENT_JURISDICTIONS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    // CA, CO, NY, WA, IL, NJ, MA, MD, RI, HI, NV, WA DC — updated as of 2024
    HashSet::from(["CA", "CO", "NY", "WA", "IL", "NJ", "MA", "MD", "RI", "HI", "NV", "DC"])
});

// lazyjob-core/src/salary/levels_fyi_parser.rs
pub struct LevelsFyiParser;
impl LevelsFyiParser {
    /// Parses pasted levels.fyi table text into MarketDataPoints
    pub fn parse_paste(text: &str) -> Result<Vec<MarketDataPoint>>;
}
```

**SQLite tables:**
```sql
CREATE TABLE offer_details (
    id                  TEXT PRIMARY KEY,
    application_id      TEXT NOT NULL REFERENCES applications(id),
    company_name        TEXT NOT NULL,
    job_title           TEXT NOT NULL,
    location            TEXT NOT NULL,
    company_stage       TEXT NOT NULL,
    base_annual_cents   INTEGER NOT NULL,
    bonus_annual_cents  INTEGER,
    bonus_pct           REAL,
    signing_bonus_cents INTEGER,
    equity_json         TEXT,              -- EquityGrant as JSON
    equity_risk_override REAL,
    received_at         TEXT NOT NULL,
    expires_at          TEXT,
    notes               TEXT,
    -- NOTE: excluded from SaaS cloud sync scope
    CONSTRAINT offer_one_per_stage CHECK(1=1)  -- allow multiple offers per application
);

CREATE TABLE market_data_references (
    id          TEXT PRIMARY KEY,
    source      TEXT NOT NULL,
    role        TEXT NOT NULL,
    company     TEXT,
    location    TEXT NOT NULL,
    base_p25    INTEGER NOT NULL,
    base_p50    INTEGER NOT NULL,
    base_p75    INTEGER NOT NULL,
    total_p50   INTEGER,
    sample_count INTEGER,
    as_of_date  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE salary_references (
    -- User-entered "my friend at X makes Y" reference points
    id          TEXT PRIMARY KEY,
    source_note TEXT NOT NULL,  -- e.g. "Friend at Google L5 2024"
    role        TEXT NOT NULL,
    company     TEXT,
    location    TEXT NOT NULL,
    base_annual INTEGER NOT NULL,
    total_comp  INTEGER,
    as_of_date  TEXT NOT NULL
);
```

## Open Questions

- **H1B LCA data quality**: LCA data is useful for base salary ranges but often covers only sponsored roles (skewed toward certain industries and visa-holders). Should we show a disclaimer that H1B LCA data may not represent the full market, especially for startups that don't sponsor?
- **Private company equity**: The risk factor table (0.15–1.0) is a simplification. Black-Scholes for options or DCF for preferred equity would be more accurate but requires more inputs. What is the right level of complexity — simple multiplier vs. formula with user inputs for expected IPO timeline, liquidation preference, dilution rate?
- **Offer expiration alerts**: Should the system surface "offer expires in 3 days" as a `WorkflowEvent` through the existing reminder system? This is low-effort (just check `expires_at` in the existing `ReminderPoller`) but requires coordinator with `application-workflow-actions.md`.
- **Competing offer tracking privacy**: Users may be sensitive about storing competing offer details (some companies ask "do you have other offers?" verbally; written evidence could be awkward). Should there be a "session-only" mode that computes the comparison but never persists the competing offer details?

## Implementation Tasks

- [ ] Define `OfferDetails`, `EquityGrant`, `EquityType`, `CompanyStage`, `TotalCompBreakdown`, `OfferEvaluation`, `MarketDataPoint` types in `lazyjob-core/src/salary/model.rs`
- [ ] Implement `SalaryIntelligenceService::compute_total_comp` with equity risk-adjustment table and RSU-vs-options distinction — all pure Rust, no LLM
- [ ] Implement `is_pay_transparent_jurisdiction` using `PAY_TRANSPARENT_JURISDICTIONS` static set in `lazyjob-core/src/salary/jurisdictions.rs`; ensure this module is shared with `job-search-ghost-job-detection.md`'s `salary_absent_in_transparency_state` signal — refs: `job-search-ghost-job-detection.md`
- [ ] Implement `LevelsFyiParser::parse_paste` for parsing user-pasted salary table text into `MarketDataPoint` records
- [ ] Create SQLite schema migration for `offer_details`, `market_data_references`, `salary_references` tables; extend `OfferRepository` with `save_offer_details` and `get_offers_for_application` — refs: `application-state-machine.md`
- [ ] Implement H1B LCA data importer (`lazyjob-core/src/salary/h1b_importer.rs`): download annual DOL LCA CSV, parse, upsert into `market_data_references` table; run as a one-time setup step
- [ ] Add TUI offer evaluation view: form for entering offer details, auto-computed `TotalCompBreakdown` displayed inline, competing offers side-by-side comparison panel — refs: `architecture-tui-skeleton.md`
- [ ] Wire `PostTransitionSuggestion::RunSalaryComparison` from `application-workflow-actions.md` to open the offer entry form in the TUI when an application transitions to `Offer` stage
