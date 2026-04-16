# Implementation Plan: Profile Resume Tailoring Pipeline

## Status
Draft

## Related Spec
[specs/profile-resume-tailoring.md](./profile-resume-tailoring.md)

## Overview

The profile resume tailoring pipeline is the primary value-add feature of LazyJob. It transforms two inputs — a user's LifeSheet (structured SQLite career data derived from YAML) and a raw job description — into a submission-ready DOCX resume that is ATS-safe, keyword-targeted, and grounded in real career history. The pipeline runs in 6 sequential stages: JD parsing → LifeSheet analysis → gap analysis → content drafting → DOCX generation → fabrication audit.

This spec is distinct from `specs/07-resume-tailoring-pipeline.md` (the architectural/pipeline design spec) in that it approaches the problem from the **profile domain perspective**: how the LifeSheet's voice, skill taxonomy, ESCO codes, and version history are leveraged to produce tailored output. Key profile-side concerns include: preserving the user's writing style via few-shot exemplars from their own bullet history, keyword density targeting across three JD tiers, cross-version diff rendering in the TUI, and a fabrication audit using `is_grounded_claim()` from the LifeSheet module as the source-of-truth oracle.

The plan targets `lazyjob-core/src/resume/` as the primary crate for all six pipeline stages. SQLite migration adds `resume_versions` and `jd_analyses` tables. The TUI gets a version browser panel and a side-by-side diff widget. Ralph loop integration (background tailoring trigger) is described but deferred to Phase 3.

## Prerequisites

### Specs/Plans that must precede this
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `LifeSheetRepository`, `is_grounded_claim()`, ESCO codes per skill entry
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, migration infrastructure, `sqlx::Pool<Sqlite>`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>`, `ChatMessage`, `StreamEvent`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel focus system

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
docx-rs            = "0.4"
strsim             = "0.11"          # Jaro-Winkler for skill fuzzy matching
regex              = "1"
once_cell          = "1"             # Lazy<Regex> patterns
sha2               = "0.10"          # Content hash for change detection
similar            = "2"             # Unified diff between resume versions
unicode-normalization = "0.1"
ammonia            = "3"             # HTML sanitization for JD input

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
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All 6 pipeline stages as independent modules; `ResumeVersionRepository`; SQLite migrations |
| `lazyjob-llm` | LLM provider trait (already exists); prompt templates for JD parsing and bullet drafting |
| `lazyjob-tui` | `ResumeDiffWidget`, `VersionBrowserPanel`, `FabricationReportPanel` |
| `lazyjob-ralph` | Background loop trigger for auto-tailoring on `ApplyWorkflow::execute` |
| `lazyjob-cli` | `lazyjob resume tailor <job-id>` subcommand |

`lazyjob-core` must have zero dependency on `lazyjob-tui` or `lazyjob-ralph`. Domain types flow outward; UI and subprocess layers import from core.

### Core Types

```rust
// lazyjob-core/src/resume/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Newtype wrapper: parse, don't validate pattern (see rust-patterns.md §2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct ResumeVersionId(pub Uuid);

impl ResumeVersionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// The canonical persisted resume snapshot tied to a specific application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersion {
    pub id: ResumeVersionId,
    /// FK → jobs.id
    pub job_id: Uuid,
    /// FK → applications.id (set when user submits the application)
    pub application_id: Option<Uuid>,
    /// The structured content tree — source of truth for diff/TUI rendering.
    pub content: ResumeContent,
    /// Raw DOCX bytes stored as BLOB in SQLite (≈50KB per version).
    pub docx_bytes: Vec<u8>,
    /// SHA-256 of docx_bytes — used to detect duplicate re-generations.
    pub content_hash: String,
    pub gap_report: GapReport,
    pub fabrication_report: FabricationReport,
    pub tailoring_options: TailoringOptions,
    /// Human label: "v1", "v2 — added Kubernetes", etc.
    pub label: String,
    /// True once the user explicitly approved and linked to an application.
    pub is_submitted: bool,
    pub created_at: DateTime<Utc>,
}

/// Section-structured resume content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeContent {
    pub header: ResumeHeader,
    pub summary: String,
    pub experience: Vec<ExperienceSection>,
    pub skills: Vec<SkillCategory>,
    pub education: Vec<EducationEntry>,
    pub projects: Vec<ProjectEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeHeader {
    pub full_name: String,
    pub email: String,
    pub phone: Option<String>,
    pub location: Option<String>,
    pub linkedin_url: Option<String>,
    pub github_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExperienceSection {
    pub company: String,
    pub title: String,
    pub start_date: String,     // "YYYY-MM" or "YYYY"
    pub end_date: Option<String>,
    pub is_current: bool,
    pub bullets: Vec<String>,   // Tailored bullets (may differ from LifeSheet)
    pub original_bullets: Vec<String>, // Preserved from LifeSheet for diff
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillCategory {
    pub category: String,
    pub skills: Vec<String>,
    pub display_order: u8,      // 0 = first (JD-relevant skills float to top)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EducationEntry {
    pub institution: String,
    pub degree: String,
    pub field: String,
    pub graduation_year: Option<u16>,
    pub included: bool,         // false = omitted in this version
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectEntry {
    pub name: String,
    pub description: String,
    pub tech_stack: Vec<String>,
    pub url: Option<String>,
    pub included: bool,
}

/// Extracted structured requirements from a job description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescriptionAnalysis {
    pub job_id: Uuid,
    /// Skills explicitly marked "required" in the JD.
    pub required_skills: Vec<String>,
    /// "Nice to have", "preferred", "bonus" skills.
    pub nice_to_have_skills: Vec<String>,
    /// All distinct keywords worth placing in summary/bullets.
    pub keywords: Vec<JdKeyword>,
    /// Cultural signal phrases: "fast-paced", "startup", etc.
    pub culture_signals: Vec<String>,
    /// Minimum years of experience if stated.
    pub experience_years_required: Option<u8>,
    pub seniority_level: Option<SeniorityLevel>,
    /// Raw analysis produced by LLM or TF-IDF fallback.
    pub extraction_method: ExtractionMethod,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JdKeyword {
    pub text: String,
    pub tier: KeywordTier,
    /// Number of times keyword appears in raw JD text.
    pub raw_frequency: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeywordTier {
    /// Required skills → target 2–3 mentions across summary + experience.
    Tier1Required,
    /// Nice-to-have → 1 mention in skills section.
    Tier2NiceToHave,
    /// Culture/soft signals → 1 mention in summary.
    Tier3Culture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SeniorityLevel { Junior, MidLevel, Senior, Staff, Principal }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExtractionMethod {
    /// LLM structured extraction succeeded.
    Llm { model: String },
    /// LLM call failed or timed out; used TF-IDF keyword extraction.
    TfIdfFallback,
}

/// Per-skill gap classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    /// Skills present in both LifeSheet and JD requirements.
    pub matched_skills: Vec<MatchedSkill>,
    /// Skills in JD but absent or only partially present in LifeSheet.
    pub missing_skills: Vec<MissingSkill>,
    /// Aggregate similarity score 0.0–1.0.
    pub match_score: f32,
    /// Coverage of Tier1 required skills specifically.
    pub tier1_coverage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedSkill {
    pub jd_name: String,
    pub life_sheet_name: String,     // May differ (alias)
    pub match_type: SkillMatchType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillMatchType {
    /// Exact string match (after normalization).
    Exact,
    /// Synonym/alias known to the system (e.g., "React.js" == "React").
    Alias,
    /// ESCO taxonomy proximity — both map to same occupation/skill ESCO code.
    EscoProximity,
    /// Jaro-Winkler similarity >= 0.88 (e.g., "Elasticsearch" matches "ElasticSearch").
    FuzzyString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingSkill {
    pub name: String,
    pub is_required: bool,
    pub fabrication_level: FabricationLevel,
    /// Human-readable explanation of why this level was assigned.
    pub reason: String,
}

/// Four-tier fabrication classification — determines what the drafter is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FabricationLevel {
    /// Skill in LifeSheet — rephrasing/emphasis only.
    Safe,
    /// Adjacent skill present (e.g., "Kafka" when LifeSheet has "RabbitMQ" + distributed systems).
    /// Drafter may include "familiar with X" or "exposure to X".
    Acceptable,
    /// No evidence in LifeSheet — flag to user, EXCLUDE from resume.
    Risky,
    /// Credentials, degrees, certifications not in LifeSheet — BLOCK generation entirely.
    Forbidden,
}

/// Audit result shown to user in TUI before approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationReport {
    pub flags: Vec<FabricationFlag>,
    /// False if any Forbidden flags exist. User cannot approve until resolved.
    pub is_submittable: bool,
    /// Yellow warnings for Risky items requiring explicit acknowledgment.
    pub warnings: Vec<String>,
    /// Keyword stuffing flags: keyword appeared > 4 times total.
    pub keyword_stuffing_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricationFlag {
    pub skill_name: String,
    pub level: FabricationLevel,
    pub location: FlagLocation,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlagLocation {
    Summary,
    ExperienceBullet { company: String, bullet_index: usize },
    SkillsSection { category: String },
}

/// Options controlling tailoring behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailoringOptions {
    /// 1 or 2 (ATS guidelines generally prefer 1-page for <10 YOE)
    pub max_pages: u8,
    pub include_projects: bool,
    pub format: ResumeFormat,
    /// If false, skip Ralph background loop; run inline.
    pub run_as_background_loop: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResumeFormat {
    /// Single-column, no tables/boxes, plain fonts — most ATS-compatible.
    AtsSimple,
    /// Two-column header (contact info + summary side by side) — Phase 2.
    AtsClean,
}

/// The complete output of `ResumeTailor::tailor`.
#[derive(Debug)]
pub struct TailoredResume {
    pub version_id: ResumeVersionId,
    pub content: ResumeContent,
    pub docx_bytes: Vec<u8>,
    pub gap_report: GapReport,
    pub fabrication_report: FabricationReport,
    /// The JD analysis produced in Stage 1 (cached in SQLite separately).
    pub jd_analysis: JobDescriptionAnalysis,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/resume/repository.rs

use async_trait::async_trait;
use uuid::Uuid;
use super::types::*;
use crate::Result;

#[async_trait]
pub trait ResumeVersionRepository: Send + Sync {
    /// Persist a new resume version. Returns the assigned ID.
    async fn save(&self, version: &ResumeVersion) -> Result<ResumeVersionId>;

    async fn get(&self, id: &ResumeVersionId) -> Result<ResumeVersion>;

    /// All versions for a given job, newest first.
    async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<ResumeVersion>>;

    /// The single version currently linked to an application.
    async fn get_submitted(&self, application_id: &Uuid) -> Result<Option<ResumeVersion>>;

    /// Mark a version as submitted, optionally linking an application FK.
    async fn mark_submitted(
        &self,
        id: &ResumeVersionId,
        application_id: &Uuid,
    ) -> Result<()>;

    /// Update the human-readable label.
    async fn update_label(&self, id: &ResumeVersionId, label: &str) -> Result<()>;

    /// Delete a non-submitted draft version.
    async fn delete_draft(&self, id: &ResumeVersionId) -> Result<()>;
}

#[async_trait]
pub trait JdAnalysisRepository: Send + Sync {
    /// Cache parsed JD analysis keyed by job_id (idempotent).
    async fn upsert(&self, analysis: &JobDescriptionAnalysis) -> Result<()>;
    async fn get_for_job(&self, job_id: &Uuid) -> Result<Option<JobDescriptionAnalysis>>;
}
```

```rust
// lazyjob-core/src/resume/pipeline.rs  — Stage trait

use async_trait::async_trait;
use crate::Result;

/// A single pipeline stage. Stages compose to form the full pipeline.
#[async_trait]
pub trait PipelineStage<Input, Output>: Send + Sync {
    async fn run(&self, input: Input) -> Result<Output>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/008_resume_versions.sql

CREATE TABLE IF NOT EXISTS jd_analyses (
    job_id          TEXT NOT NULL PRIMARY KEY,  -- FK → jobs.id (UUID)
    required_skills TEXT NOT NULL,              -- JSON array
    nice_to_have    TEXT NOT NULL,              -- JSON array
    keywords_json   TEXT NOT NULL,              -- JSON array of JdKeyword
    culture_signals TEXT NOT NULL,              -- JSON array
    years_required  INTEGER,
    seniority_level TEXT,
    extraction_method TEXT NOT NULL,            -- "llm:<model>" or "tfidf"
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS resume_versions (
    id                   TEXT NOT NULL PRIMARY KEY,   -- UUID
    job_id               TEXT NOT NULL,               -- FK → jobs.id
    application_id       TEXT,                        -- FK → applications.id (nullable)
    content_json         TEXT NOT NULL,               -- serialized ResumeContent
    docx_bytes           BLOB NOT NULL,
    content_hash         TEXT NOT NULL,               -- SHA-256 hex of docx_bytes
    gap_report_json      TEXT NOT NULL,
    fabrication_report_json TEXT NOT NULL,
    tailoring_options_json  TEXT NOT NULL,
    label                TEXT NOT NULL DEFAULT '',
    is_submitted         INTEGER NOT NULL DEFAULT 0,  -- BOOLEAN
    created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE,
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_resume_versions_job_id
    ON resume_versions(job_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_resume_versions_application_id
    ON resume_versions(application_id)
    WHERE application_id IS NOT NULL;

-- Ensure no duplicate DOCX content for the same job (avoids re-generating unchanged versions).
CREATE UNIQUE INDEX IF NOT EXISTS idx_resume_versions_content_hash_per_job
    ON resume_versions(job_id, content_hash);
```

### Module Structure

```
lazyjob-core/
  src/
    resume/
      mod.rs           # pub use; ResumeTailor orchestrator
      types.rs         # All domain types (above)
      repository.rs    # ResumeVersionRepository + JdAnalysisRepository traits
      sqlite.rs        # SqliteResumeVersionRepository + SqliteJdAnalysisRepository
      pipeline.rs      # PipelineStage<I,O> trait
      jd_parser.rs     # Stage 1: LLM extraction + TF-IDF fallback
      life_sheet_analyzer.rs  # Stage 2: profile matching (pure Rust)
      gap_analyzer.rs  # Stage 3: skill gap classification + FabricationLevel
      drafter.rs       # Stage 4: LLM bullet rewriting + summary generation
      docx_generator.rs # Stage 5: docx-rs DOCX construction
      fabrication.rs   # Stage 6: audit and keyword stuffing check
      tfidf.rs         # TF-IDF keyword extraction fallback (pure Rust)
      skill_normalizer.rs  # Skill alias dictionary + ESCO proximity check
      diff.rs          # Compute unified diff between two ResumeContent trees
  migrations/
    008_resume_versions.sql
lazyjob-tui/
  src/
    panels/
      resume_diff.rs   # ResumeDiffWidget: side-by-side or inline diff view
      version_browser.rs # VersionBrowserPanel: list of saved versions per job
      fabrication_report.rs # FabricationReportPanel: warnings + Forbidden blocks
```

---

## Implementation Phases

### Phase 1 — Core Pipeline (MVP)

**Goal:** `ResumeTailor::tailor()` returns a `TailoredResume` with all 6 stages working end-to-end, persisted to SQLite. CLI-only invocation (`lazyjob resume tailor <job-id>`).

#### Step 1.1 — Migration and Repository

**File:** `lazyjob-core/migrations/008_resume_versions.sql`

Write the DDL above. Apply in `Database::run_migrations()` using the same ordered migration runner established in the SQLite persistence plan.

**File:** `lazyjob-core/src/resume/sqlite.rs`

```rust
pub struct SqliteResumeVersionRepository {
    pool: sqlx::Pool<sqlx::Sqlite>,
}

impl SqliteResumeVersionRepository {
    pub fn new(pool: sqlx::Pool<sqlx::Sqlite>) -> Self { Self { pool } }
}

#[async_trait]
impl ResumeVersionRepository for SqliteResumeVersionRepository {
    async fn save(&self, version: &ResumeVersion) -> Result<ResumeVersionId> {
        sqlx::query!(
            "INSERT OR IGNORE INTO resume_versions
             (id, job_id, application_id, content_json, docx_bytes, content_hash,
              gap_report_json, fabrication_report_json, tailoring_options_json,
              label, is_submitted, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            version.id.0,
            version.job_id,
            version.application_id,
            serde_json::to_string(&version.content)?,
            version.docx_bytes,
            version.content_hash,
            serde_json::to_string(&version.gap_report)?,
            serde_json::to_string(&version.fabrication_report)?,
            serde_json::to_string(&version.tailoring_options)?,
            version.label,
            version.is_submitted,
            version.created_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(version.id.clone())
    }

    async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<ResumeVersion>> {
        let rows = sqlx::query!(
            "SELECT * FROM resume_versions WHERE job_id = ? ORDER BY created_at DESC",
            job_id
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| {
            Ok(ResumeVersion {
                id: ResumeVersionId(Uuid::parse_str(&r.id)?),
                job_id: Uuid::parse_str(&r.job_id)?,
                application_id: r.application_id.as_deref().map(Uuid::parse_str).transpose()?,
                content: serde_json::from_str(&r.content_json)?,
                docx_bytes: r.docx_bytes,
                content_hash: r.content_hash,
                gap_report: serde_json::from_str(&r.gap_report_json)?,
                fabrication_report: serde_json::from_str(&r.fabrication_report_json)?,
                tailoring_options: serde_json::from_str(&r.tailoring_options_json)?,
                label: r.label,
                is_submitted: r.is_submitted != 0,
                created_at: r.created_at.parse()?,
            })
        }).collect()
    }
    // ... remaining methods follow same pattern
}
```

**Verification:** `cargo test resume::sqlite` with `#[sqlx::test(migrations = "migrations")]` — insert a version, list it back, assert equality.

#### Step 1.2 — Skill Normalizer

**File:** `lazyjob-core/src/resume/skill_normalizer.rs`

This is a critical shared utility used by both Stage 2 (LifeSheet analysis) and Stage 3 (gap analysis).

```rust
use once_cell::sync::Lazy;
use std::collections::HashMap;
use strsim::jaro_winkler;

/// Known aliases: lowercase normalized → canonical name.
/// Loaded from an embedded TOML file at compile time.
static ALIAS_MAP: Lazy<HashMap<String, String>> = Lazy::new(|| {
    let raw = include_str!("../data/skill_aliases.toml");
    // parse as toml::Table, flatten to HashMap<String, String>
    toml::from_str::<toml::Table>(raw)
        .expect("skill_aliases.toml must be valid TOML")
        .into_iter()
        .flat_map(|(canonical, aliases)| {
            aliases
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|v| v.as_str().map(|s| (s.to_lowercase(), canonical.clone())))
                .collect::<Vec<_>>()
        })
        .collect()
});

pub fn normalize(skill: &str) -> String {
    let lower = skill.to_lowercase();
    ALIAS_MAP.get(&lower).cloned().unwrap_or_else(|| lower)
}

/// Returns the match type if two skill names are considered equivalent.
pub fn match_type(a: &str, b: &str) -> Option<SkillMatchType> {
    let na = normalize(a);
    let nb = normalize(b);
    if na == nb {
        if a.to_lowercase() == b.to_lowercase() {
            return Some(SkillMatchType::Exact);
        }
        // One was resolved through alias map
        return Some(SkillMatchType::Alias);
    }
    if jaro_winkler(&na, &nb) >= 0.88 {
        return Some(SkillMatchType::FuzzyString);
    }
    None
}
```

Embedded alias file at `lazyjob-core/src/data/skill_aliases.toml` (seed with ~100 common aliases):

```toml
"TypeScript" = ["ts", "typescript"]
"JavaScript" = ["js", "javascript", "ecmascript"]
"Kubernetes" = ["k8s", "kubernetes"]
"PostgreSQL"  = ["postgres", "postgresql", "pg"]
"React"       = ["react.js", "reactjs"]
"Node.js"     = ["nodejs", "node"]
"AWS"         = ["amazon web services", "aws cloud"]
# ... 90 more
```

**Verification:** Unit test confirming `match_type("React.js", "React")` returns `Some(Alias)`, `match_type("Elasticsearch", "ElasticSearch")` returns `Some(FuzzyString)`.

#### Step 1.3 — Stage 1: JD Parser

**File:** `lazyjob-core/src/resume/jd_parser.rs`

```rust
use crate::llm::{LlmProvider, ChatMessage, Role};
use super::types::*;
use super::tfidf::TfIdfExtractor;
use anyhow::Context;
use std::sync::Arc;

pub struct JdParser {
    pub llm: Arc<dyn LlmProvider>,
}

impl JdParser {
    pub async fn parse(&self, job_id: Uuid, raw_jd: &str) -> Result<JobDescriptionAnalysis> {
        let sanitized = ammonia::Builder::new()
            .tags(std::collections::HashSet::new())
            .clean(raw_jd)
            .to_string();

        match self.parse_with_llm(job_id, &sanitized).await {
            Ok(analysis) => Ok(analysis),
            Err(e) => {
                tracing::warn!("LLM JD parsing failed, falling back to TF-IDF: {e:#}");
                self.parse_with_tfidf(job_id, &sanitized).await
            }
        }
    }

    async fn parse_with_llm(
        &self,
        job_id: Uuid,
        jd: &str,
    ) -> Result<JobDescriptionAnalysis> {
        let prompt = format!(
            r#"Analyze this job description and extract structured requirements.

Return ONLY valid JSON with this exact schema:
{{
  "required_skills": ["string"],
  "nice_to_have_skills": ["string"],
  "keywords": [
    {{"text": "string", "tier": "Tier1Required|Tier2NiceToHave|Tier3Culture", "raw_frequency": 1}}
  ],
  "culture_signals": ["string"],
  "experience_years_required": null,
  "seniority_level": "Junior|MidLevel|Senior|Staff|Principal|null"
}}

Job description:
{jd}"#
        );

        let messages = vec![ChatMessage { role: Role::User, content: prompt }];
        let response = self.llm.chat(messages, None).await
            .context("LLM JD parsing request failed")?;

        let parsed: serde_json::Value = serde_json::from_str(&response.content)
            .context("LLM returned non-JSON response for JD analysis")?;

        Ok(JobDescriptionAnalysis {
            job_id,
            required_skills: serde_json::from_value(parsed["required_skills"].clone())?,
            nice_to_have_skills: serde_json::from_value(parsed["nice_to_have_skills"].clone())?,
            keywords: serde_json::from_value(parsed["keywords"].clone())?,
            culture_signals: serde_json::from_value(parsed["culture_signals"].clone())?,
            experience_years_required: parsed["experience_years_required"].as_u64().map(|v| v as u8),
            seniority_level: parsed["seniority_level"].as_str()
                .and_then(|s| serde_json::from_str(&format!(r#""{s}""#)).ok()),
            extraction_method: ExtractionMethod::Llm {
                model: self.llm.model_name().to_string(),
            },
            created_at: chrono::Utc::now(),
        })
    }

    async fn parse_with_tfidf(
        &self,
        job_id: Uuid,
        jd: &str,
    ) -> Result<JobDescriptionAnalysis> {
        let extractor = TfIdfExtractor::new();
        let keywords = extractor.extract(jd, 30);

        Ok(JobDescriptionAnalysis {
            job_id,
            required_skills: keywords.iter()
                .filter(|k| k.score > 0.5)
                .map(|k| k.term.clone())
                .collect(),
            nice_to_have_skills: vec![],
            keywords: keywords.iter().map(|k| JdKeyword {
                text: k.term.clone(),
                tier: KeywordTier::Tier1Required,
                raw_frequency: k.frequency as u8,
            }).collect(),
            culture_signals: vec![],
            experience_years_required: None,
            seniority_level: None,
            extraction_method: ExtractionMethod::TfIdfFallback,
            created_at: chrono::Utc::now(),
        })
    }
}
```

**File:** `lazyjob-core/src/resume/tfidf.rs`

Pure Rust TF-IDF over a single document corpus. Uses `regex` for tokenization, a static stop-word set via `once_cell::sync::Lazy<HashSet<&'static str>>`, and term frequency × inverse document frequency scoring (IDF approximated from a corpus size constant of 10,000 documents).

**Verification:** `parse(job_id, SAMPLE_JD)` returns `required_skills` containing "Rust" and "async" for a Rust backend JD. Test with both LLM mock (recording the response) and TF-IDF path by disabling LLM in options.

#### Step 1.4 — Stage 2: LifeSheet Analyzer

**File:** `lazyjob-core/src/resume/life_sheet_analyzer.rs`

This stage operates purely in Rust against the LifeSheet data already in SQLite. No LLM call.

```rust
use crate::life_sheet::{LifeSheet, SkillEntry};
use super::types::*;
use super::skill_normalizer::{normalize, match_type as skill_match_type};
use std::collections::HashMap;

pub struct LifeSheetAnalyzer;

pub struct ProfileAnalysis {
    /// Matched skills with their LifeSheet source entry and match type.
    pub matched_skills: Vec<MatchedSkill>,
    /// JD required skills with no counterpart in LifeSheet.
    pub unmatched_required: Vec<String>,
    /// Per-experience-entry relevance score 0.0–1.0.
    pub experience_relevance: Vec<ExperienceRelevance>,
    /// 3–5 example bullets selected for style sampling in Stage 4.
    pub style_exemplars: Vec<String>,
}

pub struct ExperienceRelevance {
    pub experience_index: usize,
    pub score: f32,
    pub matching_skills: Vec<String>,
}

impl LifeSheetAnalyzer {
    pub fn analyze(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
    ) -> ProfileAnalysis {
        let jd_normalized: HashMap<String, &JdKeyword> = jd
            .keywords
            .iter()
            .map(|k| (normalize(&k.text), k))
            .collect();

        // Build matched/unmatched skill lists.
        let mut matched = vec![];
        let mut unmatched = vec![];

        for required in &jd.required_skills {
            let norm_req = normalize(required);
            let found = life_sheet.skills.iter().find_map(|entry| {
                skill_match_type(required, &entry.name).map(|mt| (entry, mt))
            });
            match found {
                Some((entry, mt)) => matched.push(MatchedSkill {
                    jd_name: required.clone(),
                    life_sheet_name: entry.name.clone(),
                    match_type: mt,
                }),
                None => unmatched.push(required.clone()),
            }
        }

        // Score each experience entry by keyword overlap.
        let experience_relevance: Vec<ExperienceRelevance> = life_sheet
            .experience
            .iter()
            .enumerate()
            .map(|(i, exp)| {
                let exp_text = format!(
                    "{} {} {}",
                    exp.title,
                    exp.company,
                    exp.bullets.join(" ")
                );
                let matching: Vec<String> = jd.required_skills
                    .iter()
                    .filter(|skill| {
                        let ns = normalize(skill);
                        exp_text.to_lowercase().contains(&ns)
                    })
                    .cloned()
                    .collect();
                let score = matching.len() as f32
                    / jd.required_skills.len().max(1) as f32;
                ExperienceRelevance {
                    experience_index: i,
                    score,
                    matching_skills: matching,
                }
            })
            .collect();

        // Select style exemplars: pick the 5 longest bullets from highest-scoring entries.
        let style_exemplars = {
            let mut ranked_entries: Vec<_> = experience_relevance
                .iter()
                .collect();
            ranked_entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

            ranked_entries
                .iter()
                .take(3)
                .flat_map(|er| {
                    life_sheet.experience[er.experience_index]
                        .bullets
                        .iter()
                        .cloned()
                })
                .filter(|b| b.split_whitespace().count() >= 10)
                .take(5)
                .collect()
        };

        ProfileAnalysis {
            matched_skills: matched,
            unmatched_required: unmatched,
            experience_relevance,
            style_exemplars,
        }
    }
}
```

**Verification:** Unit test with a synthetic `LifeSheet` and `JobDescriptionAnalysis`; assert correct matched/unmatched counts and that style exemplars are non-empty for a typical profile.

#### Step 1.5 — Stage 3: Gap Analyzer

**File:** `lazyjob-core/src/resume/gap_analyzer.rs`

Classifies each unmatched skill into a `FabricationLevel` using the `is_grounded_claim()` oracle from the LifeSheet module (established in `profile-life-sheet-data-model-implementation-plan.md`).

```rust
use crate::life_sheet::{LifeSheet, is_grounded_claim};
use super::types::*;
use super::skill_normalizer::normalize;

pub struct GapAnalyzer;

impl GapAnalyzer {
    pub fn analyze(
        &self,
        life_sheet: &LifeSheet,
        profile_analysis: &ProfileAnalysis,
        jd: &JobDescriptionAnalysis,
    ) -> GapReport {
        let total_required = jd.required_skills.len();

        let missing_skills: Vec<MissingSkill> = profile_analysis
            .unmatched_required
            .iter()
            .map(|skill_name| {
                let level = self.classify(life_sheet, skill_name, jd);
                let reason = self.explain(&level, skill_name, life_sheet);
                MissingSkill {
                    name: skill_name.clone(),
                    is_required: true,
                    fabrication_level: level,
                    reason,
                }
            })
            .collect();

        let matched_count = profile_analysis.matched_skills.len();
        let match_score = matched_count as f32 / total_required.max(1) as f32;
        let tier1_required: Vec<_> = jd.required_skills.iter().collect();
        let tier1_matched = profile_analysis.matched_skills.iter()
            .filter(|m| tier1_required.contains(&&m.jd_name))
            .count();
        let tier1_coverage = tier1_matched as f32 / tier1_required.len().max(1) as f32;

        GapReport {
            matched_skills: profile_analysis.matched_skills.clone(),
            missing_skills,
            match_score,
            tier1_coverage,
        }
    }

    fn classify(
        &self,
        life_sheet: &LifeSheet,
        skill_name: &str,
        jd: &JobDescriptionAnalysis,
    ) -> FabricationLevel {
        // Credentials/degrees get Forbidden immediately.
        static CREDENTIAL_PATTERNS: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"(?i)(PhD|master'?s|bachelor'?s|MBA|CPA|PMP|CISSP|PE license|bar admission)").unwrap()
        });
        if CREDENTIAL_PATTERNS.is_match(skill_name) {
            let in_life_sheet = life_sheet.education.iter().any(|e| {
                e.degree.to_lowercase().contains(&skill_name.to_lowercase())
            });
            if !in_life_sheet {
                return FabricationLevel::Forbidden;
            }
        }

        // Use is_grounded_claim for a broader evidence check.
        let claim_text = format!("experience with {skill_name}");
        if is_grounded_claim(&claim_text, life_sheet) {
            return FabricationLevel::Safe;
        }

        // Check for adjacent skills (same ESCO category or related domain).
        let is_adjacent = self.has_adjacent_skill(life_sheet, skill_name);
        if is_adjacent {
            return FabricationLevel::Acceptable;
        }

        FabricationLevel::Risky
    }

    fn has_adjacent_skill(&self, life_sheet: &LifeSheet, skill_name: &str) -> bool {
        // Adjacency: both skills share an ESCO skill group, or are in the
        // same user-defined skill category with a fuzzy match threshold >= 0.70.
        let norm = normalize(skill_name);
        life_sheet.skills.iter().any(|entry| {
            let entry_norm = normalize(&entry.name);
            strsim::jaro_winkler(&norm, &entry_norm) >= 0.70
        })
    }

    fn explain(&self, level: &FabricationLevel, skill: &str, _ls: &LifeSheet) -> String {
        match level {
            FabricationLevel::Safe => format!("`{skill}` found in LifeSheet — rephrasing only"),
            FabricationLevel::Acceptable => format!("`{skill}` has an adjacent skill — use 'familiar with' language"),
            FabricationLevel::Risky => format!("`{skill}` has no evidence in LifeSheet — excluded from resume"),
            FabricationLevel::Forbidden => format!("`{skill}` is a credential not found in education history — BLOCKED"),
        }
    }
}
```

**Verification:** Test a skill matching `Forbidden` pattern ("PhD in ML") against a LifeSheet without that education entry. Test `Acceptable` for "Kafka" when LifeSheet has "RabbitMQ" in the same messaging category.

#### Step 1.6 — Stage 4: Content Drafter

**File:** `lazyjob-core/src/resume/drafter.rs`

```rust
use crate::llm::{LlmProvider, ChatMessage, Role};
use crate::life_sheet::LifeSheet;
use super::types::*;

pub struct ContentDrafter {
    pub llm: Arc<dyn LlmProvider>,
}

impl ContentDrafter {
    pub async fn draft(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gap_report: &GapReport,
        profile_analysis: &ProfileAnalysis,
        options: &TailoringOptions,
    ) -> Result<ResumeContent> {
        // 1. Rewrite each bullet in high-relevance experience entries.
        let rewritten_experience = self
            .rewrite_bullets(life_sheet, jd, profile_analysis)
            .await?;

        // 2. Generate targeted summary.
        let summary = self
            .draft_summary(life_sheet, jd, &profile_analysis.style_exemplars)
            .await?;

        // 3. Build skills section ordered by relevance.
        let skills = self.order_skills(life_sheet, jd, gap_report);

        // 4. Select education entries (drop ancient ones if > max_pages constraint).
        let education = life_sheet
            .education
            .iter()
            .map(|e| EducationEntry {
                institution: e.institution.clone(),
                degree: e.degree.clone(),
                field: e.field.clone(),
                graduation_year: e.graduation_year,
                included: true,
            })
            .collect();

        // 5. Select projects (if enabled and relevant).
        let projects = if options.include_projects {
            self.select_projects(life_sheet, jd)
        } else {
            vec![]
        };

        Ok(ResumeContent {
            header: self.build_header(life_sheet),
            summary,
            experience: rewritten_experience,
            skills,
            education,
            projects,
        })
    }

    async fn rewrite_bullets(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        profile_analysis: &ProfileAnalysis,
    ) -> Result<Vec<ExperienceSection>> {
        // Top-10 Tier1 keywords for injection prompt.
        let top_keywords: Vec<&str> = jd
            .keywords
            .iter()
            .filter(|k| k.tier == KeywordTier::Tier1Required)
            .take(10)
            .map(|k| k.text.as_str())
            .collect();

        // Style exemplars: few-shot samples preserving user voice.
        let exemplars = profile_analysis.style_exemplars.join("\n- ");

        let mut sections = vec![];

        for (i, exp) in life_sheet.experience.iter().enumerate() {
            let relevance = profile_analysis
                .experience_relevance
                .iter()
                .find(|r| r.experience_index == i);

            // Only rewrite bullets in entries with relevance score > 0.1.
            let bullets = if relevance.map(|r| r.score).unwrap_or(0.0) > 0.1 {
                self.rewrite_entry_bullets(exp, &top_keywords, &exemplars)
                    .await?
            } else {
                exp.bullets.clone()
            };

            sections.push(ExperienceSection {
                company: exp.company.clone(),
                title: exp.title.clone(),
                start_date: exp.start_date.clone(),
                end_date: exp.end_date.clone(),
                is_current: exp.is_current,
                bullets,
                original_bullets: exp.bullets.clone(),
            });
        }
        Ok(sections)
    }

    async fn rewrite_entry_bullets(
        &self,
        exp: &crate::life_sheet::ExperienceEntry,
        keywords: &[&str],
        exemplars: &str,
    ) -> Result<Vec<String>> {
        let bullets_text = exp.bullets.join("\n- ");
        let keywords_text = keywords.join(", ");

        let prompt = format!(
            r#"Rewrite these resume bullets to incorporate the target keywords naturally.

Rules:
- Keep all achievements based on real accomplishments listed below. DO NOT add new achievements.
- Incorporate keywords where they naturally fit; do not force them.
- Preserve the writing style shown in these examples:
  - {exemplars}
- Return ONLY a JSON array of strings (the rewritten bullets), no other text.

Original bullets:
- {bullets_text}

Target keywords: {keywords_text}"#
        );

        let messages = vec![ChatMessage { role: Role::User, content: prompt }];
        let response = self.llm.chat(messages, None).await?;
        let rewritten: Vec<String> = serde_json::from_str(&response.content)
            .context("LLM returned non-JSON bullet array")?;
        Ok(rewritten)
    }

    async fn draft_summary(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        exemplars: &[String],
    ) -> Result<String> {
        let years_exp = life_sheet.total_years_of_experience();
        let top_skills: Vec<_> = life_sheet.skills.iter().take(5).map(|s| &s.name).collect();
        let keywords_str = jd.required_skills.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        let exemplars_str = exemplars.iter().take(3).cloned().collect::<Vec<_>>().join(" ");

        let prompt = format!(
            r#"Write a 2-3 sentence professional resume summary.
Constraints:
- Only reference real experience (provided below).
- Naturally include these keywords: {keywords_str}
- Match the writing style of: {exemplars_str}
- Do not start with "I" or be written in first person.
- Return ONLY the summary text, no JSON, no quotes.

Profile: {years_exp} years of experience. Top skills: {top_skills:?}."#
        );

        let messages = vec![ChatMessage { role: Role::User, content: prompt }];
        let response = self.llm.chat(messages, None).await?;
        Ok(response.content.trim().to_string())
    }

    fn order_skills(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        _gap_report: &GapReport,
    ) -> Vec<SkillCategory> {
        // Float categories that contain JD required skills to the top.
        let jd_required_normalized: Vec<String> = jd.required_skills
            .iter()
            .map(|s| super::skill_normalizer::normalize(s))
            .collect();

        let mut categories: Vec<SkillCategory> = life_sheet
            .skill_categories
            .iter()
            .map(|cat| {
                let relevance_count = cat.skills.iter()
                    .filter(|s| jd_required_normalized.contains(&super::skill_normalizer::normalize(s)))
                    .count();
                SkillCategory {
                    category: cat.name.clone(),
                    skills: cat.skills.clone(),
                    display_order: (10u8.saturating_sub(relevance_count as u8)),
                }
            })
            .collect();

        categories.sort_by_key(|c| c.display_order);
        categories
    }

    fn select_projects(
        &self,
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
    ) -> Vec<ProjectEntry> {
        let jd_skills: Vec<String> = jd.required_skills.iter()
            .map(|s| super::skill_normalizer::normalize(s))
            .collect();

        life_sheet.projects.iter()
            .filter(|p| {
                p.tech_stack.iter().any(|t| {
                    jd_skills.contains(&super::skill_normalizer::normalize(t))
                })
            })
            .map(|p| ProjectEntry {
                name: p.name.clone(),
                description: p.description.clone(),
                tech_stack: p.tech_stack.clone(),
                url: p.url.clone(),
                included: true,
            })
            .take(3)
            .collect()
    }

    fn build_header(&self, life_sheet: &LifeSheet) -> ResumeHeader {
        ResumeHeader {
            full_name: life_sheet.personal.full_name.clone(),
            email: life_sheet.personal.email.clone(),
            phone: life_sheet.personal.phone.clone(),
            location: life_sheet.personal.location.clone(),
            linkedin_url: life_sheet.personal.linkedin_url.clone(),
            github_url: life_sheet.personal.github_url.clone(),
        }
    }
}
```

**Verification:** Mock the `LlmProvider` trait, assert that `rewrite_bullets` is only called for entries with relevance > 0.1. Assert summary does not start with "I".

#### Step 1.7 — Stage 5: DOCX Generator

**File:** `lazyjob-core/src/resume/docx_generator.rs`

Uses `docx-rs 0.4` to produce a single-column, ATS-safe document. No table-based layouts, no text boxes, no fancy fonts — plain paragraph + character styles only.

```rust
use docx_rs::*;
use super::types::*;

pub struct DocxGenerator;

impl DocxGenerator {
    pub fn generate(&self, content: &ResumeContent, options: &TailoringOptions) -> Result<Vec<u8>> {
        let mut docx = Docx::new();

        // Page margins: 0.75" all around (standard ATS).
        docx = docx.page_margin(PageMargin::new()
            .top(1080)   // 1080 twips = 0.75"
            .bottom(1080)
            .left(1080)
            .right(1080)
        );

        // Header: Name in 16pt bold, contact info in 10pt.
        docx = self.add_header(docx, &content.header);

        // Section divider line.
        docx = self.add_section_rule(docx, "PROFESSIONAL SUMMARY");
        docx = self.add_paragraph(docx, &content.summary, "Normal");

        docx = self.add_section_rule(docx, "EXPERIENCE");
        for exp in &content.experience {
            docx = self.add_experience_section(docx, exp);
        }

        docx = self.add_section_rule(docx, "SKILLS");
        for cat in &content.skills {
            let skills_line = format!("{}: {}", cat.category, cat.skills.join(", "));
            docx = self.add_paragraph(docx, &skills_line, "Normal");
        }

        docx = self.add_section_rule(docx, "EDUCATION");
        for edu in content.education.iter().filter(|e| e.included) {
            let line = format!(
                "{} — {}, {}{}",
                edu.institution,
                edu.degree,
                edu.field,
                edu.graduation_year.map(|y| format!(", {y}")).unwrap_or_default()
            );
            docx = self.add_paragraph(docx, &line, "Normal");
        }

        if !content.projects.is_empty() {
            docx = self.add_section_rule(docx, "PROJECTS");
            for proj in content.projects.iter().filter(|p| p.included) {
                docx = self.add_project_entry(docx, proj);
            }
        }

        let bytes = docx.build().pack()?;
        Ok(bytes)
    }

    fn add_header(&self, docx: Docx, header: &ResumeHeader) -> Docx {
        let name_run = Run::new()
            .add_text(&header.full_name)
            .bold()
            .size(32);  // 16pt = 32 half-points in OOXML

        let contact_parts = [
            Some(header.email.as_str()),
            header.phone.as_deref(),
            header.location.as_deref(),
            header.linkedin_url.as_deref(),
            header.github_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  |  ");

        docx.add_paragraph(Paragraph::new().add_run(name_run))
            .add_paragraph(
                Paragraph::new().add_run(
                    Run::new().add_text(&contact_parts).size(20)
                )
            )
    }

    fn add_experience_section(&self, mut docx: Docx, exp: &ExperienceSection) -> Docx {
        let date_range = match &exp.end_date {
            Some(end) => format!("{} – {}", exp.start_date, end),
            None if exp.is_current => format!("{} – Present", exp.start_date),
            None => exp.start_date.clone(),
        };
        let title_line = format!("{}, {} | {}", exp.title, exp.company, date_range);
        docx = self.add_paragraph(docx, &title_line, "Heading2");
        for bullet in &exp.bullets {
            docx = self.add_bullet(docx, bullet);
        }
        docx
    }

    fn add_project_entry(&self, mut docx: Docx, proj: &ProjectEntry) -> Docx {
        let title = format!("{} — {}", proj.name, proj.tech_stack.join(", "));
        docx = self.add_paragraph(docx, &title, "Heading3");
        docx = self.add_paragraph(docx, &proj.description, "Normal");
        docx
    }

    fn add_section_rule(&self, docx: Docx, title: &str) -> Docx {
        docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(title).bold().size(22))
                .add_border(ParagraphBorder::new().bottom(BorderType::Single, 6, "000000"))
        )
    }

    fn add_paragraph(&self, docx: Docx, text: &str, _style: &str) -> Docx {
        docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(text).size(20)))
    }

    fn add_bullet(&self, docx: Docx, text: &str) -> Docx {
        docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(format!("• {text}")).size(20))
                .indent(Some(360), None, None, None)  // 0.25" hanging indent
        )
    }
}
```

**Verification:** Call `generate()` with a sample `ResumeContent`, write the bytes to a temp file, assert file length > 1000 bytes and filename extension is `.docx`. The file must be openable in LibreOffice Writer.

#### Step 1.8 — Stage 6: Fabrication Auditor

**File:** `lazyjob-core/src/resume/fabrication.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;
use super::types::*;

static CREDENTIAL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(PhD|master'?s|bachelor'?s|MBA|PMP|CISSP|CPA|bar admission|licensed engineer)").unwrap()
});

pub struct FabricationAuditor;

impl FabricationAuditor {
    pub fn audit(
        &self,
        content: &ResumeContent,
        gap_report: &GapReport,
        jd: &JobDescriptionAnalysis,
    ) -> FabricationReport {
        let mut flags: Vec<FabricationFlag> = vec![];

        // Flag any Risky or Forbidden missing skills that appear in the drafted content.
        for missing in gap_report.missing_skills.iter()
            .filter(|m| m.fabrication_level >= FabricationLevel::Risky)
        {
            let locations = self.find_in_content(content, &missing.name);
            for location in locations {
                flags.push(FabricationFlag {
                    skill_name: missing.name.clone(),
                    level: missing.fabrication_level,
                    location,
                    reason: missing.reason.clone(),
                });
            }
        }

        // Check for keyword stuffing: count each Tier1 keyword across all text.
        let keyword_stuffing_flags = self.check_keyword_stuffing(content, jd);

        let has_forbidden = flags.iter().any(|f| f.level == FabricationLevel::Forbidden);
        let warnings: Vec<String> = flags
            .iter()
            .filter(|f| f.level == FabricationLevel::Risky)
            .map(|f| format!("WARNING: '{}' in {:?} — no LifeSheet evidence", f.skill_name, f.location))
            .collect();

        FabricationReport {
            flags,
            is_submittable: !has_forbidden,
            warnings,
            keyword_stuffing_flags,
        }
    }

    fn find_in_content(&self, content: &ResumeContent, skill: &str) -> Vec<FlagLocation> {
        let skill_lower = skill.to_lowercase();
        let mut locations = vec![];

        if content.summary.to_lowercase().contains(&skill_lower) {
            locations.push(FlagLocation::Summary);
        }
        for exp in &content.experience {
            for (i, bullet) in exp.bullets.iter().enumerate() {
                if bullet.to_lowercase().contains(&skill_lower) {
                    locations.push(FlagLocation::ExperienceBullet {
                        company: exp.company.clone(),
                        bullet_index: i,
                    });
                }
            }
        }
        for cat in &content.skills {
            for skill_entry in &cat.skills {
                if skill_entry.to_lowercase().contains(&skill_lower) {
                    locations.push(FlagLocation::SkillsSection {
                        category: cat.category.clone(),
                    });
                }
            }
        }
        locations
    }

    fn check_keyword_stuffing(
        &self,
        content: &ResumeContent,
        jd: &JobDescriptionAnalysis,
    ) -> Vec<String> {
        let all_text = self.flatten_text(content);
        jd.keywords
            .iter()
            .filter(|kw| kw.tier == KeywordTier::Tier1Required)
            .filter_map(|kw| {
                let lower = kw.text.to_lowercase();
                let count = all_text.matches(&lower).count();
                if count > 4 {
                    Some(format!("'{}' appears {} times (>4) — keyword stuffing risk", kw.text, count))
                } else {
                    None
                }
            })
            .collect()
    }

    fn flatten_text(&self, content: &ResumeContent) -> String {
        let mut parts = vec![content.summary.clone()];
        for exp in &content.experience {
            parts.extend_from_slice(&exp.bullets);
        }
        for cat in &content.skills {
            parts.extend_from_slice(&cat.skills);
        }
        parts.join(" ").to_lowercase()
    }
}
```

**Verification:** Test that a Forbidden skill present in a bullet generates `is_submittable: false`. Test keyword stuffing detection with a repeated keyword > 4 times.

#### Step 1.9 — ResumeTailor Orchestrator

**File:** `lazyjob-core/src/resume/mod.rs`

```rust
use std::sync::Arc;
use sha2::{Sha256, Digest};
use crate::llm::LlmProvider;
use crate::life_sheet::LifeSheetRepository;
use crate::jobs::JobRepository;
use super::types::*;
use super::{
    jd_parser::JdParser,
    life_sheet_analyzer::LifeSheetAnalyzer,
    gap_analyzer::GapAnalyzer,
    drafter::ContentDrafter,
    docx_generator::DocxGenerator,
    fabrication::FabricationAuditor,
    repository::{ResumeVersionRepository, JdAnalysisRepository},
};

pub struct ResumeTailor {
    pub llm: Arc<dyn LlmProvider>,
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub version_repo: Arc<dyn ResumeVersionRepository>,
    pub jd_analysis_repo: Arc<dyn JdAnalysisRepository>,
}

impl ResumeTailor {
    pub async fn tailor(
        &self,
        job_id: &Uuid,
        options: TailoringOptions,
    ) -> Result<TailoredResume> {
        // Load data.
        let job = self.job_repo.get(job_id).await?;
        let life_sheet = self.life_sheet_repo.load_current().await?;

        // Stage 1: Parse JD (use cache if available).
        let jd_analysis = if let Some(cached) = self.jd_analysis_repo.get_for_job(job_id).await? {
            cached
        } else {
            let parser = JdParser { llm: Arc::clone(&self.llm) };
            let analysis = parser.parse(*job_id, &job.description).await?;
            self.jd_analysis_repo.upsert(&analysis).await?;
            analysis
        };

        // Stage 2: LifeSheet analysis (pure Rust, no LLM).
        let analyzer = LifeSheetAnalyzer;
        let profile_analysis = analyzer.analyze(&life_sheet, &jd_analysis);

        // Stage 3: Gap analysis (pure Rust).
        let gap_analyzer = GapAnalyzer;
        let gap_report = gap_analyzer.analyze(&life_sheet, &profile_analysis, &jd_analysis);

        // Stage 4: Content drafting (LLM).
        let drafter = ContentDrafter { llm: Arc::clone(&self.llm) };
        let content = drafter.draft(
            &life_sheet,
            &jd_analysis,
            &gap_report,
            &profile_analysis,
            &options,
        ).await?;

        // Stage 5: DOCX generation.
        let generator = DocxGenerator;
        let docx_bytes = generator.generate(&content, &options)?;

        // Stage 6: Fabrication audit.
        let auditor = FabricationAuditor;
        let fabrication_report = auditor.audit(&content, &gap_report, &jd_analysis);

        // Hash DOCX for dedup.
        let content_hash = format!("{:x}", Sha256::digest(&docx_bytes));

        // Determine version label.
        let existing_versions = self.version_repo.list_for_job(job_id).await?;
        let label = format!("v{}", existing_versions.len() + 1);

        // Persist version.
        let version_id = ResumeVersionId::new();
        let version = ResumeVersion {
            id: version_id.clone(),
            job_id: *job_id,
            application_id: None,
            content: content.clone(),
            docx_bytes: docx_bytes.clone(),
            content_hash,
            gap_report: gap_report.clone(),
            fabrication_report: fabrication_report.clone(),
            tailoring_options: options,
            label,
            is_submitted: false,
            created_at: chrono::Utc::now(),
        };

        self.version_repo.save(&version).await?;

        Ok(TailoredResume {
            version_id,
            content,
            docx_bytes,
            gap_report,
            fabrication_report,
            jd_analysis,
        })
    }
}
```

**Verification:** Integration test using `#[sqlx::test(migrations = "migrations")]`, mocked `LlmProvider`, synthetic LifeSheet and Job — assert a `ResumeVersion` row is written and the fabrication report is non-empty.

---

### Phase 2 — TUI Integration

**Goal:** Version browser, diff viewer, and fabrication report panel integrated into the existing TUI `App`.

#### Step 2.1 — Version Browser Panel

**File:** `lazyjob-tui/src/panels/version_browser.rs`

```rust
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use lazyjob_core::resume::types::{ResumeVersion, ResumeVersionId};

pub struct VersionBrowserPanel {
    pub versions: Vec<ResumeVersion>,
    pub list_state: ListState,
    pub selected: Option<ResumeVersionId>,
}

impl VersionBrowserPanel {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.versions
            .iter()
            .map(|v| {
                let submitted_marker = if v.is_submitted { " [submitted]" } else { "" };
                let score = (v.gap_report.match_score * 100.0) as u8;
                let safe = if v.fabrication_report.is_submittable { "✓" } else { "⚠" };
                ListItem::new(format!(
                    "{} | Match {}% | Fab {safe} | {}{}",
                    v.label, score,
                    v.created_at.format("%Y-%m-%d %H:%M"),
                    submitted_marker,
                ))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Resume Versions").borders(Borders::ALL))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}
```

**Keybindings:**
- `j`/`k` — navigate versions
- `<Enter>` — open diff view for selected version
- `d` — download DOCX to `~/Downloads/`
- `D` — delete draft version (with confirm dialog)
- `l` — set label

#### Step 2.2 — Diff Widget

**File:** `lazyjob-tui/src/panels/resume_diff.rs`

Uses `similar::TextDiff` to compare `original_bullets` vs. `bullets` in each `ExperienceSection`, plus summary before/after.

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use similar::{ChangeTag, TextDiff};
use lazyjob_core::resume::types::ResumeVersion;

pub struct ResumeDiffWidget<'a> {
    pub version: &'a ResumeVersion,
    pub scroll_offset: u16,
}

impl<'a> ResumeDiffWidget<'a> {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left: original bullets.
        let original_text = self.collect_original_text();
        // Right: tailored bullets.
        let tailored_text = self.collect_tailored_text();

        // Diff rendering with green/red highlights.
        let diff_lines = self.render_diff_lines(&original_text, &tailored_text);

        let original_para = Paragraph::new(original_text.as_str())
            .block(Block::default().title("Original (LifeSheet)").borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        let tailored_para = Paragraph::new(diff_lines)
            .block(Block::default().title("Tailored (AI-rewritten)").borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(original_para, chunks[0]);
        frame.render_widget(tailored_para, chunks[1]);
    }

    fn render_diff_lines(&self, original: &str, tailored: &str) -> Vec<Line<'static>> {
        let diff = TextDiff::from_words(original, tailored);
        let mut lines = vec![];
        let mut current_line: Vec<Span<'static>> = vec![];

        for change in diff.iter_all_changes() {
            let style = match change.tag() {
                ChangeTag::Delete => Style::default().fg(Color::Red),
                ChangeTag::Insert => Style::default().fg(Color::Green),
                ChangeTag::Equal  => Style::default(),
            };
            let text = change.to_string();
            if text.contains('\n') {
                current_line.push(Span::styled(text.trim_end_matches('\n').to_string(), style));
                lines.push(Line::from(current_line.clone()));
                current_line.clear();
            } else {
                current_line.push(Span::styled(text, style));
            }
        }
        if !current_line.is_empty() {
            lines.push(Line::from(current_line));
        }
        lines
    }

    fn collect_original_text(&self) -> String {
        self.version.content.experience.iter()
            .flat_map(|e| e.original_bullets.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn collect_tailored_text(&self) -> String {
        self.version.content.experience.iter()
            .flat_map(|e| e.bullets.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}
```

#### Step 2.3 — Fabrication Report Panel

**File:** `lazyjob-tui/src/panels/fabrication_report.rs`

Renders `FabricationReport`. If `is_submittable: false`, shows a red header "BLOCKED — Forbidden items present". Lists all flags grouped by severity (Forbidden, Risky, Acceptable). Provides:
- `<a>` — acknowledge all Risky warnings (sets a `RiskyAcknowledged` flag in `App` state)
- `<q>` — dismiss panel
- Shows keyword stuffing notes in yellow.

---

### Phase 3 — Ralph Background Loop Integration

**Goal:** Auto-trigger resume tailoring when the user moves an application to the `Applied` stage, running as a background Ralph loop.

#### Step 3.1 — LoopType::ResumeTailor

In `lazyjob-ralph/src/types.rs`, add:

```rust
pub enum LoopType {
    // ... existing variants ...
    ResumeTailor,
}

impl LoopType {
    pub fn concurrency_limit(&self) -> usize {
        match self {
            LoopType::ResumeTailor => 2,  // Max 2 concurrent tailoring jobs
            // ...
        }
    }
}
```

#### Step 3.2 — Auto-Trigger on ApplyWorkflow

In `ApplyWorkflow::execute` (from `application-workflow-actions-implementation-plan.md`), after successfully writing the application record, check `TailoringOptions::run_as_background_loop` and enqueue a `ResumeTailor` loop:

```rust
if options.tailoring_options.run_as_background_loop {
    self.loop_dispatcher.dispatch(LoopType::ResumeTailor, LoopParams {
        job_id: Some(application.job_id),
        application_id: Some(application.id),
        extra: serde_json::json!({}),
    }).await?;
}
```

#### Step 3.3 — TUI Progress Events

The Ralph subprocess emits `WorkerEvent::Progress { pct: u8, message: String }` for each pipeline stage:
- Stage 1 complete: `{ pct: 16, message: "JD analysis complete" }`
- Stage 4 complete: `{ pct: 67, message: "Content drafted — reviewing fabrication" }`
- Stage 6 complete: `{ pct: 100, message: "Resume ready for review" }`

The `RalphProgressPanel` already established in the orchestration plan consumes these events.

---

### Phase 4 — CLI Subcommand

**File:** `lazyjob-cli/src/commands/resume.rs`

```rust
pub async fn cmd_resume_tailor(args: ResumeTailorArgs, ctx: AppContext) -> anyhow::Result<()> {
    let job_id: Uuid = Uuid::parse_str(&args.job_id)
        .context("Invalid job ID — must be a UUID")?;

    let tailor = ctx.resume_tailor();
    let options = TailoringOptions {
        max_pages: args.max_pages.unwrap_or(1),
        include_projects: args.include_projects,
        format: ResumeFormat::AtsSimple,
        run_as_background_loop: false,
    };

    println!("Tailoring resume for job {}...", args.job_id);
    let result = tailor.tailor(&job_id, options).await?;

    println!("Version {} created. Match score: {:.0}%",
        result.version_id.0, result.gap_report.match_score * 100.0);

    if !result.fabrication_report.is_submittable {
        eprintln!("ERROR: Forbidden fabrication flags present — review required before submission.");
    }

    if let Some(output_path) = args.output {
        std::fs::write(&output_path, &result.docx_bytes)?;
        println!("DOCX written to {}", output_path.display());
    }

    Ok(())
}
```

---

## Key Crate APIs

| Crate | API | Used in |
|-------|-----|---------|
| `docx-rs 0.4` | `Docx::new()`, `.add_paragraph(Paragraph::new().add_run(Run::new().bold().size(N)))`, `.build().pack()` | Stage 5 |
| `strsim 0.11` | `jaro_winkler(a: &str, b: &str) -> f64` | Skill normalizer |
| `similar 2` | `TextDiff::from_words(old, new)`, `.iter_all_changes()`, `ChangeTag::Insert/Delete/Equal` | TUI diff widget |
| `sha2 0.10` | `Sha256::digest(&bytes)` → `GenericArray` → `format!("{:x}", ...)` | Content hash |
| `ammonia 3` | `ammonia::Builder::new().tags(HashSet::new()).clean(html)` | JD HTML sanitization |
| `once_cell 1` | `once_cell::sync::Lazy<Regex>` | Credential detection, TF-IDF stop words |
| `regex 1` | `Regex::new(pattern)`, `.is_match(text)`, `.find_iter(text)` | Credential detection, TF-IDF |
| `sqlx 0.8` | `sqlx::query!()`, `.execute(&pool)`, `.fetch_all(&pool)`, `#[sqlx::test(migrations = "migrations")]` | Repository |
| `serde_json` | `serde_json::to_string(&val)?`, `serde_json::from_str(&s)?` | JSON column serialization |
| `tokio` | `tokio::fs::write(path, bytes).await` | DOCX file export |

---

## Error Handling

```rust
// lazyjob-core/src/resume/error.rs

use thiserror::Error;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, ResumeError>;

#[derive(Error, Debug)]
pub enum ResumeError {
    #[error("job {0} not found")]
    JobNotFound(Uuid),

    #[error("life sheet not found — run `lazyjob profile import` first")]
    LifeSheetNotFound,

    #[error("LLM call failed for stage {stage}: {source}")]
    LlmFailed {
        stage: &'static str,
        #[source]
        source: anyhow::Error,
    },

    #[error("JD analysis returned invalid JSON: {0}")]
    JdParseError(#[from] serde_json::Error),

    #[error("DOCX generation failed: {0}")]
    DocxError(String),

    #[error("resume version {0} not found")]
    VersionNotFound(Uuid),

    #[error("cannot delete a submitted resume version")]
    CannotDeleteSubmitted,

    #[error("fabrication audit: forbidden items present in drafted content — submission blocked")]
    ForbiddenFabricationBlocked,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

---

## Testing Strategy

### Unit Tests

Each pipeline stage has its own unit test module in the same file.

**JD Parser (`jd_parser.rs`):**
- `test_parse_with_llm_success` — mock `LlmProvider` returning valid JSON; assert required_skills contains expected entries
- `test_parse_falls_back_to_tfidf` — mock LLM returning `Err(...)`; assert `extraction_method == TfIdfFallback` and keywords non-empty
- `test_sanitizes_html` — JD with `<script>` tags; assert sanitized version has no HTML

**Skill Normalizer (`skill_normalizer.rs`):**
- `test_alias_map_loaded` — assert `normalize("React.js") == "react.js"` (after alias resolution)
- `test_fuzzy_match` — `match_type("Elasticsearch", "ElasticSearch") == Some(FuzzyString)`
- `test_no_false_match` — `match_type("Rust", "Ruby") == None`

**Gap Analyzer (`gap_analyzer.rs`):**
- `test_credential_detection` — `classify("PhD in Machine Learning")` returns `Forbidden` when LifeSheet has no PhD
- `test_adjacent_skill` — `classify("Kafka")` returns `Acceptable` when LifeSheet contains "RabbitMQ"
- `test_match_score` — 3 matched + 2 missing from 5 required yields `match_score == 0.6`

**Fabrication Auditor (`fabrication.rs`):**
- `test_forbidden_blocks_submission` — flag a Forbidden skill in a bullet; assert `is_submittable == false`
- `test_risky_generates_warning` — flag a Risky skill; assert `warnings.len() == 1` and `is_submittable == true`
- `test_keyword_stuffing` — keyword appears 5 times in content; assert in `keyword_stuffing_flags`

**DOCX Generator (`docx_generator.rs`):**
- `test_generates_nonempty_docx` — sample content; assert `docx_bytes.len() > 1000`
- `test_ats_format_no_tables` — assert no table XML elements in output bytes (parse XML from ZIP to verify)

### Integration Tests

**`tests/resume_tailor_integration.rs`:**

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_tailor_creates_version(pool: sqlx::Pool<sqlx::Sqlite>) {
    let llm = Arc::new(MockLlmProvider::new(vec![
        // Stage 1 JD analysis response
        r#"{"required_skills":["Rust"],"nice_to_have_skills":["Go"],"keywords":[{"text":"Rust","tier":"Tier1Required","raw_frequency":5}],"culture_signals":["fast-paced"],"experience_years_required":3,"seniority_level":"Senior"}"#,
        // Stage 4 bullet rewrite response
        r#"["Led development of async Rust microservices handling 10k req/s"]"#,
        // Stage 4 summary response
        "Senior Rust engineer with 5+ years building distributed systems.",
    ]));
    let life_sheet_repo = Arc::new(InMemoryLifeSheetRepository::with_sample());
    let job_repo = Arc::new(SqliteJobRepository::new(pool.clone()));
    let version_repo = Arc::new(SqliteResumeVersionRepository::new(pool.clone()));
    let jd_repo = Arc::new(SqliteJdAnalysisRepository::new(pool.clone()));

    let job_id = seed_test_job(&pool).await;

    let tailor = ResumeTailor { llm, life_sheet_repo, job_repo, version_repo: Arc::clone(&version_repo), jd_analysis_repo: jd_repo };
    let result = tailor.tailor(&job_id, TailoringOptions::default()).await.unwrap();

    // Version persisted
    let versions = version_repo.list_for_job(&job_id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].label, "v1");

    // Gap report populated
    assert!(!result.gap_report.matched_skills.is_empty());

    // DOCX bytes non-empty
    assert!(result.docx_bytes.len() > 1000);
}
```

### TUI Tests

Use `ratatui::backend::TestBackend` + `Terminal` for panel rendering tests:

```rust
#[test]
fn test_version_browser_renders() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut panel = VersionBrowserPanel {
        versions: vec![sample_version()],
        list_state: ListState::default(),
        selected: None,
    };
    terminal.draw(|f| panel.render(f, f.area())).unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer.content.iter().any(|c| c.symbol() == "v1"));
}
```

---

## Open Questions

1. **PDF export**: DOCX-only in Phase 1. LibreOffice headless is the most robust PDF conversion path but adds a system dependency. Evaluate in Phase 3 if user demand is high.

2. **Custom DOCX templates**: `AtsSimple` only in Phase 1. `AtsClean` (two-column header section) deferred to Phase 2 as it requires `docx-rs` table constructs that complicate ATS compatibility testing.

3. **Version pruning**: Keep all versions indefinitely (≈50KB each). Future: offer a `lazyjob resume prune --keep-last N` CLI command if storage becomes a concern.

4. **ESCO proximity for adjacent skill detection**: `has_adjacent_skill()` currently uses Jaro-Winkler >= 0.70 as a proxy. True ESCO proximity requires the ESCO API integration from `profile-life-sheet-data-model-implementation-plan.md`. Add ESCO-based adjacency in Phase 3 after ESCO integration lands.

5. **Multi-LLM stages**: Currently all LLM stages use the same `Arc<dyn LlmProvider>`. Phase 3 could route Stage 1 (JD parsing) to a cheaper model (e.g., Haiku) and Stage 4 (drafting) to a stronger model (Sonnet) via `LlmRouter`.

6. **Voice drift over multiple re-generations**: Each tailoring call re-generates from scratch, which may produce slightly different bullet styles. Phase 3: offer a "refine" mode that sends the current version as context to the LLM instead of starting from the original LifeSheet bullets.

7. **PDF reading for LifeSheet bootstrap**: Out of scope for this plan. Tracked as a separate feature request.

---

## Related Specs

- [specs/07-resume-tailoring-pipeline.md](./07-resume-tailoring-pipeline.md) — architectural pipeline spec (predates this profile-domain spec)
- [specs/07-resume-tailoring-pipeline-implementation-plan.md](./07-resume-tailoring-pipeline-implementation-plan.md) — pipeline-centric implementation plan
- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) — LifeSheet data model (provides `is_grounded_claim`)
- [specs/profile-life-sheet-data-model-implementation-plan.md](./profile-life-sheet-data-model-implementation-plan.md) — life sheet implementation plan
- [specs/agentic-llm-provider-abstraction-implementation-plan.md](./agentic-llm-provider-abstraction-implementation-plan.md) — LLM provider abstraction
- [specs/application-workflow-actions-implementation-plan.md](./application-workflow-actions-implementation-plan.md) — ApplyWorkflow (Phase 3 trigger point)
- [specs/09-tui-design-keybindings-implementation-plan.md](./09-tui-design-keybindings-implementation-plan.md) — TUI framework
- [specs/agentic-ralph-orchestration-implementation-plan.md](./agentic-ralph-orchestration-implementation-plan.md) — Ralph loop dispatch
