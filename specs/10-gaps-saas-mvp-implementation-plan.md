# Implementation Plan: SaaS & MVP Gap Analysis Closure

## Status
Draft

## Related Spec
`specs/10-gaps-saas-mvp.md`

## Overview

This plan closes the eleven gaps (GAP-99 through GAP-109) and two cross-spec concerns (V, W) identified in the SaaS/MVP gap analysis. The gaps address the commercial infrastructure — freemium limits, data portability, team workspaces, billing, onboarding, enterprise SSO, webhooks, and infrastructure — that transforms LazyJob from a polished local tool into a viable, production-grade SaaS business.

The plan is structured so that GAP-99 (freemium limits) and GAP-100 (data portability) are treated as critical prerequisites to any public SaaS launch. They directly affect user acquisition (quotas confuse free users → churn) and legal trust (GDPR export rights). GAP-101 (teams), GAP-104 (billing clarity), GAP-105 (onboarding), and GAP-107 (webhooks) are important for growth. GAP-106 (enterprise security), GAP-108 (SLA), and GAP-109 (infrastructure) are post-seed concerns. GAP-102 (mobile) and GAP-103 (shared drafts) are long-horizon and addressed only with type stubs here.

All Rust types in this plan live in `lazyjob-core/src/billing/`, `lazyjob-core/src/export/`, `lazyjob-api/src/`, and `lazyjob-core/src/onboarding/`. The `lazyjob-api` and `lazyjob-sync` crates (introduced in the SaaS migration plan) are the primary homes for cloud-side logic.

## Prerequisites

- `specs/18-saas-migration-path-implementation-plan.md` — `Plan` enum, `FeatureFlags`, `lazyjob-api` crate, `lazyjob-sync` crate all defined there.
- `specs/16-privacy-security-implementation-plan.md` — `age` encryption, `MasterPassword::derive_key()`, `Zeroizing<Vec<u8>>` key management. Cross-Spec W depends on these.
- `specs/04-sqlite-persistence-implementation-plan.md` — repository traits.

### Crates to Add

```toml
# lazyjob-core/Cargo.toml
zip = "0.6"                           # archive creation for export
csv = "1.3"                           # CSV export of tabular data
sha2 = "0.10"                         # integrity hash for export archive
hex = "0.4"                           # encode SHA-256 for manifest

# lazyjob-api/Cargo.toml
samael = "0.0.14"                     # SAML 2.0 IdP integration (GAP-106)
# or: openidconnect = "3" for OIDC-only SSO

[dev-dependencies]
wiremock = "0.6"
testcontainers = "0.15"
```

---

## Architecture

### Crate Placement

| Concern | Crate |
|---|---|
| `Plan` enum, quota limits, `QuotaService` | `lazyjob-core/src/billing/` |
| `DataExporter`, `ExportManifest` | `lazyjob-core/src/export/` |
| `TeamRecord`, `TeamMember`, RBAC | `lazyjob-core/src/team/` |
| `OnboardingState`, first-run wizard | `lazyjob-core/src/onboarding/` |
| `WebhookRegistry`, delivery loop | `lazyjob-api/src/webhooks/` |
| SSO/SAML provider | `lazyjob-api/src/auth/sso.rs` |
| Billing gate middleware (axum) | `lazyjob-api/src/middleware/plan_gate.rs` |
| Usage tracking (quota counters) | `lazyjob-core/src/billing/usage.rs` |

---

### Core Types

```rust
// lazyjob-core/src/billing/plan.rs

/// Canonical plan representation. Matches the `plan` field in config.toml and
/// the `plan` column in the cloud `users` table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    /// Local-only; no account required.
    Local,
    /// Cloud-synced free tier.
    Free,
    /// Paid individual tier ($X/month or $Y/year).
    Pro,
    /// Paid team tier (per-seat).
    Team,
    /// Enterprise; SSO, audit log, SCIM.
    Enterprise,
}

impl Plan {
    /// Returns the quota limits for this plan.
    pub fn limits(&self) -> PlanLimits {
        match self {
            Plan::Local | Plan::Free => PlanLimits {
                jobs_per_month: 25,
                applications_per_month: 10,
                ralph_runs_per_month: 5,
                contacts: 50,
                resumes_stored: 3,
                cloud_sync: false,
                team_seats: 0,
                api_calls_per_minute: 0,
            },
            Plan::Pro => PlanLimits {
                jobs_per_month: 500,
                applications_per_month: 200,
                ralph_runs_per_month: 100,
                contacts: 2000,
                resumes_stored: 50,
                cloud_sync: true,
                team_seats: 0,
                api_calls_per_minute: 60,
            },
            Plan::Team => PlanLimits {
                jobs_per_month: 2000,
                applications_per_month: 1000,
                ralph_runs_per_month: 500,
                contacts: 10_000,
                resumes_stored: 200,
                cloud_sync: true,
                team_seats: 25,
                api_calls_per_minute: 300,
            },
            Plan::Enterprise => PlanLimits {
                jobs_per_month: u32::MAX,
                applications_per_month: u32::MAX,
                ralph_runs_per_month: u32::MAX,
                contacts: u32::MAX,
                resumes_stored: u32::MAX,
                cloud_sync: true,
                team_seats: u32::MAX,
                api_calls_per_minute: u32::MAX,
            },
        }
    }

    /// Features gated by plan (returns `false` if feature not included).
    pub fn has_feature(&self, feature: PlanFeature) -> bool {
        match (self, feature) {
            (Plan::Local | Plan::Free, PlanFeature::CloudSync) => false,
            (Plan::Local | Plan::Free, PlanFeature::ApiAccess) => false,
            (Plan::Local | Plan::Free, PlanFeature::TeamWorkspaces) => false,
            (Plan::Local | Plan::Free, PlanFeature::PrioritySupport) => false,
            (Plan::Local | Plan::Free | Plan::Pro, PlanFeature::EnterpriseSSO) => false,
            (Plan::Local | Plan::Free | Plan::Pro, PlanFeature::AuditLog) => false,
            (Plan::Local | Plan::Free | Plan::Pro, PlanFeature::ScimProvisioning) => false,
            _ => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanLimits {
    pub jobs_per_month: u32,
    pub applications_per_month: u32,
    pub ralph_runs_per_month: u32,
    pub contacts: u32,
    pub resumes_stored: u32,
    pub cloud_sync: bool,
    pub team_seats: u32,
    pub api_calls_per_minute: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanFeature {
    CloudSync,
    ApiAccess,
    TeamWorkspaces,
    PrioritySupport,
    EnterpriseSSO,
    AuditLog,
    ScimProvisioning,
    DataExportFull,
    WebhookDelivery,
    CustomTemplates,
}
```

```rust
// lazyjob-core/src/billing/usage.rs

use chrono::{DateTime, Utc, NaiveDate};

/// Per-user monthly usage snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonthlyUsage {
    pub user_id: UserId,
    pub period: NaiveDate,          // first day of the month (UTC)
    pub jobs_created: u32,
    pub applications_submitted: u32,
    pub ralph_runs: u32,
    pub contacts_total: u32,        // total count, not monthly delta
    pub resumes_stored: u32,        // total count
}

/// The result of a quota check.
#[derive(Debug, PartialEq, Eq)]
pub enum QuotaResult {
    Allowed,
    SoftWarning { usage_pct: u8 },  // >= 80% used — show upsell banner
    HardBlocked { reason: QuotaBlockReason },
}

#[derive(Debug, PartialEq, Eq)]
pub enum QuotaBlockReason {
    JobsPerMonthExceeded,
    ApplicationsPerMonthExceeded,
    RalphRunsExceeded,
    ContactsLimitReached,
    ResumesLimitReached,
    FeatureNotInPlan(PlanFeature),
}

/// Service: checks and records quota usage.
pub struct QuotaService {
    db: Arc<rusqlite::Connection>,
}

impl QuotaService {
    pub fn new(db: Arc<rusqlite::Connection>) -> Self {
        Self { db }
    }

    /// Returns the current monthly usage for the local user.
    pub fn get_usage(&self) -> Result<MonthlyUsage, QuotaError> { /* ... */ }

    /// Check if an action is allowed under the current plan.
    pub fn check(
        &self,
        action: QuotableAction,
        plan: &Plan,
    ) -> Result<QuotaResult, QuotaError> { /* ... */ }

    /// Record that a quotable action was performed.
    /// Called *after* the action succeeds.
    pub fn record(
        &self,
        action: QuotableAction,
    ) -> Result<(), QuotaError> { /* ... */ }
}

#[derive(Debug, Clone, Copy)]
pub enum QuotableAction {
    CreateJob,
    SubmitApplication,
    RunRalph,
    AddContact,
    StoreResume,
}
```

```rust
// lazyjob-core/src/export/types.rs

/// A complete export archive descriptor.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportManifest {
    pub version: u8,                         // schema version = 1
    pub exported_at: DateTime<Utc>,
    pub user_id: Option<String>,             // None for local-only exports
    pub format: ExportFormat,
    pub tables: Vec<ExportedTable>,
    pub sha256: String,                      // hex SHA-256 of archive bytes before signing
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// Portable JSON + CSV bundle inside a ZIP archive.
    FullPortable,
    /// Raw SQLite dump (includes encrypted blobs; requires master password to read).
    SqliteSnapshot,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportedTable {
    pub name: String,
    pub row_count: usize,
    pub filename: String,   // e.g. "jobs.json" or "jobs.csv"
}
```

```rust
// lazyjob-core/src/team/types.rs

use uuid::Uuid;

pub type TeamId = Uuid;
pub type UserId = Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeamRecord {
    pub id: TeamId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub owner_id: UserId,
    pub plan: Plan,
    pub seat_count: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeamMember {
    pub team_id: TeamId,
    pub user_id: UserId,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by: UserId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl TeamRole {
    pub fn can_invite(&self) -> bool {
        matches!(self, TeamRole::Owner | TeamRole::Admin)
    }

    pub fn can_delete_team_data(&self) -> bool {
        matches!(self, TeamRole::Owner | TeamRole::Admin)
    }

    pub fn can_view_member_data(&self) -> bool {
        matches!(self, TeamRole::Owner | TeamRole::Admin)
    }
}
```

```rust
// lazyjob-core/src/onboarding/types.rs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnboardingState {
    pub completed_steps: Vec<OnboardingStep>,
    pub activated_at: Option<DateTime<Utc>>,
    pub skipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    ConnectedLlmProvider,
    ImportedLifeSheet,
    CreatedFirstJob,
    RanFirstRalphLoop,
    SubmittedFirstApplication,
}

impl OnboardingStep {
    /// A user is "activated" when they complete this step.
    pub fn is_activation_gate(&self) -> bool {
        matches!(self, OnboardingStep::SubmittedFirstApplication)
    }
}

#[derive(Debug, Clone)]
pub struct ImportSource {
    pub kind: ImportSourceKind,
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum ImportSourceKind {
    /// Huntr JSON export
    Huntr,
    /// Teal JSON export
    Teal,
    /// Generic LazyJob JSON export
    LazyJob,
    /// LinkedIn CSV contacts export
    LinkedInContacts,
}
```

```rust
// lazyjob-api/src/webhooks/types.rs

use uuid::Uuid;
use chrono::{DateTime, Utc};

pub type WebhookId = Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WebhookEndpoint {
    pub id: WebhookId,
    pub user_id: UserId,
    pub url: String,               // validated HTTPS URL
    pub secret: String,            // HMAC-SHA256 signing secret (user-provided or generated)
    pub events: Vec<WebhookEvent>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub last_delivery_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    ApplicationStageChanged,
    InterviewScheduled,
    OfferReceived,
    RalphLoopCompleted,
    JobDiscovered,
    ReminderFired,
}

#[derive(Debug, serde::Serialize)]
pub struct WebhookPayload {
    pub event: WebhookEvent,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WebhookDeliveryLog {
    pub id: Uuid,
    pub webhook_id: WebhookId,
    pub event: WebhookEvent,
    pub attempted_at: DateTime<Utc>,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub attempt_number: u8,
    pub next_retry_at: Option<DateTime<Utc>>,
}
```

---

### Trait Definitions

```rust
// lazyjob-core/src/billing/traits.rs

#[async_trait::async_trait]
pub trait QuotaRepository: Send + Sync {
    async fn get_monthly_usage(&self, period: NaiveDate) -> Result<MonthlyUsage, QuotaError>;
    async fn increment(&self, action: QuotableAction) -> Result<(), QuotaError>;
    async fn get_upsell_trigger(&self, plan: &Plan) -> Result<Option<UpsellTrigger>, QuotaError>;
}

// lazyjob-core/src/export/traits.rs

#[async_trait::async_trait]
pub trait DataExporter: Send + Sync {
    /// Export all user data to a ZIP archive at `dest_path`.
    async fn export_full(
        &self,
        dest_path: &Path,
        format: ExportFormat,
    ) -> Result<ExportManifest, ExportError>;

    /// Export a single table subset (e.g. just jobs or just contacts).
    async fn export_table(
        &self,
        table: ExportableTable,
        dest_path: &Path,
    ) -> Result<ExportedTable, ExportError>;
}

// lazyjob-core/src/team/traits.rs

#[async_trait::async_trait]
pub trait TeamRepository: Send + Sync {
    async fn create_team(&self, name: &str, owner_id: UserId) -> Result<TeamRecord, TeamError>;
    async fn get_team(&self, id: TeamId) -> Result<Option<TeamRecord>, TeamError>;
    async fn add_member(&self, team_id: TeamId, user_id: UserId, role: TeamRole, invited_by: UserId) -> Result<TeamMember, TeamError>;
    async fn remove_member(&self, team_id: TeamId, user_id: UserId) -> Result<(), TeamError>;
    async fn list_members(&self, team_id: TeamId) -> Result<Vec<TeamMember>, TeamError>;
    async fn get_member_role(&self, team_id: TeamId, user_id: UserId) -> Result<Option<TeamRole>, TeamError>;
}
```

---

### SQLite Schema

```sql
-- Migration 018: Quota tracking
CREATE TABLE IF NOT EXISTS monthly_usage (
    user_id              TEXT NOT NULL DEFAULT 'local',
    period               TEXT NOT NULL,           -- ISO 8601 YYYY-MM-01
    jobs_created         INTEGER NOT NULL DEFAULT 0,
    applications_submitted INTEGER NOT NULL DEFAULT 0,
    ralph_runs           INTEGER NOT NULL DEFAULT 0,
    updated_at           TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, period)
);

-- Migration 019: Onboarding state
CREATE TABLE IF NOT EXISTS onboarding_state (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),   -- singleton row
    completed_steps      TEXT NOT NULL DEFAULT '[]',           -- JSON array of step names
    activated_at         TEXT,                                  -- ISO 8601
    skipped              INTEGER NOT NULL DEFAULT 0
);

-- Migration 020: Teams
CREATE TABLE IF NOT EXISTS teams (
    id                   TEXT PRIMARY KEY,         -- UUID
    name                 TEXT NOT NULL,
    owner_id             TEXT NOT NULL,
    plan                 TEXT NOT NULL DEFAULT 'team',
    seat_count           INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS team_members (
    team_id              TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id              TEXT NOT NULL,
    role                 TEXT NOT NULL CHECK (role IN ('owner','admin','member','viewer')),
    joined_at            TEXT NOT NULL DEFAULT (datetime('now')),
    invited_by           TEXT NOT NULL,
    PRIMARY KEY (team_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id);

-- Migration 021: Webhook endpoints and delivery log
CREATE TABLE IF NOT EXISTS webhook_endpoints (
    id                   TEXT PRIMARY KEY,         -- UUID
    user_id              TEXT NOT NULL DEFAULT 'local',
    url                  TEXT NOT NULL,
    secret               TEXT NOT NULL,
    events               TEXT NOT NULL DEFAULT '[]',   -- JSON array
    active               INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    last_delivery_at     TEXT
);

CREATE TABLE IF NOT EXISTS webhook_delivery_log (
    id                   TEXT PRIMARY KEY,
    webhook_id           TEXT NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    event                TEXT NOT NULL,
    attempted_at         TEXT NOT NULL DEFAULT (datetime('now')),
    status_code          INTEGER,
    error                TEXT,
    attempt_number       INTEGER NOT NULL DEFAULT 1,
    next_retry_at        TEXT
);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_retry
    ON webhook_delivery_log(next_retry_at)
    WHERE next_retry_at IS NOT NULL;

-- Migration 022: Audit log (Enterprise)
CREATE TABLE IF NOT EXISTS audit_log (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    team_id              TEXT,
    action               TEXT NOT NULL,            -- e.g. 'export_data', 'login', 'delete_application'
    resource_type        TEXT,                     -- e.g. 'application', 'contact'
    resource_id          TEXT,
    ip_address           TEXT,
    user_agent           TEXT,
    occurred_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_log_team ON audit_log(team_id, occurred_at DESC);
```

---

### Module Structure

```
lazyjob-core/
  src/
    billing/
      mod.rs           -- pub use plan::*, usage::*, quota::*, errors::*
      plan.rs          -- Plan enum, PlanLimits, PlanFeature
      usage.rs         -- MonthlyUsage, QuotaService, QuotaResult
      quota.rs         -- QuotaRepository trait + SQLite impl
      upsell.rs        -- UpsellTrigger, upsell CTA strings
      errors.rs        -- QuotaError (thiserror)
    export/
      mod.rs
      types.rs         -- ExportManifest, ExportFormat, ExportedTable
      exporter.rs      -- SqliteDataExporter impl
      format/
        json.rs        -- serialize rows to JSON
        csv.rs         -- serialize tabular rows to CSV
      archive.rs       -- ZIP creation, SHA-256 manifest signing
      errors.rs        -- ExportError
    team/
      mod.rs
      types.rs         -- TeamRecord, TeamMember, TeamRole
      repository.rs    -- TeamRepository trait + SQLite impl
      invite.rs        -- InviteToken generation/validation
      errors.rs        -- TeamError
    onboarding/
      mod.rs
      types.rs         -- OnboardingState, OnboardingStep, ImportSource
      service.rs       -- OnboardingService (mark steps, detect activation)
      import/
        mod.rs
        huntr.rs       -- HuntrImporter
        teal.rs        -- TealImporter
        lazyjob.rs     -- LazyJobImporter
      errors.rs        -- OnboardingError

lazyjob-api/
  src/
    webhooks/
      mod.rs
      types.rs         -- WebhookEndpoint, WebhookEvent, WebhookPayload, WebhookDeliveryLog
      registry.rs      -- WebhookRegistry (in-memory + DB-backed)
      delivery.rs      -- WebhookDeliveryWorker (tokio background task)
      signing.rs       -- HMAC-SHA256 signature generation/verification
      repository.rs    -- WebhookRepository trait + Postgres impl
      errors.rs        -- WebhookError
    auth/
      sso.rs           -- SsoProvider trait, SamlProvider impl
      scim.rs          -- SCIM provisioning endpoint handlers
    middleware/
      plan_gate.rs     -- axum layer: enforces Plan::has_feature()
      quota_gate.rs    -- axum layer: checks QuotaService::check() before handler
      audit.rs         -- axum layer: writes to audit_log after handler
```

---

## Implementation Phases

### Phase 1 — Freemium Limits & Quota Enforcement (Critical MVP)

**Goal**: Every user on Free/Local plans sees their quota usage and hits a graceful upsell wall — no silent overflows, no confusing errors.

#### Step 1.1 — Define `Plan`, `PlanLimits`, `PlanFeature`

File: `lazyjob-core/src/billing/plan.rs`

- Implement `Plan` enum as shown in Core Types above.
- `Plan::limits()` returns a `PlanLimits` struct with all quota thresholds.
- `Plan::has_feature()` answers binary feature-gate questions.
- `Plan::from_config()` reads `config.toml`'s `[billing] plan = "free"` key via `serde::Deserialize`.

Verification: `cargo test -p lazyjob-core billing::plan` — test that `Plan::Free.limits().ralph_runs_per_month == 5`, `Plan::Enterprise.limits().jobs_per_month == u32::MAX`.

#### Step 1.2 — Monthly usage table + `QuotaService`

File: `lazyjob-core/src/billing/usage.rs`

- Apply migration 018 in `lazyjob-core/migrations/018_quota_tracking.sql`.
- `QuotaService::check(action, plan)`:
  1. Fetch current period row from `monthly_usage`.
  2. Compare count against `plan.limits().jobs_per_month` (or relevant field).
  3. If `>= 100%`: return `HardBlocked`.
  4. If `>= 80%`: return `SoftWarning { usage_pct }`.
  5. Otherwise `Allowed`.
- `QuotaService::record(action)`: `INSERT INTO monthly_usage ... ON CONFLICT DO UPDATE SET jobs_created = jobs_created + 1` (atomic upsert, no read-modify-write).

Key API:
- `rusqlite::Connection::execute("INSERT INTO monthly_usage ... ON CONFLICT(user_id, period) DO UPDATE SET ...", params![...])`

Verification: Unit test with in-memory SQLite — create 24 jobs, check `SoftWarning { usage_pct: 96 }`. Create 25th job, check `HardBlocked(JobsPerMonthExceeded)`.

#### Step 1.3 — Upsell trigger hooks

File: `lazyjob-core/src/billing/upsell.rs`

```rust
#[derive(Debug, Clone)]
pub struct UpsellTrigger {
    pub reason: UpsellReason,
    pub cta: &'static str,
    pub upgrade_url: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum UpsellReason {
    QuotaWarning { action: QuotableAction, usage_pct: u8 },
    FeatureGated(PlanFeature),
    HardBlock { action: QuotableAction },
}
```

- `upsell_message(reason)` returns a `UpsellTrigger` with a human-readable CTA string.
- TUI callers receive `QuotaResult::SoftWarning` and emit an `Event::UpsellBanner(trigger)` — rendered as a yellow status bar message. `QuotaResult::HardBlocked` emits `Event::UpsellDialog(trigger)` — rendered as a modal.

Verification: TUI test — when `QuotaResult::SoftWarning` is returned, confirm banner widget renders the CTA string in yellow.

---

### Phase 2 — Data Portability & Exit Migration (Critical MVP)

**Goal**: Every user can export all their data in a standard, documented format before cancellation. GDPR export right satisfied.

#### Step 2.1 — `ExportManifest`, archive format

File: `lazyjob-core/src/export/types.rs`

- `ExportFormat::FullPortable`: each table serialized as a JSON array to `<table>.json` and a CSV to `<table>.csv` inside a ZIP archive. A `manifest.json` at the root lists all tables, row counts, and the SHA-256 of the entire archive (computed after creation).
- `ExportFormat::SqliteSnapshot`: a verbatim copy of `lazyjob.db` inside a ZIP. Useful for manual restore; not portable to competitors.

#### Step 2.2 — `SqliteDataExporter` implementation

File: `lazyjob-core/src/export/exporter.rs`

```rust
pub struct SqliteDataExporter {
    db_path: PathBuf,
    conn: Arc<rusqlite::Connection>,
}

impl SqliteDataExporter {
    pub async fn export_full(
        &self,
        dest_path: &Path,
        format: ExportFormat,
    ) -> Result<ExportManifest, ExportError>;
}
```

- Open `dest_path` as a `zip::ZipWriter<BufWriter<File>>`.
- For each exportable table (`jobs`, `applications`, `profile_contacts`, `cover_letter_versions`, `resume_versions`, `life_sheet_experience`, `life_sheet_skills`, etc.):
  - Query all rows: `SELECT * FROM <table>`.
  - Serialize rows via `serde_json::to_string(&rows)?` → write to `<table>.json` entry.
  - Serialize same rows via `csv::Writer` → write to `<table>.csv` entry.
  - Append `ExportedTable { name, row_count, filename }` to manifest.
- Finalize ZIP, compute SHA-256 of resulting bytes via `sha2::Sha256::digest()`, encode as hex.
- Write `manifest.json` entry last (contains the hash).

Key APIs:
- `zip::ZipWriter::start_file(name, options)` — create a new file entry inside the ZIP.
- `rusqlite::Statement::query_map([], |row| ...)` — iterate over table rows.
- `sha2::Sha256::new()` + `sha2::digest::Update::update()` + `sha2::Digest::finalize()`.

Verification:
- Create a test SQLite DB with 3 jobs and 2 contacts.
- Call `export_full(dest, ExportFormat::FullPortable)`.
- Open the resulting ZIP, assert `jobs.json` parses to a 3-element array, `contacts.csv` has 2 data rows.
- Assert `manifest.sha256` matches `sha2::Sha256::digest(zip_bytes)`.

#### Step 2.3 — Export sub-selection

File: `lazyjob-core/src/export/exporter.rs`

```rust
pub enum ExportableTable {
    Jobs,
    Applications,
    Contacts,
    Resumes,
    CoverLetters,
    All,
}
```

- `export_table(table, dest_path)` exports a single table as `.json` + `.csv` without a ZIP wrapper.
- CLI subcommand: `lazyjob export --table contacts --format csv --output ./contacts.csv`.

#### Step 2.4 — Exit migration flow

File: `lazyjob-core/src/export/exit.rs`

- `ExitMigrationFlow::run()`:
  1. Prompt user to confirm data export.
  2. Call `export_full(default_export_path, FullPortable)`.
  3. Display the export path and SHA-256.
  4. Only after user confirms receipt, call `lazyjob account delete` (cloud side).
  5. Optionally call `SecurityLayer::wipe_all()` (from privacy spec) for local data.

- `retention_warning()` returns a message noting that cloud data is deleted 30 days after cancellation (GDPR-compliant notice).

Verification: Integration test — run `exit_migration_flow.run()` on a test DB, assert ZIP exists at expected path and manifest validates.

#### Step 2.5 — Cross-Spec W: Encrypted Export

File: `lazyjob-core/src/export/encrypted_export.rs`

- When exporting data from an age-encrypted DB, the export flow must:
  1. Require an unlock session (master password verified via `Session`).
  2. Decrypt the DB to a temp `Zeroizing<Vec<u8>>` buffer.
  3. Run the export against the decrypted DB.
  4. Re-encrypt the ZIP archive with the user's passphrase via `age::Encryptor`.
  5. Zeroize temp buffer on drop.
- The exported archive is itself age-encrypted so the user receives a portable encrypted bundle. They can decrypt it with their master password on any system.

Note: The SaaS sync layer (`lazyjob-sync`) never receives the raw decrypted SQLite bytes. Sync operations are applied as plaintext JSON events that are encrypted in transit via TLS. Server-at-rest encryption uses the cloud DB's native encryption (not the user's master password).

---

### Phase 3 — Onboarding & First-Run Experience (Important)

**Goal**: New users reach the "aha moment" (submit first application) within the first session.

#### Step 3.1 — `OnboardingState` singleton table

- Apply migration 019.
- `OnboardingService::load()` reads the singleton row.
- `OnboardingService::complete_step(step)` inserts the step into `completed_steps` JSON array via `json_each` + `json_insert` or a full-overwrite approach.
- `OnboardingService::is_activated()` checks if `OnboardingStep::SubmittedFirstApplication` is in `completed_steps` OR `activated_at IS NOT NULL`.

#### Step 3.2 — First-run wizard (TUI)

File: `lazyjob-tui/src/views/onboarding.rs`

```rust
pub struct OnboardingWizardView {
    current_step: WizardStep,
    state: OnboardingState,
}

pub enum WizardStep {
    Welcome,
    ConnectLlm,
    ImportLifeSheet,
    ImportContacts,      // optional skip
    AddFirstJob,
    Done,
}
```

- Wizard is shown on first launch if `onboarding_state.completed_steps` is empty and `onboarding_state.skipped == false`.
- User can press `s` to skip the wizard at any step.
- Each step renders a full-screen panel with a clear action button (`Enter` to proceed, `Esc` to skip step).
- On `WizardStep::ConnectLlm`: prompt for LLM provider selection (Anthropic/OpenAI/Ollama) and API key.
- On `WizardStep::ImportLifeSheet`: prompt for YAML path or offer to create a blank one.
- On `WizardStep::Done`: `OnboardingService::complete_step(ImportedLifeSheet)`, show a "You're ready!" panel with keyboard shortcut cheat sheet.

#### Step 3.3 — Competitor import

File: `lazyjob-core/src/onboarding/import/huntr.rs`

- Huntr JSON export format: documented at time of writing as a flat array of job objects with `company`, `jobTitle`, `status`, `appliedDate`, `notes` fields.
- `HuntrImporter::import(path)`:
  1. `serde_json::from_reader(File::open(path)?)`.
  2. Map each Huntr job object to a `JobRecord` and insert via `JobRepository::upsert_by_source(source="huntr", source_id=huntr_id)`.
  3. Map Huntr status strings to `ApplicationStage` variants via a `const HUNTR_STATUS_MAP: &[(&str, ApplicationStage)]` lookup.
  4. Return `ImportReport { imported: usize, skipped: usize, errors: Vec<String> }`.

```rust
const HUNTR_STATUS_MAP: &[(&str, ApplicationStage)] = &[
    ("wishlist", ApplicationStage::Discovered),
    ("applied", ApplicationStage::Applied),
    ("phone screen", ApplicationStage::PhoneScreen),
    ("interview", ApplicationStage::TechnicalInterview),
    ("offer", ApplicationStage::Offer),
    ("rejected", ApplicationStage::Rejected),
    ("accepted", ApplicationStage::Accepted),
];
```

- `TealImporter` follows the same pattern with Teal's export format fields.

#### Step 3.4 — Activation metric hook

- After `OnboardingService::complete_step(SubmittedFirstApplication)`, set `activated_at = datetime('now')`.
- If cloud sync enabled, include `activated_at` in the user profile sync payload so the server can track the activation funnel.

Verification: Unit test — create blank `OnboardingState`, call `complete_step(SubmittedFirstApplication)`, assert `is_activated() == true`. Integration test — import a Huntr JSON fixture, assert expected job count and stage mappings.

---

### Phase 4 — Team Workspaces (Important)

**Goal**: Teams can share a job board and see aggregate analytics. Personal data remains private unless explicitly shared.

#### Step 4.1 — Team formation flow

File: `lazyjob-core/src/team/repository.rs`

- `SqliteTeamRepository::create_team(name, owner_id)`: inserts into `teams`, inserts an `Owner` row into `team_members`.
- `InviteToken`: `HMAC-SHA256(team_id + user_email + expiry_epoch, secret)` — valid for 7 days, single-use (a `used_at` column in an `invite_tokens` table).

```sql
-- Migration 020 addendum
CREATE TABLE IF NOT EXISTS team_invite_tokens (
    token        TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    role         TEXT NOT NULL DEFAULT 'member',
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at   TEXT NOT NULL,
    used_at      TEXT
);
```

- `InviteToken::generate(team_id, email, role, secret)` → `String`.
- `InviteToken::validate(token, secret)` → `Result<(TeamId, Email, TeamRole), InviteError>`.
- Token is included in an invite email URL: `https://app.lazyjob.io/join?token=<token>`.

#### Step 4.2 — RBAC enforcement

File: `lazyjob-api/src/middleware/rbac.rs`

```rust
pub struct TeamRbacLayer {
    team_repo: Arc<dyn TeamRepository>,
}

/// axum extractor that resolves the caller's TeamRole for the team_id in the route.
pub struct CallerRole(TeamRole);
```

- Axum route handlers requiring team context call `caller_role.0.can_invite()` before accepting an invite action.
- Handlers returning member-level data check `caller_role.0.can_view_member_data()`.

#### Step 4.3 — Shared job board

- `SharedJobBoard` is a view over the `jobs` table filtered by `team_id IS NOT NULL AND team_id = ?`.
- Jobs are added to the shared board when a member explicitly presses `T` (TUI shortcut) while viewing a job, setting `team_id = <their_team_id>` on the job row.
- Shared jobs are visible to all team members regardless of who added them.
- Personal jobs (no `team_id`) are never visible to other members.

#### Step 4.4 — Team analytics view

File: `lazyjob-tui/src/views/team_analytics.rs`

- Aggregates: applications per member per stage (COUNT BY user_id, stage), response rates per member, interview conversion rate.
- Data pulled from `applications` WHERE `team_id = ?` — each member's application row stores their `user_id`.
- Rendered as a `ratatui::widgets::Table` with one row per team member.

---

### Phase 5 — Webhook & API Ecosystem (Important)

**Goal**: Third-party integrations (Zapier, n8n, custom scripts) can react to LazyJob events via signed webhooks.

#### Step 5.1 — `WebhookRegistry` and endpoint management

File: `lazyjob-api/src/webhooks/registry.rs`

```rust
pub struct WebhookRegistry {
    repo: Arc<dyn WebhookRepository>,
}

impl WebhookRegistry {
    /// Register a new webhook endpoint for the current user.
    pub async fn register(
        &self,
        user_id: UserId,
        url: String,
        events: Vec<WebhookEvent>,
        secret: Option<String>,
    ) -> Result<WebhookEndpoint, WebhookError>;

    /// Find all endpoints subscribed to a given event for a user.
    pub async fn endpoints_for_event(
        &self,
        user_id: UserId,
        event: WebhookEvent,
    ) -> Result<Vec<WebhookEndpoint>, WebhookError>;
}
```

- URL validation: `url::Url::parse()` + require `https` scheme (reject `http` in production).
- Secret defaults to `Uuid::new_v4().to_string()` if not provided.

#### Step 5.2 — HMAC-SHA256 signing

File: `lazyjob-api/src/webhooks/signing.rs`

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub fn sign_payload(secret: &str, body: &[u8], timestamp: &str) -> String {
    let message = format!("{}.{}", timestamp, std::str::from_utf8(body).unwrap_or(""));
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC can take any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Header: `X-LazyJob-Signature: t=<timestamp>,v1=<hex_hmac>`
pub fn signature_header(secret: &str, body: &[u8]) -> String {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let sig = sign_payload(secret, body, &timestamp);
    format!("t={},v1={}", timestamp, sig)
}
```

#### Step 5.3 — `WebhookDeliveryWorker`

File: `lazyjob-api/src/webhooks/delivery.rs`

```rust
pub struct WebhookDeliveryWorker {
    registry: Arc<WebhookRegistry>,
    repo: Arc<dyn WebhookRepository>,
    client: reqwest::Client,
}

impl WebhookDeliveryWorker {
    /// Spawned as a tokio background task.
    pub async fn run(&self, mut receiver: tokio::sync::mpsc::Receiver<WebhookFireRequest>) {
        while let Some(req) = receiver.recv().await {
            self.deliver(req).await;
        }
    }

    async fn deliver(&self, req: WebhookFireRequest) {
        let endpoints = self.registry.endpoints_for_event(req.user_id, req.event).await
            .unwrap_or_default();

        for endpoint in endpoints {
            let body = serde_json::to_vec(&req.payload).unwrap_or_default();
            let sig = signing::signature_header(&endpoint.secret, &body);

            let result = self.client
                .post(&endpoint.url)
                .header("Content-Type", "application/json")
                .header("X-LazyJob-Signature", sig)
                .body(body.clone())
                .timeout(Duration::from_secs(10))
                .send()
                .await;

            let (status_code, error) = match result {
                Ok(r) if r.status().is_success() => (Some(r.status().as_u16()), None),
                Ok(r) => (Some(r.status().as_u16()), Some(format!("Non-2xx: {}", r.status()))),
                Err(e) => (None, Some(e.to_string())),
            };

            self.log_delivery(&endpoint.id, &req.event, status_code, error.as_deref(), req.attempt_number).await;

            // Exponential backoff retry scheduling: 30s, 5m, 30m (3 attempts max)
            if error.is_some() && req.attempt_number < 3 {
                let delay = [30u64, 300, 1800][req.attempt_number as usize];
                self.schedule_retry(&endpoint.id, &req, delay).await;
            }
        }
    }
}
```

Key APIs:
- `reqwest::Client::post(url).timeout(Duration::from_secs(10)).send().await` — with a 10s timeout per delivery.
- `hmac::Hmac<sha2::Sha256>::new_from_slice(secret)` — HMAC-SHA256 signing.

Verification: `wiremock` integration test — register a mock server receiving POST requests, fire a `WebhookEvent::ApplicationStageChanged`, assert mock received the request with correct `X-LazyJob-Signature` header.

#### Step 5.4 — API rate limiting

File: `lazyjob-api/src/middleware/rate_limit.rs`

- Uses the `governor` crate's `RateLimiter<NotKeyed, InMemoryState, DefaultClock>`.
- Per-plan rate limits: `Plan::limits().api_calls_per_minute`.
- Returns HTTP 429 with `Retry-After` header when limit exceeded.

---

### Phase 6 — Enterprise Security & Compliance (Moderate)

**Goal**: Enterprise customers can authenticate with their SSO provider and administrators have an audit trail.

#### Step 6.1 — SAML/OIDC SSO

File: `lazyjob-api/src/auth/sso.rs`

```rust
pub trait SsoProvider: Send + Sync {
    fn initiate_login(&self, relay_state: &str) -> String;    // returns redirect URL
    async fn handle_callback(&self, params: HashMap<String, String>) -> Result<SsoIdentity, SsoError>;
}

pub struct SsoIdentity {
    pub email: String,
    pub name: Option<String>,
    pub groups: Vec<String>,
}

pub struct OidcProvider {
    client: openidconnect::Client<...>,
}

impl SsoProvider for OidcProvider {
    // Uses openidconnect crate's authorization_endpoint() + token_endpoint()
}
```

- SAML integration (`samael` crate): parse SAML assertion, extract `NameID` as email.
- Team admin configures SSO in team settings page: entity ID, SSO URL, signing certificate.
- Stored in `team_sso_config` table (separate from `teams` to keep the common path clean).

#### Step 6.2 — SCIM provisioning (stub for now)

File: `lazyjob-api/src/auth/scim.rs`

- SCIM 2.0 endpoints: `POST /scim/v2/Users`, `GET /scim/v2/Users`, `DELETE /scim/v2/Users/{id}`.
- Provisioning creates a `team_members` row with `role = 'member'`.
- Deprovisioning sets `active = false` on the user — data is retained per our retention policy.

#### Step 6.3 — Audit log

- Axum middleware `audit::AuditLayer` wraps every authenticated route.
- After handler returns, inserts one row into `audit_log` (migration 022).
- Sensitive fields (`api_key`, `password`) are never logged; the middleware filters request bodies through an `audit::sanitize_request_body()` function using a `once_cell::sync::Lazy<HashSet<&str>>` of blocked field names.

---

### Phase 7 — Billing Clarity (Moderate)

**Goal**: Users always know what they're paying for. No surprise overages.

#### Step 7.1 — Seat-based billing clarification

- Pro: per-user seat, $X/month or $Y/year (20% discount). One device per user.
- Team: per-seat, $Z/seat/month. Admin can add/remove seats; billing adjusts automatically.
- Seat count tracked in `teams.seat_count` column.
- Annual billing: config flag `billing.annual = true` in the user's cloud profile; price computed server-side.

#### Step 7.2 — Overage handling

- When a Free user exceeds monthly quota, `QuotaService::check()` returns `HardBlocked`.
- No automatic upgrade; user must manually upgrade via the in-app upsell flow or the billing portal.
- Pro/Team: quota limits are generous enough that overages are rare. If reached, `SoftWarning` is shown; limit increased by 10% for the remainder of the month as a grace buffer (`grace_multiplier = 1.1`).

---

## Key Crate APIs

| API | Purpose |
|---|---|
| `rusqlite::Connection::execute("INSERT OR REPLACE INTO monthly_usage...", params)` | Atomic quota upsert |
| `zip::ZipWriter::start_file(name, options)` | Create ZIP archive entries for export |
| `sha2::Sha256::digest(bytes)` → `[u8; 32]` | Integrity hash for export manifest |
| `csv::Writer::from_writer(buf)` → `writer.serialize(row)` | CSV serialization per row |
| `serde_json::to_writer_pretty(file, &rows)` | JSON export |
| `hmac::Hmac::<sha2::Sha256>::new_from_slice(secret)` + `.update(msg)` + `.finalize()` | Webhook HMAC signing |
| `reqwest::Client::post(url).timeout(Duration::from_secs(10)).send().await` | Webhook delivery |
| `governor::RateLimiter::direct(Quota::per_minute(NonZeroU32::new(60).unwrap()))` | API rate limiting |
| `uuid::Uuid::new_v4()` | Team/webhook ID generation |
| `age::Encryptor::with_user_passphrase(passphrase)` | Export bundle encryption (Cross-Spec W) |

---

## Error Handling

```rust
// lazyjob-core/src/billing/errors.rs
#[derive(thiserror::Error, Debug)]
pub enum QuotaError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("quota exceeded: {0:?}")]
    Exceeded(QuotaBlockReason),

    #[error("plan not loaded")]
    PlanNotLoaded,
}

// lazyjob-core/src/export/errors.rs
#[derive(thiserror::Error, Debug)]
pub enum ExportError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("session required: database is encrypted and no unlock session is active")]
    SessionRequired,
}

// lazyjob-core/src/team/errors.rs
#[derive(thiserror::Error, Debug)]
pub enum TeamError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("team not found: {0}")]
    NotFound(TeamId),

    #[error("user is not a member of this team")]
    NotMember,

    #[error("insufficient role: required {required:?}, caller has {caller:?}")]
    InsufficientRole { required: TeamRole, caller: TeamRole },

    #[error("invite token expired or invalid")]
    InvalidInviteToken,

    #[error("team is at seat capacity ({0} seats)")]
    SeatsExhausted(u32),
}

// lazyjob-api/src/webhooks/errors.rs
#[derive(thiserror::Error, Debug)]
pub enum WebhookError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("delivery failed after {0} attempts")]
    DeliveryFailed(u8),

    #[error("feature not available on current plan")]
    PlanGated,
}
```

---

## Testing Strategy

### Unit Tests

- `billing::plan` — test `Plan::limits()` for each plan variant. Test `Plan::has_feature()` exhaustively for all (`Plan`, `PlanFeature`) pairs.
- `billing::usage` — in-memory SQLite: test `QuotaService::check()` at 0%, 80%, and 100% usage. Test that `record()` is idempotent on concurrent calls (use `rusqlite` serialized mode).
- `export::exporter` — create a test SQLite DB, call `export_full`, unzip result, verify contents. Test `ExportManifest::sha256` integrity check.
- `team::repository` — create team, add 3 members with different roles, verify `can_invite()` logic, test seat limit enforcement.
- `webhooks::signing` — generate a signature, verify it using the same secret. Verify a tampered payload fails.
- `onboarding::import::huntr` — parse a fixture Huntr JSON file (included in `tests/fixtures/huntr_export.json`), assert mapped `ApplicationStage` values match expectations.

### Integration Tests

- `export` end-to-end: start a test app with 10 jobs, 5 applications, 3 contacts. Run `DataExporter::export_full(...)`. Re-read the ZIP, assert all table counts match. Assert SHA-256 in manifest is correct.
- `webhooks` delivery: use `wiremock` to mock the target endpoint. Fire a `WebhookEvent::ApplicationStageChanged`. Assert mock received exactly one POST with correct `X-LazyJob-Signature`.
- `webhooks` retry: mock returns 500 on first attempt, 200 on second. Assert `webhook_delivery_log` shows two rows — first with `error`, second with `status_code = 200`.
- Quota enforcement e2e: spin up a `Plan::Free` session. Insert 25 jobs (one over limit). Assert the 25th triggers `HardBlocked`. Assert TUI renders the upsell dialog.

### TUI Tests

- Onboarding wizard: simulate a new user first launch (empty `onboarding_state`). Assert `OnboardingWizardView` is the first rendered widget. Press `Enter` through each step. Assert `OnboardingService::is_activated()` remains false until `SubmittedFirstApplication` step.
- Upsell banner: mock `QuotaService` to return `SoftWarning { usage_pct: 85 }`. Assert status bar renders the CTA in `Style::default().fg(Color::Yellow)`.
- Upsell dialog: mock returns `HardBlocked(JobsPerMonthExceeded)`. Assert modal overlay renders with upgrade URL text visible.

---

## Open Questions

1. **Stripe integration timing**: The `stripe-rust` crate wraps the Stripe API; when does billing actually go live? Phase 4 or Phase 3? The plan gates all billing middleware on `Plan::has_feature(ApiAccess)` so no actual payment processing is needed for MVP — a manually-set plan in config suffices.
2. **Self-hosted option**: Should Team/Enterprise customers be able to self-host `lazyjob-api`? If yes, the webhook delivery worker must tolerate a local SQLite backend (not PostgreSQL). The `WebhookRepository` trait already abstracts this, but the deploy scripts need a SQLite-compatible backend variant.
3. **Export size limits**: For a power user with 10k jobs, the export archive could be 50-100 MB. Should we stream the ZIP to disk without loading everything into memory? `zip::ZipWriter` supports streaming writes, so this is implementable but not in the MVP.
4. **SAML vs OIDC for Phase 1**: SAML is required for Okta/Azure AD enterprise customers; OIDC covers Google Workspace and most modern IdPs. Prioritize OIDC for Phase 6 MVP (broader coverage), SAML in Phase 6 follow-on.
5. **Shared draft permissions** (GAP-103): Real-time collaboration and shared draft comments are deferred entirely. Phase 6+ would introduce a `shared_drafts` table with a `share_token` (expiry-gated public link) and a comment thread model. Not in scope here.
6. **Mobile companion app** (GAP-102): Read-only mobile app would use the `lazyjob-api` REST API, authenticating with the same JWT. The app is a React Native/Expo client outside this plan. The API surface is defined in `lazyjob-api` so mobile integration is possible once the API ships.

---

## Related Specs

- `specs/18-saas-migration-path.md` — parent SaaS plan, `Plan` enum, multi-tenancy
- `specs/16-privacy-security.md` — encryption, `age`, master password (Cross-Spec W)
- `specs/04-sqlite-persistence.md` — repository trait seam
- `specs/09-tui-design-keybindings.md` — upsell banner/modal rendering in TUI
- `specs/20-openapi-mvp.md` — 12-week build plan; onboarding and billing fit into Weeks 9–12
- `specs/application-workflow-actions.md` — fires `WebhookEvent::ApplicationStageChanged`
- `specs/interview-prep-agentic.md` — fires `WebhookEvent::RalphLoopCompleted`
