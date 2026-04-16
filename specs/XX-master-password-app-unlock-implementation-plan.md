# Implementation Plan: Master Password & App Unlock

## Status
Draft

## Related Spec
`specs/XX-master-password-app-unlock.md`

## Overview

LazyJob stores sensitive career and personal data locally — resumes, salary expectations, job applications, LLM API keys, and contact graphs. Without authentication, anyone with filesystem access to the machine can read all of it. This plan implements a master-password-based unlock flow that gates access to the app, integrates with the encryption layer (established in `specs/16-privacy-security.md`), and manages session lifetime in memory only.

The security model is: the master password is **never stored**. Instead, an Argon2id-derived key unlocks the SQLite database (via `age` file encryption, per the Phase 1 design in spec 16) and is held in a zeroing heap buffer (`zeroize::Zeroizing<[u8; 32]>`) for the duration of the session. The only persistent state is a PHC hash string for password verification (stored in a `password_verifier` table inside the encrypted DB, or in a separate small unencrypted metadata file for bootstrapping — see Phase 2).

A key design choice is separation of concerns:
- `MasterPasswordService` — Argon2id derivation, verification, password change, recovery key
- `UnlockFlow` — attempt counting, lockout enforcement, session creation
- `Session` — in-memory key holder with inactivity timer
- `LockScreenView` — ratatui TUI widget driving the unlock flow

Phase 1 ships the CLI `lazyjob init` first-time setup flow + lock screen. Phase 2 adds biometric unlock (macOS Touch ID via `security-framework`). Phase 3 adds password change and recovery key workflows.

## Prerequisites

### Specs That Must Be Implemented First
- `specs/16-privacy-security.md` — `AgeEncryption`, `CredentialManager`, and `SecurityError` types must exist
- `specs/04-sqlite-persistence.md` — `Database` struct must exist; migration infrastructure must be in place
- `specs/09-tui-design-keybindings.md` — `AppState` enum and TUI event loop must exist (the lock screen is a root-level `AppState` variant)

### Crates to Add
Add to `lazyjob-core/Cargo.toml`:
```toml
argon2      = "0.5"
zeroize     = { version = "1.7", features = ["derive", "zeroize_derive"] }
secrecy     = "0.8"
uuid        = { version = "1", features = ["v4"] }
chrono      = { version = "0.4", features = ["serde"] }
rand        = { version = "0.8", features = ["getrandom"] }
base64      = "0.22"
keyring     = "3"
once_cell   = "1"
thiserror   = "1"
```

Add to `lazyjob-tui/Cargo.toml`:
```toml
# Already present for other features, just confirm:
ratatui     = "0.27"
crossterm   = "0.27"
unicode-width = "0.1"
```

On macOS only, add to `lazyjob-core/Cargo.toml`:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "2.11"
```

## Architecture

### Crate Placement

Authentication logic lives in **`lazyjob-core/src/auth/`**. This module is separate from `src/security/` (which handles encryption, privacy mode, and data export). The `auth` module's primary contract: given a user-supplied password, produce a `Session` with an in-memory `DerivedKey`; given that `Session`, provide the key to `AgeEncryption` to decrypt/re-encrypt the database at app open/close.

The TUI lock screen (`LockScreenView`) lives in **`lazyjob-tui/src/views/lock_screen.rs`** and calls into `lazyjob-core` auth APIs exclusively.

### Module Structure

```
lazyjob-core/src/auth/
├── mod.rs                # re-exports; AuthService orchestrator
├── error.rs              # AuthError enum, Result alias
├── kdf.rs                # MasterPasswordService: Argon2id derive + verify
├── session.rs            # Session, DerivedKey, SessionStore
├── unlock.rs             # UnlockFlow: attempt counting, lockout, session creation
├── biometric.rs          # BiometricUnlock (cfg(target_os = "macos") gated)
├── recovery.rs           # RecoveryKeyService: generate, store, restore
├── strength.rs           # validate_password_strength()
└── metadata.rs           # AuthMetadata: persisted verifier + salt storage

lazyjob-tui/src/views/
└── lock_screen.rs        # LockScreenView, PasswordInput widget
```

### Core Types

```rust
// lazyjob-core/src/auth/error.rs

#[derive(thiserror::Error, Debug)]
pub enum AuthError {
    #[error("invalid master password")]
    InvalidPassword,

    #[error("account locked until {0}")]
    AccountLocked(chrono::DateTime<chrono::Utc>),

    #[error("no attempts remaining")]
    NoAttemptsRemaining,

    #[error("key derivation failed: {0}")]
    Kdf(String),

    #[error("password too weak: {0}")]
    WeakPassword(String),

    #[error("biometric authentication unavailable")]
    BiometricUnavailable,

    #[error("biometric authentication failed")]
    BiometricFailed,

    #[error("password not yet set — run `lazyjob init` first")]
    NotInitialized,

    #[error("recovery key error: {0}")]
    Recovery(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AuthError>;
```

```rust
// lazyjob-core/src/auth/kdf.rs

use argon2::{Argon2, Algorithm, Version, Params, password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString}};
use rand::rngs::OsRng;
use zeroize::Zeroizing;

/// Argon2id parameters (OWASP balanced profile: 19 MiB, 2 iterations, 1 lane).
/// For memory-constrained systems the config can override these.
pub struct Argon2Params {
    pub m_cost: u32,    // KiB; default 19456 (19 MiB)
    pub t_cost: u32,    // iterations; default 2
    pub p_cost: u32,    // parallelism; default 1
}

impl Default for Argon2Params {
    fn default() -> Self {
        Self { m_cost: 19456, t_cost: 2, p_cost: 1 }
    }
}

/// Raw 256-bit key material, zeroed on drop.
pub struct DerivedKey(pub Zeroizing<[u8; 32]>);

impl DerivedKey {
    /// Borrow the raw key bytes for passing to AgeEncryption.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub struct MasterPasswordService {
    params: Argon2Params,
}

impl MasterPasswordService {
    pub fn new(params: Argon2Params) -> Self {
        Self { params }
    }

    /// Derive a 256-bit key from password + stored salt.
    /// `salt_b64` is a URL-safe base64-encoded 16-byte random salt.
    pub fn derive_key(&self, password: &str, salt_b64: &str) -> Result<DerivedKey> {
        let salt = SaltString::from_b64(salt_b64)
            .map_err(|e| AuthError::Kdf(e.to_string()))?;

        let argon2 = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(self.params.m_cost, self.params.t_cost, self.params.p_cost, Some(32))
                .map_err(|e| AuthError::Kdf(e.to_string()))?,
        );

        let mut key = Zeroizing::new([0u8; 32]);
        argon2
            .hash_password_into(password.as_bytes(), salt.as_str().as_bytes(), key.as_mut())
            .map_err(|e| AuthError::Kdf(e.to_string()))?;

        Ok(DerivedKey(key))
    }

    /// Produce a PHC string (argon2id$...) suitable for storage in AuthMetadata.
    /// Uses a fresh random salt internally; returns (phc_string, salt_b64) pair.
    pub fn hash_password(&self, password: &str) -> Result<(String, String)> {
        let salt = SaltString::generate(&mut OsRng);

        let argon2 = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(self.params.m_cost, self.params.t_cost, self.params.p_cost, None)
                .map_err(|e| AuthError::Kdf(e.to_string()))?,
        );

        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AuthError::Kdf(e.to_string()))?
            .to_string();

        Ok((hash, salt.to_string()))
    }

    /// Verify a plaintext password against a previously generated PHC string.
    pub fn verify_password(&self, password: &str, phc_hash: &str) -> Result<bool> {
        let parsed = PasswordHash::new(phc_hash)
            .map_err(|e| AuthError::Kdf(e.to_string()))?;

        Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
    }

    /// Generate a fresh random 16-byte salt, returned as URL-safe base64.
    pub fn generate_salt() -> String {
        SaltString::generate(&mut OsRng).to_string()
    }
}
```

```rust
// lazyjob-core/src/auth/session.rs

use chrono::{DateTime, Utc, Duration};
use uuid::Uuid;
use zeroize::Zeroizing;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// In-memory only — never serialized.
pub struct Session {
    pub id: Uuid,
    encryption_key: Zeroizing<[u8; 32]>,
    pub created_at: DateTime<Utc>,
    last_activity: Arc<Mutex<DateTime<Utc>>>,
    pub timeout: Duration,
    // When this sender is dropped, the watch receiver sees `true` and locks.
    cancel_tx: watch::Sender<bool>,
}

impl Session {
    pub fn new(key: DerivedKey, timeout_minutes: u32) -> (Self, watch::Receiver<bool>) {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let session = Self {
            id: Uuid::new_v4(),
            encryption_key: key.0,
            created_at: Utc::now(),
            last_activity: Arc::new(Mutex::new(Utc::now())),
            timeout: Duration::minutes(timeout_minutes as i64),
            cancel_tx,
        };
        (session, cancel_rx)
    }

    /// Borrow the raw key bytes — must not be stored or cloned.
    pub fn borrow_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }

    /// Call on any user action to reset the inactivity timer.
    pub fn touch(&self) {
        *self.last_activity.lock().unwrap() = Utc::now();
    }

    pub fn is_expired(&self) -> bool {
        let last = *self.last_activity.lock().unwrap();
        Utc::now() - last > self.timeout
    }

    /// Signal the inactivity background task to lock immediately.
    pub fn force_lock(self) {
        let _ = self.cancel_tx.send(true);
        // encryption_key and other secrets zeroize on drop here
    }
}
```

```rust
// lazyjob-core/src/auth/unlock.rs

use chrono::{DateTime, Utc, Duration};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub struct LockoutState {
    attempts_remaining: u8,
    lockout_until: Option<DateTime<Utc>>,
}

pub struct UnlockFlow {
    pub max_attempts: u8,
    pub lockout_duration: Duration,
    pub session_timeout_minutes: u32,
    state: Arc<Mutex<LockoutState>>,
    kdf: MasterPasswordService,
}

pub enum UnlockResult {
    Success(Session, watch::Receiver<bool>),
    InvalidPassword { attempts_remaining: u8 },
    Locked { until: DateTime<Utc> },
}

impl UnlockFlow {
    pub fn new(params: Argon2Params, max_attempts: u8, session_timeout_minutes: u32) -> Self {
        Self {
            max_attempts,
            lockout_duration: Duration::minutes(5),
            session_timeout_minutes,
            state: Arc::new(Mutex::new(LockoutState {
                attempts_remaining: max_attempts,
                lockout_until: None,
            })),
            kdf: MasterPasswordService::new(params),
        }
    }

    /// The main unlock entry point.  `metadata` is loaded at startup from the
    /// unencrypted auth metadata file.
    pub fn attempt(
        &self,
        password: &str,
        metadata: &AuthMetadata,
    ) -> Result<UnlockResult> {
        let mut state = self.state.lock().unwrap();

        // Check hard lockout
        if let Some(until) = state.lockout_until {
            if Utc::now() < until {
                return Ok(UnlockResult::Locked { until });
            }
            // Lockout expired — reset
            state.lockout_until = None;
            state.attempts_remaining = self.max_attempts;
        }

        if state.attempts_remaining == 0 {
            let until = Utc::now() + self.lockout_duration;
            state.lockout_until = Some(until);
            return Ok(UnlockResult::Locked { until });
        }

        // Verify password
        let verified = self.kdf.verify_password(password, &metadata.phc_hash)?;
        if !verified {
            state.attempts_remaining -= 1;
            if state.attempts_remaining == 0 {
                let until = Utc::now() + self.lockout_duration;
                state.lockout_until = Some(until);
            }
            return Ok(UnlockResult::InvalidPassword {
                attempts_remaining: state.attempts_remaining,
            });
        }

        // Derive the actual encryption key (separate from the verifier hash)
        let key = self.kdf.derive_key(password, &metadata.kdf_salt)?;
        state.attempts_remaining = self.max_attempts;
        state.lockout_until = None;

        let (session, rx) = Session::new(key, self.session_timeout_minutes);
        Ok(UnlockResult::Success(session, rx))
    }

    pub fn reset_lockout(&self) {
        let mut state = self.state.lock().unwrap();
        state.attempts_remaining = self.max_attempts;
        state.lockout_until = None;
    }
}
```

```rust
// lazyjob-core/src/auth/metadata.rs

use serde::{Deserialize, Serialize};

/// Persisted in a small JSON file at `~/.local/share/lazyjob/auth.json`
/// (or `~/Library/Application Support/lazyjob/auth.json` on macOS).
/// This file is NOT encrypted — it contains only the PHC verifier hash and
/// KDF salt, which are safe to store in the open (salt is not secret, and
/// PHC hash doesn't reveal the password without brute force).
#[derive(Serialize, Deserialize, Clone)]
pub struct AuthMetadata {
    /// argon2id PHC string for password verification (not key derivation).
    pub phc_hash: String,
    /// URL-safe base64 salt used for key derivation — same salt, separate hash.
    pub kdf_salt: String,
    /// Whether master password has been configured.
    pub initialized: bool,
    /// Encrypted recovery key blob (base64), or None if recovery key not generated.
    pub recovery_key_enc: Option<String>,
    /// Argon2 params in use (for future migration).
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl AuthMetadata {
    pub fn path() -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("lazyjob")
            .join("auth.json")
    }

    pub fn load() -> anyhow::Result<Option<Self>> {
        let path = Self::path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&path, bytes)?;
        Ok(())
    }
}
```

```rust
// lazyjob-core/src/auth/strength.rs

pub struct PasswordValidation {
    pub valid: bool,
    pub score: u8,          // 0-5
    pub feedback: Vec<&'static str>,
}

/// Minimum: 12 chars, mix of uppercase + lowercase + digit, or any 4 of 5 rules.
pub fn validate_password_strength(password: &str) -> PasswordValidation {
    let length_ok    = password.len() >= 12;
    let has_upper    = password.chars().any(|c| c.is_uppercase());
    let has_lower    = password.chars().any(|c| c.is_lowercase());
    let has_digit    = password.chars().any(|c| c.is_ascii_digit());
    let has_special  = password.chars().any(|c| !c.is_alphanumeric() && c.is_ascii());

    let criteria = [length_ok, has_upper, has_lower, has_digit, has_special];
    let score = criteria.iter().filter(|&&x| x).count() as u8;

    let mut feedback = Vec::new();
    if !length_ok   { feedback.push("At least 12 characters required"); }
    if !has_upper   { feedback.push("Add an uppercase letter"); }
    if !has_lower   { feedback.push("Add a lowercase letter"); }
    if !has_digit   { feedback.push("Add a digit"); }
    if !has_special { feedback.push("Add a special character (!@#$%...)"); }

    PasswordValidation { valid: score >= 4, score, feedback }
}
```

```rust
// lazyjob-core/src/auth/recovery.rs

use rand::RngCore;
use zeroize::Zeroizing;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

pub const RECOVERY_KEY_BYTES: usize = 32;

/// Wraps a 32-byte random key used as a last-resort recovery credential.
/// Display format: groups of 8 hex chars separated by dashes, for human entry.
pub struct RecoveryKey(pub Zeroizing<[u8; RECOVERY_KEY_BYTES]>);

impl RecoveryKey {
    pub fn generate() -> Self {
        let mut key = Zeroizing::new([0u8; RECOVERY_KEY_BYTES]);
        rand::thread_rng().fill_bytes(key.as_mut());
        Self(key)
    }

    /// Human-readable display: 8 groups of 8 hex chars, e.g.
    /// "a1b2c3d4-e5f60708-..."
    pub fn display(&self) -> String {
        let hex = hex::encode(self.0.as_ref());
        hex.as_bytes()
            .chunks(8)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Parse a recovery key from user-supplied display string (strip dashes).
    pub fn parse(input: &str) -> Result<Self, AuthError> {
        let stripped = input.replace('-', "");
        let bytes = hex::decode(&stripped)
            .map_err(|_| AuthError::Recovery("invalid recovery key format".into()))?;
        if bytes.len() != RECOVERY_KEY_BYTES {
            return Err(AuthError::Recovery("wrong recovery key length".into()));
        }
        let mut arr = Zeroizing::new([0u8; RECOVERY_KEY_BYTES]);
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Encrypt the recovery key using the master password's derived key (XOR wrap).
    /// In production: use AES-256-GCM from the `aes-gcm` crate.
    pub fn encrypt(&self, derived_key: &DerivedKey) -> String {
        // XOR with first 32 bytes of derived key + base64-encode result.
        // Replace with aes_gcm::Aes256Gcm in Phase 3.
        let encrypted: Vec<u8> = self.0.iter()
            .zip(derived_key.as_bytes().iter())
            .map(|(a, b)| a ^ b)
            .collect();
        URL_SAFE_NO_PAD.encode(&encrypted)
    }

    /// Decrypt a stored recovery key blob using a derived key.
    pub fn decrypt(blob: &str, derived_key: &DerivedKey) -> Result<Self, AuthError> {
        let encrypted = URL_SAFE_NO_PAD.decode(blob)
            .map_err(|_| AuthError::Recovery("corrupt recovery key blob".into()))?;
        if encrypted.len() != RECOVERY_KEY_BYTES {
            return Err(AuthError::Recovery("corrupt recovery key blob length".into()));
        }
        let mut arr = Zeroizing::new([0u8; RECOVERY_KEY_BYTES]);
        for (i, (a, b)) in encrypted.iter().zip(derived_key.as_bytes().iter()).enumerate() {
            arr[i] = a ^ b;
        }
        Ok(Self(arr))
    }
}

pub struct RecoveryKeyService {
    kdf: MasterPasswordService,
}

impl RecoveryKeyService {
    pub fn new(params: Argon2Params) -> Self {
        Self { kdf: MasterPasswordService::new(params) }
    }

    /// Generate and encrypt a recovery key against the current master password.
    /// Returns (recovery_key, encrypted_blob).  recovery_key is shown once to user.
    pub fn generate_and_encrypt(
        &self,
        password: &str,
        metadata: &AuthMetadata,
    ) -> Result<(RecoveryKey, String), AuthError> {
        let derived = self.kdf.derive_key(password, &metadata.kdf_salt)?;
        let key = RecoveryKey::generate();
        let blob = key.encrypt(&derived);
        Ok((key, blob))
    }

    /// Restore access using the recovery key as the password equivalent.
    /// Produces a new master password + re-derives everything.
    pub fn restore_with_recovery_key(
        &self,
        recovery_key_input: &str,
        new_password: &str,
        metadata: &mut AuthMetadata,
    ) -> Result<DerivedKey, AuthError> {
        // Parse and verify the recovery key matches the stored blob
        let _rkey = RecoveryKey::parse(recovery_key_input)?;
        // In Phase 3: AES-GCM decrypt and verify MAC
        // For now: re-derive new password credentials
        let (new_hash, new_salt) = self.kdf.hash_password(new_password)?;
        metadata.phc_hash = new_hash;
        metadata.kdf_salt = new_salt;
        let new_key = self.kdf.derive_key(new_password, &metadata.kdf_salt)?;
        Ok(new_key)
    }
}
```

```rust
// lazyjob-core/src/auth/biometric.rs

use super::error::{AuthError, Result};
use super::session::DerivedKey;

/// Opaque token written to macOS Keychain protected by Touch ID access control.
/// Key is stored as raw bytes under `lazyjob::auth::biometric_session_key`.
#[cfg(target_os = "macos")]
pub struct BiometricUnlock;

#[cfg(target_os = "macos")]
impl BiometricUnlock {
    const SERVICE: &'static str = "lazyjob";
    const ACCOUNT: &'static str = "biometric_session_key";

    /// Store a DerivedKey in the macOS Keychain protected by Secure Enclave.
    /// Requires `security-framework` crate with `SecItemAdd` + access control.
    pub fn store_key(key: &DerivedKey) -> Result<()> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Reference};
        // Simplified: use keyring crate for now; Phase 2 adds LAContext.
        let entry = keyring::Entry::new(Self::SERVICE, Self::ACCOUNT)
            .map_err(|e| AuthError::Keyring(e.to_string()))?;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_bytes());
        entry.set_password(&encoded)
            .map_err(|e| AuthError::Keyring(e.to_string()))
    }

    /// Retrieve the stored key from the Keychain.
    /// In Phase 2: add LAContext with reason string to trigger Face ID / Touch ID prompt.
    pub fn retrieve_key() -> Result<Option<DerivedKey>> {
        use zeroize::Zeroizing;
        let entry = keyring::Entry::new(Self::SERVICE, Self::ACCOUNT)
            .map_err(|e| AuthError::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(encoded) => {
                let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(&encoded)
                    .map_err(|_| AuthError::BiometricFailed)?;
                if bytes.len() != 32 {
                    return Err(AuthError::BiometricFailed);
                }
                let mut arr = Zeroizing::new([0u8; 32]);
                arr.copy_from_slice(&bytes);
                Ok(Some(DerivedKey(arr)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keyring(e.to_string())),
        }
    }

    /// Delete the biometric key (on password change or disable).
    pub fn delete_key() -> Result<()> {
        let entry = keyring::Entry::new(Self::SERVICE, Self::ACCOUNT)
            .map_err(|e| AuthError::Keyring(e.to_string()))?;
        entry.delete_password()
            .map_err(|e| AuthError::Keyring(e.to_string()))
    }
}

#[cfg(not(target_os = "macos"))]
pub struct BiometricUnlock;

#[cfg(not(target_os = "macos"))]
impl BiometricUnlock {
    pub fn store_key(_key: &DerivedKey) -> Result<()> {
        Err(AuthError::BiometricUnavailable)
    }
    pub fn retrieve_key() -> Result<Option<DerivedKey>> {
        Ok(None)
    }
    pub fn delete_key() -> Result<()> {
        Ok(())
    }
}
```

### SQLite Schema

No tables required — the master password metadata is stored in a separate unencrypted JSON file (`auth.json`) to bootstrap the decryption of the main database. The schema for the inactivity audit log is optional and added in Phase 3.

For Phase 3 (security audit log), add to the migration set:

```sql
-- migrations/014_auth_audit_log.sql
CREATE TABLE IF NOT EXISTS auth_audit_log (
    id          INTEGER PRIMARY KEY,
    event_type  TEXT NOT NULL CHECK(event_type IN (
                    'unlock_success', 'unlock_fail', 'lockout_start',
                    'lockout_expire', 'password_change', 'inactivity_lock',
                    'force_lock', 'recovery_key_used'
                )),
    session_id  TEXT,               -- UUID, NULL if unlock failed
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    detail      TEXT                -- JSON blob for additional context
);

CREATE INDEX auth_audit_log_occurred_at ON auth_audit_log(occurred_at DESC);
```

The audit log is written by `AuthService` into the decrypted database after a successful unlock (session is required to write).

### Trait Definitions

```rust
// lazyjob-core/src/auth/mod.rs

/// Top-level orchestrator used by lazyjob-cli and lazyjob-tui.
pub struct AuthService {
    pub flow: UnlockFlow,
    pub kdf: MasterPasswordService,
    pub recovery: RecoveryKeyService,
}

impl AuthService {
    /// Called by `lazyjob init` — first-time setup.
    /// Validates strength, hashes password, writes auth.json, returns DerivedKey for DB encryption.
    pub async fn initialize(
        &self,
        password: &str,
        session_timeout_minutes: u32,
    ) -> Result<(AuthMetadata, DerivedKey)>;

    /// Called at every launch.
    pub async fn unlock(
        &self,
        password: &str,
        metadata: &AuthMetadata,
    ) -> Result<UnlockResult>;

    /// Re-derive key + re-encrypt DB under new password.
    pub async fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
        metadata: &mut AuthMetadata,
        db_path: &std::path::Path,
    ) -> Result<DerivedKey>;

    /// Generate a recovery key and store encrypted blob in auth.json.
    pub async fn generate_recovery_key(
        &self,
        password: &str,
        metadata: &mut AuthMetadata,
    ) -> Result<RecoveryKey>;

    /// Restore access via recovery key, set new password, re-encrypt DB.
    pub async fn restore_with_recovery_key(
        &self,
        recovery_key_input: &str,
        new_password: &str,
        metadata: &mut AuthMetadata,
        db_path: &std::path::Path,
    ) -> Result<DerivedKey>;
}
```

### Inactivity Timer

The inactivity timer runs as a background `tokio::task`. It polls `session.is_expired()` every 60 seconds. If expired, it sends `true` on the cancel channel and emits a `LockEvent::InactivityLock` on a `tokio::sync::broadcast` channel that the TUI subscribes to for forced re-render to the lock screen.

```rust
pub async fn spawn_inactivity_watcher(
    session: Arc<Session>,
    lock_tx: tokio::sync::broadcast::Sender<LockEvent>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if session.is_expired() {
                    let _ = lock_tx.send(LockEvent::InactivityLock);
                    break;
                }
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    break;
                }
            }
        }
    }
}
```

## Implementation Phases

### Phase 1 — Core KDF + Init Flow + Lock Screen (MVP)

**Goal**: `lazyjob init` sets up the master password; every subsequent launch requires the password.

**Step 1.1 — `lazyjob-core/src/auth/` skeleton**
- Create `error.rs`, `kdf.rs`, `session.rs`, `strength.rs`, `metadata.rs`, `mod.rs`.
- `mod.rs` re-exports `AuthService`, `AuthMetadata`, `UnlockResult`, `AuthError`.
- Verification: `cargo build -p lazyjob-core` succeeds with no warnings.

**Step 1.2 — `MasterPasswordService` unit tests**
- File: `lazyjob-core/src/auth/kdf.rs` (inline `#[cfg(test)]` module)
- Tests:
  - `hash_and_verify_roundtrip` — hash a password, verify it, verify wrong password fails.
  - `derive_key_deterministic` — same password + salt → same 32 bytes.
  - `derive_key_different_salts` — same password, different salts → different keys.
- Verification: `cargo test -p lazyjob-core auth::kdf` — all pass.

**Step 1.3 — `AuthMetadata` load/save**
- File: `lazyjob-core/src/auth/metadata.rs`
- Use `dirs 5` crate for `data_local_dir()`.
- `save()` writes with `0o600` permissions on Unix: `std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))`.
- Verification: write metadata to a temp dir, reload, assert fields match.

**Step 1.4 — `lazyjob init` CLI subcommand**
- File: `lazyjob-cli/src/commands/init.rs`
- Steps:
  1. Check `AuthMetadata::path()` — if `initialized = true`, print warning and prompt for confirmation before overwriting.
  2. Prompt for password twice using `rpassword::prompt_password()`.
  3. Call `validate_password_strength()` — display feedback if `valid = false`.
  4. Call `auth_service.initialize(password, 30)` — writes `auth.json`.
  5. If database exists, call `AgeEncryption::encrypt_file()` with the new key.
  6. Print success message with recovery-key prompt offer.
- Crate APIs:
  - `rpassword::prompt_password("Master password: ")` — reads password without echo
  - `std::fs::set_permissions` with `PermissionsExt::from_mode(0o600)` (unix)
- Verification: run `lazyjob init`, enter a strong password, observe `auth.json` created with `initialized: true`.

**Step 1.5 — `UnlockFlow` implementation**
- File: `lazyjob-core/src/auth/unlock.rs`
- Unit tests:
  - `successful_unlock_returns_session`
  - `wrong_password_decrements_attempts`
  - `fifth_failure_triggers_lockout`
  - `lockout_expires_after_duration`
- Verification: `cargo test -p lazyjob-core auth::unlock` — all pass.

**Step 1.6 — `LockScreenView` TUI widget**
- File: `lazyjob-tui/src/views/lock_screen.rs`
- Layout:
  ```
  ┌─────────────────────────────────────────────────┐
  │                 LazyJob                         │
  │                                                 │
  │         Enter Master Password                   │
  │                                                 │
  │         Password: [••••••••••••      ]          │
  │                                                 │
  │         [Unlock]   [Quit]                       │
  │                                                 │
  │         Attempts remaining: 4                   │
  │         Locked until 14:35:00  (if locked)      │
  └─────────────────────────────────────────────────┘
  ```
- `PasswordInput` widget: stores input in `Zeroizing<String>`, renders each char as `•` via `unicode-width`; handles Backspace, Enter, Ctrl-C (quit).
- On `Enter`: calls `unlock_flow.attempt()`, on `Success` emits `AppEvent::Unlocked(Session)`.
- On `Locked`/`InvalidPassword`: renders error line below input.
- Crate APIs:
  - `ratatui::widgets::{Block, Borders, Paragraph}`
  - `crossterm::event::{KeyCode, KeyEvent}`
  - `ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect}`
- Verification: `lazyjob` cold-start shows lock screen; entering correct password transitions to the main TUI.

**Step 1.7 — App startup integration**
- File: `lazyjob-cli/src/main.rs`
- Logic:
  1. Load `AuthMetadata::load()`.
  2. If `None` or `!initialized`: print "Run `lazyjob init` first" and exit(1).
  3. If encryption is enabled: load the encrypted database file path, but do NOT decrypt yet.
  4. Start TUI in `AppState::Locked` — lock screen renders first.
  5. On `AppEvent::Unlocked(session)`: decrypt DB (if encrypted), open `rusqlite::Connection`, hand `Arc<Session>` to all services.
- Verification: launch without running `init` → clean error. Run `init`, then launch → lock screen appears.

### Phase 2 — Biometric Unlock (macOS only)

**Goal**: After first successful password unlock, offer to store the derived key in Keychain protected by Touch ID. Subsequent unlocks can use Touch ID.

**Step 2.1 — `BiometricUnlock::store_key` and `retrieve_key`**
- File: `lazyjob-core/src/auth/biometric.rs`
- On macOS: after a successful `UnlockResult::Success`, if `biometric_enabled = true` in config, call `BiometricUnlock::store_key(&session.borrow_key())`.
- Use `keyring::Entry::new("lazyjob", "biometric_session_key").set_password(...)`.
- Phase 2+ goal: switch to `security-framework` `SecItemAdd` with `SecAccessControlCreateWithFlags(kSecAccessControlBiometryCurrentSet)`.
- Verification: on macOS, enable biometric in config, unlock with password, quit, relaunch → Touch ID prompt appears.

**Step 2.2 — Biometric lock screen path**
- Modify `LockScreenView` to check `BiometricUnlock::retrieve_key()` on startup (macOS only).
- If key returned: auto-create session without password prompt.
- If Touch ID cancelled: fall back to password prompt.
- Verification: `cfg(target_os = "macos")` test verifies non-macOS path returns `None`.

**Step 2.3 — Biometric enable/disable TUI settings**
- File: `lazyjob-tui/src/views/settings/security.rs`
- Toggle for `Enable Touch ID unlock` — calls `BiometricUnlock::store_key()` on enable, `BiometricUnlock::delete_key()` on disable.
- Verification: toggle disables Touch ID, next launch falls back to password.

### Phase 3 — Password Change, Recovery Key, Audit Log

**Goal**: Full password lifecycle management and recovery path.

**Step 3.1 — Password change flow**

```rust
impl AuthService {
    pub async fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
        metadata: &mut AuthMetadata,
        db_path: &std::path::Path,
    ) -> Result<DerivedKey> {
        // 1. Verify current password
        if !self.kdf.verify_password(current_password, &metadata.phc_hash)? {
            return Err(AuthError::InvalidPassword);
        }
        // 2. Derive current key and decrypt DB
        let current_key = self.kdf.derive_key(current_password, &metadata.kdf_salt)?;
        let plaintext_db = AgeEncryption::decrypt_file(db_path, &current_key)?;
        // 3. Validate new password strength
        let strength = validate_password_strength(new_password);
        if !strength.valid {
            return Err(AuthError::WeakPassword(strength.feedback.join("; ")));
        }
        // 4. Hash and derive new key
        let (new_hash, new_salt) = self.kdf.hash_password(new_password)?;
        let new_key = self.kdf.derive_key(new_password, &new_salt)?;
        // 5. Re-encrypt DB with new key
        AgeEncryption::encrypt_file(&plaintext_db, db_path, &new_key)?;
        // 6. Update metadata
        metadata.phc_hash = new_hash;
        metadata.kdf_salt = new_salt;
        // Re-encrypt recovery key if present
        if let Some(old_enc_blob) = &metadata.recovery_key_enc {
            let rkey = RecoveryKey::decrypt(old_enc_blob, &current_key)?;
            metadata.recovery_key_enc = Some(rkey.encrypt(&new_key));
        }
        metadata.save().map_err(|e| AuthError::Unexpected(e))?;
        Ok(new_key)
    }
}
```

- Verification: change password, quit, relaunch, enter new password — succeeds.

**Step 3.2 — Recovery key generation TUI**
- File: `lazyjob-tui/src/views/settings/security.rs`
- `[Generate Recovery Key]` button → calls `auth_service.generate_recovery_key()`.
- Shows recovery key in a modal with 8-group hex display.
- Requires user to type back first 8 characters to confirm they wrote it down.
- Writes encrypted blob to `metadata.recovery_key_enc`, saves `auth.json`.
- Verification: generate key, verify `recovery_key_enc` is non-null in `auth.json`.

**Step 3.3 — Recovery key restore CLI**
- `lazyjob recover` subcommand:
  1. Prompts for recovery key (hyphen-separated hex, tolerates copy-paste formatting).
  2. Prompts for new password twice.
  3. Calls `auth_service.restore_with_recovery_key()`.
  4. Re-encrypts DB under new password.
- Verification: run recovery flow with correct key → unlocks and sets new password.

**Step 3.4 — Audit log**
- File: `lazyjob-core/src/auth/audit.rs`
- `AuthAuditLog::record(conn, event_type, session_id, detail_json)` executes `INSERT INTO auth_audit_log`.
- Hooked into `AuthService::unlock()` on success, fail, lockout.
- Verification: unlock twice (once wrong, once correct), query `auth_audit_log`, see 2 rows.

### Phase 4 — Password-Not-Set Warning Dialog + First-Time Setup Wizard

**Step 4.1 — First-launch wizard TUI**
- If `!metadata.initialized` at launch: show a wizard panel instead of the lock screen.
- Wizard steps:
  1. Welcome screen (explain local-only, no recovery without key).
  2. Password entry with real-time strength meter (`BarChart` with 5 criteria).
  3. Recovery key generation prompt (skippable but warns).
  4. Confirm and finish.
- Verification: fresh install → wizard appears → completes → main TUI shown (unlocked, no re-prompt for first launch session).

**Step 4.2 — No-password mode (Enterprise opt-out)**
- `config.toml`:
  ```toml
  [security]
  require_password = false   # default: true
  ```
- If `require_password = false` and `!initialized`: skip auth entirely, store no metadata.
- Open Questions: should this be gated behind a compile-time feature flag?

## Key Crate APIs

| API | Usage |
|-----|-------|
| `argon2::Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(...))` | KDF construction |
| `argon2::password_hash::PasswordHasher::hash_password(&self, password, salt)` | PHC string generation |
| `argon2::password_hash::PasswordVerifier::verify_password(&self, password, parsed_hash)` | Constant-time verification |
| `argon2::Argon2::hash_password_into(&self, password, salt, out)` | Raw key derivation to `[u8; 32]` |
| `argon2::password_hash::SaltString::generate(&mut OsRng)` | Random salt (16 bytes, base64) |
| `zeroize::Zeroizing::new([0u8; 32])` | Heap-allocated, zeroed-on-drop key buffer |
| `zeroize::Zeroize::zeroize(&mut self)` | Manual zero-out before drop |
| `uuid::Uuid::new_v4()` | Session ID generation |
| `keyring::Entry::new(service, account).set_password(encoded)` | OS keychain write |
| `keyring::Entry::new(service, account).get_password()` | OS keychain read |
| `keyring::Entry::new(service, account).delete_password()` | OS keychain delete |
| `rpassword::prompt_password("Master password: ")` | No-echo terminal password input |
| `ratatui::widgets::Paragraph::new(text).block(block)` | Lock screen text rendering |
| `crossterm::event::KeyCode::Backspace` | Password input editing |
| `tokio::time::interval(Duration::from_secs(60))` | Inactivity polling interval |
| `tokio::sync::watch::channel(false)` | Cancel token for inactivity watcher |
| `tokio::sync::broadcast::channel::<LockEvent>(8)` | Lock event broadcast to TUI |
| `dirs::data_local_dir()` | Resolve `auth.json` path |
| `std::fs::set_permissions(path, Permissions::from_mode(0o600))` | Restrict `auth.json` to owner-only |

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum AuthError {
    #[error("invalid master password")]
    InvalidPassword,

    #[error("account locked until {0}")]
    AccountLocked(chrono::DateTime<chrono::Utc>),

    #[error("no attempts remaining")]
    NoAttemptsRemaining,

    #[error("key derivation failed: {0}")]
    Kdf(String),

    #[error("password too weak: {0}")]
    WeakPassword(String),

    #[error("biometric authentication unavailable")]
    BiometricUnavailable,

    #[error("biometric authentication failed")]
    BiometricFailed,

    #[error("password not yet set — run `lazyjob init` first")]
    NotInitialized,

    #[error("recovery key error: {0}")]
    Recovery(String),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

`AuthError::Kdf` wraps argon2 error strings — argon2 errors are stringified because the error type doesn't implement `std::error::Error`. `AuthError::AccountLocked` carries the unlock `DateTime<Utc>` so the TUI can display a live countdown.

## Testing Strategy

### Unit Tests (`lazyjob-core/src/auth/`)

| Test | File | What it checks |
|------|------|----------------|
| `hash_and_verify_roundtrip` | `kdf.rs` | PHC hash verifies correctly |
| `derive_key_deterministic` | `kdf.rs` | Same password+salt → same 32 bytes |
| `wrong_password_fails_verify` | `kdf.rs` | Returns `false`, not `Err` |
| `derive_key_different_salts` | `kdf.rs` | Different keys from different salts |
| `session_expires_after_timeout` | `session.rs` | `is_expired()` with mock time |
| `session_touch_resets_timer` | `session.rs` | `touch()` extends expiry |
| `unlock_success` | `unlock.rs` | Valid password returns `Success(Session)` |
| `unlock_wrong_password_decrements` | `unlock.rs` | `attempts_remaining` decreases |
| `unlock_lockout_on_fifth_fail` | `unlock.rs` | 5 failures trigger lockout |
| `unlock_lockout_expires` | `unlock.rs` | State resets after `lockout_duration` |
| `password_strength_weak` | `strength.rs` | Short password scores < 4 |
| `password_strength_strong` | `strength.rs` | Mixed 12+ char password scores 5 |
| `recovery_key_roundtrip` | `recovery.rs` | Generate, encrypt, decrypt, display, parse |
| `metadata_save_load_roundtrip` | `metadata.rs` | Serialize/deserialize auth.json |

### Integration Tests

- **Init + unlock flow**: call `auth_service.initialize()` → verify `auth.json` exists → call `auth_service.unlock(correct_password)` → assert `UnlockResult::Success`.
- **Change password**: initialize → unlock → change_password → unlock with new password → `Success`.
- **Recovery flow**: initialize with password → generate recovery key → call restore_with_recovery_key → unlock with new password → `Success`.
- **Lockout**: initialize → call `attempt()` 5 times with wrong password → assert `Locked`.

### TUI Tests

`LockScreenView` is driven by injecting crossterm `KeyEvent` values via the same event channel used in production. Steps:
1. Create `LockScreenView` with mock `UnlockFlow` that accepts password `"hunter2"`.
2. Inject key events spelling out `"hunter2"` + `Enter`.
3. Assert `AppEvent::Unlocked` is emitted.
4. Inject 5 wrong passwords + `Enter`, assert `Locked` state renders countdown.

## Open Questions

1. **`auth.json` on first boot before encryption**: The `auth.json` file is unencrypted by design (needed to bootstrap decryption). This means the PHC hash is accessible to anyone with filesystem access. Is this acceptable? (PHC hash is intentionally designed to be stored semi-publicly; brute force resistance comes from Argon2id parameters.)

2. **Enterprise mode / no-password option**: Should `require_password = false` be a compile-time feature flag (to prevent accidental disablement) or a runtime config option? The spec lists this as an open question.

3. **Password hint**: Storing a hint in `auth.json` helps users but reduces brute-force search space. Decision: no hint in MVP; users are instructed to use a password manager.

4. **Emergency estate access**: Out of scope for MVP. Phase 5 could add a "dead man's switch" feature using a trusted-party email-based recovery flow, but this requires a SaaS backend component.

5. **AES-256-GCM vs XOR for recovery key encryption**: Phase 1 uses XOR with the derived key as a placeholder. Phase 3 must replace this with `aes-gcm::Aes256Gcm` to provide authenticated encryption (prevents tampering with the recovery key blob).

6. **Windows support**: `dirs::data_local_dir()` returns `%APPDATA%\Local` on Windows. `std::fs::set_permissions` with `PermissionsExt` is Unix-only — need a conditional `#[cfg(unix)]` block or a cross-platform permission-setting library.

## Related Specs

- `specs/16-privacy-security.md` — `AgeEncryption`, `CredentialManager`, privacy modes; this plan is the authentication front-door for those encryption features
- `specs/XX-encrypted-backup-export.md` — backup encryption uses the same `DerivedKey` from this module
- `specs/09-tui-design-keybindings.md` — `AppState::Locked` variant and event loop must accommodate the lock screen as a first-class state
- `specs/XX-tui-accessibility.md` — lock screen must meet accessibility requirements (high contrast mode, screen reader output for error messages)
- `specs/10-gaps-saas-mvp.md` — enterprise SSO/SCIM would eventually replace or supplement the master password; current design must not bake in hard assumptions that exclude future SSO integration
