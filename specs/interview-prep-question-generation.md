# Spec: Interview Question Generation

**JTBD**: A-5 — Prepare for interviews systematically
**Topic**: Generate a personalized, role-specific question set tailored to a company's interview style and the candidate's profile gaps.
**Domain**: interview-prep

---

## What

`InterviewPrepService` generates a structured set of interview questions for a specific application — behavioral (STAR-format), technical (based on JD-extracted skills), system design (if relevant to level/role), and "questions to ask the interviewer." It combines the job description, the candidate's LifeSheet, the `CompanyRecord` from the company research pipeline, and the interview type (phone screen, behavioral, technical, on-site) to produce context-aware questions with coaching tips for each.

## Why

Interview prep is fragmented: candidates juggle LeetCode, Glassdoor, Reddit, Blind, and company careers pages manually. This research takes 2–4 hours per company. The existing tool landscape has no product that generates a personalized question set from a job posting + candidate profile — Exponent has static "company guides," LeetCode has problem databases, but nothing connects the candidate's specific background to a specific role's signals. Without this, candidates waste prep time on irrelevant questions and miss the topics that actually appear in the target company's process.

## How

**Data inputs (pre-computed):**
1. `JobListing` — the target job (title, description, required skills, seniority signals)
2. `LifeSheet` — candidate experience, skills, STAR story bank
3. `CompanyRecord` — from `lazyjob-core/src/companies/`. Contains `interview_signals` (scraped/cached from public sources: difficulty curve, format, commonly-reported topics). See `job-search-company-research.md`.
4. `InterviewType` enum — determines question mix ratios
5. Gap analysis results — which skills/topics the candidate has not demonstrated

**Pipeline:**

```
JobDescriptionAnalysis (from profile-resume-tailoring.md JD parser)
    + LifeSheet.work_experiences + LifeSheet.skills
    + CompanyRecord.interview_signals
    → PrepContextBuilder (pure Rust, no LLM)
    → PrepContext (verified facts + candidate gaps)
    → LlmProvider::complete (prompt + JSON schema)
    → Vec<InterviewQuestion>
    → stored in interview_prep_sessions table
```

**`PrepContext` is computed before the LLM call — it is a structured grounding object, never invented.** The same pattern used in resume tailoring (`JobDescriptionAnalysis` pre-computed before the rewriting prompt) and outreach drafting (`SharedContext` pre-computed before the message draft). This is the standard LazyJob anti-fabrication pattern: compute verifiable facts first, pass to LLM as verified ground truth.

**Question mix by interview type:**
- `PhoneScreen`: 2 behavioral, 1 "why this company", 2 candidate questions to ask
- `TechnicalScreen`: 3 technical (matched to JD keywords), 1 coding conceptual, 1 candidate question
- `Behavioral`: 4 STAR behavioral (mapped to company culture signals), 1 situational
- `OnSite`: 2 behavioral, 2 technical, 1 system design (if SWE/EM/Arch), 3 candidate questions
- `SystemDesign`: 2 system design scenarios (scope-appropriate to role seniority)

**Company signal sourcing (offline-first):** `CompanyRecord.interview_signals` is populated by the company research pipeline when it runs (background ralph loop). If `interview_signals` is empty, the question generator falls back to industry-standard questions for the role category. The LLM is never asked to invent company-specific interview data — it synthesizes from the signals already in `CompanyRecord`.

**STAR story mapping:** For each behavioral question generated, the system checks `LifeSheet.experiences` for candidate stories that could answer it. If a matching story exists, it is included as a `candidate_story_ref` in the `InterviewQuestion` struct — a pointer, not a verbatim copy. The STAR coach (part of the mock loop spec) uses this linkage for behavioral feedback.

**Crate placement:** `lazyjob-core/src/interview/question_gen.rs` — pure domain logic, no TUI dependency. `InterviewPrepService` takes `Arc<dyn LlmProvider>` and `Arc<CompanyRepository>`.

## Interface

```rust
// lazyjob-core/src/interview/question_gen.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterviewType {
    PhoneScreen,
    TechnicalScreen,
    Behavioral,
    OnSite,
    SystemDesign,
    ExecutiveOrBarRaiser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuestionCategory {
    Behavioral,   // STAR method
    Technical,    // skill-specific
    SystemDesign,
    CultureFit,
    ToAskInterviewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewQuestion {
    pub question: String,
    pub category: QuestionCategory,
    pub what_evaluator_looks_for: String,
    pub tip: String,
    /// If behavioral, links to a matching LifeSheet story by experience_id
    pub candidate_story_ref: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepContext {
    pub job_title: String,
    pub company_name: String,
    pub seniority_signals: Vec<String>,       // e.g. "Staff-level", "cross-functional"
    pub required_skills: Vec<String>,          // from JD
    pub candidate_skill_gaps: Vec<String>,     // verified against LifeSheet
    pub company_interview_signals: Vec<String>,// from CompanyRecord
    pub culture_keywords: Vec<String>,         // e.g. "ownership", "customer obsession"
    pub interview_type: InterviewType,
}

pub struct InterviewPrepRequest {
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub focus_areas: Vec<String>,  // optional overrides: "system design", "distributed systems"
}

pub struct InterviewPrepSession {
    pub id: Uuid,
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub questions: Vec<InterviewQuestion>,
    pub prep_context: PrepContext,
    pub generated_at: DateTime<Utc>,
}

pub struct InterviewPrepService {
    llm: Arc<dyn LlmProvider>,
    company_repo: Arc<dyn CompanyRepository>,
    life_sheet: Arc<LifeSheet>,
    jd_parser: JdParser,
}

impl InterviewPrepService {
    pub async fn generate_prep_session(
        &self,
        job: &JobListing,
        request: &InterviewPrepRequest,
    ) -> Result<InterviewPrepSession>;

    fn build_prep_context(
        &self,
        job: &JobListing,
        company: &CompanyRecord,
        interview_type: InterviewType,
    ) -> PrepContext;

    fn map_stories_to_behavioral_questions(
        &self,
        questions: &mut Vec<InterviewQuestion>,
        life_sheet: &LifeSheet,
    );
}
```

**SQLite table:**
```sql
CREATE TABLE interview_prep_sessions (
    id          TEXT PRIMARY KEY,
    application_id TEXT NOT NULL REFERENCES applications(id),
    interview_type TEXT NOT NULL,
    questions_json TEXT NOT NULL,   -- Vec<InterviewQuestion> as JSON
    prep_context_json TEXT NOT NULL,
    generated_at TEXT NOT NULL
);
```

## Open Questions

- **Stale company interview signals**: `CompanyRecord.interview_signals` may be months old (Glassdoor data is notorious for going stale). Should we show a staleness badge in the TUI when signals are >90 days old? Or silently fall back to generic questions?
- **System design depth vs. role seniority**: Should the system infer seniority level from the JD (e.g., "Staff Engineer", "L5") and adjust system design question scope accordingly, or should the user configure their target level explicitly in `lazyjob.toml`?
- **Candidate question coaching**: "Questions to ask the interviewer" are currently generated by LLM with company context. Should these be reviewed by the user before the interview session? They're lower stakes than application content, but a bad "question to ask" can still hurt impressions.

## Implementation Tasks

- [ ] Define `InterviewType`, `QuestionCategory`, `InterviewQuestion`, `PrepContext`, `InterviewPrepSession` types in `lazyjob-core/src/interview/mod.rs`
- [ ] Implement `PrepContextBuilder` in `lazyjob-core/src/interview/context.rs` that assembles verified context from `JobListing`, `LifeSheet`, and `CompanyRecord` without LLM — refs: `job-search-company-research.md`, `profile-life-sheet-data-model.md`
- [ ] Implement `InterviewPrepService::generate_prep_session` using `LlmProvider::complete` with a structured JSON schema prompt; include mix ratios per `InterviewType` — refs: `agentic-llm-provider-abstraction.md`, `agentic-prompt-templates.md`
- [ ] Implement `map_stories_to_behavioral_questions` to link generated behavioral questions to matching `LifeSheet.work_experience` entries by keyword overlap — refs: `profile-life-sheet-data-model.md`
- [ ] Create `interview_prep_sessions` table migration in `lazyjob-core/src/db/migrations/`
- [ ] Implement `InterviewPrepRepository` trait with `save_session`, `get_sessions_for_application`, and `get_latest_session_for_application` methods
- [ ] Wire `PostTransitionSuggestion::GenerateInterviewPrep` from `application-workflow-actions.md` to dispatch a ralph loop that calls `InterviewPrepService::generate_prep_session` — refs: `agentic-ralph-orchestration.md`
- [ ] Add TUI view: "Interview Prep" panel accessible from the application detail view, showing the latest `InterviewPrepSession` with questions organized by category — refs: `architecture-tui-skeleton.md`
