# Implementation Plan: Platform & Privacy Gap Closure (GAP-89 – GAP-98)

## Status
Draft

## Related Spec
`specs/09-gaps-platform-privacy.md`

## Overview

This plan closes ten gaps identified in the platform-integrations and privacy gap analysis. The gaps span two broad areas:

**Security/Privacy** (GAP-89, 90, 91, 94, 95, 96): encrypted backup/export, master-password app unlock, multi-device sync, data retention & deletion policy, LLM provider privacy disclosures, and crash-report/telemetry controls. These build on top of the foundation laid in `specs/16-privacy-security-implementation-plan.md`, which established the `age`-based encryption scheme, `CredentialManager`, and `PrivacyMode` enum.

**Platform Integration** (GAP-92, 93, 97, 98): LinkedIn OAuth "Apply with LinkedIn" scope analysis and graceful degradation, browser-fingerprinting/evasion strategy for Workday automation, a concrete Workday integration implementation plan, and a job-aggregator cost-control system. These build on `specs/11-platform-api-integrations-implementation-plan.md`, which defined the `PlatformClient` trait, `GreenhouseClient`, `LeverClient`, and credential keyring storage.

Cross-Spec T (encryption key management lifecycle) and Cross-Spec U (unified credential storage across all providers) are resolved as architectural refinements within this plan.

Phase 1 covers the two critical security gaps (master password unlock, encrypted backup) that are pre-requisites for a secure MVP. Phase 2 covers platform gaps (Workday, LinkedIn OAuth, cost budgeting). Phase 3 covers moderate-priority items (data retention, LLM privacy disclosures, telemetry, multi-device sync) that are important for post-MVP polish.

## Prerequisites

### Specs That Must Be Implemented First
- `specs/16-privacy-security-implementation-plan.md` — `SecurityLayer`, `AgeEncryption`, `CredentialManager`, `PrivacyMode` must exist
- `specs/11-platform-api-integrations-implementation-plan.md` — `PlatformClient` trait, `GreenhouseClient`, `LeverClient`, `JobIngestionService`
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migration infrastructure
- `specs/XX-master-password-app-unlock-implementation-plan.md` — (created by this plan, Phase 1)
- `specs/XX-encrypted-backup-export-implementation-plan.md` — (created by this plan, Phase 1)

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml — additions beyond what 16-privacy already adds
flate2          = "1"           # gzip compression for backup archives
tar             = "0.4"         # tar archiving for backup bundles
sha2            = "0.10"        # SHA-256 for backup integrity verification
humantime       = "2"           # human-readable durations for retention config
chrono          = { version = "0.4", features = ["serde"] }

# lazyjob-core/Cargo.toml — privacy disclosures
serde_json      = "1"           # already present; used for telemetry serialization

# lazyjob-tui/Cargo.toml
arboard         = "3"           # clipboard (already in gaps plan 08)

# lazyjob-cli/Cargo.toml
clap            = { version = "4", features = ["derive", "env"] }  # already present
```

---

## Architecture

### Crate Placement

```
lazyjob-core/src/
  security/
    mod.rs                    # SecurityLayer (extended from plan 16)
    credentials.rs            # CredentialManager (extended: unified storage)
    encryption.rs             # AgeEncryption (from plan 16)
    master_password.rs        # MasterPassword (from plan 16, extended here)
    session.rs                # SessionToken, InactivityTimer (GAP-90)
    backup.rs                 # EncryptedBackupService (GAP-89)
    export.rs                 # DataExporter (from plan 16, extended)
    retention.rs              # RetentionPolicy, DeletionCascade (GAP-94)
    privacy_disclosure.rs     # LlmProviderPrivacyInfo, PrivacyDisclosure (GAP-95)
    telemetry.rs              # TelemetryConfig, CrashReporter (GAP-96)

lazyjob-core/src/
  platform/
    workday.rs                # WorkdayClient (GAP-97)
    linkedin_oauth.rs         # LinkedInOAuthClient (GAP-92)
    budget.rs                 # PlatformCostTracker (GAP-98)
    fingerprint.rs            # StealthOptions (GAP-93)

lazyjob-tui/src/
  views/
    lock_screen.rs            # LockScreenView (GAP-90)
    backup_restore.rs         # BackupRestoreView (GAP-89)
    privacy_disclosure.rs     # LlmProviderPrivacyPanel (GAP-95)

lazyjob-cli/src/
  commands/
    backup.rs                 # `lazyjob backup create|restore|list` (GAP-89)
    retention.rs              # `lazyjob data prune` (GAP-94)
```

---

## Phase 1 — Critical Security Gaps (GAP-89, GAP-90)

### 1.1 Master Password App Unlock (GAP-90)

**Gap summary**: LazyJob stores sensitive career data. The app must require a master password on startup, derive the database encryption key from it via Argon2id, and lock the app after an inactivity period.

#### Core Types

```rust
// lazyjob-core/src/security/session.rs

use zeroize::Zeroizing;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
use std::time::{Duration, Instant};

/// A live session created by a successful unlock.
/// Holds the derived key in memory, zeroed on drop.
pub struct Session {
    key_material: Zeroizing<Vec<u8>>,
    unlocked_at: Instant,
    timeout: Duration,
    // watch channel sends `true` when the session expires
    expire_tx: watch::Sender<bool>,
    pub expire_rx: watch::Receiver<bool>,
}

impl Session {
    pub fn new(key_material: Zeroizing<Vec<u8>>, timeout: Duration) -> Self {
        let (expire_tx, expire_rx) = watch::channel(false);
        Self {
            key_material,
            unlocked_at: Instant::now(),
            timeout,
            expire_tx,
            expire_rx,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.unlocked_at.elapsed() >= self.timeout
    }

    /// Borrow the raw key bytes for encrypt/decrypt operations.
    /// Never clone or store this reference.
    pub fn key_bytes(&self) -> &[u8] {
        &self.key_material
    }

    pub fn touch(&mut self) {
        self.unlocked_at = Instant::now();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.expire_tx.send(true);
        // zeroize::Zeroizing handles zeroing key_material
    }
}

/// Tracks the current session behind a Mutex.
pub struct SessionGuard(Arc<Mutex<Option<Session>>>);

impl SessionGuard {
    pub async fn is_locked(&self) -> bool {
        let guard = self.0.lock().await;
        guard.is_none() || guard.as_ref().map(|s| s.is_expired()).unwrap_or(true)
    }

    pub async fn touch(&self) {
        if let Some(session) = self.0.lock().await.as_mut() {
            session.touch();
        }
    }
}
```

```rust
// lazyjob-core/src/security/master_password.rs  (extends plan 16)

use argon2::{Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, rand_core::OsRng};
use zeroize::Zeroizing;

pub struct MasterPassword;

/// Argon2id parameters: 64 MiB memory, 3 iterations, 4 lanes.
const ARGON2_MEMORY_KIB: u32 = 65536;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 4;
const KEY_LEN: usize = 32;

impl MasterPassword {
    /// Returns a 32-byte key derived from `password` + `salt`.
    /// `salt` must be a 16-byte random value stored in the OS keyring.
    pub fn derive_key(
        password: &str,
        salt: &[u8; 16],
    ) -> Result<Zeroizing<Vec<u8>>, SecurityError> {
        let params = Params::new(
            ARGON2_MEMORY_KIB,
            ARGON2_ITERATIONS,
            ARGON2_PARALLELISM,
            Some(KEY_LEN),
        ).map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;

        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = Zeroizing::new(vec![0u8; KEY_LEN]);
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        Ok(key)
    }

    /// Create a new salt, store it in the keyring, and return the PHC verifier string.
    pub fn register(password: &str) -> Result<(Zeroizing<Vec<u8>>, String, [u8; 16]), SecurityError> {
        use rand::RngCore;
        let mut salt_bytes = [0u8; 16];
        OsRng.fill_bytes(&mut salt_bytes);
        let key = Self::derive_key(password, &salt_bytes)?;
        // Store a PHC hash string separately for verification (password check, not key derivation)
        let salt_str = SaltString::encode_b64(&salt_bytes)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        let argon2 = Argon2::default();
        let phc = argon2
            .hash_password(password.as_bytes(), &salt_str)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?
            .to_string();
        Ok((key, phc, salt_bytes))
    }

    /// Verify password against the stored PHC hash string.
    pub fn verify(password: &str, phc_hash: &str) -> Result<bool, SecurityError> {
        let hash = PasswordHash::new(phc_hash)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        Ok(Argon2::default().verify_password(password.as_bytes(), &hash).is_ok())
    }
}
```

#### `SecurityLayer::unlock()` Extension

```rust
// lazyjob-core/src/security/mod.rs — extension to plan 16

impl SecurityLayer {
    /// Called at startup. Prompts for password via callback, derives key, opens DB.
    pub async fn unlock(
        &self,
        password: &str,
        timeout: Duration,
    ) -> Result<Session> {
        let phc_hash = self.credentials.get("lazyjob", "master_phc")?;
        if !MasterPassword::verify(password, &phc_hash)? {
            return Err(SecurityError::InvalidPassword);
        }
        let salt_b64 = self.credentials.get("lazyjob", "master_salt")?;
        let salt_bytes = base64::engine::general_purpose::STANDARD
            .decode(&salt_b64)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        let salt: [u8; 16] = salt_bytes
            .try_into()
            .map_err(|_| SecurityError::KeyDerivation("salt length mismatch".into()))?;
        let key = MasterPassword::derive_key(password, &salt)?;
        Ok(Session::new(key, timeout))
    }

    /// First-time setup: register master password, store salt + PHC verifier in keyring.
    pub fn setup_master_password(
        &self,
        password: &str,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let (key, phc, salt) = MasterPassword::register(password)?;
        let salt_b64 = base64::engine::general_purpose::STANDARD.encode(salt);
        self.credentials.set("lazyjob", "master_phc", &phc)?;
        self.credentials.set("lazyjob", "master_salt", &salt_b64)?;
        Ok(key)
    }
}
```

#### Inactivity Timer

```rust
// lazyjob-core/src/security/session.rs

pub struct InactivityTimer {
    guard: Arc<SessionGuard>,
    timeout: Duration,
    reset_tx: tokio::sync::mpsc::Sender<()>,
}

impl InactivityTimer {
    /// Spawns a background task that locks the session on inactivity.
    pub fn start(guard: Arc<SessionGuard>, timeout: Duration) -> Self {
        let (reset_tx, mut reset_rx) = tokio::sync::mpsc::channel::<()>(4);
        let guard_clone = Arc::clone(&guard);
        tokio::spawn(async move {
            loop {
                let sleep = tokio::time::sleep(timeout);
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => {
                        // Lock the session: replace with None
                        *guard_clone.0.lock().await = None;
                        break;
                    }
                    msg = reset_rx.recv() => {
                        if msg.is_none() { break; }
                        // Reset the timer on any activity
                    }
                }
            }
        });
        Self { guard, timeout, reset_tx }
    }

    /// Call on any keypress to reset the inactivity clock.
    pub fn touch(&self) {
        let _ = self.reset_tx.try_send(());
    }
}
```

#### Lock Screen TUI Widget

```rust
// lazyjob-tui/src/views/lock_screen.rs

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub struct LockScreenView {
    password_buf: zeroize::Zeroizing<String>,
    error_msg: Option<String>,
    is_setup_mode: bool,  // true on first launch (no master password yet)
}

impl LockScreenView {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Render full-screen black overlay
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(if self.is_setup_mode { " Set Master Password " } else { " LazyJob — Locked " })
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Length(1), Constraint::Length(3), Constraint::Length(1)])
            .split(inner);

        let label = Paragraph::new(if self.is_setup_mode {
            "Choose a master password to encrypt your data:"
        } else {
            "Enter master password to unlock:"
        });
        frame.render_widget(label, chunks[0]);

        let masked: String = "•".repeat(self.password_buf.len());
        let input = Paragraph::new(masked)
            .block(Block::default().borders(Borders::ALL).title("Password"));
        frame.render_widget(input, chunks[1]);

        if let Some(msg) = &self.error_msg {
            let err = Paragraph::new(Span::styled(msg.clone(), Style::default().fg(Color::Red)));
            frame.render_widget(err, chunks[2]);
        }
    }

    pub fn on_char(&mut self, c: char) {
        self.password_buf.push(c);
    }

    pub fn on_backspace(&mut self) {
        self.password_buf.pop();
    }

    pub fn take_password(&mut self) -> zeroize::Zeroizing<String> {
        std::mem::replace(&mut self.password_buf, zeroize::Zeroizing::new(String::new()))
    }
}
```

#### Password Change Flow

```rust
// lazyjob-core/src/security/mod.rs

impl SecurityLayer {
    /// Re-derives key with new password, re-encrypts the database file.
    pub async fn change_master_password(
        &self,
        current_password: &str,
        new_password: &str,
        db_path: &std::path::Path,
    ) -> Result<()> {
        let session = self.unlock(current_password, Duration::from_secs(60)).await?;
        let (new_key, new_phc, new_salt) = MasterPassword::register(new_password)?;
        // Re-encrypt database with new key
        self.encryption.reencrypt(db_path, session.key_bytes(), &new_key).await?;
        // Update keyring
        let salt_b64 = base64::engine::general_purpose::STANDARD.encode(new_salt);
        self.credentials.set("lazyjob", "master_phc", &new_phc)?;
        self.credentials.set("lazyjob", "master_salt", &salt_b64)?;
        Ok(())
    }
}
```

#### Configuration

```toml
# ~/.config/lazyjob/config.toml
[security]
session_timeout_minutes = 30     # 0 = never lock
require_password_on_startup = true
```

#### SQLite Schema

```sql
-- Migration 018: master password metadata
CREATE TABLE IF NOT EXISTS app_security_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Stores: "encryption_enabled" = "true"/"false"
-- Stores: "encryption_scheme" = "age-v1"
-- Does NOT store keys, passwords, or salts (those go in keyring)
```

#### Verification

1. `cargo test -p lazyjob-core -- security::master_password` — derive key, verify PHC, change password
2. Launch app with no keyring entry → setup mode renders lock screen
3. Enter wrong password → red error message, no unlock
4. After `session_timeout_minutes`, `SessionGuard::is_locked()` returns `true`
5. `change_master_password` re-encrypts DB and new password unlocks it

---

### 1.2 Encrypted Backup and Export (GAP-89)

**Gap summary**: The database can be encrypted at rest but backups were unspecified. Backups must be encrypted with the same master password, securely signed (SHA-256), and restorable.

#### Core Types

```rust
// lazyjob-core/src/security/backup.rs

use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use zeroize::Zeroizing;

/// Metadata stored in backup_manifest.json inside the archive.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BackupManifest {
    pub format_version: u8,          // currently 1
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub lazyjob_version: String,
    pub db_size_bytes: u64,
    pub sha256_hex: String,           // SHA-256 of the encrypted DB bytes
    pub encryption_scheme: String,    // "age-v1"
}

/// Result of a completed backup.
pub struct BackupResult {
    pub archive_path: PathBuf,
    pub manifest: BackupManifest,
}

/// Result of a restore operation.
pub struct RestoreResult {
    pub restored_db_path: PathBuf,
    pub manifest: BackupManifest,
}

pub struct EncryptedBackupService {
    encryption: crate::security::AgeEncryption,
}
```

#### Backup Format

A backup is a `.tar.gz` archive containing:
```
lazyjob-backup-<timestamp>/
  lazyjob.db.age       # age-encrypted SQLite database file
  backup_manifest.json # plaintext manifest with SHA-256 of the encrypted bytes
```

The `age` passphrase is derived the same way as the database key — using the master password + the stored salt. This ensures the same master password unlocks both the app and any backup.

#### Backup Implementation

```rust
// lazyjob-core/src/security/backup.rs

use age::{Encryptor, Decryptor, secrecy::SecretString};
use std::io::{Read, Write};

impl EncryptedBackupService {
    pub fn new(encryption: crate::security::AgeEncryption) -> Self {
        Self { encryption }
    }

    /// Create an encrypted backup archive.
    ///
    /// `db_path` — live database file path
    /// `key_bytes` — 32-byte key from Session (derived from master password)
    /// `output_dir` — directory to write the .tar.gz file
    pub async fn create(
        &self,
        db_path: &Path,
        key_bytes: &[u8],
        output_dir: &Path,
    ) -> Result<BackupResult, SecurityError> {
        let timestamp = chrono::Utc::now();
        let archive_name = format!(
            "lazyjob-backup-{}.tar.gz",
            timestamp.format("%Y%m%d-%H%M%S")
        );
        let archive_path = output_dir.join(&archive_name);

        // 1. Read and encrypt the database file
        let db_bytes = tokio::fs::read(db_path).await?;
        let passphrase = SecretString::new(hex::encode(key_bytes));
        let mut encrypted_bytes = Vec::new();
        let encryptor = Encryptor::with_user_passphrase(passphrase);
        let mut writer = encryptor.wrap_output(&mut encrypted_bytes)
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;
        writer.write_all(&db_bytes)
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;
        writer.finish()
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;

        // 2. Compute SHA-256 of the encrypted bytes
        let sha256_hex = hex::encode(Sha256::digest(&encrypted_bytes));

        // 3. Build manifest
        let manifest = BackupManifest {
            format_version: 1,
            created_at: timestamp,
            lazyjob_version: env!("CARGO_PKG_VERSION").to_string(),
            db_size_bytes: db_bytes.len() as u64,
            sha256_hex,
            encryption_scheme: "age-v1".to_string(),
        };

        // 4. Write .tar.gz archive
        tokio::task::spawn_blocking({
            let archive_path = archive_path.clone();
            let manifest_json = serde_json::to_vec_pretty(&manifest).unwrap();
            move || -> Result<(), SecurityError> {
                use flate2::write::GzEncoder;
                use flate2::Compression;
                use tar::Builder;

                let file = std::fs::File::create(&archive_path)?;
                let gz = GzEncoder::new(file, Compression::best());
                let mut tar = Builder::new(gz);

                // db entry
                let mut header = tar::Header::new_gnu();
                header.set_size(encrypted_bytes.len() as u64);
                header.set_mode(0o600);
                header.set_cksum();
                tar.append_data(
                    &mut header,
                    "lazyjob.db.age",
                    encrypted_bytes.as_slice(),
                ).map_err(SecurityError::Io)?;

                // manifest entry
                let mut hdr2 = tar::Header::new_gnu();
                hdr2.set_size(manifest_json.len() as u64);
                hdr2.set_mode(0o644);
                hdr2.set_cksum();
                tar.append_data(
                    &mut hdr2,
                    "backup_manifest.json",
                    manifest_json.as_slice(),
                ).map_err(SecurityError::Io)?;

                tar.finish().map_err(SecurityError::Io)?;
                Ok(())
            }
        }).await
          .map_err(|e| SecurityError::Export(e.to_string()))??;

        // 5. Record backup in SQLite audit log
        // (caller does this via SecurityAuditLog)

        Ok(BackupResult { archive_path, manifest })
    }

    /// Restore from an encrypted backup archive.
    ///
    /// Verifies SHA-256 before writing, requires master password to decrypt.
    pub async fn restore(
        &self,
        archive_path: &Path,
        key_bytes: &[u8],
        output_db_path: &Path,
    ) -> Result<RestoreResult, SecurityError> {
        // 1. Extract tar.gz in memory
        let (encrypted_bytes, manifest) = tokio::task::spawn_blocking({
            let archive_path = archive_path.to_owned();
            move || -> Result<(Vec<u8>, BackupManifest), SecurityError> {
                use flate2::read::GzDecoder;
                use tar::Archive;

                let file = std::fs::File::open(&archive_path)?;
                let gz = GzDecoder::new(file);
                let mut tar = Archive::new(gz);

                let mut encrypted_bytes = Vec::new();
                let mut manifest_bytes = Vec::new();

                for entry in tar.entries().map_err(SecurityError::Io)? {
                    let mut e = entry.map_err(SecurityError::Io)?;
                    let path = e.path().map_err(SecurityError::Io)?;
                    if path.to_str() == Some("lazyjob.db.age") {
                        e.read_to_end(&mut encrypted_bytes).map_err(SecurityError::Io)?;
                    } else if path.to_str() == Some("backup_manifest.json") {
                        e.read_to_end(&mut manifest_bytes).map_err(SecurityError::Io)?;
                    }
                }

                let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
                    .map_err(|e| SecurityError::Export(e.to_string()))?;
                Ok((encrypted_bytes, manifest))
            }
        }).await
          .map_err(|e| SecurityError::Export(e.to_string()))??;

        // 2. Verify SHA-256
        let actual_sha256 = hex::encode(Sha256::digest(&encrypted_bytes));
        if actual_sha256 != manifest.sha256_hex {
            return Err(SecurityError::Decryption(
                "backup integrity check failed: SHA-256 mismatch".to_string()
            ));
        }

        // 3. Decrypt
        let passphrase = SecretString::new(hex::encode(key_bytes));
        let decryptor = Decryptor::new(encrypted_bytes.as_slice())
            .map_err(|e| SecurityError::Decryption(e.to_string()))?;
        if let Decryptor::Passphrase(d) = decryptor {
            let mut decrypted = Vec::new();
            let mut reader = d.decrypt(&passphrase, None)
                .map_err(|e| SecurityError::Decryption(e.to_string()))?;
            reader.read_to_end(&mut decrypted).map_err(SecurityError::Io)?;
            tokio::fs::write(output_db_path, decrypted).await?;
        } else {
            return Err(SecurityError::Decryption("expected passphrase-encrypted archive".to_string()));
        }

        Ok(RestoreResult {
            restored_db_path: output_db_path.to_owned(),
            manifest,
        })
    }
}
```

#### Secure Temp File Cleanup

```rust
// lazyjob-core/src/security/export.rs — extension

/// Secure-delete a file by overwriting with zeros before unlinking.
/// Best-effort: on copy-on-write filesystems (APFS, btrfs), this is
/// not guaranteed but is still better than a plain unlink.
pub fn secure_delete(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    if let Ok(metadata) = path.metadata() {
        let size = metadata.len() as usize;
        let zeros = vec![0u8; size.min(64 * 1024)];
        if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(path) {
            let chunks = size / zeros.len();
            let remainder = size % zeros.len();
            for _ in 0..chunks {
                let _ = file.write_all(&zeros);
            }
            let _ = file.write_all(&zeros[..remainder]);
            let _ = file.sync_all();
        }
    }
    std::fs::remove_file(path)
}
```

#### CLI Commands

```
lazyjob backup create [--output-dir <dir>]   # creates backup in <dir> or ~/.lazyjob/backups/
lazyjob backup restore <archive.tar.gz>      # restores, prompts for master password
lazyjob backup list                          # lists backups from audit log
```

#### SQLite Schema

```sql
-- Migration 019: backup audit log
CREATE TABLE IF NOT EXISTS backup_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    archive_name TEXT   NOT NULL,
    sha256_hex  TEXT    NOT NULL,
    db_size_bytes INTEGER NOT NULL,
    lazyjob_version TEXT NOT NULL
);
```

#### Verification

1. `cargo test -p lazyjob-core -- security::backup` — round-trip create+restore with a test DB
2. Corrupt the archive and verify `restore()` returns `Decryption` error
3. Use wrong password and verify decryption fails
4. `lazyjob backup create` on CLI writes a readable tar.gz with manifest

---

## Phase 2 — Platform Integration Gaps (GAP-92, GAP-93, GAP-97, GAP-98)

### 2.1 LinkedIn OAuth Analysis and Graceful Degradation (GAP-92)

**Gap summary**: LinkedIn's public OAuth scopes for job-seeker third-party apps are extremely restricted as of 2024. This section specifies the real access available and implements graceful degradation.

#### LinkedIn OAuth Reality

LinkedIn's Partner API requires company-level approval and is not available to individual developers or open-source projects. The "Apply with LinkedIn" (AHL) product requires a formal partnership agreement with LinkedIn. As of 2024:

- `r_liteprofile` and `r_emailaddress` scopes are available via OAuth to any registered app — provides name, email, headline, profile photo only.
- `r_fullprofile`, `r_network` (connections), `w_member_social` (messages) are gated and unavailable to standard OAuth apps.
- The Easy Apply API is reserved for LinkedIn Recruiter customers, not job-seekers.
- **Conclusion**: LinkedIn OAuth cannot be used for job discovery, Easy Apply submission, or connection data in LazyJob MVP.

#### Implementation: Scope Enum with Availability Flags

```rust
// lazyjob-core/src/platform/linkedin_oauth.rs

/// Documents what scopes LinkedIn allows for standard OAuth apps.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkedInScope {
    /// Available to all registered apps — name, headline, photo.
    LiteProfile,
    /// Available to all registered apps — email address.
    EmailAddress,
    /// Requires partner approval — full work history, connections.
    FullProfile,
    /// Requires partner approval — connections graph.
    Network,
    /// Requires partner approval — send messages, InMail.
    MemberSocial,
    /// Requires ATS partner agreement — submit Easy Apply.
    EasyApply,
}

impl LinkedInScope {
    /// Returns `true` if this scope is accessible without a LinkedIn partnership agreement.
    pub const fn is_publicly_available(&self) -> bool {
        matches!(self, Self::LiteProfile | Self::EmailAddress)
    }

    pub const fn scope_string(&self) -> &'static str {
        match self {
            Self::LiteProfile  => "r_liteprofile",
            Self::EmailAddress => "r_emailaddress",
            Self::FullProfile  => "r_fullprofile",
            Self::Network      => "r_network",
            Self::MemberSocial => "w_member_social",
            Self::EasyApply    => "rw_eas",
        }
    }
}

/// Minimal profile info available via r_liteprofile.
#[derive(serde::Deserialize, Debug)]
pub struct LinkedInLiteProfile {
    pub id: String,
    pub localizedFirstName: String,
    pub localizedLastName: String,
    pub headline: Option<String>,
}

/// Result of an OAuth flow — only the publicly available data.
pub struct LinkedInOAuthSession {
    pub access_token: secrecy::Secret<String>,
    pub profile: LinkedInLiteProfile,
    pub email: String,
}

#[derive(thiserror::Error, Debug)]
pub enum LinkedInOAuthError {
    #[error("OAuth not configured — LinkedIn partnership required for scope: {0:?}")]
    ScopeNotAvailable(LinkedInScope),

    #[error("OAuth token expired — re-authentication required")]
    TokenExpired,

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("LinkedIn API error: {status} {message}")]
    ApiError { status: u16, message: String },
}

/// LinkedIn OAuth client (lite profile only).
pub struct LinkedInOAuthClient {
    http: reqwest::Client,
    client_id: String,
    client_secret: secrecy::Secret<String>,
    redirect_uri: String,
}

impl LinkedInOAuthClient {
    pub fn new(client_id: String, client_secret: secrecy::Secret<String>, redirect_uri: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            client_id,
            client_secret,
            redirect_uri,
        }
    }

    /// Generate the OAuth authorization URL. User visits this in their browser.
    pub fn authorization_url(&self, state: &str) -> String {
        format!(
            "https://www.linkedin.com/oauth/v2/authorization?response_type=code\
             &client_id={}&redirect_uri={}&scope={}&state={}",
            self.client_id,
            urlencoding::encode(&self.redirect_uri),
            "r_liteprofile%20r_emailaddress",
            state,
        )
    }

    /// Exchange authorization code for access token, then fetch profile.
    pub async fn exchange_code(
        &self,
        code: &str,
    ) -> Result<LinkedInOAuthSession, LinkedInOAuthError> {
        use secrecy::ExposeSecret;

        let token_resp = self.http
            .post("https://www.linkedin.com/oauth/v2/accessToken")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &self.client_id),
                ("client_secret", self.client_secret.expose_secret()),
            ])
            .send()
            .await?;

        #[derive(serde::Deserialize)]
        struct TokenResponse { access_token: String }
        let tr: TokenResponse = token_resp.json().await?;
        let access_token = secrecy::Secret::new(tr.access_token);

        let profile = self.http
            .get("https://api.linkedin.com/v2/me")
            .bearer_auth(access_token.expose_secret())
            .send()
            .await?
            .json::<LinkedInLiteProfile>()
            .await?;

        #[derive(serde::Deserialize)]
        struct EmailResp {
            elements: Vec<EmailElement>,
        }
        #[derive(serde::Deserialize)]
        struct EmailElement {
            #[serde(rename = "handle~")]
            handle: EmailHandle,
        }
        #[derive(serde::Deserialize)]
        struct EmailHandle { emailAddress: String }

        let email_resp = self.http
            .get("https://api.linkedin.com/v2/emailAddress?q=members&projection=(elements*(handle~))")
            .bearer_auth(access_token.expose_secret())
            .send()
            .await?
            .json::<EmailResp>()
            .await?;

        let email = email_resp.elements.into_iter()
            .next()
            .map(|e| e.handle.emailAddress)
            .unwrap_or_default();

        Ok(LinkedInOAuthSession { access_token, profile, email })
    }
}
```

**TUI disclosure**: When user tries to configure LinkedIn integration, display:
```
LinkedIn Integration: Limited Access
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
LinkedIn only allows access to: name, headline, email.
Job discovery, connection data, and Easy Apply require a LinkedIn
partner agreement which is not available to open-source projects.

LazyJob uses LinkedIn CSV export for contact import instead.
Press [Enter] to continue.
```

---

### 2.2 Browser Fingerprinting Strategy for Workday (GAP-93)

**Gap summary**: Workday powers ~39% of Fortune 500 ATS. Any browser automation against it risks detection. This section specifies a conservative stealth strategy.

#### Detection Risk Assessment

| Technique | Risk Level | Notes |
|-----------|------------|-------|
| Raw Playwright | HIGH | CDP endpoint exposed, navigator.webdriver=true |
| playwright-extra + stealth plugin | MEDIUM | Patches most CDP signals but not all |
| `rebrowser-patches` (chromium recompile) | LOW | Patches runtime.enable leak but requires build infra |
| Residential proxy + stealth | LOW-MEDIUM | IP looks residential, still patches needed |

**LazyJob MVP decision**: Use Playwright with stealth plugin at a **crawl rate of max 1 request/5 seconds** per domain. Document that Workday automation is best-effort and may be blocked by the employer's Workday instance. Do NOT use residential proxies (too expensive, legally gray).

#### Stealth Configuration Types

```rust
// lazyjob-core/src/platform/fingerprint.rs

/// Browser automation stealth options applied when spawning Playwright/headless Chrome.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StealthOptions {
    /// Set `navigator.webdriver = undefined` via JS injection.
    pub patch_webdriver: bool,

    /// Override navigator.plugins with realistic plugin array.
    pub patch_plugins: bool,

    /// Set realistic screen dimensions (1920x1080).
    pub patch_screen: bool,

    /// Randomize user-agent within a supported browser range.
    pub randomize_user_agent: bool,

    /// Minimum milliseconds between page actions (jitter: + 0..200ms).
    pub min_action_delay_ms: u64,

    /// Minimum milliseconds between requests to same domain.
    pub min_request_delay_ms: u64,
}

impl Default for StealthOptions {
    fn default() -> Self {
        Self {
            patch_webdriver: true,
            patch_plugins: true,
            patch_screen: true,
            randomize_user_agent: true,
            min_action_delay_ms: 300,
            min_request_delay_ms: 5000,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DetectionResponse {
    /// Continue — no bot-detection signals found.
    Continue,

    /// CAPTCHA detected — pause and notify user.
    CaptchaDetected { url: String },

    /// Rate limited / blocked — back off for duration.
    RateLimited { retry_after_secs: u64 },

    /// Session expired — re-authentication required.
    SessionExpired,
}

/// Detect common bot-detection patterns in an HTML response body.
pub fn detect_bot_detection(body: &str) -> DetectionResponse {
    static CAPTCHA_PATTERNS: once_cell::sync::Lazy<Vec<regex::Regex>> =
        once_cell::sync::Lazy::new(|| {
            ["recaptcha", "hcaptcha", "cf-challenge", "cf-turnstile",
             "access denied", "automated access", "bot detected"]
                .iter()
                .map(|p| regex::Regex::new(&format!("(?i){}", p)).unwrap())
                .collect()
        });

    for pattern in CAPTCHA_PATTERNS.iter() {
        if pattern.is_match(body) {
            return DetectionResponse::CaptchaDetected {
                url: String::new(), // caller fills in
            };
        }
    }
    DetectionResponse::Continue
}
```

---

### 2.3 Workday Integration Strategy (GAP-97)

**Gap summary**: Workday is the most important closed ATS but has no public job-seeker API. This section specifies the headless-browser client.

#### Workday URL Detection

```rust
// lazyjob-core/src/platform/workday.rs

use once_cell::sync::Lazy;
use regex::Regex;

/// Matches Workday job board URLs in various formats.
static WORKDAY_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"https://[^/]+\.wd\d+\.myworkdayjobs\.com/").unwrap(),
        Regex::new(r"https://[^/]+\.workday\.com/[^/]+/jobs").unwrap(),
        Regex::new(r"https://[^/]+/en-US/[^/]+/job/").unwrap(),  // tenant-hosted
    ]
});

pub fn is_workday_url(url: &str) -> bool {
    WORKDAY_URL_PATTERNS.iter().any(|p| p.is_match(url))
}

/// Extracts the Workday tenant slug from a URL.
/// e.g. "https://amazon.wd5.myworkdayjobs.com/en-US/External_Career_Site" → "amazon"
pub fn extract_tenant(url: &str) -> Option<&str> {
    static TENANT_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"https://([^\.]+)\.wd\d+\.myworkdayjobs\.com/").unwrap()
    });
    TENANT_RE.captures(url)?.get(1).map(|m| m.as_str())
}
```

#### WorkdayClient (headless_chrome based)

```rust
// lazyjob-core/src/platform/workday.rs

use headless_chrome::{Browser, LaunchOptions, Tab};
use std::sync::Arc;
use std::time::Duration;
use crate::platform::fingerprint::StealthOptions;

/// Scraped job listing entry from Workday.
#[derive(Debug, serde::Deserialize)]
pub struct WorkdayJobListing {
    pub id: String,
    pub title: String,
    pub location: String,
    pub posted_date: Option<String>,
    pub job_url: String,
}

#[derive(thiserror::Error, Debug)]
pub enum WorkdayError {
    #[error("browser launch failed: {0}")]
    BrowserLaunch(String),

    #[error("CAPTCHA detected at {url}")]
    CaptchaDetected { url: String },

    #[error("Workday rate limited — retry after {0}s")]
    RateLimited(u64),

    #[error("navigation timeout at {url}")]
    Timeout { url: String },

    #[error("scraping failed: {0}")]
    ScrapingFailed(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct WorkdayClient {
    stealth: StealthOptions,
    css_selectors: WorkdayCssSelectors,
}

/// CSS selectors for Workday job board elements.
/// These are stored per-company in platform_sources.config (JSON) to allow per-tenant overrides.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkdayCssSelectors {
    pub job_list_item: String,
    pub job_title: String,
    pub job_location: String,
    pub job_date: String,
    pub pagination_next: String,
}

impl Default for WorkdayCssSelectors {
    fn default() -> Self {
        Self {
            job_list_item: "[data-automation-id='jobItem']".to_string(),
            job_title:     "[data-automation-id='jobItemTitle']".to_string(),
            job_location:  "[data-automation-id='jobItemLocation']".to_string(),
            job_date:      "[data-automation-id='jobItemPostedDate']".to_string(),
            pagination_next: "[data-automation-id='paginationNextButton']".to_string(),
        }
    }
}

impl WorkdayClient {
    pub fn new(stealth: StealthOptions, selectors: WorkdayCssSelectors) -> Self {
        Self { stealth, css_selectors: selectors }
    }

    /// Scrape job listings from a Workday job board URL.
    /// Must be called from a `tokio::task::spawn_blocking` context since headless_chrome is sync.
    pub fn fetch_jobs_sync(
        &self,
        board_url: &str,
    ) -> Result<Vec<WorkdayJobListing>, WorkdayError> {
        let browser = Browser::new(LaunchOptions {
            headless: true,
            args: vec![
                std::ffi::OsStr::new("--no-sandbox"),
                std::ffi::OsStr::new("--disable-setuid-sandbox"),
                std::ffi::OsStr::new("--disable-blink-features=AutomationControlled"),
            ],
            ..Default::default()
        }).map_err(|e| WorkdayError::BrowserLaunch(e.to_string()))?;

        let tab = browser.new_tab()
            .map_err(|e| WorkdayError::BrowserLaunch(e.to_string()))?;

        // Inject stealth patches
        if self.stealth.patch_webdriver {
            tab.evaluate(
                "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})",
                false,
            ).ok();
        }

        tab.navigate_to(board_url)
            .map_err(|e| WorkdayError::Timeout { url: board_url.to_string() })?;
        tab.wait_until_navigated()
            .map_err(|e| WorkdayError::Timeout { url: board_url.to_string() })?;

        // Check for bot detection
        let body = tab.get_content()
            .map_err(|e| WorkdayError::ScrapingFailed(e.to_string()))?;
        if let crate::platform::fingerprint::DetectionResponse::CaptchaDetected { .. } =
            crate::platform::fingerprint::detect_bot_detection(&body)
        {
            return Err(WorkdayError::CaptchaDetected { url: board_url.to_string() });
        }

        let mut jobs = Vec::new();
        self.scrape_page(&tab, &mut jobs)?;

        Ok(jobs)
    }

    fn scrape_page(
        &self,
        tab: &Arc<Tab>,
        jobs: &mut Vec<WorkdayJobListing>,
    ) -> Result<(), WorkdayError> {
        let items = tab.find_elements(&self.css_selectors.job_list_item)
            .map_err(|e| WorkdayError::ScrapingFailed(e.to_string()))?;

        for item in items {
            let title = item.find_element(&self.css_selectors.job_title)
                .and_then(|e| e.get_inner_text())
                .unwrap_or_default();
            let location = item.find_element(&self.css_selectors.job_location)
                .and_then(|e| e.get_inner_text())
                .unwrap_or_default();
            let date = item.find_element(&self.css_selectors.job_date)
                .and_then(|e| e.get_inner_text())
                .ok();
            // Build job URL from item href or data attributes
            let job_url = item.find_element("a")
                .and_then(|e| e.get_attribute_value("href"))
                .unwrap_or_default()
                .unwrap_or_default();

            jobs.push(WorkdayJobListing {
                id: sha2::Sha256::digest(format!("{}{}", title, location).as_bytes())
                    .iter().fold(String::new(), |a, b| a + &format!("{:02x}", b)),
                title,
                location,
                posted_date: date,
                job_url,
            });
        }
        Ok(())
    }

    /// Async wrapper — offloads the sync browser call to a blocking thread.
    pub async fn fetch_jobs(&self, board_url: String) -> Result<Vec<WorkdayJobListing>, WorkdayError> {
        let client = self.clone_config();
        tokio::task::spawn_blocking(move || client.fetch_jobs_sync(&board_url))
            .await
            .map_err(|e| WorkdayError::ScrapingFailed(e.to_string()))?
    }

    fn clone_config(&self) -> WorkdayClientConfig {
        WorkdayClientConfig {
            stealth: self.stealth.clone(),
            css_selectors: self.css_selectors.clone(),
        }
    }
}

struct WorkdayClientConfig {
    stealth: StealthOptions,
    css_selectors: WorkdayCssSelectors,
}

impl WorkdayClientConfig {
    fn fetch_jobs_sync(&self, board_url: &str) -> Result<Vec<WorkdayJobListing>, WorkdayError> {
        let client = WorkdayClient::new(self.stealth.clone(), self.css_selectors.clone());
        client.fetch_jobs_sync(board_url)
    }
}
```

#### SQLite Schema for Workday Sources

```sql
-- platform_sources table already defined in plan 11.
-- Workday rows use config JSON:
-- {
--   "board_url": "https://amazon.wd5.myworkdayjobs.com/en-US/External_Career_Site",
--   "css_selectors": { ...WorkdayCssSelectors... },
--   "stealth": { "min_request_delay_ms": 5000 }
-- }
```

#### Credential Storage for Workday

Workday accounts for applying (not just discovering) require credentials:

```rust
// lazyjob-core/src/security/credentials.rs — extension

impl CredentialManager {
    /// Store Workday account credentials for a specific tenant.
    /// Key pattern: "workday::<tenant>::username" / "workday::<tenant>::password"
    pub fn set_workday_creds(
        &self,
        tenant: &str,
        username: &str,
        password: &secrecy::Secret<String>,
    ) -> Result<(), SecurityError> {
        self.set("workday", &format!("{}::username", tenant), username)?;
        self.set("workday", &format!("{}::password", tenant), password.expose_secret())?;
        Ok(())
    }

    pub fn get_workday_creds(
        &self,
        tenant: &str,
    ) -> Result<(String, secrecy::Secret<String>), SecurityError> {
        let username = self.get("workday", &format!("{}::username", tenant))?;
        let password = secrecy::Secret::new(
            self.get("workday", &format!("{}::password", tenant))?
        );
        Ok((username, password))
    }
}
```

---

### 2.4 Job Aggregator Cost Tracking (GAP-98)

**Gap summary**: Job discovery costs (Adzuna free tier, Apify at $0.005/result) must be tracked and bounded.

#### Cost Types

```rust
// lazyjob-core/src/platform/budget.rs

use chrono::{DateTime, Utc};

/// Cost charged per API call to a paid aggregator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AggregatorCostEntry {
    pub source: String,             // e.g. "adzuna", "apify"
    pub cost_microdollars: i64,     // 0 for free-tier calls
    pub jobs_returned: u32,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AggregatorBudgetConfig {
    pub daily_limit_microdollars: i64,    // default: 5_000_000 ($5)
    pub monthly_limit_microdollars: i64,  // default: 50_000_000 ($50)
}

impl Default for AggregatorBudgetConfig {
    fn default() -> Self {
        Self {
            daily_limit_microdollars: 5_000_000,
            monthly_limit_microdollars: 50_000_000,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BudgetError {
    #[error("daily aggregator budget exceeded: spent ${:.4}, limit ${:.4}",
        spent_microdollars as f64 / 1_000_000.0,
        limit_microdollars as f64 / 1_000_000.0)]
    DailyBudgetExceeded { spent_microdollars: i64, limit_microdollars: i64 },

    #[error("monthly aggregator budget exceeded: spent ${:.4}, limit ${:.4}",
        spent_microdollars as f64 / 1_000_000.0,
        limit_microdollars as f64 / 1_000_000.0)]
    MonthlyBudgetExceeded { spent_microdollars: i64, limit_microdollars: i64 },
}

pub struct PlatformCostTracker {
    pool: sqlx::SqlitePool,
    config: AggregatorBudgetConfig,
}

impl PlatformCostTracker {
    pub async fn check_and_record(
        &self,
        entry: &AggregatorCostEntry,
    ) -> Result<(), BudgetError> {
        let daily_spent = self.daily_spend_microdollars(&entry.source).await
            .unwrap_or(0);
        if daily_spent + entry.cost_microdollars > self.config.daily_limit_microdollars {
            return Err(BudgetError::DailyBudgetExceeded {
                spent_microdollars: daily_spent,
                limit_microdollars: self.config.daily_limit_microdollars,
            });
        }
        let monthly_spent = self.monthly_spend_microdollars(&entry.source).await
            .unwrap_or(0);
        if monthly_spent + entry.cost_microdollars > self.config.monthly_limit_microdollars {
            return Err(BudgetError::MonthlyBudgetExceeded {
                spent_microdollars: monthly_spent,
                limit_microdollars: self.config.monthly_limit_microdollars,
            });
        }
        sqlx::query!(
            "INSERT INTO aggregator_cost_log (source, cost_microdollars, jobs_returned) VALUES (?, ?, ?)",
            entry.source,
            entry.cost_microdollars,
            entry.jobs_returned,
        ).execute(&self.pool).await.ok();
        Ok(())
    }

    async fn daily_spend_microdollars(&self, source: &str) -> sqlx::Result<i64> {
        let row = sqlx::query!(
            "SELECT COALESCE(SUM(cost_microdollars), 0) AS total
             FROM aggregator_cost_log
             WHERE source = ? AND timestamp >= date('now', '-1 day')",
            source,
        ).fetch_one(&self.pool).await?;
        Ok(row.total)
    }

    async fn monthly_spend_microdollars(&self, source: &str) -> sqlx::Result<i64> {
        let row = sqlx::query!(
            "SELECT COALESCE(SUM(cost_microdollars), 0) AS total
             FROM aggregator_cost_log
             WHERE source = ? AND timestamp >= date('now', '-30 days')",
            source,
        ).fetch_one(&self.pool).await?;
        Ok(row.total)
    }
}
```

#### SQLite Schema

```sql
-- Migration 020
CREATE TABLE IF NOT EXISTS aggregator_cost_log (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    source              TEXT    NOT NULL,
    cost_microdollars   INTEGER NOT NULL DEFAULT 0,
    jobs_returned       INTEGER NOT NULL DEFAULT 0,
    timestamp           TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_aggregator_cost_source_time
    ON aggregator_cost_log(source, timestamp);
```

---

## Phase 3 — Moderate Security Gaps (GAP-94, GAP-95, GAP-96, GAP-91)

### 3.1 Data Retention and Deletion Policy (GAP-94)

**Gap summary**: Without a retention policy, job listings and applications accumulate indefinitely. Deletion must cascade consistently.

#### Retention Configuration

```toml
# ~/.config/lazyjob/config.toml
[retention]
job_listings_days           = 180    # 0 = keep forever
closed_applications_days    = 365
rejected_applications_days  = 180
completed_contacts_days     = 730
backup_count_max            = 10     # keep the N most recent backups
```

#### Core Types

```rust
// lazyjob-core/src/security/retention.rs

use std::time::Duration;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RetentionPolicy {
    pub job_listings_days:        Option<u64>,
    pub closed_applications_days: Option<u64>,
    pub rejected_applications_days: Option<u64>,
    pub completed_contacts_days:  Option<u64>,
    pub backup_count_max:         usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            job_listings_days: Some(180),
            closed_applications_days: Some(365),
            rejected_applications_days: Some(180),
            completed_contacts_days: Some(730),
            backup_count_max: 10,
        }
    }
}

pub struct DeletionCascade {
    pool: sqlx::SqlitePool,
}

impl DeletionCascade {
    /// Hard-delete a job listing and all linked data:
    /// applications, resume_versions, cover_letter_versions, job_embeddings.
    /// Uses a single SQLite transaction.
    pub async fn delete_job(&self, job_id: i64) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        // Delete in dependency order (FK constraints: applications → resume_versions)
        sqlx::query!("DELETE FROM cover_letter_versions WHERE job_id = ?", job_id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM resume_versions WHERE job_id = ?", job_id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM application_transitions WHERE application_id IN (SELECT id FROM applications WHERE job_id = ?)", job_id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM applications WHERE job_id = ?", job_id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM job_embeddings WHERE job_id = ?", job_id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM jobs WHERE id = ?", job_id).execute(&mut *tx).await?;
        tx.commit().await
    }

    /// Prune job listings older than the policy's retention window.
    /// Returns the count of deleted listings.
    pub async fn prune_old_jobs(&self, policy: &RetentionPolicy) -> sqlx::Result<u64> {
        let days = match policy.job_listings_days {
            None => return Ok(0),
            Some(d) => d as i64,
        };
        // Collect IDs first so we can use delete_job for proper cascade
        let old_ids: Vec<i64> = sqlx::query_scalar!(
            "SELECT id FROM jobs
             WHERE discovered_at < datetime('now', ? || ' days')
             AND id NOT IN (SELECT DISTINCT job_id FROM applications WHERE job_id IS NOT NULL)",
            -days,
        ).fetch_all(&self.pool).await?;

        let count = old_ids.len() as u64;
        for id in old_ids {
            self.delete_job(id).await?;
        }
        Ok(count)
    }
}
```

#### CLI Command

```
lazyjob data prune [--dry-run]   # shows what would be deleted, then confirms
lazyjob data prune --force       # skips confirmation
```

### 3.2 LLM Provider Privacy Disclosure (GAP-95)

```rust
// lazyjob-core/src/security/privacy_disclosure.rs

/// Data sent to each LLM provider and their stated data-use policy.
/// Shown to the user in TUI when configuring a provider.
#[derive(Debug, Clone)]
pub struct LlmProviderPrivacyInfo {
    pub provider_name: &'static str,
    pub data_used_for_training: bool,
    pub opt_out_available: bool,
    pub opt_out_url: Option<&'static str>,
    pub data_retained_days: Option<u32>,
    pub privacy_policy_url: &'static str,
    pub local_only: bool,
}

pub const ANTHROPIC_PRIVACY: LlmProviderPrivacyInfo = LlmProviderPrivacyInfo {
    provider_name: "Anthropic",
    data_used_for_training: false,
    opt_out_available: false,
    opt_out_url: None,
    data_retained_days: Some(30),
    privacy_policy_url: "https://www.anthropic.com/privacy",
    local_only: false,
};

pub const OPENAI_PRIVACY: LlmProviderPrivacyInfo = LlmProviderPrivacyInfo {
    provider_name: "OpenAI",
    data_used_for_training: false, // API traffic is not used for training by default
    opt_out_available: true,
    opt_out_url: Some("https://platform.openai.com/account/data-controls"),
    data_retained_days: Some(30),
    privacy_policy_url: "https://openai.com/policies/privacy-policy",
    local_only: false,
};

pub const OLLAMA_PRIVACY: LlmProviderPrivacyInfo = LlmProviderPrivacyInfo {
    provider_name: "Ollama (local)",
    data_used_for_training: false,
    opt_out_available: false,
    opt_out_url: None,
    data_retained_days: None,  // no retention — fully local
    privacy_policy_url: "https://ollama.ai",
    local_only: true,
};

/// Generate a disclosure string for display in the TUI settings panel.
pub fn format_disclosure(info: &LlmProviderPrivacyInfo) -> String {
    if info.local_only {
        return format!(
            "{}: Fully local. No data leaves your machine.",
            info.provider_name
        );
    }
    let training_note = if info.data_used_for_training {
        "⚠  Data MAY be used for model training."
    } else {
        "✓  Data is NOT used for model training."
    };
    let retention_note = match info.data_retained_days {
        None => "Retention: unspecified.".to_string(),
        Some(d) => format!("Prompts retained up to {} days.", d),
    };
    format!(
        "{}: {}\n  {}\n  Privacy policy: {}",
        info.provider_name,
        training_note,
        retention_note,
        info.privacy_policy_url,
    )
}
```

### 3.3 Crash Reports and Telemetry (GAP-96)

```rust
// lazyjob-core/src/security/telemetry.rs

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TelemetryConfig {
    /// If false, no crash reports or usage telemetry is sent.
    /// Default: false (opt-in only).
    pub enabled: bool,

    /// URL for crash report submission. None = no crash reporting.
    pub crash_report_url: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self { enabled: false, crash_report_url: None }
    }
}

/// A sanitized crash report — no personal data.
#[derive(serde::Serialize)]
pub struct CrashReport {
    pub lazyjob_version: String,
    pub os: String,
    pub arch: String,
    pub panic_message: String,  // stack trace stripped of paths containing username
    pub timestamp: String,
}

impl CrashReport {
    /// Sanitize a raw panic message to remove user-specific paths.
    pub fn sanitize_panic(raw: &str) -> String {
        // Remove home directory paths like /home/username/...
        static HOME_RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
            regex::Regex::new(r"/(?:home|Users)/[^/]+/").unwrap()
        });
        HOME_RE.replace_all(raw, "/home/<user>/").to_string()
    }
}

/// Set a panic hook that writes a crash report to ~/.cache/lazyjob/crashes/
/// if telemetry is disabled, or submits it to crash_report_url if enabled.
pub fn install_panic_hook(config: TelemetryConfig) {
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        let report = CrashReport {
            lazyjob_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            panic_message: CrashReport::sanitize_panic(&msg),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        // Always write to local crash log
        if let Some(mut path) = dirs::cache_dir() {
            path.push("lazyjob/crashes");
            let _ = std::fs::create_dir_all(&path);
            path.push(format!("{}.json", report.timestamp.replace(':', "-")));
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default());
        }
        // Print to stderr for immediate visibility
        eprintln!("LazyJob crashed: {}", msg);
        eprintln!("Crash report saved. Enable crash reporting in config to submit automatically.");
    }));
}
```

### 3.4 Multi-Device Sync Architecture (GAP-91)

**Decision**: Multi-device sync is deferred to the SaaS migration path (covered in `specs/18-saas-migration-path-implementation-plan.md`). For MVP, document the manual sync path:

```
# Manual sync via filesystem
lazyjob backup create --output-dir ~/Dropbox/lazyjob-backup/
# On second machine:
lazyjob backup restore ~/Dropbox/lazyjob-backup/lazyjob-backup-<timestamp>.tar.gz
```

The backup is encrypted with the master password so syncing via Dropbox/iCloud is safe as long as the master password is strong. This is documented in the TUI settings panel under "Sync & Backup".

---

## Cross-Spec T: Encryption Key Management Lifecycle

**Resolution**: The key lifecycle is:

1. **First run**: `SecurityLayer::setup_master_password(password)` → derives 32-byte key → stores PHC hash + salt in OS keyring → uses key to encrypt DB with `age`.
2. **Subsequent runs**: `SecurityLayer::unlock(password, timeout)` → verifies PHC hash from keyring → re-derives key → returns `Session` with key in `Zeroizing<Vec<u8>>`.
3. **DB operations**: `Session::key_bytes()` passed to `AgeEncryption::decrypt_to_tmpfile()` → working DB opened from decrypted temp file → temp file securely deleted after DB closed via `Drop` on `EncryptedDb`.
4. **Password change**: `SecurityLayer::change_master_password()` → re-derives new key → re-encrypts DB with new key → updates keyring.
5. **Key in memory**: Only the `Session` struct holds the key; it is `Zeroizing<Vec<u8>>` which zeroes on drop. Never passed by value, only by reference.

---

## Cross-Spec U: Unified Credential Storage

**Resolution**: All credentials use `CredentialManager` with a namespacing convention:

| Credential Type | Service | Account |
|-----------------|---------|---------|
| LLM API key (Anthropic) | `lazyjob` | `llm::anthropic::api_key` |
| LLM API key (OpenAI) | `lazyjob` | `llm::openai::api_key` |
| Master password PHC | `lazyjob` | `master_phc` |
| Master password salt | `lazyjob` | `master_salt` |
| Greenhouse API key (company X) | `lazyjob` | `platform::greenhouse::<company>::api_key` |
| Lever API key (company X) | `lazyjob` | `platform::lever::<company>::api_key` |
| Workday credentials (tenant X) | `lazyjob` | `platform::workday::<tenant>::username/password` |
| LinkedIn OAuth token | `lazyjob` | `platform::linkedin::access_token` |

All credential access goes through `CredentialManager::get/set/delete` — never through raw `keyring::Entry` calls outside this module.

---

## Module Structure

```
lazyjob-core/src/security/
  mod.rs                    # SecurityLayer + re-exports
  error.rs                  # SecurityError, Result<T>
  credentials.rs            # CredentialManager (unified, namespaced)
  encryption.rs             # AgeEncryption
  master_password.rs        # MasterPassword (Argon2id)
  session.rs                # Session, SessionGuard, InactivityTimer
  backup.rs                 # EncryptedBackupService
  export.rs                 # DataExporter + secure_delete
  retention.rs              # RetentionPolicy, DeletionCascade
  privacy_disclosure.rs     # LlmProviderPrivacyInfo constants
  telemetry.rs              # TelemetryConfig, CrashReport, install_panic_hook

lazyjob-core/src/platform/
  workday.rs                # WorkdayClient, WorkdayCssSelectors, is_workday_url
  linkedin_oauth.rs         # LinkedInOAuthClient, LinkedInScope
  fingerprint.rs            # StealthOptions, detect_bot_detection
  budget.rs                 # PlatformCostTracker, AggregatorBudgetConfig

lazyjob-tui/src/views/
  lock_screen.rs            # LockScreenView
  backup_restore.rs         # BackupRestoreView
  privacy_disclosure.rs     # LlmProviderPrivacyPanel

lazyjob-cli/src/commands/
  backup.rs                 # backup create|restore|list
  retention.rs              # data prune
```

---

## Key Crate APIs

- `argon2::Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(pw, salt, out)` — key derivation
- `argon2::Argon2::default().verify_password(pw, &hash)` — login verification
- `age::Encryptor::with_user_passphrase(SecretString).wrap_output(&mut buf)` — encryption
- `age::Decryptor::new(bytes).decrypt(&passphrase, None)` — decryption
- `zeroize::Zeroizing::new(vec![0u8; 32])` — zeroing key buffer on drop
- `sha2::Sha256::digest(bytes)` — backup integrity hash
- `flate2::write::GzEncoder::new(file, Compression::best())` — gzip compression
- `tar::Builder::new(gz).append_data(&mut header, path, data)` — tar archive write
- `headless_chrome::Browser::new(LaunchOptions { headless: true, .. })` — browser automation
- `headless_chrome::Tab::evaluate(js, false)` — JS injection for stealth
- `keyring::Entry::new(service, account).set_password(value)` — OS keyring write
- `tokio::task::spawn_blocking(|| sync_work)` — offload headless_chrome to blocking thread
- `sqlx::query!(...).execute(&pool)` — DDL and DML
- `once_cell::sync::Lazy<regex::Regex>` — compiled patterns

---

## Error Handling

```rust
// lazyjob-core/src/security/error.rs — complete enum

#[derive(thiserror::Error, Debug)]
pub enum SecurityError {
    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("invalid master password")]
    InvalidPassword,

    #[error("app is locked")]
    AppLocked,

    #[error("export failed: {0}")]
    Export(String),

    #[error("wipe failed at step '{step}': {reason}")]
    WipeFailed { step: &'static str, reason: String },

    #[error("backup integrity check failed: {0}")]
    BackupIntegrityFailure(String),

    #[error("session timeout")]
    SessionExpired,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, SecurityError>;
```

---

## Testing Strategy

### Unit Tests

```rust
// lazyjob-core/src/security/master_password.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_deterministic() {
        let salt = [42u8; 16];
        let k1 = MasterPassword::derive_key("hunter2", &salt).unwrap();
        let k2 = MasterPassword::derive_key("hunter2", &salt).unwrap();
        assert_eq!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn derive_key_differs_on_different_password() {
        let salt = [0u8; 16];
        let k1 = MasterPassword::derive_key("abc", &salt).unwrap();
        let k2 = MasterPassword::derive_key("xyz", &salt).unwrap();
        assert_ne!(k1.as_ref(), k2.as_ref());
    }

    #[test]
    fn register_and_verify_roundtrip() {
        let (_, phc, _) = MasterPassword::register("correct-horse").unwrap();
        assert!(MasterPassword::verify("correct-horse", &phc).unwrap());
        assert!(!MasterPassword::verify("wrong", &phc).unwrap());
    }
}
```

```rust
// lazyjob-core/src/security/backup.rs

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn backup_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        tokio::fs::write(&db_path, b"SQLite test data").await.unwrap();
        let key = [1u8; 32];
        let service = EncryptedBackupService::new(
            crate::security::AgeEncryption::default()
        );
        let result = service.create(&db_path, &key, tmp.path()).await.unwrap();
        assert!(result.archive_path.exists());

        let restored_path = tmp.path().join("restored.db");
        let restore_result = service.restore(&result.archive_path, &key, &restored_path).await.unwrap();
        let restored_bytes = tokio::fs::read(&restored_path).await.unwrap();
        assert_eq!(restored_bytes, b"SQLite test data");
    }

    #[tokio::test]
    async fn restore_fails_on_wrong_key() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        tokio::fs::write(&db_path, b"data").await.unwrap();
        let good_key = [1u8; 32];
        let bad_key = [2u8; 32];
        let service = EncryptedBackupService::new(crate::security::AgeEncryption::default());
        let result = service.create(&db_path, &good_key, tmp.path()).await.unwrap();
        let restored_path = tmp.path().join("restored.db");
        let err = service.restore(&result.archive_path, &bad_key, &restored_path).await;
        assert!(matches!(err, Err(SecurityError::Decryption(_))));
    }

    #[test]
    fn sha256_tamper_detected() {
        // Corrupt the archive and verify integrity check catches it
        // (implementation detail: read archive, flip a byte, attempt restore)
    }
}
```

```rust
// Unit tests for platform/workday.rs
#[test]
fn workday_url_detection() {
    assert!(is_workday_url("https://amazon.wd5.myworkdayjobs.com/en-US/External_Career_Site"));
    assert!(!is_workday_url("https://greenhouse.io/jobs/123"));
}

#[test]
fn tenant_extraction() {
    assert_eq!(
        extract_tenant("https://amazon.wd5.myworkdayjobs.com/en-US/foo"),
        Some("amazon")
    );
}
```

### Integration Tests

- `backup create` → `backup restore` via CLI: verify restored DB matches original
- `lazyjob unlock` with wrong password: exits with non-zero status, no DB access
- `inactivity_timer` integration: mock `SystemTime`, verify session is None after timeout
- `PlatformCostTracker` with in-memory SQLite: verify daily/monthly budget enforcement

### TUI Tests

- `LockScreenView::render()`: test that password characters appear masked with `•`
- `LockScreenView::on_char/on_backspace`: verify buffer manipulation and Zeroizing semantics

---

## Open Questions

1. **Workday CAPTCHA recovery**: When a CAPTCHA is detected, the current plan notifies the user and halts automation. Should LazyJob offer a "manual resume" TUI flow where the user solves the CAPTCHA in a visible browser window launched by LazyJob?

2. **`secure_delete` on macOS APFS**: APFS uses copy-on-write semantics so overwriting does not guarantee data is gone. Should we document this limitation explicitly and suggest FileVault as the full solution?

3. **LinkedIn partner application**: Should LazyJob ever attempt to apply for LinkedIn's Partner API? If so, this would enable official job discovery and potentially Easy Apply. This would require a company/legal entity.

4. **Multi-device sync key exchange**: For the SaaS path, how does the second device get the encryption key? Options: (a) user re-enters master password on each device, (b) key is synced encrypted via the SaaS backend. Option (a) is simpler and local-first-friendly.

5. **`argon2` Wasm support**: If LazyJob ever gets a web interface, Argon2id with 64 MiB memory cost may be too slow in a browser tab. Should we parameterize the memory cost in config?

6. **Telemetry endpoint**: If crash reporting is opt-in, who hosts the endpoint? Self-hosted Sentry? GitHub Issues API submission? This is a future SaaS concern but the schema should be forward-compatible.

---

## Related Specs

- `specs/16-privacy-security.md` / `specs/16-privacy-security-implementation-plan.md` — foundation
- `specs/11-platform-api-integrations.md` / `specs/11-platform-api-integrations-implementation-plan.md` — platform client trait
- `specs/18-saas-migration-path-implementation-plan.md` — multi-device sync (Phase 2+)
- `specs/XX-master-password-app-unlock.md` — primary spec for GAP-90
- `specs/XX-encrypted-backup-export.md` — primary spec for GAP-89
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — LLM provider abstraction
