# Spec: Resume Tailoring Pipeline

**JTBD**: A-2 — Apply to jobs efficiently without repetitive manual work
**Topic**: Transform the LifeSheet and a target job description into a tailored, ATS-safe DOCX resume through a 6-step LLM pipeline with fabrication guardrails.
**Domain**: profile-resume

---

## What

The resume tailoring pipeline takes two inputs — the user's LifeSheet (structured SQLite data) and a target job description — and produces a submission-ready DOCX resume. The pipeline has 6 stages: parse the JD into structured requirements, analyze the LifeSheet for relevant experience, compute skill gaps, draft tailored content (rewritten bullets, targeted summary, reordered skills), generate DOCX via `docx-rs`, and run a fabrication audit. The human reviews the fabrication report and gap analysis before approving the output. The approved version is stored against the application record for traceability.

## Why

Tailored resumes convert to interviews at 5.75–5.8% vs. 2.68–3.73% for generic — a 55–115% improvement. Yet 54% of candidates don't tailor because it takes 15–30 minutes per application. This is the single highest-ROI automation target in the entire product. The pipeline eliminates the tedious mechanics (JD analysis, keyword mapping, bullet rewriting, formatting) while keeping the human in control of truthfulness. Without fabrication guardrails, an AI pipeline could trivially improve conversion metrics by inventing skills — creating fraud risk and undermining user trust.

## How

### 6-Stage Pipeline

```
LifeSheet (SQLite) + JobDescription (text)
    │
    ▼
[1] JD Parser (LLM)
    Extract: required_skills, nice_to_have_skills, experience_level,
             responsibilities, keywords, soft_skills, culture_signals
    Output: JobDescriptionAnalysis
    Fallback: if LLM fails → TF-IDF keyword extraction from raw JD text
    │
    ▼
[2] LifeSheet Analyzer (pure Rust)
    Match experience entries to JD requirements by:
      - skill intersection (exact + alias)
      - ESCO taxonomy proximity (if codes present)
      - relevance_tags overlap
    Output: ProfileAnalysis { matched_skills, potential_matches, experience_relevance_scores }
    │
    ▼
[3] Gap Analyzer (pure Rust)
    For each required skill: classified as MATCHED | PARTIAL | MISSING
    Fabrication level per gap: Safe | Acceptable | Risky | Forbidden
    Output: GapAnalysis { matched_skills, missing_skills, fabrication_flags }
    Fabrication rules:
      Safe     → skill in LifeSheet, just needs different phrasing
      Acceptable → adjacent skill in LifeSheet (e.g., "Kafka" when user has "RabbitMQ" + distributed systems)
      Risky    → no evidence in LifeSheet; flag to user, do NOT include in resume
      Forbidden → credentials, licenses, degrees not in LifeSheet; BLOCK generation
    │
    ▼
[4] Content Drafter (LLM)
    a. Summary: 2-3 sentence targeted narrative using matched skills + years of experience
    b. Experience bullets: rewrite existing bullets to incorporate JD keywords naturally
       - Input to LLM: original bullet + top 10 JD keywords
       - Constraint: "keep achievements based on real accomplishments; do NOT add new achievements"
    c. Skills section: reorder categories to lead with JD-relevant skills
    d. Education: select relevant entries (omit ancient/irrelevant degrees if crowding)
    e. Projects: include if directly relevant to JD requirements
    Output: ResumeContent
    │
    ▼
[5] DOCX Generator (docx-rs)
    Template: single-column, ATS-safe (no tables for layout, no text boxes)
    Sections: Header → Summary → Experience → Skills → Education → Projects
    File: written to ~/.lazyjob/resumes/{job_id}_{timestamp}.docx
    │
    ▼
[6] Fabrication Audit + Human Review
    Present FabricationReport to user in TUI
    If any Forbidden items: BLOCK submission, require user to remove
    If Risky items: show yellow warning, require explicit user acknowledgment
    User approves → ResumeVersion saved to SQLite with job_id FK
    ResumeVersion stores: content snapshot, docx bytes, fabrication_report, created_at
```

### Voice Preservation Strategy

AI-rewritten bullets must preserve the user's writing style. The drafter prompt includes 3–5 bullet examples from the user's existing LifeSheet experience descriptions as "style samples." This is the primary mitigation for the uniformity problem (all AI resumes sounding the same). No style inference model is needed — few-shot exemplars in the LLM prompt are sufficient at this scale.

### Keyword Density Targeting

JD keywords are categorized into tiers:
- Tier 1 (required skills): target 2–3 mentions across summary + experience
- Tier 2 (nice-to-have): 1 mention in skills section
- Tier 3 (culture signals): 1 mention in summary ("fast-paced" etc.)

Over-optimization check: if the drafter places the same keyword >4 times, it is flagged in the fabrication report as "keyword stuffing" (detectable by recruiter, harms readability).

### Version Tracking

Every approved tailored resume is stored in `resume_versions` (FK to `applications.id`). This enables:
- Retrieval of the exact resume sent to each company
- Interview prep: "you positioned yourself as X for this company"
- Feed-back loop: mark which versions led to interviews (for future ML training)

## Interface

```rust
// lazyjob-core/src/resume/mod.rs

pub struct ResumeTailor {
    pub llm: Arc<dyn LlmProvider>,
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
}

impl ResumeTailor {
    pub async fn tailor(
        &self,
        job: &Job,
        options: TailoringOptions,
    ) -> Result<TailoredResume>;
}

pub struct TailoringOptions {
    pub max_pages: u8,            // 1 or 2
    pub include_projects: bool,
    pub format: ResumeFormat,     // AtsSimple | Modern
}

pub struct TailoredResume {
    pub content: ResumeContent,
    pub docx_bytes: Vec<u8>,
    pub gap_analysis: GapAnalysis,
    pub fabrication_report: FabricationReport,
    pub version_id: Uuid,
}

pub struct JobDescriptionAnalysis {
    pub required_skills: Vec<String>,
    pub nice_to_have_skills: Vec<String>,
    pub keywords: Vec<String>,
    pub culture_signals: Vec<String>,
    pub experience_years_required: Option<u8>,
}

pub struct GapAnalysis {
    pub matched_skills: Vec<MatchedSkill>,   // in LifeSheet + JD requires
    pub missing_skills: Vec<MissingSkill>,    // in JD but not LifeSheet
    pub match_score: f32,                     // 0.0–1.0
}

pub struct MissingSkill {
    pub name: String,
    pub is_required: bool,
    pub fabrication_level: FabricationLevel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FabricationLevel {
    Safe,       // Rephrasing of existing LifeSheet data
    Acceptable, // Adjacent skill; can add "familiar with"
    Risky,      // No evidence; flag to user, exclude from resume
    Forbidden,  // Credential/degree not earned; BLOCK
}

pub struct FabricationReport {
    pub flags: Vec<FabricationFlag>,
    pub is_submittable: bool,   // false if any Forbidden flags
    pub warnings: Vec<String>,  // shown in TUI for Risky flags
}

// Resume version persistence
pub struct ResumeVersion {
    pub id: Uuid,
    pub job_id: Uuid,
    pub application_id: Option<Uuid>,
    pub content_json: String,         // serialized ResumeContent
    pub docx_bytes: Vec<u8>,
    pub fabrication_report_json: String,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait ResumeVersionRepository: Send + Sync {
    async fn save(&self, version: &ResumeVersion) -> Result<Uuid>;
    async fn get(&self, id: &Uuid) -> Result<ResumeVersion>;
    async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<ResumeVersion>>;
}
```

## Open Questions

- **PDF reading**: Should the pipeline support importing an existing PDF/DOCX resume to bootstrap a LifeSheet for users who don't want to write YAML? LLM-based parsing can extract 90%+ of structure. Adds cold-start value at the cost of validation complexity.
- **Custom templates**: Should users be able to pick from 3–5 pre-built DOCX templates (ATS safe, visual, one-page)? `docx-rs` supports arbitrary styles. Proposal: 2 templates in Phase 1 (ATS minimal, ATS clean) — defer visual layouts.
- **Multi-format output**: PDF export in addition to DOCX. `docx-rs` doesn't produce PDF. Options: shell out to LibreOffice headless (reliable, heavy dep), use a pure-Rust PDF crate (limited formatting), or just deliver DOCX (most ATS prefer it anyway). Proposal: DOCX only in Phase 1.
- **Version pruning**: How many versions to retain per job? Proposal: keep all versions, but only the most recent is "active" (linked to application). Storage cost is negligible (each DOCX is ~50KB).

## Implementation Tasks

- [ ] Implement `JobDescriptionAnalysis::parse` in `lazyjob-core/src/resume/jd_parser.rs` — LLM extracts structured requirements from raw JD text, falls back to TF-IDF on LLM error
- [ ] Implement `GapAnalysis::compute` in `lazyjob-core/src/resume/gap_analysis.rs` — pure Rust comparison of LifeSheet skills (with ESCO alias expansion) against JD requirements, producing `FabricationLevel` per missing item
- [ ] Implement `FabricationLevel` enum and `FabricationReport::generate` in `lazyjob-core/src/resume/fabrication.rs` — uses `is_grounded_claim` from LifeSheet module as the ground truth check
- [ ] Implement `ResumeContent::draft` in `lazyjob-core/src/resume/drafter.rs` — LLM rewrites bullets with JD keywords, generates targeted summary using style examples extracted from LifeSheet
- [ ] Implement `generate_resume_docx` in `lazyjob-core/src/resume/docx_generator.rs` using `docx-rs` — single-column ATS-safe format with all sections
- [ ] Implement `SqliteResumeVersionRepository` in `lazyjob-core/src/resume/sqlite.rs` with `save`, `get`, `list_for_job` methods
- [ ] Wire `ResumeTailor::tailor` in `lazyjob-core/src/resume/mod.rs` composing all 6 pipeline stages, returning `TailoredResume` with fabrication report for TUI review step
- [ ] Add `resume_versions` table to `lazyjob-core/migrations/002_applications.sql` with FK to `jobs.id` and `applications.id`
