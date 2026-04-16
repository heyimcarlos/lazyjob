# Implementation Plan: Resume Version Management

## Status
Draft

## Related Spec
[specs/XX-resume-version-management.md](./XX-resume-version-management.md)

## Overview

Resume version management tracks the full lifecycle of every tailored resume from first generation through user edits to final submission. Without this layer, users lose track of which version was sent to which company — a critical failure mode for job seekers who manage 20+ active applications simultaneously. This plan addresses version storage, branching (create-from-existing), sent-state tracking, per-application pinning, diff visualization, cleanup policies, and PDF/DOCX export.

The plan builds on top of the `ResumeVersion` type already introduced in `profile-resume-tailoring-implementation-plan.md`. It adds a dedicated `ResumeVersionRepository` trait with a `SqliteResumeVersionRepository` implementation, a `ResumeVersionService` with full lifecycle operations, a `VersionDiffer` component using the `similar` crate operating at the bullet-point granularity, cleanup policy execution, and TUI panels for browsing and comparing versions side-by-side. The tagging subsystem (`VersionTag`) provides human-friendly labels (e.g., "sent-to-stripe", "interview-prep") independently from the machine-readable `VersionStatus`.

All version content is stored as structured JSON in a `content` TEXT column in SQLite, and the raw DOCX bytes are stored in a `docx_blob` BLOB column. The JSON content is the source of truth for diffing and TUI rendering; the DOCX blob is the source of truth for download/export. The two are kept in sync at write time by the service layer — never independently.

## Prerequisites

### Specs/Plans that must be implemented first
- `specs/profile-resume-tailoring-implementation-plan.md` — provides `ResumeVersionId`, `ResumeId`, `ResumeContent`, `ResumeVersion`, `GapReport`, `FabricationReport`, migration 008, `resume_versions` table
- `specs/07-resume-tailoring-pipeline-implementation-plan.md` — provides `TailoredResume`, `ResumeTailor::tailor()`
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, `SqlitePool`, migration infrastructure
- `specs/application-state-machine-implementation-plan.md` — provides `ApplicationId`, `ApplicationStage`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel focus system, `Action` enum

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml — additions
[dependencies]
similar            = "2"             # Section/bullet-level diffing between versions
sha2               = "0.10"          # Content hash for dedup guard
hex                = "0.4"           # Hex encoding of SHA-256 content_hash
chrono             = { version = "0.4", features = ["serde"] }
uuid               = { version = "1", features = ["v4", "serde"] }
serde_json         = "1"
sqlx               = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
thiserror          = "2"
anyhow             = "1"
tracing            = "0.1"
async-trait        = "0.1"
tokio              = { version = "1", features = ["full"] }

# lazyjob-tui/Cargo.toml — additions
ratatui            = "0.28"
crossterm          = "0.28"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All domain logic: `ResumeVersionRepository`, `ApplicationVersionLinkRepository`, `TagRepository`, `ResumeVersionService`, `VersionDiffer`, `CleanupExecutor`, all new types, SQLite migrations |
| `lazyjob-tui` | `ResumeVersionBrowserWidget`, `ResumeDiffPanel`, `ResumeVersionDetailPanel` |
| `lazyjob-cli` | `lazyjob resume versions <job-id>` subcommand, `lazyjob resume mark-sent <version-id>` subcommand, `lazyjob resume export <version-id> --format docx|pdf` |

No changes required to `lazyjob-llm`. `lazyjob-ralph` already creates versions via `ResumeVersionService::create_from_tailoring()` (defined in the tailoring plan); this plan adds the lifecycle management layer on top.

### Core Types

```rust
// lazyjob-core/src/resume/version_types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Newtype ID — parse, don't validate (rust-patterns.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct ResumeVersionId(pub Uuid);

impl ResumeVersionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Parent resume entity (the base, job-independent profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct ResumeId(pub Uuid);

/// Lifecycle status of a resume version.
///
/// `Sent` versions are legal records and MUST NOT be deleted or archived
/// by automated cleanup policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum VersionStatus {
    Draft,
    Sent,
    Archived,
}

/// How this version came to exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionSource {
    UserEdited,
    RalphTailoring { loop_id: Uuid },
    Import,
    BranchedFrom { parent_version_id: ResumeVersionId },
}

/// Channel through which the resume was submitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionChannel {
    Email { to: String },
    CompanyPortal,
    LinkedIn,
    Greenhouse,
    Lever,
    Other { label: String },
}

/// Immutable record written when a resume version is submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationVersionLink {
    pub application_id: ApplicationId,
    pub resume_version_id: ResumeVersionId,
    pub sent_at: DateTime<Utc>,
    pub sent_via: SubmissionChannel,
}

/// User-applied label for visual organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionTag {
    pub id: TagId,
    pub name: String,
    /// CSS-style hex color, e.g. "#FF6B6B". Used for TUI span styling.
    pub color_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct TagId(pub Uuid);

impl TagId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Policy for automatic pruning of old versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanupPolicy {
    /// Manual management only.
    KeepAll,
    /// Keep the N most recently created versions per resume (non-Sent only).
    KeepLastN { count: u32 },
    /// Archive Draft versions older than `days` days.
    KeepSentVersions { max_draft_age_days: u32 },
    /// After offer accepted, archive all Draft versions for that resume.
    AutoPruneAfterHiring,
}

/// The diff between two resume versions at the section/bullet granularity.
#[derive(Debug, Clone)]
pub struct ResumeDiff {
    pub from_version: ResumeVersionId,
    pub to_version: ResumeVersionId,
    pub section_diffs: Vec<SectionDiff>,
    pub keywords_added: Vec<String>,
    pub keywords_removed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SectionDiff {
    pub section_name: String,
    pub bullet_changes: Vec<BulletChange>,
}

#[derive(Debug, Clone)]
pub struct BulletChange {
    pub index: usize,
    pub change_type: ChangeType,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Unchanged,
}

/// Extended version record with full lifecycle metadata.
/// `content` and `docx_blob` are stored separately in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersionRecord {
    pub id: ResumeVersionId,
    pub resume_id: ResumeId,
    pub name: String,
    pub version_number: u32,
    /// Structured JSON content (source of truth for diffing/TUI).
    pub content: ResumeContent,
    /// SHA-256 of `serde_json::to_string(&content)` in hex.
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: VersionSource,
    pub status: VersionStatus,
    pub sent_at: Option<DateTime<Utc>>,
    pub sent_via: Option<SubmissionChannel>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/resume/repository.rs

use async_trait::async_trait;

#[async_trait]
pub trait ResumeVersionRepository: Send + Sync {
    async fn insert(&self, version: &ResumeVersionRecord) -> Result<(), VersionRepoError>;
    async fn update(&self, version: &ResumeVersionRecord) -> Result<(), VersionRepoError>;
    async fn get(&self, id: ResumeVersionId) -> Result<ResumeVersionRecord, VersionRepoError>;
    async fn list_for_resume(
        &self,
        resume_id: ResumeId,
    ) -> Result<Vec<ResumeVersionRecord>, VersionRepoError>;
    async fn list_for_resume_asc(
        &self,
        resume_id: ResumeId,
    ) -> Result<Vec<ResumeVersionRecord>, VersionRepoError>;
    async fn get_latest_version(
        &self,
        resume_id: ResumeId,
    ) -> Result<Option<ResumeVersionRecord>, VersionRepoError>;
    async fn get_docx_blob(&self, id: ResumeVersionId) -> Result<Vec<u8>, VersionRepoError>;
    async fn update_docx_blob(
        &self,
        id: ResumeVersionId,
        blob: &[u8],
    ) -> Result<(), VersionRepoError>;
    async fn archive(&self, id: ResumeVersionId) -> Result<(), VersionRepoError>;
    async fn delete(&self, id: ResumeVersionId) -> Result<(), VersionRepoError>;
    async fn count_for_resume(&self, resume_id: ResumeId) -> Result<u32, VersionRepoError>;
    /// Returns versions whose linked application reached Accepted stage.
    async fn list_sent_for_accepted_applications(&self) -> Result<Vec<ResumeVersionRecord>, VersionRepoError>;
    /// Next auto-increment version_number for a given resume.
    async fn next_version_number(&self, resume_id: ResumeId) -> Result<u32, VersionRepoError>;
}

#[async_trait]
pub trait ApplicationVersionLinkRepository: Send + Sync {
    async fn insert(&self, link: &ApplicationVersionLink) -> Result<(), VersionRepoError>;
    async fn list_for_application(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<ApplicationVersionLink>, VersionRepoError>;
    async fn list_for_version(
        &self,
        version_id: ResumeVersionId,
    ) -> Result<Vec<ApplicationVersionLink>, VersionRepoError>;
    async fn get_pinned_version(
        &self,
        application_id: ApplicationId,
    ) -> Result<Option<ResumeVersionId>, VersionRepoError>;
    async fn pin_version(
        &self,
        application_id: ApplicationId,
        version_id: ResumeVersionId,
    ) -> Result<(), VersionRepoError>;
}

#[async_trait]
pub trait VersionTagRepository: Send + Sync {
    async fn get_or_create(&self, name: &str, color_hex: &str) -> Result<VersionTag, VersionRepoError>;
    async fn add_tag(&self, version_id: ResumeVersionId, tag_id: TagId) -> Result<(), VersionRepoError>;
    async fn remove_tag(&self, version_id: ResumeVersionId, tag_id: TagId) -> Result<(), VersionRepoError>;
    async fn list_tags_for_version(&self, version_id: ResumeVersionId) -> Result<Vec<VersionTag>, VersionRepoError>;
    async fn list_all_tags(&self) -> Result<Vec<VersionTag>, VersionRepoError>;
    async fn delete_unused_tags(&self) -> Result<u32, VersionRepoError>;
}
```

### SQLite Schema

```sql
-- Migration: lazyjob-core/migrations/011_resume_version_management.sql
--
-- The resume_versions table was introduced in migration 008 by the tailoring plan
-- with core columns. This migration adds lifecycle management columns and creates
-- the linking/tagging tables.

-- Add version lifecycle columns
ALTER TABLE resume_versions ADD COLUMN version_number   INTEGER NOT NULL DEFAULT 1;
ALTER TABLE resume_versions ADD COLUMN created_by       TEXT    NOT NULL DEFAULT 'ralph_tailoring';
ALTER TABLE resume_versions ADD COLUMN status           TEXT    NOT NULL DEFAULT 'draft';
ALTER TABLE resume_versions ADD COLUMN sent_at          TEXT;
ALTER TABLE resume_versions ADD COLUMN sent_via         TEXT;   -- JSON SubmissionChannel

-- Fast lookup: all versions for a resume ordered by version_number
CREATE INDEX IF NOT EXISTS idx_rv_resume_id_vnum
    ON resume_versions(resume_id, version_number DESC);

-- Active versions only (default view excludes archived)
CREATE INDEX IF NOT EXISTS idx_rv_status
    ON resume_versions(status)
    WHERE status != 'archived';

-- Application ↔ resume version explicit link table.
-- Tracks which resume version was actually submitted for each application,
-- and which version is currently pinned for future edits.
CREATE TABLE IF NOT EXISTS application_resume_version_links (
    application_id        TEXT    NOT NULL
        REFERENCES applications(id) ON DELETE CASCADE,
    resume_version_id     TEXT    NOT NULL
        REFERENCES resume_versions(id) ON DELETE CASCADE,
    sent_at               TEXT    NOT NULL,
    sent_via              TEXT    NOT NULL DEFAULT 'other',   -- JSON SubmissionChannel
    is_pinned             INTEGER NOT NULL DEFAULT 0,         -- 1 = pinned for this app
    created_at            TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (application_id, resume_version_id)
);

CREATE INDEX IF NOT EXISTS idx_arvl_application_id
    ON application_resume_version_links(application_id);

CREATE INDEX IF NOT EXISTS idx_arvl_version_id
    ON application_resume_version_links(resume_version_id);

-- Partial index for pinned version lookup
CREATE UNIQUE INDEX IF NOT EXISTS idx_arvl_pinned
    ON application_resume_version_links(application_id)
    WHERE is_pinned = 1;

-- User-defined tags for version labelling
CREATE TABLE IF NOT EXISTS resume_version_tags (
    id          TEXT PRIMARY KEY,       -- UUID
    name        TEXT NOT NULL UNIQUE,
    color_hex   TEXT NOT NULL DEFAULT '#AAAAAA',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Many-to-many: versions ↔ tags
CREATE TABLE IF NOT EXISTS resume_version_tag_assignments (
    version_id  TEXT NOT NULL REFERENCES resume_versions(id) ON DELETE CASCADE,
    tag_id      TEXT NOT NULL REFERENCES resume_version_tags(id) ON DELETE CASCADE,
    PRIMARY KEY (version_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_rvta_version_id
    ON resume_version_tag_assignments(version_id);
```

### Module Structure

```
lazyjob-core/
  src/
    resume/
      mod.rs                         -- re-exports: ResumeVersionRecord, ResumeVersionService, etc.
      types.rs                       -- existing from tailoring plan: ResumeContent, GapReport, etc.
      version_types.rs               -- NEW: VersionStatus, VersionSource, SubmissionChannel,
                                     --      ApplicationVersionLink, VersionTag, CleanupPolicy,
                                     --      ResumeDiff, SectionDiff, BulletChange, ChangeType
      repository.rs                  -- NEW: ResumeVersionRepository, ApplicationVersionLinkRepository,
                                     --      VersionTagRepository traits
      sqlite_version_repo.rs         -- NEW: SqliteResumeVersionRepository impl
      sqlite_link_repo.rs            -- NEW: SqliteApplicationVersionLinkRepository impl
      sqlite_tag_repo.rs             -- NEW: SqliteVersionTagRepository impl
      version_service.rs             -- NEW: ResumeVersionService (create_from, mark_sent,
                                     --      pin_version, tag_version, apply_cleanup)
      differ.rs                      -- NEW: ResumeDiffer using `similar`
      cleanup.rs                     -- NEW: CleanupExecutor
      errors.rs                      -- NEW: VersionRepoError, VersionServiceError

lazyjob-tui/
  src/
    views/
      resume/
        version_browser.rs           -- NEW: ResumeVersionBrowserWidget
        version_diff_panel.rs        -- NEW: ResumeDiffPanel
        version_detail_panel.rs      -- NEW: ResumeVersionDetailPanel (metadata + tags)
        mod.rs                       -- re-exports
```

---

## Implementation Phases

### Phase 1 — Core Storage and Repository (MVP)

#### Step 1.1 — Apply Migration 011

File: `lazyjob-core/migrations/011_resume_version_management.sql`

Apply the DDL above via `sqlx::migrate!()` in `Database::connect()`. Add `version_number`, `created_by`, `status`, `sent_at`, `sent_via` columns to `resume_versions`; create `application_resume_version_links`, `resume_version_tags`, and `resume_version_tag_assignments` tables.

Verification: `cargo sqlx prepare --check` passes; `sqlx migrate run` against a test database exits 0.

---

#### Step 1.2 — Implement Content Hash

File: `lazyjob-core/src/resume/version_types.rs`

```rust
use sha2::{Digest, Sha256};
use hex;

impl ResumeVersionRecord {
    /// Compute SHA-256 hex of canonical JSON content.
    /// Must be called before every `insert()` or `update()` to keep
    /// `content_hash` consistent with the `content` column.
    pub fn compute_content_hash(content: &ResumeContent) -> String {
        let canonical = serde_json::to_string(content)
            .expect("ResumeContent is always serializable");
        let digest = Sha256::digest(canonical.as_bytes());
        hex::encode(digest)
    }
}
```

Key APIs:
- `sha2::Sha256::digest(&[u8])` — returns a `GenericArray<u8, U32>`
- `hex::encode(impl AsRef<[u8]>)` — lowercase hex string

Verification: unit test asserts `compute_content_hash` is deterministic and changes when any field in `ResumeContent` changes.

---

#### Step 1.3 — Implement `SqliteResumeVersionRepository`

File: `lazyjob-core/src/resume/sqlite_version_repo.rs`

```rust
pub struct SqliteResumeVersionRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteResumeVersionRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl ResumeVersionRepository for SqliteResumeVersionRepository {
    async fn insert(&self, version: &ResumeVersionRecord) -> Result<(), VersionRepoError> {
        let id = version.id.0.to_string();
        let resume_id = version.resume_id.0.to_string();
        let content_json = serde_json::to_string(&version.content)
            .map_err(VersionRepoError::Serialization)?;
        let created_by_json = serde_json::to_string(&version.created_by)
            .map_err(VersionRepoError::Serialization)?;
        let sent_via_json = version.sent_via.as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(VersionRepoError::Serialization)?;
        let status_str = match version.status {
            VersionStatus::Draft    => "draft",
            VersionStatus::Sent     => "sent",
            VersionStatus::Archived => "archived",
        };

        sqlx::query!(
            r#"
            INSERT INTO resume_versions
                (id, resume_id, name, version_number, content, content_hash,
                 created_at, updated_at, created_by, status, sent_at, sent_via)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id, resume_id, version.name, version.version_number,
            content_json, version.content_hash,
            version.created_at, version.updated_at,
            created_by_json, status_str,
            version.sent_at, sent_via_json,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(())
    }

    async fn get(&self, id: ResumeVersionId) -> Result<ResumeVersionRecord, VersionRepoError> {
        let id_str = id.0.to_string();
        let row = sqlx::query!(
            "SELECT id, resume_id, name, version_number, content, content_hash,
                    created_at, updated_at, created_by, status, sent_at, sent_via
             FROM resume_versions WHERE id = ?",
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?
        .ok_or(VersionRepoError::NotFound(id))?;

        Self::map_row(row)
    }

    async fn archive(&self, id: ResumeVersionId) -> Result<(), VersionRepoError> {
        let id_str = id.0.to_string();
        let updated_at = Utc::now();
        sqlx::query!(
            "UPDATE resume_versions SET status = 'archived', updated_at = ? WHERE id = ?",
            updated_at, id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;
        Ok(())
    }

    async fn next_version_number(&self, resume_id: ResumeId) -> Result<u32, VersionRepoError> {
        let rid = resume_id.0.to_string();
        let row = sqlx::query!(
            "SELECT COALESCE(MAX(version_number), 0) AS max_vnum FROM resume_versions WHERE resume_id = ?",
            rid,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(row.max_vnum as u32 + 1)
    }

    async fn get_docx_blob(&self, id: ResumeVersionId) -> Result<Vec<u8>, VersionRepoError> {
        let id_str = id.0.to_string();
        let row = sqlx::query!(
            "SELECT docx_blob FROM resume_versions WHERE id = ?",
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?
        .ok_or(VersionRepoError::NotFound(id))?;

        row.docx_blob.ok_or(VersionRepoError::NoBlobStored(id))
    }

    // ... remaining methods follow the same pattern
}
```

Key APIs:
- `sqlx::query!()` macro — compile-time verified SQL
- `sqlx::SqlitePool::execute()` / `fetch_optional()` / `fetch_all()` — async SQLite ops
- `serde_json::to_string()` / `from_str()` — serialize `ResumeContent`, `VersionSource`, `SubmissionChannel` as TEXT columns
- `chrono::Utc::now()` — timestamps

Verification: `#[sqlx::test(migrations = "migrations")]` test inserts a version and retrieves it, asserting all fields round-trip correctly, including the JSON-serialized `created_by` and `sent_via`.

---

#### Step 1.4 — Implement `SqliteApplicationVersionLinkRepository`

File: `lazyjob-core/src/resume/sqlite_link_repo.rs`

```rust
pub struct SqliteApplicationVersionLinkRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait]
impl ApplicationVersionLinkRepository for SqliteApplicationVersionLinkRepository {
    async fn insert(&self, link: &ApplicationVersionLink) -> Result<(), VersionRepoError> {
        let app_id = link.application_id.0.to_string();
        let version_id = link.resume_version_id.0.to_string();
        let sent_via_json = serde_json::to_string(&link.sent_via)
            .map_err(VersionRepoError::Serialization)?;

        sqlx::query!(
            r#"
            INSERT INTO application_resume_version_links
                (application_id, resume_version_id, sent_at, sent_via)
            VALUES (?, ?, ?, ?)
            "#,
            app_id, version_id, link.sent_at, sent_via_json,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(())
    }

    async fn pin_version(
        &self,
        application_id: ApplicationId,
        version_id: ResumeVersionId,
    ) -> Result<(), VersionRepoError> {
        let app_id = application_id.0.to_string();
        let vid = version_id.0.to_string();

        // Atomically: unpin all others, then pin the target.
        // Uses a single transaction for consistency.
        let mut tx = self.pool.begin().await.map_err(VersionRepoError::Database)?;

        sqlx::query!(
            "UPDATE application_resume_version_links SET is_pinned = 0 WHERE application_id = ?",
            app_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(VersionRepoError::Database)?;

        sqlx::query!(
            r#"
            UPDATE application_resume_version_links
            SET is_pinned = 1
            WHERE application_id = ? AND resume_version_id = ?
            "#,
            app_id, vid,
        )
        .execute(&mut *tx)
        .await
        .map_err(VersionRepoError::Database)?;

        tx.commit().await.map_err(VersionRepoError::Database)?;
        Ok(())
    }

    async fn get_pinned_version(
        &self,
        application_id: ApplicationId,
    ) -> Result<Option<ResumeVersionId>, VersionRepoError> {
        let app_id = application_id.0.to_string();
        let row = sqlx::query!(
            "SELECT resume_version_id FROM application_resume_version_links
             WHERE application_id = ? AND is_pinned = 1",
            app_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(row.map(|r| ResumeVersionId(Uuid::parse_str(&r.resume_version_id).unwrap())))
    }
}
```

Key design: `pin_version()` uses a transaction to atomically unpin all others before pinning the target — the partial index `WHERE is_pinned = 1` guarantees at most one pinned version per application at the database level.

Verification: `#[sqlx::test]` test: insert two links, pin the second, assert `get_pinned_version` returns the second, insert a third and pin it, assert `get_pinned_version` now returns the third.

---

#### Step 1.5 — Implement `SqliteVersionTagRepository`

File: `lazyjob-core/src/resume/sqlite_tag_repo.rs`

```rust
pub struct SqliteVersionTagRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait]
impl VersionTagRepository for SqliteVersionTagRepository {
    async fn get_or_create(&self, name: &str, color_hex: &str) -> Result<VersionTag, VersionRepoError> {
        let id = TagId::new().0.to_string();

        // Use INSERT OR IGNORE + SELECT to avoid a TOCTOU race.
        sqlx::query!(
            "INSERT OR IGNORE INTO resume_version_tags (id, name, color_hex) VALUES (?, ?, ?)",
            id, name, color_hex,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        let row = sqlx::query!(
            "SELECT id, name, color_hex FROM resume_version_tags WHERE name = ?",
            name,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(VersionTag {
            id: TagId(Uuid::parse_str(&row.id).unwrap()),
            name: row.name,
            color_hex: row.color_hex,
        })
    }

    async fn add_tag(&self, version_id: ResumeVersionId, tag_id: TagId) -> Result<(), VersionRepoError> {
        let vid = version_id.0.to_string();
        let tid = tag_id.0.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO resume_version_tag_assignments (version_id, tag_id) VALUES (?, ?)",
            vid, tid,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;
        Ok(())
    }

    async fn delete_unused_tags(&self) -> Result<u32, VersionRepoError> {
        let result = sqlx::query!(
            r#"
            DELETE FROM resume_version_tags
            WHERE id NOT IN (
                SELECT DISTINCT tag_id FROM resume_version_tag_assignments
            )
            "#
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;
        Ok(result.rows_affected() as u32)
    }
}
```

Verification: test creates two tags, assigns one to a version, calls `delete_unused_tags`, asserts only the assigned tag survives.

---

### Phase 2 — Version Service Operations

#### Step 2.1 — Implement `ResumeVersionService`

File: `lazyjob-core/src/resume/version_service.rs`

```rust
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct ResumeVersionService {
    version_repo:  Arc<dyn ResumeVersionRepository>,
    link_repo:     Arc<dyn ApplicationVersionLinkRepository>,
    tag_repo:      Arc<dyn VersionTagRepository>,
    event_tx:      broadcast::Sender<VersionEvent>,
}

/// Events broadcast to the TUI for re-render.
#[derive(Debug, Clone)]
pub enum VersionEvent {
    VersionCreated   { id: ResumeVersionId },
    VersionSent      { id: ResumeVersionId, channel: SubmissionChannel },
    VersionPinned    { application_id: ApplicationId, version_id: ResumeVersionId },
    VersionArchived  { id: ResumeVersionId },
    TagAssigned      { version_id: ResumeVersionId, tag: VersionTag },
    CleanupCompleted { pruned: u32 },
}
```

**`create_from()` — branch a new Draft version from an existing one**

```rust
impl ResumeVersionService {
    /// Create a new Draft version that starts as a copy of `source_version_id`.
    /// `VersionSource::BranchedFrom` records the lineage.
    pub async fn create_from(
        &self,
        source_version_id: ResumeVersionId,
        name: String,
    ) -> Result<ResumeVersionId, VersionServiceError> {
        let source = self.version_repo.get(source_version_id).await?;
        let content_hash = ResumeVersionRecord::compute_content_hash(&source.content);
        let version_number = self.version_repo.next_version_number(source.resume_id).await?;

        let new_version = ResumeVersionRecord {
            id: ResumeVersionId::new(),
            resume_id: source.resume_id,
            name,
            version_number,
            content: source.content.clone(),
            content_hash,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::BranchedFrom { parent_version_id: source_version_id },
            status: VersionStatus::Draft,
            sent_at: None,
            sent_via: None,
        };

        self.version_repo.insert(&new_version).await?;
        let _ = self.event_tx.send(VersionEvent::VersionCreated { id: new_version.id });
        Ok(new_version.id)
    }
```

**`create_from_tailoring()` — create a version from a Ralph tailoring result**

```rust
    /// Create a new version from a Ralph tailoring loop result.
    /// This is the primary entry point called by the tailoring pipeline.
    pub async fn create_from_tailoring(
        &self,
        resume_id: ResumeId,
        tailoring_result: TailoredResume,
    ) -> Result<ResumeVersionId, VersionServiceError> {
        let content_hash = ResumeVersionRecord::compute_content_hash(&tailoring_result.content);
        let version_number = self.version_repo.next_version_number(resume_id).await?;
        let name = format!("Tailored v{}", version_number);

        let new_version = ResumeVersionRecord {
            id: ResumeVersionId::new(),
            resume_id,
            name,
            version_number,
            content: tailoring_result.content,
            content_hash,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::RalphTailoring { loop_id: tailoring_result.loop_id },
            status: VersionStatus::Draft,
            sent_at: None,
            sent_via: None,
        };

        self.version_repo.insert(&new_version).await?;
        // DOCX blob written separately after generation step
        self.version_repo.update_docx_blob(new_version.id, &tailoring_result.docx_bytes).await?;

        let _ = self.event_tx.send(VersionEvent::VersionCreated { id: new_version.id });
        Ok(new_version.id)
    }
```

**`mark_sent()` — atomic sent-state transition with application link**

```rust
    /// Mark a version as sent, record the submission, and optionally link to an application.
    ///
    /// If `application_id` is provided, the version is linked to that application and
    /// set as the pinned version. Non-fatal if the version is already Sent
    /// (re-sending to multiple recipients is allowed).
    pub async fn mark_sent(
        &self,
        version_id: ResumeVersionId,
        channel: SubmissionChannel,
        application_id: Option<ApplicationId>,
    ) -> Result<(), VersionServiceError> {
        let mut version = self.version_repo.get(version_id).await?;
        let now = Utc::now();

        version.status = VersionStatus::Sent;
        version.sent_via = Some(channel.clone());
        version.sent_at = Some(now);
        version.updated_at = now;
        self.version_repo.update(&version).await?;

        if let Some(app_id) = application_id {
            let link = ApplicationVersionLink {
                application_id: app_id,
                resume_version_id: version_id,
                sent_at: now,
                sent_via: channel.clone(),
            };
            self.link_repo.insert(&link).await?;
            self.link_repo.pin_version(app_id, version_id).await?;
            let _ = self.event_tx.send(VersionEvent::VersionPinned {
                application_id: app_id,
                version_id,
            });
        }

        let _ = self.event_tx.send(VersionEvent::VersionSent { id: version_id, channel });
        Ok(())
    }
```

**`pin_version_for_application()` — pin without marking sent**

```rust
    /// Pin a version to an application without marking it sent.
    /// Useful when the user wants to track which version they're editing
    /// before submitting.
    pub async fn pin_version_for_application(
        &self,
        version_id: ResumeVersionId,
        application_id: ApplicationId,
    ) -> Result<(), VersionServiceError> {
        self.link_repo.pin_version(application_id, version_id).await?;
        let _ = self.event_tx.send(VersionEvent::VersionPinned { application_id, version_id });
        Ok(())
    }
```

**`tag_version()` — apply a user label**

```rust
    /// Apply a tag (get-or-create) to a version.
    pub async fn tag_version(
        &self,
        version_id: ResumeVersionId,
        tag_name: &str,
        color_hex: &str,
    ) -> Result<VersionTag, VersionServiceError> {
        let tag = self.tag_repo.get_or_create(tag_name, color_hex).await?;
        self.tag_repo.add_tag(version_id, tag.id).await?;
        let _ = self.event_tx.send(VersionEvent::TagAssigned {
            version_id,
            tag: tag.clone(),
        });
        Ok(tag)
    }
```

**`archive()` — guard against archiving Sent versions**

```rust
    pub async fn archive(&self, id: ResumeVersionId) -> Result<(), VersionServiceError> {
        let version = self.version_repo.get(id).await?;
        if version.status == VersionStatus::Sent {
            return Err(VersionServiceError::CannotArchiveSent(id));
        }
        self.version_repo.archive(id).await?;
        let _ = self.event_tx.send(VersionEvent::VersionArchived { id });
        Ok(())
    }
}
```

Verification: unit tests for each method using `Arc<MockResumeVersionRepository>` (backed by `Arc<Mutex<HashMap<ResumeVersionId, ResumeVersionRecord>>>`).

---

#### Step 2.2 — Implement `CleanupExecutor`

File: `lazyjob-core/src/resume/cleanup.rs`

```rust
pub struct CleanupExecutor {
    version_repo: Arc<dyn ResumeVersionRepository>,
}

impl CleanupExecutor {
    pub async fn apply(
        &self,
        policy: &CleanupPolicy,
        resume_id: ResumeId,
    ) -> Result<u32, VersionServiceError> {
        match policy {
            CleanupPolicy::KeepAll => Ok(0),

            CleanupPolicy::KeepLastN { count } => {
                self.prune_to_count(resume_id, *count).await
            }

            CleanupPolicy::KeepSentVersions { max_draft_age_days } => {
                self.archive_old_drafts(resume_id, *max_draft_age_days as i64).await
            }

            CleanupPolicy::AutoPruneAfterHiring => {
                self.archive_all_drafts(resume_id).await
            }
        }
    }

    async fn prune_to_count(
        &self,
        resume_id: ResumeId,
        keep: u32,
    ) -> Result<u32, VersionServiceError> {
        // Fetch all non-archived versions ordered oldest first (created_at ASC).
        let all = self.version_repo.list_for_resume_asc(resume_id).await?
            .into_iter()
            .filter(|v| v.status != VersionStatus::Archived)
            .collect::<Vec<_>>();

        let total = all.len() as u32;
        if total <= keep {
            return Ok(0);
        }

        let to_archive = &all[..(total - keep) as usize];
        let mut pruned = 0u32;
        for v in to_archive {
            // Sent versions are legal records — never archive them.
            if v.status != VersionStatus::Sent {
                self.version_repo.archive(v.id).await?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    async fn archive_old_drafts(
        &self,
        resume_id: ResumeId,
        max_days: i64,
    ) -> Result<u32, VersionServiceError> {
        let cutoff = Utc::now() - chrono::Duration::days(max_days);
        let all = self.version_repo.list_for_resume_asc(resume_id).await?;
        let mut pruned = 0u32;
        for v in all {
            if v.status == VersionStatus::Draft && v.created_at < cutoff {
                self.version_repo.archive(v.id).await?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    async fn archive_all_drafts(&self, resume_id: ResumeId) -> Result<u32, VersionServiceError> {
        let all = self.version_repo.list_for_resume_asc(resume_id).await?;
        let mut pruned = 0u32;
        for v in all {
            if v.status == VersionStatus::Draft {
                self.version_repo.archive(v.id).await?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }
}
```

Verification: unit tests for `KeepLastN` assert exactly `count` non-archived versions survive; Sent versions are never archived regardless of policy.

---

### Phase 3 — Diff Engine

#### Step 3.1 — Implement `ResumeDiffer`

File: `lazyjob-core/src/resume/differ.rs`

The `ResumeContent` type (from the tailoring plan) is a structured document with named sections, each section containing a `Vec<String>` of bullet points. The differ operates at two levels:
1. **Section level**: detect added/removed sections by name.
2. **Bullet level**: for sections present in both versions, run `similar::TextDiff` on the bullet arrays.

```rust
use similar::{ChangeTag, TextDiff};
use once_cell::sync::Lazy;
use regex::Regex;

/// Keywords extractor: finds technical terms for the keyword diff.
static KEYWORD_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b[A-Z][a-zA-Z0-9+#./]+\b").expect("valid regex")
});

pub struct ResumeDiffer;

impl ResumeDiffer {
    /// Compute a bullet-level diff between two resume versions.
    ///
    /// Sections are matched by exact `section_name` equality.
    /// Sections present in `from` but not in `to` appear as fully Removed.
    /// Sections present in `to` but not in `from` appear as fully Added.
    pub fn diff(from: &ResumeVersionRecord, to: &ResumeVersionRecord) -> ResumeDiff {
        let from_sections = Self::section_map(&from.content);
        let to_sections   = Self::section_map(&to.content);

        let all_names: Vec<&String> = from_sections.keys()
            .chain(to_sections.keys())
            .collect::<std::collections::BTreeSet<_>>()  // sorted, deduplicated
            .into_iter()
            .collect();

        let mut section_diffs = Vec::new();

        for name in all_names {
            let from_bullets = from_sections.get(name).map(|v| v.as_slice()).unwrap_or(&[]);
            let to_bullets   = to_sections.get(name).map(|v| v.as_slice()).unwrap_or(&[]);

            let from_strs: Vec<&str> = from_bullets.iter().map(String::as_str).collect();
            let to_strs:   Vec<&str> = to_bullets.iter().map(String::as_str).collect();

            let text_diff = TextDiff::from_slices(&from_strs, &to_strs);
            let mut bullet_changes = Vec::new();
            let mut index = 0;

            for change in text_diff.iter_all_changes() {
                match change.tag() {
                    ChangeTag::Equal => {
                        bullet_changes.push(BulletChange {
                            index,
                            change_type: ChangeType::Unchanged,
                            before: Some(change.value().to_string()),
                            after:  Some(change.value().to_string()),
                        });
                        index += 1;
                    }
                    ChangeTag::Delete => {
                        bullet_changes.push(BulletChange {
                            index,
                            change_type: ChangeType::Removed,
                            before: Some(change.value().to_string()),
                            after:  None,
                        });
                        index += 1;
                    }
                    ChangeTag::Insert => {
                        // Collapse adjacent Delete+Insert into Modified
                        if let Some(last) = bullet_changes.last_mut() {
                            if last.change_type == ChangeType::Removed && last.after.is_none() {
                                last.change_type = ChangeType::Modified;
                                last.after = Some(change.value().to_string());
                                continue;
                            }
                        }
                        bullet_changes.push(BulletChange {
                            index,
                            change_type: ChangeType::Added,
                            before: None,
                            after:  Some(change.value().to_string()),
                        });
                        index += 1;
                    }
                }
            }

            section_diffs.push(SectionDiff {
                section_name: name.clone(),
                bullet_changes,
            });
        }

        // Keyword diff: extract from full text of each version
        let from_keywords = Self::extract_keywords(&from.content);
        let to_keywords   = Self::extract_keywords(&to.content);
        let keywords_added: Vec<String> = to_keywords.difference(&from_keywords).cloned().collect();
        let keywords_removed: Vec<String> = from_keywords.difference(&to_keywords).cloned().collect();

        ResumeDiff {
            from_version: from.id,
            to_version:   to.id,
            section_diffs,
            keywords_added,
            keywords_removed,
        }
    }

    fn section_map(content: &ResumeContent) -> std::collections::HashMap<String, Vec<String>> {
        content.sections.iter()
            .map(|s| (s.name.clone(), s.bullets.clone()))
            .collect()
    }

    fn extract_keywords(content: &ResumeContent) -> std::collections::HashSet<String> {
        let full_text: String = content.sections.iter()
            .flat_map(|s| s.bullets.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        KEYWORD_PATTERN.find_iter(&full_text)
            .map(|m| m.as_str().to_string())
            .collect()
    }
}
```

Key APIs:
- `similar::TextDiff::from_slices(&[&str], &[&str])` — element-level diff (bullets = elements)
- `similar::Change::tag()` → `ChangeTag::{Equal, Delete, Insert}` — classify each bullet
- `similar::Change::value()` → `&str` — bullet text
- `once_cell::sync::Lazy<Regex>` — compile keyword regex once
- `std::collections::BTreeSet` — sorted, deduplicated section name union

Verification: unit tests:
- `diff_unchanged`: identical versions produce all `ChangeType::Unchanged` bullets, empty keyword lists
- `diff_single_modified`: change one bullet → one `Modified` entry with correct before/after
- `diff_section_added`: add a new section to `to` → `Added` entries for all bullets
- `diff_keyword_tracking`: add a bullet containing "Kubernetes" → `keywords_added` contains "Kubernetes"

---

### Phase 4 — TUI Version Browser and Diff Panel

#### Step 4.1 — Implement `ResumeVersionBrowserWidget`

File: `lazyjob-tui/src/views/resume/version_browser.rs`

The version browser is a full-screen overlay (triggered by `v` in the resume view) using `ratatui::widgets::Clear` to erase the background before rendering.

```rust
use std::collections::HashMap;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget},
};

pub struct ResumeVersionBrowserWidget {
    pub versions: Vec<ResumeVersionRecord>,
    pub tags_by_version: HashMap<ResumeVersionId, Vec<VersionTag>>,
    pub list_state: ListState,
    pub pinned_version: Option<ResumeVersionId>,
}

impl ResumeVersionBrowserWidget {
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // Erase background before overlay
        Clear.render(area, buf);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(3),  // keybind hint bar
            ])
            .split(area);

        // Version list
        let items: Vec<ListItem> = self.versions.iter().map(|v| {
            let status_span = match v.status {
                VersionStatus::Draft    => Span::styled("[DRAFT]",    Style::default().fg(Color::Yellow)),
                VersionStatus::Sent     => Span::styled("[SENT]",     Style::default().fg(Color::Green)),
                VersionStatus::Archived => Span::styled("[ARCHIVED]", Style::default().fg(Color::DarkGray)),
            };
            let pin_span = if Some(v.id) == self.pinned_version {
                Span::styled(" [PIN]", Style::default().fg(Color::Cyan))
            } else {
                Span::raw("")
            };
            let tags_span = {
                let tags = self.tags_by_version.get(&v.id).cloned().unwrap_or_default();
                let tag_text: String = tags.iter().map(|t| format!(" #{}", t.name)).collect();
                Span::styled(tag_text, Style::default().fg(Color::Magenta))
            };
            let name_span = Span::raw(format!(
                " v{} {} {}",
                v.version_number,
                v.name,
                v.created_at.format("%Y-%m-%d"),
            ));

            ListItem::new(Line::from(vec![status_span, pin_span, name_span, tags_span]))
        }).collect();

        let list = List::new(items)
            .block(Block::default().title("Resume Versions").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan))
            .highlight_symbol("> ");

        ratatui::widgets::StatefulWidget::render(list, layout[0], buf, &mut self.list_state);

        // Keybind hint bar
        let hints = Paragraph::new(
            "[j/k] navigate  [d] diff  [s] mark sent  [b] branch  [p] pin  [t] tag  [a] archive  [q] close"
        )
        .block(Block::default().borders(Borders::ALL));
        hints.render(layout[1], buf);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> Option<VersionBrowserAction> {
        match key {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                let len = self.versions.len().saturating_sub(1);
                let i = self.list_state.selected().map(|i| (i + 1).min(len)).unwrap_or(0);
                self.list_state.select(Some(i));
                None
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                let i = self.list_state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
                self.list_state.select(Some(i));
                None
            }
            crossterm::event::KeyCode::Char('d') => {
                self.selected_version().map(|v| VersionBrowserAction::Diff(v.id))
            }
            crossterm::event::KeyCode::Char('s') => {
                self.selected_version().map(|v| VersionBrowserAction::MarkSent(v.id))
            }
            crossterm::event::KeyCode::Char('b') => {
                self.selected_version().map(|v| VersionBrowserAction::Branch(v.id))
            }
            crossterm::event::KeyCode::Char('p') => {
                self.selected_version().map(|v| VersionBrowserAction::Pin(v.id))
            }
            crossterm::event::KeyCode::Char('t') => {
                self.selected_version().map(|v| VersionBrowserAction::AddTag(v.id))
            }
            crossterm::event::KeyCode::Char('a') => {
                self.selected_version().map(|v| VersionBrowserAction::Archive(v.id))
            }
            crossterm::event::KeyCode::Char('e') | crossterm::event::KeyCode::Enter => {
                self.selected_version().map(|v| VersionBrowserAction::Export(v.id))
            }
            crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                Some(VersionBrowserAction::Close)
            }
            _ => None,
        }
    }

    fn selected_version(&self) -> Option<&ResumeVersionRecord> {
        self.list_state.selected().and_then(|i| self.versions.get(i))
    }
}

pub enum VersionBrowserAction {
    Diff(ResumeVersionId),
    MarkSent(ResumeVersionId),
    Branch(ResumeVersionId),
    Pin(ResumeVersionId),
    AddTag(ResumeVersionId),
    Archive(ResumeVersionId),
    Export(ResumeVersionId),
    Close,
}
```

Keybindings:

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `d` | Diff with previous version |
| `s` | Mark as sent (opens channel picker dialog) |
| `b` | Branch new version from selected |
| `p` | Pin selected version to current application |
| `t` | Add tag (opens tag name prompt) |
| `a` | Archive selected version |
| `Enter` / `e` | Export to DOCX |
| `q` / `Esc` | Close browser |

---

#### Step 4.2 — Implement `ResumeDiffPanel`

File: `lazyjob-tui/src/views/resume/version_diff_panel.rs`

```rust
pub struct ResumeDiffPanel {
    pub diff: Option<ResumeDiff>,
    pub scroll: u16,
}

impl ResumeDiffPanel {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Block, Borders, Paragraph};
        use ratatui::style::{Color, Style};
        use ratatui::text::{Line, Span};

        let Some(diff) = &self.diff else { return; };

        let mut lines: Vec<Line> = Vec::new();

        // Keyword summary at top
        if !diff.keywords_added.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  Keywords added: {}", diff.keywords_added.join(", ")),
                Style::default().fg(Color::Green),
            )));
        }
        if !diff.keywords_removed.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  Keywords removed: {}", diff.keywords_removed.join(", ")),
                Style::default().fg(Color::Red),
            )));
        }
        if !diff.keywords_added.is_empty() || !diff.keywords_removed.is_empty() {
            lines.push(Line::from(Span::raw("")));
        }

        // Per-section diff
        for section in &diff.section_diffs {
            // Skip sections with no changes
            if section.bullet_changes.iter().all(|c| c.change_type == ChangeType::Unchanged) {
                continue;
            }

            lines.push(Line::from(Span::styled(
                format!("  ── {} ──", section.section_name),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));

            for change in &section.bullet_changes {
                match change.change_type {
                    ChangeType::Unchanged => {
                        // Omit unchanged bullets to reduce noise
                    }
                    ChangeType::Removed => {
                        let text = change.before.as_deref().unwrap_or("");
                        lines.push(Line::from(Span::styled(
                            format!("  - {}", text),
                            Style::default().fg(Color::Red),
                        )));
                    }
                    ChangeType::Added => {
                        let text = change.after.as_deref().unwrap_or("");
                        lines.push(Line::from(Span::styled(
                            format!("  + {}", text),
                            Style::default().fg(Color::Green),
                        )));
                    }
                    ChangeType::Modified => {
                        let before = change.before.as_deref().unwrap_or("");
                        let after  = change.after.as_deref().unwrap_or("");
                        lines.push(Line::from(Span::styled(
                            format!("  - {}", before),
                            Style::default().fg(Color::Red),
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("  + {}", after),
                            Style::default().fg(Color::Green),
                        )));
                    }
                }
            }
            lines.push(Line::from(Span::raw("")));
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::raw("  (no differences between versions)")));
        }

        let title = format!(
            "Diff: v{} → v{}",
            &diff.from_version.0.to_string()[..8],
            &diff.to_version.0.to_string()[..8],
        );

        let para = Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .scroll((self.scroll, 0));

        Widget::render(para, area, buf);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            crossterm::event::KeyCode::Char('G') => {
                self.scroll = u16::MAX; // scroll to bottom — ratatui clamps
            }
            crossterm::event::KeyCode::Char('g') => {
                self.scroll = 0;
            }
            _ => {}
        }
    }
}
```

Key design decisions:
- Unchanged bullets are omitted from the diff view to reduce noise. Sections with zero changes are skipped entirely.
- Keyword changes are shown as a summary block at the top (most impactful diff for ATS optimization).
- `Paragraph::scroll((u16, 0))` — ratatui clamps scroll to content height automatically.

Verification: snapshot test using `ratatui::backend::TestBackend` — render a known `ResumeDiff` with one Modified bullet and one Added keyword, assert the `-` and `+` prefixes appear in the output at the correct screen positions.

---

### Phase 5 — Export and CLI

#### Step 5.1 — DOCX and PDF Export

File: `lazyjob-core/src/resume/version_service.rs` (extend `ResumeVersionService`)

```rust
impl ResumeVersionService {
    /// Retrieve the raw DOCX bytes for a version.
    /// Returns `VersionServiceError::NoBlobStored` if the tailoring pipeline
    /// did not store a DOCX blob (e.g. for user-imported plain-text versions).
    pub async fn export_docx(
        &self,
        version_id: ResumeVersionId,
    ) -> Result<Vec<u8>, VersionServiceError> {
        self.version_repo.get_docx_blob(version_id).await.map_err(Into::into)
    }

    /// Write the DOCX bytes to a file path.
    pub async fn export_docx_to_file(
        &self,
        version_id: ResumeVersionId,
        dest: &std::path::Path,
    ) -> Result<(), VersionServiceError> {
        let bytes = self.export_docx(version_id).await?;
        tokio::fs::write(dest, &bytes).await
            .map_err(VersionServiceError::Io)
    }
}
```

PDF export: In Phase 1, PDF export delegates to an external tool. The recommended approach is to invoke `libreoffice --headless --convert-to pdf <file>.docx` via `tokio::process::Command` in `lazyjob-cli/src/commands/resume.rs`. This avoids a native Rust PDF rendering dependency.

```rust
// lazyjob-cli/src/commands/resume.rs
pub async fn export_pdf(version_id: ResumeVersionId, dest: &Path, service: &ResumeVersionService) -> anyhow::Result<()> {
    // Write DOCX to a temp file
    let tmp_dir = tempfile::tempdir()?;
    let docx_path = tmp_dir.path().join("resume.docx");
    service.export_docx_to_file(version_id, &docx_path).await?;

    // Convert to PDF via LibreOffice headless
    let status = tokio::process::Command::new("libreoffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir",
               tmp_dir.path().to_str().unwrap(),
               docx_path.to_str().unwrap()])
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("libreoffice conversion failed (exit: {:?})", status.code());
    }

    let pdf_path = tmp_dir.path().join("resume.pdf");
    tokio::fs::copy(&pdf_path, dest).await?;
    Ok(())
}
```

Note: LibreOffice dependency is an open question — see Open Questions §4.

---

#### Step 5.2 — CLI Subcommands

File: `lazyjob-cli/src/commands/resume.rs`

```rust
/// lazyjob resume versions <resume-id>
pub async fn list_versions(resume_id: ResumeId, service: &ResumeVersionService) -> anyhow::Result<()> {
    let versions = service.version_repo.list_for_resume(resume_id).await?;
    for v in &versions {
        println!("v{:02} {:?} {} | {}", v.version_number, v.status, v.name, v.id.0);
    }
    Ok(())
}

/// lazyjob resume mark-sent <version-id> [--app <application-id>] [--channel email|portal|linkedin]
pub async fn mark_sent(
    version_id: ResumeVersionId,
    application_id: Option<ApplicationId>,
    channel: SubmissionChannel,
    service: &ResumeVersionService,
) -> anyhow::Result<()> {
    service.mark_sent(version_id, channel, application_id).await?;
    println!("Version {} marked as sent.", version_id.0);
    Ok(())
}

/// lazyjob resume export <version-id> --format docx|pdf --output <path>
pub async fn export(
    version_id: ResumeVersionId,
    format: ExportFormat,
    dest: &std::path::Path,
    service: &ResumeVersionService,
) -> anyhow::Result<()> {
    match format {
        ExportFormat::Docx => service.export_docx_to_file(version_id, dest).await?,
        ExportFormat::Pdf  => export_pdf(version_id, dest, service).await?,
    }
    println!("Exported to {}", dest.display());
    Ok(())
}

pub enum ExportFormat { Docx, Pdf }
```

Verification: integration test creates a version, calls `mark-sent`, runs `list-versions` and asserts the version appears with `[SENT]` status.

---

### Phase 6 — Application Workflow Integration

**Step 6.1 — AutoPruneAfterHiring hook**

When `ApplicationWorkflow::move_stage()` transitions to `ApplicationStage::Accepted`, fire a `WorkflowEvent::ApplicationAccepted`. The TUI event handler (or a background tokio task) subscribes and calls:

```rust
cleanup_executor
    .apply(&CleanupPolicy::AutoPruneAfterHiring, resume_id)
    .await?;
```

This is explicitly non-fatal: `tracing::warn!` on failure, no rollback of the acceptance.

**Step 6.2 — Ralph tailoring trigger integration**

`ApplyWorkflow::execute()` (from the workflow actions plan) automatically triggers `LoopType::ResumeTailoring` when the user submits an application. After the loop completes, the Ralph result handler calls:

```rust
version_service
    .create_from_tailoring(resume_id, tailoring_result)
    .await?;
version_service
    .pin_version_for_application(new_version_id, application_id)
    .await?;
```

The TUI receives `VersionEvent::VersionCreated` and `VersionEvent::VersionPinned` via the broadcast channel and refreshes the resume panel.

---

## Key Crate APIs

| API | Usage |
|-----|-------|
| `similar::TextDiff::from_slices(&[&str], &[&str])` | Bullet-level diff between two resume version sections |
| `similar::Change::tag()` → `ChangeTag::{Equal,Delete,Insert}` | Classify each bullet change |
| `similar::Change::value()` → `&str` | Retrieve bullet text for a change entry |
| `sha2::Sha256::digest(&[u8])` | Compute `content_hash` for dedup/equality check |
| `hex::encode(GenericArray)` | Produce lowercase hex content_hash string |
| `sqlx::query!()` | Compile-time SQL in all repository methods |
| `sqlx::SqlitePool::execute()` / `fetch_optional()` / `fetch_all()` | Async SQLite operations |
| `sqlx::SqlitePool::begin()` + `Transaction::commit()` | Atomic pin-version transaction |
| `serde_json::to_string(&T)` / `from_str(&str)` | Serialize `ResumeContent`, `VersionSource`, `SubmissionChannel` as TEXT columns |
| `chrono::Utc::now()` | Timestamps for `created_at`, `updated_at`, `sent_at` |
| `tokio::sync::broadcast::Sender<VersionEvent>` | Notify TUI of state changes without polling |
| `ratatui::widgets::Clear` | Erase background before overlay rendering |
| `ratatui::widgets::List` + `ListState` | Scrollable version browser list |
| `ratatui::widgets::Paragraph::scroll((u16, u16))` | Scrollable diff panel |
| `ratatui::text::Span::styled(text, Style)` | Colored diff lines (red/green/cyan) |
| `ratatui::widgets::StatefulWidget::render` | Render `List` with cursor state |
| `crossterm::event::KeyCode` | Key dispatch in browser and diff panel |
| `tokio::process::Command::new("libreoffice")` | PDF export via headless LibreOffice |
| `tempfile::tempdir()` | Temp directory for DOCX→PDF conversion staging |
| `once_cell::sync::Lazy<Regex>` | Compile keyword extraction regex once at startup |

---

## Error Handling

```rust
// lazyjob-core/src/resume/errors.rs

#[derive(thiserror::Error, Debug)]
pub enum VersionRepoError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("version not found: {0:?}")]
    NotFound(ResumeVersionId),

    #[error("no DOCX blob stored for version {0:?}")]
    NoBlobStored(ResumeVersionId),
}

#[derive(thiserror::Error, Debug)]
pub enum VersionServiceError {
    #[error("repository error: {0}")]
    Repo(#[from] VersionRepoError),

    #[error("cannot archive Sent version {0:?} — it is a legal submission record")]
    CannotArchiveSent(ResumeVersionId),

    #[error("source version not found: {0:?}")]
    SourceNotFound(ResumeVersionId),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("export failed: {0}")]
    ExportFailed(String),
}
```

`VersionServiceError::CannotArchiveSent` is a product constraint — the TUI surfaces it as a dismissable error dialog: "This version has been submitted and cannot be archived."

---

## Testing Strategy

### Unit Tests

| Test | File | Strategy |
|------|------|----------|
| `content_hash_deterministic` | `version_types.rs` | Two calls with identical content → identical hash |
| `content_hash_changes_on_mutation` | `version_types.rs` | Change one bullet → hash changes |
| `differ_identical_versions` | `differ.rs` | All `Unchanged`, empty keyword lists |
| `differ_one_modified_bullet` | `differ.rs` | One bullet changed → one `Modified` entry |
| `differ_section_added` | `differ.rs` | New section in `to` → all `Added` bullets |
| `differ_section_removed` | `differ.rs` | Missing section in `to` → all `Removed` bullets |
| `differ_keyword_added` | `differ.rs` | "Kubernetes" only in `to` → `keywords_added = ["Kubernetes"]` |
| `cleanup_keep_last_n_basic` | `cleanup.rs` | 5 drafts + `KeepLastN { count: 2 }` → 3 archived |
| `cleanup_keep_last_n_preserves_sent` | `cleanup.rs` | 3 drafts + 2 sent + `KeepLastN { count: 1 }` → 3 drafts archived, 2 sent survive |
| `cleanup_old_drafts` | `cleanup.rs` | Draft older than threshold → archived; newer draft and sent → survive |
| `cleanup_auto_prune_after_hiring` | `cleanup.rs` | All drafts archived; all sent survive |
| `service_create_from_sets_lineage` | `version_service.rs` | New version `created_by == VersionSource::BranchedFrom { .. }` |
| `service_mark_sent_updates_status` | `version_service.rs` | After `mark_sent`, `get()` returns `status == Sent` |
| `service_archive_sent_fails` | `version_service.rs` | `archive(sent_id)` returns `Err(CannotArchiveSent)` |
| `service_events_broadcast` | `version_service.rs` | Subscribe to broadcast channel; `mark_sent` → `VersionSent` event received |

All unit tests use `Arc<MockResumeVersionRepository>` backed by `Arc<Mutex<HashMap<ResumeVersionId, ResumeVersionRecord>>>` — no SQLite in unit tests.

### Integration Tests

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_round_trip_insert_and_get(pool: sqlx::SqlitePool) {
    let repo = SqliteResumeVersionRepository::new(pool);
    let version = make_test_version();
    repo.insert(&version).await.unwrap();
    let fetched = repo.get(version.id).await.unwrap();
    assert_eq!(fetched.name, version.name);
    assert_eq!(fetched.content_hash, version.content_hash);
    assert_eq!(fetched.status, VersionStatus::Draft);
    assert_eq!(fetched.version_number, version.version_number);
}

#[sqlx::test(migrations = "migrations")]
async fn test_pin_version_is_exclusive(pool: sqlx::SqlitePool) {
    let link_repo = SqliteApplicationVersionLinkRepository::new(pool.clone());
    let v1 = ResumeVersionId::new();
    let v2 = ResumeVersionId::new();
    let app_id = ApplicationId::new();

    // Insert two links
    link_repo.insert(&make_link(app_id, v1)).await.unwrap();
    link_repo.insert(&make_link(app_id, v2)).await.unwrap();

    // Pin v1, then v2
    link_repo.pin_version(app_id, v1).await.unwrap();
    link_repo.pin_version(app_id, v2).await.unwrap();

    // Only v2 should be pinned
    let pinned = link_repo.get_pinned_version(app_id).await.unwrap();
    assert_eq!(pinned, Some(v2));
}

#[sqlx::test(migrations = "migrations")]
async fn test_tag_get_or_create_is_idempotent(pool: sqlx::SqlitePool) {
    let repo = SqliteVersionTagRepository::new(pool);
    let t1 = repo.get_or_create("sent-to-stripe", "#FF6B6B").await.unwrap();
    let t2 = repo.get_or_create("sent-to-stripe", "#AAAAAA").await.unwrap(); // different color, same name
    assert_eq!(t1.id, t2.id); // same row returned
}
```

### TUI Tests

```rust
#[test]
fn test_diff_panel_renders_minus_plus_lines() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    let diff = ResumeDiff {
        from_version: ResumeVersionId::new(),
        to_version:   ResumeVersionId::new(),
        section_diffs: vec![SectionDiff {
            section_name: "Experience".to_string(),
            bullet_changes: vec![
                BulletChange { index: 0, change_type: ChangeType::Removed,
                               before: Some("Built APIs for 10M users".into()), after: None },
                BulletChange { index: 0, change_type: ChangeType::Added,
                               before: None, after: Some("Built APIs for 50M users".into()) },
            ],
        }],
        keywords_added: vec![],
        keywords_removed: vec![],
    };

    let panel = ResumeDiffPanel { diff: Some(diff), scroll: 0 };
    terminal.draw(|f| { panel.render(f.area(), f.buffer_mut()); }).unwrap();

    let output: String = terminal.backend().buffer().content().iter()
        .map(|c| c.symbol()).collect();
    assert!(output.contains("- Built APIs for 10M users"));
    assert!(output.contains("+ Built APIs for 50M users"));
}

#[test]
fn test_version_browser_j_k_navigation() {
    let mut widget = ResumeVersionBrowserWidget {
        versions: vec![make_test_record(1), make_test_record(2), make_test_record(3)],
        tags_by_version: Default::default(),
        list_state: ListState::default(),
        pinned_version: None,
    };
    widget.list_state.select(Some(0));

    widget.handle_key(crossterm::event::KeyCode::Char('j'));
    assert_eq!(widget.list_state.selected(), Some(1));

    widget.handle_key(crossterm::event::KeyCode::Char('j'));
    assert_eq!(widget.list_state.selected(), Some(2));

    widget.handle_key(crossterm::event::KeyCode::Char('j'));
    assert_eq!(widget.list_state.selected(), Some(2)); // clamped at last

    widget.handle_key(crossterm::event::KeyCode::Char('k'));
    assert_eq!(widget.list_state.selected(), Some(1));
}
```

---

## Open Questions

1. **Version DAG vs. linear history**: The spec asks whether versions form a DAG or a linear list. The `VersionSource::BranchedFrom` field records lineage for display purposes, but the schema treats versions as a flat list ordered by `version_number`. Recommendation: start with a flat list per resume. If DAG visualization is needed, add a `parent_version_id` FK column and a recursive CTE query in a follow-up migration — no schema changes break compatibility.

2. **Template promotion**: Can a `ResumeVersion` be promoted to a reusable base template? Spec leaves this open. Recommendation: add `is_template: bool` column and a `template_name: Option<String>` column in migration 012. The tailoring pipeline's `get_base_version()` query would filter `WHERE is_template = 1 AND resume_id = ?`.

3. **ATS-specific plain-text variant**: Some portals strip DOCX formatting. Recommendation: add a `plain_text_content: Option<String>` column in migration 012. When `SubmissionChannel::CompanyPortal`, `Greenhouse`, or `Lever` is selected, offer the user a "Copy plain text" action that strips markdown/bullets. The stripping logic lives in `ResumeVersionService::generate_plain_text()` using a simple recursive `ResumeContent` → `String` serializer (no external crate needed).

4. **PDF export via LibreOffice**: LibreOffice is not always available on developer machines. Alternatives: (a) require it as a documented system dependency; (b) use the `printpdf` crate for a Rust-native PDF with simpler formatting; (c) ship a font-embedded PDF template and fill it via `pdf-min`. Recommendation for MVP: require LibreOffice and document the dependency clearly in `README.md`. Revisit post-MVP.

5. **`version_number` auto-increment strategy**: The current approach queries `MAX(version_number)` in `next_version_number()`. Under concurrent writes this has a TOCTOU window. Mitigation: use `BEGIN IMMEDIATE` transaction when creating a version, or switch to a `resume_meta` counter table updated by a trigger. For a local-first single-user app this is not a practical risk, but should be documented.

6. **DOCX blob storage size**: At ~50KB per version × 20 versions per resume × 20 active resumes = ~20MB of BLOB data. SQLite handles this fine, but the `get_docx_blob()` method should use `LIMIT 1` and not load all blobs when listing versions. The `list_for_resume()` query must explicitly exclude `docx_blob` from the `SELECT *` to avoid loading large blobs during list operations.

---

## Related Specs

- [`specs/XX-resume-version-management.md`](./XX-resume-version-management.md) — source spec for this plan
- [`specs/profile-resume-tailoring.md`](./profile-resume-tailoring.md) — resume tailoring pipeline (creates `ResumeVersion` entries)
- [`specs/profile-resume-tailoring-implementation-plan.md`](./profile-resume-tailoring-implementation-plan.md) — tailoring pipeline implementation plan (must be implemented first)
- [`specs/07-resume-tailoring-pipeline-implementation-plan.md`](./07-resume-tailoring-pipeline-implementation-plan.md) — pipeline architecture (also must precede this plan)
- [`specs/application-workflow-actions-implementation-plan.md`](./application-workflow-actions-implementation-plan.md) — workflow actions (provides `ApplyWorkflow`, `ApplicationId`)
- [`specs/application-state-machine-implementation-plan.md`](./application-state-machine-implementation-plan.md) — state machine (provides `ApplicationStage::Accepted` transition)
- [`specs/XX-cover-letter-version-management-implementation-plan.md`](./XX-cover-letter-version-management-implementation-plan.md) — parallel cover letter version management (same structural pattern)
