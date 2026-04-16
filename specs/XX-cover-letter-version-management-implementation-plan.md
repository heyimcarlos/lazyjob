# Implementation Plan: Cover Letter Version Management

## Status
Draft

## Related Spec
[specs/XX-cover-letter-version-management.md](./XX-cover-letter-version-management.md)

## Overview

Cover letter version management tracks the full lifecycle of every cover letter draft from AI generation through user edits to final submission. Without this layer, users lose track of which version was sent to which company — a failure mode that causes genuine harm (sending a "Dear Google" letter to Meta). This plan addresses version storage, sent-state tracking, diff comparison, branching (duplicate-for-application), submission channel recording, and cleanup policy enforcement.

The plan is structured as an extension of the `lazyjob-core` cover letter domain already defined in `profile-cover-letter-generation-implementation-plan.md`. It adds a `CoverLetterVersionRepository` trait with a `SqliteCoverLetterVersionRepository` implementation, a `SentRecordRepository` trait with its implementation, a `VersionDiffer` component using the `similar` crate, cleanup policy execution, and TUI version history browser. The existing `CoverLetterService` from the generation plan is extended with `mark_sent()`, `create_from()`, `duplicate_for_application()`, `link_to_application()`, and `apply_cleanup()` methods.

All version content is stored as structured JSON in SQLite (`content` TEXT column). Sent records live in a separate `cover_letter_sent_records` table so one version can record multiple send events (e.g., sent to the recruiter and the hiring manager). The diff view is paragraph-aware: the `VersionDiffer` splits content into paragraph blocks and uses `similar::capture_diff_slices` to produce `ParagraphDiff` values for the TUI renderer.

## Prerequisites

### Specs/Plans that must be implemented first
- `specs/profile-cover-letter-generation-implementation-plan.md` — provides `CoverLetterVersionId`, `CoverLetterContent`, `CoverLetterVersion`, `CoverLetterService`, SQLite migration 009, `cover_letter_versions` table
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, `SqlitePool`, migration infrastructure
- `specs/application-state-machine-implementation-plan.md` — provides `ApplicationId`, `ApplicationStage`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel focus infrastructure, `Action` enum

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml — additions
[dependencies]
similar            = "2"             # Paragraph-level diffing between versions
sha2               = "0.10"          # Content hash for dedup guard
hex                = "0.4"           # Hex encoding of SHA-256 hash
chrono             = { version = "0.4", features = ["serde"] }
uuid               = { version = "1", features = ["v4", "serde"] }
serde_json         = "1"
sqlx               = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
thiserror          = "2"
anyhow             = "1"
tracing            = "0.1"
async-trait        = "0.1"

# lazyjob-tui/Cargo.toml — additions
ratatui            = "0.28"
crossterm          = "0.28"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All domain logic: `CoverLetterVersionRepository`, `SentRecordRepository`, `VersionDiffer`, `CleanupExecutor`, `VersionService`, all new types, SQLite migrations |
| `lazyjob-tui` | `VersionHistoryView`, `VersionDiffPanel`, `VersionBrowserWidget` |
| `lazyjob-cli` | `lazyjob cover-letter versions <job-id>` subcommand, `lazyjob cover-letter mark-sent <version-id>` subcommand |

No changes required to `lazyjob-llm` or `lazyjob-ralph`.

### Core Types

```rust
// lazyjob-core/src/cover_letter/version_types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Re-use from cover letter generation plan
pub use super::types::{CoverLetterVersionId, CoverLetterContent};
pub use crate::application::ApplicationId;

/// Lifecycle status of a cover letter version.
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
    RalphGeneration { loop_id: Uuid },
    Import,
    BranchedFrom { parent_version_id: CoverLetterVersionId },
}

/// Channel through which the cover letter was submitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentChannel {
    Email { to: String, subject: String },
    CompanyPortal,
    LinkedIn,
    Greenhouse,
    Lever,
    Other { label: String },
}

/// Confirmation / read-receipt tracking status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TrackingStatus {
    Pending,
    ConfirmedSent,
    Delivered,
    Bounced,
    OpenedRead,
}

/// A single cover letter version row.
///
/// `content_hash` = SHA-256 of `serde_json::to_string(&content)` in hex.
/// Used as a fast-path equality check; duplicate identical content is allowed
/// (user may want two versions with same body, different names).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterVersion {
    pub id: CoverLetterVersionId,
    pub name: String,
    pub application_id: Option<ApplicationId>,
    pub content: CoverLetterContent,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: VersionSource,
    pub status: VersionStatus,
    pub sent_via: Option<SentChannel>,
    pub sent_at: Option<DateTime<Utc>>,
}

/// Unique ID for a sent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct SentRecordId(pub Uuid);

impl SentRecordId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Immutable record written when a version is submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentCoverLetter {
    pub id: SentRecordId,
    pub version_id: CoverLetterVersionId,
    pub channel: SentChannel,
    pub recipient: String,
    pub subject: String,
    pub sent_at: DateTime<Utc>,
    pub confirmation_id: Option<String>,
    pub tracking_status: TrackingStatus,
}

/// Extra metadata provided when recording a submission.
#[derive(Debug, Clone)]
pub struct SubmissionMetadata {
    pub recipient: String,
    pub subject: String,
    pub confirmation_id: Option<String>,
}

/// User-visible cleanup policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanupPolicy {
    KeepAll,
    KeepLastN { count: u32 },
    KeepSentVersions,
    AutoPruneAfterHiring,
}

/// The diff between two versions at paragraph granularity.
#[derive(Debug, Clone)]
pub struct VersionDiff {
    pub from_version: CoverLetterVersionId,
    pub to_version: CoverLetterVersionId,
    pub paragraph_changes: Vec<ParagraphDiff>,
}

#[derive(Debug, Clone)]
pub struct ParagraphDiff {
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
```

### Trait Definitions

```rust
// lazyjob-core/src/cover_letter/repository.rs

use async_trait::async_trait;
use crate::cover_letter::version_types::*;
use crate::application::ApplicationId;

#[async_trait]
pub trait CoverLetterVersionRepository: Send + Sync {
    async fn insert(&self, version: &CoverLetterVersion) -> Result<(), VersionRepoError>;
    async fn update(&self, version: &CoverLetterVersion) -> Result<(), VersionRepoError>;
    async fn get(&self, id: CoverLetterVersionId) -> Result<CoverLetterVersion, VersionRepoError>;
    async fn list_for_application(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<CoverLetterVersion>, VersionRepoError>;
    async fn list_all_drafts(&self) -> Result<Vec<CoverLetterVersion>, VersionRepoError>;
    async fn delete(&self, id: CoverLetterVersionId) -> Result<(), VersionRepoError>;
    async fn archive(&self, id: CoverLetterVersionId) -> Result<(), VersionRepoError>;
    async fn count_for_application(
        &self,
        application_id: ApplicationId,
    ) -> Result<u32, VersionRepoError>;
    /// Returns all non-archived versions ordered by `created_at DESC`, oldest
    /// first (KeepLastN operates on this ordering).
    async fn list_by_created_asc(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<CoverLetterVersion>, VersionRepoError>;
    /// All versions whose linked application reached Accepted stage.
    async fn list_for_accepted_applications(&self) -> Result<Vec<CoverLetterVersion>, VersionRepoError>;
}

#[async_trait]
pub trait SentRecordRepository: Send + Sync {
    async fn insert(&self, record: &SentCoverLetter) -> Result<(), VersionRepoError>;
    async fn list_for_version(
        &self,
        version_id: CoverLetterVersionId,
    ) -> Result<Vec<SentCoverLetter>, VersionRepoError>;
    async fn update_tracking_status(
        &self,
        record_id: SentRecordId,
        status: TrackingStatus,
    ) -> Result<(), VersionRepoError>;
}
```

### SQLite Schema

```sql
-- Migration 010 (in lazyjob-core/migrations/010_cover_letter_version_management.sql)
-- The cover_letter_versions table was first introduced in migration 009 by the
-- cover letter generation plan with minimal columns.  This migration adds the
-- new columns required for full version management.

ALTER TABLE cover_letter_versions ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE cover_letter_versions ADD COLUMN created_by  TEXT NOT NULL DEFAULT 'user_edited';
ALTER TABLE cover_letter_versions ADD COLUMN sent_via    TEXT;
ALTER TABLE cover_letter_versions ADD COLUMN sent_at     TEXT;

-- Index: fast lookup of all versions for a given application
CREATE INDEX IF NOT EXISTS idx_clv_application_id
    ON cover_letter_versions(application_id)
    WHERE application_id IS NOT NULL;

-- Index: all non-archived drafts (default view)
CREATE INDEX IF NOT EXISTS idx_clv_status
    ON cover_letter_versions(status)
    WHERE status != 'archived';

-- Sent records table (one version can have multiple send events)
CREATE TABLE IF NOT EXISTS cover_letter_sent_records (
    id               TEXT PRIMARY KEY,          -- UUID
    version_id       TEXT NOT NULL
        REFERENCES cover_letter_versions(id) ON DELETE CASCADE,
    channel          TEXT NOT NULL,             -- JSON-serialized SentChannel
    recipient        TEXT NOT NULL DEFAULT '',
    subject          TEXT NOT NULL DEFAULT '',
    sent_at          TEXT NOT NULL,             -- ISO-8601
    confirmation_id  TEXT,
    tracking_status  TEXT NOT NULL DEFAULT 'pending',
    created_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_clsr_version_id
    ON cover_letter_sent_records(version_id);

-- Application ↔ version explicit link table
-- (application_id already stored on cover_letter_versions for the primary link;
--  this table handles the many-to-many case: one version sent to multiple apps)
CREATE TABLE IF NOT EXISTS cover_letter_application_links (
    application_id         TEXT NOT NULL
        REFERENCES applications(id) ON DELETE CASCADE,
    cover_letter_version_id TEXT NOT NULL
        REFERENCES cover_letter_versions(id) ON DELETE CASCADE,
    linked_at              TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (application_id, cover_letter_version_id)
);
```

### Module Structure

```
lazyjob-core/
  src/
    cover_letter/
      mod.rs                    -- re-exports: CoverLetterVersion, VersionService, etc.
      types.rs                  -- existing from generation plan
      version_types.rs          -- NEW: VersionStatus, SentChannel, VersionDiff, etc.
      repository.rs             -- NEW: CoverLetterVersionRepository, SentRecordRepository traits
      sqlite_version_repo.rs    -- NEW: SqliteCoverLetterVersionRepository impl
      sqlite_sent_repo.rs       -- NEW: SqliteSentRecordRepository impl
      version_service.rs        -- NEW: VersionService (mark_sent, create_from, diff, cleanup)
      differ.rs                 -- NEW: VersionDiffer using `similar`
      cleanup.rs                -- NEW: CleanupExecutor
      errors.rs                 -- NEW: VersionRepoError, VersionServiceError

lazyjob-tui/
  src/
    views/
      cover_letter/
        version_browser.rs      -- NEW: VersionBrowserWidget (list + keybinds)
        version_diff_panel.rs   -- NEW: VersionDiffPanel (paragraph diff view)
        mod.rs                  -- re-exports
```

---

## Implementation Phases

### Phase 1 — Core Storage and Repository (MVP)

**Step 1.1 — Apply migration 010**

File: `lazyjob-core/migrations/010_cover_letter_version_management.sql`

Add `content_hash`, `created_by`, `sent_via`, `sent_at` columns to `cover_letter_versions`; create `cover_letter_sent_records` and `cover_letter_application_links` tables with the DDL above.

Apply via `sqlx::migrate!()` in the `Database::connect()` startup path.

Verification: `cargo sqlx prepare --check` passes with the updated `sqlx-data.json`.

---

**Step 1.2 — Implement `content_hash` computation**

File: `lazyjob-core/src/cover_letter/version_types.rs`

```rust
use sha2::{Digest, Sha256};

impl CoverLetterVersion {
    /// Compute SHA-256 of the canonical JSON-serialized content.
    /// Call this before every `insert()` or `update()` to keep the field
    /// consistent with the `content` column.
    pub fn compute_content_hash(content: &CoverLetterContent) -> String {
        let canonical = serde_json::to_string(content)
            .expect("CoverLetterContent is always serializable");
        let digest = Sha256::digest(canonical.as_bytes());
        hex::encode(digest)
    }
}
```

Key APIs:
- `sha2::Sha256::digest(&[u8])` — returns a `GenericArray`
- `hex::encode(bytes)` — returns lowercase hex string

Verification: unit test asserts `compute_content_hash` is deterministic across two calls with identical content.

---

**Step 1.3 — Implement `SqliteCoverLetterVersionRepository`**

File: `lazyjob-core/src/cover_letter/sqlite_version_repo.rs`

```rust
pub struct SqliteCoverLetterVersionRepository {
    pool: sqlx::SqlitePool,
}

impl SqliteCoverLetterVersionRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CoverLetterVersionRepository for SqliteCoverLetterVersionRepository {
    async fn insert(&self, version: &CoverLetterVersion) -> Result<(), VersionRepoError> {
        let id = version.id.0.to_string();
        let application_id = version.application_id.map(|a| a.0.to_string());
        let content_json = serde_json::to_string(&version.content)
            .map_err(VersionRepoError::Serialization)?;
        let created_by_json = serde_json::to_string(&version.created_by)
            .map_err(VersionRepoError::Serialization)?;
        let sent_via_json = version.sent_via.as_ref()
            .map(|c| serde_json::to_string(c))
            .transpose()
            .map_err(VersionRepoError::Serialization)?;
        let status_str = match version.status {
            VersionStatus::Draft => "draft",
            VersionStatus::Sent => "sent",
            VersionStatus::Archived => "archived",
        };

        sqlx::query!(
            r#"
            INSERT INTO cover_letter_versions
                (id, name, application_id, content, content_hash, created_at, updated_at,
                 created_by, status, sent_via, sent_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id,
            version.name,
            application_id,
            content_json,
            version.content_hash,
            version.created_at,
            version.updated_at,
            created_by_json,
            status_str,
            sent_via_json,
            version.sent_at,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(())
    }

    async fn get(&self, id: CoverLetterVersionId) -> Result<CoverLetterVersion, VersionRepoError> {
        let id_str = id.0.to_string();
        let row = sqlx::query!(
            "SELECT * FROM cover_letter_versions WHERE id = ?",
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?
        .ok_or(VersionRepoError::NotFound(id))?;

        Self::map_row(row)
    }

    async fn archive(&self, id: CoverLetterVersionId) -> Result<(), VersionRepoError> {
        let id_str = id.0.to_string();
        let updated_at = chrono::Utc::now();
        sqlx::query!(
            "UPDATE cover_letter_versions SET status = 'archived', updated_at = ? WHERE id = ?",
            updated_at,
            id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;
        Ok(())
    }

    // ... remaining methods follow the same pattern
}
```

Key APIs:
- `sqlx::query!()` macro with named bind params — compile-time checked SQL
- `sqlx::SqlitePool::execute()` / `fetch_optional()` / `fetch_all()`
- `serde_json::to_string()` / `from_str()` for JSON columns
- `chrono::Utc::now()` for `updated_at`

Verification: `#[sqlx::test(migrations = "migrations")]` test inserts a version and retrieves it, asserting all fields round-trip correctly.

---

**Step 1.4 — Implement `SqliteSentRecordRepository`**

File: `lazyjob-core/src/cover_letter/sqlite_sent_repo.rs`

```rust
pub struct SqliteSentRecordRepository {
    pool: sqlx::SqlitePool,
}

#[async_trait]
impl SentRecordRepository for SqliteSentRecordRepository {
    async fn insert(&self, record: &SentCoverLetter) -> Result<(), VersionRepoError> {
        let id = record.id.0.to_string();
        let version_id = record.version_id.0.to_string();
        let channel_json = serde_json::to_string(&record.channel)
            .map_err(VersionRepoError::Serialization)?;
        let tracking_status = format!("{:?}", record.tracking_status).to_lowercase();

        sqlx::query!(
            r#"
            INSERT INTO cover_letter_sent_records
                (id, version_id, channel, recipient, subject, sent_at,
                 confirmation_id, tracking_status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id, version_id, channel_json,
            record.recipient, record.subject, record.sent_at,
            record.confirmation_id, tracking_status,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;

        Ok(())
    }

    async fn update_tracking_status(
        &self,
        record_id: SentRecordId,
        status: TrackingStatus,
    ) -> Result<(), VersionRepoError> {
        let id_str = record_id.0.to_string();
        let status_str = format!("{:?}", status).to_lowercase();
        sqlx::query!(
            "UPDATE cover_letter_sent_records SET tracking_status = ? WHERE id = ?",
            status_str, id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(VersionRepoError::Database)?;
        Ok(())
    }

    // list_for_version: SELECT * WHERE version_id = ? ORDER BY sent_at ASC
}
```

Verification: `#[sqlx::test]` test inserts a sent record, lists it by version ID, and updates tracking status.

---

### Phase 2 — Version Service Operations

**Step 2.1 — Implement `VersionService`**

File: `lazyjob-core/src/cover_letter/version_service.rs`

```rust
pub struct VersionService {
    version_repo: Arc<dyn CoverLetterVersionRepository>,
    sent_repo: Arc<dyn SentRecordRepository>,
    event_tx: tokio::sync::broadcast::Sender<VersionEvent>,
}

/// Events broadcast to the TUI for re-render.
#[derive(Debug, Clone)]
pub enum VersionEvent {
    VersionCreated { id: CoverLetterVersionId },
    VersionSent { id: CoverLetterVersionId, channel: SentChannel },
    VersionArchived { id: CoverLetterVersionId },
    CleanupCompleted { pruned: u32 },
}
```

**`mark_sent()` — atomic sent-state transition**

```rust
impl VersionService {
    /// Mark a version as sent, write a SentCoverLetter record, and broadcast
    /// VersionEvent::VersionSent for TUI re-render.
    ///
    /// Non-fatal if the version is already Sent — re-sending is allowed
    /// (user may send the same letter to multiple recipients).
    pub async fn mark_sent(
        &self,
        version_id: CoverLetterVersionId,
        channel: SentChannel,
        metadata: SubmissionMetadata,
    ) -> Result<SentRecordId, VersionServiceError> {
        let sent_record = SentCoverLetter {
            id: SentRecordId::new(),
            version_id,
            channel: channel.clone(),
            recipient: metadata.recipient,
            subject: metadata.subject,
            sent_at: Utc::now(),
            confirmation_id: metadata.confirmation_id,
            tracking_status: TrackingStatus::Pending,
        };

        self.sent_repo.insert(&sent_record).await?;

        // Update version status to Sent (idempotent if already Sent)
        let mut version = self.version_repo.get(version_id).await?;
        version.status = VersionStatus::Sent;
        version.sent_via = Some(channel.clone());
        version.sent_at = Some(sent_record.sent_at);
        version.updated_at = Utc::now();
        self.version_repo.update(&version).await?;

        let _ = self.event_tx.send(VersionEvent::VersionSent {
            id: version_id,
            channel,
        });

        Ok(sent_record.id)
    }
```

**`create_from()` — branch a new version from an existing one**

```rust
    /// Create a new Draft version that starts as a copy of `source_version`.
    /// `VersionSource::BranchedFrom` records the lineage.
    pub async fn create_from(
        &self,
        source_version_id: CoverLetterVersionId,
        name: String,
    ) -> Result<CoverLetterVersionId, VersionServiceError> {
        let source = self.version_repo.get(source_version_id).await?;
        let content_hash = CoverLetterVersion::compute_content_hash(&source.content);

        let new_version = CoverLetterVersion {
            id: CoverLetterVersionId::new(),
            name,
            application_id: None,
            content: source.content.clone(),
            content_hash,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::BranchedFrom { parent_version_id: source_version_id },
            status: VersionStatus::Draft,
            sent_via: None,
            sent_at: None,
        };

        self.version_repo.insert(&new_version).await?;
        let _ = self.event_tx.send(VersionEvent::VersionCreated { id: new_version.id });
        Ok(new_version.id)
    }
```

**`duplicate_for_application()` — create a linked branch**

```rust
    /// Branch from `source_version` and immediately link it to `application_id`.
    /// The generated name is `"<source_name> (copy for <application_id>)"`.
    pub async fn duplicate_for_application(
        &self,
        source_version_id: CoverLetterVersionId,
        application_id: ApplicationId,
    ) -> Result<CoverLetterVersionId, VersionServiceError> {
        let source = self.version_repo.get(source_version_id).await?;
        let name = format!("{} (copy)", source.name);
        let new_id = self.create_from(source_version_id, name).await?;

        let mut new_version = self.version_repo.get(new_id).await?;
        new_version.application_id = Some(application_id);
        new_version.updated_at = Utc::now();
        self.version_repo.update(&new_version).await?;

        Ok(new_id)
    }
```

**`link_to_application()` — associate an existing version with an application**

```rust
    pub async fn link_to_application(
        &self,
        version_id: CoverLetterVersionId,
        application_id: ApplicationId,
    ) -> Result<(), VersionServiceError> {
        let mut version = self.version_repo.get(version_id).await?;
        version.application_id = Some(application_id);
        version.updated_at = Utc::now();
        self.version_repo.update(&version).await?;
        Ok(())
    }
}
```

Verification: unit tests for each method using `Arc<MockVersionRepository>` (hand-written mock with an `Arc<Mutex<HashMap>>` backing store).

---

**Step 2.2 — Implement `CleanupExecutor`**

File: `lazyjob-core/src/cover_letter/cleanup.rs`

```rust
pub struct CleanupExecutor {
    version_repo: Arc<dyn CoverLetterVersionRepository>,
}

impl CleanupExecutor {
    pub async fn apply(
        &self,
        policy: &CleanupPolicy,
        application_id: ApplicationId,
    ) -> Result<u32, VersionServiceError> {
        match policy {
            CleanupPolicy::KeepAll => Ok(0),
            CleanupPolicy::KeepLastN { count } => {
                self.prune_to_count(application_id, *count).await
            }
            CleanupPolicy::KeepSentVersions => {
                self.archive_unsent_older_than_days(application_id, 30).await
            }
            CleanupPolicy::AutoPruneAfterHiring => {
                self.archive_non_sent_for_accepted(application_id).await
            }
        }
    }

    async fn prune_to_count(
        &self,
        application_id: ApplicationId,
        keep: u32,
    ) -> Result<u32, VersionServiceError> {
        // Fetch all versions ordered oldest first.
        let all = self.version_repo
            .list_by_created_asc(application_id)
            .await?;

        let total = all.len() as u32;
        if total <= keep {
            return Ok(0);
        }

        let to_archive = &all[..(total - keep) as usize];
        let mut pruned = 0u32;

        for v in to_archive {
            // Never archive Sent versions — they are legal records.
            if v.status != VersionStatus::Sent {
                self.version_repo.archive(v.id).await?;
                pruned += 1;
            }
        }

        Ok(pruned)
    }

    async fn archive_unsent_older_than_days(
        &self,
        application_id: ApplicationId,
        days: i64,
    ) -> Result<u32, VersionServiceError> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let all = self.version_repo
            .list_by_created_asc(application_id)
            .await?;

        let mut pruned = 0u32;
        for v in all {
            if v.status == VersionStatus::Draft && v.created_at < cutoff {
                self.version_repo.archive(v.id).await?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    async fn archive_non_sent_for_accepted(
        &self,
        application_id: ApplicationId,
    ) -> Result<u32, VersionServiceError> {
        // Only archive drafts; leave Sent versions as legal records.
        let all = self.version_repo
            .list_by_created_asc(application_id)
            .await?;

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

Verification: unit tests for `KeepLastN` assert exactly `count` versions survive after pruning; Sent versions are never archived.

---

### Phase 3 — Version Diffing

**Step 3.1 — Implement `VersionDiffer`**

File: `lazyjob-core/src/cover_letter/differ.rs`

The `CoverLetterContent` type (from the generation plan) stores a structured list of paragraphs. The differ extracts paragraph strings and feeds them to `similar::capture_diff_slices`.

```rust
use similar::{ChangeTag, TextDiff};

pub struct VersionDiffer;

impl VersionDiffer {
    /// Compute a paragraph-level diff between two versions.
    ///
    /// Paragraphs are extracted from `CoverLetterContent::paragraphs`
    /// (assumed to be `Vec<String>`).
    pub fn diff(from: &CoverLetterVersion, to: &CoverLetterVersion) -> VersionDiff {
        let from_paragraphs: Vec<&str> = from.content.paragraphs
            .iter()
            .map(String::as_str)
            .collect();
        let to_paragraphs: Vec<&str> = to.content.paragraphs
            .iter()
            .map(String::as_str)
            .collect();

        let diff = TextDiff::from_slices(&from_paragraphs, &to_paragraphs);
        let mut changes = Vec::new();
        let mut index = 0usize;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    changes.push(ParagraphDiff {
                        index,
                        change_type: ChangeType::Unchanged,
                        before: Some(change.value().to_string()),
                        after: Some(change.value().to_string()),
                    });
                    index += 1;
                }
                ChangeTag::Delete => {
                    changes.push(ParagraphDiff {
                        index,
                        change_type: ChangeType::Removed,
                        before: Some(change.value().to_string()),
                        after: None,
                    });
                    index += 1;
                }
                ChangeTag::Insert => {
                    // Collapse adjacent Delete+Insert into Modified
                    if let Some(last) = changes.last_mut() {
                        if last.change_type == ChangeType::Removed && last.after.is_none() {
                            last.change_type = ChangeType::Modified;
                            last.after = Some(change.value().to_string());
                            continue;
                        }
                    }
                    changes.push(ParagraphDiff {
                        index,
                        change_type: ChangeType::Added,
                        before: None,
                        after: Some(change.value().to_string()),
                    });
                    index += 1;
                }
            }
        }

        VersionDiff {
            from_version: from.id,
            to_version: to.id,
            paragraph_changes: changes,
        }
    }
}
```

Key APIs:
- `similar::TextDiff::from_slices(&[&str], &[&str])` — element-level (paragraph) diff
- `similar::Change::tag()` → `ChangeTag::{Equal, Delete, Insert}`
- `similar::Change::value()` → `&str` of the paragraph text

Verification: unit test diffs two known versions with one paragraph modified; asserts `paragraph_changes` contains exactly one `ChangeType::Modified` entry with correct `before`/`after` text.

---

### Phase 4 — TUI Version Browser

**Step 4.1 — Implement `VersionBrowserWidget`**

File: `lazyjob-tui/src/views/cover_letter/version_browser.rs`

The version browser is a modal overlay triggered by pressing `v` in the cover letter view. It uses a `ratatui::widgets::List` to display versions with status badges.

```rust
pub struct VersionBrowserWidget {
    pub versions: Vec<CoverLetterVersion>,
    pub state: ratatui::widgets::ListState,
    pub sent_records: HashMap<CoverLetterVersionId, Vec<SentCoverLetter>>,
}

impl VersionBrowserWidget {
    pub fn render(&mut self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::widgets::{Block, Borders, List, ListItem};
        use ratatui::style::{Color, Modifier, Style};

        let items: Vec<ListItem> = self.versions.iter().map(|v| {
            let status_badge = match v.status {
                VersionStatus::Draft    => ratatui::text::Span::styled("[DRAFT]",    Style::default().fg(Color::Yellow)),
                VersionStatus::Sent     => ratatui::text::Span::styled("[SENT]",     Style::default().fg(Color::Green)),
                VersionStatus::Archived => ratatui::text::Span::styled("[ARCHIVED]", Style::default().fg(Color::DarkGray)),
            };
            let sent_count = self.sent_records.get(&v.id).map(|r| r.len()).unwrap_or(0);
            let label = if sent_count > 0 {
                format!("{} {}  ({} send(s))", v.name, v.created_at.format("%Y-%m-%d"), sent_count)
            } else {
                format!("{} {}", v.name, v.created_at.format("%Y-%m-%d"))
            };
            ListItem::new(ratatui::text::Line::from(vec![
                status_badge,
                ratatui::text::Span::raw(format!(" {}", label)),
            ]))
        }).collect();

        let list = List::new(items)
            .block(Block::default().title("Cover Letter Versions").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan))
            .highlight_symbol("> ");

        ratatui::widgets::StatefulWidget::render(list, area, buf, &mut self.state);
    }

    /// Handle key events. Returns the selected action or None.
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> Option<VersionBrowserAction> {
        match key {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                let i = self.state.selected().map(|i| (i + 1).min(self.versions.len().saturating_sub(1))).unwrap_or(0);
                self.state.select(Some(i));
                None
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                let i = self.state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
                self.state.select(Some(i));
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
            crossterm::event::KeyCode::Char('a') => {
                self.selected_version().map(|v| VersionBrowserAction::Archive(v.id))
            }
            crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                Some(VersionBrowserAction::Close)
            }
            _ => None,
        }
    }

    fn selected_version(&self) -> Option<&CoverLetterVersion> {
        self.state.selected().and_then(|i| self.versions.get(i))
    }
}

pub enum VersionBrowserAction {
    Diff(CoverLetterVersionId),
    MarkSent(CoverLetterVersionId),
    Branch(CoverLetterVersionId),
    Archive(CoverLetterVersionId),
    Close,
}
```

Keybindings:
| Key | Action |
|-----|--------|
| `j` / `↓` | Move down in list |
| `k` / `↑` | Move up in list |
| `d` | Show diff from previous version |
| `s` | Mark selected version as sent |
| `b` | Branch new version from selected |
| `a` | Archive selected version |
| `q` / `Esc` | Close browser |

---

**Step 4.2 — Implement `VersionDiffPanel`**

File: `lazyjob-tui/src/views/cover_letter/version_diff_panel.rs`

```rust
pub struct VersionDiffPanel {
    pub diff: Option<VersionDiff>,
    pub scroll: u16,
}

impl VersionDiffPanel {
    pub fn render(&self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::widgets::{Block, Borders, Paragraph};
        use ratatui::style::{Color, Style};
        use ratatui::text::{Line, Span};

        let Some(diff) = &self.diff else {
            return;
        };

        let lines: Vec<Line> = diff.paragraph_changes.iter().flat_map(|p| {
            match p.change_type {
                ChangeType::Unchanged => {
                    let text = p.before.as_deref().unwrap_or("");
                    vec![Line::from(Span::raw(format!("  {}", text)))]
                }
                ChangeType::Removed => {
                    let text = p.before.as_deref().unwrap_or("");
                    vec![
                        Line::from(Span::styled(
                            format!("- {}", text),
                            Style::default().fg(Color::Red),
                        )),
                    ]
                }
                ChangeType::Added => {
                    let text = p.after.as_deref().unwrap_or("");
                    vec![
                        Line::from(Span::styled(
                            format!("+ {}", text),
                            Style::default().fg(Color::Green),
                        )),
                    ]
                }
                ChangeType::Modified => {
                    let before = p.before.as_deref().unwrap_or("");
                    let after  = p.after.as_deref().unwrap_or("");
                    vec![
                        Line::from(Span::styled(format!("- {}", before), Style::default().fg(Color::Red))),
                        Line::from(Span::styled(format!("+ {}", after),  Style::default().fg(Color::Green))),
                    ]
                }
            }
        }).collect();

        let title = format!(
            "Diff: v{} → v{}",
            &diff.from_version.0.to_string()[..8],
            &diff.to_version.0.to_string()[..8],
        );

        let para = Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .scroll((self.scroll, 0));

        ratatui::widgets::Widget::render(para, area, buf);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            _ => {}
        }
    }
}
```

Verification: snapshot test using `ratatui::backend::TestBackend` asserts that a known `VersionDiff` renders removed lines in red and added lines in green.

---

### Phase 5 — Cleanup Policy Integration and CLI

**Step 5.1 — Wire CleanupExecutor into the application workflow**

When `ApplicationWorkflow::move_stage()` transitions to `ApplicationStage::Accepted`, broadcast a `WorkflowEvent::ApplicationAccepted` that the TUI (or a background hook) handles by calling:

```rust
cleanup_executor
    .apply(&CleanupPolicy::AutoPruneAfterHiring, application_id)
    .await?;
```

This is non-fatal: log a `tracing::warn!` if cleanup fails, do not rollback the acceptance.

---

**Step 5.2 — CLI subcommands**

File: `lazyjob-cli/src/commands/cover_letter.rs`

```rust
/// lazyjob cover-letter versions <application-id>
/// Lists all versions for a given application in a table.
pub async fn list_versions(app_id: ApplicationId, service: &VersionService) {
    let versions = service.version_repo.list_for_application(app_id).await?;
    for v in &versions {
        println!("{} | {:?} | {}", v.id.0, v.status, v.name);
    }
}

/// lazyjob cover-letter mark-sent <version-id> --channel email --to "hiring@company.com" --subject "..."
pub async fn mark_sent_cmd(
    version_id: CoverLetterVersionId,
    channel: SentChannel,
    metadata: SubmissionMetadata,
    service: &VersionService,
) {
    let record_id = service.mark_sent(version_id, channel, metadata).await?;
    println!("Recorded sent event: {}", record_id.0);
}
```

Verification: integration test spawns a CLI process, runs `cover-letter mark-sent`, then `cover-letter versions` and asserts the version appears with `[SENT]` status.

---

## Key Crate APIs

| API | Usage |
|-----|-------|
| `similar::TextDiff::from_slices(&[&str], &[&str])` | Paragraph-level diff between two versions |
| `similar::Change::tag()` → `ChangeTag::{Equal,Delete,Insert}` | Classify each paragraph change |
| `sha2::Sha256::digest(&[u8])` | Compute content hash for dedup/equality check |
| `hex::encode(GenericArray)` | Produce lowercase hex content_hash string |
| `sqlx::query!()` | Compile-time SQL in all repository methods |
| `sqlx::SqlitePool::execute()` / `fetch_optional()` / `fetch_all()` | Async SQLite operations |
| `serde_json::to_string(&T)` / `from_str(&str)` | Serialize `CoverLetterContent`, `SentChannel`, `VersionSource` as TEXT columns |
| `chrono::Utc::now()` | Timestamps for `created_at`, `updated_at`, `sent_at` |
| `tokio::sync::broadcast::Sender<VersionEvent>` | Notify TUI of version state changes |
| `ratatui::widgets::List` + `ListState` | Version browser scrollable list |
| `ratatui::widgets::Paragraph::scroll((u16, u16))` | Scrollable diff panel |
| `ratatui::text::Span::styled(text, Style)` | Red/green diff lines |
| `crossterm::event::KeyCode` | Key dispatch in browser and diff panel |

---

## Error Handling

```rust
// lazyjob-core/src/cover_letter/errors.rs

#[derive(thiserror::Error, Debug)]
pub enum VersionRepoError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("version not found: {0:?}")]
    NotFound(CoverLetterVersionId),
}

#[derive(thiserror::Error, Debug)]
pub enum VersionServiceError {
    #[error("repository error: {0}")]
    Repo(#[from] VersionRepoError),

    #[error("cannot archive a Sent version: {0:?}")]
    CannotArchiveSent(CoverLetterVersionId),

    #[error("source version not found: {0:?}")]
    SourceNotFound(CoverLetterVersionId),
}
```

`VersionServiceError::CannotArchiveSent` is returned (rather than silently skipping) so the TUI can surface a dismissable error dialog: "This version has been sent and cannot be archived."

---

## Testing Strategy

### Unit Tests

| Test | File | Strategy |
|------|------|----------|
| `content_hash_deterministic` | `version_types.rs` | Assert two calls with identical content produce identical hashes |
| `content_hash_differs_on_change` | `version_types.rs` | Mutate one paragraph, assert hash changes |
| `differ_unchanged` | `differ.rs` | Identical versions produce all `Unchanged` entries |
| `differ_modified` | `differ.rs` | Change one paragraph, assert single `Modified` entry with correct before/after |
| `differ_added` | `differ.rs` | Add a paragraph to `to`, assert `Added` entry at correct index |
| `differ_removed` | `differ.rs` | Remove a paragraph from `from`, assert `Removed` entry |
| `cleanup_keep_last_n` | `cleanup.rs` | 5 drafts + `KeepLastN { count: 2 }` → 3 archived, 2 survive |
| `cleanup_keep_sent_versions` | `cleanup.rs` | 1 sent + 3 drafts → all 3 drafts archived, sent survives |
| `cleanup_never_archives_sent` | `cleanup.rs` | `KeepLastN { count: 0 }` still leaves Sent versions untouched |
| `mark_sent_updates_status` | `version_service.rs` | After `mark_sent()`, `get()` returns `status == VersionStatus::Sent` |
| `create_from_sets_lineage` | `version_service.rs` | New version `created_by == VersionSource::BranchedFrom { parent_version_id }` |

All unit tests use `Arc<MockVersionRepository>` backed by `Arc<Mutex<HashMap<CoverLetterVersionId, CoverLetterVersion>>>`.

### Integration Tests

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_round_trip_version(pool: sqlx::SqlitePool) {
    let repo = SqliteCoverLetterVersionRepository::new(pool);
    let version = make_test_version();
    repo.insert(&version).await.unwrap();
    let fetched = repo.get(version.id).await.unwrap();
    assert_eq!(fetched.name, version.name);
    assert_eq!(fetched.content_hash, version.content_hash);
    assert_eq!(fetched.status, VersionStatus::Draft);
}

#[sqlx::test(migrations = "migrations")]
async fn test_sent_record_persists(pool: sqlx::SqlitePool) {
    let version_repo = SqliteCoverLetterVersionRepository::new(pool.clone());
    let sent_repo = SqliteSentRecordRepository::new(pool.clone());
    let version = make_test_version();
    version_repo.insert(&version).await.unwrap();

    let record = SentCoverLetter {
        id: SentRecordId::new(),
        version_id: version.id,
        channel: SentChannel::Email { to: "hr@acme.com".into(), subject: "Application".into() },
        recipient: "hr@acme.com".into(),
        subject: "Application".into(),
        sent_at: Utc::now(),
        confirmation_id: None,
        tracking_status: TrackingStatus::Pending,
    };
    sent_repo.insert(&record).await.unwrap();

    let records = sent_repo.list_for_version(version.id).await.unwrap();
    assert_eq!(records.len(), 1);
}
```

### TUI Tests

```rust
#[test]
fn test_version_diff_panel_renders_diff() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    let diff = VersionDiff {
        from_version: CoverLetterVersionId::new(),
        to_version: CoverLetterVersionId::new(),
        paragraph_changes: vec![
            ParagraphDiff {
                index: 0,
                change_type: ChangeType::Removed,
                before: Some("Old paragraph.".into()),
                after: None,
            },
            ParagraphDiff {
                index: 0,
                change_type: ChangeType::Added,  // this is the replacement
                before: None,
                after: Some("New paragraph.".into()),
            },
        ],
    };

    let panel = VersionDiffPanel { diff: Some(diff), scroll: 0 };
    terminal.draw(|f| {
        panel.render(f.area(), f.buffer_mut());
    }).unwrap();

    let buffer = terminal.backend().buffer().clone();
    // Assert the "-" prefix appears somewhere in the output
    let output: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(output.contains("- Old paragraph."));
    assert!(output.contains("+ New paragraph."));
}
```

---

## Open Questions

1. **Template promotion**: Can a `CoverLetterVersion` be promoted to a reusable template? The spec raises this as open. Recommended approach: add a `is_template: bool` column and a `template_name: Option<String>` in a follow-up migration; `TemplateSelector` reads from the same table with `WHERE is_template = true`.

2. **ATS plain-text variants**: Some ATS portals strip HTML/markdown. Should the system auto-generate a plain-text variant when `SentChannel::CompanyPortal` or `SentChannel::Greenhouse` is selected? Recommended: yes, add `plain_text_content: Option<String>` column and a `strip_markdown()` pass in Phase 2.

3. **Email read receipts**: `TrackingStatus::OpenedRead` and `TrackingStatus::Delivered` imply email tracking pixel or SMTP delivery confirmation. Neither is feasible without a sending server. Phase 1 should treat these as user-settable only (the user manually marks "confirmed delivered" from their email client). Remove `Delivered` and `OpenedRead` from the MVP scope to avoid misleading users.

4. **Multiple applications, same version**: The `cover_letter_application_links` many-to-many table handles this, but `CoverLetterVersion.application_id` is a singular foreign key from the original spec. These two designs are in tension. Recommendation: keep `application_id` as the *primary* link (the application this was originally generated for) and treat `cover_letter_application_links` as the *additional links* table. Document this dual-path clearly in the struct doc comment.

5. **Version ordering in KeepLastN**: "Last N" is ambiguous — does it mean the N most recently *created* or the N most recently *updated*? Recommended: most recently *created*, since that is stable (no writes change `created_at`).

---

## Related Specs

- [`specs/profile-cover-letter-generation.md`](./profile-cover-letter-generation.md) — cover letter generation pipeline
- [`specs/profile-cover-letter-generation-implementation-plan.md`](./profile-cover-letter-generation-implementation-plan.md) — cover letter generation implementation plan (must be implemented first)
- [`specs/application-workflow-actions.md`](./application-workflow-actions.md) — application submission workflow
- [`specs/application-workflow-actions-implementation-plan.md`](./application-workflow-actions-implementation-plan.md) — workflow actions implementation plan
- [`specs/XX-resume-version-management.md`](./XX-resume-version-management.md) — parallel resume version management spec (same structural pattern)
- [`specs/05-gaps-cover-letter-interview-implementation-plan.md`](./05-gaps-cover-letter-interview-implementation-plan.md) — covers `CoverLetterSentStatus` gap (GAP-49)
