# Spec: Counter-Offer Drafting

**JTBD**: A-6 — Negotiate the best possible compensation offer
**Topic**: Generate a personalized counter-offer email draft and negotiation talking points grounded in verified market data and the candidate's competing offers.
**Domain**: salary-negotiation

---

## What

`CounterOfferDraftService` generates a counter-offer email draft and phone negotiation talking points for a specific offer. It is grounded in the `OfferEvaluation` from `salary-market-intelligence.md` — it never invents market data or competing offer leverage. The draft is produced by the LLM and surfaced in the TUI for the user to review, edit, and copy. Nothing is sent automatically. After the negotiation concludes, the user records the outcome (accepted, rejected, revised offer) to close the negotiation loop and update the application record.

## Why

40–50% of candidates who negotiate receive a better offer, yet most don't negotiate — the primary barrier is not knowing what to say. A well-structured counter-offer email requires: market data justification, total comp framing (not just base salary), appropriate tone calibration, and avoidance of common failure modes (accepting on the spot, negotiating against yourself, lying about competing offers, making 3+ counter rounds). No existing AI tool generates counter-offer letters — only human coaches. LazyJob gives every user access to this expertise at the moment of offer receipt.

## How

**Strict grounding requirement:** The counter-offer draft is generated from an `OfferEvaluation` struct that was computed from user-entered verified data. The LLM receives:
- The offer's actual annualized total comp (verified calculation)
- The market p50 from real data sources (H1B LCA, levels.fyi paste, user references)
- The gap percentage (offer vs. market, computed, not estimated)
- Competing offers (user-entered, if any)
- The user's stated priority (base vs. equity vs. signing vs. start date)
- The company name and role

The LLM is explicitly NOT given: invented market ranges, made-up comp from the internet, or fabricated competing offers. If market data is insufficient (`market_data.is_empty()` or `sample_count < 3`), the service generates a weaker "principle-based" counter (no data justification, relies on enthusiasm + BATNA framing) and displays a warning.

**Competing offer usage rules:**
- If the user has recorded a competing offer in `offer_details`, the draft WILL reference it by value: "I've received a competing offer of $X total comp"
- If the user has NOT entered a competing offer, the draft MUST NOT invent one or hint that one exists — this is a hard constraint. The prompt instructs the LLM explicitly: "Do not mention a competing offer unless competing_offer_annualized is provided."
- Lying about competing offers is explicitly called out in the research as a failure mode: companies sometimes verify. LazyJob will never draft a fabricated competing offer.

**Tone calibration:** The user selects one of three tones:
- `Professional` — formal, data-driven, suitable for large companies with HR processes
- `Enthusiastic` — warm, signals genuine interest while negotiating, suitable for startups
- `Assertive` — direct, less hedging, appropriate when the offer gap is large (>15%)

**What's negotiable vs. not:** The draft intelligently focuses on negotiable components based on `CompanyStage`:
- Public company: base, RSU grant size, signing, start date
- Private (Series B+): base, option grant size, signing, title/level
- Early startup: base (often tight), equity percentage, cliff timing, advisor shares
- Never suggest negotiating benefits packages at standard companies — it signals misplaced priorities

**Negotiation outcome tracking:** After the user completes the negotiation, they record the outcome in the TUI:
- `Accepted(OfferDetails)` — records the final negotiated offer
- `Rejected` — marks the application as withdrawn/rejected
- `OfferRevised(OfferDetails)` — records a revised offer (creates a new `OfferDetails` linked to the same application with `is_negotiated = true`)
- `Deferred` — user is still in the process

The delta between initial offer and final offer is computed: `NegotiationOutcome.comp_delta`. This data accumulates silently and can later feed a salary intelligence dashboard ("Users who negotiated at Company X typically gained $Y").

**Human-in-the-loop boundary (strict):** The TUI shows the draft with a `[DRAFT - NOT SENT]` header. There is no "send" button in LazyJob — the user copies the text. This is the same principle enforced in networking outreach drafting: agent drafts, human copies and sends. The application workflow state does NOT automatically advance when a counter-offer is generated — the user explicitly marks negotiation as active by entering an offer.

**Crate placement:** `CounterOfferDraftService` lives in `lazyjob-core/src/salary/counter_offer.rs`. The prompt template lives in `lazyjob-llm/src/prompts/salary_negotiation.rs`. `NegotiationOutcome` and `NegotiationHistory` live in `lazyjob-core/src/salary/outcome.rs`.

## Interface

```rust
// lazyjob-core/src/salary/counter_offer.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NegotiationTone {
    Professional,
    Enthusiastic,
    Assertive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NegotiationPriority {
    Base,
    Equity,
    SigningBonus,
    StartDate,
    Title,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterOfferRequest {
    pub offer_evaluation: OfferEvaluation,     // from salary-market-intelligence.md
    pub tone: NegotiationTone,
    pub user_priorities: Vec<NegotiationPriority>, // ordered by importance
    pub target_base_cents: Option<i64>,        // user's target base, if known
    pub target_total_cents: Option<i64>,       // user's target total comp, if known
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterOfferDraft {
    pub id: Uuid,
    pub offer_id: Uuid,
    pub email_subject: String,
    pub email_body: String,                    // full draft email
    pub talking_points: Vec<String>,           // for phone negotiation
    pub negotiation_warnings: Vec<String>,     // e.g. "3+ rounds damages relationship"
    pub data_quality_warning: Option<String>,  // set if market data was insufficient
    pub generated_at: DateTime<Utc>,
    pub tone: NegotiationTone,
}

// lazyjob-core/src/salary/outcome.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NegotiationOutcome {
    Accepted {
        final_offer: OfferDetails,
    },
    Rejected,
    OfferRevised {
        revised_offer: OfferDetails,
        round: u8,  // which negotiation round (1, 2, 3...)
    },
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationHistory {
    pub id: Uuid,
    pub application_id: Uuid,
    pub initial_offer_id: Uuid,
    pub final_offer_id: Option<Uuid>,
    pub rounds: Vec<NegotiationRound>,
    pub outcome: Option<NegotiationOutcome>,
    pub initial_annualized_cents: i64,        // denormalized for delta calc
    pub final_annualized_cents: Option<i64>,
    pub comp_delta_cents: Option<i64>,        // final - initial
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationRound {
    pub round_number: u8,
    pub draft_id: Uuid,
    pub outcome: NegotiationOutcome,
    pub recorded_at: DateTime<Utc>,
}

pub struct CounterOfferDraftService {
    llm: Arc<dyn LlmProvider>,
    offer_repo: Arc<dyn OfferRepository>,
}

impl CounterOfferDraftService {
    pub async fn generate_draft(
        &self,
        request: &CounterOfferRequest,
    ) -> Result<CounterOfferDraft>;

    pub async fn record_outcome(
        &self,
        application_id: Uuid,
        outcome: NegotiationOutcome,
    ) -> Result<NegotiationHistory>;

    fn build_negotiation_context(request: &CounterOfferRequest) -> NegotiationContext;
    // NegotiationContext is the grounding struct passed to LLM — pure Rust, no LLM
}
```

**SQLite tables:**
```sql
CREATE TABLE counter_offer_drafts (
    id              TEXT PRIMARY KEY,
    offer_id        TEXT NOT NULL REFERENCES offer_details(id),
    application_id  TEXT NOT NULL REFERENCES applications(id),
    email_subject   TEXT NOT NULL,
    email_body      TEXT NOT NULL,
    talking_points_json TEXT NOT NULL,
    negotiation_warnings_json TEXT,
    data_quality_warning TEXT,
    tone            TEXT NOT NULL,
    generated_at    TEXT NOT NULL
);

CREATE TABLE negotiation_history (
    id                      TEXT PRIMARY KEY,
    application_id          TEXT NOT NULL REFERENCES applications(id),
    initial_offer_id        TEXT NOT NULL REFERENCES offer_details(id),
    final_offer_id          TEXT REFERENCES offer_details(id),
    rounds_json             TEXT NOT NULL,         -- Vec<NegotiationRound>
    outcome                 TEXT,                  -- enum variant name
    initial_annualized      INTEGER NOT NULL,
    final_annualized        INTEGER,
    comp_delta              INTEGER,               -- computed on close
    started_at              TEXT NOT NULL,
    completed_at            TEXT
);
```

## Open Questions

- **Counter-offer round limit warning**: Research shows 3+ counter-offer rounds damages relationships. Should the system display a warning when the user goes to generate a third draft ("Round 3 of negotiation — most companies reach a limit here. Consider whether to accept or withdraw")? Or is this paternalistic?
- **Offer letter parsing**: Should LazyJob support parsing a pasted offer letter PDF/text to auto-populate `OfferDetails` fields? This would reduce friction significantly but adds an LLM parsing step with risk of mis-extraction. If implemented, all parsed values must be shown for explicit user confirmation before saving.
- **Gender-aware coaching**: Research establishes that women face documented social backlash for aggressive negotiation. Should the system offer tone guidance that acknowledges this? Or does that risk being presumptuous and stereotyping? This is a product values decision, not just a technical one.
- **Negotiation outcome analytics**: `comp_delta` data across multiple negotiation outcomes could produce "negotiation patterns by company" insights. Should this be a feature (aggregate deltas across users in SaaS mode)? Requires careful consent and privacy handling — offer details are explicitly excluded from default SaaS sync.

## Implementation Tasks

- [ ] Define `CounterOfferRequest`, `CounterOfferDraft`, `NegotiationHistory`, `NegotiationOutcome`, `NegotiationRound` types in `lazyjob-core/src/salary/counter_offer.rs` and `lazyjob-core/src/salary/outcome.rs`
- [ ] Implement `CounterOfferDraftService::generate_draft` using `LlmProvider::complete` with a strict grounding prompt: include verified comp figures, never generate competing offer references unless `competing_offer_annualized` is present in `OfferEvaluation` — refs: `agentic-prompt-templates.md`, `agentic-llm-provider-abstraction.md`
- [ ] Implement negotiation prompt template in `lazyjob-llm/src/prompts/salary_negotiation.rs` with tone variants, priority ordering, and per-company-stage negotiable components list
- [ ] Create `counter_offer_drafts` and `negotiation_history` schema migration in `lazyjob-core/src/db/migrations/`
- [ ] Implement `CounterOfferDraftService::record_outcome` to close the negotiation loop: save `NegotiationHistory`, compute `comp_delta`, update `application_contacts` with final hiring-manager contact if provided
- [ ] Add TUI counter-offer view: `CounterOfferRequest` form (tone selector, priorities, target comp), draft display panel with `[DRAFT - NOT SENT]` header and copy-to-clipboard action, talking points accordion — refs: `architecture-tui-skeleton.md`
- [ ] Add negotiation outcome recording UI: after draft is viewed, prompt user to record outcome when they return to the application detail view; wire outcome to `PostTransitionSuggestion::RunSalaryComparison` completing the loop — refs: `application-workflow-actions.md`
