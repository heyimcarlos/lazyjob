# Spec: Cover Letter Generation

**JTBD**: A-2 — Apply to jobs efficiently without repetitive manual work
**Topic**: Generate a personalized, company-researched cover letter from the user's LifeSheet and a target job description, producing a human-reviewed draft before submission.
**Domain**: profile-resume

---

## What

The cover letter generation module produces a 250–400 word cover letter draft tailored to a specific job and company. It draws on three inputs: the user's LifeSheet (for relevant experience, achievements, and career narrative), the job description (for role context and requirements), and the `CompanyRecord` from `lazyjob-core/src/companies/` (for mission, values, culture signals, and recent news). Three templates support different positioning strategies: Standard Professional, Problem-Solution, and Career Changer. Output is a human-reviewed Markdown draft and an optional DOCX file. Nothing is submitted without user approval.

## Why

Cover letters remain relevant for roles where ATS isn't the primary filter — typically mid-to-senior roles, smaller companies, referral-backed applications, and career-change narratives. A well-personalized cover letter is the only place to proactively address concerns (career gaps, pivots, over/under-qualification) that a resume can't explain. The manual effort is 20–40 minutes per letter. Automating the research and drafting reduces this to a 2-minute review cycle.

Career changers (JTBD C-1) particularly benefit from the Career Changer template, which explicitly acknowledges the non-linear background and reframes transferable skills — something generic AI tools produce poorly because they lack the structured pivot narrative from the user's LifeSheet `goals` field.

The human-in-the-loop principle is non-negotiable here: cover letters are sent directly to people and carry personal voice. The system drafts; the user owns the output.

## How

### Company Research Integration

Cover letter personalization requires company context. This module does NOT maintain its own company data store. It queries `CompanyRepository` (defined in `job-search-company-research.md`) for the `CompanyRecord` associated with the target job:

```
CompanyRecord {
    name, website, mission, culture_signals, tech_stack,
    recent_news: Vec<NewsItem>, headcount_range, funding_stage, glassdoor_rating
}
```

If `CompanyRecord` has not been populated for this company, the generation service triggers a background company research job (phase 1: website scrape + news RSS; phase 2: Crunchbase/Glassdoor). If research is unavailable, the generator falls back to JD-only personalization with a warning: "Company research not available — letter uses job description only."

This dependency on `CompanyRepository` is an explicit architecture rule: cover letter generation and interview prep are the two primary consumers of company data, and both must share the same source rather than maintaining duplicate company info.

### Template Selection

```
Template::StandardProfessional
  Best for: in-field applications, formal company culture, senior roles
  Structure: Role hook → company research hook → achievement evidence → CTA
  Word target: 300

Template::ProblemSolution
  Best for: technical roles, problem-first companies (startups, scale-ups)
  Structure: Problem identification → parallel experience → why this company → CTA
  Word target: 275

Template::CareerChanger
  Best for: career pivots, return-to-workforce, non-linear backgrounds
  Structure: Pivot narrative acknowledgment → transferable skill bridge → specific fit → CTA
  Requires: LifeSheet.goals.short_term (pivot target), TransferableSkillMap from gap analysis
  Word target: 325
```

Template auto-selection heuristic: if `life_sheet.goals` indicates a pivot AND the target role domain differs from the majority of `work_experience` domains → `CareerChanger`. Otherwise default to `ProblemSolution` for tech roles or `StandardProfessional` for enterprise/corporate.

### Generation Pipeline

```
1. Fetch CompanyRecord from CompanyRepository (or trigger research job)
2. Fetch LifeSheet from LifeSheetRepository
3. Select top 2 relevant experiences for the role (using same ProfileAnalysis logic as resume tailoring)
4. Select template (auto or user override)
5. Build generation prompt:
   - System context: role title, company name, company mission, culture signals, recent news hook
   - User context: top 2 relevant experiences with quantified achievements
   - Career goals: LifeSheet.goals.short_term (for narrative alignment)
   - Template structure instruction: numbered paragraph structure from selected template
   - Constraints: 250-400 words, no clichés ("I am writing to express my interest"), no first-person-heavy opening
6. Generate draft via LlmProvider (streaming SSE → TUI displays live typing)
7. Run anti-fabrication check: verify all achievement claims in generated text exist in LifeSheet
8. Present to user in TUI review pane:
   - Left: editable Markdown draft
   - Right: metadata (company research used, template, word count, fabrication audit)
9. User edits, approves
10. Save CoverLetterVersion to SQLite (FK to jobs.id + applications.id)
11. Optional: export to DOCX via docx-rs
```

### Anti-Fabrication in Cover Letters

Cover letters are narrative, not structured, which makes fabrication harder to detect. The anti-fabrication check for cover letters works differently than for resumes:

1. Extract all noun phrases that sound like achievement claims from the generated text using a regex pattern matching quantified metrics (numbers + units)
2. For each extracted claim, verify the metric appears in `achievement.metric_value` or `achievement.description` in the LifeSheet
3. Flag any numeric claim not found in LifeSheet as `FabricationLevel::Risky`
4. Soft-skills claims ("strong communicator", "data-driven") are not checked — they cannot be verified and are standard professional language

### Tone Calibration

```rust
pub enum CoverLetterTone {
    Professional,  // Formal language, full sentences, "I have"
    Conversational, // Warmer, contractions allowed, "I've"
    Direct,        // Short sentences, minimal filler, startup-appropriate
}
```

Tone auto-selection from CompanyRecord.culture_signals:
- "fast-paced", "startup", "move fast" → `Direct`
- "collaborative", "inclusive", "people-first" → `Conversational`
- "enterprise", "global", "Fortune 500" → `Professional`

### Version Storage

`CoverLetterVersion` is stored in `cover_letter_versions` table:
- FK to `jobs.id` (always set)
- FK to `applications.id` (set when an application is submitted)
- `content_md` (Markdown text)
- `company_research_snapshot_json` (what company data was used)
- `template_used` enum
- `created_at`

Multiple versions per job are supported (user can regenerate with different tone/template). The most recent is "active."

## Interface

```rust
// lazyjob-core/src/cover_letter/mod.rs

pub struct CoverLetterService {
    pub llm: Arc<dyn LlmProvider>,
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
    pub company_repo: Arc<dyn CompanyRepository>,  // shared with job-search domain
    pub job_repo: Arc<dyn JobRepository>,
}

impl CoverLetterService {
    /// Generate a cover letter draft for the given job.
    pub async fn generate(
        &self,
        job_id: &Uuid,
        options: CoverLetterOptions,
    ) -> Result<CoverLetterDraft>;

    /// Save an approved version linked to a job (and optionally an application).
    pub async fn save_version(
        &self,
        draft: &CoverLetterDraft,
        job_id: &Uuid,
        application_id: Option<&Uuid>,
    ) -> Result<CoverLetterVersion>;

    /// Export approved version to DOCX file.
    pub async fn export_docx(
        &self,
        version_id: &Uuid,
        path: &Path,
    ) -> Result<()>;
}

pub struct CoverLetterOptions {
    pub template: Option<CoverLetterTemplate>,  // None = auto-select
    pub tone: Option<CoverLetterTone>,           // None = auto-select from company signals
    pub max_words: u16,                          // default 350
    pub include_company_research: bool,          // default true
}

pub struct CoverLetterDraft {
    pub content_md: String,
    pub word_count: usize,
    pub template_used: CoverLetterTemplate,
    pub tone_used: CoverLetterTone,
    pub company_research: Option<CompanyRecord>,
    pub fabrication_flags: Vec<FabricationFlag>,
    pub is_approvable: bool,  // false if Forbidden fabrication flags
}

pub struct CoverLetterVersion {
    pub id: Uuid,
    pub job_id: Uuid,
    pub application_id: Option<Uuid>,
    pub content_md: String,
    pub company_research_snapshot: Option<String>,  // JSON
    pub template_used: CoverLetterTemplate,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait CoverLetterVersionRepository: Send + Sync {
    async fn save(&self, version: &CoverLetterVersion) -> Result<Uuid>;
    async fn get(&self, id: &Uuid) -> Result<CoverLetterVersion>;
    async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<CoverLetterVersion>>;
}
```

```sql
-- lazyjob-core/migrations/002_applications.sql (append)
CREATE TABLE cover_letter_versions (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id TEXT REFERENCES applications(id) ON DELETE SET NULL,
    content_md TEXT NOT NULL,
    company_research_snapshot TEXT,  -- JSON snapshot of CompanyRecord used
    template_used TEXT NOT NULL,
    tone_used TEXT NOT NULL,
    word_count INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

## Open Questions

- **Quick draft mode**: Should there be a fast path that skips company research (JD-only, 10s generation vs. 30s with research)? Proposal: always attempt research; if `CompanyRecord` is already cached for this company (from earlier discovery), research is instant. Only skip if company is unknown and user explicitly requests quick mode.
- **Multiple variants**: Should the system generate 2–3 variants for the user to choose from? Doubles LLM cost. Proposal: generate one draft; user can regenerate with different tone/template if unsatisfied.
- **Cover letter necessity detection**: Some ATS job submissions explicitly say "cover letter optional" or "no cover letter needed." Should LazyJob suppress the cover letter step in those cases? Proposal: surface the signal from the JD analysis and let the user decide; never auto-suppress.
- **Streaming to TUI**: The cover letter draft can stream live (token by token) to the TUI review pane, giving the user a sense of what's being written. This requires the TUI review view to handle incremental Markdown rendering. Architecture: LlmProvider SSE stream → RalphEvent::Status with partial content → TUI updates preview buffer.

## Implementation Tasks

- [ ] Implement `CoverLetterService::generate` in `lazyjob-core/src/cover_letter/mod.rs` — orchestrates CompanyRepository lookup, LifeSheet experience selection, template selection, and LLM generation
- [ ] Implement three prompt templates (StandardProfessional, ProblemSolution, CareerChanger) in `lazyjob-core/src/cover_letter/templates.rs` with placeholder substitution from company research and LifeSheet data
- [ ] Implement tone and template auto-selection heuristics in `lazyjob-core/src/cover_letter/selector.rs` using CompanyRecord.culture_signals and LifeSheet.goals
- [ ] Implement cover letter fabrication checker in `lazyjob-core/src/cover_letter/fabrication.rs` — extract quantified claims from generated text, verify each against LifeSheet achievement metrics
- [ ] Implement `SqliteCoverLetterVersionRepository` in `lazyjob-core/src/cover_letter/sqlite.rs` with `save`, `get`, `list_for_job`
- [ ] Implement DOCX export for cover letters via `docx-rs` in `lazyjob-core/src/cover_letter/docx.rs` — single-page letter format with name/date header
- [ ] Build TUI cover letter review view in `lazyjob-tui/src/views/cover_letter_review.rs` — left pane editable Markdown draft, right pane metadata panel (research used, word count, fabrication audit), confirm/regenerate/export actions
