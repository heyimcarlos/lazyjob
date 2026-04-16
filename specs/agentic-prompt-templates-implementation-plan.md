# Implementation Plan: Agentic Prompt Templates (Per-Loop Context, Output Types, and Fabrication Detection)

## Status
Draft

## Related Spec
`specs/agentic-prompt-templates.md`

## Overview

This plan implements the **product-layer** of the LazyJob prompt system: the concrete context structs that ground each LLM call in verified SQLite data, the typed Rust output structs that parse and validate the LLM's JSON response, and the fabrication detection layer that enforces a three-tier constraint system across all seven Ralph loop types.

The companion spec `specs/17-ralph-prompt-templates.md` covers the infrastructure layer (TOML loader, `SimpleTemplateEngine`, `DefaultPromptRegistry`, injection sanitizer). This plan builds on top of that infrastructure. It does not duplicate the TOML template authoring or interpolation engine — it defines what the templates produce and what the loop worker does with that output.

The central design insight is the **grounding-before-generation** pattern: before any LLM call, a pure Rust `ContextBuilder` queries SQLite and assembles a strongly typed context struct. That struct is serialized into template variables and passed to `TemplateEngine::render()`. The LLM's raw JSON response is then deserialized into a typed output struct, and fabrication checks are run deterministically against the same context that grounded the call. No LLM output is persisted to SQLite unless it passes all three validation stages: JSON parse, schema conformance, and fabrication audit.

The fabrication detection layer lives in `lazyjob-core/src/life_sheet/fabrication.rs`. It is pure Rust with no async: claim matching via normalized string comparison, quantity extraction via regex, competing-offer detection via token scan. This keeps the most safety-critical code fast, testable, and independent of LLM availability.

## Prerequisites

### Specs that must be implemented first
- `specs/02-llm-provider-abstraction-implementation-plan.md` — `LlmProvider`, `ChatMessage`, `LlmResponse` types must exist.
- `specs/17-ralph-prompt-templates-implementation-plan.md` — `TemplateEngine`, `RenderedPrompt`, `PromptRegistry`, `sanitize_user_value()` must be in place; this plan builds on them.
- `specs/03-life-sheet-data-model-implementation-plan.md` — `LifeSheet`, `LifeSheetExperience`, `LifeSheetSkill`, `LifeSheetStory` types required by context structs.
- `specs/04-sqlite-persistence-implementation-plan.md` — repositories that context builders query.

### Crates to add to Cargo.toml

In `lazyjob-llm/Cargo.toml`:
```toml
[dependencies]
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
thiserror   = "1"
anyhow      = "1"
tracing     = "0.1"
regex       = "1"
once_cell   = "1"   # Lazy<Regex> pattern for prohibited-phrase detection
```

In `lazyjob-core/Cargo.toml` (already present; confirm):
```toml
regex     = "1"
once_cell = "1"
```

## Architecture

### Crate Placement

| Component | Crate | Reasoning |
|---|---|---|
| Context structs (7 types) | `lazyjob-llm/src/prompts/context.rs` | Context is the LLM input boundary — belongs with prompts |
| Output structs (7 types) | `lazyjob-llm/src/prompts/output.rs` | Typed parsing of LLM responses — belongs with prompts |
| Per-loop modules | `lazyjob-llm/src/prompts/{loop}.rs` | `system_prompt()`, `user_prompt()`, `validate_output()` per loop |
| `FabricationLevel`, `FabricationFinding` | `lazyjob-core/src/life_sheet/fabrication.rs` | Fabrication checking is a domain rule about LifeSheet truth — belongs in core |
| `is_grounded_claim()` | `lazyjob-core/src/life_sheet/fabrication.rs` | Same reasoning |
| `check_negotiation_fabrication()` | `lazyjob-core/src/life_sheet/fabrication.rs` | Same reasoning |

`lazyjob-ralph` imports context types from `lazyjob-llm` to build contexts, but never directly calls `validate_output()` — it calls into the per-loop module's public API.

### Core Types

#### Context Structs (`lazyjob-llm/src/prompts/context.rs`)

```rust
use lazyjob_core::{
    life_sheet::{LifeSheet, LifeSheetExperience, LifeSheetSkill, LifeSheetStory},
    job::{Job, JobDescriptionAnalysis},
    company::CompanyRecord,
    contact::{ProfileContact, SharedHistory},
    application::{OfferDetails, CompRange},
    interview::InterviewType,
};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Grounding context for the Job Discovery loop.
/// Computed from LifeSheet + target company list; no LLM calls.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobDiscoveryContext {
    /// Canonical skill names from life_sheet.skills
    pub skills: Vec<String>,
    /// One-paragraph synthesis of experience for prompt injection
    pub experience_summary: String,
    /// Remote/hybrid/onsite preference, salary floor
    pub preferences: JobSearchPreferences,
    /// List of company slugs to query job boards for
    pub target_companies: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobSearchPreferences {
    pub remote_ok: bool,
    pub hybrid_ok: bool,
    pub onsite_ok: bool,
    pub minimum_salary_usd: Option<i64>,
    pub excluded_industries: Vec<String>,
    pub locations: Vec<String>,
}

/// Grounding context for the Resume Tailoring loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResumeTailoringContext {
    /// Output of deterministic JD parser (regex + keyword extraction), NOT LLM
    pub jd_analysis: JobDescriptionAnalysis,
    /// Full experience entries from LifeSheet, ordered by recency
    pub experience_items: Vec<LifeSheetExperience>,
    /// Skills present in JD but absent or weak in LifeSheet
    pub skill_gap: Vec<SkillGapItem>,
    /// All quantified claims provable from LifeSheet (the fabrication baseline)
    pub fabrication_baseline: Vec<VerifiedClaim>,
    /// Which job this resume is being tailored for
    pub job_id: Uuid,
    pub job_title: String,
    pub company_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillGapItem {
    pub skill: String,
    pub required: bool,   // false = preferred
    pub in_life_sheet: bool,
    pub life_sheet_proficiency: Option<String>, // "beginner" | "intermediate" | "expert"
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifiedClaim {
    pub claim_text: String,
    pub source_experience_id: Uuid,
    pub claim_type: VerifiedClaimType,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedClaimType {
    Quantity,       // "increased revenue by 40%"
    Skill,          // "5 years Python"
    Credential,     // "AWS Solutions Architect"
    Achievement,    // "shipped product used by 10k users"
}

/// Grounding context for the Cover Letter loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoverLetterContext {
    pub company: CompanyRecord,
    pub jd_summary: String,           // 1-2 sentence synopsis, computed by caller
    pub relevant_experience: Vec<LifeSheetExperience>,
    pub user_name: String,
    pub template_type: CoverLetterTemplate,
    /// All experience claims that may be used; others are fabrication
    pub allowed_claims: Vec<VerifiedClaim>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTemplate {
    Standard,
    Story,
    CareerChange,
}

/// Grounding context for the Interview Prep loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InterviewContext {
    pub company: CompanyRecord,
    pub job: Job,
    pub interview_type: InterviewType,
    /// STAR stories from LifeSheet linked to this application
    pub candidate_stories: Vec<LifeSheetStory>,
    /// Skills mentioned in JD that the candidate has
    pub matched_skills: Vec<String>,
}

/// State for one turn of the Mock Interview loop.
/// The multi-turn conversation history is maintained in-memory by the worker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MockInterviewContext {
    pub session_id: Uuid,
    pub interview_type: InterviewType,
    /// The question just asked (from a prior LLM call or question bank)
    pub current_question: String,
    /// The candidate's raw text response for this turn
    pub candidate_response: String,
    /// Story bank to check claims against
    pub candidate_stories: Vec<LifeSheetStory>,
    /// Running conversation history (max last 10 turns to bound token usage)
    pub conversation_history: Vec<ConversationTurn>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationTurn {
    pub turn_id: Uuid,
    pub question: String,
    pub response: String,
    pub feedback_summary: String,
}

/// Grounding context for the Salary + Counter-Offer loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SalaryContext {
    pub offer: OfferDetails,
    pub market_data: Vec<SalaryDataPoint>,
    pub user_target_comp: Option<CompRange>,
    /// Only VERIFIED competing offers — from SQLite application records.
    /// The LLM must NEVER reference a competing offer not in this list.
    pub competing_offers: Vec<OfferDetails>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SalaryDataPoint {
    pub source: String,          // "h1b_lca" | "levels_fyi" | "glassdoor"
    pub role: String,
    pub location: String,
    pub yoe_min: u8,
    pub yoe_max: u8,
    pub base_salary_cents: i64,
    pub total_comp_cents: Option<i64>,
    pub sample_count: Option<u32>,
    pub as_of: DateTime<Utc>,
}

/// Grounding context for the Networking Outreach loop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NetworkingContext {
    pub contact: ProfileContact,
    /// Shared employers, schools, communities — computed from SQLite joins
    pub shared_history: SharedHistory,
    /// Present only when outreach is job-search-related
    pub target_company: Option<CompanyRecord>,
    pub user_goals: String,
    pub outreach_type: OutreachType,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutreachType {
    ColdOutreach,
    ReferralRequest,
    ThankYou,
    FollowUp,
}
```

#### Output Structs (`lazyjob-llm/src/prompts/output.rs`)

```rust
use uuid::Uuid;
use serde::{Deserialize, Serialize};

/// Validated output from the Job Discovery loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobDiscoveryOutput {
    pub scored_jobs: Vec<ScoredJob>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScoredJob {
    pub job_id: String,              // source job ID (not our UUID yet)
    pub match_score: f32,
    pub match_reasons: Vec<String>,
    pub gap_notes: Vec<String>,
    pub salary_range: Option<SalaryRange>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SalaryRange {
    pub min_usd: i64,
    pub max_usd: i64,
    pub currency: String,
}

/// Validated output from the Resume Tailoring loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TailoredResumeOutput {
    pub summary: String,
    pub experience: Vec<TailoredExperience>,
    pub skills_to_highlight: Vec<String>,
    pub fabrication_warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TailoredExperience {
    pub experience_id: Uuid,
    pub tailored_bullets: Vec<String>,
    pub original_bullets: Vec<String>,
    pub changes: Vec<String>,
}

/// Validated output from the Cover Letter loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoverLetterOutput {
    pub body: String,               // Full cover letter text (250-400 words)
    pub word_count: u32,
    pub template_used: CoverLetterTemplate,
    pub company_specific_details_used: Vec<String>,
}

/// Validated output from the Interview Prep loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InterviewPrepOutput {
    pub session_id: Uuid,
    pub questions: Vec<PrepQuestion>,
    pub company_cheat_sheet: CompanyCheatSheet,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PrepQuestion {
    pub question: String,
    #[serde(rename = "type")]
    pub question_type: String,     // "behavioral" | "technical" | "system_design" | "culture_fit"
    pub difficulty: String,        // "easy" | "medium" | "hard"
    pub linked_story_id: Option<Uuid>,
    pub story_gap: bool,
    pub tips: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompanyCheatSheet {
    pub mission: String,
    pub recent_news: Vec<String>,
    pub interview_signals: Vec<String>,
    pub culture_notes: Vec<String>,
}

/// Validated output from one Mock Interview turn.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MockInterviewFeedback {
    pub turn_id: Uuid,
    pub scores: MockInterviewScores,
    pub unverified_claims: Vec<String>,
    pub suggestions: Vec<String>,
    pub follow_up_question: Option<String>,  // None on final turn
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MockInterviewScores {
    pub structure: u8,    // 1-5
    pub depth: u8,
    pub authenticity: u8,
    pub result_clarity: u8,
}

/// Validated output from the Salary / Counter-Offer loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CounterOfferOutput {
    pub strategy: String,           // "market_gap" | "competing_offer" | "skills_scarcity" | "multi-lever"
    pub draft_email: String,
    pub talking_points: Vec<String>,
    pub leverage_used: Vec<String>,
    pub competing_offer_referenced: bool,
    pub risk_level: String,         // "low" | "medium" | "high"
}

/// Validated output from the Networking Outreach loop.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutreachDraftOutput {
    pub message: String,
    pub word_count: u32,
    pub outreach_type: OutreachType,
    pub shared_context_used: Vec<String>,   // which shared_history items were cited
}
```

#### Fabrication Types (`lazyjob-core/src/life_sheet/fabrication.rs`)

```rust
use uuid::Uuid;

/// Severity of a detected fabrication issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FabricationLevel {
    /// No fabrication detected.
    Clean,
    /// Claim is plausible but not explicitly in LifeSheet. Surface to user as advisory.
    Warning,
    /// Claim contradicts or extends LifeSheet data. Block output entirely.
    Critical,
}

/// A single fabrication finding produced by the post-processing checks.
#[derive(Debug, Clone)]
pub struct FabricationFinding {
    /// Human-readable description of what was found
    pub message: String,
    /// The specific text span that triggered the finding
    pub offending_text: String,
    pub level: FabricationLevel,
    /// If applicable, which experience/claim it was checked against
    pub checked_against_id: Option<Uuid>,
}
```

#### PromptError (`lazyjob-llm/src/prompts/error.rs`)

```rust
use crate::life_sheet::fabrication::FabricationFinding;

#[derive(thiserror::Error, Debug)]
pub enum PromptError {
    #[error("LLM response is not valid JSON: {0}")]
    NotJson(#[from] serde_json::Error),

    #[error("LLM response is missing required field: '{0}'")]
    MissingField(String),

    #[error("LLM response schema mismatch: expected {expected}, got {got}")]
    SchemaMismatch { expected: String, got: String },

    #[error("fabrication detected in LLM output")]
    FabricationDetected(Vec<FabricationFinding>),

    #[error("prohibited phrase detected in cover letter: '{phrase}'")]
    ProhibitedPhrase { phrase: String },

    #[error("score out of range: {field} = {value}, must be in [{min}, {max}]")]
    ScoreOutOfRange { field: String, value: f64, min: f64, max: f64 },

    #[error("template error: {0}")]
    Template(#[from] super::TemplateError),
}

pub type PromptResult<T> = std::result::Result<T, PromptError>;
```

### Trait Definitions

Each per-loop module exposes a consistent interface. The trait is not object-safe (associated types), so it is a convention rather than a dyn-dispatched trait:

```rust
// Convention (not a dyn trait) for each loop module:

// 1. A static system prompt (embedded in binary)
pub fn system_prompt() -> &'static str;

// 2. A user prompt builder (serializes ctx into TemplateVars, calls engine.render)
pub fn user_prompt(ctx: &XxxContext, engine: &dyn TemplateEngine, registry: &dyn PromptRegistry)
    -> PromptResult<RenderedPrompt>;

// 3. Output validator (parse JSON, check schema, run fabrication checks)
pub fn validate_output(raw: &str, ctx: &XxxContext) -> PromptResult<XxxOutput>;
```

The reason it is not a `dyn` trait is that each loop's context type differs, so a single dispatch surface would require `Any` downcasting — an anti-pattern. Instead, `lazyjob-ralph` imports the concrete module and calls the typed functions directly.

### SQLite Schema

No new tables are created by this spec. Fabrication findings and validation errors are emitted as `WorkerEvent::Error` payloads and logged via `tracing`; they are not persisted to SQLite. If finding history becomes important later, a `fabrication_log` table can be added without breaking this design.

The existing `token_usage_log` table (from the LLM provider abstraction plan) captures the raw request/response metadata including which template version was used — sufficient for audit without a dedicated fabrication table.

### Module Structure

```
lazyjob-llm/
  src/
    prompts/
      mod.rs              # re-exports: context, output, error, per-loop modules
      context.rs          # all 7 context structs + helper types
      output.rs           # all 7 output structs
      error.rs            # PromptError, PromptResult
      job_discovery.rs    # job discovery loop prompt API
      resume_tailoring.rs # resume tailoring loop prompt API
      cover_letter.rs     # cover letter loop prompt API + prohibited phrase detector
      interview_prep.rs   # interview prep loop prompt API
      mock_interview.rs   # mock interview multi-turn prompt API
      salary.rs           # salary/counter-offer loop prompt API
      networking.rs       # networking outreach loop prompt API

lazyjob-core/
  src/
    life_sheet/
      mod.rs              # (existing)
      fabrication.rs      # FabricationLevel, FabricationFinding, grounding checks
      fabrication_regex.rs # Lazy<Regex> patterns (quantity extraction, offer phrases)
```

## Implementation Phases

### Phase 1 — Fabrication Detection Layer

This is the most safety-critical code and must be built first so all loop implementations can depend on it.

**Step 1.1 — Define fabrication types**

File: `lazyjob-core/src/life_sheet/fabrication.rs`

Implement `FabricationLevel` (with `PartialOrd` so `max()` works across a Vec of findings) and `FabricationFinding` exactly as shown in Core Types above.

Add a helper to reduce a `Vec<FabricationFinding>` to its worst level:
```rust
pub fn worst_level(findings: &[FabricationFinding]) -> FabricationLevel {
    findings.iter().map(|f| f.level).max().unwrap_or(FabricationLevel::Clean)
}
```

Verification: `cargo test -p lazyjob-core life_sheet::fabrication` compiles clean.

**Step 1.2 — Compile-time regex pool**

File: `lazyjob-core/src/life_sheet/fabrication_regex.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;

/// Matches quantified claims: "50%", "3x", "$200k", "10,000 users"
pub static QUANTITY_CLAIM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(\d[\d,]*(?:\.\d+)?(?:x|%|\s*(?:million|billion|k|m|b))|\$[\d,]+(?:k|m)?)\b")
        .expect("static regex must compile")
});

/// Matches phrases that assert a competing offer exists.
/// Used by check_negotiation_fabrication().
pub static COMPETING_OFFER_PHRASES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(i (?:have|received|got) (?:an?|another) offer|another company (?:has )?offered|competing offer|offer from (?:another|a different)|rival offer)"
    )
    .expect("static regex must compile")
});

/// Matches skill mentions (e.g. "5 years of Python", "proficient in Rust")
pub static SKILL_CLAIM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(\d+\+?\s+years?(?:\s+of)?\s+)(\w+)")
        .expect("static regex must compile")
});
```

**Step 1.3 — `is_grounded_claim()`**

```rust
// lazyjob-core/src/life_sheet/fabrication.rs

use super::fabrication_regex::*;
use crate::life_sheet::{LifeSheet, LifeSheetExperience};

/// Check whether a single claim string is traceable to the LifeSheet.
///
/// Strategy:
/// 1. Extract all quantity tokens from `claim` using QUANTITY_CLAIM regex.
/// 2. For each quantity token, scan all experience bullet text in `life_sheet`.
///    If the exact token appears in any bullet → `Clean`.
///    If the claim is a paraphrase (no exact token match) → `Warning`.
/// 3. Extract skill-year phrases ("5 years Python"). If the skill appears in
///    `life_sheet.skills` with years >= claimed years → `Clean`; else `Warning`.
/// 4. If none of the extracted tokens appear anywhere in the LifeSheet → `Critical`.
pub fn is_grounded_claim(claim: &str, life_sheet: &LifeSheet) -> FabricationLevel {
    let all_bullet_text: String = life_sheet
        .experiences
        .iter()
        .flat_map(|e| e.bullets.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let all_text_lower = all_bullet_text.to_lowercase();

    // Check quantity claims
    let quantities: Vec<&str> = QUANTITY_CLAIM
        .find_iter(claim)
        .map(|m| m.as_str())
        .collect();

    if quantities.is_empty() {
        // Non-quantified claim — classify as Warning (plausible but unverifiable)
        return FabricationLevel::Warning;
    }

    let mut any_critical = false;
    for q in &quantities {
        if all_text_lower.contains(&q.to_lowercase()) {
            // Quantity appears verbatim in life sheet → grounded
            continue;
        }
        // Quantity not found anywhere → Critical
        any_critical = true;
    }

    if any_critical {
        FabricationLevel::Critical
    } else {
        FabricationLevel::Clean
    }
}
```

Verification:
- `test_grounded_claim_exact_quantity()` — quantity in claim matches LifeSheet bullet → `Clean`.
- `test_grounded_claim_invented_quantity()` — quantity not in LifeSheet → `Critical`.
- `test_grounded_claim_no_quantity()` — no numeric tokens → `Warning`.

**Step 1.4 — `check_negotiation_fabrication()`**

```rust
/// Scan counter-offer text for competing-offer phrases.
/// Returns a finding if a phrase is detected AND is not backed by `verified_offers`.
///
/// If `verified_offers` is empty and ANY competing-offer phrase is detected → Critical finding.
/// If `verified_offers` is non-empty but a phrase references an offer with no matching company → Warning.
pub fn check_negotiation_fabrication(
    counter_offer_text: &str,
    verified_offers: &[crate::application::OfferDetails],
) -> Option<FabricationFinding> {
    if let Some(m) = COMPETING_OFFER_PHRASES.find(counter_offer_text) {
        if verified_offers.is_empty() {
            return Some(FabricationFinding {
                message: "Counter-offer text references a competing offer but no verified competing offers exist".to_owned(),
                offending_text: m.as_str().to_owned(),
                level: FabricationLevel::Critical,
                checked_against_id: None,
            });
        }
        // Non-empty verified list: phrase is allowed (the offer data was in context)
        // Still surface as Warning so the user can review the draft
        return Some(FabricationFinding {
            message: "Counter-offer references competing offer — verify accuracy".to_owned(),
            offending_text: m.as_str().to_owned(),
            level: FabricationLevel::Warning,
            checked_against_id: None,
        });
    }
    None
}
```

Verification:
- `test_negotiation_fabrication_no_phrase()` → `None`.
- `test_negotiation_fabrication_phrase_no_offers()` → `Some(Critical)`.
- `test_negotiation_fabrication_phrase_with_offers()` → `Some(Warning)`.

---

### Phase 2 — Context Structs, Output Structs, PromptError

**Step 2.1 — Context structs**

File: `lazyjob-llm/src/prompts/context.rs`

Implement all 7 context structs as shown in Core Types. Key design decisions:
- All structs derive `serde::Serialize` so they can be serialized to JSON for template variable injection.
- Context structs reference types from `lazyjob-core` — add `lazyjob-core` as a path dependency in `lazyjob-llm/Cargo.toml`.
- `MockInterviewContext::conversation_history` is capped at 10 entries by the context builder to bound prompt size.

Implement a helper on each context for converting to `TemplateVars`:
```rust
impl ResumeTailoringContext {
    /// Serialize this context into TemplateVars for injection into the template.
    pub fn to_template_vars(&self) -> TemplateVars {
        let mut vars = TemplateVars::new();
        vars.insert("job_title".into(), sanitize_user_value(&self.job_title));
        vars.insert("company_name".into(), sanitize_user_value(&self.company_name));
        vars.insert(
            "job_description".into(),
            sanitize_user_value(&serde_json::to_string_pretty(&self.jd_analysis)
                .unwrap_or_default()),
        );
        vars.insert(
            "user_experience".into(),
            sanitize_user_value(&serde_json::to_string_pretty(&self.experience_items)
                .unwrap_or_default()),
        );
        vars.insert(
            "skill_gap".into(),
            sanitize_user_value(&serde_json::to_string_pretty(&self.skill_gap)
                .unwrap_or_default()),
        );
        vars
    }
}
```

Each context struct implements `to_template_vars()` following the same pattern.

Verification: Unit test serializes a minimal `ResumeTailoringContext` and calls `to_template_vars()`; assert expected keys are present.

**Step 2.2 — Output structs**

File: `lazyjob-llm/src/prompts/output.rs`

Implement all 7 output structs as shown in Core Types. Key decisions:
- All derive `serde::Deserialize` for `serde_json::from_str()`.
- `MockInterviewScores` fields are `u8` not `f32` to avoid floating-point comparison in tests.
- `CounterOfferOutput::strategy` is `String` (not enum) because LLM output is unreliable for exact enum matching; callers parse it downstream.

Verification: Each struct has a `#[test]` that parses a golden JSON string via `serde_json::from_str::<XxxOutput>(GOLDEN_JSON)`.

**Step 2.3 — PromptError**

File: `lazyjob-llm/src/prompts/error.rs`

Implement as shown in Core Types. Note the `From<serde_json::Error>` impl automatically converts JSON parse failures.

---

### Phase 3 — Per-Loop Prompt Modules

Each module follows the same three-function pattern. Below are the key implementation details per loop.

#### Step 3.1 — `job_discovery.rs`

```rust
// lazyjob-llm/src/prompts/job_discovery.rs

use super::{context::JobDiscoveryContext, output::JobDiscoveryOutput, error::PromptResult,
            PromptError, TemplateEngine, PromptRegistry, LoopType};

pub const SYSTEM_PROMPT: &str = include_str!("../templates/job_discovery_system.txt");

pub fn user_prompt(
    ctx: &JobDiscoveryContext,
    engine: &dyn TemplateEngine,
    registry: &dyn PromptRegistry,
) -> PromptResult<RenderedPrompt> {
    let vars = ctx.to_template_vars();
    let template = registry.get(LoopType::JobDiscovery)?;
    Ok(engine.render(template, &vars)?)
}

pub fn validate_output(raw: &str, _ctx: &JobDiscoveryContext) -> PromptResult<JobDiscoveryOutput> {
    // Step 1: parse JSON
    let output: JobDiscoveryOutput = serde_json::from_str(raw)?;

    // Step 2: validate score ranges
    for job in &output.scored_jobs {
        if !(0.0..=1.0).contains(&job.match_score) {
            return Err(PromptError::ScoreOutOfRange {
                field: format!("scored_jobs[job_id={}].match_score", job.job_id),
                value: job.match_score as f64,
                min: 0.0,
                max: 1.0,
            });
        }
    }

    // No fabrication checks needed: job discovery reports real jobs from external APIs,
    // not user-originated claims.
    Ok(output)
}
```

#### Step 3.2 — `resume_tailoring.rs`

This module enforces **Tier 1 fabrication** (profile fabrication).

```rust
pub fn validate_output(
    raw: &str,
    ctx: &ResumeTailoringContext,
) -> PromptResult<TailoredResumeOutput> {
    let output: TailoredResumeOutput = serde_json::from_str(raw)?;

    // Tier 1: scan all tailored bullets for ungrounded claims
    let life_sheet = &ctx.life_sheet; // stored on context for fabrication checks
    let mut findings: Vec<FabricationFinding> = Vec::new();

    for exp in &output.experience {
        for bullet in &exp.tailored_bullets {
            match is_grounded_claim(bullet, life_sheet) {
                FabricationLevel::Clean => {}
                FabricationLevel::Warning => {
                    findings.push(FabricationFinding {
                        message: format!("Unverifiable claim in bullet: {}", bullet),
                        offending_text: bullet.clone(),
                        level: FabricationLevel::Warning,
                        checked_against_id: Some(exp.experience_id),
                    });
                }
                FabricationLevel::Critical => {
                    findings.push(FabricationFinding {
                        message: format!("Fabricated claim detected in bullet: {}", bullet),
                        offending_text: bullet.clone(),
                        level: FabricationLevel::Critical,
                        checked_against_id: Some(exp.experience_id),
                    });
                }
            }
        }
    }

    // Block on any Critical finding
    if findings.iter().any(|f| f.level == FabricationLevel::Critical) {
        return Err(PromptError::FabricationDetected(findings));
    }

    // Warnings propagate into the output for user review
    // (The caller can surface them in the TUI)
    Ok(output)
}
```

`ResumeTailoringContext` must also include a reference to `LifeSheet` for the fabrication check.

#### Step 3.3 — `cover_letter.rs`

This module enforces **Tier 2 fabrication** (narrative fabrication) plus **prohibited phrase detection**.

```rust
// lazyjob-llm/src/prompts/cover_letter.rs

use once_cell::sync::Lazy;
use regex::Regex;

/// Prohibited phrases are product guardrails, not user-configurable.
/// Compiled once at startup.
static PROHIBITED_PHRASES: Lazy<Vec<Regex>> = Lazy::new(|| {
    let patterns = [
        r"(?i)i'?m passionate about",
        r"(?i)\bhard worker\b",
        r"(?i)\bteam player\b",
        r"(?i)\bresults.?driven\b",
        r"(?i)\bgo.?getter\b",
        r"(?i)i am excited to",   // generic opener
        r"(?i)i hope this (?:message|email|letter)",  // generic opener
    ];
    patterns.iter()
        .map(|p| Regex::new(p).expect("static regex"))
        .collect()
});

pub fn validate_output(
    raw: &str,
    ctx: &CoverLetterContext,
) -> PromptResult<CoverLetterOutput> {
    // Parse JSON envelope
    let output: CoverLetterOutput = serde_json::from_str(raw)?;

    // Prohibited phrase scan
    for re in PROHIBITED_PHRASES.iter() {
        if let Some(m) = re.find(&output.body) {
            return Err(PromptError::ProhibitedPhrase {
                phrase: m.as_str().to_owned(),
            });
        }
    }

    // Word count sanity check (spec says 250–400 words)
    let word_count = output.body.split_whitespace().count() as u32;
    if word_count < 200 || word_count > 500 {
        tracing::warn!(word_count, "Cover letter word count outside expected range");
    }

    // Tier 2: company-specific claims must be from company research
    // Check that every item in company_specific_details_used appears in ctx.company research text
    let company_research_text = ctx.company.research_summary.to_lowercase();
    let mut findings = Vec::new();
    for detail in &output.company_specific_details_used {
        // Fuzzy match: if the first 10 chars of the detail appear in research text it's grounded
        let prefix = &detail.to_lowercase()[..detail.len().min(15)];
        if !company_research_text.contains(prefix) {
            findings.push(FabricationFinding {
                message: format!("Company claim not found in research: {}", detail),
                offending_text: detail.clone(),
                level: FabricationLevel::Warning,
                checked_against_id: None,
            });
        }
    }

    if findings.iter().any(|f| f.level == FabricationLevel::Critical) {
        return Err(PromptError::FabricationDetected(findings));
    }

    Ok(output)
}
```

#### Step 3.4 — `interview_prep.rs`

Output validation checks that `linked_story_id` values (when non-null) refer to actual story IDs in `ctx.candidate_stories`. Any reference to a non-existent story is a `Critical` fabrication.

```rust
pub fn validate_output(
    raw: &str,
    ctx: &InterviewContext,
) -> PromptResult<InterviewPrepOutput> {
    let output: InterviewPrepOutput = serde_json::from_str(raw)?;

    let known_story_ids: std::collections::HashSet<Uuid> =
        ctx.candidate_stories.iter().map(|s| s.id).collect();

    let mut findings = Vec::new();
    for q in &output.questions {
        if let Some(story_id) = q.linked_story_id {
            if !known_story_ids.contains(&story_id) {
                findings.push(FabricationFinding {
                    message: format!("Question links to non-existent story_id {}", story_id),
                    offending_text: q.question.clone(),
                    level: FabricationLevel::Critical,
                    checked_against_id: Some(story_id),
                });
            }
        }
    }

    if !findings.is_empty() {
        return Err(PromptError::FabricationDetected(findings));
    }

    Ok(output)
}
```

#### Step 3.5 — `mock_interview.rs` (multi-turn pattern)

The mock interview loop is interactive. The `MockInterviewWorker` (in `lazyjob-ralph`) maintains state across turns. The prompt module provides per-turn functions:

```rust
// lazyjob-llm/src/prompts/mock_interview.rs

/// Build the conversation messages for one feedback turn.
/// The conversation history is prepended as alternating user/assistant messages.
pub fn feedback_turn_messages(ctx: &MockInterviewContext) -> Vec<ChatMessage> {
    let mut messages = vec![ChatMessage::System(SYSTEM_PROMPT.to_owned())];

    // Inject history as alternating turns
    for turn in &ctx.conversation_history {
        messages.push(ChatMessage::User(turn.question.clone()));
        messages.push(ChatMessage::Assistant(turn.feedback_summary.clone()));
    }

    // Current turn
    messages.push(ChatMessage::User(format!(
        "Question: {}\n\nCandidate response:\n{}",
        ctx.current_question,
        ctx.candidate_response
    )));

    messages
}

pub fn validate_turn_feedback(
    raw: &str,
    ctx: &MockInterviewContext,
) -> PromptResult<MockInterviewFeedback> {
    let feedback: MockInterviewFeedback = serde_json::from_str(raw)?;

    // Validate score ranges (1-5)
    let scores = &feedback.scores;
    for (field, val) in [
        ("structure", scores.structure),
        ("depth", scores.depth),
        ("authenticity", scores.authenticity),
        ("result_clarity", scores.result_clarity),
    ] {
        if !(1..=5).contains(&val) {
            return Err(PromptError::ScoreOutOfRange {
                field: field.to_owned(),
                value: val as f64,
                min: 1.0,
                max: 5.0,
            });
        }
    }

    // Validate that unverified_claims is populated from the response, not invented
    // (We cannot do deeper verification here without another LLM call — this is the
    // open question in the spec about false positive rate. For now, surface all
    // unverified_claims to the user without blocking output.)

    Ok(feedback)
}
```

The `MockInterviewWorker` in `lazyjob-ralph` uses this pattern:
```rust
loop {
    // 1. Get next question from question bank or LLM follow-up
    // 2. Display question to user in TUI
    // 3. Collect user's text response
    // 4. Build MockInterviewContext with updated history
    // 5. let messages = feedback_turn_messages(&ctx);
    // 6. let raw = provider.chat(messages).await?;
    // 7. let feedback = validate_turn_feedback(&raw, &ctx)?;
    // 8. Display feedback in TUI
    // 9. Append turn to ctx.conversation_history (cap at 10)
    // 10. If feedback.follow_up_question is None → session complete
}
```

#### Step 3.6 — `salary.rs`

Enforces **Tier 3 fabrication** (strictest — competing offer fabrication).

```rust
pub fn validate_output(raw: &str, ctx: &SalaryContext) -> PromptResult<CounterOfferOutput> {
    let output: CounterOfferOutput = serde_json::from_str(raw)?;

    // Tier 3: check for fabricated competing offers
    if let Some(finding) = check_negotiation_fabrication(&output.draft_email, &ctx.competing_offers) {
        if finding.level == FabricationLevel::Critical {
            // Hard block: no degraded output
            return Err(PromptError::FabricationDetected(vec![finding]));
        }
        // Warning: surface to user but do not block
        tracing::warn!(
            offending_text = %finding.offending_text,
            "Counter-offer references competing offer — user review required"
        );
    }

    // Validate strategy is a known value
    let valid_strategies = ["market_gap", "competing_offer", "skills_scarcity", "multi-lever"];
    if !valid_strategies.contains(&output.strategy.as_str()) {
        return Err(PromptError::SchemaMismatch {
            expected: format!("one of {:?}", valid_strategies),
            got: output.strategy.clone(),
        });
    }

    // Validate risk_level
    let valid_risks = ["low", "medium", "high"];
    if !valid_risks.contains(&output.risk_level.as_str()) {
        return Err(PromptError::SchemaMismatch {
            expected: format!("one of {:?}", valid_risks),
            got: output.risk_level.clone(),
        });
    }

    // If competing_offer_referenced = true but no verified offers exist → block
    if output.competing_offer_referenced && ctx.competing_offers.is_empty() {
        return Err(PromptError::FabricationDetected(vec![FabricationFinding {
            message: "Output claims competing_offer_referenced=true but no verified offers in context".into(),
            offending_text: output.draft_email.clone(),
            level: FabricationLevel::Critical,
            checked_against_id: None,
        }]));
    }

    Ok(output)
}
```

#### Step 3.7 — `networking.rs`

Checks that every item in `shared_context_used` actually appears in `ctx.shared_history`.

```rust
pub fn validate_output(raw: &str, ctx: &NetworkingContext) -> PromptResult<OutreachDraftOutput> {
    let output: OutreachDraftOutput = serde_json::from_str(raw)?;

    // Word count: spec says max 150 words
    let word_count = output.message.split_whitespace().count() as u32;
    if word_count > 200 {
        tracing::warn!(word_count, "Outreach message exceeds recommended 150-word limit");
    }

    // Verify all cited shared-context items exist in ctx.shared_history
    let known_contexts: std::collections::HashSet<&str> = ctx
        .shared_history
        .items
        .iter()
        .map(|i| i.description.as_str())
        .collect();

    let mut findings = Vec::new();
    for cited in &output.shared_context_used {
        // Fuzzy match: if any known context item contains the cited string as substring
        let found = known_contexts.iter().any(|k| k.contains(cited.as_str()));
        if !found {
            findings.push(FabricationFinding {
                message: format!("Cited shared context not in verified history: {}", cited),
                offending_text: cited.clone(),
                level: FabricationLevel::Critical,
                checked_against_id: None,
            });
        }
    }

    if !findings.is_empty() {
        return Err(PromptError::FabricationDetected(findings));
    }

    Ok(output)
}
```

---

### Phase 4 — Context Builders

Context builders are the bridges between SQLite repositories and the context structs. They live in `lazyjob-ralph/src/context_builders/` because they are Ralph-specific orchestration code that queries the domain repositories.

```rust
// lazyjob-ralph/src/context_builders/resume_tailoring.rs

use lazyjob_core::{
    life_sheet::LifeSheetRepository,
    job::JobRepository,
    application::ApplicationRepository,
};
use lazyjob_llm::prompts::context::{ResumeTailoringContext, SkillGapItem, VerifiedClaim};
use uuid::Uuid;

pub async fn build_resume_tailoring_context(
    job_id: Uuid,
    job_repo: &JobRepository,
    life_sheet_repo: &LifeSheetRepository,
) -> anyhow::Result<ResumeTailoringContext> {
    let job = job_repo.get_by_id(job_id).await?
        .ok_or_else(|| anyhow::anyhow!("job {} not found", job_id))?;
    let life_sheet = life_sheet_repo.load().await?;

    // Deterministic JD analysis (no LLM)
    let jd_analysis = lazyjob_core::job::analyze_job_description(&job.description);

    // Compute skill gap: JD required skills not in LifeSheet
    let life_sheet_skills: std::collections::HashSet<String> = life_sheet
        .skills
        .iter()
        .map(|s| s.name.to_lowercase())
        .collect();
    let skill_gap: Vec<SkillGapItem> = jd_analysis
        .required_skills
        .iter()
        .map(|s| SkillGapItem {
            skill: s.clone(),
            required: true,
            in_life_sheet: life_sheet_skills.contains(&s.to_lowercase()),
            life_sheet_proficiency: life_sheet
                .skills
                .iter()
                .find(|ls| ls.name.to_lowercase() == s.to_lowercase())
                .map(|ls| ls.proficiency.clone()),
        })
        .collect();

    // Extract fabrication baseline: all quantified claims from experience bullets
    let fabrication_baseline: Vec<VerifiedClaim> = life_sheet
        .experiences
        .iter()
        .flat_map(|exp| {
            exp.bullets.iter().filter_map(|bullet| {
                // Only include bullets with verifiable quantities
                if fabrication_regex::QUANTITY_CLAIM.is_match(bullet) {
                    Some(VerifiedClaim {
                        claim_text: bullet.clone(),
                        source_experience_id: exp.id,
                        claim_type: VerifiedClaimType::Quantity,
                    })
                } else {
                    None
                }
            })
        })
        .collect();

    Ok(ResumeTailoringContext {
        jd_analysis,
        experience_items: life_sheet.experiences.clone(),
        skill_gap,
        fabrication_baseline,
        job_id,
        job_title: job.title.clone(),
        company_name: job.company_name.clone(),
        life_sheet,
    })
}
```

Context builders for the other 6 loop types follow the same pattern.

Verification: Unit test with in-memory SQLite (via `sqlx::test`) builds a context and asserts non-empty `fabrication_baseline`.

---

### Phase 5 — Integration and WorkerEvent wiring

When `validate_output()` returns `Err(PromptError::FabricationDetected(findings))`, the Ralph worker must:
1. NOT write the LLM output to SQLite.
2. Emit a `WorkerEvent::Error` on the event channel with `code: "fabrication_detected"`.
3. Log the findings via `tracing::warn!`.

```rust
// lazyjob-ralph/src/workers/resume_tailor.rs (example)

match resume_tailoring::validate_output(&raw_response, &ctx) {
    Ok(output) => {
        // Persist to SQLite
        resume_repo.save_version(&output, ctx.job_id).await?;
        event_tx.send(WorkerEvent::Progress {
            message: "Resume tailored successfully".into(),
        })?;
    }
    Err(PromptError::FabricationDetected(findings)) => {
        for f in &findings {
            tracing::warn!(
                level = ?f.level,
                offending = %f.offending_text,
                "fabrication detected in resume output"
            );
        }
        event_tx.send(WorkerEvent::Error {
            code: "fabrication_detected".into(),
            message: format!("{} fabrication issue(s) detected — output blocked", findings.len()),
        })?;
    }
    Err(e) => {
        return Err(e.into());
    }
}
```

---

## Key Crate APIs

| Purpose | API |
|---|---|
| Parse LLM JSON output into typed struct | `serde_json::from_str::<TailoredResumeOutput>(raw)?` |
| Detect prohibited phrases | `PROHIBITED_PHRASES.iter().any(\|re\| re.is_match(text))` |
| Extract quantity tokens | `QUANTITY_CLAIM.find_iter(text).map(\|m\| m.as_str())` |
| Detect competing-offer phrases | `COMPETING_OFFER_PHRASES.find(text)` |
| One-time regex compilation | `once_cell::sync::Lazy<Regex>` with `Regex::new(pattern).expect(...)` |
| Template variable sanitization | `sanitize_user_value(raw)` from `lazyjob-llm::prompts::sanitizer` |
| Worst fabrication level | `fabrication::worst_level(&findings)` → `FabricationLevel` |
| Tracing fabrication events | `tracing::warn!(level=?f.level, offending=%f.offending_text, "...")` |
| Cap conversation history | `ctx.conversation_history.truncate(10)` |

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum PromptError {
    #[error("LLM response is not valid JSON: {0}")]
    NotJson(#[from] serde_json::Error),

    #[error("LLM response is missing required field: '{0}'")]
    MissingField(String),

    #[error("LLM response schema mismatch: expected {expected}, got {got}")]
    SchemaMismatch { expected: String, got: String },

    #[error("fabrication detected in LLM output ({} finding(s))", .0.len())]
    FabricationDetected(Vec<FabricationFinding>),

    #[error("prohibited phrase in cover letter: '{phrase}'")]
    ProhibitedPhrase { phrase: String },

    #[error("score out of range: {field} = {value} (must be {min}–{max})")]
    ScoreOutOfRange { field: String, value: f64, min: f64, max: f64 },

    #[error("template error: {0}")]
    Template(#[from] TemplateError),
}

pub type PromptResult<T> = std::result::Result<T, PromptError>;
```

**Handling at the call site in Ralph workers:**

| Error variant | Action |
|---|---|
| `NotJson` | Retry once (LLM sometimes produces non-JSON preamble); if still fails → `WorkerEvent::Error` |
| `MissingField` | Emit `WorkerEvent::Error`; log at `error` level (indicates template regression) |
| `SchemaMismatch` | Same as `MissingField` |
| `FabricationDetected(Critical)` | Block output, emit `WorkerEvent::Error`, log all findings |
| `FabricationDetected(Warning only)` | Allow output, surface warnings to user in TUI |
| `ProhibitedPhrase` | Retry with explicit instruction to avoid the phrase; cap at 2 retries |
| `ScoreOutOfRange` | Clamp silently with a `tracing::warn!` |

## Testing Strategy

### Unit Tests — Fabrication Layer (`lazyjob-core`)

File: `lazyjob-core/src/life_sheet/fabrication.rs` (inline `#[cfg(test)]` block)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_life_sheet_with_bullet(bullet: &str) -> LifeSheet {
        // construct minimal LifeSheet with one experience containing the bullet
        LifeSheet {
            experiences: vec![LifeSheetExperience {
                id: Uuid::new_v4(),
                bullets: vec![bullet.to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn clean_when_quantity_matches_life_sheet() {
        let ls = make_life_sheet_with_bullet("increased revenue by 40%");
        assert_eq!(is_grounded_claim("increased revenue by 40%", &ls), FabricationLevel::Clean);
    }

    #[test]
    fn critical_when_quantity_not_in_life_sheet() {
        let ls = make_life_sheet_with_bullet("increased revenue by 40%");
        assert_eq!(is_grounded_claim("increased revenue by 60%", &ls), FabricationLevel::Critical);
    }

    #[test]
    fn warning_when_no_quantity_in_claim() {
        let ls = make_life_sheet_with_bullet("led team to success");
        assert_eq!(is_grounded_claim("led the team effectively", &ls), FabricationLevel::Warning);
    }

    #[test]
    fn negotiation_fabrication_no_phrase_returns_none() {
        let result = check_negotiation_fabrication("Based on market data, I believe $X is fair.", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn negotiation_fabrication_phrase_no_offers_is_critical() {
        let result = check_negotiation_fabrication(
            "I have received an offer from another company for $200k.",
            &[],
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().level, FabricationLevel::Critical);
    }

    #[test]
    fn negotiation_fabrication_phrase_with_offers_is_warning() {
        let offers = vec![OfferDetails {
            company_name: "Acme Corp".into(),
            base_salary_cents: 20_000_000,
            ..Default::default()
        }];
        let result = check_negotiation_fabrication(
            "I have an offer from another company.",
            &offers,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap().level, FabricationLevel::Warning);
    }
}
```

### Unit Tests — Per-Loop `validate_output()` Golden Tests

Each loop module has a test submodule with three golden cases:

1. **Clean pass**: a well-formed JSON string passes validation.
2. **Fabrication case**: a JSON string containing a fabricated claim returns `FabricationDetected`.
3. **Missing field case**: JSON missing a required field returns `NotJson` or `MissingField`.

Example for `resume_tailoring.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN_OUTPUT: &str = r#"{
      "summary": "Experienced engineer with focus on distributed systems.",
      "experience": [{
        "experience_id": "00000000-0000-0000-0000-000000000001",
        "tailored_bullets": ["Scaled Redis cluster to handle 50k RPS"],
        "original_bullets": ["Built Redis cluster handling 50k RPS"],
        "changes": ["emphasized scale metric"]
      }],
      "skills_to_highlight": ["Rust", "Redis"],
      "fabrication_warnings": []
    }"#;

    const FABRICATED_OUTPUT: &str = r#"{
      "summary": "...",
      "experience": [{
        "experience_id": "00000000-0000-0000-0000-000000000001",
        "tailored_bullets": ["Scaled revenue by 500%"],
        "original_bullets": ["Contributed to revenue growth"],
        "changes": []
      }],
      "skills_to_highlight": [],
      "fabrication_warnings": []
    }"#;

    fn make_ctx_with_bullet(bullet: &str) -> ResumeTailoringContext {
        // minimal context; life_sheet has bullet with "50k RPS"
        // ...
    }

    #[test]
    fn clean_output_passes() {
        let ctx = make_ctx_with_bullet("Built Redis cluster handling 50k RPS");
        let result = validate_output(CLEAN_OUTPUT, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn fabricated_quantity_is_blocked() {
        let ctx = make_ctx_with_bullet("Contributed to revenue growth");
        let result = validate_output(FABRICATED_OUTPUT, &ctx);
        assert!(matches!(result, Err(PromptError::FabricationDetected(_))));
    }

    #[test]
    fn malformed_json_returns_not_json() {
        let ctx = make_ctx_with_bullet("anything");
        let result = validate_output("{invalid json}", &ctx);
        assert!(matches!(result, Err(PromptError::NotJson(_))));
    }
}
```

### Unit Tests — Prohibited Phrase Detection

```rust
#[test]
fn prohibited_phrase_blocked() {
    let raw = r#"{"body": "I'm passionate about joining your team...", "word_count": 10, ...}"#;
    let result = validate_output(raw, &make_cover_letter_ctx());
    assert!(matches!(result, Err(PromptError::ProhibitedPhrase { .. })));
}

#[test]
fn normal_cover_letter_passes() {
    // A well-written cover letter with no prohibited phrases
    let raw = r#"{"body": "In my three years at Acme...", "word_count": 280, ...}"#;
    let result = validate_output(raw, &make_cover_letter_ctx());
    assert!(result.is_ok());
}
```

### Integration Tests (require LLM key, CI-optional)

```rust
// lazyjob-llm/tests/prompt_integration.rs

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn resume_tailoring_prompt_produces_grounded_output() {
    let ctx = build_test_resume_tailoring_context().await;
    let registry = DefaultPromptRegistry::new().unwrap();
    let engine = SimpleTemplateEngine;
    let rendered = resume_tailoring::user_prompt(&ctx, &engine, &registry).unwrap();
    let provider = AnthropicProvider::from_env().unwrap();
    let raw = provider.chat(rendered.into_chat_messages()).await.unwrap().content;
    let output = resume_tailoring::validate_output(&raw, &ctx)
        .expect("LLM output must pass fabrication checks");
    assert!(!output.experience.is_empty());
}
```

### TUI Tests

No TUI integration required for this spec. The fabrication findings surface as `WorkerEvent::Error` payloads which the TUI already renders as error overlays (from the application workflow plan). No new TUI components are needed.

## Open Questions

1. **Prohibited phrases: product guardrail vs. user configuration?** The spec lists "I'm passionate about" as prohibited. Some users may genuinely prefer this phrasing. Current plan: hardcoded product guardrail. Resolution path: add an `allow_prohibited_phrases: bool` field to `~/.config/lazyjob/config.toml` — default `false`. If `true`, skip `PROHIBITED_PHRASES` check. Do NOT make individual phrases configurable (list management is a UX burden).

2. **Mock interview `unverified_claims` false positive rate.** The LLM is asked to compare the candidate's spoken response against the story bank and flag unverifiable claims. This is itself an LLM reasoning task — it will produce false positives. Current plan: surface `unverified_claims` as advisory warnings (never block mock interview output), with a UI note "AI-flagged — review manually". A future improvement could use embedding cosine similarity to compare the response against story bank text as a lower-false-positive approach.

3. **Counter-offer `risk_level` computation.** The spec acknowledges that LLM-assessed risk is inaccurate. Proposed deterministic alternative: `risk_level = "high"` if ask exceeds offer by >30%, `"medium"` if 10-30%, `"low"` if <10%. This rule is easy to unit-test and does not rely on LLM judgment. The spec leaves this open — the plan implements deterministic risk computation as a post-processing override that replaces the LLM's `risk_level` field.

4. **Quantity normalization in `is_grounded_claim()`.** "40 percent" and "40%" are semantically identical but the current string-contains check would miss the first if the LifeSheet contains the second. A future improvement: normalize all quantity representations before comparison. Deferred to Phase 6.

5. **Context builder placement.** Context builders are placed in `lazyjob-ralph` because they orchestrate multiple repositories. If `lazyjob-tui` ever needs to display a preview of what the LLM will receive, it would need to import context builders from `lazyjob-ralph`, creating a crate dependency in the wrong direction. Alternative: move context builders to `lazyjob-core` as pure data assembly functions, with repository ports passed as trait objects. Deferred — current placement is correct for MVP.

## Related Specs
- `specs/17-ralph-prompt-templates.md` — template infrastructure this plan builds on
- `specs/02-llm-provider-abstraction.md` — `LlmProvider`, `ChatMessage` types
- `specs/03-life-sheet-data-model.md` — `LifeSheet`, `LifeSheetExperience`, `LifeSheetStory`
- `specs/06-ralph-loop-integration.md` — Ralph loop runner consuming `RenderedPrompt`
- `specs/07-resume-tailoring-pipeline.md` — ResumeTailoring feature that uses context types from this plan
- `specs/08-cover-letter-generation.md` — CoverLetter feature using the fabrication Tier 2 system
- `specs/16-privacy-security.md` — sanitization of user data (prompt injection defense) referenced here
