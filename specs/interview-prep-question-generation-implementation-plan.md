# Implementation Plan: Interview Prep Question Generation

## Status
Draft

## Related Spec
[specs/interview-prep-question-generation.md](./interview-prep-question-generation.md)

## Overview

The interview prep question generation module produces a personalized, role-specific set of interview questions for a specific job application. It combines a `JobListing`, the candidate's `LifeSheet`, the `CompanyRecord` from the company research pipeline, and an `InterviewType` enum to synthesize a set of `InterviewQuestion` values through a structured LLM completion call. Each question includes evaluator coaching tips and, for behavioral questions, a pointer to a matching LifeSheet story.

The module follows LazyJob's canonical anti-fabrication pattern: a `PrepContextBuilder` assembles all verified facts as a `PrepContext` struct before any LLM call is made. The LLM receives only this pre-verified object as its grounding, never raw unstructured text. This eliminates the class of hallucination bugs where the LLM invents company-specific interview processes that don't exist.

Architecturally, the service lives entirely in `lazyjob-core` with no TUI dependency. Question generation is triggered by `PostTransitionSuggestion::GenerateInterviewPrep` in the workflow actions layer and dispatched as a Ralph loop via `lazyjob-ralph`. The TUI "Interview Prep" panel — accessible from the application detail view — renders the latest `InterviewPrepSession` for the application.

## Prerequisites

### Specs/Plans that must precede this
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `WorkExperience`, `SkillEntry`, `LifeSheetRepository`
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, migration runner, `sqlx::Pool<Sqlite>`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>`, `ChatMessage`, `CompletionRequest`
- `specs/job-search-discovery-engine-implementation-plan.md` — provides `JobRepository`, `JobListing`, `JobId`
- `specs/job-search-company-research-implementation-plan.md` — provides `CompanyRecord`, `CompanyRepository`, `interview_signals`
- `specs/profile-resume-tailoring-implementation-plan.md` — provides `JdParser`, `JobDescriptionAnalysis`
- `specs/application-workflow-actions-implementation-plan.md` — provides `PostTransitionSuggestion::GenerateInterviewPrep`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel system
- `specs/agentic-ralph-orchestration-implementation-plan.md` — provides `LoopType`, `LoopQueue`

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml — new additions
strsim = "0.11"          # Jaro-Winkler for STAR story keyword matching
regex  = "1"
once_cell = "1"          # Lazy<Regex> for seniority signal patterns

# Already present from prior plans:
sqlx      = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
serde     = { version = "1", features = ["derive"] }
serde_json = "1"
chrono    = { version = "0.4", features = ["serde"] }
uuid      = { version = "1", features = ["v4", "serde"] }
tokio     = { version = "1", features = ["full"] }
thiserror = "2"
anyhow    = "1"
tracing   = "0.1"
async-trait = "0.1"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All domain logic: `InterviewType`, `QuestionCategory`, `InterviewQuestion`, `PrepContext`, `PrepContextBuilder`, `InterviewPrepSession`, `InterviewPrepService`, `InterviewPrepRepository`, story-mapping logic, SQLite migrations |
| `lazyjob-llm` | Prompt template `LoopType::InterviewPrepGen` — TOML template embedded at compile time |
| `lazyjob-ralph` | Dispatch glue: receives `PostTransitionSuggestion::GenerateInterviewPrep`, enqueues `LoopType::InterviewPrepGen` |
| `lazyjob-tui` | `InterviewPrepView`, `QuestionListWidget`, `QuestionDetailPanel` |
| `lazyjob-cli` | `lazyjob interview prep <application-id> [--type phone|technical|behavioral|onsite|system-design]` subcommand |

`lazyjob-core` has no TUI or CLI dependencies. All types flow upward.

### Core Types

```rust
// lazyjob-core/src/interview/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which stage/format the interview preparation is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterviewType {
    PhoneScreen,
    TechnicalScreen,
    Behavioral,
    OnSite,
    SystemDesign,
    ExecutiveOrBarRaiser,
}

impl InterviewType {
    /// Returns (behavioral, technical, system_design, to_ask_interviewer, culture_fit)
    /// question counts for this interview type.
    pub fn question_mix(&self) -> QuestionMix {
        match self {
            Self::PhoneScreen       => QuestionMix { behavioral: 2, technical: 0, system_design: 0, to_ask: 2, culture_fit: 1 },
            Self::TechnicalScreen   => QuestionMix { behavioral: 0, technical: 3, system_design: 0, to_ask: 1, culture_fit: 1 },
            Self::Behavioral        => QuestionMix { behavioral: 4, technical: 0, system_design: 0, to_ask: 0, culture_fit: 1 },
            Self::OnSite            => QuestionMix { behavioral: 2, technical: 2, system_design: 1, to_ask: 3, culture_fit: 0 },
            Self::SystemDesign      => QuestionMix { behavioral: 0, technical: 0, system_design: 2, to_ask: 1, culture_fit: 0 },
            Self::ExecutiveOrBarRaiser => QuestionMix { behavioral: 3, technical: 1, system_design: 0, to_ask: 2, culture_fit: 1 },
        }
    }

    /// Serializes to the TEXT stored in SQLite.
    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::PhoneScreen           => "phone_screen",
            Self::TechnicalScreen       => "technical_screen",
            Self::Behavioral            => "behavioral",
            Self::OnSite                => "on_site",
            Self::SystemDesign          => "system_design",
            Self::ExecutiveOrBarRaiser  => "executive_or_bar_raiser",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, InterviewPrepError> {
        match s {
            "phone_screen"              => Ok(Self::PhoneScreen),
            "technical_screen"          => Ok(Self::TechnicalScreen),
            "behavioral"                => Ok(Self::Behavioral),
            "on_site"                   => Ok(Self::OnSite),
            "system_design"             => Ok(Self::SystemDesign),
            "executive_or_bar_raiser"   => Ok(Self::ExecutiveOrBarRaiser),
            other => Err(InterviewPrepError::InvalidInterviewType(other.to_owned())),
        }
    }
}

/// The counts for each question category produced for a given `InterviewType`.
#[derive(Debug, Clone, Copy)]
pub struct QuestionMix {
    pub behavioral:   u8,
    pub technical:    u8,
    pub system_design: u8,
    pub to_ask:       u8,
    pub culture_fit:  u8,
}

/// Classification of a generated interview question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCategory {
    Behavioral,         // STAR-method answer expected
    Technical,          // skill-specific, JD-keyword-driven
    SystemDesign,       // architecture / design scope
    CultureFit,         // values / motivations
    ToAskInterviewer,   // candidate's questions for the panel
}

/// Seniority level inferred from JD signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeniorityLevel {
    Junior,    // IC1–IC3 equivalents
    Mid,       // IC4
    Senior,    // IC5 / Senior Engineer
    Staff,     // IC6 / Staff / Principal
    Manager,
    Director,
}

/// A single generated interview question with coaching metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewQuestion {
    pub id: Uuid,
    /// The question text shown to the candidate.
    pub question: String,
    pub category: QuestionCategory,
    /// What the interviewer is assessing — shown in the coaching panel.
    pub what_evaluator_looks_for: String,
    /// Actionable tip for answering this question well.
    pub tip: String,
    /// For behavioral questions, a pointer to a matching LifeSheet work experience.
    /// `None` when no story matches or for non-behavioral categories.
    pub candidate_story_ref: Option<Uuid>,
    /// Matched JD keywords that motivated this question. Used for highlighting.
    pub source_keywords: Vec<String>,
}

/// Verified context assembled before any LLM call.
/// All fields come from persisted data — nothing is invented.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepContext {
    pub job_title: String,
    pub company_name: String,
    /// Seniority inferred from JD text (e.g. "Staff Engineer", "L5 / Senior").
    pub seniority_level: SeniorityLevel,
    /// Raw seniority signal strings extracted from JD for transparency.
    pub seniority_signals: Vec<String>,
    /// Required skills extracted from JD by `JdParser`.
    pub required_skills: Vec<String>,
    /// Preferred/nice-to-have skills from JD.
    pub preferred_skills: Vec<String>,
    /// Skills in `required_skills` that the candidate is missing from LifeSheet.
    pub candidate_skill_gaps: Vec<String>,
    /// Signals from `CompanyRecord.interview_signals` (may be empty if not yet scraped).
    pub company_interview_signals: Vec<String>,
    /// Culture keywords from `CompanyRecord.culture_signals`.
    pub culture_keywords: Vec<String>,
    /// Whether `interview_signals` is stale (>90 days old).
    pub interview_signals_stale: bool,
    pub interview_type: InterviewType,
    /// Optional user-supplied focus areas e.g. ["distributed systems", "kafka"].
    pub focus_areas: Vec<String>,
}

/// Full input for a prep session generation request.
#[derive(Debug, Clone)]
pub struct InterviewPrepRequest {
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    /// Optional additional topics the candidate wants emphasized.
    pub focus_areas: Vec<String>,
    /// If true, skip the stale-signals warning in the UI (user acknowledged).
    pub bypass_stale_signals_warning: bool,
}

/// A persisted set of generated questions for one application + interview type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewPrepSession {
    pub id: Uuid,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub questions: Vec<InterviewQuestion>,
    pub prep_context: PrepContext,
    pub generated_at: DateTime<Utc>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/interview/repository.rs

#[async_trait::async_trait]
pub trait InterviewPrepRepository: Send + Sync + 'static {
    /// Persist a newly generated session.
    async fn save_session(&self, session: &InterviewPrepSession) -> Result<(), InterviewPrepError>;

    /// All sessions for a given application, ordered by `generated_at DESC`.
    async fn get_sessions_for_application(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<InterviewPrepSession>, InterviewPrepError>;

    /// Most recently generated session for an application.
    async fn get_latest_session(
        &self,
        application_id: Uuid,
    ) -> Result<Option<InterviewPrepSession>, InterviewPrepError>;

    /// Retrieve by primary key.
    async fn get_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<InterviewPrepSession>, InterviewPrepError>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/src/db/migrations/014_interview_prep.sql

CREATE TABLE IF NOT EXISTS interview_prep_sessions (
    id               TEXT    PRIMARY KEY NOT NULL,
    application_id   TEXT    NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type   TEXT    NOT NULL,
    questions_json   TEXT    NOT NULL,   -- serde_json of Vec<InterviewQuestion>
    prep_context_json TEXT   NOT NULL,   -- serde_json of PrepContext
    generated_at     TEXT    NOT NULL    -- ISO-8601 UTC timestamp
);

CREATE INDEX IF NOT EXISTS idx_interview_prep_sessions_application_id
    ON interview_prep_sessions(application_id);

CREATE INDEX IF NOT EXISTS idx_interview_prep_sessions_generated_at
    ON interview_prep_sessions(application_id, generated_at DESC);
```

### Module Structure

```
lazyjob-core/
  src/
    interview/
      mod.rs          -- pub use re-exports (facade pattern §7)
      types.rs        -- InterviewType, QuestionCategory, InterviewQuestion,
                         PrepContext, InterviewPrepRequest, InterviewPrepSession,
                         SeniorityLevel, QuestionMix
      context.rs      -- PrepContextBuilder (pure Rust, no LLM)
      story_map.rs    -- map_stories_to_behavioral_questions
      seniority.rs    -- infer_seniority_level (regex-based)
      service.rs      -- InterviewPrepService
      repository.rs   -- InterviewPrepRepository trait
      sqlite.rs       -- SqliteInterviewPrepRepository

lazyjob-llm/
  src/
    prompts/
      interview_prep_gen.toml   -- embedded prompt template

lazyjob-tui/
  src/
    views/
      interview_prep/
        mod.rs
        view.rs         -- InterviewPrepView
        question_list.rs -- QuestionListWidget (ratatui List)
        detail_panel.rs  -- QuestionDetailPanel (scrollable text)

lazyjob-cli/
  src/
    commands/
      interview.rs    -- `lazyjob interview prep` subcommand
```

---

## Implementation Phases

### Phase 1 — Core Types and Pure Context Assembly (MVP)

**Step 1.1 — Define all types in `lazyjob-core/src/interview/types.rs`**

Implement all structs and enums as defined in the Core Types section above:
- `InterviewType` with `question_mix()` and DB string round-trip
- `QuestionMix`, `SeniorityLevel`, `QuestionCategory`
- `InterviewQuestion`, `PrepContext`, `InterviewPrepRequest`, `InterviewPrepSession`

All types derive `Debug`, `Clone`, `Serialize`, `Deserialize`. `InterviewPrepSession.questions` is `Vec<InterviewQuestion>`, stored as a JSON blob in SQLite (single-column approach avoids a join for the common read path).

Verification: `cargo test -p lazyjob-core interview::types` — test round-trip serialization of `InterviewPrepSession`.

**Step 1.2 — Seniority inference in `lazyjob-core/src/interview/seniority.rs`**

```rust
use once_cell::sync::Lazy;
use regex::Regex;

// Ordered longest-match-first so "principal staff" doesn't match "staff" first.
static STAFF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(principal|distinguished|fellow|staff\s+engineer|ic6|l6|e6)\b").unwrap()
});
static SENIOR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(senior|sr\.?\s+engineer|ic5|l5|e5)\b").unwrap()
});
static MID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(mid-?level|ic4|l4|e4)\b").unwrap()
});
static MANAGER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(engineering\s+manager|em\b|team\s+lead)\b").unwrap()
});
static DIRECTOR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(director|vp\s+of\s+engineering|head\s+of\s+engineering)\b").unwrap()
});

/// Extract seniority signals and infer a `SeniorityLevel` from raw JD text.
pub fn infer_seniority(jd_text: &str) -> (SeniorityLevel, Vec<String>) {
    let mut signals = Vec::new();
    for m in DIRECTOR_RE.find_iter(jd_text)  { signals.push(m.as_str().to_owned()); }
    for m in MANAGER_RE.find_iter(jd_text)   { signals.push(m.as_str().to_owned()); }
    for m in STAFF_RE.find_iter(jd_text)     { signals.push(m.as_str().to_owned()); }
    for m in SENIOR_RE.find_iter(jd_text)    { signals.push(m.as_str().to_owned()); }
    for m in MID_RE.find_iter(jd_text)       { signals.push(m.as_str().to_owned()); }
    signals.dedup();

    // Use the highest-priority signal found.
    let level = if DIRECTOR_RE.is_match(jd_text) {
        SeniorityLevel::Director
    } else if MANAGER_RE.is_match(jd_text) {
        SeniorityLevel::Manager
    } else if STAFF_RE.is_match(jd_text) {
        SeniorityLevel::Staff
    } else if SENIOR_RE.is_match(jd_text) {
        SeniorityLevel::Senior
    } else if MID_RE.is_match(jd_text) {
        SeniorityLevel::Mid
    } else {
        SeniorityLevel::Junior
    };

    (level, signals)
}
```

Verification: unit tests covering each seniority variant; test that "Staff Software Engineer" matches `Staff` and that "Senior Staff" matches `Staff` not `Senior`.

**Step 1.3 — `PrepContextBuilder` in `lazyjob-core/src/interview/context.rs`**

`PrepContextBuilder` is a pure Rust struct (no async, no LLM calls). It takes references to pre-loaded domain objects.

```rust
pub struct PrepContextBuilder<'a> {
    job: &'a JobListing,
    jd_analysis: &'a JobDescriptionAnalysis,  // from JdParser (already computed)
    company: &'a CompanyRecord,
    life_sheet: &'a LifeSheet,
}

impl<'a> PrepContextBuilder<'a> {
    pub fn new(
        job: &'a JobListing,
        jd_analysis: &'a JobDescriptionAnalysis,
        company: &'a CompanyRecord,
        life_sheet: &'a LifeSheet,
    ) -> Self { ... }

    pub fn build(
        &self,
        interview_type: InterviewType,
        focus_areas: Vec<String>,
    ) -> PrepContext {
        let (seniority_level, seniority_signals) =
            infer_seniority(&self.job.description);

        let required_skills: Vec<String> = self.jd_analysis
            .required_skills.iter().map(|s| s.name.clone()).collect();

        let preferred_skills: Vec<String> = self.jd_analysis
            .preferred_skills.iter().map(|s| s.name.clone()).collect();

        // Build candidate skill set from LifeSheet (explicit + inferred).
        let candidate_skills = self.collect_candidate_skills();

        // Gap = required JD skills not found in candidate skills.
        // Use SkillNormalizer from gap_analysis for consistent normalization.
        let candidate_skill_gaps = self.compute_skill_gaps(&required_skills, &candidate_skills);

        // Company signals + staleness check.
        let interview_signals_stale = self.company.interview_signals_fetched_at
            .map(|t| Utc::now() - t > chrono::Duration::days(90))
            .unwrap_or(true);

        PrepContext {
            job_title: self.job.title.clone(),
            company_name: self.company.name.clone(),
            seniority_level,
            seniority_signals,
            required_skills,
            preferred_skills,
            candidate_skill_gaps,
            company_interview_signals: self.company.interview_signals.clone(),
            culture_keywords: self.company.culture_signals.clone(),
            interview_signals_stale,
            interview_type,
            focus_areas,
        }
    }

    fn collect_candidate_skills(&self) -> Vec<String> {
        let mut skills: Vec<String> = self.life_sheet.skills
            .iter().map(|s| s.name.to_lowercase()).collect();
        // Also include tech_stack from all work experiences.
        for exp in &self.life_sheet.work_experiences {
            for tech in &exp.tech_stack {
                skills.push(tech.to_lowercase());
            }
        }
        skills.sort();
        skills.dedup();
        skills
    }

    fn compute_skill_gaps(
        &self,
        required: &[String],
        candidate: &[String],
    ) -> Vec<String> {
        required.iter().filter(|req| {
            let req_lower = req.to_lowercase();
            // Exact match first.
            if candidate.contains(&req_lower) { return false; }
            // Jaro-Winkler fuzzy: if any candidate skill is >= 0.88, consider it covered.
            candidate.iter().any(|c| strsim::jaro_winkler(&req_lower, c) >= 0.88)
            .not()  // gap if NOT covered
        }).cloned().collect()
    }
}
```

Verification: unit test with a hand-crafted `JobListing` + `CompanyRecord` + `LifeSheet` confirming that a skill in `required_skills` that the candidate has is not in `candidate_skill_gaps`, and a missing skill is.

**Step 1.4 — STAR story mapping in `lazyjob-core/src/interview/story_map.rs`**

Maps generated behavioral questions to LifeSheet work experience entries by keyword overlap.

```rust
use strsim::jaro_winkler;

/// After the LLM generates `Vec<InterviewQuestion>`, this function mutates
/// each `Behavioral` question to set `candidate_story_ref` to the most
/// relevant work experience UUID from the LifeSheet.
pub fn map_stories_to_behavioral_questions(
    questions: &mut Vec<InterviewQuestion>,
    life_sheet: &LifeSheet,
) {
    for q in questions.iter_mut() {
        if q.category != QuestionCategory::Behavioral {
            continue;
        }
        let best = life_sheet.work_experiences.iter()
            .max_by_key(|exp| {
                // Score = number of question words that fuzzy-match any word in
                // the experience description + achievement bullets.
                let exp_text = format!(
                    "{} {} {}",
                    exp.title,
                    exp.description.as_deref().unwrap_or(""),
                    exp.achievements.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(" ")
                ).to_lowercase();

                let q_words: Vec<&str> = q.question.split_whitespace().collect();
                q_words.iter().filter(|w| {
                    // Skip stopwords (short words).
                    if w.len() < 4 { return false; }
                    exp_text.contains(&w.to_lowercase())
                        || exp_text.split_whitespace().any(|ew| jaro_winkler(w, ew) >= 0.88)
                }).count()
            });

        if let Some(exp) = best {
            // Only link if at least 2 keywords matched (threshold prevents spurious links).
            let exp_text = format!("{} {}", exp.title, exp.description.as_deref().unwrap_or("")).to_lowercase();
            let match_count = q.question.split_whitespace()
                .filter(|w| w.len() >= 4 && exp_text.contains(&w.to_lowercase()))
                .count();

            if match_count >= 2 {
                q.candidate_story_ref = Some(exp.id);
            }
        }
    }
}
```

Verification: unit test with a LifeSheet containing two experiences and a behavioral question about "conflict resolution" — confirm it links to the right experience.

---

### Phase 2 — LLM Integration and Prompt Design

**Step 2.1 — Prompt template in `lazyjob-llm/src/prompts/interview_prep_gen.toml`**

```toml
[template]
loop_type = "interview_prep_gen"
version = "1"
cache_system_prompt = true

[system]
text = """
You are an expert interview coach preparing a candidate for a {interview_type_label} interview
at {company_name} for the role of {job_title} ({seniority_label}).

Ground rules:
- Generate ONLY questions grounded in the provided context.
- Do NOT invent company-specific interview processes, frameworks, or values not present in
  `company_interview_signals` or `culture_keywords`.
- Each question MUST relate to at least one item in `required_skills`, `culture_keywords`,
  `company_interview_signals`, or `focus_areas`.
- The `what_evaluator_looks_for` field must be 1–2 sentences, factual, and evaluator-perspective.
- The `tip` field must be actionable and specific to this question.
- Return ONLY valid JSON matching the schema below. No markdown fences, no commentary.
"""

[user]
text = """
Context:
{prep_context_json}

Generate {question_count} interview questions with this distribution:
{question_mix_instructions}

JSON schema for each question:
{
  "id": "<uuidv4>",
  "question": "<string>",
  "category": "<behavioral|technical|system_design|culture_fit|to_ask_interviewer>",
  "what_evaluator_looks_for": "<string>",
  "tip": "<string>",
  "candidate_story_ref": null,
  "source_keywords": ["<keyword>", ...]
}

Return a JSON array of questions. No other text.
"""
```

The system prompt is marked `cache_system_prompt = true` for Anthropic prompt caching. The `PrepContext` is serialized to JSON and injected as `{prep_context_json}` — this is the only LLM input. No raw JD text reaches the LLM.

**Step 2.2 — `InterviewPrepService` in `lazyjob-core/src/interview/service.rs`**

```rust
pub struct InterviewPrepService {
    llm: Arc<dyn LlmProvider>,
    company_repo: Arc<dyn CompanyRepository>,
    job_repo: Arc<dyn JobRepository>,
    life_sheet_repo: Arc<dyn LifeSheetRepository>,
    jd_parser: JdParser,
    prep_repo: Arc<dyn InterviewPrepRepository>,
}

impl InterviewPrepService {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        company_repo: Arc<dyn CompanyRepository>,
        job_repo: Arc<dyn JobRepository>,
        life_sheet_repo: Arc<dyn LifeSheetRepository>,
        prep_repo: Arc<dyn InterviewPrepRepository>,
    ) -> Self { ... }

    /// Main entry point. Loads all required data, builds PrepContext,
    /// calls LLM, maps stories, persists session.
    pub async fn generate_prep_session(
        &self,
        job: &JobListing,
        request: &InterviewPrepRequest,
    ) -> Result<InterviewPrepSession, InterviewPrepError> {
        // 1. Load company record.
        let company = self.company_repo
            .get_by_name_normalized(&company_normalize(&job.company_name))
            .await?
            .unwrap_or_else(|| CompanyRecord::stub(&job.company_name));

        // 2. Load (or compute) JD analysis — check cache first.
        let jd_analysis = self.jd_parser.parse_or_load_cached(job).await?;

        // 3. Load LifeSheet.
        let life_sheet = self.life_sheet_repo.load_current().await?;

        // 4. Build PrepContext (pure, no LLM).
        let prep_context = PrepContextBuilder::new(job, &jd_analysis, &company, &life_sheet)
            .build(request.interview_type, request.focus_areas.clone());

        // 5. Warn on stale signals (non-fatal).
        if prep_context.interview_signals_stale && !request.bypass_stale_signals_warning {
            tracing::warn!(
                company = %company.name,
                "interview_signals are stale or missing; falling back to generic questions"
            );
            return Err(InterviewPrepError::StaleInterviewSignals {
                company: company.name.clone(),
                last_fetched: company.interview_signals_fetched_at,
            });
        }

        // 6. Render prompt template.
        let prompt = self.render_prompt(&prep_context)?;

        // 7. LLM call.
        let raw_json = self.llm.complete(CompletionRequest {
            messages: vec![
                ChatMessage::system(prompt.system_text),
                ChatMessage::user(prompt.user_text),
            ],
            max_tokens: 4096,
            temperature: Some(0.3),  // Lower temperature for structured output.
            ..Default::default()
        }).await?;

        // 8. Parse JSON array of questions.
        let mut questions: Vec<InterviewQuestion> =
            serde_json::from_str(&raw_json.content)
                .map_err(|e| InterviewPrepError::LlmOutputParseFailed(e.to_string()))?;

        // 9. Map behavioral questions to LifeSheet stories.
        map_stories_to_behavioral_questions(&mut questions, &life_sheet);

        // 10. Assemble session.
        let session = InterviewPrepSession {
            id: Uuid::new_v4(),
            application_id: request.application_id,
            interview_type: request.interview_type,
            questions,
            prep_context,
            generated_at: Utc::now(),
        };

        // 11. Persist.
        self.prep_repo.save_session(&session).await?;

        Ok(session)
    }

    fn render_prompt(&self, ctx: &PrepContext) -> Result<RenderedPrompt, InterviewPrepError> {
        let mix = ctx.interview_type.question_mix();
        let total = mix.behavioral + mix.technical + mix.system_design + mix.to_ask + mix.culture_fit;

        let mix_instructions = format!(
            "- {} behavioral (STAR-method)\n\
             - {} technical (JD skills)\n\
             - {} system design\n\
             - {} questions to ask the interviewer\n\
             - {} culture fit",
            mix.behavioral, mix.technical, mix.system_design, mix.to_ask, mix.culture_fit
        );

        let interview_type_label = match ctx.interview_type {
            InterviewType::PhoneScreen => "Phone Screen",
            InterviewType::TechnicalScreen => "Technical Screen",
            InterviewType::Behavioral => "Behavioral",
            InterviewType::OnSite => "On-Site",
            InterviewType::SystemDesign => "System Design",
            InterviewType::ExecutiveOrBarRaiser => "Executive / Bar Raiser",
        };

        let prep_context_json = serde_json::to_string_pretty(ctx)
            .map_err(|e| InterviewPrepError::SerializationFailed(e.to_string()))?;

        // Template variable substitution via SimpleTemplateEngine from spec 17.
        let engine = SimpleTemplateEngine::new(INTERVIEW_PREP_GEN_TEMPLATE);
        engine.render([
            ("interview_type_label", interview_type_label),
            ("company_name", &ctx.company_name),
            ("job_title", &ctx.job_title),
            ("seniority_label", &format!("{:?}", ctx.seniority_level)),
            ("prep_context_json", &prep_context_json),
            ("question_count", &total.to_string()),
            ("question_mix_instructions", &mix_instructions),
        ])
        .map_err(InterviewPrepError::TemplateFailed)
    }
}
```

Verification: integration test with `wiremock` mocking the Anthropic API returning a canned JSON array; assert all 5 question types are present, `candidate_story_ref` is set on at least one behavioral question.

**Step 2.3 — `SqliteInterviewPrepRepository` in `lazyjob-core/src/interview/sqlite.rs`**

```rust
pub struct SqliteInterviewPrepRepository {
    pool: sqlx::Pool<sqlx::Sqlite>,
}

#[async_trait::async_trait]
impl InterviewPrepRepository for SqliteInterviewPrepRepository {
    async fn save_session(&self, session: &InterviewPrepSession)
        -> Result<(), InterviewPrepError>
    {
        let questions_json = serde_json::to_string(&session.questions)
            .map_err(|e| InterviewPrepError::SerializationFailed(e.to_string()))?;
        let prep_context_json = serde_json::to_string(&session.prep_context)
            .map_err(|e| InterviewPrepError::SerializationFailed(e.to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO interview_prep_sessions
                (id, application_id, interview_type, questions_json, prep_context_json, generated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            session.id,
            session.application_id,
            session.interview_type.to_db_str(),
            questions_json,
            prep_context_json,
            session.generated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(InterviewPrepError::Database)?;

        Ok(())
    }

    async fn get_sessions_for_application(
        &self,
        application_id: Uuid,
    ) -> Result<Vec<InterviewPrepSession>, InterviewPrepError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, application_id, interview_type,
                   questions_json, prep_context_json, generated_at
            FROM interview_prep_sessions
            WHERE application_id = ?1
            ORDER BY generated_at DESC
            "#,
            application_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(InterviewPrepError::Database)?;

        rows.into_iter().map(|r| Self::row_to_session(r)).collect()
    }

    async fn get_latest_session(
        &self,
        application_id: Uuid,
    ) -> Result<Option<InterviewPrepSession>, InterviewPrepError> {
        let row = sqlx::query!(
            r#"
            SELECT id, application_id, interview_type,
                   questions_json, prep_context_json, generated_at
            FROM interview_prep_sessions
            WHERE application_id = ?1
            ORDER BY generated_at DESC
            LIMIT 1
            "#,
            application_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(InterviewPrepError::Database)?;

        row.map(Self::row_to_session).transpose()
    }

    async fn get_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<InterviewPrepSession>, InterviewPrepError> {
        let row = sqlx::query!(
            r#"
            SELECT id, application_id, interview_type,
                   questions_json, prep_context_json, generated_at
            FROM interview_prep_sessions
            WHERE id = ?1
            "#,
            session_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(InterviewPrepError::Database)?;

        row.map(Self::row_to_session).transpose()
    }
}

impl SqliteInterviewPrepRepository {
    fn row_to_session(r: /* sqlx row */) -> Result<InterviewPrepSession, InterviewPrepError> {
        let questions: Vec<InterviewQuestion> = serde_json::from_str(&r.questions_json)
            .map_err(|e| InterviewPrepError::DeserializationFailed(e.to_string()))?;
        let prep_context: PrepContext = serde_json::from_str(&r.prep_context_json)
            .map_err(|e| InterviewPrepError::DeserializationFailed(e.to_string()))?;
        let interview_type = InterviewType::from_db_str(&r.interview_type)?;

        Ok(InterviewPrepSession {
            id: r.id.parse().map_err(|_| InterviewPrepError::InvalidUuid(r.id))?,
            application_id: r.application_id.parse()
                .map_err(|_| InterviewPrepError::InvalidUuid(r.application_id))?,
            interview_type,
            questions,
            prep_context,
            generated_at: r.generated_at.parse()
                .map_err(|e: chrono::ParseError| InterviewPrepError::InvalidTimestamp(e.to_string()))?,
        })
    }
}
```

Verification: `#[sqlx::test(migrations = "migrations")]` integration test that saves + loads a session and asserts all fields round-trip correctly.

---

### Phase 3 — Ralph Dispatch Integration

**Step 3.1 — `LoopType::InterviewPrepGen` in `lazyjob-ralph/src/loop_type.rs`**

Add to the existing `LoopType` enum:

```rust
InterviewPrepGen {
    application_id: Uuid,
    interview_type: InterviewType,
    focus_areas: Vec<String>,
},
```

`concurrency_limit()` returns `2` (prep sessions are user-initiated, can run concurrently with discovery). `priority()` returns `8` (user-triggered, high priority).

**Step 3.2 — Dispatch from `PostTransitionSuggestion`**

In `lazyjob-ralph/src/dispatch.rs`, the exhaustive `match` on `PostTransitionSuggestion` already has an arm for `GenerateInterviewPrep`. Implement it:

```rust
PostTransitionSuggestion::GenerateInterviewPrep { application_id, interview_type } => {
    let pushed = manager.queue.push(QueuedLoop {
        loop_type: LoopType::InterviewPrepGen {
            application_id,
            interview_type: InterviewType::from(interview_type),
            focus_areas: vec![],
        },
        priority: LoopType::InterviewPrepGen { .. }.priority(),
        enqueued_at: Utc::now(),
    });
    if !pushed {
        tracing::warn!(
            application_id = %application_id,
            "InterviewPrepGen dropped: loop queue is full"
        );
    }
}
```

**Step 3.3 — Worker implementation in `lazyjob-ralph/src/workers/interview_prep.rs`**

The worker spawns `InterviewPrepService::generate_prep_session` from within the Ralph subprocess context. It emits `WorkerEvent::Progress` with human-readable status strings during the three stages (loading context, calling LLM, mapping stories) and `WorkerEvent::Done { output_json }` on success.

```rust
pub async fn run_interview_prep_gen(
    params: InterviewPrepGenParams,
    ctx: WorkerContext,
) -> Result<(), WorkerError> {
    ctx.emit(WorkerEvent::Progress("Loading job and company data...".into())).await;

    let job = ctx.job_repo.get(params.application.job_id).await?
        .ok_or(WorkerError::JobNotFound(params.application.job_id))?;

    ctx.emit(WorkerEvent::Progress("Building prep context...".into())).await;

    let request = InterviewPrepRequest {
        application_id: params.application_id,
        interview_type: params.interview_type,
        focus_areas: params.focus_areas,
        bypass_stale_signals_warning: false,
    };

    ctx.emit(WorkerEvent::Progress("Generating questions with LLM...".into())).await;

    let session = ctx.interview_prep_service
        .generate_prep_session(&job, &request)
        .await
        .map_err(WorkerError::InterviewPrep)?;

    let output_json = serde_json::to_string(&session)
        .map_err(|e| WorkerError::Serialization(e.to_string()))?;

    ctx.emit(WorkerEvent::Done { output_json }).await;
    Ok(())
}
```

Verification: integration test that wires a mock `LlmProvider` and mock repos, runs the worker to completion, and confirms a `WorkerEvent::Done` message is emitted.

---

### Phase 4 — TUI Interview Prep View

**Step 4.1 — `InterviewPrepView` in `lazyjob-tui/src/views/interview_prep/view.rs`**

The view is accessible from the application detail view with the keybinding `p` (prep). It holds:
- A `ListState` for the question list
- The currently selected `InterviewPrepSession`
- A `FocusTarget` enum (`{ List, Detail }`), toggled with `Tab`

Layout (60/40 horizontal split):
```
┌─ Questions (5) ──────────────────────┐ ┌─ Question Detail ────────────────────────┐
│ [B] Tell me about a time you...      │ │ Category: Behavioral                      │
│ [T] Describe your experience with... │ │                                           │
│ [SD] Design a URL shortener          │ │ What they're evaluating:                  │
│ [CF] Why do you want to join us?     │ │ Ability to handle ambiguity and deliver   │
│ [?] What does the eng org look like? │ │ under pressure...                         │
└──────────────────────────────────────┘ │                                           │
                                         │ Tip: Use the STAR framework. Focus on...  │
                                         │                                           │
                                         │ Your story: (IC5 → Staff migration, 2023) │
                                         └───────────────────────────────────────────┘
```

Category badges:
- `[B]` = Behavioral (yellow)
- `[T]` = Technical (cyan)
- `[SD]` = System Design (blue)
- `[CF]` = Culture Fit (green)
- `[?]` = To Ask Interviewer (magenta)

The `candidate_story_ref` UUID is resolved at render time by calling `LifeSheetRepository::get_experience(uuid)` and rendering the experience title + date range in muted style.

**Step 4.2 — `QuestionListWidget` in `question_list.rs`**

Wraps `ratatui::widgets::List` with:
- `ListItem` per question with category badge styled span
- `ListState` for keyboard navigation (`j`/`k`, `g`/`G`)
- `?` keybinding opens an info modal showing `prep_context.interview_signals_stale` warning

```rust
pub fn render(
    &mut self,
    frame: &mut Frame,
    area: Rect,
    session: &InterviewPrepSession,
    is_focused: bool,
) {
    let items: Vec<ListItem> = session.questions.iter().map(|q| {
        let badge = category_badge(q.category);
        let line = Line::from(vec![
            Span::styled(badge, badge_style(q.category)),
            Span::raw(" "),
            Span::raw(truncate_to_width(&q.question, area.width as usize - 6)),
        ]);
        ListItem::new(line)
    }).collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(format!(" Questions ({}) ", session.questions.len()))
            .border_style(if is_focused { Style::default().fg(Color::Cyan) }
                          else { Style::default() }))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut self.list_state);
}
```

**Step 4.3 — `QuestionDetailPanel` in `detail_panel.rs`**

Scrollable `Paragraph` widget inside a `Block`. On `Enter` from the list widget, focus transitions to the detail panel, enabling `j`/`k` scrolling within the detail text.

Shows:
1. Question text (bold)
2. Category + source keywords (muted)
3. Evaluator notes section
4. Tip section
5. "Your story:" section (only for behavioral with `candidate_story_ref`)

If `prep_context.interview_signals_stale`, a yellow warning bar is prepended:
```
⚠ Company interview signals are >90 days old. Questions may be generic.
```

Verification: snapshot test using `ratatui::backend::TestBackend`, asserting the rendered text contains the warning bar when `interview_signals_stale = true`.

---

### Phase 5 — CLI Subcommand and Session Management

**Step 5.1 — `lazyjob interview prep` in `lazyjob-cli/src/commands/interview.rs`**

```rust
/// lazyjob interview prep <application-id> [--type phone|technical|behavioral|onsite|system-design]
#[derive(Debug, clap::Parser)]
pub struct InterviewPrepCommand {
    pub application_id: Uuid,
    #[arg(long, default_value = "behavioral")]
    pub r#type: InterviewTypeArg,
    #[arg(long, num_args = 0..)]
    pub focus: Vec<String>,
}

impl InterviewPrepCommand {
    pub async fn run(self, deps: &AppDeps) -> anyhow::Result<()> {
        let job = deps.job_repo.get_for_application(self.application_id).await?
            .context("application not found")?;

        let request = InterviewPrepRequest {
            application_id: self.application_id,
            interview_type: self.r#type.into(),
            focus_areas: self.focus,
            bypass_stale_signals_warning: false,
        };

        let session = deps.interview_prep_service
            .generate_prep_session(&job, &request)
            .await?;

        println!("Generated {} questions for {} at {}",
            session.questions.len(),
            session.prep_context.job_title,
            session.prep_context.company_name,
        );

        for (i, q) in session.questions.iter().enumerate() {
            println!("\n[{}] {:?}: {}", i + 1, q.category, q.question);
            println!("  Tip: {}", q.tip);
        }

        Ok(())
    }
}
```

**Step 5.2 — Session history browser in TUI**

In the `InterviewPrepView`, pressing `h` opens a session picker that shows all past sessions for the application ordered by `generated_at` DESC. Each row shows: interview type, question count, and timestamp. Selecting a row replaces the current session in the view without re-generating.

---

## Key Crate APIs

| Crate | API used | Purpose |
|-------|----------|---------|
| `sqlx` | `sqlx::query!()`, `#[sqlx::test(migrations = "...")]` | Compile-time SQL, test harness |
| `serde_json` | `serde_json::to_string()`, `serde_json::from_str()` | JSON blob serialization |
| `uuid` | `Uuid::new_v4()` | Question and session IDs |
| `chrono` | `Utc::now()`, `DateTime<Utc>` | Timestamps |
| `once_cell` | `Lazy<Regex>` | Compiled seniority regex patterns |
| `regex` | `Regex::new()`, `Regex::find_iter()`, `is_match()` | Seniority signal extraction |
| `strsim` | `strsim::jaro_winkler(a, b) -> f64` | Fuzzy skill and story matching |
| `async-trait` | `#[async_trait]` | `InterviewPrepRepository` object safety |
| `ratatui` | `List`, `ListState`, `ListItem`, `Paragraph`, `Block`, `Frame::render_stateful_widget` | TUI question list and detail panel |
| `tracing` | `tracing::warn!()`, `#[tracing::instrument]` | Stale-signals warning, function spans |

---

## Error Handling

```rust
// lazyjob-core/src/interview/error.rs

#[derive(Debug, thiserror::Error)]
pub enum InterviewPrepError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("LLM completion failed: {0}")]
    LlmFailed(String),

    #[error("LLM output could not be parsed as JSON: {0}")]
    LlmOutputParseFailed(String),

    #[error("serialization failed: {0}")]
    SerializationFailed(String),

    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("invalid interview type string: {0}")]
    InvalidInterviewType(String),

    #[error("invalid UUID: {0}")]
    InvalidUuid(String),

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("prompt template rendering failed: {0}")]
    TemplateFailed(String),

    #[error(
        "company interview signals for '{company}' are stale \
         (last fetched: {last_fetched:?}); re-run company research first"
    )]
    StaleInterviewSignals {
        company: String,
        last_fetched: Option<DateTime<Utc>>,
    },

    #[error("job not found for application")]
    JobNotFound(Uuid),

    #[error("application not found: {0}")]
    ApplicationNotFound(Uuid),
}

pub type Result<T> = std::result::Result<T, InterviewPrepError>;
```

`StaleInterviewSignals` is a **non-fatal** error in the TUI: the view renders it as a dismissable yellow banner with a `[r]efresh` keybinding that enqueues a `LoopType::CompanyResearch`. In the CLI, it is a hard error.

---

## Testing Strategy

### Unit Tests

**`lazyjob-core/src/interview/seniority.rs`** — test `infer_seniority`:
- `"Senior Software Engineer"` → `SeniorityLevel::Senior`, signals = `["Senior"]`
- `"Staff Engineer (IC6)"` → `Staff`
- `"Engineering Manager"` → `Manager`
- `"Software Engineer"` (no signal) → `Junior`
- `"Principal Staff Engineer"` → `Staff` (not `Senior`)

**`lazyjob-core/src/interview/context.rs`** — test `PrepContextBuilder::build`:
- JD requires `["Rust", "Kafka"]`, LifeSheet has `["Rust"]` → `candidate_skill_gaps = ["Kafka"]`
- JD requires `["Python"]`, LifeSheet has `["python"]` (case difference) → no gap (normalization works)
- Jaro-Winkler: `"TypeScript"` vs `"typescript"` → covered

**`lazyjob-core/src/interview/story_map.rs`** — test `map_stories_to_behavioral_questions`:
- Question about "conflict resolution", experience 1 is "Led product team through org restructuring conflict" (should link), experience 2 is "Built Kafka consumers" (should not link)
- Confirm `candidate_story_ref` is `Some(exp1.id)`

**`lazyjob-core/src/interview/types.rs`** — round-trip serde:
- `InterviewPrepSession` → JSON → `InterviewPrepSession`, field equality

**`InterviewType::question_mix()`** — assert each variant returns correct counts, all counts sum correctly.

### Integration Tests (SQLite)

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_save_and_load_session(pool: sqlx::Pool<sqlx::Sqlite>) {
    let repo = SqliteInterviewPrepRepository::new(pool);
    let session = fake_session(InterviewType::OnSite);
    repo.save_session(&session).await.unwrap();

    let loaded = repo.get_latest_session(session.application_id).await.unwrap();
    assert_eq!(loaded.unwrap().id, session.id);

    let all = repo.get_sessions_for_application(session.application_id).await.unwrap();
    assert_eq!(all.len(), 1);
}
```

### Service Integration Test with wiremock

```rust
#[tokio::test]
async fn test_generate_session_full_pipeline() {
    let mock_server = wiremock::MockServer::start().await;

    let question_json = serde_json::json!([
        {
            "id": Uuid::new_v4(),
            "question": "Tell me about a time you led a distributed systems migration.",
            "category": "behavioral",
            "what_evaluator_looks_for": "Ownership, technical depth, cross-team collaboration.",
            "tip": "Use STAR. Focus on the scale of the system and your specific decisions.",
            "candidate_story_ref": null,
            "source_keywords": ["distributed systems", "migration"]
        }
    ]);

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/messages"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .set_body_json(anthropic_response_for(question_json.to_string())))
        .mount(&mock_server)
        .await;

    let llm = AnthropicProvider::with_base_url(mock_server.uri(), "test-key");
    // ... build all mock repos, call generate_prep_session, assert session.questions.len() == 1
}
```

### TUI Tests

```rust
#[test]
fn test_stale_signals_warning_visible() {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    let session = fake_session_with_stale_signals();

    terminal.draw(|f| {
        let view = InterviewPrepView::new(session);
        view.render(f, f.area());
    }).unwrap();

    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content.iter().map(|c| c.symbol().to_owned()).collect();
    assert!(content.contains("Company interview signals are >90 days old"));
}
```

---

## Open Questions

1. **System design depth vs. role seniority**: The plan infers seniority from JD text via regex, which covers common title formats. However, level strings vary wildly across companies (L5 at Amazon ≠ L5 at Google for scope expectations). A user-configurable `[profile] target_level = "senior"` key in `lazyjob.toml` should override the inferred level in Phase 2 to allow explicit control. This is not implemented in Phase 1 MVP.

2. **Stale company interview signals — TUI UX**: The current plan returns `InterviewPrepError::StaleInterviewSignals` as a non-fatal error, causing the TUI to show a dismissable warning. An alternative is to silently fall back to generic questions and show a muted indicator. The former is more transparent and guides users to re-run company research; the latter is less interruptive. Decision deferred to UX review.

3. **Candidate questions to ask**: "Questions to ask the interviewer" are low-risk LLM-generated content but can still hurt impressions if they reveal ignorance of public company information. Phase 2 should add a flag `[interview_prep] review_candidate_questions = true` in `lazyjob.toml` that forces the TUI to show a confirmation step before surfacing them, defaulting to `false` for MVP to avoid friction.

4. **Question dedup across sessions**: If a user generates multiple sessions for the same application (different `InterviewType`), overlapping questions (e.g., "Why this company?" appearing in both PhoneScreen and OnSite) will be stored as independent records. Phase 2 should add a `question_bank` table with `UNIQUE(application_id, question_hash)` to deduplicate across sessions.

5. **Glassdoor scraping for `interview_signals`**: The company research plan defers `interview_signals` collection to Phase 2 of that spec, meaning most users will hit the stale-signals fallback path early on. The question generation should gracefully produce high-quality generic questions when `company_interview_signals` is empty — the prompt template currently handles this ("fall back to industry-standard questions for the role category") but this should be validated empirically.

---

## Related Specs

- [specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md) — uses `InterviewPrepSession.questions` as input for the mock interview turn loop
- [specs/interview-prep-agentic.md](./interview-prep-agentic.md) — autonomous research that populates `CompanyRecord.interview_signals`
- [specs/job-search-company-research.md](./job-search-company-research.md) — sources `CompanyRecord.interview_signals` and `culture_signals`
- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) — provides `WorkExperience` and `SkillEntry` for story mapping and gap computation
- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — `LoopType::InterviewPrepGen` dispatch
- [specs/application-workflow-actions.md](./application-workflow-actions.md) — `PostTransitionSuggestion::GenerateInterviewPrep` trigger
- [specs/agentic-prompt-templates.md](./agentic-prompt-templates.md) — `SimpleTemplateEngine` used for prompt rendering
- [specs/agentic-llm-provider-abstraction.md](./agentic-llm-provider-abstraction.md) — `LlmProvider`, `CompletionRequest`, `ChatMessage`
