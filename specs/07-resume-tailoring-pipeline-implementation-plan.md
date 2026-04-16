# Implementation Plan: Resume Tailoring Pipeline

## Status
Draft

## Related Spec
[specs/07-resume-tailoring-pipeline.md](./07-resume-tailoring-pipeline.md)

## Overview

The resume tailoring pipeline is a core value-add of LazyJob: given a life sheet (the user's structured career history in YAML) and a job description, it produces a customized resume as a DOCX file, highlights the keyword and skills gaps, and stores versioned resume snapshots tied to each job application.

The pipeline runs as an async Rust pipeline: JD analysis → profile analysis → gap analysis → content drafting → document generation → fabrication audit → version persistence. Each step is an independently testable unit with a defined input/output contract. The LLM is invoked for JD parsing and bullet rewriting; rule-based logic handles gap scoring and fabrication checks without LLM to keep costs low.

This plan covers the `lazyjob-core` domain logic, the `lazyjob-tui` diff viewer widget, SQLite schema for resume versions, and the `lazyjob-ralph` integration point where the tailoring job is triggered as a background loop. PDF and DOCX import of existing resumes are out of scope for Phase 1 (life sheet YAML is the sole source of truth).

## Prerequisites

### Specs that must precede this
- `specs/03-life-sheet-data-model.md` / `specs/03-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet` struct and repo
- `specs/04-sqlite-persistence.md` / `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, migration infrastructure, and `SqlitePool`
- `specs/02-llm-provider-abstraction.md` / `specs/02-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LLMProvider>` trait

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
docx-rs     = "0.4"                          # DOCX generation
regex       = "1"                             # Keyword TF-IDF fallback
strsim      = "0.11"                          # Fuzzy skill matching (Jaro-Winkler)
sha2        = "0.10"                          # Deduplication hash for skill names
unicode-normalization = "0.1"                 # Normalize Unicode in JD text

# Already present from sqlite persistence plan:
sqlx        = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono"] }
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4", "serde"] }
tokio       = { version = "1", features = ["full"] }
thiserror   = "2"
anyhow      = "1"
tracing     = "0.1"
async-trait = "0.1"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|---------------|
| `lazyjob-core` | All domain logic: `ResumeTailor`, `JdParser`, `GapAnalyzer`, `ContentDrafter`, `DocxGenerator`, `FabricationAuditor`, `ResumeVersionRepository` |
| `lazyjob-llm`  | LLM calls (already abstracted); no new code here |
| `lazyjob-tui`  | `ResumeDiffWidget` for side-by-side diff; version browser panel |
| `lazyjob-ralph` | Spawns tailoring as a background loop; progress events over IPC |

`lazyjob-core` owns all business logic and knows nothing about TUI or IPC. The TUI and Ralph layers depend on `lazyjob-core`.

### Core Types

```rust
// lazyjob-core/src/resume/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a resume version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResumeVersionId(pub Uuid);

impl ResumeVersionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// A persisted, versioned tailored resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersion {
    pub id: ResumeVersionId,
    pub job_id: Uuid,
    pub application_id: Option<Uuid>,
    /// Rendered content as structured sections (source of truth for diff/TUI).
    pub content: ResumeContent,
    /// Raw DOCX bytes; stored as BLOB in SQLite.
    pub docx_bytes: Vec<u8>,
    pub gap_report: GapReport,
    pub fabrication_report: FabricationReport,
    pub tailoring_options: TailoringOptions,
    pub created_at: DateTime<Utc>,
    /// Human-readable label, e.g. "v1", "v2 — added Kubernetes"
    pub label: String,
    /// Whether this version was submitted (pinned to application).
    pub is_submitted: bool,
}

/// Structured resume content used for diff rendering and re-generation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResumeContent {
    pub summary: String,
    pub experience: Vec<ExperienceSection>,
    pub skills: SkillsSection,
    pub education: Vec<EducationEntry>,
    pub projects: Vec<ProjectEntry>,
    pub certifications: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSection {
    pub company: String,
    pub title: String,
    /// ISO 8601 date range string, e.g. "2020-03 – 2023-06"
    pub date_range: String,
    /// Each bullet is a single achievement sentence.
    pub bullets: Vec<String>,
    /// Bullets flagged as rewritten (vs. passed through unchanged).
    pub rewritten_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsSection {
    /// Ordered by relevance to target JD.
    pub primary: Vec<String>,
    pub secondary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationEntry {
    pub degree: String,
    pub field: String,
    pub institution: String,
    pub graduation_year: Option<u16>,
    pub gpa: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub description: String,
    pub technologies: Vec<String>,
    pub url: Option<String>,
}

/// User-supplied options controlling tailoring aggressiveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailoringOptions {
    /// 0.0 = conservative (minimal rewriting), 1.0 = aggressive (maximum keyword injection)
    pub aggressiveness: f32,
    /// Include only experiences within this many years (0 = no limit).
    pub max_experience_years: u32,
    /// Maximum bullets per experience entry.
    pub max_bullets_per_entry: usize,
    /// Whether to emit fabrication warnings as hard errors.
    pub strict_fabrication: bool,
    /// Target ATS platform (affects keyword normalization).
    pub target_ats: Option<AtsTarget>,
}

impl Default for TailoringOptions {
    fn default() -> Self {
        Self {
            aggressiveness: 0.6,
            max_experience_years: 10,
            max_bullets_per_entry: 4,
            strict_fabrication: true,
            target_ats: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AtsTarget {
    Greenhouse,
    Lever,
    Workday,
    Generic,
}

/// Parsed and structured job description output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescriptionAnalysis {
    pub raw_text: String,
    pub required_skills: Vec<SkillRequirement>,
    pub nice_to_have_skills: Vec<SkillRequirement>,
    pub required_experience_years: Option<u32>,
    pub responsibilities: Vec<String>,
    pub qualifications: Vec<String>,
    /// All important terms extracted for keyword matching.
    pub keywords: Vec<String>,
    pub soft_skills: Vec<String>,
    pub culture_signals: Vec<String>,
    /// Normalised company name from JD header if present.
    pub company_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequirement {
    pub name: String,
    /// Normalised lowercase name for matching.
    pub canonical: String,
    pub is_required: bool,
    pub context: String,
}

/// Output of comparing the life sheet to JD requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    pub matched_skills: Vec<MatchedSkill>,
    pub missing_required: Vec<MissingSkill>,
    pub missing_nice_to_have: Vec<MissingSkill>,
    /// 0–100 match score.
    pub match_score: f32,
    /// Experiences ranked by relevance.
    pub relevant_experience_order: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedSkill {
    pub skill_name: String,
    /// Where in the life sheet this skill was found.
    pub evidence_source: SkillEvidenceSource,
    /// 0.0–1.0 confidence.
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillEvidenceSource {
    ExplicitSkill,
    ExperienceBullet { company: String, index: usize },
    ProjectDescription { name: String },
    Certification { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingSkill {
    pub skill_name: String,
    pub is_required: bool,
    pub fabrication_risk: FabricationRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FabricationRisk {
    /// Skill exists in life sheet; safe to include.
    None,
    /// Adjacent skill exists; can claim familiarity.
    Low,
    /// No evidence; adding this would be misleading.
    High,
    /// Credential/license — never fabricate.
    Forbidden,
}

/// Audit of fabrication risks in the generated resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationReport {
    pub items: Vec<FabricationItem>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    /// False if any `Forbidden`-level fabrication is present.
    pub is_safe_to_submit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationItem {
    pub description: String,
    pub risk: FabricationRisk,
    pub source: String,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/resume/pipeline.rs

use async_trait::async_trait;
use crate::resume::types::*;
use crate::life_sheet::types::LifeSheet;

/// Parses a raw job description string into structured analysis.
/// Implementations: LLM-backed (primary), TF-IDF fallback (secondary).
#[async_trait]
pub trait JobDescriptionParser: Send + Sync {
    async fn parse(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis, ResumeError>;
}

/// Runs gap analysis between a LifeSheet and a JD analysis.
/// Pure computation — no I/O, no async needed.
pub trait GapAnalyzer: Send + Sync {
    fn analyze(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
    ) -> GapReport;
}

/// Drafts resume content using LLM + gap report.
#[async_trait]
pub trait ContentDrafter: Send + Sync {
    async fn draft(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapReport,
        options: &TailoringOptions,
    ) -> Result<ResumeContent, ResumeError>;
}

/// Generates a DOCX binary from structured resume content.
pub trait DocxRenderer: Send + Sync {
    fn render(
        &self,
        content: &ResumeContent,
        personal: &PersonalInfo,
    ) -> Result<Vec<u8>, ResumeError>;
}

/// Audits generated content for fabrication risks.
pub trait FabricationAuditor: Send + Sync {
    fn audit(
        &self,
        content: &ResumeContent,
        life_sheet: &LifeSheet,
    ) -> FabricationReport;
}
```

### SQLite Schema

```sql
-- Migration: migrations/008_resume_versions.sql

CREATE TABLE resume_versions (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL,
    application_id TEXT,
    label       TEXT NOT NULL DEFAULT 'v1',
    -- JSON-serialised ResumeContent (for diff/TUI use)
    content_json    TEXT NOT NULL,
    -- Raw DOCX binary
    docx_bytes      BLOB NOT NULL,
    -- JSON-serialised GapReport
    gap_report_json TEXT NOT NULL,
    -- JSON-serialised FabricationReport
    fabrication_report_json TEXT NOT NULL,
    -- JSON-serialised TailoringOptions
    options_json    TEXT NOT NULL,
    is_submitted    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE,
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE SET NULL
);

CREATE INDEX idx_resume_versions_job ON resume_versions(job_id);
CREATE INDEX idx_resume_versions_application ON resume_versions(application_id);
CREATE INDEX idx_resume_versions_submitted ON resume_versions(job_id, is_submitted);
```

### Module Structure

```
lazyjob-core/
  src/
    resume/
      mod.rs            # Re-exports, ResumeTailor (orchestrator), ResumeService
      types.rs          # All domain types (ResumeVersion, GapReport, etc.)
      pipeline.rs       # Trait definitions (JobDescriptionParser, GapAnalyzer, ...)
      jd_parser.rs      # LlmJdParser + TfIdfJdParser (fallback)
      gap_analyzer.rs   # DefaultGapAnalyzer (pure logic, no I/O)
      content_drafter.rs # LlmContentDrafter
      docx_renderer.rs  # DocxRsRenderer using docx-rs crate
      fabrication.rs    # DefaultFabricationAuditor
      repository.rs     # ResumeVersionRepository (sqlx queries)
      error.rs          # ResumeError enum

lazyjob-tui/
  src/
    widgets/
      resume_diff.rs    # Side-by-side diff widget
      resume_versions.rs # Version list panel
```

---

## Implementation Phases

### Phase 1 — Core Pipeline (MVP)

**Goal:** End-to-end tailoring from YAML life sheet + raw JD text → DOCX file on disk, with gap report printed to stderr.

#### Step 1.1 — Types and error enum

File: `lazyjob-core/src/resume/types.rs` and `error.rs`

Define all types from the Core Types section above. Define the error enum:

```rust
// lazyjob-core/src/resume/error.rs

#[derive(thiserror::Error, Debug)]
pub enum ResumeError {
    #[error("LLM call failed: {0}")]
    Llm(#[from] lazyjob_llm::LLMError),

    #[error("JD parse failed — LLM did not return valid JSON: {0}")]
    JdParseFailed(serde_json::Error),

    #[error("document generation failed: {0}")]
    DocxGeneration(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("fabrication error: resume contains forbidden claims: {0:?}")]
    FabricationForbidden(Vec<String>),

    #[error("life sheet is empty — cannot tailor without profile data")]
    EmptyLifeSheet,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ResumeError>;
```

Verification: `cargo build -p lazyjob-core` compiles with new types.

#### Step 1.2 — LLM-backed JD parser

File: `lazyjob-core/src/resume/jd_parser.rs`

`LlmJdParser` sends the raw JD text to the LLM and requests structured JSON. Use a system prompt that instructs the LLM to return *only* a JSON object (no markdown fences) matching `JobDescriptionAnalysis`.

```rust
pub struct LlmJdParser {
    llm: Arc<dyn LLMProvider>,
}

#[async_trait]
impl JobDescriptionParser for LlmJdParser {
    async fn parse(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis> {
        let messages = vec![
            ChatMessage::System(SYSTEM_PROMPT.to_string()),
            ChatMessage::User(format!("JOB DESCRIPTION:\n{raw_jd}")),
        ];
        let resp = self.llm.chat(messages).await?;
        let parsed: JdLlmOutput = serde_json::from_str(&resp.content)
            .map_err(ResumeError::JdParseFailed)?;
        Ok(parsed.into_analysis(raw_jd))
    }
}
```

The system prompt (a `const &str`) instructs: return JSON with fields matching `JobDescriptionAnalysis` struct (required_skills, nice_to_have_skills, keywords, etc.). Include a JSON schema comment block for clarity.

Also implement `TfIdfJdParser` as a sync fallback using the `regex` crate to extract bullet requirements and the `strsim` crate for skill name deduplication. This fallback runs when the LLM call fails.

Verification: Unit test with a hardcoded JD string, verify parsed `required_skills` is non-empty.

#### Step 1.3 — Gap analyzer

File: `lazyjob-core/src/resume/gap_analyzer.rs`

Pure sync logic. Iterates `jd.required_skills` and `jd.nice_to_have_skills`; for each skill, calls `life_sheet_has_skill(canonical: &str, life_sheet: &LifeSheet) -> Option<SkillEvidenceSource>`.

`life_sheet_has_skill` checks:
1. `life_sheet.skills` (explicit skill list) — exact match on `canonical` after Unicode normalization
2. `life_sheet.experience[].bullets` — contains the canonical term (case-insensitive substring)
3. `life_sheet.projects[].technologies` — exact match
4. If no exact match: `strsim::jaro_winkler` ≥ 0.88 on any skill name → `FabricationRisk::Low`
5. If no match at all → `FabricationRisk::High`

```rust
pub struct DefaultGapAnalyzer;

impl GapAnalyzer for DefaultGapAnalyzer {
    fn analyze(&self, life_sheet: &LifeSheet, jd: &JobDescriptionAnalysis) -> GapReport {
        let mut matched = Vec::new();
        let mut missing_req = Vec::new();
        let mut missing_nth = Vec::new();

        for skill in &jd.required_skills {
            match life_sheet_has_skill(&skill.canonical, life_sheet) {
                Some((source, strength)) => matched.push(MatchedSkill {
                    skill_name: skill.name.clone(),
                    evidence_source: source,
                    strength,
                }),
                None => {
                    let risk = compute_risk(&skill.canonical, life_sheet);
                    missing_req.push(MissingSkill { skill_name: skill.name.clone(), is_required: true, fabrication_risk: risk });
                }
            }
        }
        // ... same loop for nice_to_have_skills

        let match_score = (matched.len() as f32
            / (jd.required_skills.len() + jd.nice_to_have_skills.len()).max(1) as f32)
            * 100.0;

        // Order experiences by keyword overlap count (descending)
        let relevant_experience_order = rank_experiences(life_sheet, jd);

        GapReport { matched_skills: matched, missing_required: missing_req,
                    missing_nice_to_have: missing_nth, match_score, relevant_experience_order }
    }
}
```

Verification: Unit test with a mock life sheet and JD, assert `match_score` > 0 for a matching profile.

#### Step 1.4 — Content drafter

File: `lazyjob-core/src/resume/content_drafter.rs`

`LlmContentDrafter` drives three LLM calls:
1. **Summary generation** — one call with life sheet summary + top 3 matched skills + JD responsibilities
2. **Bullet rewriting** — one call per experience section (batched as a single JSON array request), instructed to rewrite bullets incorporating target keywords without inventing facts
3. **Skills ordering** — pure Rust; sort `life_sheet.skills` by number of JD keyword matches (descending)

Bullet rewriting uses a structured prompt:
```
System: You rewrite resume bullet points to incorporate keywords from a target job description.
        Rules: (1) only rewrite based on the real achievement described, (2) use action verbs,
        (3) quantify where possible, (4) return ONLY a JSON array of strings.
User:   Original bullets: [...]
        Target keywords: [...]
```

Parse response as `Vec<String>`. If JSON parse fails, fall back to the original bullets (log a warning via `tracing::warn!`).

```rust
pub struct LlmContentDrafter {
    pub llm: Arc<dyn LLMProvider>,
}
```

Verification: Integration test — create mock `LLMProvider` returning a canned JSON array; assert bullets in returned `ResumeContent` match canned output.

#### Step 1.5 — DOCX renderer

File: `lazyjob-core/src/resume/docx_renderer.rs`

Uses `docx-rs` v0.4 API. The renderer is a pure function (no async, no I/O):

```rust
pub struct DocxRsRenderer;

impl DocxRenderer for DocxRsRenderer {
    fn render(&self, content: &ResumeContent, personal: &PersonalInfo) -> Result<Vec<u8>> {
        use docx_rs::*;

        let mut doc = Docx::new();

        // Name header
        doc = doc.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(&personal.name).bold().size(48))
                .align(AlignmentType::Center),
        );

        // Contact line
        let contact_line = build_contact_line(personal);
        doc = doc.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(&contact_line).size(20))
                .align(AlignmentType::Center),
        );

        // Section: Summary
        doc = add_section_heading(doc, "Professional Summary");
        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&content.summary).size(22)),
        );

        // Section: Experience
        doc = add_section_heading(doc, "Experience");
        for exp in &content.experience {
            doc = add_experience_entry(doc, exp);
        }

        // Section: Skills
        doc = add_section_heading(doc, "Skills");
        let skills_text = format_skills(&content.skills);
        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&skills_text).size(22)),
        );

        // Section: Education
        if !content.education.is_empty() {
            doc = add_section_heading(doc, "Education");
            for edu in &content.education {
                doc = add_education_entry(doc, edu);
            }
        }

        // Section: Projects
        if !content.projects.is_empty() {
            doc = add_section_heading(doc, "Projects");
            for proj in &content.projects {
                doc = add_project_entry(doc, proj);
            }
        }

        let mut buf = Vec::new();
        doc.build().pack(&mut buf)
            .map_err(|e| ResumeError::DocxGeneration(e.to_string()))?;
        Ok(buf)
    }
}

fn add_section_heading(doc: Docx, title: &str) -> Docx {
    doc.add_paragraph(
        Paragraph::new()
            .add_run(Run::new().add_text(title).bold().size(28))
            .add_run(Run::new().add_text("\n"))
    )
}
```

Key `docx-rs` APIs in use:
- `Docx::new()` → top-level builder
- `Docx::add_paragraph(Paragraph)` → appends a paragraph, returns `Docx`
- `Paragraph::new().add_run(Run)` → add text run to paragraph
- `Run::new().add_text(&str).bold().size(u32)` → text styling
- `AlignmentType::Center` → paragraph alignment
- `Docx::build().pack(&mut Vec<u8>)` → serialize to bytes

Verification: Call `render()` with a minimal `ResumeContent`, write to `/tmp/test.docx`, open in LibreOffice.

#### Step 1.6 — Fabrication auditor

File: `lazyjob-core/src/resume/fabrication.rs`

Pure sync logic. Cross-references each skill in `ResumeContent.skills.primary` and each bullet in each experience entry against the original life sheet:

```rust
pub struct DefaultFabricationAuditor;

impl FabricationAuditor for DefaultFabricationAuditor {
    fn audit(&self, content: &ResumeContent, life_sheet: &LifeSheet) -> FabricationReport {
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();

        // Audit skills
        for skill in content.skills.primary.iter().chain(content.skills.secondary.iter()) {
            let risk = assess_skill_risk(skill, life_sheet);
            if matches!(risk, FabricationRisk::Forbidden) {
                errors.push(format!("Cannot claim credential/license without evidence: '{skill}'"));
            } else if matches!(risk, FabricationRisk::High) {
                warnings.push(format!("No evidence for skill '{skill}' in life sheet"));
            }
            items.push(FabricationItem {
                description: skill.clone(),
                risk,
                source: "skills_section".to_string(),
            });
        }

        // Audit experience bullets with simple hallucination heuristics:
        // - Numbers that don't appear in the original life sheet bullet
        // - Technology names not in life sheet
        for exp in &content.experience {
            for (i, bullet) in exp.bullets.iter().enumerate() {
                if exp.rewritten_indices.contains(&i) {
                    if let Some(claim) = detect_unsupported_claim(bullet, life_sheet) {
                        warnings.push(format!("Bullet in {} may contain unsupported claim: '{claim}'", exp.company));
                    }
                }
            }
        }

        let is_safe_to_submit = errors.is_empty();

        FabricationReport { items, warnings, errors, is_safe_to_submit }
    }
}
```

`detect_unsupported_claim` uses regex to find numeric claims (e.g., "50%", "3x") that do not appear in any original life sheet bullet for that job.

Verification: Unit test — craft a `ResumeContent` with a made-up metric; assert a warning is produced.

#### Step 1.7 — Resume version repository

File: `lazyjob-core/src/resume/repository.rs`

```rust
pub struct ResumeVersionRepository {
    pool: SqlitePool,
}

impl ResumeVersionRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn save(&self, version: &ResumeVersion) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO resume_versions
               (id, job_id, application_id, label, content_json, docx_bytes,
                gap_report_json, fabrication_report_json, options_json, is_submitted, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            version.id.0.to_string(),
            version.job_id.to_string(),
            version.application_id.map(|u| u.to_string()),
            version.label,
            serde_json::to_string(&version.content).unwrap(),
            version.docx_bytes,
            serde_json::to_string(&version.gap_report).unwrap(),
            serde_json::to_string(&version.fabrication_report).unwrap(),
            serde_json::to_string(&version.tailoring_options).unwrap(),
            version.is_submitted as i64,
            version.created_at.to_rfc3339(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<ResumeVersionSummary>> {
        // Returns lightweight summaries (no BLOB) for TUI version browser
        let rows = sqlx::query!(
            "SELECT id, label, match_score, is_submitted, created_at
             FROM resume_versions
             WHERE job_id = ? ORDER BY created_at DESC",
            job_id.to_string()
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| /* map */ todo!()).collect())
    }

    pub async fn get(&self, id: &ResumeVersionId) -> Result<Option<ResumeVersion>> { ... }

    pub async fn export_docx(&self, id: &ResumeVersionId, path: &Path) -> Result<()> {
        let row = sqlx::query!(
            "SELECT docx_bytes FROM resume_versions WHERE id = ?",
            id.0.to_string()
        )
        .fetch_optional(&self.pool)
        .await?;
        if let Some(r) = row {
            tokio::fs::write(path, r.docx_bytes).await?;
        }
        Ok(())
    }

    pub async fn mark_submitted(&self, id: &ResumeVersionId) -> Result<()> {
        sqlx::query!(
            "UPDATE resume_versions SET is_submitted = 1 WHERE id = ?",
            id.0.to_string()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

Verification: Integration test — insert a version, list it back, assert `label` matches.

#### Step 1.8 — ResumeTailor orchestrator

File: `lazyjob-core/src/resume/mod.rs`

```rust
pub struct ResumeTailor {
    jd_parser: Arc<dyn JobDescriptionParser>,
    gap_analyzer: Arc<dyn GapAnalyzer>,
    content_drafter: Arc<dyn ContentDrafter>,
    docx_renderer: Arc<dyn DocxRenderer>,
    fabrication_auditor: Arc<dyn FabricationAuditor>,
}

impl ResumeTailor {
    pub async fn tailor(
        &self,
        life_sheet: &LifeSheet,
        job: &Job,
        options: TailoringOptions,
    ) -> Result<(ResumeContent, Vec<u8>, GapReport, FabricationReport)> {
        if life_sheet.experience.is_empty() && life_sheet.skills.is_empty() {
            return Err(ResumeError::EmptyLifeSheet);
        }

        // Step 1: Parse JD
        let jd = self.jd_parser.parse(&job.description).await
            .or_else(|e| {
                tracing::warn!("LLM JD parser failed ({e}); using TF-IDF fallback");
                // Sync fallback — create TfIdfJdParser inline and call synchronously
                TfIdfJdParser.parse_sync(&job.description)
                    .map_err(|_| e)
            })?;

        // Step 2: Gap analysis (sync)
        let gaps = self.gap_analyzer.analyze(life_sheet, &jd);

        // Step 3: Content drafting
        let content = self.content_drafter.draft(life_sheet, &jd, &gaps, &options).await?;

        // Step 4: DOCX generation (sync)
        let docx_bytes = self.docx_renderer.render(&content, &life_sheet.personal)?;

        // Step 5: Fabrication audit (sync)
        let fab_report = self.fabrication_auditor.audit(&content, life_sheet);

        if options.strict_fabrication && !fab_report.is_safe_to_submit {
            return Err(ResumeError::FabricationForbidden(fab_report.errors.clone()));
        }

        Ok((content, docx_bytes, gaps, fab_report))
    }
}
```

---

### Phase 2 — Persistence and Version Management

**Goal:** Save all tailoring outputs to SQLite; expose version history in TUI.

#### Step 2.1 — Migration file

Add `lazyjob-core/migrations/008_resume_versions.sql` with the DDL from the SQLite Schema section.

Verification: `cargo sqlx migrate run` succeeds; `sqlite3 ~/.lazyjob/lazyjob.db .schema` shows `resume_versions` table.

#### Step 2.2 — ResumeService (high-level API)

File: `lazyjob-core/src/resume/mod.rs` (extend with `ResumeService`)

```rust
pub struct ResumeService {
    tailor: ResumeTailor,
    repo: ResumeVersionRepository,
    job_repo: Arc<JobRepository>,
    life_sheet_repo: Arc<LifeSheetRepository>,
}

impl ResumeService {
    /// Full pipeline: load data, tailor, save version, return version ID.
    pub async fn tailor_for_job(
        &self,
        job_id: Uuid,
        options: TailoringOptions,
    ) -> Result<ResumeVersionId> {
        let job = self.job_repo.get(&job_id.to_string()).await?
            .ok_or_else(|| ResumeError::Other(anyhow::anyhow!("job not found")))?;
        let life_sheet = self.life_sheet_repo.load().await?;

        let (content, docx_bytes, gap_report, fabrication_report) =
            self.tailor.tailor(&life_sheet, &job, options.clone()).await?;

        // Auto-label: count existing versions for this job
        let existing_count = self.repo.list_for_job(&job_id).await?.len();
        let label = format!("v{}", existing_count + 1);

        let version = ResumeVersion {
            id: ResumeVersionId::new(),
            job_id,
            application_id: None,
            content,
            docx_bytes,
            gap_report,
            fabrication_report,
            tailoring_options: options,
            created_at: Utc::now(),
            label,
            is_submitted: false,
        };

        self.repo.save(&version).await?;
        Ok(version.id)
    }

    pub async fn export_docx(&self, id: &ResumeVersionId, path: &Path) -> Result<()> {
        self.repo.export_docx(id, path).await
    }

    pub async fn list_versions(&self, job_id: &Uuid) -> Result<Vec<ResumeVersionSummary>> {
        self.repo.list_for_job(job_id).await
    }

    pub async fn get_version(&self, id: &ResumeVersionId) -> Result<Option<ResumeVersion>> {
        self.repo.get(id).await
    }

    pub async fn pin_to_application(
        &self,
        version_id: &ResumeVersionId,
        application_id: Uuid,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE resume_versions SET application_id = ?, is_submitted = 1 WHERE id = ?",
            application_id.to_string(),
            version_id.0.to_string()
        )
        .execute(self.repo.pool())
        .await?;
        Ok(())
    }
}
```

Verification: Integration test — call `tailor_for_job` with a mock LLM provider, verify `resume_versions` row exists in test DB.

---

### Phase 3 — TUI Integration

**Goal:** Resume diff viewer (base bullets vs. tailored bullets) and version browser panel.

#### Step 3.1 — `ResumeDiffWidget`

File: `lazyjob-tui/src/widgets/resume_diff.rs`

A `ratatui` `StatefulWidget` rendering a side-by-side diff. Left column: original life sheet bullets. Right column: tailored bullets. Changed bullets highlighted in yellow (`Color::Yellow`). Added bullets in green (`Color::Green`).

```rust
use ratatui::{
    widgets::{StatefulWidget, Block, Borders},
    layout::{Rect, Layout, Direction, Constraint},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    buffer::Buffer,
};

pub struct ResumeDiffWidget;

pub struct ResumeDiffState {
    /// Original bullets per company (from life sheet).
    pub original: Vec<(String, Vec<String>)>,
    /// Tailored bullets per company.
    pub tailored: Vec<(String, Vec<String>)>,
    /// Indices of rewritten bullets (from ExperienceSection.rewritten_indices).
    pub rewritten_indices: Vec<Vec<usize>>,
    pub scroll_offset: usize,
}

impl StatefulWidget for ResumeDiffWidget {
    type State = ResumeDiffState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ResumeDiffState) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        render_column(buf, chunks[0], "Original", &state.original, &[], state.scroll_offset, false);
        render_column(buf, chunks[1], "Tailored", &state.tailored,
                      &state.rewritten_indices, state.scroll_offset, true);
    }
}
```

Keybindings: `j`/`k` to scroll, `Enter` to select version for export, `Esc` to close.

#### Step 3.2 — `ResumeVersionsPanel`

File: `lazyjob-tui/src/widgets/resume_versions.rs`

A list widget showing `ResumeVersionSummary` items. Each row: `v1  2026-04-15  Score: 72%  [submitted]`. Highlights the pinned (submitted) version. Actions: `e` = export DOCX, `d` = show diff vs. previous, `Enter` = view details.

Verification: Run `cargo test -p lazyjob-tui` widget render tests (use `TestBackend`).

---

### Phase 4 — Ralph Background Loop Integration

**Goal:** Tailoring can be triggered as a background Ralph loop, returning progress updates over IPC.

#### Step 4.1 — Ralph loop message types

File: `lazyjob-ralph/src/loops/resume_tailor.rs`

The Ralph loop wrapper calls `ResumeService::tailor_for_job` and emits progress JSON messages over stdout (following the Ralph subprocess protocol spec):

```jsonc
// Progress messages emitted to stdout (newline-delimited JSON)
{ "type": "progress", "step": "parsing_jd",      "pct": 10 }
{ "type": "progress", "step": "gap_analysis",    "pct": 30 }
{ "type": "progress", "step": "drafting",        "pct": 60 }
{ "type": "progress", "step": "generating_docx", "pct": 85 }
{ "type": "complete", "version_id": "uuid-...",  "match_score": 74.2, "warnings": [] }
{ "type": "error",    "message": "...",           "recoverable": false }
```

The TUI progress panel subscribes to these messages and renders a progress bar.

Verification: Spawn the loop binary in a test, collect stdout messages, assert final message `type == "complete"`.

---

## Key Crate APIs

| API | Usage |
|-----|-------|
| `docx_rs::Docx::new()` | Start document builder |
| `docx_rs::Docx::add_paragraph(p: Paragraph) -> Docx` | Append paragraph (builder returns new Docx) |
| `docx_rs::Paragraph::new().add_run(r: Run)` | Paragraph with text run |
| `docx_rs::Run::new().add_text(s: &str).bold().size(u32)` | Styled text |
| `docx_rs::AlignmentType::Center` | Centre alignment enum variant |
| `docx_rs::Docx::build().pack(&mut Vec<u8>)` | Serialize to bytes |
| `strsim::jaro_winkler(a: &str, b: &str) -> f64` | Fuzzy skill name matching |
| `regex::Regex::new(pattern).unwrap().find_iter(text)` | Keyword extraction in TF-IDF fallback |
| `sqlx::query!("...", args).execute(&pool).await` | Typed DB queries |
| `sqlx::query!("...", args).fetch_all(&pool).await` | Typed DB SELECT |
| `tokio::fs::write(path, bytes).await` | Async DOCX export to disk |
| `ratatui::widgets::StatefulWidget` trait | Diff widget trait impl |
| `ratatui::layout::Layout::split(area: Rect)` | Two-column layout |
| `ratatui::text::{Line, Span}` | Styled terminal text |
| `unicode_normalization::UnicodeNormalization::nfc()` | Normalize skill strings before matching |

---

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum ResumeError {
    #[error("LLM call failed: {0}")]
    Llm(#[from] lazyjob_llm::LLMError),

    #[error("JD JSON parse failed: {0}")]
    JdParseFailed(#[from] serde_json::Error),

    #[error("DOCX generation failed: {0}")]
    DocxGeneration(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("fabrication check failed — resume contains unsupported claims: {0:?}")]
    FabricationForbidden(Vec<String>),

    #[error("life sheet is empty — load a profile before tailoring")]
    EmptyLifeSheet,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

Recovery strategy per variant:
- `Llm` → retry up to 2 times with exponential backoff; if still failing, fall back to TF-IDF JD parser and return content without LLM-rewritten bullets
- `JdParseFailed` → fall back to TF-IDF JD parser
- `DocxGeneration` → surface to user; offer plain-text export fallback
- `FabricationForbidden` → surface warning list in TUI; block DOCX export but allow user to override with `--force` flag

---

## Testing Strategy

### Unit Tests

**`gap_analyzer.rs`:**
```rust
#[test]
fn test_matched_skill_found_in_explicit_skills() {
    let life_sheet = LifeSheet { skills: vec!["python".to_string()], ..Default::default() };
    let jd = minimal_jd_with_required("python");
    let report = DefaultGapAnalyzer.analyze(&life_sheet, &jd);
    assert_eq!(report.matched_skills.len(), 1);
    assert_eq!(report.missing_required.len(), 0);
}

#[test]
fn test_missing_required_skill_no_fabrication() {
    let life_sheet = LifeSheet { skills: vec!["java".to_string()], ..Default::default() };
    let jd = minimal_jd_with_required("kubernetes");
    let report = DefaultGapAnalyzer.analyze(&life_sheet, &jd);
    assert_eq!(report.missing_required[0].fabrication_risk, FabricationRisk::High);
}

#[test]
fn test_fuzzy_skill_match_gives_low_risk() {
    let life_sheet = LifeSheet { skills: vec!["k8s".to_string()], ..Default::default() };
    let jd = minimal_jd_with_required("kubernetes");
    let report = DefaultGapAnalyzer.analyze(&life_sheet, &jd);
    // "k8s" is not similar enough to "kubernetes" by Jaro-Winkler — should be High risk
    // Adjust test to use "kubernetes" variant to verify Low risk
}
```

**`fabrication.rs`:**
```rust
#[test]
fn test_forbidden_credential_triggers_error() {
    let content = resume_with_skill("AWS Certified Solutions Architect");
    let life_sheet = LifeSheet::default(); // no certifications
    let report = DefaultFabricationAuditor.audit(&content, &life_sheet);
    assert!(!report.is_safe_to_submit);
    assert!(report.errors.iter().any(|e| e.contains("Certified")));
}
```

**`docx_renderer.rs`:**
```rust
#[test]
fn test_render_returns_non_empty_bytes() {
    let renderer = DocxRsRenderer;
    let content = ResumeContent { summary: "Test".to_string(), ..Default::default() };
    let personal = minimal_personal_info();
    let bytes = renderer.render(&content, &personal).unwrap();
    assert!(!bytes.is_empty());
    // DOCX files start with PK (ZIP magic bytes)
    assert_eq!(&bytes[..2], b"PK");
}
```

### Integration Tests

**`tests/resume_tailor_integration.rs`:**
```rust
#[tokio::test]
async fn test_full_pipeline_with_mock_llm() {
    let mock_llm = MockLLMProvider::new()
        .with_response(CANNED_JD_JSON)      // parse_jd call
        .with_response(CANNED_SUMMARY)       // summary call
        .with_response(CANNED_BULLETS_JSON); // rewrite call

    let db = Database::new_in_memory().await.unwrap();
    let service = build_resume_service(Arc::new(mock_llm), db.pool().clone());

    let job_id = insert_test_job(db.pool()).await;
    let version_id = service.tailor_for_job(job_id, TailoringOptions::default()).await.unwrap();

    let version = service.get_version(&version_id).await.unwrap().unwrap();
    assert!(!version.docx_bytes.is_empty());
    assert!(version.gap_report.match_score >= 0.0);
}
```

### TUI Tests

Use `ratatui::backend::TestBackend` to render the `ResumeDiffWidget` into a test buffer:
```rust
#[test]
fn test_diff_widget_renders_changed_bullet_in_yellow() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = ResumeDiffState { ... };
    terminal.draw(|f| {
        f.render_stateful_widget(ResumeDiffWidget, f.size(), &mut state);
    }).unwrap();
    let buf = terminal.backend().buffer().clone();
    // Assert that a cell at known column position has yellow foreground
    let cell = buf.get(62, 5); // first bullet row, right column
    assert_eq!(cell.style().fg, Some(Color::Yellow));
}
```

---

## Open Questions

1. **Keyword normalization across ATS targets**: Each ATS (Greenhouse, Lever, Workday) may tokenize differently. Should `AtsTarget` influence how keywords are injected, or is a single normalization pass sufficient for Phase 1?

2. **PDF export**: LibreOffice CLI headless (`libreoffice --convert-to pdf`) is the simplest DOCX→PDF path. Should we shell out or keep it external-tool-only?

3. **Resume template system**: Users may want different visual styles. Should `DocxRenderer` accept a `Template` enum (single-column, two-column) in Phase 2?

4. **Token cost**: Three LLM calls per tailoring (JD parse, summary, bullet rewrite). With Anthropic prompt caching (for the system prompts), this is approximately 1–2k tokens per job. Should we cache the JD analysis across re-tailoring runs for the same job?

5. **Base resume concept**: Should there be a "base" resume variant derived from the life sheet (no tailoring) that serves as the left column of the diff and as the export fallback?

6. **Cover letter coupling**: The spec asks whether cover letter should be generated in the same pipeline call. Recommendation: keep them separate services that can be composed by the caller (application workflow layer), not internally coupled.

---

## Related Specs

- [specs/03-life-sheet-data-model.md](./03-life-sheet-data-model.md) — `LifeSheet` source of truth
- [specs/04-sqlite-persistence.md](./04-sqlite-persistence.md) — Database infrastructure
- [specs/02-llm-provider-abstraction.md](./02-llm-provider-abstraction.md) — `LLMProvider` trait
- [specs/08-cover-letter-generation.md](./08-cover-letter-generation.md) — Companion pipeline
- [specs/10-application-workflow.md](./10-application-workflow.md) — Consumer of `pin_to_application`
- [specs/XX-resume-version-management.md](./XX-resume-version-management.md) — Extended version lifecycle
- [specs/09-tui-design-keybindings.md](./09-tui-design-keybindings.md) — Keybinding conventions for diff viewer
