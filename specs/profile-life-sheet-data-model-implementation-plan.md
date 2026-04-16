# Implementation Plan: LifeSheet Data Model

## Status
Draft

## Related Spec
[specs/profile-life-sheet-data-model.md](profile-life-sheet-data-model.md)

## Overview

The LifeSheet is LazyJob's canonical professional identity store — a structured, machine-readable representation of everything a user has ever done professionally. It lives as a human-editable YAML file at `~/.lazyjob/life-sheet.yaml` and is mirrored into SQLite for programmatic access. Every AI-powered feature in LazyJob (resume tailoring, cover letter generation, skills gap analysis, interview prep) reads exclusively from the SQLite mirror, ensuring the anti-fabrication constraint: all AI-generated content must be traceable to an explicit LifeSheet entity.

The dual-layer design is intentional. YAML is the source of truth for the user — transparent, version-controllable, and editable without a GUI. SQLite is the query layer for the application — indexed, joinable, and efficient for relational lookups across skills, experience, and taxonomy codes. Import from YAML to SQLite uses full truncate-and-reimport semantics with deterministic IDs (hashed from content) so that stable IDs survive re-imports even after the YAML is edited out-of-order.

This plan covers Phase 1 (MVP: YAML schema, SQLite DDL, import/export logic, CLI commands, fabrication check) and Phase 2 (incremental diff import, ESCO API auto-tagging, LinkedIn export parser, TUI profile editor). The result is the foundational data layer that all subsequent specs build on.

## Prerequisites

### Plans That Must Be Implemented First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database` struct, `SqlitePool`, WAL-mode setup, migration runner

### Crates to Add to Cargo.toml
```toml
[workspace.dependencies]
serde_yaml   = "0.9"
uuid         = { version = "1", features = ["v4", "serde"] }
chrono       = { version = "0.4", features = ["serde"] }
sha2         = "0.10"
hex          = "0.4"
regex        = "1"
once_cell    = "1"
validator    = { version = "0.18", features = ["derive"] }
strsim       = "0.11"
reqwest      = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

All crates except `reqwest` (Phase 2 ESCO API) are needed in Phase 1.

## Architecture

### Crate Placement

All LifeSheet code lives in `lazyjob-core`. This crate owns domain types and persistence. The TUI (`lazyjob-tui`) depends on `lazyjob-core` but never imports SQLite directly — it accesses the LifeSheet via the `LifeSheetRepository` trait. The CLI (`lazyjob-cli`) wires the trait to `SqliteLifeSheetRepository` and dispatches import/export subcommands.

### Core Types

```rust
// lazyjob-core/src/life_sheet/types.rs

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Deterministic ID type: SHA-256 of canonical fields, hex-encoded, first 32 chars.
/// Stable across re-imports if the content doesn't change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LifeSheetId(String);

impl LifeSheetId {
    pub fn from_fields(fields: &[&str]) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for f in fields {
            hasher.update(f.as_bytes());
            hasher.update(b"\x00");
        }
        let hex = hex::encode(hasher.finalize());
        Self(hex[..32].to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Top-level in-memory representation of a fully-parsed LifeSheet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifeSheet {
    pub meta: LifeSheetMeta,
    pub basics: PersonalInfo,
    pub experience: Vec<WorkExperience>,
    pub education: Vec<EducationEntry>,
    pub skills: Vec<SkillCategory>,
    pub certifications: Vec<Certification>,
    pub languages: Vec<Language>,
    pub projects: Vec<Project>,
    pub preferences: JobPreferences,
    pub goals: Option<CareerGoal>,
    pub profile_contacts: Vec<ProfileContact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifeSheetMeta {
    pub version: String,
    pub created_at: NaiveDate,
    pub updated_at: NaiveDate,
    /// SHA-256 of the canonical YAML bytes. Compared at import time to detect changes.
    pub version_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalInfo {
    pub id: LifeSheetId,   // derived: hash("personal_info")
    pub name: String,
    pub label: String,     // current/target title
    pub email: String,
    pub phone: Option<String>,
    pub url: Option<String>,
    pub location: Location,
    pub summary: Option<String>,
    pub profiles: Vec<SocialProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub remote_preference: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialProfile {
    pub network: String,
    pub username: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkExperience {
    /// Stable ID: hash(company_name + position + start_date)
    pub id: LifeSheetId,
    pub company_name: String,
    pub position: String,
    pub location: Option<String>,
    pub company_url: Option<String>,
    pub start_date: YearMonth,
    pub end_date: Option<YearMonth>,
    pub is_current: bool,
    pub summary: Option<String>,
    pub context: ExperienceContext,
    pub achievements: Vec<Achievement>,
    /// ESCO URIs for skills demonstrated in this role
    pub esco_codes: Vec<String>,
    /// O*NET codes for skills demonstrated in this role  
    pub onet_codes: Vec<String>,
    /// User-defined tags for filtering ("backend", "startup", "management")
    pub relevance_tags: Vec<String>,
}

/// "YYYY-MM" year-month representation stored as TEXT in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct YearMonth(String);

impl YearMonth {
    pub fn parse(s: &str) -> Result<Self, LifeSheetError> {
        // Validate "YYYY-MM" format
        let re = once_cell::sync::Lazy::force(&YEAR_MONTH_RE);
        if re.is_match(s) {
            Ok(Self(s.to_string()))
        } else {
            Err(LifeSheetError::InvalidDate(s.to_string()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceContext {
    pub team_size: Option<i32>,
    pub org_size: Option<i32>,
    pub industry: Option<String>,
    pub tech_stack: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Achievement {
    /// Stable ID: hash(experience_id + description[..64])
    pub id: LifeSheetId,
    pub experience_id: LifeSheetId,
    pub description: String,
    pub metric_type: Option<String>,   // "improvement_percent", "timeframe_months", etc.
    pub metric_value: Option<f64>,
    pub metric_unit: Option<String>,
    pub evidence: Option<String>,      // URL or citation
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationEntry {
    /// Stable ID: hash(institution + degree + field + start_date)
    pub id: LifeSheetId,
    pub institution: String,
    pub degree: String,
    pub field: Option<String>,
    pub start_date: YearMonth,
    pub end_date: Option<YearMonth>,
    pub score: Option<String>,
    pub thesis: Option<String>,
    pub courses: Vec<String>,
    pub relevance_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCategory {
    /// Stable ID: hash("skill_category" + name)
    pub id: LifeSheetId,
    pub name: String,
    pub level: Option<String>,    // "Advanced", "Intermediate", etc.
    pub keywords: Vec<Skill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Stable ID: hash(category_id + name)
    pub id: LifeSheetId,
    pub category_id: LifeSheetId,
    pub name: String,
    pub years_experience: Option<i32>,
    pub proficiency: SkillProficiency,
    pub esco_code: Option<String>,
    pub onet_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillProficiency {
    Expert,
    Advanced,
    Intermediate,
    Beginner,
}

impl SkillProficiency {
    pub fn to_db_str(self) -> &'static str {
        match self {
            Self::Expert       => "expert",
            Self::Advanced     => "advanced",
            Self::Intermediate => "intermediate",
            Self::Beginner     => "beginner",
        }
    }
    pub fn from_db_str(s: &str) -> Result<Self, LifeSheetError> {
        match s {
            "expert"       => Ok(Self::Expert),
            "advanced"     => Ok(Self::Advanced),
            "intermediate" => Ok(Self::Intermediate),
            "beginner"     => Ok(Self::Beginner),
            other          => Err(LifeSheetError::InvalidProficiency(other.to_string())),
        }
    }
    /// Numeric weight used by gap analysis scoring (0..=4).
    pub fn weight(self) -> u8 {
        match self {
            Self::Expert       => 4,
            Self::Advanced     => 3,
            Self::Intermediate => 2,
            Self::Beginner     => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certification {
    /// Stable ID: hash(name + authority + date)
    pub id: LifeSheetId,
    pub name: String,
    pub authority: String,
    pub date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub credential_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    /// Stable ID: hash("language" + name)
    pub id: LifeSheetId,
    pub name: String,
    pub proficiency: String,   // "Native", "Fluent", "Conversational", "Basic"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Stable ID: hash("project" + name + start_date.unwrap_or(""))
    pub id: LifeSheetId,
    pub name: String,
    pub description: Option<String>,
    pub url: Option<String>,
    pub start_date: Option<YearMonth>,
    pub end_date: Option<YearMonth>,
    pub highlights: Vec<String>,
    /// Skill IDs referenced by this project
    pub skill_ids: Vec<LifeSheetId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPreferences {
    pub job_types: Vec<String>,          // "full-time", "contract", etc.
    pub locations: Vec<PreferredLocation>,
    pub industries: Vec<String>,
    pub salary_min_cents: Option<i64>,
    pub salary_max_cents: Option<i64>,
    pub salary_currency: Option<String>,
    pub salary_is_total_comp: bool,      // false = base only
    pub notice_period_weeks: Option<i32>,
    pub visa_sponsorship_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredLocation {
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub remote_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareerGoal {
    pub short_term: Option<String>,
    pub long_term: Option<String>,
    pub timeline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileContact {
    /// Stable ID: hash("profile_contact" + name + company.unwrap_or(""))
    pub id: LifeSheetId,
    pub name: String,
    pub relationship: Option<String>,   // "manager", "mentor", "peer"
    pub company: Option<String>,
    pub email: Option<String>,
    pub linkedin_url: Option<String>,
    pub notes: Option<String>,
}

/// Flat skill representation returned by `get_skills_flat` for gap analysis.
#[derive(Debug, Clone)]
pub struct FlatSkill {
    pub id: LifeSheetId,
    pub name: String,
    pub category: String,
    pub proficiency: SkillProficiency,
    pub years_experience: Option<i32>,
    pub esco_code: Option<String>,
    pub onet_code: Option<String>,
}

/// JSON Resume schema (https://jsonresume.org/schema/) output type.
#[derive(Debug, Serialize)]
pub struct JsonResume {
    pub basics: JsonResumeBasics,
    pub work: Vec<JsonResumeWork>,
    pub education: Vec<JsonResumeEducation>,
    pub skills: Vec<JsonResumeSkill>,
    pub certificates: Vec<JsonResumeCert>,
    pub languages: Vec<JsonResumeLang>,
    pub projects: Vec<JsonResumeProject>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/life_sheet/repository.rs

use async_trait::async_trait;
use crate::life_sheet::types::*;
use crate::life_sheet::error::LifeSheetError;

pub type Result<T> = std::result::Result<T, LifeSheetError>;

#[async_trait]
pub trait LifeSheetRepository: Send + Sync {
    /// Load the full LifeSheet from the SQLite mirror.
    async fn get(&self) -> Result<LifeSheet>;

    /// Truncate all life sheet tables and re-import from a parsed YAML struct.
    /// Uses a single SQLite transaction — either all rows are inserted or none.
    async fn import(&self, parsed: &LifeSheetYaml) -> Result<ImportReport>;

    /// Export the SQLite mirror as a JSON Resume v1.0.0 document.
    async fn export_json_resume(&self) -> Result<JsonResume>;

    /// Flat list of all skills — used by gap analysis and semantic matching.
    async fn get_skills_flat(&self) -> Result<Vec<FlatSkill>>;

    /// Work experience entries structured for resume tailoring context.
    async fn get_experience_for_tailoring(&self) -> Result<Vec<WorkExperience>>;

    /// Return the stored `version_hash` so callers can detect staleness.
    async fn get_version_hash(&self) -> Result<Option<String>>;

    /// Check whether the YAML file is newer than the last import.
    async fn needs_reimport(&self, yaml_path: &std::path::Path) -> Result<bool>;
}

/// Result of an import operation — returned to the CLI for user feedback.
#[derive(Debug)]
pub struct ImportReport {
    pub experience_count: usize,
    pub skill_count: usize,
    pub education_count: usize,
    pub project_count: usize,
    pub contact_count: usize,
    pub version_hash: String,
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/001_life_sheet.sql

-- ─────────────────────────────────────────────────
-- Metadata table (single row)
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS life_sheet_meta (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version   TEXT    NOT NULL DEFAULT '1.0',
    version_hash     TEXT,                        -- SHA-256 of last imported YAML
    yaml_updated_at  TEXT,                        -- mtime of YAML file at import time
    imported_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- ─────────────────────────────────────────────────
-- Personal info (single row)
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS personal_info (
    id               TEXT PRIMARY KEY,            -- hash("personal_info")
    name             TEXT NOT NULL,
    label            TEXT NOT NULL,
    email            TEXT NOT NULL,
    phone            TEXT,
    url              TEXT,
    city             TEXT,
    region           TEXT,
    country          TEXT,
    remote_preference INTEGER NOT NULL DEFAULT 0, -- BOOL
    summary          TEXT,
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS social_profile (
    id               TEXT PRIMARY KEY,            -- hash(personal_id + network + username)
    personal_id      TEXT NOT NULL REFERENCES personal_info(id) ON DELETE CASCADE,
    network          TEXT NOT NULL,
    username         TEXT NOT NULL,
    url              TEXT NOT NULL
);

-- ─────────────────────────────────────────────────
-- Work experience
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS work_experience (
    id               TEXT PRIMARY KEY,            -- hash(company_name + position + start_date)
    company_name     TEXT NOT NULL,
    position         TEXT NOT NULL,
    location         TEXT,
    company_url      TEXT,
    start_date       TEXT NOT NULL,               -- "YYYY-MM"
    end_date         TEXT,                        -- "YYYY-MM" or NULL if current
    is_current       INTEGER NOT NULL DEFAULT 0,
    summary          TEXT,
    team_size        INTEGER,
    org_size         INTEGER,
    industry         TEXT,
    tech_stack       TEXT NOT NULL DEFAULT '[]',  -- JSON array
    esco_codes       TEXT NOT NULL DEFAULT '[]',  -- JSON array of URIs
    onet_codes       TEXT NOT NULL DEFAULT '[]',  -- JSON array
    relevance_tags   TEXT NOT NULL DEFAULT '[]',  -- JSON array
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_work_experience_current
    ON work_experience(is_current) WHERE is_current = 1;

CREATE INDEX IF NOT EXISTS idx_work_experience_start_date
    ON work_experience(start_date);

CREATE TABLE IF NOT EXISTS achievement (
    id               TEXT PRIMARY KEY,            -- hash(experience_id + description[:64])
    experience_id    TEXT NOT NULL REFERENCES work_experience(id) ON DELETE CASCADE,
    description      TEXT NOT NULL,
    metric_type      TEXT,
    metric_value     REAL,
    metric_unit      TEXT,
    evidence         TEXT
);

CREATE INDEX IF NOT EXISTS idx_achievement_experience
    ON achievement(experience_id);

-- ─────────────────────────────────────────────────
-- Education
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS education (
    id               TEXT PRIMARY KEY,            -- hash(institution + degree + field + start_date)
    institution      TEXT NOT NULL,
    degree           TEXT NOT NULL,
    field            TEXT,
    start_date       TEXT NOT NULL,               -- "YYYY-MM"
    end_date         TEXT,
    score            TEXT,
    thesis           TEXT,
    courses          TEXT NOT NULL DEFAULT '[]',  -- JSON array
    relevance_tags   TEXT NOT NULL DEFAULT '[]',  -- JSON array
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ─────────────────────────────────────────────────
-- Skills
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS skill_category (
    id               TEXT PRIMARY KEY,            -- hash("skill_category" + name)
    name             TEXT NOT NULL,
    level            TEXT,
    sort_order       INTEGER NOT NULL DEFAULT 0,
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS skill (
    id               TEXT PRIMARY KEY,            -- hash(category_id + name)
    category_id      TEXT NOT NULL REFERENCES skill_category(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    years_experience INTEGER,
    proficiency      TEXT NOT NULL DEFAULT 'intermediate',  -- expert/advanced/intermediate/beginner
    esco_code        TEXT,                        -- ESCO URI e.g. "http://data.europa.eu/esco/skill/abc"
    onet_code        TEXT                         -- O*NET code e.g. "2.A.1.a"
);

CREATE INDEX IF NOT EXISTS idx_skill_esco
    ON skill(esco_code) WHERE esco_code IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_skill_onet
    ON skill(onet_code) WHERE onet_code IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_skill_category
    ON skill(category_id);

-- ─────────────────────────────────────────────────
-- Certifications
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS certification (
    id               TEXT PRIMARY KEY,            -- hash(name + authority + date)
    name             TEXT NOT NULL,
    authority        TEXT NOT NULL,
    date             TEXT,                        -- "YYYY-MM-DD"
    expiry_date      TEXT,
    credential_id    TEXT,
    url              TEXT
);

-- ─────────────────────────────────────────────────
-- Languages
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS language (
    id               TEXT PRIMARY KEY,            -- hash("language" + name)
    name             TEXT NOT NULL,
    proficiency      TEXT NOT NULL               -- "Native", "Fluent", etc.
);

-- ─────────────────────────────────────────────────
-- Projects
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS project (
    id               TEXT PRIMARY KEY,            -- hash("project" + name + start_date)
    name             TEXT NOT NULL,
    description      TEXT,
    url              TEXT,
    start_date       TEXT,
    end_date         TEXT,
    highlights       TEXT NOT NULL DEFAULT '[]'   -- JSON array
);

CREATE TABLE IF NOT EXISTS project_skill (
    project_id       TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    skill_id         TEXT NOT NULL REFERENCES skill(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, skill_id)
);

-- ─────────────────────────────────────────────────
-- Job preferences (single row)
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS job_preferences (
    id                          INTEGER PRIMARY KEY CHECK (id = 1),
    job_types                   TEXT NOT NULL DEFAULT '[]',       -- JSON array
    preferred_locations         TEXT NOT NULL DEFAULT '[]',       -- JSON array of objects
    industries                  TEXT NOT NULL DEFAULT '[]',       -- JSON array
    salary_min_cents            INTEGER,
    salary_max_cents            INTEGER,
    salary_currency             TEXT DEFAULT 'USD',
    salary_is_total_comp        INTEGER NOT NULL DEFAULT 0,
    notice_period_weeks         INTEGER,
    visa_sponsorship_required   INTEGER NOT NULL DEFAULT 0,
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ─────────────────────────────────────────────────
-- Career goals (single row)
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS career_goal (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    short_term       TEXT,
    long_term        TEXT,
    timeline         TEXT,
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ─────────────────────────────────────────────────
-- Profile contacts (relationship network)
-- NOTE: distinct from application_contacts (spec 04)
-- ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS profile_contacts (
    id               TEXT PRIMARY KEY,            -- hash("profile_contact" + name + company)
    name             TEXT NOT NULL,
    relationship     TEXT,
    company          TEXT,
    email            TEXT,
    linkedin_url     TEXT,
    notes            TEXT,
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Module Structure

```
lazyjob-core/
  src/
    life_sheet/
      mod.rs           -- re-exports public surface
      types.rs         -- all domain types (LifeSheet, WorkExperience, Skill, …)
      yaml.rs          -- serde_yaml structs for parsing; converts to canonical types
      repository.rs    -- LifeSheetRepository trait + ImportReport
      sqlite.rs        -- SqliteLifeSheetRepository: trait impl
      import.rs        -- import_life_sheet: YAML → SQLite truncate/re-insert
      export.rs        -- export_json_resume: SQLite rows → JsonResume
      fabrication.rs   -- is_grounded_claim predicate
      validation.rs    -- validate_life_sheet: structural checks before import
      error.rs         -- LifeSheetError enum (thiserror)
  migrations/
    001_life_sheet.sql
```

## Implementation Phases

### Phase 1 — Core Data Layer (MVP)

#### Step 1.1 — Error Type

**File:** `lazyjob-core/src/life_sheet/error.rs`

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LifeSheetError {
    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("invalid date format (expected YYYY-MM): {0}")]
    InvalidDate(String),

    #[error("invalid proficiency level: {0}")]
    InvalidProficiency(String),

    #[error("life sheet not found — run `lazyjob profile import`")]
    NotFound,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, LifeSheetError>;
```

**Verification:** `cargo check` passes; error variants cover all import/export paths.

---

#### Step 1.2 — YAML Parsing Structs

**File:** `lazyjob-core/src/life_sheet/yaml.rs`

Define a parallel set of structs tagged `#[derive(Deserialize)]` that mirror the YAML schema exactly. These are intentionally distinct from the domain types in `types.rs` because the YAML schema uses snake_case field names from the user-facing spec that may differ from internal representation.

Key design decisions:
- All date fields deserialized as `String` and validated later in `validation.rs`
- `salary` block deserialized into raw `f64` (then converted to cents × 100 as `i64` on import)
- Optional fields use `Option<T>` — `serde(default)` on `Vec` fields so missing = empty vec

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LifeSheetYaml {
    pub meta:             MetaYaml,
    pub basics:           BasicsYaml,
    #[serde(default)]
    pub experience:       Vec<WorkExperienceYaml>,
    #[serde(default)]
    pub education:        Vec<EducationYaml>,
    #[serde(default)]
    pub skills:           Vec<SkillCategoryYaml>,
    #[serde(default)]
    pub certifications:   Vec<CertificationYaml>,
    #[serde(default)]
    pub languages:        Vec<LanguageYaml>,
    #[serde(default)]
    pub projects:         Vec<ProjectYaml>,
    pub preferences:      Option<PreferencesYaml>,
    pub goals:            Option<GoalsYaml>,
    #[serde(default)]
    pub profile_contacts: Vec<ProfileContactYaml>,
}

// … (all nested structs following same pattern)

impl LifeSheetYaml {
    /// Parse from YAML bytes. Returns `LifeSheetError::YamlParse` on failure.
    pub fn from_bytes(bytes: &[u8]) -> crate::life_sheet::Result<Self> {
        serde_yaml::from_slice(bytes).map_err(LifeSheetError::YamlParse)
    }

    /// Read from the canonical path `~/.lazyjob/life-sheet.yaml`.
    pub async fn from_default_path() -> crate::life_sheet::Result<(Self, std::path::PathBuf)> {
        let path = crate::config::life_sheet_path()?;
        let bytes = tokio::fs::read(&path).await?;
        Ok((Self::from_bytes(&bytes)?, path))
    }
}
```

**Verification:** `cargo test yaml::tests::parse_minimal_yaml` passes against a 20-line minimal YAML fixture.

---

#### Step 1.3 — Validation

**File:** `lazyjob-core/src/life_sheet/validation.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;
use crate::life_sheet::yaml::LifeSheetYaml;
use crate::life_sheet::error::{LifeSheetError, Result};

static YEAR_MONTH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\d{4}-(0[1-9]|1[0-2])$").unwrap()
});

static EMAIL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[^@\s]+@[^@\s]+\.[^@\s]+$").unwrap()
});

/// Validate a parsed LifeSheetYaml before importing to SQLite.
/// Returns `Err(LifeSheetError::Validation)` on the first structural problem.
pub fn validate_life_sheet(yaml: &LifeSheetYaml) -> Result<()> {
    if yaml.basics.name.trim().is_empty() {
        return Err(LifeSheetError::Validation("basics.name is required".into()));
    }
    if !EMAIL_RE.is_match(&yaml.basics.email) {
        return Err(LifeSheetError::Validation(
            format!("basics.email '{}' is not a valid email", yaml.basics.email)
        ));
    }
    for exp in &yaml.experience {
        YEAR_MONTH_RE.is_match(&exp.start_date)
            .then_some(())
            .ok_or_else(|| LifeSheetError::InvalidDate(exp.start_date.clone()))?;
        if let Some(end) = &exp.end_date {
            YEAR_MONTH_RE.is_match(end)
                .then_some(())
                .ok_or_else(|| LifeSheetError::InvalidDate(end.clone()))?;
            if end < &exp.start_date {
                return Err(LifeSheetError::Validation(
                    format!("experience '{}': end_date {} is before start_date {}",
                        exp.company, end, exp.start_date)
                ));
            }
        }
    }
    // … similar checks for education dates, salary min <= max, etc.
    Ok(())
}
```

**Verification:** Unit tests cover empty name, invalid email, end < start, invalid YYYY-MM strings.

---

#### Step 1.4 — SQLite Migration

**File:** `lazyjob-core/migrations/001_life_sheet.sql`

Full DDL from the SQLite Schema section above. Applied by `sqlx migrate run` or `sqlx::migrate!("migrations").run(&pool)` on startup.

**Key index decisions:**
- `idx_skill_esco` and `idx_skill_onet` are partial indexes (`WHERE NOT NULL`) — zero cost when codes are absent
- `idx_work_experience_current` enables fast "current job" lookups without scanning all experience rows
- `idx_work_experience_start_date` enables chronological sorting

**Verification:** `sqlx migrate run --database-url sqlite://test.db` succeeds; `sqlite3 test.db .schema` shows all tables.

---

#### Step 1.5 — Import Logic

**File:** `lazyjob-core/src/life_sheet/import.rs`

```rust
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;
use crate::life_sheet::{types::*, yaml::LifeSheetYaml, error::Result};

/// Full truncate-and-reimport. Runs inside a single SQLite transaction.
/// Uses deterministic IDs so downstream application-linked rows remain valid
/// across re-imports where only non-identity fields change.
pub async fn import_life_sheet(
    conn: &mut SqliteConnection,
    yaml: &LifeSheetYaml,
    yaml_bytes: &[u8],
) -> Result<ImportReport> {
    let version_hash = {
        let mut h = Sha256::new();
        h.update(yaml_bytes);
        hex::encode(h.finalize())
    };

    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

    // Truncate all life sheet tables in dependency order
    for table in &[
        "project_skill", "project", "profile_contacts", "career_goal",
        "job_preferences", "language", "certification", "skill",
        "skill_category", "achievement", "work_experience", "education",
        "social_profile", "personal_info", "life_sheet_meta",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *conn)
            .await?;
    }

    // Insert personal_info
    let personal_id = LifeSheetId::from_fields(&["personal_info"]);
    insert_personal_info(conn, &personal_id, &yaml.basics).await?;

    // Insert experience + achievements
    let mut skill_count = 0usize;
    let mut experience_count = 0usize;
    for exp_yaml in &yaml.experience {
        let exp = map_experience(exp_yaml);
        insert_work_experience(conn, &exp).await?;
        for ach in &exp.achievements {
            insert_achievement(conn, ach).await?;
        }
        experience_count += 1;
    }

    // Insert skills
    for (sort_order, cat_yaml) in yaml.skills.iter().enumerate() {
        let cat = map_skill_category(cat_yaml, sort_order);
        insert_skill_category(conn, &cat).await?;
        for skill_yaml in &cat_yaml.keywords {
            let skill = map_skill(skill_yaml, &cat.id);
            insert_skill(conn, &skill).await?;
            skill_count += 1;
        }
    }

    // … insert education, certifications, languages, projects, preferences, goals, contacts

    // Update meta row
    sqlx::query(
        "INSERT OR REPLACE INTO life_sheet_meta (id, version_hash, imported_at)
         VALUES (1, ?, datetime('now'))"
    )
    .bind(&version_hash)
    .execute(&mut *conn)
    .await?;

    sqlx::query("COMMIT").execute(&mut *conn).await?;

    Ok(ImportReport {
        experience_count,
        skill_count,
        // …
        version_hash,
    })
}
```

Key crate APIs used:
- `sha2::Sha256::new()` + `h.update(bytes)` + `hex::encode(h.finalize())` — version hash
- `sqlx::query("BEGIN IMMEDIATE").execute(conn)` — write transaction
- `sqlx::query!()` macros for compile-time checked inserts (Phase 2 polish — Phase 1 can use `sqlx::query()` to avoid the offline DB requirement)

**Verification:** Integration test: parse fixture YAML → import → re-import → assert row counts are identical; assert all IDs match between the two imports.

---

#### Step 1.6 — Export Logic

**File:** `lazyjob-core/src/life_sheet/export.rs`

Map SQLite rows to the JSON Resume v1.0.0 schema. Used by resume tailoring and the `lazyjob profile export-json-resume` CLI command.

```rust
pub async fn export_json_resume(conn: &mut SqliteConnection) -> Result<JsonResume> {
    let basics_row = sqlx::query_as!(
        PersonalInfoRow,
        "SELECT * FROM personal_info LIMIT 1"
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(LifeSheetError::NotFound)?;

    let work_rows = sqlx::query_as!(
        WorkExperienceRow,
        "SELECT * FROM work_experience ORDER BY start_date DESC"
    )
    .fetch_all(&mut *conn)
    .await?;

    // Build JSON Resume work entries, expanding tech_stack JSON array
    let work = work_rows.into_iter().map(|row| {
        let highlights: Vec<String> = sqlx::block_in_place(|| {
            // achievements fetched separately and merged here in Phase 1
            // Phase 2: use a JOIN
            vec![]
        });
        JsonResumeWork {
            name: row.company_name,
            position: row.position,
            url: row.company_url,
            start_date: row.start_date,
            end_date: row.end_date,
            summary: row.summary,
            highlights,
        }
    }).collect();

    // … map education, skills, certifications, languages, projects

    Ok(JsonResume { basics: map_basics(basics_row), work, /* … */ })
}
```

**Verification:** `lazyjob profile export-json-resume > resume.json` produces valid JSON Resume v1.0.0 output validated by `jsonresume/resume-schema` validator.

---

#### Step 1.7 — SqliteLifeSheetRepository

**File:** `lazyjob-core/src/life_sheet/sqlite.rs`

```rust
use std::sync::Arc;
use sqlx::SqlitePool;
use async_trait::async_trait;
use crate::life_sheet::{repository::*, types::*, import, export, error::Result};

pub struct SqliteLifeSheetRepository {
    pool: Arc<SqlitePool>,
}

impl SqliteLifeSheetRepository {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LifeSheetRepository for SqliteLifeSheetRepository {
    async fn get(&self) -> Result<LifeSheet> {
        let mut conn = self.pool.acquire().await?;
        load_life_sheet(&mut conn).await
    }

    async fn import(&self, yaml: &LifeSheetYaml) -> Result<ImportReport> {
        // Serialize back to bytes for hash
        let bytes = serde_yaml::to_string(yaml)?.into_bytes();
        let mut conn = self.pool.acquire().await?;
        import::import_life_sheet(&mut conn, yaml, &bytes).await
    }

    async fn export_json_resume(&self) -> Result<JsonResume> {
        let mut conn = self.pool.acquire().await?;
        export::export_json_resume(&mut conn).await
    }

    async fn get_skills_flat(&self) -> Result<Vec<FlatSkill>> {
        let mut conn = self.pool.acquire().await?;
        load_skills_flat(&mut conn).await
    }

    async fn get_experience_for_tailoring(&self) -> Result<Vec<WorkExperience>> {
        let mut conn = self.pool.acquire().await?;
        load_experience(&mut conn).await
    }

    async fn get_version_hash(&self) -> Result<Option<String>> {
        let hash = sqlx::query_scalar!(
            "SELECT version_hash FROM life_sheet_meta WHERE id = 1"
        )
        .fetch_optional(&*self.pool)
        .await?
        .flatten();
        Ok(hash)
    }

    async fn needs_reimport(&self, yaml_path: &std::path::Path) -> Result<bool> {
        let stored_hash = self.get_version_hash().await?;
        let Some(stored) = stored_hash else { return Ok(true) };
        let bytes = tokio::fs::read(yaml_path).await?;
        let current_hash = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(&bytes))
        };
        Ok(current_hash != stored)
    }
}
```

**Verification:** `cargo test` passes all repository tests using `#[sqlx::test(migrations = "migrations")]`.

---

#### Step 1.8 — Fabrication Check

**File:** `lazyjob-core/src/life_sheet/fabrication.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;
use crate::life_sheet::types::LifeSheet;

/// Stopwords to ignore when checking claim grounding.
static NOISE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(the|a|an|and|or|of|in|at|for|with|to|is|was|are|were)\b").unwrap()
});

/// Returns true if `claim` text is traceable to at least one LifeSheet entity.
/// Used by resume tailoring and cover letter gen to enforce anti-fabrication.
/// 
/// Matching strategy:
/// 1. Exact substring match against experience summaries and achievement descriptions
/// 2. Fuzzy match (jaro_winkler >= 0.88) against skill names if claim is short (< 30 chars)
/// 3. Keyword overlap >= 60% of significant words in the claim
pub fn is_grounded_claim(claim: &str, life_sheet: &LifeSheet) -> bool {
    let normalized = NOISE.replace_all(&claim.to_lowercase(), " ");
    let claim_words: Vec<&str> = normalized.split_whitespace().collect();
    if claim_words.is_empty() {
        return false;
    }

    // Check 1: substring match in experience summaries
    for exp in &life_sheet.experience {
        if let Some(summary) = &exp.summary {
            if summary.to_lowercase().contains(claim.to_lowercase().as_str()) {
                return true;
            }
        }
        for ach in &exp.achievements {
            if ach.description.to_lowercase().contains(claim.to_lowercase().as_str()) {
                return true;
            }
        }
    }

    // Check 2: fuzzy skill match for short claims
    if claim.len() < 30 {
        for cat in &life_sheet.skills {
            for skill in &cat.keywords {
                if strsim::jaro_winkler(&skill.name.to_lowercase(), &claim.to_lowercase()) >= 0.88 {
                    return true;
                }
            }
        }
    }

    // Check 3: keyword overlap
    let all_text = collect_all_text(life_sheet).to_lowercase();
    let all_words: std::collections::HashSet<&str> =
        all_text.split_whitespace().collect();
    let matches = claim_words.iter().filter(|w| all_words.contains(*w)).count();
    let overlap = matches as f64 / claim_words.len() as f64;
    overlap >= 0.60
}

fn collect_all_text(ls: &LifeSheet) -> String {
    let mut parts = Vec::new();
    for exp in &ls.experience {
        parts.push(exp.company_name.as_str());
        parts.push(exp.position.as_str());
        if let Some(s) = &exp.summary { parts.push(s.as_str()); }
        for ach in &exp.achievements { parts.push(ach.description.as_str()); }
        for tag in &exp.relevance_tags { parts.push(tag.as_str()); }
    }
    for cat in &ls.skills {
        for skill in &cat.keywords { parts.push(skill.name.as_str()); }
    }
    if let Some(g) = &ls.goals {
        if let Some(s) = &g.short_term { parts.push(s.as_str()); }
        if let Some(l) = &g.long_term  { parts.push(l.as_str()); }
    }
    parts.join(" ")
}
```

**Verification:** Unit tests: known claim "Led team of 5 engineers" traced to an experience with `team_size = 5`; fabricated claim "Managed $50M budget" returns false on a life sheet with no such entry.

---

#### Step 1.9 — CLI Wiring

**File:** `lazyjob-cli/src/commands/profile.rs`

```rust
use clap::{Args, Subcommand};
use lazyjob_core::life_sheet::{yaml::LifeSheetYaml, validation, repository::LifeSheetRepository};

#[derive(Debug, Args)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// Import (or re-import) life-sheet.yaml into the local SQLite database.
    Import {
        /// Path to YAML file. Defaults to ~/.lazyjob/life-sheet.yaml.
        #[arg(long)]
        path: Option<std::path::PathBuf>,
    },
    /// Check if a re-import is needed (YAML changed since last import).
    Status,
    /// Export the SQLite profile as a JSON Resume document.
    ExportJsonResume {
        #[arg(long, default_value = "resume.json")]
        output: std::path::PathBuf,
    },
}

pub async fn run(args: ProfileArgs, repo: &dyn LifeSheetRepository) -> anyhow::Result<()> {
    match args.command {
        ProfileCommand::Import { path } => {
            let yaml_path = path.unwrap_or_else(lazyjob_core::config::life_sheet_path);
            let bytes = tokio::fs::read(&yaml_path).await
                .context(format!("cannot read {}", yaml_path.display()))?;
            let yaml = LifeSheetYaml::from_bytes(&bytes)?;
            validation::validate_life_sheet(&yaml)?;
            let report = repo.import(&yaml).await?;
            println!("Imported: {} experience entries, {} skills, {} contacts",
                report.experience_count, report.skill_count, report.contact_count);
            println!("Version hash: {}", &report.version_hash[..8]);
            Ok(())
        }
        ProfileCommand::Status => {
            let yaml_path = lazyjob_core::config::life_sheet_path()?;
            let needs = repo.needs_reimport(&yaml_path).await?;
            if needs {
                println!("YAML has changed — run `lazyjob profile import` to sync");
            } else {
                println!("SQLite mirror is up to date");
            }
            Ok(())
        }
        ProfileCommand::ExportJsonResume { output } => {
            let json_resume = repo.export_json_resume().await?;
            let json = serde_json::to_string_pretty(&json_resume)?;
            tokio::fs::write(&output, json).await?;
            println!("Written to {}", output.display());
            Ok(())
        }
    }
}
```

**Verification:** `lazyjob profile import` prints row counts and exits 0; `lazyjob profile status` prints "up to date" on second run.

---

### Phase 2 — ESCO/O*NET Integration

#### Step 2.1 — ESCO Skill Code Auto-Suggestion

**File:** `lazyjob-core/src/life_sheet/esco.rs`

ESCO provides a free REST API: `https://ec.europa.eu/esco/api/search?text={skill}&type=skill&language=en`

```rust
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct EscoSearchResult {
    #[serde(rename = "_embedded")]
    embedded: EscoEmbedded,
}

#[derive(Debug, Deserialize)]
struct EscoEmbedded {
    results: Vec<EscoSkillHit>,
}

#[derive(Debug, Deserialize)]
pub struct EscoSkillHit {
    pub uri: String,
    pub title: String,
    pub score: f64,
}

pub struct EscoClient {
    http: Client,
    base_url: String,
}

impl EscoClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            base_url: "https://ec.europa.eu/esco/api".into(),
        }
    }

    /// Test override — inject a mock base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: Client::new(), base_url: base_url.into() }
    }

    /// Returns the top hit if score >= 0.80.
    pub async fn suggest_code(&self, skill_name: &str) -> anyhow::Result<Option<EscoSkillHit>> {
        let url = format!("{}/search?text={}&type=skill&language=en",
            self.base_url,
            urlencoding::encode(skill_name)
        );
        let resp: EscoSearchResult = self.http.get(&url).send().await?.json().await?;
        Ok(resp.embedded.results.into_iter().next().filter(|h| h.score >= 0.80))
    }
}
```

Background loop: `LoopType::EscoTagging` runs nightly via the Ralph scheduler. It fetches all skills with `esco_code IS NULL`, calls `EscoClient::suggest_code()`, and writes confirmed codes. Suggestions with `0.70 <= score < 0.80` are stored in a `pending_esco_suggestions` table for TUI review.

---

#### Step 2.2 — Incremental Diff Import

Instead of full truncate-reimport, compute a SHA-256 hash per entity (based on all fields except the timestamp) and compare against stored hashes. Insert/update only changed rows. This preserves foreign key references in downstream tables (`resume_versions`, `cover_letter_versions`) across re-imports.

Implementation: a `life_sheet_entity_hashes` table stores `(entity_id, content_hash)` pairs. On import, compute the current hash, compare, and skip the row if unchanged.

This avoids the risk of losing application-linked resume versions when a user corrects a typo in a skill name.

---

#### Step 2.3 — LinkedIn Export Parser

**File:** `lazyjob-core/src/life_sheet/import_linkedin.rs`

LinkedIn's data export (ZIP containing `Profile.csv`, `Positions.csv`, `Education.csv`, `Skills.csv`, `Connections.csv`) can be parsed and mapped to `LifeSheetYaml` without any API calls.

```rust
pub async fn parse_linkedin_export(zip_path: &std::path::Path) -> Result<LifeSheetYaml> {
    use tokio::task::spawn_blocking;
    spawn_blocking(move || {
        let file = std::fs::File::open(zip_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        // … extract CSVs, parse with csv::Reader, map to LifeSheetYaml
    }).await??
}
```

CLI: `lazyjob profile import-linkedin-export <path/to/LinkedIn_export.zip>`

The parser is additive: it generates a starter YAML that the user edits before running `lazyjob profile import`. It does not overwrite an existing life-sheet.yaml.

---

#### Step 2.4 — TUI Profile Editor

**File:** `lazyjob-tui/src/views/profile.rs`

A read-only profile browser with `?` inline help. Full editing is deferred to direct YAML editing + `lazyjob profile import`. The TUI shows:

- `ProfileOverviewPane`: name, label, email, import status (hash, last imported, whether YAML is newer)
- `SkillsBrowserPane`: skills by category with proficiency badges, ESCO code display
- `ExperienceBrowserPane`: chronological timeline, expanding to show achievements
- `ContactsBrowserPane`: alphabetical list, with relationship and company

Import-status display: if `needs_reimport()` returns `true`, a yellow banner shows "YAML modified — press `I` to re-import".

Keybindings (Normal mode):
| Key    | Action                            |
|--------|-----------------------------------|
| `Tab`  | Cycle through panes               |
| `j/k`  | Navigate list items               |
| `Enter`| Expand/collapse experience entry  |
| `I`    | Trigger import from YAML          |
| `E`    | Open YAML in `$EDITOR`            |
| `?`    | Toggle help overlay               |

Phase 2 inline editing (long-term): use a `TextArea` widget (via `tui-textarea` crate) for editing individual fields directly in the TUI, writing changes back to YAML on save.

---

### Phase 3 — Version History & Multi-Variant Support

#### Step 3.1 — YAML Version History

Store timestamped snapshots of the YAML file in `~/.lazyjob/life-sheet-history/YYYYMMDD-HHMMSS.yaml`. Limit to the last 10 snapshots. On every successful import, rotate the archive.

```rust
pub async fn archive_yaml(yaml_path: &std::path::Path) -> anyhow::Result<()> {
    let history_dir = yaml_path.parent().unwrap().join("life-sheet-history");
    tokio::fs::create_dir_all(&history_dir).await?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let dest = history_dir.join(format!("{ts}.yaml"));
    tokio::fs::copy(yaml_path, &dest).await?;
    // Prune to 10 most recent
    let mut entries: Vec<_> = tokio::fs::read_dir(&history_dir)
        .await?
        .collect::<Vec<_>>()
        .await;
    entries.sort_by_key(|e| e.as_ref().unwrap().file_name());
    for old in entries.iter().rev().skip(10) {
        tokio::fs::remove_file(old.as_ref().unwrap().path()).await?;
    }
    Ok(())
}
```

#### Step 3.2 — Relevance-Tag Filtering

`LifeSheetRepository::get_experience_filtered(tags: &[&str])` returns only experience entries where `relevance_tags` contains at least one of the given tags. Uses a JSON array containment check:

```sql
SELECT * FROM work_experience
WHERE EXISTS (
    SELECT 1 FROM json_each(relevance_tags)
    WHERE json_each.value IN (/* bind tag list */)
)
```

This supports career changers filtering their profile to "engineering" vs "management" experience without maintaining separate LifeSheets.

## Key Crate APIs

| API | Usage |
|-----|-------|
| `serde_yaml::from_slice::<LifeSheetYaml>(bytes)` | Parse YAML bytes to typed struct |
| `sha2::Sha256::digest(bytes)` → `hex::encode(hash)` | Version hash for change detection |
| `LifeSheetId::from_fields(&["company", "position", "2022-03"])` | Deterministic entity IDs |
| `sqlx::query("BEGIN IMMEDIATE").execute(conn)` | Exclusive write transaction |
| `sqlx::query_as!()` | Compile-time checked SELECT → struct |
| `once_cell::sync::Lazy<Regex>` | Compile-once regex patterns |
| `strsim::jaro_winkler(a, b) >= 0.88` | Fuzzy skill name matching in fabrication check |
| `reqwest::Client::get(url).send().await?.json::<T>()` | ESCO REST API (Phase 2) |
| `tokio::fs::read(path)` | Async file read for YAML |
| `serde_json::to_string_pretty(&json_resume)` | JSON Resume export |
| `chrono::Utc::now().format("%Y%m%d-%H%M%S")` | Version archive timestamps |
| `zip::ZipArchive::new(file)` | LinkedIn export ZIP parser (Phase 2) |

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum LifeSheetError {
    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("invalid date format (expected YYYY-MM): {0}")]
    InvalidDate(String),

    #[error("invalid proficiency level: {0}")]
    InvalidProficiency(String),

    #[error("life sheet not found — run `lazyjob profile import` to update")]
    NotFound,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ESCO API error: {0}")]
    EscoApi(#[from] reqwest::Error),   // Phase 2 only
}
```

Import errors are non-recoverable (the user must fix their YAML). Export and `get` errors propagate to the caller via `anyhow` context chains in the CLI. The TUI catches `NotFound` and renders an onboarding prompt: "No profile found — run `lazyjob profile import`".

## Testing Strategy

### Unit Tests

**`validation.rs`:**
- Empty `basics.name` → `Err(Validation)` 
- Invalid email format → `Err(Validation)`
- `end_date < start_date` → `Err(Validation)`
- Invalid `"202X-13"` date → `Err(InvalidDate)`
- Valid minimal YAML → `Ok(())`

**`fabrication.rs`:**
- Known phrase from experience summary → `true`
- Fabricated accomplishment with no YAML basis → `false`
- Short skill name with fuzzy match (typo) → `true`
- Borderline 60% keyword overlap → exact threshold test

**`types.rs`:**
- `LifeSheetId::from_fields` determinism — same input → same ID, different input → different ID
- `SkillProficiency::from_db_str` round-trip — all 4 variants

### Integration Tests

Use `#[sqlx::test(migrations = "migrations")]` which auto-creates an in-memory SQLite and applies all migrations.

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_import_roundtrip(pool: sqlx::SqlitePool) {
    let repo = SqliteLifeSheetRepository::new(Arc::new(pool));
    let yaml_bytes = include_bytes!("../fixtures/life_sheet_full.yaml");
    let yaml = LifeSheetYaml::from_bytes(yaml_bytes).unwrap();
    let report1 = repo.import(&yaml).await.unwrap();

    // Re-import should produce identical IDs and counts
    let report2 = repo.import(&yaml).await.unwrap();
    assert_eq!(report1.experience_count, report2.experience_count);
    assert_eq!(report1.version_hash, report2.version_hash);

    // Fetch should return all data
    let sheet = repo.get().await.unwrap();
    assert!(!sheet.experience.is_empty());
    assert!(!sheet.skills.is_empty());
}

#[sqlx::test(migrations = "migrations")]
async fn test_import_then_needs_reimport_false(pool: sqlx::SqlitePool) {
    let repo = SqliteLifeSheetRepository::new(Arc::new(pool));
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let yaml_bytes = include_bytes!("../fixtures/life_sheet_minimal.yaml");
    tokio::fs::write(tmp.path(), yaml_bytes).await.unwrap();
    let yaml = LifeSheetYaml::from_bytes(yaml_bytes).unwrap();
    repo.import(&yaml).await.unwrap();
    assert!(!repo.needs_reimport(tmp.path()).await.unwrap());
}

#[sqlx::test(migrations = "migrations")]
async fn test_export_json_resume(pool: sqlx::SqlitePool) {
    // import → export → validate JSON Resume structure
    let repo = SqliteLifeSheetRepository::new(Arc::new(pool));
    let yaml = LifeSheetYaml::from_bytes(include_bytes!("../fixtures/life_sheet_full.yaml")).unwrap();
    repo.import(&yaml).await.unwrap();
    let json_resume = repo.export_json_resume().await.unwrap();
    assert!(!json_resume.basics.name.is_empty());
    assert!(!json_resume.work.is_empty());
}
```

### CLI Integration Tests

```rust
// lazyjob-cli/tests/profile_import_test.rs
#[tokio::test]
async fn test_profile_import_status_cycle() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let yaml_path = tmp_dir.path().join("life-sheet.yaml");
    tokio::fs::copy("fixtures/life_sheet_full.yaml", &yaml_path).await.unwrap();
    
    // First import: needs_reimport = true initially, false after
    // Second `profile status`: prints "up to date"
    // Modify YAML: status prints "YAML modified"
}
```

### Fixtures

Create `lazyjob-core/tests/fixtures/`:
- `life_sheet_minimal.yaml` — only `basics` and one `experience` entry
- `life_sheet_full.yaml` — all sections populated, used for roundtrip and export tests
- `life_sheet_invalid_date.yaml` — triggers `InvalidDate` error
- `life_sheet_missing_name.yaml` — triggers `Validation` error

## Open Questions

1. **Salary units in YAML**: The spec uses `salary: { min, max }` as bare numbers. Should these be interpreted as annual figures in the user's local currency? Proposal: add an explicit `currency` and `units: annual|hourly` field to the YAML schema and document it clearly, converting to cents on import.

2. **Profile contact dedup**: If a user re-imports with a contact whose name changed (married name), the deterministic ID changes and a duplicate row appears. Proposal: add an `email`-based upsert fallback — if `INSERT OR IGNORE` fails on ID, try `UPDATE WHERE email = ?`.

3. **Multi-language YAML**: ESCO supports 27 EU languages. Should skill names and summaries in the YAML be allowed in non-English languages? The ESCO auto-tagging query passes `language=en` today. Proposal: add an optional `language: "de"` field to the YAML meta and propagate it to ESCO queries.

4. **`export_json_resume` achievements**: JSON Resume's `work[].highlights` is a flat string array. Achievement metrics should be serialized as part of the string (e.g. "Reduced build time by 40% over 3 months") rather than machine-readable JSON. The mapping function should format this sentence automatically if `metric_type` and `metric_value` are both present.

5. **Git integration (Phase 3)**: Auto-commit the YAML file to a local git repo on every successful import. This gives users a clean history without any additional tooling. Requires `git2` crate.

## Related Specs

- [specs/04-sqlite-persistence.md](04-sqlite-persistence.md) — Database, SqlitePool, migration runner
- [specs/07-resume-tailoring-pipeline.md](07-resume-tailoring-pipeline.md) — Consumes `get_experience_for_tailoring()`
- [specs/08-cover-letter-generation.md](08-cover-letter-generation.md) — Consumes `get()` for personalization
- [specs/profile-skills-gap-analysis.md](profile-skills-gap-analysis.md) — Consumes `get_skills_flat()`
- [specs/agentic-prompt-templates.md](agentic-prompt-templates.md) — Defines `LifeSheetContext` for prompt injection
- [specs/16-privacy-security.md](16-privacy-security.md) — YAML file encryption at rest
