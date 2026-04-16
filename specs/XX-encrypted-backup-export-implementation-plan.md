# Implementation Plan: Encrypted Backup and Export

## Status
Draft

## Related Spec
`specs/XX-encrypted-backup-export.md`

## Overview

LazyJob stores sensitive career data — resumes, cover letters, job applications, salary expectations, contact graphs, LLM API keys. If a user's local-first database is damaged, deleted, or if they migrate machines, they have no recourse without a backup system. This plan implements encrypted backup creation, restoration, and portable data export.

The backup format is an `age`-encrypted `.tar.gz` archive containing the raw SQLite database file, any binary attachments (DOCX resume files, PDF cover letters), a JSON manifest with integrity hashes, and a YAML metadata header. Encryption uses the `age` Rust crate (`rage`) with either a passphrase-derived key (Argon2id, same salt infrastructure as the master password system) or a randomly generated key file for cloud backup scenarios where the user stores the key separately.

The data export path is intentionally separate: JSON and CSV exports are **not** encrypted — the user explicitly owns their data at rest in plaintext. An optional encrypted export bundle for migration to another machine (or cold archive) uses the same `age` encryption as the backup format.

Architecture has three layers: (1) `BackupService` in `lazyjob-core` orchestrates archive creation and restoration; (2) `ExportService` collects all structured data from repositories and serializes it; (3) `BackupScheduler` runs as a background tokio task for automatic backups. TUI integration provides a `BackupView` for manual triggers, history browsing, and restoration confirmation dialogs.

## Prerequisites

### Specs That Must Be Implemented First
- `specs/16-privacy-security.md` — `AgeEncryption`, `SecureDelete`, and `CredentialManager` must exist
- `specs/XX-master-password-app-unlock.md` — `Session` and `DerivedKey` types must exist; the Argon2id KDF infrastructure is shared
- `specs/04-sqlite-persistence.md` — `Database` struct and migration runner must exist; `db.path()` must be callable

### Crates to Add

Add to `lazyjob-core/Cargo.toml`:
```toml
age             = { version = "0.10", features = ["armor"] }   # rage Rust implementation
tar             = "0.4"
flate2          = "1.0"
tempfile        = "3"
sha2            = { version = "0.10", features = ["oid"] }
hex             = "0.4"
serde_yaml      = "0.9"
walkdir         = "2"
tokio-util      = { version = "0.7", features = ["io"] }
argon2          = "0.5"    # already present from auth plan, shared
zeroize         = { version = "1.7", features = ["derive"] }
secrecy         = "0.8"
```

Add to `lazyjob-cli/Cargo.toml` (for `lazyjob backup` subcommand):
```toml
clap = { version = "4", features = ["derive"] }
```

## Architecture

### Crate Placement

All backup and export logic lives in **`lazyjob-core/src/backup/`**. The `lazyjob-cli` binary wires up the `lazyjob backup create|restore|list|export` subcommands that call into the core. The TUI backup view (`lazyjob-tui/src/views/backup.rs`) wraps the same `BackupService` API.

Cloud backup integration (Phase 4) lives in a separate `lazyjob-core/src/backup/cloud/` submodule so that cloud-specific dependencies are gated behind feature flags.

### Module Structure

```
lazyjob-core/src/backup/
├── mod.rs               # re-exports; public API surface
├── error.rs             # BackupError enum, Result alias
├── types.rs             # BackupRecord, BackupManifest, BackupMetadata, RestoreReport
├── service.rs           # BackupService: create, restore, list, verify
├── export.rs            # ExportService: JSON/CSV/encrypted bundle export
├── archive.rs           # archive_dir(), extract_archive() helpers
├── encryption.rs        # BackupEncryption wrapper around age crate
├── scheduler.rs         # BackupScheduler: background tokio task
├── cloud/
│   ├── mod.rs           # CloudBackupProvider trait
│   ├── local_fs.rs      # LocalFsProvider (copy to configured path)
│   └── stub.rs          # future: google_drive.rs, dropbox.rs

lazyjob-tui/src/views/
└── backup.rs            # BackupView, RestoreConfirmDialog

lazyjob-cli/src/commands/
└── backup.rs            # BackupCommand enum, CLI dispatch
```

### Core Types

```rust
// lazyjob-core/src/backup/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persisted record of a completed backup, stored in `backup_history` SQLite table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,                          // UUIDv4
    pub created_at: DateTime<Utc>,
    pub path: PathBuf,                       // absolute path on disk
    pub size_bytes: u64,
    pub checksum_sha256: String,             // hex-encoded SHA-256 of the encrypted blob
    pub app_version: String,                 // env!("CARGO_PKG_VERSION")
    pub db_row_counts: DbRowCounts,          // snapshot of row counts at backup time
    pub encryption_mode: BackupEncryptionMode,
    pub verified_at: Option<DateTime<Utc>>,  // last successful integrity verification
}

/// Row counts captured at backup creation time for quick sanity check on restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRowCounts {
    pub jobs: i64,
    pub applications: i64,
    pub contacts: i64,
    pub resume_versions: i64,
    pub cover_letter_versions: i64,
}

/// How the backup blob is encrypted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupEncryptionMode {
    /// Argon2id-derived key from the user's master password (same salt).
    MasterPassword { salt_hex: String },
    /// A randomly generated age key file; the user is responsible for storing it.
    RandomKeyFile { key_file_path: PathBuf },
    /// Unencrypted — only allowed for local export bundles, never for cloud.
    Plaintext,
}

/// Embedded inside the `.tar.gz` before encryption.
/// Written as `manifest.json` in the archive root.
/// This file is encrypted together with the payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub backup_id: String,
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    pub schema_version: u32,               // SQLite user_version pragma value
    pub db_sha256: String,                 // hex SHA-256 of database.sqlite inside archive
    pub attachment_count: usize,
    pub attachment_files: Vec<String>,     // relative paths inside archive
    pub db_row_counts: DbRowCounts,
}

/// Written as `metadata.yaml` at the archive root (also encrypted).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub lazyjob_version: String,
    pub created_at: DateTime<Utc>,
    pub platform: String,                  // e.g. "linux-x86_64"
    pub backup_format_version: u8,         // bump when archive layout changes
}

/// Returned from `BackupService::create()`.
pub struct BackupResult {
    pub record: BackupRecord,
    pub warnings: Vec<String>,
}

/// Returned from `BackupService::restore()`.
pub struct RestoreResult {
    pub jobs_restored: i64,
    pub applications_restored: i64,
    pub contacts_restored: i64,
    pub attachments_restored: usize,
    pub previous_db_saved_at: PathBuf,    // path to the .bak copy of the replaced DB
}

/// Options controlling backup creation.
pub struct BackupOptions {
    pub destination: PathBuf,
    pub encryption: BackupEncryptionMode,
    pub include_attachments: bool,
    pub verify_after_create: bool,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/backup/encryption.rs

use age::secrecy::SecretString;
use std::path::Path;

/// Wraps age encryption/decryption for backup blobs.
pub struct BackupEncryption;

impl BackupEncryption {
    /// Encrypt the file at `src` to `dst` using a passphrase (Argon2id via age).
    pub fn encrypt_with_passphrase(
        src: &Path,
        dst: &Path,
        passphrase: &SecretString,
    ) -> Result<(), BackupError>;

    /// Decrypt the file at `src` to `dst` using a passphrase.
    pub fn decrypt_with_passphrase(
        src: &Path,
        dst: &Path,
        passphrase: &SecretString,
    ) -> Result<(), BackupError>;

    /// Encrypt using a randomly generated age identity written to `key_path`.
    /// Returns the armored public key that must be paired for decryption.
    pub fn encrypt_with_new_random_key(
        src: &Path,
        dst: &Path,
        key_path: &Path,
    ) -> Result<age::x25519::Identity, BackupError>;

    /// Decrypt using a stored age identity file.
    pub fn decrypt_with_key_file(
        src: &Path,
        dst: &Path,
        key_path: &Path,
    ) -> Result<(), BackupError>;
}
```

```rust
// lazyjob-core/src/backup/service.rs

use std::path::Path;
use std::sync::Arc;

pub struct BackupService {
    db: Arc<crate::Database>,
    config: Arc<crate::Config>,
    attachment_dir: PathBuf,
}

impl BackupService {
    pub fn new(
        db: Arc<crate::Database>,
        config: Arc<crate::Config>,
        attachment_dir: PathBuf,
    ) -> Self;

    /// Create an encrypted backup archive at `options.destination`.
    /// Returns a `BackupResult` with the persisted `BackupRecord`.
    pub async fn create(&self, options: BackupOptions) -> Result<BackupResult, BackupError>;

    /// Restore from the encrypted archive at `path`.
    /// Caller is responsible for shutting down Ralph loops before calling.
    /// Saves a `.bak` copy of the current DB before replacing it.
    pub async fn restore(
        &self,
        path: &Path,
        encryption: BackupEncryptionMode,
        passphrase: Option<&SecretString>,
    ) -> Result<RestoreResult, BackupError>;

    /// List all `BackupRecord` entries ordered by `created_at DESC`.
    pub async fn list(&self) -> Result<Vec<BackupRecord>, BackupError>;

    /// Verify the integrity of a backup without restoring it.
    /// Decrypts to a temp dir, checks the manifest SHA-256, runs SQLite
    /// `PRAGMA integrity_check` on the inner DB, then securely deletes the temp dir.
    pub async fn verify(&self, path: &Path, passphrase: Option<&SecretString>) -> Result<(), BackupError>;

    /// Update `backup_history.verified_at` for the given backup id.
    async fn mark_verified(&self, id: &str) -> Result<(), BackupError>;

    async fn capture_row_counts(&self) -> Result<DbRowCounts, BackupError>;

    async fn build_manifest(
        &self,
        backup_id: &str,
        db_path: &Path,
        attachments: &[PathBuf],
    ) -> Result<BackupManifest, BackupError>;
}
```

```rust
// lazyjob-core/src/backup/export.rs

pub struct ExportService {
    db: Arc<crate::Database>,
}

impl ExportService {
    pub fn new(db: Arc<crate::Database>) -> Self;

    /// Export all structured data as a pretty-printed JSON file (NOT encrypted).
    pub async fn export_json(&self, path: &Path) -> Result<ExportResult, BackupError>;

    /// Export tabular data (jobs, applications, contacts) as a ZIP of CSV files (NOT encrypted).
    pub async fn export_csv_zip(&self, path: &Path) -> Result<ExportResult, BackupError>;

    /// Export everything as an encrypted JSON bundle for machine migration.
    /// Uses the same age passphrase encryption as backup blobs.
    pub async fn export_encrypted_bundle(
        &self,
        path: &Path,
        passphrase: &SecretString,
    ) -> Result<ExportResult, BackupError>;

    async fn collect_all_data(&self) -> Result<AllData, BackupError>;
}

/// Root export JSON schema.
#[derive(Serialize, Deserialize)]
pub struct AllData {
    pub exported_at: DateTime<Utc>,
    pub app_version: String,
    pub jobs: Vec<crate::JobRecord>,
    pub applications: Vec<crate::ApplicationRecord>,
    pub contacts: Vec<crate::ProfileContact>,
    pub resume_versions: Vec<crate::ResumeVersion>,
    pub cover_letter_versions: Vec<crate::CoverLetterVersion>,
    pub life_sheet: Option<crate::LifeSheetYaml>,
    pub companies: Vec<crate::CompanyRecord>,
}

pub struct ExportResult {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub row_counts: DbRowCounts,
}
```

```rust
// lazyjob-core/src/backup/cloud/mod.rs

#[async_trait::async_trait]
pub trait CloudBackupProvider: Send + Sync {
    /// Upload the file at `local_path` to the provider.
    /// Returns a provider-specific identifier for the uploaded file.
    async fn upload(&self, local_path: &Path, remote_name: &str) -> Result<String, BackupError>;

    /// List available remote backups, ordered newest-first.
    async fn list_remote(&self) -> Result<Vec<RemoteBackupEntry>, BackupError>;

    /// Download a remote backup to `local_path`.
    async fn download(&self, remote_id: &str, local_path: &Path) -> Result<(), BackupError>;

    /// Delete a remote backup by id.
    async fn delete_remote(&self, remote_id: &str) -> Result<(), BackupError>;

    fn provider_name(&self) -> &'static str;
}

pub struct RemoteBackupEntry {
    pub remote_id: String,
    pub name: String,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
}
```

### SQLite Schema

```sql
-- Migration 020: backup_history
CREATE TABLE IF NOT EXISTS backup_history (
    id                   TEXT PRIMARY KEY,          -- UUIDv4
    created_at           TEXT NOT NULL,             -- ISO-8601 UTC
    path                 TEXT NOT NULL,             -- absolute path on disk
    size_bytes           INTEGER NOT NULL,
    checksum_sha256      TEXT NOT NULL,
    app_version          TEXT NOT NULL,
    db_row_counts_json   TEXT NOT NULL,             -- JSON: DbRowCounts
    encryption_mode_json TEXT NOT NULL,             -- JSON: BackupEncryptionMode
    verified_at          TEXT                       -- ISO-8601 UTC, NULL if never verified
);

CREATE INDEX IF NOT EXISTS idx_backup_history_created_at
    ON backup_history (created_at DESC);

-- Migration 020: backup_schedule config row (one row, upserted)
CREATE TABLE IF NOT EXISTS backup_schedule (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),  -- singleton
    enabled               INTEGER NOT NULL DEFAULT 1,
    frequency_hours       INTEGER NOT NULL DEFAULT 24,
    max_backups_to_keep   INTEGER NOT NULL DEFAULT 10,
    destination_dir       TEXT NOT NULL,             -- user-configured path
    encryption_mode_json  TEXT NOT NULL,
    last_backup_at        TEXT,
    next_backup_at        TEXT
);
```

## Implementation Phases

### Phase 1 — Core Backup and Restore (MVP)

#### Step 1.1 — Define types and error enum

Create `lazyjob-core/src/backup/error.rs`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum BackupError {
    #[error("archive creation failed: {0}")]
    ArchiveCreation(anyhow::Error),

    #[error("encryption failed: {0}")]
    Encryption(anyhow::Error),

    #[error("decryption failed: {context}: {source}")]
    Decryption { context: &'static str, source: anyhow::Error },

    #[error("wrong passphrase")]
    WrongPassphrase,

    #[error("manifest verification failed: {0}")]
    ManifestVerification(String),

    #[error("database integrity check failed: {0}")]
    DbIntegrityFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("no backup found at path: {0}")]
    NotFound(std::path::PathBuf),

    #[error("backup already in progress")]
    AlreadyInProgress,

    #[error("cloud provider error: {0}")]
    CloudProvider(String),
}

pub type Result<T> = std::result::Result<T, BackupError>;
```

Create `lazyjob-core/src/backup/types.rs` with all types above.

**Verification:** `cargo check -p lazyjob-core` compiles cleanly.

#### Step 1.2 — Archive helpers

Create `lazyjob-core/src/backup/archive.rs`:

```rust
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;
use std::path::Path;
use walkdir::WalkDir;

/// Create a `.tar.gz` archive of all files in `src_dir`, rooted under `archive_prefix`.
/// The archive is written to `dst`.
pub fn create_tar_gz(src_dir: &Path, dst: &Path, archive_prefix: &str) -> crate::backup::Result<()> {
    let file = std::fs::File::create(dst)?;
    let gz = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(gz);

    for entry in WalkDir::new(src_dir).min_depth(1) {
        let entry = entry.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let path = entry.path();
        let rel = path.strip_prefix(src_dir)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let archive_path = format!("{}/{}", archive_prefix, rel.display());

        if path.is_file() {
            builder.append_path_with_name(path, &archive_path)?;
        }
    }
    builder.into_inner()?.finish()?;
    Ok(())
}

/// Extract a `.tar.gz` archive into `dst_dir`.
pub fn extract_tar_gz(archive_path: &Path, dst_dir: &Path) -> crate::backup::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(dst_dir)?;
    Ok(())
}
```

**Verification:** Unit test creates a temp dir with 3 files, archives and extracts, confirms file count and content match.

#### Step 1.3 — Backup encryption wrapper

Create `lazyjob-core/src/backup/encryption.rs`. The `age` crate (`rage` in Cargo) provides:
- `age::Encryptor::with_user_passphrase(passphrase: SecretString)` → impl `Write`
- `age::Decryptor::new(reader)` → match on `age::Decryptor::Passphrase(d)`
- `age::x25519::Identity::generate()` for random key mode

```rust
use age::secrecy::SecretString;
use std::{io::{Read, Write}, path::Path};

pub struct BackupEncryption;

impl BackupEncryption {
    pub fn encrypt_with_passphrase(
        src: &Path,
        dst: &Path,
        passphrase: &SecretString,
    ) -> crate::backup::Result<()> {
        let input = std::fs::File::open(src)?;
        let output = std::fs::File::create(dst)?;

        let encryptor = age::Encryptor::with_user_passphrase(passphrase.clone());
        let mut writer = encryptor.wrap_output(
            age::armor::ArmoredWriter::wrap_output(output, age::armor::Format::AsciiArmor)
                .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?,
        )
        .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?;

        let mut buf = Vec::new();
        { let mut f = input; f.read_to_end(&mut buf)?; }
        writer.write_all(&buf)?;
        writer.finish()
            .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?;
        Ok(())
    }

    pub fn decrypt_with_passphrase(
        src: &Path,
        dst: &Path,
        passphrase: &SecretString,
    ) -> crate::backup::Result<()> {
        let input = std::fs::File::open(src)?;
        let reader = age::armor::ArmoredReader::new(input);

        let decryptor = match age::Decryptor::new(reader)
            .map_err(|e| crate::backup::BackupError::Decryption {
                context: "parsing age header",
                source: e.into(),
            })? {
            age::Decryptor::Passphrase(d) => d,
            _ => return Err(crate::backup::BackupError::Decryption {
                context: "expected passphrase-encrypted blob",
                source: anyhow::anyhow!("wrong encryption mode"),
            }),
        };

        let mut reader = decryptor
            .decrypt(passphrase, None)
            .map_err(|e| match e {
                age::DecryptError::DecryptionFailed => crate::backup::BackupError::WrongPassphrase,
                other => crate::backup::BackupError::Decryption {
                    context: "decryption",
                    source: other.into(),
                },
            })?;

        let mut output = std::fs::File::create(dst)?;
        std::io::copy(&mut reader, &mut output)?;
        Ok(())
    }

    pub fn encrypt_with_new_random_key(
        src: &Path,
        dst: &Path,
        key_path: &Path,
    ) -> crate::backup::Result<age::x25519::Identity> {
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public();

        let input = std::fs::File::open(src)?;
        let output = std::fs::File::create(dst)?;

        let encryptor = age::Encryptor::with_recipients(vec![Box::new(recipient)])
            .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?;
        let mut writer = encryptor
            .wrap_output(
                age::armor::ArmoredWriter::wrap_output(output, age::armor::Format::AsciiArmor)
                    .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?,
            )
            .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?;

        let mut buf = Vec::new();
        { let mut f = input; f.read_to_end(&mut buf)?; }
        writer.write_all(&buf)?;
        writer.finish()
            .map_err(|e| crate::backup::BackupError::Encryption(e.into()))?;

        // Write the key file (owner-only permissions)
        let key_content = identity.to_string();
        std::fs::write(key_path, key_content.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(identity)
    }
}
```

**Verification:** Unit test round-trips a 1 MiB random payload through `encrypt_with_passphrase` then `decrypt_with_passphrase` and confirms bytes are identical.

#### Step 1.4 — SHA-256 file hashing helper

```rust
// lazyjob-core/src/backup/archive.rs (add to existing file)

use sha2::{Digest, Sha256};

pub fn sha256_file(path: &Path) -> crate::backup::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}
```

#### Step 1.5 — Apply SQLite migration 020

Add `20_backup.sql` to the migrations directory:

```sql
CREATE TABLE IF NOT EXISTS backup_history ( ... );
CREATE INDEX IF NOT EXISTS idx_backup_history_created_at ON backup_history (created_at DESC);
CREATE TABLE IF NOT EXISTS backup_schedule ( ... );
```

**Verification:** `cargo test -p lazyjob-core -- migration` passes; `PRAGMA user_version` is 20.

#### Step 1.6 — BackupService::create()

```rust
// lazyjob-core/src/backup/service.rs

impl BackupService {
    #[tracing::instrument(skip(self, options))]
    pub async fn create(&self, options: BackupOptions) -> Result<BackupResult> {
        let backup_id = uuid::Uuid::new_v4().to_string();
        let temp_dir = tempfile::tempdir()?;
        let staging = temp_dir.path().join("lazyjob-backup");
        std::fs::create_dir_all(&staging)?;

        // 1. Export SQLite via online backup API
        let db_staging_path = staging.join("database.sqlite");
        self.db.backup_to(&db_staging_path).await
            .map_err(|e| BackupError::ArchiveCreation(e.into()))?;

        // 2. Capture row counts
        let row_counts = self.capture_row_counts().await?;

        // 3. Copy attachments if requested
        let mut attachment_files = Vec::new();
        if options.include_attachments {
            let attach_dst = staging.join("attachments");
            std::fs::create_dir_all(&attach_dst)?;
            for entry in walkdir::WalkDir::new(&self.attachment_dir).min_depth(1) {
                let entry = entry.map_err(|e| BackupError::Io(
                    std::io::Error::new(std::io::ErrorKind::Other, e)
                ))?;
                if entry.file_type().is_file() {
                    let rel = entry.path().strip_prefix(&self.attachment_dir).unwrap();
                    let dst = attach_dst.join(rel);
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(entry.path(), &dst)?;
                    attachment_files.push(format!("attachments/{}", rel.display()));
                }
            }
        }

        // 4. Build manifest and metadata
        let db_sha256 = sha256_file(&db_staging_path)?;
        let manifest = BackupManifest {
            backup_id: backup_id.clone(),
            created_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: self.db.schema_version().await?,
            db_sha256,
            attachment_count: attachment_files.len(),
            attachment_files,
            db_row_counts: row_counts.clone(),
        };
        std::fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        let metadata = BackupMetadata {
            lazyjob_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: Utc::now(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            backup_format_version: 1,
        };
        std::fs::write(
            staging.join("metadata.yaml"),
            serde_yaml::to_string(&metadata).map_err(|e| BackupError::Serialization(e.into()))?,
        )?;

        // 5. Create .tar.gz archive
        let archive_path = temp_dir.path().join("payload.tar.gz");
        create_tar_gz(&staging, &archive_path, "lazyjob-backup")?;

        // 6. Encrypt
        let encrypted_path = &options.destination;
        match &options.encryption {
            BackupEncryptionMode::MasterPassword { .. } => {
                // passphrase is passed in by caller via SecretString
                // BackupOptions should carry Option<SecretString>
                let passphrase = options.passphrase
                    .as_ref()
                    .ok_or_else(|| BackupError::Encryption(anyhow::anyhow!(
                        "passphrase required for MasterPassword mode"
                    )))?;
                BackupEncryption::encrypt_with_passphrase(&archive_path, encrypted_path, passphrase)?;
            }
            BackupEncryptionMode::RandomKeyFile { key_file_path } => {
                BackupEncryption::encrypt_with_new_random_key(
                    &archive_path,
                    encrypted_path,
                    key_file_path,
                )?;
            }
            BackupEncryptionMode::Plaintext => {
                std::fs::copy(&archive_path, encrypted_path)?;
            }
        }

        // 7. Compute final checksum
        let checksum = sha256_file(encrypted_path)?;
        let size_bytes = std::fs::metadata(encrypted_path)?.len();

        // 8. Securely delete staging directory
        for entry in walkdir::WalkDir::new(&staging).min_depth(1).contents_first(true) {
            let entry = entry.map_err(|e| BackupError::Io(
                std::io::Error::new(std::io::ErrorKind::Other, e)
            ))?;
            if entry.file_type().is_file() {
                crate::security::secure_delete(entry.path())?;
            }
        }
        // temp_dir drops here, cleaning up the directory itself

        // 9. Persist BackupRecord
        let record = BackupRecord {
            id: backup_id,
            created_at: Utc::now(),
            path: encrypted_path.clone(),
            size_bytes,
            checksum_sha256: checksum,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            db_row_counts: row_counts,
            encryption_mode: options.encryption.clone(),
            verified_at: None,
        };
        self.persist_record(&record).await?;

        Ok(BackupResult { record, warnings: vec![] })
    }
}
```

**Verification:**
- `cargo test -p lazyjob-core backup::tests::test_create_backup` with a seeded in-memory SQLite and a temp destination directory.
- Inspect output: `file destination.blob` should report an ASCII-armored age file.
- Re-decrypt manually with `rage -d -p` and extract with `tar xzf`.

#### Step 1.7 — BackupService::restore()

```rust
impl BackupService {
    #[tracing::instrument(skip(self, passphrase))]
    pub async fn restore(
        &self,
        path: &Path,
        encryption: BackupEncryptionMode,
        passphrase: Option<&SecretString>,
    ) -> Result<RestoreResult> {
        if !path.exists() {
            return Err(BackupError::NotFound(path.to_path_buf()));
        }

        let temp_dir = tempfile::tempdir()?;

        // 1. Decrypt archive
        let archive_path = temp_dir.path().join("payload.tar.gz");
        match &encryption {
            BackupEncryptionMode::MasterPassword { .. } => {
                let pp = passphrase.ok_or(BackupError::WrongPassphrase)?;
                BackupEncryption::decrypt_with_passphrase(path, &archive_path, pp)?;
            }
            BackupEncryptionMode::RandomKeyFile { key_file_path } => {
                BackupEncryption::decrypt_with_key_file(path, &archive_path, key_file_path)?;
            }
            BackupEncryptionMode::Plaintext => {
                std::fs::copy(path, &archive_path)?;
            }
        }

        // 2. Extract
        let extract_dir = temp_dir.path().join("extracted");
        std::fs::create_dir_all(&extract_dir)?;
        extract_tar_gz(&archive_path, &extract_dir)?;

        let backup_root = extract_dir.join("lazyjob-backup");

        // 3. Verify manifest
        let manifest_bytes = std::fs::read(backup_root.join("manifest.json"))?;
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        let actual_db_sha256 = sha256_file(&backup_root.join("database.sqlite"))?;
        if manifest.db_sha256 != actual_db_sha256 {
            return Err(BackupError::ManifestVerification(format!(
                "database.sqlite SHA-256 mismatch: expected {} got {}",
                manifest.db_sha256, actual_db_sha256
            )));
        }

        // 4. Save current DB as .bak
        let db_path = self.db.path();
        let bak_path = db_path.with_extension("sqlite.bak");
        if db_path.exists() {
            std::fs::copy(&db_path, &bak_path)?;
        }

        // 5. Replace database
        std::fs::copy(backup_root.join("database.sqlite"), &db_path)?;

        // 6. Verify SQLite integrity
        let integrity = self.db.run_integrity_check().await?;
        if integrity != "ok" {
            // Rollback
            std::fs::copy(&bak_path, &db_path)?;
            return Err(BackupError::DbIntegrityFailed(integrity));
        }

        // 7. Restore attachments
        let attachments_dir = backup_root.join("attachments");
        let mut attachments_restored = 0usize;
        if attachments_dir.exists() {
            for entry in walkdir::WalkDir::new(&attachments_dir).min_depth(1) {
                let entry = entry.map_err(|e| BackupError::Io(
                    std::io::Error::new(std::io::ErrorKind::Other, e)
                ))?;
                if entry.file_type().is_file() {
                    let rel = entry.path().strip_prefix(&attachments_dir).unwrap();
                    let dst = self.attachment_dir.join(rel);
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(entry.path(), &dst)?;
                    attachments_restored += 1;
                }
            }
        }

        Ok(RestoreResult {
            jobs_restored: manifest.db_row_counts.jobs,
            applications_restored: manifest.db_row_counts.applications,
            contacts_restored: manifest.db_row_counts.contacts,
            attachments_restored,
            previous_db_saved_at: bak_path,
        })
    }
}
```

**Verification:**
- Round-trip test: create backup → wipe temp DB → restore → confirm row counts match `manifest.db_row_counts`.
- Test wrong passphrase returns `BackupError::WrongPassphrase`.
- Test tampered archive SHA-256 returns `BackupError::ManifestVerification`.

### Phase 2 — Data Export (JSON / CSV)

#### Step 2.1 — ExportService::export_json()

`export_json` calls `collect_all_data()` which queries all repository methods concurrently:

```rust
impl ExportService {
    async fn collect_all_data(&self) -> Result<AllData> {
        let (jobs, applications, contacts, resume_versions, cover_letter_versions, companies) = tokio::try_join!(
            self.db.get_all_jobs(),
            self.db.get_all_applications(),
            self.db.get_all_contacts(),
            self.db.get_all_resume_versions(),
            self.db.get_all_cover_letter_versions(),
            self.db.get_all_companies(),
        ).map_err(|e| BackupError::Database(e))?;

        let life_sheet = self.db.get_life_sheet_yaml().await.ok();

        Ok(AllData {
            exported_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            jobs,
            applications,
            contacts,
            resume_versions,
            cover_letter_versions,
            life_sheet,
            companies,
        })
    }

    pub async fn export_json(&self, path: &Path) -> Result<ExportResult> {
        let data = self.collect_all_data().await?;
        let json = serde_json::to_vec_pretty(&data)?;
        std::fs::write(path, &json)?;
        Ok(ExportResult {
            path: path.to_path_buf(),
            size_bytes: json.len() as u64,
            row_counts: DbRowCounts {
                jobs: data.jobs.len() as i64,
                applications: data.applications.len() as i64,
                contacts: data.contacts.len() as i64,
                resume_versions: data.resume_versions.len() as i64,
                cover_letter_versions: data.cover_letter_versions.len() as i64,
            },
        })
    }
}
```

#### Step 2.2 — ExportService::export_csv_zip()

Uses the `csv 1.3` crate and `zip 2` crate to produce a ZIP file containing per-table CSV files:

```rust
pub async fn export_csv_zip(&self, path: &Path) -> Result<ExportResult> {
    let data = self.collect_all_data().await?;

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Jobs CSV
    zip.start_file("jobs.csv", options)?;
    let mut wtr = csv::Writer::from_writer(std::io::Cursor::new(Vec::new()));
    for job in &data.jobs {
        wtr.serialize(job)?;
    }
    zip.write_all(&wtr.into_inner()?.into_inner())?;

    // Applications CSV (repeat for each table)
    // ...

    let size_bytes = zip.finish()?.metadata()?.len();
    Ok(ExportResult { path: path.to_path_buf(), size_bytes, row_counts: ... })
}
```

Add to `lazyjob-core/Cargo.toml`:
```toml
csv = "1.3"
zip = { version = "2", features = ["deflate"] }
```

#### Step 2.3 — Encrypted export bundle

```rust
pub async fn export_encrypted_bundle(
    &self,
    path: &Path,
    passphrase: &SecretString,
) -> Result<ExportResult> {
    let temp_file = tempfile::NamedTempFile::new()?;
    let result = self.export_json(temp_file.path()).await?;
    BackupEncryption::encrypt_with_passphrase(temp_file.path(), path, passphrase)?;
    crate::security::secure_delete(temp_file.path())?;
    let size = std::fs::metadata(path)?.len();
    Ok(ExportResult { path: path.to_path_buf(), size_bytes: size, row_counts: result.row_counts })
}
```

**Verification:** `export_json` on a seeded DB produces valid JSON that round-trips through `serde_json::from_str::<AllData>`. `export_csv_zip` produces a ZIP with N CSV files (one per entity type) readable by `csv::Reader`.

### Phase 3 — Automatic Backup Scheduler

#### Step 3.1 — BackupScheduler

```rust
// lazyjob-core/src/backup/scheduler.rs

use tokio::time::{interval, Duration, MissedTickBehavior};

pub struct BackupScheduler {
    service: Arc<BackupService>,
    db: Arc<crate::Database>,
}

impl BackupScheduler {
    pub async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut tick = interval(Duration::from_secs(60 * 60)); // check every hour
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = self.maybe_run_backup().await {
                        tracing::warn!("automatic backup check failed: {e}");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() { break; }
                }
            }
        }
    }

    async fn maybe_run_backup(&self) -> crate::backup::Result<()> {
        let schedule = self.db.get_backup_schedule().await?;
        if !schedule.enabled { return Ok(()); }

        let now = Utc::now();
        if let Some(next) = schedule.next_backup_at {
            if now < next { return Ok(()); }
        }

        let options = BackupOptions {
            destination: schedule.destination_dir.join(
                format!("lazyjob-backup-{}.blob", now.format("%Y%m%d-%H%M%S"))
            ),
            encryption: serde_json::from_str(&schedule.encryption_mode_json)?,
            include_attachments: true,
            verify_after_create: true,
            passphrase: None, // scheduler cannot prompt; uses keyring-stored passphrase
        };

        let result = self.service.create(options).await?;
        tracing::info!(
            backup_id = %result.record.id,
            size_bytes = result.record.size_bytes,
            "automatic backup created"
        );

        self.prune_old_backups(schedule.max_backups_to_keep).await?;
        Ok(())
    }

    async fn prune_old_backups(&self, keep: i64) -> crate::backup::Result<()> {
        let all = self.service.list().await?;
        let to_delete = all.into_iter().skip(keep as usize).collect::<Vec<_>>();
        for record in to_delete {
            if record.path.exists() {
                crate::security::secure_delete(&record.path)?;
            }
            self.db.delete_backup_record(&record.id).await?;
        }
        Ok(())
    }
}
```

The scheduler reads the passphrase from the OS keyring using the existing `CredentialManager` (established in spec 16). If the keyring entry is absent (no cached master password), the scheduler logs a warning and skips the backup.

**Verification:** Integration test mocks time to be past `next_backup_at`, calls `maybe_run_backup`, confirms `backup_history` has a new row and the old backup file is deleted when count exceeds `max_backups_to_keep`.

### Phase 4 — CLI Subcommands

#### Step 4.1 — `lazyjob backup` subcommands

```rust
// lazyjob-cli/src/commands/backup.rs

#[derive(clap::Subcommand)]
pub enum BackupCommand {
    /// Create an encrypted backup
    Create {
        #[arg(long, default_value = "~/.config/lazyjob/backups")]
        output: PathBuf,
        #[arg(long, default_value = "master-password")]
        encryption: String,  // "master-password" or "random-key"
    },
    /// Restore from a backup file
    Restore {
        path: PathBuf,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List all backups
    List,
    /// Verify backup integrity without restoring
    Verify {
        path: PathBuf,
    },
    /// Export data in portable formats
    Export {
        #[arg(long, default_value = "json")]
        format: String,   // "json", "csv", "encrypted-json"
        output: PathBuf,
    },
}
```

Each subcommand:
1. Prompts for master password via `rpassword::read_password()` (stdin, no echo).
2. Calls the appropriate `BackupService` or `ExportService` method.
3. Prints a summary (bytes written, counts, path).

Add `rpassword = "7"` to `lazyjob-cli/Cargo.toml`.

**Verification:** `lazyjob backup create --output /tmp/test.blob` creates an age-encrypted file; `lazyjob backup restore /tmp/test.blob` replaces the DB and reports counts.

#### Step 4.2 — Backup verify command

```
$ lazyjob backup verify ~/.config/lazyjob/backups/lazyjob-backup-20260416-120000.blob
Enter master password: ****
✓ Decryption successful
✓ Manifest SHA-256 verified (db_sha256: a1b2c3...)
✓ SQLite integrity check: ok
✓ Row counts: 124 jobs, 37 applications, 89 contacts
Last backup: 2026-04-16 12:00:00 UTC
```

### Phase 5 — TUI Backup View

#### Step 5.1 — BackupView widget

```rust
// lazyjob-tui/src/views/backup.rs

pub struct BackupView {
    backup_list: Vec<BackupRecord>,
    list_state: ratatui::widgets::ListState,
    mode: BackupViewMode,
}

pub enum BackupViewMode {
    Browsing,
    ConfirmRestore { record: BackupRecord },
    Creating,
    Verifying { record_id: String },
}

impl BackupView {
    pub fn render(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        // 60/40 horizontal split: backup list | detail panel
        use ratatui::layout::{Constraint, Direction, Layout};
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        self.render_list(frame, chunks[0]);
        self.render_detail(frame, chunks[1]);
    }

    fn render_list(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let items: Vec<ratatui::widgets::ListItem> = self.backup_list.iter().map(|r| {
            let label = format!(
                "{} {:>8} SHA:{}",
                r.created_at.format("%Y-%m-%d %H:%M"),
                format_bytes(r.size_bytes),
                &r.checksum_sha256[..8],
            );
            ratatui::widgets::ListItem::new(label)
        }).collect();

        let list = ratatui::widgets::List::new(items)
            .highlight_style(ratatui::style::Style::default().add_modifier(
                ratatui::style::Modifier::REVERSED
            ));
        frame.render_stateful_widget(list, area, &mut self.list_state.clone());
    }

    fn render_detail(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        if let Some(idx) = self.list_state.selected() {
            if let Some(record) = self.backup_list.get(idx) {
                let text = vec![
                    format!("ID: {}", &record.id[..8]),
                    format!("Created: {}", record.created_at),
                    format!("Size: {}", format_bytes(record.size_bytes)),
                    format!("Jobs: {}", record.db_row_counts.jobs),
                    format!("Applications: {}", record.db_row_counts.applications),
                    format!("Verified: {}", record.verified_at.map(|t| t.to_string()).unwrap_or_else(|| "never".to_string())),
                ];
                let paragraph = ratatui::widgets::Paragraph::new(
                    text.join("\n")
                ).block(ratatui::widgets::Block::default().title("Backup Details").borders(ratatui::widgets::Borders::ALL));
                frame.render_widget(paragraph, area);
            }
        }
    }
}
```

Keybindings:
- `j`/`k` — navigate list
- `c` — create new backup (prompts for passphrase in a masked input)
- `r` — restore selected (shows `RestoreConfirmDialog`)
- `v` — verify selected backup
- `d` — delete selected backup (with confirmation prompt)
- `e` — export submenu (json / csv / encrypted-json)

#### Step 5.2 — RestoreConfirmDialog

Before restoring, show a full-screen `Clear`-backed overlay warning:

```
╔══════════════════════════════════════════════════════╗
║  WARNING: Restore Backup?                            ║
║                                                      ║
║  This will REPLACE your current database with:      ║
║  Backup from 2026-04-15 10:22 UTC                   ║
║  124 jobs · 37 applications · 89 contacts            ║
║                                                      ║
║  Your current database will be saved to:             ║
║  ~/.config/lazyjob/lazyjob.sqlite.bak                ║
║                                                      ║
║  All running Ralph loops will be terminated.         ║
║                                                      ║
║  Type "restore" to confirm, or press Esc to cancel.  ║
╚══════════════════════════════════════════════════════╝
```

The dialog requires the user to type "restore" (not just press Enter) to prevent accidental data loss.

**Verification:** TUI snapshot test (ratatui `TestBackend`) confirms dialog renders correctly. Manual test: confirm that cancelling leaves the database unchanged.

### Phase 6 — Cloud Backup Integration (Optional)

#### Step 6.1 — LocalFsProvider (MVP stand-in)

```rust
// lazyjob-core/src/backup/cloud/local_fs.rs

pub struct LocalFsProvider {
    pub base_dir: PathBuf,
}

#[async_trait::async_trait]
impl CloudBackupProvider for LocalFsProvider {
    async fn upload(&self, local_path: &Path, remote_name: &str) -> Result<String, BackupError> {
        let dst = self.base_dir.join(remote_name);
        std::fs::copy(local_path, &dst)?;
        Ok(remote_name.to_string())
    }

    async fn list_remote(&self) -> Result<Vec<RemoteBackupEntry>, BackupError> {
        let mut entries = Vec::new();
        for e in std::fs::read_dir(&self.base_dir)? {
            let e = e?;
            let meta = e.metadata()?;
            entries.push(RemoteBackupEntry {
                remote_id: e.file_name().to_string_lossy().to_string(),
                name: e.file_name().to_string_lossy().to_string(),
                size_bytes: meta.len(),
                created_at: meta.modified()?.into(),
            });
        }
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entries)
    }

    async fn download(&self, remote_id: &str, local_path: &Path) -> Result<(), BackupError> {
        std::fs::copy(self.base_dir.join(remote_id), local_path)?;
        Ok(())
    }

    async fn delete_remote(&self, remote_id: &str) -> Result<(), BackupError> {
        crate::security::secure_delete(&self.base_dir.join(remote_id))
    }

    fn provider_name(&self) -> &'static str { "local-fs" }
}
```

Phase 6 real cloud providers (Google Drive, Dropbox) are gated behind Cargo features:

```toml
[features]
cloud-google-drive = ["google-drive3", "yup-oauth2"]
cloud-dropbox = ["dropbox-sdk"]
```

These remain stubbed until Phase 6. `BackupScheduler` always calls the `CloudBackupProvider` trait — the concrete implementation is injected at startup from config.

## Key Crate APIs

| Operation | Crate | API |
|---|---|---|
| Age encryption (passphrase) | `age` | `age::Encryptor::with_user_passphrase(SecretString)` |
| Age encryption (recipient) | `age` | `age::Encryptor::with_recipients(Vec<Box<dyn Recipient>>)` |
| Age decryption | `age` | `age::Decryptor::new(reader)` → `Decryptor::Passphrase(d)` → `d.decrypt(passphrase, max_work_factor)` |
| Age identity generation | `age` | `age::x25519::Identity::generate()` |
| Armor wrapping | `age` | `age::armor::ArmoredWriter::wrap_output(w, Format::AsciiArmor)` |
| Tar archive creation | `tar` | `tar::Builder::new(w).append_path_with_name(path, name)` |
| Gzip compression | `flate2` | `flate2::write::GzEncoder::new(w, Compression::default())` |
| SHA-256 file hash | `sha2` | `sha2::Sha256::new()` + `std::io::copy` |
| Directory traversal | `walkdir` | `walkdir::WalkDir::new(dir).min_depth(1)` |
| Temp files/dirs | `tempfile` | `tempfile::tempdir()`, `tempfile::NamedTempFile::new()` |
| CSV writing | `csv` | `csv::Writer::from_writer(w).serialize(record)` |
| ZIP archive | `zip` | `zip::ZipWriter::new(f).start_file(name, options)` |
| SQLite online backup | `rusqlite` | `Connection::backup(DatabaseName::Main, dst_path, None)` |
| SQLite integrity check | `rusqlite` | `conn.query_row("PRAGMA integrity_check", [], ...)` |
| Password prompt (CLI) | `rpassword` | `rpassword::read_password()` |
| Hex encoding | `hex` | `hex::encode(bytes)` |
| YAML serialization | `serde_yaml` | `serde_yaml::to_string(&val)` |

## SQLite Online Backup API

The SQLite C API provides `sqlite3_backup_init/step/finish` for hot backups while the DB is in use. `rusqlite` exposes this as `Connection::backup()`:

```rust
impl Database {
    pub async fn backup_to(&self, dst: &Path) -> anyhow::Result<()> {
        let dst_path = dst.to_owned();
        let conn = self.pool.get().await?;
        tokio::task::spawn_blocking(move || {
            let dst_conn = rusqlite::Connection::open(&dst_path)?;
            let backup = rusqlite::backup::Backup::new(&conn, &dst_conn)?;
            backup.run_to_completion(5, std::time::Duration::from_millis(250), None)?;
            Ok::<_, anyhow::Error>(())
        }).await??;
        Ok(())
    }

    pub async fn run_integrity_check(&self) -> anyhow::Result<String> {
        let conn = self.pool.get().await?;
        let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok(result)
    }
}
```

This is safe under concurrent reads/writes — SQLite WAL mode ensures consistency.

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum BackupError {
    #[error("archive creation failed: {0}")]
    ArchiveCreation(#[source] anyhow::Error),

    #[error("encryption failed: {0}")]
    Encryption(#[source] anyhow::Error),

    #[error("decryption failed at {context}: {source}")]
    Decryption { context: &'static str, source: anyhow::Error },

    #[error("wrong passphrase")]
    WrongPassphrase,

    #[error("manifest verification failed: {0}")]
    ManifestVerification(String),

    #[error("database integrity check failed: {0}")]
    DbIntegrityFailed(String),

    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("json serialization error")]
    Json(#[from] serde_json::Error),

    #[error("database error")]
    Database(#[from] rusqlite::Error),

    #[error("backup not found: {0}")]
    NotFound(std::path::PathBuf),

    #[error("backup already in progress")]
    AlreadyInProgress,

    #[error("cloud provider: {0}")]
    CloudProvider(String),

    #[error("feature not available: {0}")]
    NotAvailable(&'static str),
}

pub type Result<T> = std::result::Result<T, BackupError>;
```

The `WrongPassphrase` variant is deliberately opaque — the TUI renders it as "Incorrect passphrase" with no further detail to prevent oracle attacks. `ManifestVerification` and `DbIntegrityFailed` include the actual mismatch details so the user can diagnose a corrupted archive.

## Testing Strategy

### Unit Tests

**`backup::tests::test_archive_round_trip`**
- Create a temp dir with 5 files of various sizes.
- Call `create_tar_gz` → `extract_tar_gz`.
- Assert all 5 files are present in the extraction dir with identical content.

**`backup::tests::test_encrypt_decrypt_passphrase`**
- Generate 1 MiB of random bytes.
- `encrypt_with_passphrase` to a temp file.
- `decrypt_with_passphrase` with the same passphrase.
- Assert bytes are identical.
- Call `decrypt_with_passphrase` with a wrong passphrase.
- Assert `BackupError::WrongPassphrase`.

**`backup::tests::test_sha256_stability`**
- Write a known byte sequence to a file.
- Assert `sha256_file` produces the expected hex string.

**`backup::tests::test_manifest_verification`**
- Build a `BackupManifest` with a known `db_sha256`.
- Write a temp DB file with that SHA-256 prefix.
- Call `restore()` with a tampered manifest (wrong `db_sha256`).
- Assert `BackupError::ManifestVerification`.

**`backup::tests::test_export_json_round_trip`**
- Use `#[sqlx::test]` with a seeded DB.
- `export_json` to a temp file.
- Parse back with `serde_json::from_str::<AllData>`.
- Assert `jobs.len()` matches the seeded count.

**`backup::tests::test_export_csv_zip`**
- `export_csv_zip` to a temp file.
- Open with `zip::ZipArchive`.
- Assert the archive contains `jobs.csv`, `applications.csv`, `contacts.csv`.
- Parse `jobs.csv` with `csv::Reader` and assert row count matches seed.

### Integration Tests

**`backup::integration::test_full_backup_restore_cycle`**
- Seed a SQLite database with 10 jobs, 3 applications, 5 contacts.
- `BackupService::create` to a temp encrypted blob.
- Delete all rows from the DB.
- `BackupService::restore` from the blob.
- Assert row counts are back to 10 / 3 / 5.

**`backup::integration::test_automatic_pruning`**
- Create 12 backups with a `max_backups_to_keep = 10` schedule.
- After the 12th creation, call `prune_old_backups(10)`.
- Assert `backup_history` table has exactly 10 rows.
- Assert the 2 oldest files no longer exist on disk.

**`backup::integration::test_verify_command`**
- Create a valid backup.
- `BackupService::verify` — assert `Ok(())`.
- Corrupt the encrypted blob (flip 10 bytes in the middle).
- `BackupService::verify` — assert `Err(BackupError::Decryption { .. })` or `WrongPassphrase`.

### TUI Tests

- Snapshot test of `BackupView::render` with 3 seeded `BackupRecord`s using ratatui `TestBackend` (16×80).
- Snapshot test of `RestoreConfirmDialog` with a known record.
- Event simulation: pressing `j` advances `list_state.selected` by 1.
- Event simulation: typing "restore" in the confirm dialog and pressing Enter triggers `BackupService::restore`.

## Open Questions

1. **Key escrow / recovery key**: Should LazyJob generate a 24-word BIP-39 mnemonic as a backup recovery code? This would require the `bip39` crate and a dedicated recovery flow. Deferred to post-MVP.

2. **Incremental backup**: Full SQLite copy on every backup is safe but wastes space for large attachment dirs. SQLite WAL snapshots or page-level incremental backup (using SQLite's `sqlite3_backup_step` with page iteration) are future optimizations.

3. **Backup retention policy**: The spec lists the `max_backups_to_keep` config but does not specify time-based retention (e.g., "keep daily for 7 days, weekly for 4 weeks"). A simple `keep N most recent` is implemented in Phase 3. Time-based retention is Phase 6.

4. **Passphrase caching for automatic backups**: The `BackupScheduler` needs the master password to encrypt automatic backups. Currently it reads from the OS keyring via `CredentialManager`. If the app is locked (Session expired), the automatic backup is skipped. An alternative: derive the backup key during app unlock and persist it in the keyring separately from the master password. Needs UX decision.

5. **Temp file location**: The spec mentions "RAM-backed filesystem if available". On Linux, `/dev/shm` or `tmpfs` can be used for staging. Implementing this as `TMPDIR=/dev/shm tempfile::tempdir()` is simple but not portable. Platform-specific temp dir selection is a Phase 3 enhancement.

6. **Backup format versioning**: `BackupMetadata.backup_format_version = 1`. When the archive layout changes (e.g., adding new subdirectories), `restore()` must check this field and apply version-specific extraction logic. A `BackupFormatMigrator` registry should be added in Phase 3 before the format stabilizes.

7. **Post-quantum encryption**: The spec mentions `-pq` flag for post-quantum hybrid keys. The `age` Rust crate (`rage`) does not yet support the hybrid post-quantum ML-KEM extension. Deferred to when `rage` ships support.

## Related Specs

- `specs/16-privacy-security.md` — `AgeEncryption`, `SecureDelete`, `PrivacyMode` — encryption primitives reused by this plan
- `specs/XX-master-password-app-unlock.md` — `Session`, `DerivedKey`, Argon2id KDF — key derivation shared
- `specs/04-sqlite-persistence.md` — `Database` struct and `backup_to()` method — online SQLite backup
- `specs/10-gaps-saas-mvp.md` — `SqliteDataExporter` — the SaaS export path shares `AllData` types with this plan's `ExportService`
- `specs/18-saas-migration-path.md` — cloud sync and backup overlap; this plan owns the local backup format, the SaaS plan owns the cloud sync protocol
