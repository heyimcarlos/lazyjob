# Implementation Plan: Profile Cover Letter Generation

## Status
Draft

## Related Spec
[specs/profile-cover-letter-generation.md](./profile-cover-letter-generation.md)

## Overview

The profile cover letter generation module produces personalized, company-research-backed 250–400 word cover letter drafts tailored to a specific job and company. Unlike the general cover letter generation spec (`specs/08-cover-letter-generation.md`), this plan is anchored to the **profile domain**: how the LifeSheet's structured career narrative, user voice, career goals, and transferable skill map are leveraged to produce letters that sound genuinely human rather than AI-generic.

Three template strategies address distinct positioning needs: `StandardProfessional` (in-field, corporate), `ProblemSolution` (startup/technical), and `CareerChanger` (pivots, return-to-workforce). Template and tone auto-selection derives from `CompanyRecord.culture_signals` and `LifeSheet.goals`, with full user override. Anti-fabrication enforcement scans numeric claims in the generated text against LifeSheet achievement metrics using regex extraction, flagging any unverifiable figure as `FabricationLevel::Risky` before the draft reaches the user.

This plan lives primarily in `lazyjob-core/src/cover_letter/` and composes existing infrastructure: `LifeSheetRepository`, `CompanyRepository`, `Arc<dyn LlmProvider>`, `SqlitePool`, and the fabrication oracle from the life sheet module. The TUI cover letter review view is an editor panel with live streaming draft rendering, a fabrication audit sidebar, and version history browser. DOCX export uses `docx-rs` for single-page letter formatting.

## Prerequisites

### Specs/Plans that must be implemented first
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `WorkExperience`, `Achievement`, `LifeSheetRepository`, `is_grounded_claim()`
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, `SqlitePool`, migration infrastructure
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>`, `ChatMessage`, `StreamEvent`
- `specs/job-search-company-research-implementation-plan.md` — provides `CompanyRecord`, `CompanyRepository`
- `specs/profile-resume-tailoring-implementation-plan.md` — provides `ProfileAnalysis`, `ExperienceRelevance`, `SkillNormalizer`, `TransferableSkillMap`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel focus infrastructure

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml (additions to existing dependencies)
[dependencies]
# Document generation
docx-rs            = "0.4"           # DOCX generation (single-page letter format)

# Text diffing (version history)
similar            = "2"             # Unified diff between cover letter versions

# Numeric claim extraction
regex              = "1"
once_cell          = "1"             # Lazy<Regex> patterns compiled once

# SHA-256 for content dedup
sha2               = "0.10"
hex                = "0.4"

# Already present from prior plans:
sqlx               = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
chrono             = { version = "0.4", features = ["serde"] }
uuid               = { version = "1", features = ["v4", "serde"] }
tokio              = { version = "1", features = ["full"] }
thiserror          = "2"
anyhow             = "1"
tracing            = "0.1"
async-trait        = "0.1"
strsim             = "0.11"          # Jaro-Winkler for achievement text matching

# TUI (lazyjob-tui/Cargo.toml)
ratatui            = "0.28"
crossterm          = "0.28"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All domain logic: `CoverLetterService`, `TemplateSelector`, `ToneSelector`, `PromptBuilder`, `FabricationChecker`, `SqliteCoverLetterVersionRepository`, DOCX export, all core types |
| `lazyjob-llm` | No new code; existing `Arc<dyn LlmProvider>` trait, `ChatMessage`, streaming `StreamEvent` |
| `lazyjob-tui` | `CoverLetterReviewView`, `VersionHistoryPanel`, streaming draft preview buffer |
| `lazyjob-ralph` | `LoopType::CoverLetterGeneration` subprocess loop (Phase 3) |
| `lazyjob-cli` | `lazyjob cover-letter generate <job-id>` subcommand |

`lazyjob-core` owns all state and logic. TUI, Ralph, and CLI crates are thin adapters calling into `CoverLetterService`.

### Core Types

```rust
// lazyjob-core/src/cover_letter/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Newtype wrapper for cover letter version IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct CoverLetterVersionId(pub Uuid);

impl CoverLetterVersionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Positioning strategy for the cover letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum CoverLetterTemplate {
    /// Role hook → company research hook → achievement evidence → CTA. ~300 words.
    StandardProfessional,
    /// Problem identification → parallel experience → why this company → CTA. ~275 words.
    ProblemSolution,
    /// Pivot narrative → transferable skill bridge → specific fit → CTA. ~325 words.
    /// Requires: LifeSheet.goals.short_term + TransferableSkillMap from gap analysis.
    CareerChanger,
}

impl CoverLetterTemplate {
    pub fn target_word_count(&self) -> u16 {
        match self {
            Self::StandardProfessional => 300,
            Self::ProblemSolution => 275,
            Self::CareerChanger => 325,
        }
    }

    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::StandardProfessional => "standard_professional",
            Self::ProblemSolution => "problem_solution",
            Self::CareerChanger => "career_changer",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, CoverLetterError> {
        match s {
            "standard_professional" => Ok(Self::StandardProfessional),
            "problem_solution" => Ok(Self::ProblemSolution),
            "career_changer" => Ok(Self::CareerChanger),
            other => Err(CoverLetterError::InvalidTemplate(other.to_string())),
        }
    }
}

/// Voice and register for the generated letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum CoverLetterTone {
    /// Formal language, full sentences. Enterprise / Fortune 500.
    Professional,
    /// Warmer register, contractions allowed. Collaborative / people-first.
    Conversational,
    /// Short sentences, minimal filler. Startup / move-fast culture.
    Direct,
}

impl CoverLetterTone {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::Professional => "professional",
            Self::Conversational => "conversational",
            Self::Direct => "direct",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, CoverLetterError> {
        match s {
            "professional" => Ok(Self::Professional),
            "conversational" => Ok(Self::Conversational),
            "direct" => Ok(Self::Direct),
            other => Err(CoverLetterError::InvalidTone(other.to_string())),
        }
    }
}

/// Options passed by the caller to override auto-selection defaults.
#[derive(Debug, Clone)]
pub struct CoverLetterOptions {
    /// `None` = auto-select based on LifeSheet.goals + target role domain.
    pub template: Option<CoverLetterTemplate>,
    /// `None` = auto-select from CompanyRecord.culture_signals.
    pub tone: Option<CoverLetterTone>,
    /// Maximum word count. Defaults to template's `target_word_count()`.
    pub max_words: Option<u16>,
    /// Whether to fetch and inject company research. Defaults to true.
    pub include_company_research: bool,
    /// If true, skip fabrication check (for re-generation after user edits).
    pub skip_fabrication_check: bool,
}

impl Default for CoverLetterOptions {
    fn default() -> Self {
        Self {
            template: None,
            tone: None,
            max_words: None,
            include_company_research: true,
            skip_fabrication_check: false,
        }
    }
}

/// A fabrication flag raised by the anti-fabrication checker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationFlag {
    /// The numeric claim extracted from the generated text.
    pub claim_text: String,
    /// Level of concern.
    pub level: FabricationLevel,
    /// Brief explanation for the TUI tooltip.
    pub explanation: String,
}

/// Risk level for a fabrication flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FabricationLevel {
    Safe,       // claim found in LifeSheet
    Risky,      // numeric claim not found in LifeSheet
    Forbidden,  // claim contradicts LifeSheet data
}

/// A generated, not-yet-persisted cover letter draft.
#[derive(Debug, Clone)]
pub struct CoverLetterDraft {
    pub content_md: String,
    pub word_count: usize,
    pub template_used: CoverLetterTemplate,
    pub tone_used: CoverLetterTone,
    /// The CompanyRecord snapshot used for this generation (None if research skipped).
    pub company_research: Option<CompanyResearchSnapshot>,
    pub fabrication_flags: Vec<FabricationFlag>,
    /// False if any `Forbidden` fabrication flags are present. The TUI must prevent
    /// saving unless the user has resolved all Forbidden flags.
    pub is_approvable: bool,
}

/// A snapshot of company data used during generation (serialized for storage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyResearchSnapshot {
    pub company_name: String,
    pub mission: Option<String>,
    pub culture_signals: Vec<String>,
    pub recent_news_hooks: Vec<String>,  // top 2 headlines used in prompt
    pub glassdoor_rating: Option<f32>,
    pub funding_stage: Option<String>,
}

/// A persisted cover letter version (one row in cover_letter_versions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterVersion {
    pub id: CoverLetterVersionId,
    /// FK to jobs.id.
    pub job_id: Uuid,
    /// FK to applications.id — null until application is submitted.
    pub application_id: Option<Uuid>,
    /// Monotonic per-job version counter (1-based).
    pub version_number: u32,
    /// Full Markdown content of the letter.
    pub content_md: String,
    /// Plain text (markdown stripped) for clipboard / ATS paste.
    pub plain_text: String,
    pub word_count: u32,
    pub template_used: CoverLetterTemplate,
    pub tone_used: CoverLetterTone,
    /// JSON-serialized `CompanyResearchSnapshot` used during generation.
    pub company_research_snapshot: Option<String>,
    /// SHA-256 of `content_md` bytes for dedup.
    pub content_hash: String,
    /// Unified diff from the previous version (None for version 1).
    pub diff_from_prev: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/cover_letter/repository.rs

#[async_trait]
pub trait CoverLetterVersionRepository: Send + Sync {
    async fn save(&self, version: &CoverLetterVersion) -> Result<CoverLetterVersionId, CoverLetterError>;
    async fn get(&self, id: &CoverLetterVersionId) -> Result<CoverLetterVersion, CoverLetterError>;
    async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<CoverLetterVersion>, CoverLetterError>;
    async fn get_latest_for_job(&self, job_id: &Uuid) -> Result<Option<CoverLetterVersion>, CoverLetterError>;
    async fn link_to_application(
        &self,
        version_id: &CoverLetterVersionId,
        application_id: &Uuid,
    ) -> Result<(), CoverLetterError>;
    async fn next_version_number(&self, job_id: &Uuid) -> Result<u32, CoverLetterError>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/009_cover_letter_versions.sql

CREATE TABLE cover_letter_versions (
    id                        TEXT NOT NULL PRIMARY KEY,
    job_id                    TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id            TEXT REFERENCES applications(id) ON DELETE SET NULL,
    version_number            INTEGER NOT NULL,
    content_md                TEXT NOT NULL,
    plain_text                TEXT NOT NULL,
    word_count                INTEGER NOT NULL,
    template_used             TEXT NOT NULL CHECK(template_used IN ('standard_professional','problem_solution','career_changer')),
    tone_used                 TEXT NOT NULL CHECK(tone_used IN ('professional','conversational','direct')),
    company_research_snapshot TEXT,         -- JSON of CompanyResearchSnapshot
    content_hash              TEXT NOT NULL,
    diff_from_prev            TEXT,         -- unified diff from previous version
    created_at                TEXT NOT NULL DEFAULT (datetime('now')),

    UNIQUE(job_id, version_number),
    UNIQUE(job_id, content_hash)            -- prevent saving identical re-generations
);

CREATE INDEX idx_cover_letter_versions_job_id ON cover_letter_versions(job_id);
CREATE INDEX idx_cover_letter_versions_application_id ON cover_letter_versions(application_id)
    WHERE application_id IS NOT NULL;
```

### Module Structure

```
lazyjob-core/
  src/
    cover_letter/
      mod.rs          # re-exports: CoverLetterService, all public types
      types.rs        # CoverLetterVersion, CoverLetterDraft, FabricationFlag, etc.
      service.rs      # CoverLetterService orchestrator
      selector.rs     # TemplateSelector, ToneSelector (auto-selection heuristics)
      prompt.rs       # PromptBuilder — assembles LLM system+user messages
      fabrication.rs  # FabricationChecker — extracts numeric claims, verifies against LifeSheet
      repository.rs   # CoverLetterVersionRepository trait
      sqlite.rs       # SqliteCoverLetterVersionRepository
      docx.rs         # CoverLetterDocxExporter
      diff.rs         # version_diff() using similar crate

lazyjob-tui/
  src/
    views/
      cover_letter_review.rs  # CoverLetterReviewView (editor + metadata)
      version_history.rs      # VersionHistoryPanel (version list + diff browser)
```

---

## Implementation Phases

### Phase 1 — Core Types, Repository, and SQLite (MVP)

#### Step 1.1 — Core types

Create `lazyjob-core/src/cover_letter/types.rs` with all structs and enums from the Core Types section above. Add `lazyjob-core/src/cover_letter/mod.rs` that re-exports public types.

**Verification:** `cargo build -p lazyjob-core` compiles with no errors.

#### Step 1.2 — SQLite migration

Create `lazyjob-core/migrations/009_cover_letter_versions.sql` with the DDL above. Apply via `sqlx::migrate!()` in the `Database::new()` init path (same as existing migrations).

**Verification:** `cargo sqlx migrate run` against a fresh SQLite file creates the table and indices.

#### Step 1.3 — Repository trait + SQLite implementation

Create `lazyjob-core/src/cover_letter/repository.rs` with the `CoverLetterVersionRepository` trait. Create `lazyjob-core/src/cover_letter/sqlite.rs`:

```rust
// lazyjob-core/src/cover_letter/sqlite.rs

use sqlx::{Pool, Sqlite};
use uuid::Uuid;

pub struct SqliteCoverLetterVersionRepository {
    pool: Pool<Sqlite>,
}

impl SqliteCoverLetterVersionRepository {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CoverLetterVersionRepository for SqliteCoverLetterVersionRepository {
    async fn save(&self, v: &CoverLetterVersion) -> Result<CoverLetterVersionId, CoverLetterError> {
        sqlx::query!(
            r#"
            INSERT INTO cover_letter_versions
                (id, job_id, application_id, version_number, content_md, plain_text,
                 word_count, template_used, tone_used, company_research_snapshot,
                 content_hash, diff_from_prev, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            v.id.0,
            v.job_id,
            v.application_id,
            v.version_number,
            v.content_md,
            v.plain_text,
            v.word_count,
            v.template_used.to_db_str(),
            v.tone_used.to_db_str(),
            v.company_research_snapshot,
            v.content_hash,
            v.diff_from_prev,
            v.created_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                CoverLetterError::DuplicateVersion {
                    job_id: v.job_id,
                    content_hash: v.content_hash.clone(),
                }
            }
            other => CoverLetterError::Database(other.into()),
        })?;
        Ok(v.id)
    }

    async fn next_version_number(&self, job_id: &Uuid) -> Result<u32, CoverLetterError> {
        let row = sqlx::query!(
            "SELECT COALESCE(MAX(version_number), 0) AS max_v FROM cover_letter_versions WHERE job_id = ?1",
            job_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CoverLetterError::Database(e.into()))?;
        Ok(row.max_v as u32 + 1)
    }

    // ... list_for_job, get, get_latest_for_job, link_to_application
}
```

**Verification:** `#[sqlx::test(migrations = "migrations")]` unit tests confirm round-trip `save` → `get` → `list_for_job`.

#### Step 1.4 — Error enum

Create `lazyjob-core/src/cover_letter/error.rs`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum CoverLetterError {
    #[error("Company not found for job {job_id}")]
    CompanyNotFound { job_id: Uuid },

    #[error("LifeSheet not found — run `lazyjob profile import` first")]
    LifeSheetNotFound,

    #[error("Invalid template string: {0}")]
    InvalidTemplate(String),

    #[error("Invalid tone string: {0}")]
    InvalidTone(String),

    #[error("Duplicate version (job={job_id}, hash={content_hash})")]
    DuplicateVersion { job_id: Uuid, content_hash: String },

    #[error("LLM generation failed: {0}")]
    LlmError(#[from] anyhow::Error),

    #[error("DOCX export failed: {0}")]
    DocxError(String),

    #[error("Database error: {0}")]
    Database(#[from] anyhow::Error),

    #[error("Forbidden fabrication flags — letter cannot be saved without review")]
    ForbiddenFabrication(Vec<FabricationFlag>),
}
```

---

### Phase 2 — Template Selection, Tone Selection, and Prompt Building

#### Step 2.1 — TemplateSelector

Create `lazyjob-core/src/cover_letter/selector.rs`:

```rust
// lazyjob-core/src/cover_letter/selector.rs

use crate::life_sheet::LifeSheet;
use crate::companies::CompanyRecord;
use super::types::{CoverLetterTemplate, CoverLetterTone};

pub struct TemplateSelector;

impl TemplateSelector {
    /// Auto-select a template based on LifeSheet career goals and company role domain.
    ///
    /// Logic:
    /// 1. If `life_sheet.goals.short_term` signals a pivot AND the target role's domain
    ///    differs from the majority work_experience domain → CareerChanger
    /// 2. If `company.culture_signals` contains startup/problem keywords → ProblemSolution
    /// 3. Default → StandardProfessional
    pub fn select(
        life_sheet: &LifeSheet,
        company: Option<&CompanyRecord>,
        target_role_title: &str,
    ) -> CoverLetterTemplate {
        if Self::is_career_pivot(life_sheet, target_role_title) {
            return CoverLetterTemplate::CareerChanger;
        }
        if let Some(c) = company {
            if Self::is_problem_solving_culture(&c.culture_signals) {
                return CoverLetterTemplate::ProblemSolution;
            }
        }
        CoverLetterTemplate::StandardProfessional
    }

    fn is_career_pivot(life_sheet: &LifeSheet, target_role: &str) -> bool {
        static PIVOT_KEYWORDS: &[&str] = &[
            "transition", "pivot", "switch", "change career", "new direction",
            "return to", "re-enter", "moving into",
        ];
        let goals_text = life_sheet.goals
            .as_ref()
            .and_then(|g| g.short_term.as_deref())
            .unwrap_or("");
        let has_pivot_signal = PIVOT_KEYWORDS.iter()
            .any(|kw| goals_text.to_lowercase().contains(kw));
        // Also check: if target role domain is very different from majority experience domain.
        // Domain detection: simple keyword approach using the role title.
        has_pivot_signal
    }

    fn is_problem_solving_culture(signals: &[String]) -> bool {
        static STARTUP_KEYWORDS: &[&str] = &[
            "startup", "fast-paced", "move fast", "scrappy", "ownership",
            "technical", "engineering-led", "product-led", "scale-up",
        ];
        signals.iter().any(|s| {
            STARTUP_KEYWORDS.iter().any(|kw| s.to_lowercase().contains(kw))
        })
    }
}

pub struct ToneSelector;

impl ToneSelector {
    /// Auto-select tone from CompanyRecord.culture_signals.
    ///
    /// Priority: Direct > Conversational > Professional (fallback)
    pub fn select(company: Option<&CompanyRecord>) -> CoverLetterTone {
        let signals = company
            .map(|c| c.culture_signals.as_slice())
            .unwrap_or(&[]);
        let combined = signals.iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        if Self::matches_direct(&combined) {
            CoverLetterTone::Direct
        } else if Self::matches_conversational(&combined) {
            CoverLetterTone::Conversational
        } else {
            CoverLetterTone::Professional
        }
    }

    fn matches_direct(combined: &str) -> bool {
        ["fast-paced", "startup", "move fast", "scrappy", "ownership", "direct"]
            .iter().any(|kw| combined.contains(kw))
    }

    fn matches_conversational(combined: &str) -> bool {
        ["collaborative", "inclusive", "people-first", "team", "empathetic", "warm"]
            .iter().any(|kw| combined.contains(kw))
    }
}
```

**Verification:** Unit tests confirm `CareerChanger` is selected for pivot goals, `ProblemSolution` for startup signals, `Professional` fallback.

#### Step 2.2 — PromptBuilder

Create `lazyjob-core/src/cover_letter/prompt.rs`:

```rust
// lazyjob-core/src/cover_letter/prompt.rs

use crate::life_sheet::{LifeSheet, WorkExperience};
use crate::companies::CompanyRecord;
use crate::resume::ProfileAnalysis;
use super::types::{CoverLetterTemplate, CoverLetterTone, CompanyResearchSnapshot};

pub struct CoverLetterPromptInput<'a> {
    pub life_sheet: &'a LifeSheet,
    pub job_description: &'a str,
    pub job_title: &'a str,
    pub company: Option<&'a CompanyRecord>,
    pub template: CoverLetterTemplate,
    pub tone: CoverLetterTone,
    /// Top 2 most relevant work experiences (from ProfileAnalysis).
    pub top_experiences: &'a [&'a WorkExperience],
    pub max_words: u16,
}

pub struct BuiltPrompt {
    pub system_message: String,
    pub user_message: String,
    /// Snapshot for storage (derived from company, None if no company).
    pub research_snapshot: Option<CompanyResearchSnapshot>,
}

pub struct PromptBuilder;

impl PromptBuilder {
    pub fn build(input: &CoverLetterPromptInput<'_>) -> BuiltPrompt {
        let research_snapshot = input.company.map(|c| CompanyResearchSnapshot {
            company_name: c.name.clone(),
            mission: c.mission.clone(),
            culture_signals: c.culture_signals.clone(),
            recent_news_hooks: c.recent_news
                .iter()
                .take(2)
                .map(|n| n.headline.clone())
                .collect(),
            glassdoor_rating: c.glassdoor_rating,
            funding_stage: c.funding_stage.clone(),
        });

        let system_message = Self::build_system(input, &research_snapshot);
        let user_message = Self::build_user(input);
        BuiltPrompt { system_message, user_message, research_snapshot }
    }

    fn build_system(input: &CoverLetterPromptInput<'_>, snapshot: &Option<CompanyResearchSnapshot>) -> String {
        let tone_instruction = match input.tone {
            CoverLetterTone::Professional => "Use formal, complete sentences. Avoid contractions. Tone: confident and professional.",
            CoverLetterTone::Conversational => "Use a warm, human tone. Contractions are fine. Be specific and genuine.",
            CoverLetterTone::Direct => "Use short, clear sentences. No filler phrases. Be concrete and direct.",
        };

        let template_structure = match input.template {
            CoverLetterTemplate::StandardProfessional => {
                "Structure: (1) Role hook + company research hook. (2) Strongest relevant achievement with metric. \
                 (3) Second achievement + culture alignment. (4) Forward-looking CTA. No 'I am writing to express' opening."
            }
            CoverLetterTemplate::ProblemSolution => {
                "Structure: (1) Name the key challenge this role addresses. (2) Show your direct experience solving that exact problem. \
                 (3) Why this company specifically. (4) Brief CTA. Lead with the problem, not yourself."
            }
            CoverLetterTemplate::CareerChanger => {
                "Structure: (1) Briefly acknowledge the non-linear path — own it as a strength. (2) Bridge transferable skills to the target role. \
                 (3) Specific evidence of fit (achievement + company alignment). (4) CTA that references forward momentum. \
                 Do not apologize for the career change."
            }
        };

        let company_section = snapshot.as_ref().map(|s| {
            let mission_line = s.mission.as_deref()
                .map(|m| format!("Mission: {m}"))
                .unwrap_or_default();
            let news_line = if !s.recent_news_hooks.is_empty() {
                format!("Recent news hooks (use one naturally): {}", s.recent_news_hooks.join("; "))
            } else {
                String::new()
            };
            format!(
                "Company context for {name}:\n{mission}\nCulture signals: {signals}\n{news}",
                name = s.company_name,
                mission = mission_line,
                signals = s.culture_signals.join(", "),
                news = news_line,
            )
        }).unwrap_or_else(|| "No company research available — focus on job description.".to_string());

        format!(
            "You are writing a cover letter for a job application. Follow these rules exactly:\n\
             - {tone}\n\
             - Target word count: {max_words} words (250–{max_words} range acceptable).\n\
             - {structure}\n\
             - No clichéd phrases: 'I am writing to', 'I am passionate about', 'fast-paced environment', 'team player'.\n\
             - All achievement claims MUST be verifiable — only use metrics from the provided career data.\n\
             - Output raw Markdown only. No preamble, no commentary.\n\n\
             {company}",
            tone = tone_instruction,
            max_words = input.max_words,
            structure = template_structure,
            company = company_section,
        )
    }

    fn build_user(input: &CoverLetterPromptInput<'_>) -> String {
        let experiences_text = input.top_experiences.iter().enumerate().map(|(i, exp)| {
            let achievements = exp.achievements.iter()
                .take(3)
                .map(|a| format!("  - {} ({})", a.description, a.metric_value.as_deref().unwrap_or("no metric")))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Experience {n}:\n  Company: {company}\n  Title: {title}\n  Period: {start}–{end}\n  Achievements:\n{ach}",
                n = i + 1,
                company = exp.company,
                title = exp.position,
                start = exp.start_date,
                end = exp.end_date.as_deref().unwrap_or("present"),
                ach = achievements,
            )
        }).collect::<Vec<_>>().join("\n\n");

        let goals_line = input.life_sheet.goals.as_ref()
            .and_then(|g| g.short_term.as_deref())
            .map(|g| format!("\nCareer goal: {g}"))
            .unwrap_or_default();

        format!(
            "Job title: {title}\n\nJob description:\n{jd}\n\nMy career data:\n{exp}{goals}\n\nWrite the cover letter now.",
            title = input.job_title,
            jd = input.job_description,
            exp = experiences_text,
            goals = goals_line,
        )
    }
}
```

**Verification:** Unit test confirms prompt contains company mission, achievement metrics, and template structure keywords.

---

### Phase 3 — Fabrication Checker

#### Step 3.1 — Numeric claim extraction

Create `lazyjob-core/src/cover_letter/fabrication.rs`:

```rust
// lazyjob-core/src/cover_letter/fabrication.rs
//
// Cover letter fabrication check: extract numeric claims from generated text,
// verify each against LifeSheet achievement metrics.
//
// Soft-skills claims ("strong communicator") are intentionally NOT checked.

use once_cell::sync::Lazy;
use regex::Regex;
use strsim::jaro_winkler;

use crate::life_sheet::LifeSheet;
use super::types::{FabricationFlag, FabricationLevel};

/// Matches quantified claims: "increased by 40%", "$2.5M", "10x", "3 months", "200+ users"
static NUMERIC_CLAIM_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(\d[\d,\.]*(?:\+|x|%|M|K|B)?(?:\s+\w+){0,4})\b").expect("valid regex")
});

pub struct FabricationChecker;

impl FabricationChecker {
    /// Check a generated cover letter for fabricated numeric claims.
    /// Returns a list of flags (empty = no issues).
    pub fn check(content_md: &str, life_sheet: &LifeSheet) -> Vec<FabricationFlag> {
        // Build a corpus of all achievement metric values from the LifeSheet.
        let known_metrics: Vec<String> = life_sheet.work_experience.iter()
            .flat_map(|exp| exp.achievements.iter())
            .filter_map(|a| a.metric_value.clone())
            .collect();

        // Also include numeric values mentioned anywhere in achievement descriptions.
        let known_text_metrics: Vec<String> = life_sheet.work_experience.iter()
            .flat_map(|exp| exp.achievements.iter())
            .map(|a| a.description.clone())
            .collect();

        let all_known: Vec<&str> = known_metrics.iter()
            .chain(known_text_metrics.iter())
            .map(|s| s.as_str())
            .collect();

        let mut flags = Vec::new();
        for mat in NUMERIC_CLAIM_REGEX.find_iter(content_md) {
            let claim = mat.as_str();
            if Self::is_in_life_sheet(claim, &all_known) {
                flags.push(FabricationFlag {
                    claim_text: claim.to_string(),
                    level: FabricationLevel::Safe,
                    explanation: "Found in LifeSheet".to_string(),
                });
            } else {
                flags.push(FabricationFlag {
                    claim_text: claim.to_string(),
                    level: FabricationLevel::Risky,
                    explanation: format!(
                        "Numeric claim '{claim}' not found in LifeSheet. Verify before sending."
                    ),
                });
            }
        }
        flags
    }

    fn is_in_life_sheet(claim: &str, known: &[&str]) -> bool {
        // Exact substring match.
        if known.iter().any(|k| k.contains(claim)) {
            return true;
        }
        // Jaro-Winkler similarity for abbreviated variants (e.g. "2.5M" vs "$2.5 million").
        known.iter().any(|k| jaro_winkler(claim, k) >= 0.88)
    }

    /// Determine if the draft is approvable: no Forbidden flags.
    pub fn is_approvable(flags: &[FabricationFlag]) -> bool {
        !flags.iter().any(|f| f.level == FabricationLevel::Forbidden)
    }
}
```

**Verification:** Unit tests: (1) a claim extracted from the spec example ("40% revenue growth") found in LifeSheet returns `Safe`; (2) a made-up "$50M ARR" not in LifeSheet returns `Risky`.

---

### Phase 4 — CoverLetterService Orchestrator

Create `lazyjob-core/src/cover_letter/service.rs`:

```rust
// lazyjob-core/src/cover_letter/service.rs

use std::sync::Arc;
use uuid::Uuid;

use crate::companies::{CompanyRepository, CompanyRecord};
use crate::life_sheet::LifeSheetRepository;
use crate::jobs::JobRepository;
use crate::resume::ProfileAnalyzer;
use lazyjob_llm::LlmProvider;

use super::{
    types::*,
    repository::CoverLetterVersionRepository,
    selector::{TemplateSelector, ToneSelector},
    prompt::{PromptBuilder, CoverLetterPromptInput},
    fabrication::FabricationChecker,
    diff::version_diff,
    docx::CoverLetterDocxExporter,
};

pub struct CoverLetterService {
    pub llm: Arc<dyn LlmProvider>,
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
    pub company_repo: Arc<dyn CompanyRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub version_repo: Arc<dyn CoverLetterVersionRepository>,
}

impl CoverLetterService {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        life_sheet_repo: Arc<dyn LifeSheetRepository>,
        company_repo: Arc<dyn CompanyRepository>,
        job_repo: Arc<dyn JobRepository>,
        version_repo: Arc<dyn CoverLetterVersionRepository>,
    ) -> Self {
        Self { llm, life_sheet_repo, company_repo, job_repo, version_repo }
    }

    /// Generate a cover letter draft for the given job. Does not persist — caller must
    /// call `save_version` after user review and approval.
    #[tracing::instrument(skip(self, options), fields(job_id = %job_id))]
    pub async fn generate(
        &self,
        job_id: &Uuid,
        options: CoverLetterOptions,
    ) -> Result<CoverLetterDraft, CoverLetterError> {
        // 1. Load job.
        let job = self.job_repo.get(job_id)
            .await
            .map_err(|_| CoverLetterError::LifeSheetNotFound)?;

        // 2. Load LifeSheet.
        let life_sheet = self.life_sheet_repo.load_current()
            .await
            .map_err(|_| CoverLetterError::LifeSheetNotFound)?;

        // 3. Load CompanyRecord (if available and requested).
        let company: Option<CompanyRecord> = if options.include_company_research {
            match self.company_repo.find_by_name(&job.company_name).await {
                Ok(Some(c)) => Some(c),
                Ok(None) => {
                    tracing::warn!(
                        company = %job.company_name,
                        "No company research found — generating from JD only"
                    );
                    None
                }
                Err(e) => {
                    tracing::error!(err = %e, "CompanyRepository lookup failed");
                    None
                }
            }
        } else {
            None
        };

        // 4. Select top 2 relevant experiences.
        let analyzer = ProfileAnalyzer::new(&life_sheet);
        let analysis = analyzer.analyze(&job.description);
        let top_experiences: Vec<&_> = analysis.ranked_experiences
            .iter()
            .take(2)
            .map(|r| &r.experience)
            .collect();

        // 5. Select template and tone.
        let template = options.template.unwrap_or_else(|| {
            TemplateSelector::select(&life_sheet, company.as_ref(), &job.title)
        });
        let tone = options.tone.unwrap_or_else(|| {
            ToneSelector::select(company.as_ref())
        });
        let max_words = options.max_words.unwrap_or(template.target_word_count());

        // 6. Build prompt.
        let prompt_input = CoverLetterPromptInput {
            life_sheet: &life_sheet,
            job_description: &job.description,
            job_title: &job.title,
            company: company.as_ref(),
            template,
            tone,
            top_experiences: &top_experiences,
            max_words,
        };
        let built = PromptBuilder::build(&prompt_input);

        // 7. Generate via LLM (non-streaming path; streaming variant in Phase 5).
        let response = self.llm.chat(vec![
            lazyjob_llm::ChatMessage::system(built.system_message),
            lazyjob_llm::ChatMessage::user(built.user_message),
        ])
        .await
        .map_err(|e| CoverLetterError::LlmError(e.into()))?;

        let content_md = response.content;
        let word_count = content_md.split_whitespace().count();

        // 8. Anti-fabrication check.
        let fabrication_flags = if options.skip_fabrication_check {
            vec![]
        } else {
            FabricationChecker::check(&content_md, &life_sheet)
        };
        let is_approvable = FabricationChecker::is_approvable(&fabrication_flags);

        Ok(CoverLetterDraft {
            content_md,
            word_count,
            template_used: template,
            tone_used: tone,
            company_research: built.research_snapshot,
            fabrication_flags,
            is_approvable,
        })
    }

    /// Persist an approved draft as a new version. Computes diff from previous version.
    pub async fn save_version(
        &self,
        draft: &CoverLetterDraft,
        job_id: &Uuid,
        application_id: Option<&Uuid>,
    ) -> Result<CoverLetterVersion, CoverLetterError> {
        if !draft.is_approvable {
            return Err(CoverLetterError::ForbiddenFabrication(
                draft.fabrication_flags.clone()
            ));
        }

        // Compute content hash for dedup.
        use sha2::{Sha256, Digest};
        let hash = hex::encode(Sha256::digest(draft.content_md.as_bytes()));

        // Get previous version for diff.
        let prev = self.version_repo.get_latest_for_job(job_id).await?;
        let diff_from_prev = prev.as_ref().map(|p| version_diff(&p.content_md, &draft.content_md));

        // Strip Markdown to plain text.
        let plain_text = strip_markdown(&draft.content_md);

        let version_number = self.version_repo.next_version_number(job_id).await?;

        let version = CoverLetterVersion {
            id: CoverLetterVersionId::new(),
            job_id: *job_id,
            application_id: application_id.copied(),
            version_number,
            content_md: draft.content_md.clone(),
            plain_text,
            word_count: draft.word_count as u32,
            template_used: draft.template_used,
            tone_used: draft.tone_used,
            company_research_snapshot: draft.company_research.as_ref()
                .and_then(|s| serde_json::to_string(s).ok()),
            content_hash: hash,
            diff_from_prev,
            created_at: chrono::Utc::now(),
        };

        self.version_repo.save(&version).await?;
        Ok(version)
    }

    /// Export a persisted version to a DOCX file.
    pub async fn export_docx(
        &self,
        version_id: &CoverLetterVersionId,
        path: &std::path::Path,
    ) -> Result<(), CoverLetterError> {
        let version = self.version_repo.get(version_id).await?;
        CoverLetterDocxExporter::export(&version, path)
    }
}

/// Strip basic Markdown syntax (bold, italic, headers, links) to plain text.
fn strip_markdown(md: &str) -> String {
    md.lines()
        .map(|line| {
            let line = line.trim_start_matches('#').trim();
            // Remove **bold** and *italic* markers
            let line = line.replace("**", "").replace('*', "");
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

**Verification:** Integration test generates a draft for a mocked job, verifies `word_count > 0`, `template_used` matches expected, and saving returns `version_number = 1`.

---

### Phase 5 — Streaming Generation + DOCX Export

#### Step 5.1 — Streaming path

Add a `generate_streaming` method to `CoverLetterService` that returns a `tokio::sync::mpsc::Receiver<StreamToken>`:

```rust
pub struct StreamToken {
    /// Partial token text from the LLM.
    pub text: String,
    /// True on the final token — triggers fabrication check in the TUI.
    pub is_final: bool,
}

impl CoverLetterService {
    pub async fn generate_streaming(
        &self,
        job_id: &Uuid,
        options: CoverLetterOptions,
        tx: tokio::sync::mpsc::Sender<StreamToken>,
    ) -> Result<(), CoverLetterError> {
        // Build prompt same as generate(), then call llm.stream_chat().
        // Forward each StreamEvent token to tx.
        // On completion, assemble full content_md, run fabrication check,
        // send final StreamToken { text: full_content_md, is_final: true }.
        todo!("Phase 5")
    }
}
```

The TUI cover letter review view subscribes to this channel via a `tokio::sync::mpsc::Receiver` stored in `App` state, updating the preview buffer on each received token.

#### Step 5.2 — DOCX export

Create `lazyjob-core/src/cover_letter/docx.rs`:

```rust
// lazyjob-core/src/cover_letter/docx.rs

use docx_rs::*;
use std::path::Path;
use super::types::{CoverLetterVersion, CoverLetterError};

pub struct CoverLetterDocxExporter;

impl CoverLetterDocxExporter {
    /// Export a cover letter version to a single-page DOCX file.
    /// Format: Calibri 11pt, 1-inch margins, left-aligned, standard business letter.
    pub fn export(version: &CoverLetterVersion, path: &Path) -> Result<(), CoverLetterError> {
        let mut doc = Docx::new()
            .page_margin(PageMargin {
                top: 1440,    // 1 inch in twips
                right: 1440,
                bottom: 1440,
                left: 1440,
                header: 720,
                footer: 720,
                gutter: 0,
            });

        // Split plain_text into paragraphs and add each.
        for para_text in version.plain_text.split("\n\n") {
            if para_text.trim().is_empty() { continue; }
            let para = Paragraph::new()
                .add_run(Run::new().add_text(para_text.trim()))
                .style("Normal");
            doc = doc.add_paragraph(para);
        }

        let buf = doc.build().pack()
            .map_err(|e| CoverLetterError::DocxError(e.to_string()))?;
        std::fs::write(path, &buf)
            .map_err(|e| CoverLetterError::DocxError(e.to_string()))
    }
}
```

**Verification:** Export test produces a non-empty `.docx` file that opens without error in LibreOffice.

#### Step 5.3 — Version diff

Create `lazyjob-core/src/cover_letter/diff.rs`:

```rust
// lazyjob-core/src/cover_letter/diff.rs

use similar::{ChangeTag, TextDiff};

/// Compute a unified diff between two cover letter Markdown texts.
pub fn version_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(3)
        .header("prev", "new")
        .to_string()
}
```

**Verification:** Unit test confirms diff of two differing texts produces non-empty output; diff of identical texts produces empty output.

---

### Phase 6 — TUI Cover Letter Review View

#### Step 6.1 — CoverLetterReviewView

Create `lazyjob-tui/src/views/cover_letter_review.rs`:

```rust
// lazyjob-tui/src/views/cover_letter_review.rs
//
// Two-pane view:
//   Left (60%): scrollable editable Markdown draft
//   Right (40%): metadata panel (template, tone, word count, fabrication flags)
//
// Keybindings:
//   <Enter>      → approve and save version
//   r            → regenerate (with same options)
//   t            → cycle template
//   o            → cycle tone
//   e            → open $EDITOR for manual edits (spawns process, re-reads on exit)
//   d            → export to DOCX
//   q / <Esc>    → cancel / discard draft

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub struct CoverLetterReviewView {
    pub draft: Option<lazyjob_core::cover_letter::CoverLetterDraft>,
    pub scroll_offset: u16,
    pub is_loading: bool,
    pub streaming_buffer: String,   // accumulates tokens during streaming
    pub status_message: Option<String>,
}

impl CoverLetterReviewView {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        self.render_draft_pane(frame, chunks[0]);
        self.render_metadata_pane(frame, chunks[1]);
    }

    fn render_draft_pane(&self, frame: &mut Frame, area: Rect) {
        let content = if self.is_loading {
            format!("{}\u{258C}", self.streaming_buffer)  // cursor block
        } else {
            self.draft.as_ref()
                .map(|d| d.content_md.clone())
                .unwrap_or_else(|| "No draft generated.".to_string())
        };

        let para = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title("Cover Letter Draft"))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));
        frame.render_widget(para, area);
    }

    fn render_metadata_pane(&self, frame: &mut Frame, area: Rect) {
        let Some(draft) = &self.draft else {
            let placeholder = Paragraph::new("Generating...")
                .block(Block::default().borders(Borders::ALL).title("Details"));
            frame.render_widget(placeholder, area);
            return;
        };

        let fab_lines: Vec<Line> = draft.fabrication_flags.iter().map(|f| {
            let color = match f.level {
                lazyjob_core::cover_letter::FabricationLevel::Safe => Color::Green,
                lazyjob_core::cover_letter::FabricationLevel::Risky => Color::Yellow,
                lazyjob_core::cover_letter::FabricationLevel::Forbidden => Color::Red,
            };
            Line::from(vec![
                Span::styled(format!("{:?} ", f.level), Style::default().fg(color)),
                Span::raw(f.claim_text.clone()),
            ])
        }).collect();

        let items: Vec<ListItem> = vec![
            ListItem::new(format!("Template : {:?}", draft.template_used)),
            ListItem::new(format!("Tone     : {:?}", draft.tone_used)),
            ListItem::new(format!("Words    : {}", draft.word_count)),
            ListItem::new(if draft.is_approvable { "Status   : ✓ Approvable" } else { "Status   : ✗ Review flags" }),
            ListItem::new(""),
            ListItem::new("Fabrication Audit:"),
        ];
        let fab_items: Vec<ListItem> = fab_lines.into_iter()
            .map(ListItem::new)
            .collect();

        let all_items: Vec<ListItem> = items.into_iter().chain(fab_items).collect();
        let list = List::new(all_items)
            .block(Block::default().borders(Borders::ALL).title("Details"));
        frame.render_widget(list, area);
    }
}
```

#### Step 6.2 — VersionHistoryPanel

Create `lazyjob-tui/src/views/version_history.rs`:

```rust
// lazyjob-tui/src/views/version_history.rs
//
// Shows a list of CoverLetterVersion entries for the selected job.
// Keybindings:
//   j/k          → navigate versions
//   <Enter>      → open selected version in review view
//   d            → show diff against previous version in a floating panel
//   x            → export selected version to DOCX

use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use lazyjob_core::cover_letter::CoverLetterVersion;

pub struct VersionHistoryPanel {
    pub versions: Vec<CoverLetterVersion>,
    pub list_state: ListState,
}

impl VersionHistoryPanel {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.versions.iter().map(|v| {
            let label = format!(
                "v{} — {:?} / {:?} — {} words — {}",
                v.version_number,
                v.template_used,
                v.tone_used,
                v.word_count,
                v.created_at.format("%Y-%m-%d %H:%M"),
            );
            ListItem::new(label)
        }).collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Version History"))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}
```

**Verification:** TUI integration test renders the review view with a mock draft, confirms word count and template name appear in the metadata pane.

---

## Key Crate APIs

| API | Purpose |
|-----|---------|
| `sqlx::query!("INSERT INTO cover_letter_versions ...", ...)` | Type-checked DDL/DML at compile time |
| `sqlx::query!("SELECT ... FROM cover_letter_versions ...", job_id)` | Type-safe SELECT with macro |
| `similar::TextDiff::from_lines(old, new).unified_diff()` | Version diff generation |
| `sha2::Sha256::digest(bytes)` | Content hash for dedup |
| `hex::encode(hash)` | Human-readable hex content hash |
| `regex::Regex::new(r"\b(\d[\d,\.]*...)")` | Numeric claim extraction from generated text |
| `once_cell::sync::Lazy<Regex>` | Compile-once regex patterns |
| `strsim::jaro_winkler(a, b) >= 0.88` | Fuzzy metric matching for fabrication check |
| `docx_rs::Docx::new().page_margin(...).add_paragraph(...)` | DOCX generation |
| `docx_rs::Docx::build().pack()` | Serialize DOCX to bytes |
| `ratatui::widgets::Paragraph::new(text).scroll((offset, 0))` | Scrollable draft pane |
| `ratatui::widgets::List::new(items).highlight_symbol(">")` | Version history browser |
| `ratatui::layout::Layout::default().constraints([60%, 40%]).split(area)` | Two-pane layout |
| `tokio::sync::mpsc::channel::<StreamToken>()` | Streaming generation channel |
| `tracing::instrument(skip(self, options))` | Structured logging per generation call |

---

## Error Handling

```rust
// lazyjob-core/src/cover_letter/error.rs

#[derive(thiserror::Error, Debug)]
pub enum CoverLetterError {
    #[error("Company not found for job {job_id} — run company research first")]
    CompanyNotFound { job_id: Uuid },

    #[error("LifeSheet not loaded — run `lazyjob profile import` first")]
    LifeSheetNotFound,

    #[error("Job not found: {0}")]
    JobNotFound(Uuid),

    #[error("Invalid template string: {0}")]
    InvalidTemplate(String),

    #[error("Invalid tone string: {0}")]
    InvalidTone(String),

    #[error("Duplicate version — identical content already saved (job={job_id})")]
    DuplicateVersion { job_id: Uuid, content_hash: String },

    #[error("LLM generation failed: {0}")]
    LlmError(#[source] anyhow::Error),

    #[error("DOCX export failed: {0}")]
    DocxError(String),

    #[error("Database error: {0}")]
    Database(#[source] anyhow::Error),

    /// Non-terminal: TUI must present confirmation dialog rather than blocking.
    #[error("Fabrication flags require review before saving")]
    ForbiddenFabrication(Vec<FabricationFlag>),

    #[error("Version not found: {0}")]
    VersionNotFound(CoverLetterVersionId),
}

pub type Result<T> = std::result::Result<T, CoverLetterError>;
```

---

## Testing Strategy

### Unit Tests

**`selector.rs`**
- `TemplateSelector::select` returns `CareerChanger` when `life_sheet.goals.short_term = "transition to product management"` and company signals are neutral.
- Returns `ProblemSolution` for a company with `culture_signals = ["fast-paced", "startup"]`.
- Returns `StandardProfessional` with empty signals and non-pivot goal.

**`fabrication.rs`**
- `FabricationChecker::check`: a claim of "40%" that exists in LifeSheet `metric_value` returns `Safe`.
- A claim of "$50M ARR" not in any LifeSheet achievement returns `Risky`.
- Empty draft returns empty flags vec.

**`prompt.rs`**
- `PromptBuilder::build` produces a system message containing `target word count: 275` for `ProblemSolution`.
- Company mission appears in system message when `CompanyRecord` is provided.
- Goals `short_term` appears in user message when populated.

**`diff.rs`**
- Identical inputs produce empty diff string.
- Changed paragraph produces non-empty unified diff with `+`/`-` markers.

**`sqlite.rs` (`#[sqlx::test(migrations = "migrations")]`)**
- `save` → `get` round-trip returns identical `content_md`.
- `save` of same `content_hash` for same job returns `DuplicateVersion`.
- `next_version_number` returns 1 for a new job, 2 after first save.
- `list_for_job` returns versions ordered by `version_number ASC`.

### Integration Tests

**End-to-end generation** (`tests/cover_letter_e2e.rs`):
1. Insert a `Job` and a `LifeSheet` into an in-memory SQLite DB with migrations applied.
2. Insert a `CompanyRecord` with known culture signals.
3. Construct `CoverLetterService` with a `MockLlmProvider` that returns a fixed draft.
4. Call `generate()` — verify `word_count > 0`, `template_used` matches expected selection.
5. Call `save_version()` — verify `version_number = 1`, row exists in DB.
6. Call `generate()` again with `MockLlmProvider` returning same text — verify `save_version` returns `DuplicateVersion`.
7. Call `generate()` with `MockLlmProvider` returning text with a fabricated "$99M" claim — verify `fabrication_flags.len() > 0` and `is_approvable = false`.

**DOCX export** (`tests/cover_letter_docx_test.rs`):
1. Generate a version, call `export_docx` to a temp file.
2. Verify the file exists and its size > 0.
3. Read the raw bytes and verify the DOCX magic header (`PK\x03\x04`).

### TUI Tests

- Render `CoverLetterReviewView` with a mock draft using `ratatui::backend::TestBackend`.
- Assert buffer contains "Cover Letter Draft", word count, and template name.
- Assert fabrication flag appears in the metadata pane with correct color indicator.

---

## Open Questions

1. **Streaming TUI rendering**: The `generate_streaming` method (Phase 5) requires the TUI to handle incremental Markdown rendering. The `App` event loop must pump `StreamToken` messages on the same `tokio::select!` tick as keyboard events. Architecture question: should the streaming channel be a `tokio::sync::mpsc::UnboundedSender` (no backpressure, simpler) or bounded (prevents LLM racing ahead)? Recommendation: bounded with size 32 — prevents buffering the full draft before TUI can render.

2. **`$EDITOR` integration**: When the user presses `e` in the review view to manually edit the draft, the TUI must suspend (raw mode off), spawn `$EDITOR`, wait for it to exit, re-read the file, then resume TUI (raw mode on). This is the same pattern used in `lazygit`. Implement in Phase 6 as a `editor_edit` helper in `lazyjob-tui/src/editor.rs` using `std::process::Command::new(editor).status()`.

3. **Multiple quick drafts**: The spec explicitly defers the "generate 2–3 variants" feature. When it is eventually needed, the cleanest approach is to pass a `variant_count: u8` in `CoverLetterOptions` and run `join_all` over N independent LLM calls — not a single prompt asking for N variants (which produces worse output).

4. **CareerChanger template and TransferableSkillMap**: The `CareerChanger` template requires `TransferableSkillMap` from the profile gap analysis spec (`specs/profile-skills-gap-analysis.md`). Until that spec is implemented, the `CareerChanger` template falls back to injecting `life_sheet.goals.short_term` text only, without the structured skill bridge. This fallback should be documented in the TUI metadata pane.

5. **Cover letter necessity detection**: The spec defers the "is a cover letter even needed?" signal to user decision. However, `JdParser` (from the resume tailoring plan) already extracts structured fields from the JD. Adding a `CoverLetterRequired::Optional | Required | NotMentioned` enum to `JdAnalysis` (in the JD parser) would surface this signal without additional LLM calls — just regex on phrases like "cover letter optional", "no cover letter necessary".

---

## Related Specs

- [specs/profile-cover-letter-generation.md](./profile-cover-letter-generation.md) — source spec
- [specs/08-cover-letter-generation.md](./08-cover-letter-generation.md) — architectural cover letter spec (company research integration, general pipeline)
- [specs/08-cover-letter-generation-implementation-plan.md](./08-cover-letter-generation-implementation-plan.md) — prior implementation plan (broader architectural view)
- [specs/profile-life-sheet-data-model-implementation-plan.md](./profile-life-sheet-data-model-implementation-plan.md) — provides LifeSheet, is_grounded_claim()
- [specs/job-search-company-research-implementation-plan.md](./job-search-company-research-implementation-plan.md) — provides CompanyRecord, CompanyRepository
- [specs/profile-resume-tailoring-implementation-plan.md](./profile-resume-tailoring-implementation-plan.md) — provides ProfileAnalyzer, TransferableSkillMap
- [specs/agentic-prompt-templates-implementation-plan.md](./agentic-prompt-templates-implementation-plan.md) — provides PROHIBITED_PHRASES guard, FabricationLevel types
- [specs/XX-cover-letter-version-management.md](./XX-cover-letter-version-management.md) — dedicated version management spec (Phase 3 extension)
