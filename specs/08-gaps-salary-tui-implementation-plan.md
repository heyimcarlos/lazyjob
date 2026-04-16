# Implementation Plan: Salary / TUI Gap Closure

## Status
Draft

## Related Spec
[specs/08-gaps-salary-tui.md](08-gaps-salary-tui.md)

## Overview

This plan closes all 10 identified gaps (GAP-78 through GAP-87) and 2 cross-spec concerns
(Cross-Spec R: TUI State ↔ Application State, Cross-Spec S: Salary Privacy ↔ SaaS Sync) in
the salary subsystem and TUI layer. The gap spec spans two areas — the TUI design/keybindings
spec and the three salary specs — so this plan organises work accordingly.

Three gaps are Critical: TUI accessibility (GAP-78), vim mode (GAP-79), and clipboard
integration (GAP-80). Together they define the full user interaction contract of the terminal
UI. Two are Important: startup equity valuation (GAP-81) and offer letter parsing (GAP-82).
Five are Moderate and addressed in Phase 3. The two cross-spec gaps define data-consistency
contracts that must be respected by all four phases.

Implementation phases: Phase 1 (Critical TUI gaps — accessibility, vim mode, clipboard),
Phase 2 (Important salary gaps — equity valuation, offer parsing), Phase 3 (Moderate gaps —
benefits, i18n, notifications, mouse, negotiation rounds), Phase 4 (Cross-spec contracts).

## Prerequisites

### Must be implemented first
- `specs/09-tui-design-keybindings-implementation-plan.md` — `App`, `EventLoop`, `KeyCombo`, `KeyContext`, `Action`, ratatui widget hierarchy
- `specs/salary-market-intelligence-implementation-plan.md` — `TotalCompBreakdown`, `EquityGrant`, `OfferEvaluation`, `SqliteMarketDataRepository`
- `specs/salary-negotiation-offers-implementation-plan.md` — `OfferRecord`, `NegotiationRecord`, `NegotiationStatus`
- `specs/salary-counter-offer-drafting-implementation-plan.md` — `NegotiationHistory`, `CounterOfferLoop`, `NegotiationRound`
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `StageTransitionEvent`, `SqliteApplicationRepository`
- `specs/04-sqlite-persistence-implementation-plan.md` — `run_migrations`, `DbPool`
- `specs/16-privacy-security-implementation-plan.md` — `PrivacyMode`, `SecurityLayer`
- `specs/18-saas-migration-path-implementation-plan.md` — `FeatureFlags`, `sync_outbox` table

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# Phase 1 — TUI accessibility + vim mode + clipboard
arboard           = "3.4"         # System clipboard read/write (X11, Wayland, macOS)
crossterm         = "0.28"        # mouse events, cursor shape (already present, enable feature flags)

# Phase 2 — offer letter parsing
pdf-extract       = "0.7"         # PDF text extraction (pure Rust)
# Alternative if pdf-extract proves insufficient:
# lopdf           = "0.33"

# Phase 3 — benefits, i18n, notifications
notify-rust       = { version = "4.10", features = ["z-notify"] }  # desktop notifications (already present)
rust_decimal      = "1.34"        # multi-currency / decimal arithmetic without float errors
iso_currency      = "0.4"         # ISO 4217 currency code enum

# Already present (verify):
once_cell         = "1.19"
regex             = "1.10"
strsim            = "0.11"
serde             = { version = "1", features = ["derive"] }
serde_json        = "1"
sqlx              = { version = "0.8", features = ["sqlite", "runtime-tokio", "chrono", "uuid"] }
thiserror         = "1"
anyhow            = "1"
tokio             = { version = "1", features = ["full"] }
tracing           = "0.1"
chrono            = { version = "0.4", features = ["serde"] }
uuid              = { version = "1", features = ["v4", "serde"] }
```

---

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `AccessibilityConfig` + theme palette | `lazyjob-tui` | `src/accessibility/mod.rs` |
| `VimMode` state machine | `lazyjob-tui` | `src/vim/mod.rs` |
| `VimCommandBuffer` + motion engine | `lazyjob-tui` | `src/vim/commands.rs` |
| `Clipboard` wrapper (arboard) | `lazyjob-tui` | `src/clipboard.rs` |
| `MouseHandler` | `lazyjob-tui` | `src/mouse.rs` |
| `ToastQueue` in-TUI notifications | `lazyjob-tui` | `src/notifications/toast.rs` |
| `TuiNotificationPoller` | `lazyjob-tui` | `src/notifications/poller.rs` |
| `UiStateStore` (Cross-Spec R) | `lazyjob-tui` | `src/state_store.rs` |
| `EquityValuationService` (Black-Scholes) | `lazyjob-core` | `src/salary/equity_valuation.rs` |
| `OfferLetterParser` | `lazyjob-core` | `src/salary/offer_letter.rs` |
| `BenefitsValuationService` | `lazyjob-core` | `src/salary/benefits.rs` |
| `CurrencyConverter` | `lazyjob-core` | `src/salary/currency.rs` |
| `NegotiationRoundTracker` | `lazyjob-core` | `src/salary/negotiation_rounds.rs` |
| `SalaryPrivacyFilter` (Cross-Spec S) | `lazyjob-core` | `src/salary/privacy_filter.rs` |
| SQLite migrations (022–026) | `lazyjob-core` | `migrations/022_*` … `026_*` |
| TUI accessibility settings view | `lazyjob-tui` | `src/views/settings/accessibility.rs` |
| TUI offer letter import view | `lazyjob-tui` | `src/views/salary/offer_letter_import.rs` |
| TUI benefits entry form | `lazyjob-tui` | `src/views/salary/benefits_form.rs` |

---

### Core Types

```rust
// lazyjob-tui/src/accessibility/mod.rs

/// Configures runtime appearance and behaviour accommodations.
/// Stored in ~/.config/lazyjob/config.toml under [accessibility].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessibilityConfig {
    pub color_mode: ColorMode,
    pub focus_indicator_style: FocusIndicatorStyle,
    pub reduced_motion: bool,
    /// Each tick the TUI scrolls by this many lines (default 3).
    pub scroll_speed: u8,
    pub mouse_enabled: bool,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::Dark,
            focus_indicator_style: FocusIndicatorStyle::Border,
            reduced_motion: false,
            scroll_speed: 3,
            mouse_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    /// Default dark theme.
    Dark,
    /// High-contrast dark (WCAG AA ratios, no low-opacity text).
    HighContrastDark,
    /// High-contrast light (for bright displays).
    HighContrastLight,
    /// Deuteranopia safe (avoids red/green distinguishers).
    ColorBlindDeuteranopia,
    /// Tritanopia safe (avoids blue/yellow distinguishers).
    ColorBlindTritanopia,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusIndicatorStyle {
    /// Bold coloured border around focused widget (default).
    Border,
    /// Underline the focused row/item.
    Underline,
    /// Reverse video (swap fg/bg) on focused item.
    ReverseVideo,
}

/// A fully resolved palette of ratatui `Color` values for one `ColorMode`.
#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub background:           ratatui::style::Color,
    pub foreground:           ratatui::style::Color,
    pub sidebar_bg:           ratatui::style::Color,
    pub border_focused:       ratatui::style::Color,
    pub border_unfocused:     ratatui::style::Color,
    pub status_applied:       ratatui::style::Color,
    pub status_rejected:      ratatui::style::Color,
    pub status_offer:         ratatui::style::Color,
    pub ghost_badge:          ratatui::style::Color,
    pub warning:              ratatui::style::Color,
    pub success:              ratatui::style::Color,
    pub selection_bg:         ratatui::style::Color,
    pub selection_fg:         ratatui::style::Color,
}
```

```rust
// lazyjob-tui/src/vim/mod.rs

/// Modal editing mode for the TUI.
/// The mode drives both rendering (cursor shape, status bar label) and key dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Normal,
    /// Active when cursor is inside an editable widget (text input, textarea).
    Insert,
    /// Active after pressing `v`; accumulates a character range.
    Visual { anchor: usize },
    /// Active after pressing `:` in Normal mode; reads an ex-command line.
    Command,
    /// Operator pending: user typed `d`, `c`, `y` — waiting for motion.
    OperatorPending { operator: VimOperator },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimOperator {
    Delete,
    Change,
    Yank,
}

/// Cross-widget command buffer that accumulates keystrokes in Normal mode.
/// e.g. `3j` — count=3, motion=Down; `dd` — operator+line.
#[derive(Debug, Default)]
pub struct VimCommandBuffer {
    count: Option<u16>,
    pending_keys: Vec<crossterm::event::KeyEvent>,
}

impl VimCommandBuffer {
    /// Feed one keypress; returns the resolved command if complete, None if still accumulating.
    pub fn feed(&mut self, key: crossterm::event::KeyEvent) -> Option<VimCommand> {
        // ... resolved below in implementation
        todo!()
    }

    pub fn clear(&mut self) {
        self.count = None;
        self.pending_keys.clear();
    }
}

/// A fully resolved vim command, ready to execute against the focused widget.
#[derive(Debug, Clone)]
pub enum VimCommand {
    Motion(VimMotion, u16),          // motion + count (1 if not specified)
    Operator(VimOperator, VimMotion, u16),
    EnterInsert(InsertEntry),
    EnterVisual,
    ExitToNormal,
    DeleteLine(u16),                  // `dd` * count
    YankLine(u16),                    // `yy` * count
    Paste(PasteTarget),
    Undo,
    Redo,
    EnterCommand,                     // `:`
    Repeat,                           // `.`
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimMotion {
    Up, Down, Left, Right,
    WordForward,                      // w
    WordBackward,                     // b
    EndOfWord,                        // e
    LineStart,                        // 0 or ^
    LineEnd,                          // $
    DocumentStart,                    // gg
    DocumentEnd,                      // G
    NextChar(char),                   // f<char>
    PrevChar(char),                   // F<char>
    ParagraphForward,                 // }
    ParagraphBackward,                // {
    HalfPageDown,                     // Ctrl-d
    HalfPageUp,                       // Ctrl-u
    SearchNext,                       // n
    SearchPrev,                       // N
}

#[derive(Debug, Clone)]
pub enum InsertEntry {
    Before,     // i
    After,      // a
    LineAbove,  // O
    LineBelow,  // o
    Append,     // A
}

#[derive(Debug, Clone)]
pub enum PasteTarget {
    After,   // p
    Before,  // P
}
```

```rust
// lazyjob-tui/src/clipboard.rs

/// Thin wrapper around `arboard::Clipboard` with graceful degradation.
/// arboard panics on WSL/headless environments; we wrap errors instead.
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Clipboard {
    /// Returns a `Clipboard` instance. If arboard cannot open a display
    /// connection (headless, WSL without X), `inner` is `None` and all
    /// operations silently no-op.
    pub fn try_new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }

    pub fn get_text(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        self.inner
            .as_mut()
            .and_then(|c| c.set_text(text.into()).ok())
            .is_some()
    }
}
```

```rust
// lazyjob-tui/src/mouse.rs

/// Wraps `crossterm::event::MouseEvent` and routes to widget-level handlers.
pub struct MouseHandler {
    /// Widget hit regions registered each render pass (cleared + rebuilt on each frame).
    regions: Vec<MouseRegion>,
}

/// A named rectangular region associated with a dispatched action.
#[derive(Debug, Clone)]
pub struct MouseRegion {
    pub area: ratatui::layout::Rect,
    pub action: MouseAction,
}

#[derive(Debug, Clone)]
pub enum MouseAction {
    /// Focus and select the item at list index.
    SelectListItem { view_id: ViewId, index: usize },
    /// Scroll the content pane up/down by N rows.
    Scroll { view_id: ViewId, delta: i16 },
    /// Activate a button.
    Button(crate::actions::Action),
}

/// Stable view identifier used in mouse region routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    JobsFeed,
    ApplicationKanban,
    ContactList,
    SalaryOfferList,
    Settings,
    RalphPanel,
}
```

```rust
// lazyjob-tui/src/notifications/toast.rs

/// A short-lived message displayed in the TUI status bar area.
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: uuid::Uuid,
    pub level: ToastLevel,
    pub message: String,
    /// When this toast should be removed from the queue.
    pub expires_at: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// FIFO queue of toasts. Capacity-capped at 5 — oldest are dropped.
pub struct ToastQueue {
    inner: std::collections::VecDeque<Toast>,
}

impl ToastQueue {
    pub const MAX_CAPACITY: usize = 5;
    pub const DEFAULT_TTL_MS: u64 = 4_000;

    pub fn push(&mut self, level: ToastLevel, message: impl Into<String>) {
        let toast = Toast {
            id: uuid::Uuid::new_v4(),
            level,
            message: message.into(),
            expires_at: std::time::Instant::now()
                + std::time::Duration::from_millis(Self::DEFAULT_TTL_MS),
        };
        if self.inner.len() >= Self::MAX_CAPACITY {
            self.inner.pop_front();
        }
        self.inner.push_back(toast);
    }

    /// Remove expired toasts. Called once per frame before rendering.
    pub fn tick(&mut self) {
        let now = std::time::Instant::now();
        self.inner.retain(|t| t.expires_at > now);
    }

    pub fn current(&self) -> Option<&Toast> {
        self.inner.front()
    }
}
```

```rust
// lazyjob-tui/src/state_store.rs — Cross-Spec R

/// Persisted TUI session state (view selection, scroll offsets, last active pane).
/// Written to ~/.config/lazyjob/tui_state.json on clean exit.
/// Never stored in SQLite; a missing file resets cleanly to defaults.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TuiSessionState {
    pub last_view: Option<ViewId>,
    pub jobs_feed_scroll: u16,
    pub application_selected_id: Option<uuid::Uuid>,
    pub contacts_scroll: u16,
    pub salary_selected_offer_id: Option<uuid::Uuid>,
}

/// In-memory cache of application-level entities that the TUI holds
/// for the current render. Invalidated by `StageTransitionEvent` (via
/// tokio::sync::broadcast), re-fetched from SQLite on the next tick.
pub struct UiStateStore {
    pub session: TuiSessionState,
    /// Receive application changes published by `SqliteApplicationRepository`.
    stage_rx: tokio::sync::broadcast::Receiver<crate::application::StageTransitionEvent>,
    /// Set to `true` when a `StageTransitionEvent` is received; cleared after re-fetch.
    pub applications_stale: bool,
}

impl UiStateStore {
    pub fn new(
        session: TuiSessionState,
        stage_rx: tokio::sync::broadcast::Receiver<crate::application::StageTransitionEvent>,
    ) -> Self {
        Self {
            session,
            stage_rx,
            applications_stale: false,
        }
    }

    /// Poll (non-blocking) for stage change events from the application repository.
    /// Sets `applications_stale = true` if any event is received.
    pub fn poll_events(&mut self) {
        while let Ok(_event) = self.stage_rx.try_recv() {
            self.applications_stale = true;
        }
    }
}
```

```rust
// lazyjob-core/src/salary/equity_valuation.rs — GAP-81

/// Black-Scholes call option price in microdollars.
/// All monetary inputs are integer cents; output is cents.
/// Requires: S (stock price cents), K (strike price cents),
///           t (years to expiry f64), r (risk-free rate f64), sigma (volatility f64)
pub fn black_scholes_call(
    stock_price_cents: i64,
    strike_price_cents: i64,
    years_to_expiry: f64,
    risk_free_rate: f64,
    volatility: f64,
) -> i64 {
    use std::f64::consts::E;
    let s = stock_price_cents as f64;
    let k = strike_price_cents as f64;
    let t = years_to_expiry;
    let r = risk_free_rate;
    let sigma = volatility;

    if t <= 0.0 || sigma <= 0.0 {
        // Options at or past expiry, or zero volatility: intrinsic value only
        return (s - k).max(0.0) as i64;
    }

    let d1 = ((s / k).ln() + (r + 0.5 * sigma * sigma) * t) / (sigma * t.sqrt());
    let d2 = d1 - sigma * t.sqrt();

    let nd1 = standard_normal_cdf(d1);
    let nd2 = standard_normal_cdf(d2);

    let call = s * nd1 - k * E.powf(-r * t) * nd2;
    call.round() as i64
}

/// Hart approximation for N(x). Error < 7.5e-8.
fn standard_normal_cdf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.2316419 * x.abs());
    let poly = t * (0.319381530
        + t * (-0.356563782
            + t * (1.781477937
                + t * (-1.821255978
                    + t * 1.330274429))));
    let pdf = (-x * x / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
    if x >= 0.0 {
        1.0 - pdf * poly
    } else {
        pdf * poly
    }
}

/// Inputs required to value a stock option grant.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OptionValuationInputs {
    pub shares: i64,
    /// Current 409A FMV in cents.
    pub current_fmv_cents: i64,
    /// Strike price (exercise price) in cents.
    pub strike_price_cents: i64,
    /// Vesting cliff in months.
    pub cliff_months: u8,
    /// Total vest period in months.
    pub vest_total_months: u8,
    /// Expected annualized volatility (0.3 = 30%).
    pub expected_volatility: f64,
    /// Risk-free rate (use current 10-year treasury yield, e.g. 0.045).
    pub risk_free_rate: f64,
    /// Liquidity preference multiplier applied to intrinsic value (0.0–1.0).
    /// PublicOrLate = 1.0, MidPrivate = 0.65, EarlyPrivate = 0.25
    pub liquidity_discount: f64,
}

/// Present value of the total option package.
#[derive(Debug, Clone)]
pub struct OptionValuationResult {
    pub black_scholes_per_share_cents: i64,
    pub total_grant_value_cents: i64,
    pub total_grant_value_with_liquidity_cents: i64,
    pub intrinsic_value_cents: i64,
    pub time_value_cents: i64,
    /// Warning to surface when inputs are highly uncertain.
    pub uncertainty_warning: Option<String>,
}

pub fn value_option_grant(inputs: &OptionValuationInputs) -> OptionValuationResult {
    let years = inputs.vest_total_months as f64 / 12.0;
    let bs = black_scholes_call(
        inputs.current_fmv_cents,
        inputs.strike_price_cents,
        years,
        inputs.risk_free_rate,
        inputs.expected_volatility,
    );
    let intrinsic = (inputs.current_fmv_cents - inputs.strike_price_cents).max(0);
    let total = bs * inputs.shares;
    let total_with_discount = (total as f64 * inputs.liquidity_discount).round() as i64;
    let uncertainty_warning = if inputs.expected_volatility > 0.6 {
        Some("High assumed volatility (>60%). Estimate range is very wide.".to_string())
    } else if inputs.liquidity_discount < 0.3 {
        Some("Early-stage company. Option value is highly speculative.".to_string())
    } else {
        None
    };
    OptionValuationResult {
        black_scholes_per_share_cents: bs,
        total_grant_value_cents: total,
        total_grant_value_with_liquidity_cents: total_with_discount,
        intrinsic_value_cents: intrinsic * inputs.shares,
        time_value_cents: (bs - intrinsic) * inputs.shares,
        uncertainty_warning,
    }
}
```

```rust
// lazyjob-core/src/salary/offer_letter.rs — GAP-82

/// Plain-text representation extracted from an offer letter document.
#[derive(Debug)]
pub struct ExtractedText {
    pub text: String,
    pub source_format: OfferLetterFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferLetterFormat {
    Pdf,
    PlainText,
}

/// Extracts plain text from an offer letter PDF.
pub fn extract_pdf_text(pdf_bytes: &[u8]) -> Result<ExtractedText, OfferLetterError> {
    let text = pdf_extract::extract_text_from_mem(pdf_bytes)
        .map_err(|e| OfferLetterError::PdfExtraction(e.to_string()))?;
    Ok(ExtractedText {
        text,
        source_format: OfferLetterFormat::Pdf,
    })
}

/// LLM-parsed fields from offer letter text. All fields optional since
/// the parser may not find every field in every format.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ParsedOfferFields {
    pub base_salary_cents: Option<i64>,
    pub signing_bonus_cents: Option<i64>,
    pub target_bonus_pct: Option<u8>,
    pub equity_shares: Option<i64>,
    pub equity_grant_type: Option<String>,      // "RSU", "Options", etc.
    pub cliff_months: Option<u8>,
    pub vest_total_months: Option<u8>,
    pub start_date: Option<chrono::NaiveDate>,
    pub job_title: Option<String>,
    pub company_name: Option<String>,
    /// Which fields the parser is uncertain about.
    pub uncertain_fields: Vec<String>,
}

/// Sends extracted text to LLM and returns a parsed `ParsedOfferFields` JSON.
/// Callers must confirm all fields with the user before saving to SQLite.
pub async fn parse_offer_fields_via_llm(
    text: &str,
    llm: &dyn lazyjob_llm::LlmProvider,
) -> Result<ParsedOfferFields, OfferLetterError> {
    let prompt = build_offer_parse_prompt(text);
    let response = llm
        .complete(&prompt)
        .await
        .map_err(|e| OfferLetterError::LlmError(e.to_string()))?;
    serde_json::from_str::<ParsedOfferFields>(&response)
        .map_err(|e| OfferLetterError::ParseError(e.to_string()))
}

fn build_offer_parse_prompt(text: &str) -> String {
    format!(
        r#"Extract offer letter fields from the following text. \
Return ONLY a JSON object matching this schema. \
For any field you cannot find or are uncertain about, omit it and add its name to "uncertain_fields".

Schema:
{{
  "base_salary_cents": integer | null,
  "signing_bonus_cents": integer | null,
  "target_bonus_pct": integer | null,
  "equity_shares": integer | null,
  "equity_grant_type": "RSU" | "Options" | null,
  "cliff_months": integer | null,
  "vest_total_months": integer | null,
  "start_date": "YYYY-MM-DD" | null,
  "job_title": string | null,
  "company_name": string | null,
  "uncertain_fields": [string]
}}

Offer letter text:
---
{text}
---"#
    )
}

#[derive(thiserror::Error, Debug)]
pub enum OfferLetterError {
    #[error("PDF extraction failed: {0}")]
    PdfExtraction(String),
    #[error("LLM call failed: {0}")]
    LlmError(String),
    #[error("Offer field parse failed: {0}")]
    ParseError(String),
    #[error("Unsupported document format")]
    UnsupportedFormat,
}
```

```rust
// lazyjob-core/src/salary/benefits.rs — GAP-83

/// Structured benefits from one offer. All monetary values in cents/year.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OfferBenefits {
    pub offer_id: uuid::Uuid,
    /// Annual health premium the company pays (for chosen plan tier).
    pub employer_health_premium_cents: Option<i64>,
    /// Annual 401k match in cents (e.g. 50% of first 6% = 3% of salary).
    pub employer_401k_match_cents: Option<i64>,
    /// PTO days per year.
    pub pto_days: Option<u16>,
    /// Paid parental leave weeks.
    pub parental_leave_weeks: Option<u16>,
    /// Gym/wellness stipend in cents/year.
    pub wellness_stipend_cents: Option<i64>,
    /// Remote work allowance in cents/year (internet, equipment).
    pub remote_stipend_cents: Option<i64>,
    /// Free-form notes (e.g. "unlimited PTO", "company-paid dental/vision").
    pub notes: Option<String>,
}

/// Estimated total benefits value in cents/year.
pub fn estimate_benefits_value(b: &OfferBenefits) -> i64 {
    b.employer_health_premium_cents.unwrap_or(0)
        + b.employer_401k_match_cents.unwrap_or(0)
        + b.wellness_stipend_cents.unwrap_or(0)
        + b.remote_stipend_cents.unwrap_or(0)
}

/// Extended total comp including benefits.
pub fn total_comp_with_benefits_cents(
    base_comp_cents: i64,
    benefits: &OfferBenefits,
) -> i64 {
    base_comp_cents + estimate_benefits_value(benefits)
}
```

```rust
// lazyjob-core/src/salary/currency.rs — GAP-84

/// ISO 4217 currency code. Stored as TEXT in SQLite (3-char code).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CurrencyCode(pub String);

impl CurrencyCode {
    pub fn usd() -> Self { Self("USD".to_string()) }
    pub fn eur() -> Self { Self("EUR".to_string()) }
}

/// Cached exchange rates. Refreshed daily via config toggle.
/// Stored in `exchange_rates` SQLite table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExchangeRate {
    pub from: CurrencyCode,
    pub to: CurrencyCode,
    pub rate: rust_decimal::Decimal,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

pub struct CurrencyConverter {
    rates: std::collections::HashMap<(String, String), rust_decimal::Decimal>,
}

impl CurrencyConverter {
    pub fn new(rates: Vec<ExchangeRate>) -> Self {
        let map = rates
            .into_iter()
            .map(|r| ((r.from.0, r.to.0), r.rate))
            .collect();
        Self { rates: map }
    }

    /// Convert `amount_cents` from `from` currency to `to` currency.
    /// Returns `None` if no rate is available.
    pub fn convert(&self, amount_cents: i64, from: &str, to: &str) -> Option<i64> {
        if from == to {
            return Some(amount_cents);
        }
        let rate = self.rates.get(&(from.to_string(), to.to_string()))?;
        let result = rust_decimal::Decimal::from(amount_cents) * rate;
        Some(result.round().try_into().unwrap_or(i64::MAX))
    }
}
```

```rust
// lazyjob-core/src/salary/negotiation_rounds.rs — GAP-87

/// Tracks negotiation round count per application. Emits warnings at thresholds.
pub struct NegotiationRoundTracker;

impl NegotiationRoundTracker {
    /// Threshold at which a yellow warning banner should appear.
    pub const CAUTION_THRESHOLD: u8 = 2;
    /// Threshold at which a red warning appears and user must confirm to continue.
    pub const DANGER_THRESHOLD: u8 = 3;

    pub fn assess_round(current_round: u8) -> RoundRiskLevel {
        match current_round {
            0..=1 => RoundRiskLevel::Normal,
            2 => RoundRiskLevel::Caution,
            _ => RoundRiskLevel::Danger,
        }
    }

    pub fn warning_message(current_round: u8) -> Option<&'static str> {
        match current_round {
            2 => Some("Round 2 of negotiation. Research shows 3+ rounds can signal hesitancy. Proceed thoughtfully."),
            r if r >= 3 => Some("Round 3+. Continuing to negotiate at this stage risks damaging the relationship. Confirm to proceed."),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundRiskLevel {
    Normal,
    Caution,  // Yellow banner, dismissable
    Danger,   // Red banner, requires explicit confirmation
}
```

```rust
// lazyjob-core/src/salary/privacy_filter.rs — Cross-Spec S

/// Determines which salary fields are safe to include in `sync_outbox`.
/// `NegotiationHistory` and `OfferBenefits` are NEVER synced.
/// `market_data_references` (public aggregates) ARE safe to sync.
#[derive(Debug, Clone)]
pub struct SalaryPrivacyFilter {
    pub privacy_mode: crate::security::PrivacyMode,
}

impl SalaryPrivacyFilter {
    /// Returns `true` if the given salary table is safe to include in SaaS sync.
    pub fn is_table_syncable(&self, table: SalaryTable) -> bool {
        match (table, &self.privacy_mode) {
            // These contain personal offer details — never sync
            (SalaryTable::Offers, _) => false,
            (SalaryTable::NegotiationHistory, _) => false,
            (SalaryTable::OfferBenefits, _) => false,
            // Market data is public aggregates — safe except in Stealth mode
            (SalaryTable::MarketDataCache, crate::security::PrivacyMode::Stealth) => false,
            (SalaryTable::MarketDataCache, _) => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SalaryTable {
    Offers,
    NegotiationHistory,
    OfferBenefits,
    MarketDataCache,
}
```

---

### SQLite Schema

```sql
-- Migration 022: benefits valuation
CREATE TABLE IF NOT EXISTS offer_benefits (
    id            TEXT PRIMARY KEY,                    -- UUID
    offer_id      TEXT NOT NULL REFERENCES offers(id) ON DELETE CASCADE,
    employer_health_premium_cents  INTEGER,
    employer_401k_match_cents      INTEGER,
    pto_days                       INTEGER,
    parental_leave_weeks           INTEGER,
    wellness_stipend_cents         INTEGER,
    remote_stipend_cents           INTEGER,
    notes                          TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_offer_benefits_offer_id ON offer_benefits(offer_id);

-- Migration 023: currency exchange rates cache
CREATE TABLE IF NOT EXISTS exchange_rates (
    from_currency TEXT NOT NULL,
    to_currency   TEXT NOT NULL,
    rate          TEXT NOT NULL,    -- stored as string for rust_decimal precision
    fetched_at    TEXT NOT NULL,
    PRIMARY KEY (from_currency, to_currency)
);

-- Migration 024: option valuation inputs (for Black-Scholes)
CREATE TABLE IF NOT EXISTS option_valuation_inputs (
    id                    TEXT PRIMARY KEY,
    offer_id              TEXT NOT NULL REFERENCES offers(id) ON DELETE CASCADE,
    shares                INTEGER NOT NULL,
    current_fmv_cents     INTEGER NOT NULL,
    strike_price_cents    INTEGER NOT NULL,
    cliff_months          INTEGER NOT NULL DEFAULT 12,
    vest_total_months     INTEGER NOT NULL DEFAULT 48,
    expected_volatility   REAL NOT NULL DEFAULT 0.40,
    risk_free_rate        REAL NOT NULL DEFAULT 0.045,
    liquidity_discount    REAL NOT NULL DEFAULT 1.0,
    created_at            TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at            TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Migration 025: negotiation round tracking
-- Adds round_number column to existing negotiation_rounds table (if exists).
-- If negotiation_rounds doesn't exist, created by salary-negotiation plan.
-- This migration is idempotent via column existence check.
-- (Handled via ALTER TABLE in migration script; round_number is already
--  tracked in NegotiationHistory from the counter-offer-drafting plan.)

-- Migration 026: TUI session state (stored as JSON blob)
CREATE TABLE IF NOT EXISTS tui_session_state (
    id          INTEGER PRIMARY KEY CHECK (id = 1),   -- singleton row
    state_json  TEXT NOT NULL,
    saved_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
```

---

### Module Structure

```
lazyjob-tui/
  src/
    accessibility/
      mod.rs           # AccessibilityConfig, ColorMode, ColorPalette
      palette.rs       # ColorPalette::for_mode() palette lookup table
    vim/
      mod.rs           # VimMode, VimCommandBuffer, VimCommand
      commands.rs      # VimCommandBuffer::feed() implementation
      motions.rs       # VimMotion handlers for text widgets
      ex_commands.rs   # ExCommandParser (`:q`, `:w`, custom commands)
    clipboard.rs       # Clipboard wrapper (arboard)
    mouse.rs           # MouseHandler, MouseRegion, MouseAction
    state_store.rs     # UiStateStore, TuiSessionState (Cross-Spec R)
    notifications/
      toast.rs         # Toast, ToastQueue
      poller.rs        # TuiNotificationPoller (background tokio task)
    views/
      settings/
        accessibility.rs   # TUI settings panel for AccessibilityConfig
      salary/
        offer_letter_import.rs  # Import flow for PDF offer letters
        benefits_form.rs        # Form for entering benefits breakdown
        option_valuation.rs     # Black-Scholes input form + result display
        currency_selector.rs    # Currency picker for international offers

lazyjob-core/
  src/
    salary/
      equity_valuation.rs    # black_scholes_call, OptionValuationInputs, value_option_grant
      offer_letter.rs        # extract_pdf_text, parse_offer_fields_via_llm
      benefits.rs            # OfferBenefits, estimate_benefits_value
      currency.rs            # CurrencyCode, ExchangeRate, CurrencyConverter
      negotiation_rounds.rs  # NegotiationRoundTracker, RoundRiskLevel
      privacy_filter.rs      # SalaryPrivacyFilter, SalaryTable (Cross-Spec S)
  migrations/
    022_offer_benefits.sql
    023_exchange_rates.sql
    024_option_valuation_inputs.sql
    025_negotiation_rounds.sql
    026_tui_session_state.sql
```

---

## Implementation Phases

### Phase 1 — Critical TUI Gaps (MVP)

#### Step 1.1 — Accessibility config and color palettes (GAP-78)

**File**: `lazyjob-tui/src/accessibility/palette.rs`

Implement `ColorPalette::for_mode(mode: &ColorMode) -> ColorPalette`. Define all five palette variants using hardcoded `ratatui::style::Color::Rgb(r, g, b)` values. The `HighContrastDark` palette must pass WCAG AA for 4.5:1 contrast ratio on all text/background pairs. The `ColorBlindDeuteranopia` palette replaces all red/green status distinguishers with blue/orange pairs.

**File**: `lazyjob-tui/src/accessibility/mod.rs`

Add `AccessibilityConfig::load(config_dir: &Path) -> Self` that reads from `config.toml` `[accessibility]` table via `config::Config`. On parse failure, fall back to `Default::default()`.

**Verification**: Unit test each `ColorPalette` for key contrast ratios. Integration test: write an `AccessibilityConfig` to a temp TOML file, load it, assert `color_mode == HighContrastDark`.

#### Step 1.2 — Focus indicators

**File**: `lazyjob-tui/src/lib.rs` (or `app.rs`)

The `EventLoop::render()` method currently renders borders with a uniform style. Change to: `let border_style = if widget_is_focused { palette.border_focused } else { palette.border_unfocused }`. For `FocusIndicatorStyle::ReverseVideo`, apply `Style::default().reversed()` to the focused row in `List` and `Table` widgets. For `FocusIndicatorStyle::Underline`, apply `Modifier::UNDERLINED`.

**Verification**: TUI smoke test: toggle `FocusIndicatorStyle` variants and visually verify focus is clearly visible on each widget.

#### Step 1.3 — Reduced motion

`App` holds a bool `reduced_motion: bool` loaded from `AccessibilityConfig`. Pass it to all scroll and fade transition helpers. When `true`, all `smooth_scroll()` helpers skip intermediate frames and jump directly to the target position.

#### Step 1.4 — Vim mode state machine (GAP-79)

**File**: `lazyjob-tui/src/vim/commands.rs`

Implement `VimCommandBuffer::feed()`:
1. If the buffered key is a digit (0-9) and no operator is pending: accumulate into `count`.
2. Match against the normal-mode command table (a static `HashMap<&str, VimCommand>` built once with `once_cell::sync::Lazy`).
3. Operator pending (`d`, `c`, `y`): wait for a motion key; on receipt, return `VimCommand::Operator(op, motion, count)`.
4. Double-key commands (`dd`, `yy`): detect by checking `pending_keys` length.

The command table must include all motion keys documented in GAP-79: `hjkl`, `w`, `b`, `e`, `0`, `$`, `gg`, `G`, `f<char>`, `F<char>`, `{}`, `Ctrl-d`, `Ctrl-u`, `n`, `N`, `i`, `a`, `o`, `O`, `A`, `v`, `:`, `.`, `u`, `Ctrl-r`, `p`, `P`.

**File**: `lazyjob-tui/src/vim/motions.rs`

`apply_motion(motion: &VimMotion, count: u16, cursor: &mut CursorState, content: &str)` — implement each `VimMotion` variant as a pure function over the cursor position and content string.

**File**: `lazyjob-tui/src/app.rs` (EventLoop key dispatch)

Modify `EventLoop::handle_key_event()`:
1. If current `VimMode` is `Insert`: send key to the focused widget's text input handler.
2. If `Normal` or `OperatorPending`: feed key to `VimCommandBuffer::feed()`. On `Some(cmd)`, dispatch `cmd` to the focused widget or global action handler.
3. If `Command`: send key to the ex-command line widget.

**Status bar rendering**: Display current mode in the status bar: `NORMAL`, `INSERT`, `VISUAL`, `COMMAND`. Change cursor shape via `crossterm::cursor::SetCursorStyle::BlinkingBlock` (Normal/Visual) vs `crossterm::cursor::SetCursorStyle::BlinkingBar` (Insert).

**Verification**: Unit tests for `VimCommandBuffer::feed()` covering: digit accumulation, `3j`, `dd`, `gg`, `dw`, `ciw`, `yy`, mode transitions. Integration test: launch TUI in test harness, inject `gg` key sequence, assert cursor moves to document start.

#### Step 1.5 — Ex command parser (GAP-79 continued)

**File**: `lazyjob-tui/src/vim/ex_commands.rs`

`ExCommandParser::parse(input: &str) -> Option<ExCommand>` — match against:
- `:q` → `ExCommand::Quit`
- `:w` → `ExCommand::Save`
- `:wq` → `ExCommand::SaveAndQuit`
- `:/pattern` → `ExCommand::Search(pattern)`
- `:noh` → `ExCommand::ClearSearch`
- Any unknown command → `ExCommand::Unknown(input.to_string())`

`ExCommand::Unknown` is surfaced as a toast error, not a panic.

#### Step 1.6 — Clipboard integration (GAP-80)

**File**: `lazyjob-tui/src/clipboard.rs`

Implement `Clipboard::try_new()`, `get_text()`, `set_text()` as defined in Core Types above.

**Integration in vim layer**: `VimCommand::Paste(_)` → call `App.clipboard.get_text()` and insert at cursor position in the focused widget. `VimCommand::YankLine(_)` → call `App.clipboard.set_text(line_text)`.

**Smart copy actions**: In `JobsFeedView`, pressing `yy` yanks the job URL (not raw text). In `CoverLetterEditorPanel`, `yy` yanks the current paragraph. In any view, `Ctrl-c` (non-vim) yanks the focused item's primary field.

**Paste-into-input-fields**: Text input widgets (search bar, notes fields) already accept `Insert` mode keystrokes. When `VimMode::Insert` is active, `Ctrl-v` calls `clipboard.get_text()` and inserts at the widget's cursor.

**Verification**: Unit test `Clipboard::try_new()` when arboard is unavailable (mock by feature flag). Integration test: yank a job URL, assert clipboard contains `"https://..."`.

#### Step 1.7 — Visual mode selection

**File**: `lazyjob-tui/src/vim/mod.rs`

`VimMode::Visual { anchor }` tracks the anchor character offset. Each motion command during Visual mode extends/contracts the selection range `[min(anchor, cursor), max(anchor, cursor)]`. Selected text is highlighted with `palette.selection_bg` / `palette.selection_fg`. `y` in Visual mode yanks the selected range to the clipboard; `d` deletes it (only in editable widgets — read-only views block `d` and show an error toast).

**Verification**: Visual mode unit test: enter visual mode at offset 5, press `w` three times, assert selection range is `[5, 5+3*word_len]`.

---

### Phase 2 — Important Salary Gaps

#### Step 2.1 — Black-Scholes option valuation (GAP-81)

**File**: `lazyjob-core/src/salary/equity_valuation.rs`

Implement `black_scholes_call()` and `value_option_grant()` exactly as specified in Core Types. The standard normal CDF uses the Hart approximation (no external math crate needed).

**TUI form** (`lazyjob-tui/src/views/salary/option_valuation.rs`):
- Multi-field form: shares, current_fmv (dollar input), strike_price (dollar input), vest_total_months (dropdown: 24/36/48/60), expected_volatility (slider 10%–90%), liquidity_discount (dropdown by company stage, maps to Public=1.0/LatePrivate=0.7/MidPrivate=0.4/EarlyPrivate=0.25).
- On change: call `value_option_grant()` synchronously and update the result panel in real time.
- Result panel shows: per-share BS value, total grant value, with-liquidity value, uncertainty_warning in yellow if present.
- `Tab` to advance fields; `Enter` to save to `option_valuation_inputs`.

**Verification**: Unit test `black_scholes_call(10000, 10000, 4.0, 0.045, 0.4)` (ATM option) — result should be roughly 38–42% of spot price. Test `black_scholes_call(10000, 20000, 0.1, 0.045, 0.4)` (deep OTM near expiry) — result should be near 0.

#### Step 2.2 — Offer letter PDF parsing (GAP-82)

**File**: `lazyjob-core/src/salary/offer_letter.rs`

Implement `extract_pdf_text()` using `pdf_extract::extract_text_from_mem()`. Implement `parse_offer_fields_via_llm()` with the prompt in Core Types. On `serde_json::from_str` failure: retry once by appending `"\n\nPlease output ONLY valid JSON."` to the assistant turn.

**TUI import flow** (`lazyjob-tui/src/views/salary/offer_letter_import.rs`):
1. Step 1 — file picker: press `o` in the offer list to open a path input dialog. User types or pastes the path to their PDF.
2. Step 2 — extraction: `tokio::task::spawn_blocking(|| extract_pdf_text(bytes))`. Show a spinner toast `"Reading offer letter..."`.
3. Step 3 — LLM parsing: show `"Parsing fields..."` toast. Send to LLM.
4. Step 4 — review form: pre-populate `OfferRecord` form fields with parsed values. Uncertain fields are highlighted in yellow. User must tab through all fields to confirm before saving.
5. Step 5 — save: on `Enter` from the last field, write `OfferRecord` to SQLite.

Privacy note: Offer letter text is sent to the configured LLM provider. Show a `Privacy: offer letter text will be sent to [provider_name]` warning in step 3.

**Verification**: Unit test `build_offer_parse_prompt()` — assert it contains "base_salary_cents". Integration test: mock LLM returning valid JSON; assert `ParsedOfferFields::base_salary_cents == Some(150_000_00)`.

---

### Phase 3 — Moderate Gaps

#### Step 3.1 — Benefits valuation comparison (GAP-83)

**File**: `lazyjob-core/src/salary/benefits.rs`

Implement `OfferBenefits`, `estimate_benefits_value()`, `total_comp_with_benefits_cents()`.

Apply migration 022.

**Repository** (`lazyjob-core/src/salary/benefits_repository.rs`):
```rust
pub async fn upsert_benefits(
    pool: &sqlx::SqlitePool,
    benefits: &OfferBenefits,
) -> sqlx::Result<()>;

pub async fn find_by_offer(
    pool: &sqlx::SqlitePool,
    offer_id: uuid::Uuid,
) -> sqlx::Result<Option<OfferBenefits>>;
```

**TUI form** (`lazyjob-tui/src/views/salary/benefits_form.rs`): Triggered by pressing `b` on a selected offer in `OfferComparisonView`. Dollar-amount fields accept `$200k`/`200,000`/`200` formats (reuse the `parse_dollar_amount()` function from the salary-negotiation plan). After save, `OfferComparisonView` recalculates and re-renders `total_comp_with_benefits_cents` for all offers side-by-side.

**Verification**: Unit test `estimate_benefits_value()` with a fully populated `OfferBenefits`. Integration test: save benefits, re-query `total_comp_with_benefits_cents`, assert it equals `base_comp + benefits_value`.

#### Step 3.2 — Currency conversion (GAP-84)

**File**: `lazyjob-core/src/salary/currency.rs`

Implement `CurrencyConverter` as defined in Core Types. Exchange rates are fetched from the free [exchangerate.host](https://exchangerate.host) JSON API (no key required) via `reqwest` once daily. Store in `exchange_rates` SQLite table (migration 023). `CurrencyConverter::new(rates)` is a pure constructor enabling unit tests without HTTP.

**OfferRecord changes**: Add `currency: CurrencyCode` field (default `USD`). When a non-USD currency is selected in the offer form, the offer's `TotalCompBreakdown` is computed in the native currency, and a "Converted to USD" row is shown in the comparison table using the cached rate.

**Verification**: Unit test `CurrencyConverter::convert(100_00, "EUR", "USD")` with a seeded rate of 1.08 → asserts result = 108_00. Test `convert()` when no rate exists for the pair → returns `None`.

#### Step 3.3 — In-TUI notifications and OS notifications (GAP-85)

**File**: `lazyjob-tui/src/notifications/toast.rs`

Implement `ToastQueue` as defined. Add `ToastQueue` to `App` struct. After each `EventLoop` tick, call `toasts.tick()` then render the frontmost toast as a styled `Paragraph` in the bottom-right corner of the layout using `ratatui::widgets::Clear` beneath it.

**File**: `lazyjob-tui/src/notifications/poller.rs`

`TuiNotificationPoller` is a background `tokio::task` that:
1. Queries `application_pipeline_metrics::list_action_items()` every 60 seconds.
2. For any new `ActionItem::ExpiringOffer` or `ActionItem::UpcomingInterview` not seen before (tracked by `HashSet<uuid::Uuid>` in memory), sends a `tokio::sync::mpsc::Sender<ToastPayload>` message to the `App`.
3. On OS notification delivery, calls `notify_rust::Notification::new().summary("LazyJob").body(&msg).show()`. OS notification is attempted first; if `show()` fails (headless), falls back to `ToastQueue` only.

**Notification preferences** in `config.toml` `[notifications]`:
```toml
[notifications]
os_notifications = true
quiet_hours_start = "22:00"   # Optional; format "HH:MM"
quiet_hours_end   = "08:00"   # Optional
```

`TuiNotificationPoller` checks quiet hours before calling `notify_rust`. Uses `chrono::Local::now().time()` for the check.

**Verification**: Unit test `ToastQueue::push()` capacity cap (6th push drops the oldest). Integration test: mock `list_action_items()` returning an `ExpiringOffer`; assert a `ToastPayload::Warning` message is sent on the mpsc channel.

#### Step 3.4 — Mouse support (GAP-86)

**File**: `lazyjob-tui/src/mouse.rs`

Implement `MouseHandler::register_region()` and `MouseHandler::dispatch()`:

```rust
impl MouseHandler {
    pub fn register_region(&mut self, region: MouseRegion) {
        self.regions.push(region);
    }

    pub fn clear_regions(&mut self) {
        self.regions.clear();
    }

    /// Called on `crossterm::event::MouseEvent`. Returns dispatched action if found.
    pub fn dispatch(
        &self,
        event: crossterm::event::MouseEvent,
    ) -> Option<&MouseAction> {
        use crossterm::event::MouseEventKind;
        let (col, row) = (event.column, event.row);
        match event.kind {
            MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.regions
                    .iter()
                    .find(|r| {
                        col >= r.area.x
                            && col < r.area.x + r.area.width
                            && row >= r.area.y
                            && row < r.area.y + r.area.height
                    })
                    .map(|r| &r.action)
            }
            _ => None,
        }
    }
}
```

Enable mouse capture via `crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)` in `EventLoop::init()`, and `DisableMouseCapture` in `EventLoop::restore_terminal()`.

Each view registers its mouse regions during `render()`. Scroll: `MouseEventKind::ScrollDown` maps to a `Scroll { delta: -3 }` action; `ScrollUp` to `delta: 3`. Click-to-select: `MouseEventKind::Down(MouseButton::Left)` computes which list index the row corresponds to and dispatches `SelectListItem`.

Mouse support is gated by `AccessibilityConfig.mouse_enabled`. When `false`, mouse events are ignored (not registered via crossterm) to prevent accidental clicks.

**Verification**: Unit test `MouseHandler::dispatch()` with a mock region spanning rows 5–10, col 0–20 — clicking row 7 col 5 dispatches correctly; clicking row 11 returns `None`.

#### Step 3.5 — Negotiation round warning (GAP-87)

**File**: `lazyjob-core/src/salary/negotiation_rounds.rs`

Implement `NegotiationRoundTracker::assess_round()` and `warning_message()` as defined.

**Integration in `CounterOfferDraftService::draft()`** (`lazyjob-core/src/salary/counter_offer.rs`):
Before calling the LLM, compute `current_round = negotiation_history.rounds.len()`. Call `assess_round(current_round)`. If `Danger`, return `Err(CounterOfferError::ExcessiveRoundsWarning { round: current_round, message })` — a non-fatal error that the TUI presents as a confirmation dialog. The caller must pass `bypass_round_warning: true` in `CounterOfferOptions` to retry.

**TUI handling**: `CounterOfferError::ExcessiveRoundsWarning` triggers a modal confirmation dialog with the warning text + `[Cancel]` / `[Continue Anyway]` buttons.

**Verification**: Unit test `assess_round(0..=1) == Normal`, `assess_round(2) == Caution`, `assess_round(5) == Danger`. Integration test: call `CounterOfferDraftService::draft()` with a history of 3 rounds; assert `ExcessiveRoundsWarning` is returned before the LLM is called.

---

### Phase 4 — Cross-Spec Contracts

#### Step 4.1 — TUI State ↔ Application State (Cross-Spec R)

**File**: `lazyjob-tui/src/state_store.rs`

Implement `UiStateStore::poll_events()` as defined. Integrate into `EventLoop::tick()`:

```rust
// In EventLoop::tick()
self.state_store.poll_events();
if self.state_store.applications_stale {
    self.applications = self.app_repo.list_all().await?;
    self.state_store.applications_stale = false;
}
```

Session persistence: On `EventLoop::cleanup()` (called from `Drop`), serialise `state_store.session` to `~/.config/lazyjob/tui_state.json` via `serde_json::to_writer`. On startup, load via `serde_json::from_reader`. Missing file → `TuiSessionState::default()`.

**Note**: `TuiSessionState` is also written to the `tui_session_state` SQLite table (migration 026) as a backup for crash recovery, with an `ON CONFLICT(id) DO UPDATE` upsert. The JSON file is authoritative; the SQLite copy is secondary.

**Verification**: Unit test `UiStateStore::poll_events()` with a mock `broadcast::Receiver` that has a pending `StageTransitionEvent` — assert `applications_stale == true` after the call.

#### Step 4.2 — Salary Privacy ↔ SaaS Sync (Cross-Spec S)

**File**: `lazyjob-core/src/salary/privacy_filter.rs`

Implement `SalaryPrivacyFilter::is_table_syncable()` as defined.

**Integration in `lazyjob-sync`** (from the SaaS migration plan's `sync_outbox` writer):

In `OutboxWriter::record_change(table: &str, ...)`, check:
```rust
let salary_table = SalaryTable::from_str(table);
if let Some(t) = salary_table {
    if !SalaryPrivacyFilter { privacy_mode }.is_table_syncable(t) {
        return Ok(()); // skip silently
    }
}
```

Write a `#[test]` that constructs a `SalaryPrivacyFilter { privacy_mode: PrivacyMode::Full }` and asserts `is_table_syncable(NegotiationHistory) == false` and `is_table_syncable(MarketDataCache) == true`.

**Verification**: Integration test: write an `OfferRecord` to SQLite; assert no row is inserted in `sync_outbox` for the `offers` table.

---

## Key Crate APIs

```
// Phase 1
arboard::Clipboard::new() -> Result<Clipboard, arboard::Error>
arboard::Clipboard::get_text(&mut self) -> Result<String, arboard::Error>
arboard::Clipboard::set_text(&mut self, text: impl Into<String>) -> Result<(), arboard::Error>

crossterm::event::EnableMouseCapture          // crossterm::execute!()
crossterm::event::DisableMouseCapture
crossterm::event::MouseEvent { kind, column, row, modifiers }
crossterm::event::MouseEventKind::{Down, ScrollDown, ScrollUp}
crossterm::cursor::SetCursorStyle::BlinkingBlock
crossterm::cursor::SetCursorStyle::BlinkingBar

ratatui::style::Color::Rgb(u8, u8, u8)        // per-palette colours
ratatui::style::Style::default().reversed()   // ReverseVideo focus indicator
ratatui::style::Modifier::UNDERLINED          // Underline focus indicator
ratatui::widgets::Clear                        // erase background under modal/toast
ratatui::layout::Rect                          // mouse hit-test region

// Phase 2
pdf_extract::extract_text_from_mem(bytes: &[u8]) -> Result<String, PdfError>

// Phase 3
notify_rust::Notification::new()
    .summary("LazyJob")
    .body(&message)
    .show() -> Result<NotificationHandle, notify_rust::Error>

rust_decimal::Decimal::from(i64)
rust_decimal::Decimal::round()

// Cross-cutting
once_cell::sync::Lazy<HashMap<...>>            // VimCommandBuffer command table
tokio::sync::broadcast::Sender<StageTransitionEvent>::subscribe()
tokio::sync::mpsc::Sender<ToastPayload>
tokio::task::spawn_blocking(|| ...)            // PDF extraction on thread pool
sqlx::query!(...).execute(pool)                // migrations
```

---

## Error Handling

```rust
// lazyjob-tui/src/errors.rs

#[derive(thiserror::Error, Debug)]
pub enum TuiError {
    #[error("Clipboard unavailable: {0}")]
    ClipboardUnavailable(String),

    #[error("Mouse event dispatch failed")]
    MouseDispatchFailed,

    #[error("Session state load failed: {0}")]
    SessionStateLoad(#[from] std::io::Error),

    #[error("Session state parse failed: {0}")]
    SessionStateParse(#[from] serde_json::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

// lazyjob-core/src/salary/errors.rs (additions)

#[derive(thiserror::Error, Debug)]
pub enum SalaryGapError {
    #[error("PDF extraction failed: {0}")]
    PdfExtraction(String),

    #[error("Offer letter LLM parse failed: {0}")]
    OfferLetterParse(String),

    #[error("Currency conversion unavailable for {from}->{to}")]
    CurrencyConversionUnavailable { from: String, to: String },

    #[error("Exchange rate fetch failed: {0}")]
    ExchangeRateFetch(String),

    #[error("Excessive negotiation rounds (round {round}): {message}")]
    ExcessiveRoundsWarning { round: u8, message: &'static str },

    #[error(transparent)]
    Db(#[from] sqlx::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

---

## Testing Strategy

### Unit Tests

**Accessibility**:
- `ColorPalette::for_mode(HighContrastDark)` — assert `foreground` and `background` contrast ratio ≥ 4.5:1 (use inline luma calculation).
- `ColorMode::ColorBlindDeuteranopia` — assert no `status_applied` and `status_rejected` share the same red/green hue range.

**Vim mode**:
- `VimCommandBuffer::feed()` for all documented commands; test count accumulation (`3j`), operator pending (`dw`, `dd`), mode transitions (`i`, `v`, `:`).
- `apply_motion(VimMotion::WordForward, 2, ...)` over `"hello world foo"` from offset 0 — assert cursor lands at offset 12.
- `ExCommandParser::parse(":q")` → `ExCommand::Quit`.

**Clipboard**:
- `Clipboard::try_new()` when arboard errors → `inner = None`; subsequent `set_text()` returns `false` without panicking.

**Mouse**:
- `MouseHandler::dispatch()` — hit test inside/outside/edge of registered region.

**Equity valuation**:
- `black_scholes_call(10000, 10000, 4.0, 0.045, 0.4)` — result ∈ [3_500, 4_500].
- `black_scholes_call(10000, 10000, 0.0, 0.045, 0.4)` — result = 0 (at expiry).
- `value_option_grant()` with `liquidity_discount = 0.25` — assert `total_grant_value_with_liquidity_cents < total_grant_value_cents`.

**Benefits**:
- `estimate_benefits_value()` with all fields `Some(1000)` and 5 fields → result = 5000.
- `total_comp_with_benefits_cents()` — addition of base and benefits.

**Currency**:
- `CurrencyConverter::convert(100_00, "USD", "USD")` → `Some(100_00)` (identity).
- `CurrencyConverter::convert(100_00, "EUR", "GBP")` with seeded EUR→USD and USD→GBP rates → correct chained conversion.
- Missing rate → `None`.

**Negotiation rounds**:
- `assess_round(0)` → `Normal`, `assess_round(2)` → `Caution`, `assess_round(3)` → `Danger`.
- `warning_message(1)` → `None`, `warning_message(2)` → `Some(...)`, `warning_message(5)` → `Some(...)`.

**Offer letter**:
- `build_offer_parse_prompt("Annual salary: $150,000")` — assert prompt contains `"base_salary_cents"`.
- Mock LLM returning `{"base_salary_cents": 15000000, ...}` → `ParsedOfferFields::base_salary_cents == Some(15000000)`.
- Mock LLM returning malformed JSON → `OfferLetterError::ParseError`.

**Privacy filter**:
- `SalaryPrivacyFilter { Full }.is_table_syncable(Offers)` → `false`.
- `SalaryPrivacyFilter { Stealth }.is_table_syncable(MarketDataCache)` → `false`.
- `SalaryPrivacyFilter { Full }.is_table_syncable(MarketDataCache)` → `true`.

### Integration Tests

- **Vim + clipboard end-to-end**: Use `crossterm::event::KeyEvent` injection to simulate `gg`, `dw`, `yy`, `p` in a text widget; assert clipboard and buffer state match expected.
- **Mouse click-to-select**: Register a 10-row list region, inject a `MouseEvent::Down` at row 7, assert `SelectListItem { index: 2 }` is dispatched (accounting for header row offset).
- **Toast lifecycle**: Push 6 toasts (exceeds MAX_CAPACITY), assert queue length = 5. Advance time past `DEFAULT_TTL_MS`, call `tick()`, assert queue is empty.
- **Offer letter parse + save**: Load a test PDF bytes fixture, call `extract_pdf_text()`, mock LLM, assert `ParsedOfferFields` fields. Submit the confirmation form, assert `offers` row is written with correct `base_salary_cents`.
- **Benefits comparison**: Save two `OfferBenefits` records for two offers; call `total_comp_with_benefits_cents` for each; assert ordering matches the expected ranked order in the comparison view.
- **Negotiation round warning in `CounterOfferDraftService`**: Set up `NegotiationHistory` with 3 completed rounds; call `draft()`; assert `ExcessiveRoundsWarning` is returned before the mock LLM is called (verify LLM mock was NOT invoked).
- **TUI state invalidation (Cross-Spec R)**: Broadcast a `StageTransitionEvent` on the broadcast channel; call `UiStateStore::poll_events()`; assert `applications_stale == true`.

### TUI Visual Tests

Use `ratatui::backend::TestBackend` with `ratatui::Terminal::new(TestBackend::new(80, 24))` to render frames and assert `buffer.get(col, row).symbol` matches expected chars/styles:

- Render the vim status bar with `VimMode::Insert`; assert bottom-left shows `INSERT` in green.
- Render `ToastQueue` with one `ToastLevel::Warning` message; assert the toast cell at the expected coordinates shows the message text.
- Render `FocusIndicatorStyle::Border` on two adjacent panels; assert the focused panel's border cell uses `palette.border_focused` color.
- Render `FocusIndicatorStyle::ReverseVideo` on a list; assert the selected row has `modifier = Modifier::REVERSED`.

---

## Open Questions

1. **arboard on Wayland**: `arboard` requires Wayland clipboard access via wl-clipboard. In environments without a display server (e.g., pure SSH), clipboard ops silently fail. Should LazyJob warn the user at startup if no clipboard backend is detected?

2. **VimCommand::Repeat (`.`)**: The last modifying command must be persisted across mode transitions. This requires a `last_command: Option<VimCommand>` field in `App`. Is `.` scoped to the current widget, or global across all views?

3. **Exchange rate source**: `exchangerate.host` is free but has rate limits and occasional downtime. Should LazyJob fall back to cached rates (possibly stale) on fetch failure, or surface an error in the currency conversion UI? Recommendation: always fall back to cached; show a timestamp of last successful fetch.

4. **PDF extraction quality**: `pdf-extract` works well for text-based PDFs but fails on scanned/image PDFs. Should LazyJob detect this case (empty extracted text) and surface a "manual entry required" message? Or attempt OCR via `tesseract` bindings? Recommendation: detect empty text, require manual entry — avoid the `tesseract` system dependency.

5. **Black-Scholes for illiquid assets**: The model assumes a liquid, tradeable asset with known volatility. For early-stage private companies, volatility is unknown and the model may produce misleading false precision. The `uncertainty_warning` field addresses this partially — should there be a harder block that prevents showing BS results for `CompanyStage::EarlyPrivate` without an explicit user acknowledgement?

6. **Benefits PTO valuation**: Converting PTO days to dollars (e.g., `pto_days * daily_rate`) introduces a contentious assumed hourly rate. Should PTO be displayed as a standalone "days" metric rather than monetized? Recommendation: display as days, not dollars — avoid the monetization debate.

7. **Cross-Spec R session state SQLite vs file**: The plan stores `TuiSessionState` both in a JSON file and in SQLite. The JSON file is authoritative. Should the SQLite copy be removed to reduce complexity, or retained as a crash-recovery mechanism?

8. **Mouse drag for kanban**: `crossterm` mouse events include `MouseEventKind::Drag` but it's unreliable across terminals. Implementing kanban drag would require tracking `MouseEventKind::Down`, `Drag`, and `Up` across frames. Recommendation: defer drag to Phase 5; use `m` (move) key + direction keys for kanban card movement instead.

---

## Related Specs

- [specs/09-tui-design-keybindings.md](09-tui-design-keybindings.md) — base TUI spec
- [specs/09-tui-design-keybindings-implementation-plan.md](09-tui-design-keybindings-implementation-plan.md)
- [specs/salary-market-intelligence.md](salary-market-intelligence.md)
- [specs/salary-market-intelligence-implementation-plan.md](salary-market-intelligence-implementation-plan.md)
- [specs/salary-negotiation-offers.md](salary-negotiation-offers.md)
- [specs/salary-negotiation-offers-implementation-plan.md](salary-negotiation-offers-implementation-plan.md)
- [specs/salary-counter-offer-drafting.md](salary-counter-offer-drafting.md)
- [specs/salary-counter-offer-drafting-implementation-plan.md](salary-counter-offer-drafting-implementation-plan.md)
- [specs/application-state-machine-implementation-plan.md](application-state-machine-implementation-plan.md)
- [specs/18-saas-migration-path-implementation-plan.md](18-saas-migration-path-implementation-plan.md)
- [specs/16-privacy-security-implementation-plan.md](16-privacy-security-implementation-plan.md)
- [specs/XX-tui-vim-mode.md](XX-tui-vim-mode.md) — new spec created by gap analysis
- [specs/XX-tui-accessibility.md](XX-tui-accessibility.md) — new spec created by gap analysis
