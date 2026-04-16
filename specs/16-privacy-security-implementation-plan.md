# Implementation Plan: Privacy & Security

## Status
Draft

## Related Spec
`specs/16-privacy-security.md`

## Overview

LazyJob stores highly sensitive personal and professional data: career history, salary expectations, API keys for LLM providers, and detailed job search activity. This plan establishes the security layer that protects that data across all threat surfaces — at rest (SQLite), in memory (via `zeroize` + `secrecy`), and during export/wipe flows.

The security architecture follows a defense-in-depth model:

1. **Secrets in OS keychain** — LLM API keys, OAuth tokens, and other credentials are stored exclusively via the system keyring (`keyring` crate). They are loaded at runtime, wrapped in `secrecy::Secret<String>`, and never written to SQLite.
2. **Optional at-rest encryption** — The SQLite database can be encrypted using the `age` crate for file-level envelope encryption. The encryption key is derived from a user-supplied master password via Argon2id and stored (as a sealed key blob) in the OS keychain.
3. **Secure memory handling** — All sensitive values implement `zeroize::Zeroize` or are wrapped in types that zeroize on drop. Sensitive data is not cloned unnecessarily. Password buffers use `zeroize::Zeroizing<Vec<u8>>`.
4. **Privacy modes** — Users can select a privacy mode that governs what data is persisted and what reaches LLM providers.
5. **Data export and wipe** — The user can export a full plaintext JSON dump of all their data, or wipe all data (SQLite + keychain entries + config).

The plan does **not** implement SQLCipher (requires compilation of C extensions with OpenSSL and a license or patch — high complexity for MVP). Instead we use `age` file-level encryption (pure Rust, modern, simple). SQLCipher can be added in Phase 3 as a power-user option for those who want transparent page-level encryption with database tooling compatibility.

## Prerequisites

### Specs That Must Be Implemented First
- `specs/04-sqlite-persistence.md` — `Database` struct and `SqlitePool` must exist
- `specs/01-architecture.md` — overall crate structure must be agreed upon
- `specs/03-life-sheet-data-model.md` — life sheet types needed for export schema

### Crates to Add to `lazyjob-core/Cargo.toml`
```toml
[dependencies]
keyring     = "3"
age         = { version = "0.10", features = ["armor"] }
argon2      = "0.5"
zeroize     = { version = "1", features = ["derive"] }
secrecy     = "0.8"
rand        = { version = "0.8", features = ["getrandom"] }   # for nonce/salt generation
base64      = "0.22"
```

## Architecture

### Crate Placement

All security logic lives in **`lazyjob-core`** under `src/security/`. This crate is the single source of truth for the `Database` and is the right place for encryption wrappers, credential management, and privacy-mode enforcement. `lazyjob-tui` calls into `lazyjob-core` security APIs to show the lock screen, confirm wipe, and display privacy-mode status. `lazyjob-cli` bootstraps the security layer before handing off to the TUI.

```
lazyjob-core/src/security/
├── mod.rs              # Re-exports; SecurityLayer orchestrator
├── error.rs            # SecurityError, Result alias
├── credentials.rs      # CredentialManager (keyring wrapper)
├── encryption.rs       # AgeEncryption: encrypt/decrypt database file
├── master_password.rs  # MasterPassword: Argon2id key derivation, session token
├── privacy.rs          # PrivacyMode, PrivacySettings
├── export.rs           # DataExport: full JSON export + wipe
└── audit.rs            # SecurityAuditLog: writes to security_audit_log table
```

### Core Types

```rust
// lazyjob-core/src/security/error.rs

#[derive(thiserror::Error, Debug)]
pub enum SecurityError {
    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("decryption error: {0}")]
    Decryption(String),

    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("invalid master password")]
    InvalidPassword,

    #[error("app is locked — call unlock() first")]
    AppLocked,

    #[error("export failed: {0}")]
    Export(String),

    #[error("wipe failed at step {step}: {reason}")]
    WipeFailed { step: &'static str, reason: String },

    #[error("database backup required before enabling encryption")]
    BackupRequired,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SecurityError>;
```

```rust
// lazyjob-core/src/security/credentials.rs

use keyring::Entry;
use secrecy::{ExposeSecret, Secret};
use zeroize::Zeroizing;

/// Namespace for all keyring entries.
const SERVICE: &str = "lazyjob";

/// Manages API keys and OAuth tokens in the OS keychain.
pub struct CredentialManager;

impl CredentialManager {
    pub fn store_api_key(
        &self,
        provider: &str,
        key: &Secret<String>,
    ) -> Result<()>;

    pub fn get_api_key(
        &self,
        provider: &str,
    ) -> Result<Option<Secret<String>>>;

    pub fn delete_api_key(&self, provider: &str) -> Result<()>;

    /// Store an opaque blob (e.g. sealed encryption key).
    pub fn store_blob(&self, key_name: &str, blob: &[u8]) -> Result<()>;

    /// Retrieve an opaque blob.
    pub fn get_blob(&self, key_name: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;

    /// Delete a blob entry.
    pub fn delete_blob(&self, key_name: &str) -> Result<()>;

    /// List all service keys managed by lazyjob (for wipe).
    pub fn list_keys(&self) -> Result<Vec<String>>;
}
```

```rust
// lazyjob-core/src/security/master_password.rs

use argon2::{Algorithm, Argon2, Params, Version};
use secrecy::{ExposeSecret, Secret};
use zeroize::Zeroizing;

/// Derive a 32-byte AES/age encryption key from a master password + stored salt.
/// The salt is stored in the OS keychain under the key "master_salt".
pub struct MasterPassword;

impl MasterPassword {
    /// First-time setup: generate a random 16-byte salt, derive a key,
    /// store the salt in the keychain, and return the derived key.
    pub fn initialize(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<Zeroizing<Vec<u8>>>;

    /// On subsequent unlocks: retrieve salt from keychain, derive key.
    pub fn derive_key(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<Zeroizing<Vec<u8>>>;

    /// Verify a password attempt without exposing the key.
    /// Returns true if the password matches.
    pub fn verify(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<bool>;
}

/// Argon2id parameters (OWASP recommended minimums for interactive use).
fn argon2_instance() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, Some(32)).expect("valid argon2 params"),
    )
}
```

```rust
// lazyjob-core/src/security/encryption.rs

use age::{Decryptor, Encryptor};
use secrecy::Secret;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Wraps the age encryption library for encrypting the SQLite database file.
/// Strategy: encrypt a *copy* of the database to `<path>.age`; the live
/// database is always plaintext while the app is running.
/// On startup, if `<path>.age` exists and `<path>` does not, decrypt to
/// working copy before opening.
pub struct AgeEncryption {
    key_material: Zeroizing<Vec<u8>>,
}

impl AgeEncryption {
    pub fn new(key_material: Zeroizing<Vec<u8>>) -> Self;

    /// Encrypt `source` → `dest` using a passphrase derived from key material.
    pub fn encrypt_file(&self, source: &Path, dest: &Path) -> Result<()>;

    /// Decrypt `source` → `dest`.
    pub fn decrypt_file(&self, source: &Path, dest: &Path) -> Result<()>;

    /// Encrypt in-memory bytes (for backup export).
    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>>;

    /// Decrypt in-memory bytes.
    pub fn decrypt_bytes(&self, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>>;
}
```

```rust
// lazyjob-core/src/security/privacy.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyMode {
    /// Full mode: all features, all LLM calls, full persistence.
    Full,
    /// Minimal: LLM calls permitted only for explicit user actions;
    /// no background Ralph loops; job descriptions not sent to LLM.
    Minimal,
    /// Stealth: no LLM calls; all data exists in-session only;
    /// SQLite writes are suppressed.
    Stealth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettings {
    pub mode: PrivacyMode,
    pub encrypt_database: bool,
    pub require_master_password: bool,
    pub auto_lock_minutes: Option<u32>,
    pub send_telemetry: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            mode: PrivacyMode::Full,
            encrypt_database: false,
            require_master_password: false,
            auto_lock_minutes: None,
            send_telemetry: false,
        }
    }
}
```

```rust
// lazyjob-core/src/security/export.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct FullExport {
    pub version: u32,            // export format version
    pub exported_at: DateTime<Utc>,
    pub jobs: Vec<crate::jobs::Job>,
    pub applications: Vec<crate::applications::Application>,
    pub contacts: Vec<crate::contacts::Contact>,
    pub interviews: Vec<crate::interviews::Interview>,
    pub offers: Vec<crate::offers::Offer>,
    pub life_sheet: Option<crate::life_sheet::LifeSheet>,
    pub settings: serde_json::Value,
}

#[derive(Debug)]
pub struct ExportReport {
    pub path: std::path::PathBuf,
    pub record_counts: RecordCounts,
    pub exported_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct RecordCounts {
    pub jobs: usize,
    pub applications: usize,
    pub contacts: usize,
    pub interviews: usize,
    pub offers: usize,
}

pub struct DataExporter {
    db: std::sync::Arc<crate::persistence::Database>,
}

impl DataExporter {
    pub async fn export_json(&self, dest: &Path) -> Result<ExportReport>;
    pub async fn export_encrypted(
        &self,
        dest: &Path,
        enc: &AgeEncryption,
    ) -> Result<ExportReport>;
    pub async fn import_json(&self, src: &Path) -> Result<RecordCounts>;
    pub async fn wipe_all(
        &self,
        cred: &CredentialManager,
        db_path: &Path,
    ) -> Result<()>;
}
```

```rust
// lazyjob-core/src/security/mod.rs

/// Top-level orchestrator for the security layer.
/// Created once at startup and shared via Arc.
pub struct SecurityLayer {
    pub cred: CredentialManager,
    pub privacy: PrivacySettings,
    encryption: Option<AgeEncryption>,    // Some if encrypt_database enabled
    locked: std::sync::atomic::AtomicBool,
}

impl SecurityLayer {
    /// Boot without a master password (encryption disabled, keyring still used).
    pub fn new(privacy: PrivacySettings) -> Self;

    /// Boot and unlock: derive key from password, decrypt DB file if needed.
    pub async fn unlock(
        password: &secrecy::Secret<String>,
        privacy: PrivacySettings,
        db_path: &std::path::Path,
    ) -> Result<Self>;

    /// Lock: zero the encryption key material.
    pub fn lock(&self);

    pub fn is_locked(&self) -> bool;

    /// Guard: call before any LLM operation in Minimal/Stealth mode.
    pub fn check_llm_allowed(&self) -> Result<()>;

    /// Guard: call before any write in Stealth mode.
    pub fn check_persistence_allowed(&self) -> Result<()>;

    /// Write an encrypted backup of the live database.
    pub async fn backup_encrypted(
        &self,
        db_path: &std::path::Path,
        backup_dest: &std::path::Path,
    ) -> Result<()>;
}
```

### Trait Definitions

```rust
/// Any type holding sensitive data must implement this to ensure
/// zeroize on drop.
pub trait SensitiveData: zeroize::Zeroize {
    fn expose(&self) -> &[u8];
}

/// Consumers of LLM providers must check the privacy guard.
/// LlmProvider implementations already exist; the guard is called by the
/// SecurityLayer wrapper, not the provider itself.
pub trait LlmGuard {
    fn check_allowed(&self, layer: &SecurityLayer) -> Result<()>;
}
```

### SQLite Schema

```sql
-- Migration 014: security audit log
CREATE TABLE IF NOT EXISTS security_audit_log (
    id       TEXT    PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    event    TEXT    NOT NULL,   -- 'unlock', 'lock', 'export', 'wipe', 'key_stored', 'key_deleted'
    actor    TEXT    NOT NULL DEFAULT 'user',
    detail   TEXT,               -- JSON blob with event-specific metadata
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_security_audit_created ON security_audit_log(created_at DESC);

-- Migration 015: privacy settings persisted alongside config
-- (stored in existing app_config table if present, else in new table)
CREATE TABLE IF NOT EXISTS privacy_settings (
    id              INTEGER PRIMARY KEY CHECK (id = 1),  -- singleton row
    mode            TEXT    NOT NULL DEFAULT 'Full',
    encrypt_database INTEGER NOT NULL DEFAULT 0,
    require_master_password INTEGER NOT NULL DEFAULT 0,
    auto_lock_minutes INTEGER,
    send_telemetry  INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO privacy_settings(id) VALUES(1);
```

### Module Structure

```
lazyjob-core/
  src/
    security/
      mod.rs              # SecurityLayer, re-exports
      error.rs            # SecurityError, Result
      credentials.rs      # CredentialManager
      encryption.rs       # AgeEncryption
      master_password.rs  # MasterPassword (Argon2id)
      privacy.rs          # PrivacyMode, PrivacySettings
      export.rs           # DataExporter, FullExport, ExportReport
      audit.rs            # SecurityAuditLog (writes to DB)
    persistence/
      mod.rs              # Database (existing)
      ...
  migrations/
    014_security_audit_log.sql
    015_privacy_settings.sql
```

## Implementation Phases

### Phase 1 — Credential Management (MVP)

**Goal**: All LLM API keys are stored in the OS keychain. No keys in SQLite, config files, or logs.

#### Step 1.1 — Add dependencies

In `lazyjob-core/Cargo.toml`:
```toml
keyring = "3"
secrecy = "0.8"
zeroize = { version = "1", features = ["derive"] }
```

Run `cargo build` to verify cross-platform keyring feature detection succeeds.

#### Step 1.2 — Implement `CredentialManager`

File: `lazyjob-core/src/security/credentials.rs`

```rust
use keyring::Entry;
use secrecy::{ExposeSecret, Secret};
use zeroize::Zeroizing;
use crate::security::{Result, SecurityError};

const SERVICE: &str = "lazyjob";

pub struct CredentialManager;

impl CredentialManager {
    pub fn store_api_key(
        &self,
        provider: &str,
        key: &Secret<String>,
    ) -> Result<()> {
        let entry = Entry::new(SERVICE, &format!("api_key:{provider}"))
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        entry
            .set_password(key.expose_secret())
            .map_err(|e| SecurityError::Keyring(e.to_string()))
    }

    pub fn get_api_key(&self, provider: &str) -> Result<Option<Secret<String>>> {
        let entry = Entry::new(SERVICE, &format!("api_key:{provider}"))
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(Secret::new(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecurityError::Keyring(e.to_string())),
        }
    }

    pub fn delete_api_key(&self, provider: &str) -> Result<()> {
        let entry = Entry::new(SERVICE, &format!("api_key:{provider}"))
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        entry
            .delete_credential()
            .map_err(|e| SecurityError::Keyring(e.to_string()))
    }

    pub fn store_blob(&self, key_name: &str, blob: &[u8]) -> Result<()> {
        // keyring stores strings; base64-encode blobs
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(blob);
        let entry = Entry::new(SERVICE, key_name)
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        entry
            .set_password(&encoded)
            .map_err(|e| SecurityError::Keyring(e.to_string()))
    }

    pub fn get_blob(&self, key_name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        use base64::Engine;
        let entry = Entry::new(SERVICE, key_name)
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(encoded) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|e| SecurityError::Keyring(e.to_string()))?;
                Ok(Some(Zeroizing::new(bytes)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecurityError::Keyring(e.to_string())),
        }
    }

    pub fn delete_blob(&self, key_name: &str) -> Result<()> {
        let entry = Entry::new(SERVICE, key_name)
            .map_err(|e| SecurityError::Keyring(e.to_string()))?;
        entry
            .delete_credential()
            .map_err(|e| SecurityError::Keyring(e.to_string()))
    }
}
```

**Verification**: Write a unit test that stores, retrieves, and deletes a mock API key. The test should pass on CI (Linux with `secret-service` mock via `keyring`'s `MockCredentialStore`).

#### Step 1.3 — Wire LLM providers to use `CredentialManager`

Update `lazyjob-llm/src/anthropic.rs` (and other providers) to load their API key via `CredentialManager::get_api_key("anthropic")` at construction time. Remove all direct reads from environment variables or config files for API keys.

**Key crate APIs**:
- `keyring::Entry::new(service, username) -> keyring::Result<Entry>`
- `Entry::set_password(&self, password: &str) -> keyring::Result<()>`
- `Entry::get_password(&self) -> keyring::Result<String>`
- `Entry::delete_credential(&self) -> keyring::Result<()>`
- `keyring::Error::NoEntry` — the "not found" variant

**Verification**: `cargo test -p lazyjob-core -- security::credentials` passes. Integration: run `lazyjob` with no env vars set and confirm it reads the API key from the keychain.

---

### Phase 2 — Argon2id Master Password + age Encryption

**Goal**: Users who opt in can protect their SQLite database with a master password. The database file is encrypted at rest using `age`; the app decrypts it to a working copy on unlock.

#### Step 2.1 — Add encryption dependencies

```toml
age     = { version = "0.10", features = ["armor"] }
argon2  = "0.5"
rand    = { version = "0.8", features = ["getrandom"] }
base64  = "0.22"
```

#### Step 2.2 — Implement `MasterPassword`

File: `lazyjob-core/src/security/master_password.rs`

```rust
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, Version};
use rand::RngCore;
use secrecy::{ExposeSecret, Secret};
use zeroize::Zeroizing;
use crate::security::{credentials::CredentialManager, Result, SecurityError};

const SALT_KEY: &str = "master_salt";
const VERIFIER_KEY: &str = "master_verifier";  // stored hash for password verify
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;

fn argon2() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, Some(KEY_LEN)).expect("valid params"),
    )
}

pub struct MasterPassword;

impl MasterPassword {
    /// First-time initialization: generate salt, derive key, store verifier.
    pub fn initialize(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let mut salt = vec![0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        let key = Self::derive_raw(password, &salt)?;

        // Store salt in keychain
        cred.store_blob(SALT_KEY, &salt)?;

        // Store PHC hash string as password verifier (for verify() later)
        use argon2::password_hash::{PasswordHasher, SaltString};
        let salt_str = SaltString::encode_b64(&salt)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        let hash = argon2()
            .hash_password(password.expose_secret().as_bytes(), &salt_str)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?
            .to_string();
        cred.store_blob(VERIFIER_KEY, hash.as_bytes())?;

        Ok(key)
    }

    /// Unlock: load salt from keychain, re-derive key.
    pub fn derive_key(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let salt = cred
            .get_blob(SALT_KEY)?
            .ok_or(SecurityError::InvalidPassword)?;
        Self::derive_raw(password, &salt)
    }

    /// Verify a password without exposing the derived key.
    pub fn verify(
        password: &Secret<String>,
        cred: &CredentialManager,
    ) -> Result<bool> {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let verifier_bytes = cred
            .get_blob(VERIFIER_KEY)?
            .ok_or(SecurityError::InvalidPassword)?;
        let hash_str = std::str::from_utf8(&verifier_bytes)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        let parsed = PasswordHash::new(hash_str)
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        Ok(argon2()
            .verify_password(password.expose_secret().as_bytes(), &parsed)
            .is_ok())
    }

    fn derive_raw(
        password: &Secret<String>,
        salt: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        let mut key = Zeroizing::new(vec![0u8; KEY_LEN]);
        argon2()
            .hash_password_into(
                password.expose_secret().as_bytes(),
                salt,
                &mut key,
            )
            .map_err(|e| SecurityError::KeyDerivation(e.to_string()))?;
        Ok(key)
    }
}
```

**Key crate APIs**:
- `argon2::Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(m, t, p, len))`
- `Argon2::hash_password_into(&self, password, salt, out) -> argon2::Result<()>`
- `argon2::password_hash::PasswordHasher::hash_password(&self, pw, salt) -> Result<PasswordHash>`
- `argon2::password_hash::PasswordVerifier::verify_password(&self, pw, hash) -> Result<()>`
- `rand::thread_rng().fill_bytes(buf)`
- `zeroize::Zeroizing<Vec<u8>>` — wraps a Vec and calls `zeroize()` on drop

**Verification**: Unit test creates a password, initializes, derives key, verifies correct/wrong passwords.

#### Step 2.3 — Implement `AgeEncryption`

File: `lazyjob-core/src/security/encryption.rs`

```rust
use age::{scrypt, Decryptor, Encryptor};
use secrecy::Secret;
use std::{
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;
use crate::security::{Result, SecurityError};

pub struct AgeEncryption {
    passphrase: Secret<String>,  // derived from key material
}

impl AgeEncryption {
    /// Build from raw key material (32 bytes).
    /// We base64-encode the key material as the age passphrase.
    pub fn from_key(key_material: &Zeroizing<Vec<u8>>) -> Self {
        use base64::Engine;
        let passphrase = base64::engine::general_purpose::STANDARD
            .encode(key_material.as_slice());
        Self {
            passphrase: Secret::new(passphrase),
        }
    }

    pub fn encrypt_file(&self, source: &Path, dest: &Path) -> Result<()> {
        use secrecy::ExposeSecret;
        let plaintext = std::fs::read(source)?;
        let ciphertext = self.encrypt_bytes(&plaintext)?;
        std::fs::write(dest, &ciphertext)?;
        Ok(())
    }

    pub fn decrypt_file(&self, source: &Path, dest: &Path) -> Result<()> {
        let ciphertext = std::fs::read(source)?;
        let plaintext = self.decrypt_bytes(&ciphertext)?;
        std::fs::write(dest, plaintext.as_slice())?;
        Ok(())
    }

    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        use secrecy::ExposeSecret;
        let encryptor = Encryptor::with_user_passphrase(
            Secret::new(self.passphrase.expose_secret().to_string()),
        );
        let mut output = Vec::new();
        let mut writer = encryptor
            .wrap_output(&mut output)
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;
        writer
            .write_all(plaintext)
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| SecurityError::Encryption(e.to_string()))?;
        Ok(output)
    }

    pub fn decrypt_bytes(&self, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        use secrecy::ExposeSecret;
        let decryptor = Decryptor::new(ciphertext)
            .map_err(|e| SecurityError::Decryption(e.to_string()))?;
        let mut decryptor = match decryptor {
            Decryptor::Passphrase(d) => d,
            _ => return Err(SecurityError::Decryption(
                "unexpected age recipient type".into(),
            )),
        };
        let mut plaintext = Zeroizing::new(Vec::new());
        let mut reader = decryptor
            .decrypt(
                &Secret::new(self.passphrase.expose_secret().to_string()),
                None,
            )
            .map_err(|e| SecurityError::Decryption(e.to_string()))?;
        reader
            .read_to_end(&mut plaintext)
            .map_err(|e| SecurityError::Decryption(e.to_string()))?;
        Ok(plaintext)
    }
}
```

**Key crate APIs**:
- `age::Encryptor::with_user_passphrase(passphrase: Secret<String>) -> Encryptor`
- `Encryptor::wrap_output(output: impl Write) -> age::Result<impl Write + 'static>`
- `age::Decryptor::new(reader: impl Read) -> age::Result<Decryptor>`
- `age::decryptor::PassphraseDecryptor::decrypt(passphrase, max_work_factor) -> age::Result<impl Read>`

**Verification**: Round-trip test encrypts a known plaintext, decrypts it, asserts equality. Wrong passphrase test confirms decryption fails with `SecurityError::Decryption`.

#### Step 2.4 — Database startup flow

In `lazyjob-core/src/persistence/mod.rs`, add a method that handles the encrypted DB boot sequence:

```rust
impl Database {
    /// Open the database, decrypting from `<db_path>.age` if encrypted.
    /// If encryption is enabled but no `.age` file exists, assumes first run
    /// and proceeds with plaintext (encryption begins on next backup/rotate).
    pub async fn open_with_security(
        db_path: &Path,
        encryption: Option<&AgeEncryption>,
    ) -> crate::Result<Self> {
        let encrypted_path = db_path.with_extension("db.age");

        if let Some(enc) = encryption {
            if encrypted_path.exists() && !db_path.exists() {
                // Decrypt to working copy
                enc.decrypt_file(&encrypted_path, db_path)
                    .map_err(|e| DbError::SecurityError(e.to_string()))?;
            }
        }

        Self::new(db_path).await
    }
}
```

And a complementary method for shutdown:
```rust
impl Database {
    /// On graceful shutdown, if encryption is enabled, overwrite the `.age`
    /// file with a fresh encrypted backup, then optionally zero-overwrite
    /// and delete the plaintext working copy.
    pub async fn close_encrypted(
        self,
        db_path: &Path,
        encryption: &AgeEncryption,
        wipe_plaintext: bool,
    ) -> crate::Result<()> {
        self.close().await?;
        let encrypted_path = db_path.with_extension("db.age");
        encryption.encrypt_file(db_path, &encrypted_path)
            .map_err(|e| DbError::SecurityError(e.to_string()))?;
        if wipe_plaintext {
            secure_delete(db_path)?;
        }
        Ok(())
    }
}

/// Overwrite with zeros before deletion to prevent recovery from disk.
fn secure_delete(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let len = std::fs::metadata(path)?.len();
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    let zeros = vec![0u8; len as usize];
    f.write_all(&zeros)?;
    f.flush()?;
    drop(f);
    std::fs::remove_file(path)
}
```

**Verification**: Test: create DB, insert rows, close encrypted, open with encryption, query rows → identical.

---

### Phase 3 — Privacy Modes + Auto-Lock

**Goal**: Enforce `PrivacyMode` at runtime boundaries.

#### Step 3.1 — `SecurityLayer` orchestrator

File: `lazyjob-core/src/security/mod.rs`

```rust
use std::sync::atomic::{AtomicBool, Ordering};

pub struct SecurityLayer {
    pub cred: CredentialManager,
    pub privacy: PrivacySettings,
    encryption: Option<AgeEncryption>,
    locked: AtomicBool,
    lock_deadline: Option<tokio::time::Instant>,
}

impl SecurityLayer {
    pub fn new_unencrypted(privacy: PrivacySettings) -> Self {
        Self {
            cred: CredentialManager,
            encryption: None,
            locked: AtomicBool::new(false),
            lock_deadline: privacy.auto_lock_minutes.map(|m| {
                tokio::time::Instant::now()
                    + std::time::Duration::from_secs(m as u64 * 60)
            }),
            privacy,
        }
    }

    pub async fn unlock(
        password: &secrecy::Secret<String>,
        privacy: PrivacySettings,
        db_path: &std::path::Path,
    ) -> crate::security::Result<(Self, crate::persistence::Database)> {
        let cred = CredentialManager;

        // Verify password
        if !MasterPassword::verify(password, &cred)? {
            return Err(SecurityError::InvalidPassword);
        }

        let key = MasterPassword::derive_key(password, &cred)?;
        let encryption = AgeEncryption::from_key(&key);

        let db = crate::persistence::Database::open_with_security(
            db_path,
            Some(&encryption),
        )
        .await
        .map_err(|e| SecurityError::Decryption(e.to_string()))?;

        let layer = Self {
            cred,
            encryption: Some(encryption),
            locked: AtomicBool::new(false),
            lock_deadline: privacy.auto_lock_minutes.map(|m| {
                tokio::time::Instant::now()
                    + std::time::Duration::from_secs(m as u64 * 60)
            }),
            privacy,
        };

        Ok((layer, db))
    }

    pub fn lock(&self) {
        self.locked.store(true, Ordering::SeqCst);
        // Drop encryption key by zeroing its storage is handled when the
        // SecurityLayer is replaced; this method sets the "locked" flag
        // to block further DB access from the TUI.
    }

    pub fn is_locked(&self) -> bool {
        if let Some(deadline) = self.lock_deadline {
            if tokio::time::Instant::now() >= deadline {
                self.locked.store(true, Ordering::SeqCst);
            }
        }
        self.locked.load(Ordering::SeqCst)
    }

    pub fn check_llm_allowed(&self) -> crate::security::Result<()> {
        if self.is_locked() {
            return Err(SecurityError::AppLocked);
        }
        match self.privacy.mode {
            PrivacyMode::Stealth => Err(SecurityError::Encryption(
                "LLM calls disabled in Stealth mode".into(),
            )),
            _ => Ok(()),
        }
    }

    pub fn check_persistence_allowed(&self) -> crate::security::Result<()> {
        if self.is_locked() {
            return Err(SecurityError::AppLocked);
        }
        match self.privacy.mode {
            PrivacyMode::Stealth => Err(SecurityError::Encryption(
                "persistence disabled in Stealth mode".into(),
            )),
            _ => Ok(()),
        }
    }
}
```

#### Step 3.2 — Auto-lock background task

In `lazyjob-tui` or `lazyjob-cli`, spawn a background tokio task that polls the auto-lock deadline:

```rust
// lazyjob-tui/src/auto_lock.rs

pub async fn run_auto_lock(
    layer: std::sync::Arc<crate::security::SecurityLayer>,
    tx: tokio::sync::mpsc::Sender<crate::AppEvent>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        if layer.is_locked() {
            let _ = tx.send(crate::AppEvent::AppLocked).await;
            return;
        }
    }
}
```

The TUI receives `AppEvent::AppLocked` and transitions to a lock screen view.

---

### Phase 4 — Data Export and Wipe

**Goal**: Users can export all their data to JSON (or encrypted JSON), import from a backup, and perform a verified full wipe.

#### Step 4.1 — Implement `DataExporter`

File: `lazyjob-core/src/security/export.rs`

```rust
impl DataExporter {
    pub async fn export_json(&self, dest: &Path) -> Result<ExportReport> {
        let payload = self.build_payload().await?;
        let json = serde_json::to_string_pretty(&payload)
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        // Write to a temp file first; atomically rename to dest
        let tmp = dest.with_extension("tmp");
        tokio::fs::write(&tmp, json.as_bytes())
            .await
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        tokio::fs::rename(&tmp, dest)
            .await
            .map_err(|e| SecurityError::Export(e.to_string()))?;

        Ok(ExportReport {
            path: dest.to_path_buf(),
            record_counts: payload.counts(),
            exported_at: payload.exported_at,
        })
    }

    pub async fn export_encrypted(
        &self,
        dest: &Path,
        enc: &AgeEncryption,
    ) -> Result<ExportReport> {
        let payload = self.build_payload().await?;
        let json = serde_json::to_string_pretty(&payload)
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        let ciphertext = enc.encrypt_bytes(json.as_bytes())?;
        let tmp = dest.with_extension("tmp");
        tokio::fs::write(&tmp, &ciphertext)
            .await
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        tokio::fs::rename(&tmp, dest)
            .await
            .map_err(|e| SecurityError::Export(e.to_string()))?;

        Ok(ExportReport {
            path: dest.to_path_buf(),
            record_counts: payload.counts(),
            exported_at: payload.exported_at,
        })
    }

    pub async fn import_json(&self, src: &Path) -> Result<RecordCounts> {
        let json = tokio::fs::read_to_string(src)
            .await
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        let payload: FullExport = serde_json::from_str(&json)
            .map_err(|e| SecurityError::Export(e.to_string()))?;
        self.persist_payload(payload).await
    }

    /// Full data wipe:
    /// 1. Delete all keychain entries for lazyjob
    /// 2. Secure-delete the SQLite database file (overwrite + remove)
    /// 3. Remove config directory
    /// Returns an error at the first failed step.
    pub async fn wipe_all(
        &self,
        cred: &CredentialManager,
        db_path: &Path,
    ) -> Result<()> {
        // Step 1: Drop all keychain entries
        for key in cred.list_keys()? {
            cred.delete_blob(&key).unwrap_or(());
        }
        for provider in &["anthropic", "openai", "ollama"] {
            cred.delete_api_key(provider).unwrap_or(());
        }

        // Step 2: Secure-delete database files
        for path in &[
            db_path.to_path_buf(),
            db_path.with_extension("db.age"),
            db_path.with_extension("db-wal"),
            db_path.with_extension("db-shm"),
        ] {
            if path.exists() {
                secure_delete(path).map_err(SecurityError::Io)?;
            }
        }

        // Step 3: Remove config directory (caller confirms before calling)
        let config_dir = db_path
            .parent()
            .ok_or_else(|| SecurityError::WipeFailed {
                step: "config_dir",
                reason: "cannot determine config directory".into(),
            })?;
        tokio::fs::remove_dir_all(config_dir)
            .await
            .map_err(|e| SecurityError::WipeFailed {
                step: "config_dir",
                reason: e.to_string(),
            })?;

        Ok(())
    }

    async fn build_payload(&self) -> Result<FullExport> { ... }
    async fn persist_payload(&self, payload: FullExport) -> Result<RecordCounts> { ... }
}
```

#### Step 4.2 — TUI export/wipe flow

In `lazyjob-tui`, add an `ExportPanel` with three actions:
- **Export plaintext** → prompt for destination path → call `DataExporter::export_json`
- **Export encrypted** → prompt for destination path → call `DataExporter::export_encrypted`
- **Wipe all data** → show a typed confirmation prompt (`"type WIPE to confirm"`) → call `DataExporter::wipe_all`

The TUI displays an `ExportReport` summary in a popup after success.

---

### Phase 5 — Security Audit Log + TUI Status Panel

#### Step 5.1 — `SecurityAuditLog`

File: `lazyjob-core/src/security/audit.rs`

```rust
pub struct SecurityAuditLog {
    pool: sqlx::SqlitePool,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "event")]
pub enum AuditEvent {
    Unlock { success: bool },
    Lock,
    KeyStored { provider: String },
    KeyDeleted { provider: String },
    Export { path: String, record_count: usize },
    Wipe,
}

impl SecurityAuditLog {
    pub async fn record(&self, event: AuditEvent) -> crate::Result<()> {
        let event_name = match &event {
            AuditEvent::Unlock { .. } => "unlock",
            AuditEvent::Lock => "lock",
            AuditEvent::KeyStored { .. } => "key_stored",
            AuditEvent::KeyDeleted { .. } => "key_deleted",
            AuditEvent::Export { .. } => "export",
            AuditEvent::Wipe => "wipe",
        };
        let detail = serde_json::to_string(&event).ok();
        sqlx::query!(
            r#"INSERT INTO security_audit_log (event, detail) VALUES (?, ?)"#,
            event_name,
            detail,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn recent(&self, limit: u32) -> crate::Result<Vec<AuditRow>> {
        sqlx::query_as!(
            AuditRow,
            r#"SELECT id, event, actor, detail, created_at
               FROM security_audit_log
               ORDER BY created_at DESC
               LIMIT ?"#,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}
```

#### Step 5.2 — TUI security status indicator

In the TUI status bar (bottom row), display:
- `[locked]` if `SecurityLayer::is_locked()`
- `[encrypted]` if `privacy.encrypt_database`
- `[stealth]` if `privacy.mode == PrivacyMode::Stealth`
- `[minimal]` if `privacy.mode == PrivacyMode::Minimal`

These are rendered as `ratatui::widgets::Span` with styled colors using `ratatui::style::Style`.

---

### Phase 6 — SQLCipher (Optional, Power Users)

This phase is deferred. SQLCipher requires:
1. The `rusqlite` crate compiled with the `bundled-sqlcipher` feature flag (pulls in OpenSSL or uses sqlcipher amalgamation).
2. A compilation dependency on `cc` for the C code.
3. A key pragma: `PRAGMA key = 'passphrase'` immediately after connection open.

If implemented, add a `DatabaseEncryptionBackend` enum:
```rust
pub enum DatabaseEncryptionBackend {
    None,
    AgeFull,        // Phase 2: encrypt whole file with age
    SqlCipher,      // Phase 6: page-level transparent encryption
}
```

**Reason for deferral**: `age` file-level encryption covers the data-at-rest threat model for 99% of users. SQLCipher adds build complexity with marginal additional security for LazyJob's local-first use case (the threat model is a lost laptop, not a live database attack).

## Key Crate APIs

| API | Use |
|-----|-----|
| `keyring::Entry::new(service, username) -> Result<Entry>` | Create a keyring entry handle |
| `Entry::set_password(&self, pw: &str) -> keyring::Result<()>` | Store credential |
| `Entry::get_password(&self) -> keyring::Result<String>` | Retrieve credential |
| `Entry::delete_credential(&self) -> keyring::Result<()>` | Remove credential |
| `keyring::Error::NoEntry` | Distinguish "not set" from error |
| `argon2::Argon2::new(alg, ver, params)` | Create Argon2 instance |
| `Argon2::hash_password_into(&self, pw, salt, out)` | Derive key bytes |
| `argon2::password_hash::PasswordHasher::hash_password(&self, pw, salt)` | Create PHC hash string for verification storage |
| `argon2::password_hash::PasswordVerifier::verify_password(&self, pw, hash)` | Verify password |
| `age::Encryptor::with_user_passphrase(passphrase: Secret<String>)` | Create age encryptor |
| `Encryptor::wrap_output(output: impl Write) -> age::Result<Box<dyn Write>>` | Wrap output stream |
| `age::Decryptor::new(reader: impl Read) -> age::Result<Decryptor>` | Detect age envelope format |
| `age::decryptor::PassphraseDecryptor::decrypt(pw, max_work_factor)` | Decrypt with passphrase |
| `secrecy::Secret::new(value)` | Wrap sensitive value |
| `secrecy::ExposeSecret::expose_secret(&self)` | Access raw value |
| `zeroize::Zeroizing::new(value)` | Wrap buffer with auto-zeroize on drop |
| `rand::thread_rng().fill_bytes(buf)` | Cryptographically secure random bytes |
| `base64::engine::general_purpose::STANDARD.encode(bytes)` | Encode blob for keyring storage |
| `sqlx::query!()` | Compile-time-checked SQL for audit log writes |

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum SecurityError {
    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("decryption failed (wrong password?)")]
    Decryption(String),

    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("invalid master password")]
    InvalidPassword,

    #[error("app is locked — call unlock() first")]
    AppLocked,

    #[error("LLM call not allowed in current privacy mode")]
    LlmDisabled,

    #[error("database write not allowed in Stealth mode")]
    PersistenceDisabled,

    #[error("export failed: {0}")]
    Export(String),

    #[error("wipe failed at step '{step}': {reason}")]
    WipeFailed { step: &'static str, reason: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SecurityError>;
```

**Error handling rules**:
- Never log raw `SecurityError::InvalidPassword` with the password value — it contains none, so this is safe, but log at `tracing::warn!` level only.
- `SecurityError::Keyring` always wraps the underlying keyring error as a `String`, not the raw type, to avoid leaking implementation details.
- The TUI maps `SecurityError::AppLocked` → show lock screen; `SecurityError::LlmDisabled` → show a "disabled in current privacy mode" inline message.

## Testing Strategy

### Unit Tests

**`credentials.rs`**:
```rust
#[cfg(test)]
mod tests {
    use keyring::mock;

    #[test]
    fn round_trip_api_key() {
        let cred = CredentialManager;
        let key = Secret::new("sk-ant-test".to_string());
        cred.store_api_key("anthropic", &key).unwrap();
        let retrieved = cred.get_api_key("anthropic").unwrap().unwrap();
        assert_eq!(retrieved.expose_secret(), "sk-ant-test");
        cred.delete_api_key("anthropic").unwrap();
        assert!(cred.get_api_key("anthropic").unwrap().is_none());
    }
}
```

Use `keyring`'s built-in mock credential store (enabled in tests with `keyring::set_default_credential_builder(keyring::mock::default_credential_builder())`).

**`master_password.rs`**:
```rust
#[test]
fn argon2_round_trip() {
    // Initialize with password "hunter2"
    // Derive key → same 32 bytes
    // Verify "hunter2" → true
    // Verify "wrong" → false
}
```

**`encryption.rs`**:
```rust
#[test]
fn age_round_trip() {
    let key_material = Zeroizing::new(vec![0xABu8; 32]);
    let enc = AgeEncryption::from_key(&key_material);
    let plaintext = b"hello world";
    let ciphertext = enc.encrypt_bytes(plaintext).unwrap();
    let decrypted = enc.decrypt_bytes(&ciphertext).unwrap();
    assert_eq!(decrypted.as_slice(), plaintext);
}

#[test]
fn wrong_passphrase_fails() {
    let enc1 = AgeEncryption::from_key(&Zeroizing::new(vec![0xABu8; 32]));
    let enc2 = AgeEncryption::from_key(&Zeroizing::new(vec![0xCDu8; 32]));
    let ciphertext = enc1.encrypt_bytes(b"secret").unwrap();
    assert!(enc2.decrypt_bytes(&ciphertext).is_err());
}
```

**`export.rs`**:
- Test `export_json` creates a valid JSON file
- Test `import_json` round-trips the same data
- Test `wipe_all` removes the DB file and config dir (use `tempfile::TempDir`)

### Integration Tests

```rust
// tests/security_integration.rs

#[tokio::test]
async fn database_encrypt_decrypt_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // 1. Open unencrypted DB, insert a job
    let db = Database::new(&db_path).await.unwrap();
    db.jobs().insert(&make_test_job()).await.unwrap();
    db.close().await.unwrap();

    // 2. Encrypt the DB
    let key = Zeroizing::new(vec![0xAAu8; 32]);
    let enc = AgeEncryption::from_key(&key);
    enc.encrypt_file(&db_path, &db_path.with_extension("db.age")).unwrap();
    std::fs::remove_file(&db_path).unwrap();

    // 3. Open with decryption
    let db = Database::open_with_security(&db_path, Some(&enc)).await.unwrap();
    let jobs = db.jobs().list(&Default::default()).await.unwrap();
    assert_eq!(jobs.len(), 1);
}
```

### TUI Tests

- `ExportPanel` renders with 3 action buttons; pressing `e` triggers export flow
- Lock screen renders `[locked]` indicator; pressing `u` opens password prompt
- Privacy mode badge renders correctly for each `PrivacyMode` variant

## Open Questions

1. **Keyring availability on headless CI**: `keyring = "3"` falls back to a file-based credential store on platforms without a keyring daemon. This is acceptable for development but not production. CI should either mock the keyring or use the in-memory mock backend explicitly.

2. **secure_delete reliability**: Overwriting with zeros before `remove_file` is not guaranteed to overwrite physical disk blocks on SSDs with wear leveling or copy-on-write filesystems (btrfs, ZFS, APFS). For a truly paranoid wipe, we would need OS-specific calls (`BLKSECDISCARD` on Linux, `purge` on macOS). For MVP, the overwrite + delete is sufficient for typical laptop threat models.

3. **age format version stability**: The `age` crate is on version 0.10 and the format is stable (age v1). The binary format header `age-encryption.org/v1` ensures future versions remain decodable.

4. **Master password UX**: If the user forgets their master password, all encrypted data is irrecoverable. The TUI must display a clear warning at the point of enabling encryption and prompt for a recovery code export (see `XX-encrypted-backup-export.md`).

5. **Privacy mode in Stealth**: In Stealth mode, even failed write attempts should be silent (no errors shown to user). Or should they show a brief status bar message? Needs UX decision.

6. **Cross-device credential sharing**: Keychain entries are per-device. If a user moves to a new device, they must re-enter all API keys. This is acceptable for the local-first MVP.

7. **Memory locking (mlock)**: On Linux, `mlock(2)` prevents sensitive pages from being swapped to disk. The `zeroize` crate does not call `mlock`. Adding `nix::sys::mman::mlock` around Argon2 output buffers would be a security enhancement for Phase 5+.

## Related Specs
- `specs/04-sqlite-persistence.md` — Database struct this plan extends
- `specs/XX-master-password-app-unlock.md` — Lock screen TUI design
- `specs/XX-encrypted-backup-export.md` — Encrypted backup format
- `specs/02-llm-provider-abstraction.md` — LLM providers that consume API keys from `CredentialManager`
- `specs/11-platform-api-integrations.md` — Platform credentials also stored via `CredentialManager`
