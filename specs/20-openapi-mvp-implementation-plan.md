# Implementation Plan: OpenAPI MVP — End-to-End Build

## Status
Draft

## Related Spec
`specs/20-openapi-mvp.md`

## Overview

This plan is the executable, engineer-facing counterpart to the product-level MVP spec. Where the spec defines *what* to build and *why*, this plan defines *how* to build it: exact Cargo workspace layout, concrete Rust types, SQL DDL, module file paths, specific crate API calls, and phase-by-phase verification criteria.

LazyJob MVP is a 12-week, single-engineer build of a lazygit-style terminal UI for job search management backed by autonomous AI agent loops (Ralph). The build is decomposed into 6 phases: workspace bootstrap, core data model + TUI shell, application tracking, LLM + Ralph subprocess integration, resume tailoring, and polish + release prep.

The guiding constraint throughout is **local-first correctness before features**: SQLite WAL-mode concurrency, repository pattern with compile-time SQL checking, newtype wrappers preventing primitive confusion, and error handling that surfaces actionable messages to the user.

## Prerequisites

### Specs That Inform This Plan
All prior specs are synthesized here. Key dependencies:
- `specs/01-architecture-implementation-plan.md` — crate layout and TUI patterns
- `specs/04-sqlite-persistence-implementation-plan.md` — repository pattern and SQLite conventions
- `specs/02-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait and implementations
- `specs/06-ralph-loop-integration-implementation-plan.md` — subprocess IPC protocol
- `specs/07-resume-tailoring-pipeline-implementation-plan.md` — resume pipeline stages
- `specs/09-tui-design-keybindings-implementation-plan.md` — ratatui widget hierarchy and event loop

### Crates Required (Full Workspace Dependency Set)

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    "lazyjob-core",
    "lazyjob-llm",
    "lazyjob-ralph",
    "lazyjob-tui",
    "lazyjob-cli",
]
resolver = "2"

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# Error handling
thiserror = "2"
anyhow = "1"

# Logging / tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono"] }

# TUI
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }

# LLM
async-openai = "0.28"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Security / secrets
keyring = "3"
secrecy = { version = "0.8", features = ["serde"] }
zeroize = { version = "1", features = ["derive"] }

# Utilities
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
strsim = "0.11"
dirs = "5"

# Document generation
docx-rs = "0.4"

# Subprocess
tokio-util = { version = "0.7", features = ["codec"] }
bytes = "1"
```

## Architecture

### Crate Placement

```
lazyjob-core/       — Domain models, SQLite repositories, discovery sources,
                      life sheet parsing, resume pipeline, shared error types
lazyjob-llm/        — LlmProvider trait + OpenAI/Ollama/Anthropic impls,
                      prompt rendering, embedding
lazyjob-ralph/      — RalphProcessManager, stdio JSON codec, loop entrypoints
lazyjob-tui/        — ratatui views, widgets, event loop, keybindings, theme
lazyjob-cli/        — Thin binary: parse args, boot TUI or Ralph subcommand
```

**Dependency direction** (no cycles):
```
lazyjob-cli
  └─ lazyjob-tui
       ├─ lazyjob-ralph
       │    ├─ lazyjob-llm
       │    │    └─ lazyjob-core
       │    └─ lazyjob-core
       └─ lazyjob-core
```

### Core Types

#### Identifiers (newtype wrappers — parse, don't validate)

```rust
// lazyjob-core/src/ids.rs
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct JobId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ApplicationId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CompanyId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ContactId(String);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
// Same pattern for ApplicationId, CompanyId, ContactId
```

#### Domain Models

```rust
// lazyjob-core/src/models/job.rs
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Job {
    pub id: JobId,
    pub title: String,
    pub company_name: String,
    pub company_id: Option<CompanyId>,
    pub location: Option<String>,
    pub remote: RemotePolicy,
    pub url: Option<String>,
    pub description: Option<String>,
    pub salary_min: Option<i64>,  // cents
    pub salary_max: Option<i64>,  // cents
    pub salary_currency: String,
    pub status: JobStatus,
    pub interest_level: InterestLevel,
    pub source: JobSource,
    pub source_id: Option<String>,  // platform-native ID for dedup
    pub discovered_at: DateTime<Utc>,
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum JobStatus {
    Discovered,
    Interested,
    Archived,
    Applied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum RemotePolicy {
    Remote,
    Hybrid,
    OnSite,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct InterestLevel(u8); // 1-5, validated on construction

impl InterestLevel {
    pub fn new(n: u8) -> Result<Self, CoreError> {
        if (1..=5).contains(&n) {
            Ok(Self(n))
        } else {
            Err(CoreError::InvalidInterestLevel(n))
        }
    }
    pub fn value(self) -> u8 { self.0 }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum JobSource {
    Greenhouse,
    Lever,
    Manual,
    Other(String),
}
```

```rust
// lazyjob-core/src/models/application.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Application {
    pub id: ApplicationId,
    pub job_id: JobId,
    pub stage: ApplicationStage,
    pub resume_version_id: Option<String>,
    pub cover_letter_version_id: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub last_contact_at: Option<DateTime<Utc>>,
    pub next_follow_up: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ApplicationStage {
    Interested,
    Applied,
    PhoneScreen,
    Technical,
    Onsite,
    Offer,
    Accepted,
    Rejected,
    Withdrawn,
}

impl ApplicationStage {
    pub fn valid_transitions(&self) -> &[ApplicationStage] {
        use ApplicationStage::*;
        match self {
            Interested    => &[Applied, Rejected, Withdrawn],
            Applied       => &[PhoneScreen, Rejected, Withdrawn],
            PhoneScreen   => &[Technical, Onsite, Offer, Rejected, Withdrawn],
            Technical     => &[Onsite, Offer, Rejected, Withdrawn],
            Onsite        => &[Offer, Rejected, Withdrawn],
            Offer         => &[Accepted, Rejected, Withdrawn],
            Accepted | Rejected | Withdrawn => &[],
        }
    }

    pub fn can_transition_to(&self, next: ApplicationStage) -> bool {
        self.valid_transitions().contains(&next)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, ApplicationStage::Accepted | ApplicationStage::Rejected | ApplicationStage::Withdrawn)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageTransition {
    pub id: String,
    pub application_id: ApplicationId,
    pub from_stage: ApplicationStage,
    pub to_stage: ApplicationStage,
    pub transitioned_at: DateTime<Utc>,
    pub reason: Option<String>,
}
```

```rust
// lazyjob-core/src/models/company.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Company {
    pub id: CompanyId,
    pub name: String,
    pub greenhouse_board_token: Option<String>,
    pub lever_company_id: Option<String>,
    pub website: Option<String>,
    pub linkedin_url: Option<String>,
    pub industry: Option<String>,
    pub size: Option<CompanySize>,
    pub research_summary: Option<String>,   // LLM-synthesized
    pub research_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum CompanySize {
    Seed,
    SeriesAB,
    Growth,
    Late,
    Public,
    Enterprise,
}
```

```rust
// lazyjob-core/src/models/contact.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Contact {
    pub id: ContactId,
    pub name: String,
    pub role: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin_url: Option<String>,
    pub company_id: Option<CompanyId>,
    pub relationship: RelationshipType,
    pub quality: ContactQuality,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum RelationshipType {
    Recruiter,
    HiringManager,
    Interviewer,
    Referral,
    Network,
    Friend,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ContactQuality(u8); // 1-5
```

### Life Sheet Types

```rust
// lazyjob-core/src/life_sheet/types.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LifeSheet {
    pub basics: Basics,
    pub experience: Vec<ExperienceEntry>,
    pub education: Vec<EducationEntry>,
    pub skills: Vec<SkillEntry>,
    pub certifications: Vec<Certification>,
    pub projects: Vec<Project>,
    pub preferences: JobPreferences,
    pub goals: Goals,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExperienceEntry {
    pub company: String,
    pub position: String,
    pub start_date: String,   // YYYY-MM
    pub end_date: Option<String>,
    pub summary: Option<String>,
    pub context: ExperienceContext,
    pub achievements: Vec<Achievement>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExperienceContext {
    pub team_size: Option<u32>,
    pub org_size: Option<u32>,
    pub industry: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Achievement {
    pub description: String,
    pub metrics: Option<AchievementMetrics>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AchievementMetrics {
    pub before: Option<String>,
    pub after: Option<String>,
    pub impact: Option<String>,
}
```

### SQLite Schema

#### Migration 001: Initial Schema

```sql
-- lazyjob-core/migrations/001_initial_schema.sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE companies (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    greenhouse_board_token TEXT,
    lever_company_id TEXT,
    website TEXT,
    linkedin_url TEXT,
    industry TEXT,
    size TEXT,
    research_summary TEXT,
    research_updated_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    company_name TEXT NOT NULL,
    company_id TEXT REFERENCES companies(id) ON DELETE SET NULL,
    location TEXT,
    remote TEXT NOT NULL DEFAULT 'unknown',
    url TEXT,
    description TEXT,
    salary_min INTEGER,
    salary_max INTEGER,
    salary_currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'discovered',
    interest_level INTEGER NOT NULL DEFAULT 3,
    source TEXT NOT NULL DEFAULT 'manual',
    source_id TEXT,
    discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX idx_jobs_source_dedup ON jobs(source, source_id)
    WHERE source_id IS NOT NULL;
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_company ON jobs(company_id);

CREATE TABLE applications (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    stage TEXT NOT NULL DEFAULT 'interested',
    resume_version_id TEXT,
    cover_letter_version_id TEXT,
    submitted_at TEXT,
    last_contact_at TEXT,
    next_follow_up TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_applications_job ON applications(job_id);
CREATE INDEX idx_applications_stage ON applications(stage);

CREATE TABLE application_transitions (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    from_stage TEXT NOT NULL,
    to_stage TEXT NOT NULL,
    transitioned_at TEXT NOT NULL DEFAULT (datetime('now')),
    reason TEXT
);

CREATE INDEX idx_transitions_application ON application_transitions(application_id);

CREATE TABLE contacts (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    role TEXT,
    email TEXT,
    phone TEXT,
    linkedin_url TEXT,
    company_id TEXT REFERENCES companies(id) ON DELETE SET NULL,
    relationship TEXT NOT NULL DEFAULT 'network',
    quality INTEGER NOT NULL DEFAULT 3,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_contacts_company ON contacts(company_id);

CREATE TABLE interviews (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    scheduled_at TEXT,
    duration_minutes INTEGER,
    location TEXT,
    meeting_url TEXT,
    interviewer_names TEXT,   -- JSON array
    status TEXT NOT NULL DEFAULT 'scheduled',
    feedback TEXT,
    rating INTEGER,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE offers (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    base_salary_cents INTEGER NOT NULL,
    bonus_cents INTEGER,
    equity_summary TEXT,
    start_date TEXT,
    expires_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE reminders (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    description TEXT,
    due_at TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0,
    application_id TEXT REFERENCES applications(id) ON DELETE SET NULL,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_reminders_due ON reminders(due_at) WHERE completed = 0;

CREATE TABLE activity_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    details TEXT,  -- JSON
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_activity_entity ON activity_log(entity_type, entity_id);
CREATE INDEX idx_activity_created ON activity_log(created_at DESC);

CREATE TABLE ralph_loops (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    loop_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending|running|done|failed|cancelled
    params TEXT,     -- JSON
    result TEXT,     -- JSON
    error TEXT,
    pid INTEGER,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_ralph_loops_status ON ralph_loops(status);
```

### Module Structure

```
lazyjob-core/
  src/
    lib.rs
    error.rs
    ids.rs
    models/
      mod.rs
      job.rs
      application.rs
      company.rs
      contact.rs
      interview.rs
      offer.rs
    life_sheet/
      mod.rs       # LifeSheetService: load, validate, sync_to_db
      types.rs     # LifeSheet, ExperienceEntry, SkillEntry, ...
      sync.rs      # YAML -> SQLite mirroring
    persistence/
      mod.rs       # Database struct, pool, migrations
      job_repo.rs
      application_repo.rs
      company_repo.rs
      contact_repo.rs
      activity_repo.rs
      ralph_repo.rs
    discovery/
      mod.rs       # DiscoveryService
      greenhouse.rs
      lever.rs
      dedup.rs     # source+source_id dedup logic
    resume/
      mod.rs       # ResumeTailor orchestrator
      jd_parser.rs
      gap_analysis.rs
      fabrication_guardrails.rs
      drafting.rs
      docx_generator.rs

lazyjob-llm/
  src/
    lib.rs
    error.rs
    provider.rs    # LlmProvider trait
    message.rs     # ChatMessage, ChatResponse, Embedding
    registry.rs    # ProviderRegistry
    providers/
      mod.rs
      openai.rs
      anthropic.rs
      ollama.rs

lazyjob-ralph/
  src/
    lib.rs
    error.rs
    process.rs     # RalphProcessManager
    protocol.rs    # RalphMessage enum, JSON codec
    loops/
      mod.rs
      job_discovery.rs
      company_research.rs
      resume_tailor.rs

lazyjob-tui/
  src/
    lib.rs
    app.rs         # App struct, run() event loop
    error.rs
    keymap.rs      # KeyContext, KeyCombo, Action, Keymap
    theme.rs       # Color constants, Style helpers
    views/
      mod.rs
      dashboard.rs
      jobs_list.rs
      job_detail.rs
      applications.rs  # kanban pipeline
      contacts.rs
      ralph_panel.rs
      settings.rs
      help.rs
    widgets/
      mod.rs
      job_card.rs
      application_card.rs
      stat_block.rs
      modal.rs
      confirm_dialog.rs
      input_field.rs
      progress_bar.rs

lazyjob-cli/
  src/
    main.rs        # clap parse + dispatch
```

---

## Implementation Phases

### Phase 1 — Foundation: Workspace + Core Data + TUI Shell (Weeks 1-3)

#### Step 1.1: Cargo Workspace Bootstrap

**File**: `Cargo.toml` (workspace root)

- Create workspace manifest with all 5 members
- Set `resolver = "2"`
- Define `[workspace.dependencies]` with all shared crates pinned
- Each member `Cargo.toml` references workspace deps with `workspace = true`

**Verification**: `cargo build --workspace` compiles successfully.

#### Step 1.2: Tracing Setup

**File**: `lazyjob-cli/src/main.rs`

```rust
use tracing_subscriber::{fmt, EnvFilter};

fn init_tracing() {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();
}
```

**Key API**: `tracing_subscriber::fmt::init()`, `EnvFilter::from_default_env()`

**Verification**: `RUST_LOG=debug lazyjob-cli jobs list` shows structured log output.

#### Step 1.3: Error Types

**File**: `lazyjob-core/src/error.rs`

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found: {entity} with id {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: String, to: String },
    #[error("invalid interest level {0}: must be 1-5")]
    InvalidInterestLevel(u8),
    #[error("life sheet parse error: {0}")]
    LifeSheetParse(#[from] serde_yaml::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

#### Step 1.4: Database Initialization

**File**: `lazyjob-core/src/persistence/mod.rs`

```rust
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::path::Path;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePool::connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool { &self.pool }

    pub async fn close(self) { self.pool.close().await; }
}
```

**Key APIs**:
- `SqliteConnectOptions::journal_mode(SqliteJournalMode::Wal)` — WAL mode
- `SqliteConnectOptions::foreign_keys(true)` — enforce FK constraints
- `sqlx::migrate!("./migrations").run(&pool)` — apply migrations at startup

**Verification**: SQLite file created at `~/.lazyjob/lazyjob.db` with WAL file present.

#### Step 1.5: Repository Pattern

**File**: `lazyjob-core/src/persistence/job_repo.rs`

```rust
use sqlx::SqlitePool;
use crate::{models::job::Job, error::Result, ids::JobId};

pub struct JobRepository {
    pool: SqlitePool,
}

#[derive(Default)]
pub struct JobFilter {
    pub status: Option<String>,
    pub company_id: Option<String>,
    pub source: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl JobRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>> {
        // Build dynamic WHERE clause using QueryBuilder
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM jobs WHERE 1=1");
        if let Some(status) = &filter.status {
            qb.push(" AND status = ").push_bind(status);
        }
        if let Some(company_id) = &filter.company_id {
            qb.push(" AND company_id = ").push_bind(company_id);
        }
        qb.push(" ORDER BY discovered_at DESC");
        if let Some(limit) = filter.limit {
            qb.push(" LIMIT ").push_bind(limit);
        }
        let jobs = qb.build_query_as::<Job>().fetch_all(&self.pool).await?;
        Ok(jobs)
    }

    pub async fn get(&self, id: &JobId) -> Result<Option<Job>> {
        let job = sqlx::query_as!(
            Job,
            "SELECT * FROM jobs WHERE id = ?",
            id.as_str()
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(job)
    }

    pub async fn insert(&self, job: &Job) -> Result<()> {
        sqlx::query!(
            "INSERT INTO jobs (id, title, company_name, company_id, location, remote,
             url, description, salary_min, salary_max, salary_currency, status,
             interest_level, source, source_id, discovered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            job.id.as_str(), job.title, job.company_name,
            // ... bind remaining fields
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_by_source(&self, job: &Job) -> Result<bool> {
        // Returns true if new job was inserted (false = already existed)
        let result = sqlx::query!(
            "INSERT INTO jobs (id, title, company_name, source, source_id, ...)
             VALUES (?, ?, ?, ?, ?, ...)
             ON CONFLICT(source, source_id) DO NOTHING",
            job.id.as_str(), job.title, job.company_name, ...
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update(&self, job: &Job) -> Result<()> {
        sqlx::query!(
            "UPDATE jobs SET title=?, company_name=?, status=?, interest_level=?,
             notes=?, updated_at=datetime('now') WHERE id=?",
            job.title, job.company_name, job.status as _, job.interest_level.value(),
            job.id.as_str()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &JobId) -> Result<()> {
        sqlx::query!("DELETE FROM jobs WHERE id = ?", id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
```

**Key APIs**:
- `sqlx::QueryBuilder::new()` + `push_bind()` for dynamic filters (prevents SQL injection)
- `sqlx::query_as!()` macro for compile-time checked typed queries
- `ON CONFLICT(source, source_id) DO NOTHING` for idempotent source ingestion

#### Step 1.6: Application Repository with Transition Logging

**File**: `lazyjob-core/src/persistence/application_repo.rs`

```rust
impl ApplicationRepository {
    pub async fn transition_stage(
        &self,
        id: &ApplicationId,
        to: ApplicationStage,
        reason: Option<String>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let current = sqlx::query_scalar!(
            "SELECT stage FROM applications WHERE id = ?",
            id.as_str()
        )
        .fetch_one(&mut *tx)
        .await?;

        let from: ApplicationStage = current.parse().map_err(|_| CoreError::Other(
            anyhow::anyhow!("corrupt stage value in db: {current}")
        ))?;

        if !from.can_transition_to(to) {
            return Err(CoreError::InvalidTransition {
                from: format!("{from:?}"),
                to: format!("{to:?}"),
            });
        }

        sqlx::query!(
            "UPDATE applications SET stage=?, updated_at=datetime('now') WHERE id=?",
            to as _, id.as_str()
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "INSERT INTO application_transitions (id, application_id, from_stage, to_stage, reason)
             VALUES (lower(hex(randomblob(16))), ?, ?, ?, ?)",
            id.as_str(), from as _, to as _, reason
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
}
```

**Key API**: `pool.begin().await?` + `tx.commit().await?` for atomic multi-statement transactions.

#### Step 1.7: TUI Shell

**File**: `lazyjob-tui/src/app.rs`

```rust
use ratatui::{Terminal, backend::CrosstermBackend};
use crossterm::{
    event::{self, Event, KeyCode, EventStream},
    execute,
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tokio_stream::StreamExt;

pub struct App {
    pub active_view: View,
    pub jobs_list_state: JobsListState,
    pub applications_state: ApplicationsState,
    pub contacts_state: ContactsState,
    pub ralph_events: tokio::sync::broadcast::Receiver<RalphEvent>,
    pub db: Arc<Database>,
    pub should_quit: bool,
}

pub enum View { Dashboard, JobsList, JobDetail, Applications, Contacts, Settings, Help }

pub async fn run(db: Arc<Database>) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db.clone());
    let mut event_stream = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(16)); // 60fps

    let result = async {
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    terminal.draw(|f| ui::render(f, &app))?;
                }
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        handle_key(&mut app, key)?;
                    }
                    if app.should_quit { break; }
                }
                Ok(ralph_event) = app.ralph_events.recv() => {
                    app.handle_ralph_event(ralph_event);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}
```

**Key APIs**:
- `ratatui::Terminal::new(CrosstermBackend::new(stdout))` — terminal setup
- `crossterm::terminal::enable_raw_mode()` + `execute!(stdout, EnterAlternateScreen)` — raw mode
- `crossterm::event::EventStream::new()` — async event stream (requires `event-stream` feature)
- `tokio::select!` over tick + event stream + Ralph broadcast — unified event loop
- `terminal.draw(|f| render(f, &app))` — frame rendering

**Verification**: TUI launches in alternate screen. `j/k` moves selection. `q` exits cleanly and restores terminal.

#### Step 1.8: Layout Rendering

**File**: `lazyjob-tui/src/views/jobs_list.rs`

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn render(f: &mut Frame, state: &JobsListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // header
            Constraint::Min(0),      // list
            Constraint::Length(1),   // status bar
        ])
        .split(f.area());

    let header = Paragraph::new("LazyJob — Jobs")
        .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state.jobs.iter().map(|job| {
        let title = Span::styled(&job.title, Style::default().add_modifier(Modifier::BOLD));
        let company = Span::raw(format!(" · {}", job.company_name));
        ListItem::new(Line::from(vec![title, company]))
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Jobs "))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut state.list_state.clone());
}
```

**Key APIs**:
- `Layout::default().constraints([...]).split(f.area())` — responsive layout
- `List::new(items).highlight_style(...)` — selectable list with highlight
- `f.render_stateful_widget(list, rect, &mut list_state)` — stateful widget rendering

#### Step 1.9: Greenhouse and Lever API Clients

**File**: `lazyjob-core/src/discovery/greenhouse.rs`

```rust
use reqwest::Client;
use serde::Deserialize;

pub struct GreenhouseClient {
    http: Client,
    base_url: String,  // configurable for testing
}

#[derive(Deserialize)]
struct GreenhouseJobsResponse {
    jobs: Vec<GreenhouseJob>,
    meta: GreenhouseMeta,
}

#[derive(Deserialize)]
struct GreenhouseJob {
    id: u64,
    title: String,
    location: GreenhouseLocation,
    content: String,       // HTML description
    updated_at: String,    // ISO8601
    absolute_url: String,
}

impl GreenhouseClient {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("LazyJob/0.1")
                .build()
                .expect("reqwest client"),
            base_url: "https://boards-api.greenhouse.io".to_string(),
        }
    }

    // For integration tests: override base URL to a wiremock server
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { http: Client::new(), base_url: base_url.into() }
    }

    pub async fn fetch_jobs(&self, board_token: &str) -> Result<Vec<Job>, DiscoveryError> {
        let url = format!("{}/v1/boards/{}/jobs?content=true", self.base_url, board_token);
        let resp: GreenhouseJobsResponse = self.http
            .get(&url)
            .send()
            .await
            .map_err(DiscoveryError::Http)?
            .error_for_status()
            .map_err(DiscoveryError::Http)?
            .json()
            .await
            .map_err(DiscoveryError::Http)?;

        Ok(resp.jobs.into_iter().map(|g| Job {
            id: JobId::new(),
            title: g.title,
            company_name: board_token.to_string(), // enriched by caller
            source: JobSource::Greenhouse,
            source_id: Some(g.id.to_string()),
            url: Some(g.absolute_url),
            description: Some(ammonia::clean(&g.content)), // strip HTML tags
            location: Some(g.location.name),
            // ... defaults for other fields
            ..Default::default()
        }).collect())
    }
}
```

**Key APIs**:
- `reqwest::Client::builder().timeout().user_agent().build()` — HTTP client with config
- `.error_for_status()` — fail on 4xx/5xx
- `.json::<T>()` — typed deserialization
- `ammonia::clean()` — strip HTML from job descriptions safely

---

### Phase 2 — Application Tracking: Pipeline, Contacts, Dashboard (Weeks 4-5)

#### Step 2.1: Kanban Pipeline View

**File**: `lazyjob-tui/src/views/applications.rs`

The kanban board renders 9 columns for `ApplicationStage` variants. Since ratatui has no native equal-column layout, column widths are computed manually:

```rust
pub fn render(f: &mut Frame, state: &ApplicationsState) {
    let area = f.area();
    let stage_columns = ApplicationStage::ordered_columns();
    let col_width = area.width / stage_columns.len() as u16;

    for (i, stage) in stage_columns.iter().enumerate() {
        let col_rect = Rect {
            x: area.x + i as u16 * col_width,
            y: area.y,
            width: col_width,
            height: area.height,
        };
        render_stage_column(f, col_rect, stage, state);
    }
}

fn render_stage_column(
    f: &mut Frame,
    rect: Rect,
    stage: &ApplicationStage,
    state: &ApplicationsState,
) {
    let apps: Vec<&ApplicationCard> = state.cards.iter()
        .filter(|c| c.stage == *stage)
        .collect();

    let items: Vec<ListItem> = apps.iter().map(|card| {
        let age = card.days_in_stage();
        let color = if age > 7 { Color::Red } else { Color::White };
        ListItem::new(format!("{}\n{} ({}d)", card.company, card.title, age))
            .style(Style::default().fg(color))
    }).collect();

    let border_style = if state.focused_stage == *stage {
        Style::default().fg(Color::Blue)
    } else {
        Style::default()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(stage.label())
        .border_style(border_style);

    f.render_widget(List::new(items).block(block), rect);
}
```

#### Step 2.2: Dashboard Statistics

**File**: `lazyjob-tui/src/views/dashboard.rs`

```rust
pub struct DashboardStats {
    pub total_jobs: u64,
    pub active_applications: u64,
    pub response_rate: f64,   // 0.0-1.0
    pub interviews_scheduled: u64,
    pub offers_pending: u64,
    pub actions_required: Vec<ActionItem>,
}

// Computed from SQLite in lazyjob-core:
pub async fn compute_stats(pool: &SqlitePool) -> Result<DashboardStats> {
    let total_jobs = sqlx::query_scalar!("SELECT COUNT(*) FROM jobs")
        .fetch_one(pool).await? as u64;
    let active_applications = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM applications WHERE stage NOT IN ('accepted','rejected','withdrawn')"
    ).fetch_one(pool).await? as u64;
    // ... etc.
    Ok(DashboardStats { total_jobs, active_applications, .. })
}
```

---

### Phase 3 — LLM Integration + Ralph Foundation (Weeks 6-7)

#### Step 3.1: LlmProvider Trait

**File**: `lazyjob-llm/src/provider.rs`

```rust
use async_trait::async_trait;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, messages: Vec<ChatMessage>) -> LlmResult<ChatResponse>;

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> LlmResult<Pin<Box<dyn Stream<Item = LlmResult<String>> + Send>>>;

    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>>;

    fn model_id(&self) -> &str;
    fn max_context_tokens(&self) -> usize;
}

pub type BoxLlmProvider = Arc<dyn LlmProvider>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Role { System, User, Assistant }

#[derive(Debug)]
pub struct ChatResponse {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub model: String,
}
```

#### Step 3.2: OpenAI Provider

**File**: `lazyjob-llm/src/providers/openai.rs`

```rust
use async_openai::{Client, types::{CreateChatCompletionRequest, ChatCompletionRequestMessage}};

pub struct OpenAiProvider {
    client: Client<async_openai::config::OpenAIConfig>,
    model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &secrecy::Secret<String>, model: impl Into<String>) -> Self {
        let config = async_openai::config::OpenAIConfig::new()
            .with_api_key(api_key.expose_secret());
        Self {
            client: Client::with_config(config),
            model: model.into(),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(&self, messages: Vec<ChatMessage>) -> LlmResult<ChatResponse> {
        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: messages.into_iter().map(|m| match m.role {
                Role::System => ChatCompletionRequestMessage::System(
                    async_openai::types::ChatCompletionRequestSystemMessage {
                        content: m.content.into(),
                        ..Default::default()
                    }
                ),
                // ... User, Assistant
            }).collect(),
            ..Default::default()
        };

        let response = self.client.chat().create(request).await
            .map_err(LlmError::OpenAi)?;

        let choice = response.choices.into_iter().next()
            .ok_or_else(|| LlmError::EmptyResponse)?;

        Ok(ChatResponse {
            content: choice.message.content.unwrap_or_default(),
            input_tokens: response.usage.as_ref().map_or(0, |u| u.prompt_tokens),
            output_tokens: response.usage.as_ref().map_or(0, |u| u.completion_tokens),
            model: response.model,
        })
    }

    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        let request = async_openai::types::CreateEmbeddingRequest {
            model: "text-embedding-3-small".to_string(),
            input: async_openai::types::EmbeddingInput::String(text.to_string()),
            ..Default::default()
        };
        let resp = self.client.embeddings().create(request).await
            .map_err(LlmError::OpenAi)?;
        Ok(resp.data.into_iter().next().map(|e| e.embedding).unwrap_or_default())
    }

    fn model_id(&self) -> &str { &self.model }
    fn max_context_tokens(&self) -> usize { 128_000 }
}
```

#### Step 3.3: Credential Management

**File**: `lazyjob-core/src/credentials.rs`

```rust
use keyring::Entry;
use secrecy::Secret;

pub struct CredentialManager {
    service: &'static str,
}

impl CredentialManager {
    pub fn new() -> Self { Self { service: "lazyjob" } }

    pub fn store_api_key(&self, provider: &str, key: &Secret<String>) -> anyhow::Result<()> {
        Entry::new(self.service, provider)?
            .set_password(key.expose_secret())?;
        Ok(())
    }

    pub fn load_api_key(&self, provider: &str) -> anyhow::Result<Secret<String>> {
        let raw = Entry::new(self.service, provider)?.get_password()?;
        Ok(Secret::new(raw))
    }

    pub fn delete_api_key(&self, provider: &str) -> anyhow::Result<()> {
        Entry::new(self.service, provider)?.delete_credential()?;
        Ok(())
    }
}
```

**Key APIs**:
- `keyring::Entry::new(service, account)` — creates keychain entry handle
- `.set_password()`, `.get_password()`, `.delete_credential()` — CRUD operations
- Wraps returned string in `secrecy::Secret<String>` immediately

#### Step 3.4: Ralph Process Manager

**File**: `lazyjob-ralph/src/process.rs`

```rust
use tokio::process::{Command, Child};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;
use std::collections::HashMap;

pub struct RalphProcessManager {
    running: HashMap<LoopId, ChildHandle>,
    event_tx: broadcast::Sender<RalphEvent>,
}

struct ChildHandle {
    child: Child,
    loop_type: LoopType,
}

impl RalphProcessManager {
    pub fn new() -> (Self, broadcast::Receiver<RalphEvent>) {
        let (tx, rx) = broadcast::channel(256);
        (Self { running: HashMap::new(), event_tx: tx }, rx)
    }

    pub async fn start_loop(
        &mut self,
        loop_type: LoopType,
        params: serde_json::Value,
        db_path: &std::path::Path,
    ) -> anyhow::Result<LoopId> {
        let loop_id = LoopId::new();
        let mut child = Command::new(std::env::current_exe()?)
            .arg("ralph")
            .arg(loop_type.subcommand())
            .arg("--db")
            .arg(db_path)
            .arg("--loop-id")
            .arg(loop_id.as_str())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Send params via stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let params_json = serde_json::to_vec(&params)?;
            stdin.write_all(&params_json).await?;
            stdin.write_all(b"\n").await?;
        }

        // Spawn stdout reader task
        let stdout = child.stdout.take().expect("stdout piped");
        let tx = self.event_tx.clone();
        let lid = loop_id.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(msg) = serde_json::from_str::<RalphMessage>(&line) {
                    let _ = tx.send(RalphEvent { loop_id: lid.clone(), message: msg });
                }
            }
        });

        self.running.insert(loop_id.clone(), ChildHandle { child, loop_type });
        Ok(loop_id)
    }

    pub async fn cancel_loop(&mut self, loop_id: &LoopId) -> anyhow::Result<()> {
        if let Some(mut handle) = self.running.remove(loop_id) {
            handle.child.kill().await?;
        }
        Ok(())
    }
}
```

**Key APIs**:
- `tokio::process::Command::new().arg().stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()` — subprocess spawn
- `tokio::io::BufReader::new(stdout).lines().next_line().await` — async line-by-line stdout reading
- `broadcast::Sender::send()` — fan-out Ralph events to all TUI subscribers
- `child.kill().await` — cancellation

#### Step 3.5: Ralph JSON Protocol

**File**: `lazyjob-ralph/src/protocol.rs`

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RalphMessage {
    Status { phase: String, progress: f32, message: String },
    Results { data: serde_json::Value },
    Done { success: bool },
    Error { code: String, message: String },
    Heartbeat,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TuiMessage {
    Start { loop_type: String, params: serde_json::Value },
    Cancel,
    Pause,
    Resume,
}

#[derive(Debug, Clone)]
pub struct RalphEvent {
    pub loop_id: LoopId,
    pub message: RalphMessage,
}
```

**Serde pattern**: `#[serde(tag = "type")]` produces `{"type": "status", "phase": "...", ...}` — matches the protocol spec exactly.

---

### Phase 4 — Resume Tailoring (Weeks 8-9)

#### Step 4.1: Resume Tailoring Pipeline

**File**: `lazyjob-core/src/resume/mod.rs`

```rust
pub struct ResumeTailor {
    llm: Arc<dyn LlmProvider>,
    db: Arc<Database>,
    life_sheet: Arc<LifeSheet>,
}

pub struct TailoringResult {
    pub gap_report: GapReport,
    pub fabrication_report: FabricationReport,
    pub tailored_bullets: Vec<TailoredBullet>,
    pub tailored_summary: String,
    pub keyword_coverage: f32,
}

impl ResumeTailor {
    pub async fn tailor(&self, job: &Job) -> Result<TailoringResult> {
        // Step 1: Parse JD
        let analysis = self.parse_job_description(job).await?;

        // Step 2: Gap analysis (LLM + strsim fuzzy matching)
        let gap_report = self.analyze_gaps(&analysis).await?;

        // Step 3: Fabrication guardrails check
        let fabrication_report = self.check_fabrication(&gap_report)?;

        // Step 4: Draft content (only if no Forbidden risks)
        if fabrication_report.has_forbidden_risks() {
            return Err(CoreError::FabricationRisk(fabrication_report));
        }

        let tailored_bullets = self.draft_bullets(job, &gap_report).await?;
        let tailored_summary = self.draft_summary(job, &gap_report).await?;

        Ok(TailoringResult {
            gap_report,
            fabrication_report,
            tailored_bullets,
            tailored_summary,
            keyword_coverage: 0.0, // computed post-draft
        })
    }
}
```

**File**: `lazyjob-core/src/resume/gap_analysis.rs`

```rust
use strsim::jaro_winkler;

pub struct GapReport {
    pub required_skills: Vec<SkillGap>,
    pub preferred_skills: Vec<SkillGap>,
    pub matched_keywords: Vec<String>,
    pub missing_keywords: Vec<String>,
}

pub struct SkillGap {
    pub skill: String,
    pub required: bool,
    pub found_in_life_sheet: bool,
    pub similarity_score: f64,  // best match score from strsim
    pub matched_entry: Option<String>,
}

pub fn compute_skill_gaps(
    required_skills: &[String],
    life_sheet_skills: &[SkillEntry],
) -> Vec<SkillGap> {
    required_skills.iter().map(|req| {
        // Find best fuzzy match in life sheet
        let best = life_sheet_skills.iter()
            .map(|s| (jaro_winkler(req, &s.name), &s.name))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let (score, matched) = best.unwrap_or((0.0, &String::new()));
        SkillGap {
            skill: req.clone(),
            required: true,
            found_in_life_sheet: score > 0.85,
            similarity_score: score,
            matched_entry: if score > 0.85 { Some(matched.clone()) } else { None },
        }
    }).collect()
}
```

#### Step 4.2: DOCX Generation

**File**: `lazyjob-core/src/resume/docx_generator.rs`

```rust
use docx_rs::{Docx, Paragraph, Run, Table, TableRow, TableCell};

pub struct DocxGenerator;

impl DocxGenerator {
    pub fn generate(
        &self,
        basics: &Basics,
        summary: &str,
        experience: &[TailoredExperience],
        skills: &[String],
        education: &[EducationEntry],
    ) -> anyhow::Result<Vec<u8>> {
        let mut docx = Docx::new();

        // Name + contact header
        docx = docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(&basics.name).bold())
        );
        docx = docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(&format!(
                    "{} · {}",
                    basics.email, basics.phone.as_deref().unwrap_or("")
                )))
        );

        // Summary section
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("Summary").bold())
        );
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(summary))
        );

        // Experience entries
        for exp in experience {
            docx = docx.add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text(&exp.position).bold())
                    .add_run(Run::new().add_text(&format!(" — {}", exp.company)))
            );
            for bullet in &exp.bullets {
                docx = docx.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(&format!("• {}", bullet)))
                );
            }
        }

        // Skills
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("Skills").bold())
        );
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&skills.join(", ")))
        );

        let mut buf = Vec::new();
        docx.build().pack(&mut std::io::Cursor::new(&mut buf))?;
        Ok(buf)
    }
}
```

**Key APIs**:
- `docx_rs::Docx::new()` — new document
- `Paragraph::new().add_run(Run::new().add_text(...).bold())` — styled paragraphs
- `docx.build().pack(&mut cursor)` — serialize to bytes

---

### Phase 5 — Polish + Discovery Loop (Weeks 10-12)

#### Step 5.1: Ralph JobDiscovery Loop Entrypoint

**File**: `lazyjob-ralph/src/loops/job_discovery.rs`

```rust
pub async fn run_job_discovery(
    pool: SqlitePool,
    loop_id: LoopId,
    params: JobDiscoveryParams,
) -> anyhow::Result<()> {
    let company_repo = CompanyRepository::new(pool.clone());
    let job_repo = JobRepository::new(pool.clone());
    let gh_client = GreenhouseClient::new();
    let lever_client = LeverClient::new();

    let companies = company_repo.list_with_board_tokens().await?;
    let total = companies.len();

    for (i, company) in companies.iter().enumerate() {
        let progress = i as f32 / total as f32;
        emit_status("fetching", progress, &format!("Fetching {}", company.name));

        let mut new_count = 0u32;

        if let Some(token) = &company.greenhouse_board_token {
            match gh_client.fetch_jobs(token).await {
                Ok(jobs) => {
                    for mut job in jobs {
                        job.company_id = Some(company.id.clone());
                        job.company_name = company.name.clone();
                        if job_repo.upsert_by_source(&job).await? {
                            new_count += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(company = %company.name, error = %e, "Greenhouse fetch failed");
                    emit_error("fetch_failed", &e.to_string());
                }
            }
        }

        // Same for Lever...
    }

    emit_done(true);
    Ok(())
}

fn emit_status(phase: &str, progress: f32, message: &str) {
    let msg = RalphMessage::Status {
        phase: phase.to_string(),
        progress,
        message: message.to_string(),
    };
    println!("{}", serde_json::to_string(&msg).unwrap());
}
```

**Verification**: Run `lazyjob-cli ralph job-discovery --db ~/.lazyjob/lazyjob.db`, see NDJSON status messages on stdout, verify new job rows in SQLite.

#### Step 5.2: CLI Entry Point

**File**: `lazyjob-cli/src/main.rs`

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lazyjob", version, about = "AI-powered job search terminal")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the terminal UI
    Tui,
    /// Ralph subprocess entrypoint (internal use)
    Ralph {
        #[command(subcommand)]
        loop_type: RalphLoop,
        #[arg(long)]
        db: std::path::PathBuf,
        #[arg(long)]
        loop_id: String,
    },
    /// Manage job database directly
    Jobs {
        #[command(subcommand)]
        action: JobsAction,
    },
}

#[derive(Subcommand)]
enum RalphLoop {
    JobDiscovery,
    CompanyResearch { company_id: String },
    ResumeTailor { job_id: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    lazyjob_cli::init_tracing();
    let cli = Cli::parse();

    let db_path = dirs::home_dir()
        .expect("home dir")
        .join(".lazyjob/lazyjob.db");

    match cli.command {
        Commands::Tui => {
            let db = Arc::new(Database::open(&db_path).await?);
            lazyjob_tui::run(db).await?;
        }
        Commands::Ralph { loop_type, db, loop_id } => {
            let pool = Database::open(&db).await?.pool().clone();
            match loop_type {
                RalphLoop::JobDiscovery => {
                    let params: JobDiscoveryParams = read_stdin_json().await?;
                    lazyjob_ralph::loops::job_discovery::run(pool, loop_id.into(), params).await?;
                }
                // ...
            }
        }
        Commands::Jobs { action } => {
            let db = Database::open(&db_path).await?;
            lazyjob_cli::jobs::handle(action, &db).await?;
        }
    }

    Ok(())
}
```

---

## Key Crate APIs

| Operation | Crate API |
|-----------|-----------|
| SQLite pool init | `SqlitePool::connect_with(SqliteConnectOptions)` |
| WAL mode | `SqliteConnectOptions::journal_mode(SqliteJournalMode::Wal)` |
| Run migrations | `sqlx::migrate!("./migrations").run(&pool).await` |
| Dynamic SQL | `sqlx::QueryBuilder::new(base).push(" AND x = ").push_bind(val)` |
| Typed query | `sqlx::query_as!(Model, "SELECT ...", args).fetch_all(&pool).await` |
| Transaction | `let mut tx = pool.begin().await?; tx.commit().await?` |
| TUI init | `enable_raw_mode()? + execute!(stdout, EnterAlternateScreen)` |
| TUI render | `terminal.draw(\|f\| render(f, &app))?` |
| Async key events | `EventStream::new()` from `crossterm::event` with `event-stream` feature |
| Subprocess spawn | `Command::new(exe).arg(...).stdout(Stdio::piped()).spawn()?` |
| Stdout lines | `BufReader::new(stdout).lines().next_line().await` |
| Broadcast events | `broadcast::channel::<RalphEvent>(256)` |
| HTTP fetch + JSON | `client.get(url).send().await?.json::<T>().await?` |
| Keychain store | `keyring::Entry::new(service, account).set_password(val)` |
| Keychain load | `keyring::Entry::new(service, account).get_password()` |
| Fuzzy match | `strsim::jaro_winkler(a, b) -> f64` |
| DOCX build | `Docx::new().add_paragraph(...).build().pack(&mut cursor)` |
| Secret wrap | `secrecy::Secret::new(string)` / `.expose_secret()` |

---

## Error Handling

```rust
// lazyjob-core/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("invalid stage transition from {from:?} to {to:?}")]
    InvalidTransition { from: String, to: String },
    #[error("fabrication risk detected: {0:?}")]
    FabricationRisk(FabricationReport),
    #[error("life sheet: {0}")]
    LifeSheet(#[from] serde_yaml::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// lazyjob-llm/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum LlmError {
    #[error("openai api: {0}")]
    OpenAi(#[from] async_openai::error::OpenAIError),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("empty response from model")]
    EmptyResponse,
    #[error("provider not configured: {0}")]
    NotConfigured(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// lazyjob-tui/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum TuiError {
    #[error("terminal: {0}")]
    Terminal(#[from] std::io::Error),
    #[error("crossterm: {0}")]
    Crossterm(#[from] crossterm::ErrorKind),
    #[error("core: {0}")]
    Core(#[from] lazyjob_core::error::CoreError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

---

## Testing Strategy

### Unit Tests

**State machine** (`lazyjob-core/src/models/application.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transition_applied_to_phone_screen() {
        assert!(ApplicationStage::Applied.can_transition_to(ApplicationStage::PhoneScreen));
    }

    #[test]
    fn invalid_transition_interested_to_offer() {
        assert!(!ApplicationStage::Interested.can_transition_to(ApplicationStage::Offer));
    }

    #[test]
    fn terminal_states_have_no_transitions() {
        assert!(ApplicationStage::Accepted.valid_transitions().is_empty());
        assert!(ApplicationStage::Rejected.valid_transitions().is_empty());
    }
}
```

**Gap analysis** (`lazyjob-core/src/resume/gap_analysis.rs`):
```rust
#[test]
fn fuzzy_skill_match_typescript_vs_ts() {
    let skills = vec![SkillEntry { name: "TypeScript".into(), .. }];
    let gaps = compute_skill_gaps(&["typescript".into()], &skills);
    assert!(gaps[0].found_in_life_sheet);
}
```

**Repository with in-memory SQLite**:
```rust
#[sqlx::test]
async fn job_upsert_dedup(pool: SqlitePool) {
    let repo = JobRepository::new(pool);
    let job = make_test_job("greenhouse", "12345");
    let inserted = repo.upsert_by_source(&job).await.unwrap();
    assert!(inserted);
    let dup = repo.upsert_by_source(&job).await.unwrap();
    assert!(!dup); // duplicate — not inserted
}
```

**Note**: `#[sqlx::test]` attribute automatically creates a fresh in-memory SQLite pool and runs migrations before each test.

### Integration Tests

**Full discovery pipeline** (`tests/integration/discovery.rs`):
```rust
#[tokio::test]
async fn greenhouse_client_fetches_jobs() {
    // Start wiremock server
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/boards/stripe/jobs"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({
                "jobs": [{"id": 1, "title": "SWE", ...}],
                "meta": {"total": 1}
            })))
        .mount(&mock_server)
        .await;

    let client = GreenhouseClient::with_base_url(mock_server.uri());
    let jobs = client.fetch_jobs("stripe").await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].source_id, Some("1".to_string()));
}
```

**Stage transition atomicity** (`tests/integration/workflow.rs`):
```rust
#[sqlx::test]
async fn transition_logs_history(pool: SqlitePool) {
    let app_repo = ApplicationRepository::new(pool.clone());
    let id = insert_test_application(&pool, ApplicationStage::Applied).await;

    app_repo.transition_stage(&id, ApplicationStage::PhoneScreen, None).await.unwrap();

    let transitions = app_repo.list_transitions(&id).await.unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].from_stage, ApplicationStage::Applied);
    assert_eq!(transitions[0].to_stage, ApplicationStage::PhoneScreen);
}
```

### TUI Tests

TUI widget rendering can be tested without a real terminal using `ratatui::backend::TestBackend`:

```rust
#[test]
fn jobs_list_renders_job_title() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = JobsListState {
        jobs: vec![make_test_job("Google SWE")],
        list_state: ListState::default(),
    };
    terminal.draw(|f| jobs_list::render(f, &state)).unwrap();
    let rendered = terminal.backend().buffer().clone();
    // Assert expected cell content
    assert!(rendered.content().iter().any(|cell| cell.symbol() == "G"));
}
```

---

## Open Questions

1. **`sqlx` compile-time macro offline mode**: `sqlx::query!` requires `DATABASE_URL` at compile time or a prepared query cache (`sqlx-data.json`). Decision: Use `sqlx prepare` in CI to generate the cache, committed to the repo. Developers need `DATABASE_URL` set locally.

2. **Embedding storage for job matching**: The spec defers vector DB and uses in-memory cosine similarity. When job counts exceed ~10,000 the in-memory approach will be slow. Defer `sqlite-vec` extension until that threshold is reached. For MVP: embed on demand when viewing Job Detail, not bulk.

3. **`async-openai` version compatibility**: `async-openai` has a fast release cadence. Pin to `0.28` in workspace and test before bumping.

4. **Ralph as same binary vs. separate binary**: Current plan has `lazyjob-cli ralph job-discovery` as the subprocess — same binary, different subcommand. This simplifies distribution but means the TUI binary must be on PATH or the subprocess spawn must use `std::env::current_exe()`. Use `current_exe()`.

5. **DOCX viewer integration**: After generating the DOCX, `open::that(path)` (from the `open` crate) launches the default OS viewer. Add `open = "5"` to workspace deps.

6. **Keybinding configuration format**: Phase 1 hardcodes keybindings. Phase 3 adds TOML config override. Format: `~/.config/lazyjob/keybindings.toml` with `[normal] g = "go_top"` entries. Define `KeymapConfig` in `lazyjob-tui`.

7. **Life sheet YAML schema validation**: `serde_yaml` will deserialize into `LifeSheet` but unknown fields are silently ignored. Add `#[serde(deny_unknown_fields)]` for strict validation in Phase 2, but with a user-facing error that shows the offending field name.

8. **CI binary caching**: GitHub Actions with `actions/cache` on `~/.cargo/registry` and `target/` reduces CI time from ~10min to ~2min for incremental builds.

---

## Related Specs

- `specs/01-architecture-implementation-plan.md` — crate layout, ratatui patterns
- `specs/02-llm-provider-abstraction-implementation-plan.md` — LlmProvider trait detail
- `specs/04-sqlite-persistence-implementation-plan.md` — repository pattern, WAL mode
- `specs/06-ralph-loop-integration-implementation-plan.md` — subprocess IPC protocol
- `specs/07-resume-tailoring-pipeline-implementation-plan.md` — full resume pipeline stages
- `specs/09-tui-design-keybindings-implementation-plan.md` — ratatui widget hierarchy
- `specs/10-application-workflow-implementation-plan.md` — state machine transitions
- `specs/11-platform-api-integrations-implementation-plan.md` — Greenhouse/Lever clients
- `specs/16-privacy-security-implementation-plan.md` — keychain, secret handling
- `specs/17-ralph-prompt-templates-implementation-plan.md` — prompt template system
