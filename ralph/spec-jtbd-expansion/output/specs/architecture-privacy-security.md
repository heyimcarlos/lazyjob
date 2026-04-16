# Spec: Architecture — Privacy & Security

**JTBD**: Keep my data private and portable
**Topic**: Define the privacy and security layer: OS keyring for API keys, optional SQLite encryption with age, data export/import, and privacy mode
**Domain**: architecture

---

## What

LazyJob stores sensitive job search data: target companies, salary expectations, career moves, API keys for LLM providers. This spec defines how that data is protected at rest (keyring, encryption), in transit (TLS), and in terms of user control (export, privacy mode). The core principle: **no telemetry, no cloud sync without user consent, user owns their data**.

## Why

Job search data is sensitive. Users are applying to jobs at competitors while currently employed. Their search history, salary expectations, and application activity could be damaging if exposed. LazyJob's local-first architecture means data never leaves the user's machine unless the user explicitly exports or (in the future) opts into cloud sync.

The key architectural decisions:
- API keys live in the OS keyring, not in `lazyjob.toml` or source code
- SQLite database is encrypted at rest with a user-provided key (optional, off by default)
- Data export always produces decrypted output — user can always read their data
- Privacy mode disables all network I/O for air-gapped operation

## How

### Credential Storage: OS Keyring

```rust
// lazyjob-core/src/security/credentials.rs

use keyring::Entry;

pub struct CredentialManager {
    service: String,
}

impl CredentialManager {
    pub fn new(service: &str) -> Self {
        Self { service: service.to_string() }
    }

    pub fn store_api_key(&self, provider: &str, key: &str) -> Result<()> {
        let entry = Entry::new(&self.service, &format!("api_key:{}", provider))
            .map_err(|e| Error::Keyring(e.to_string()))?;
        entry.set_password(key)
            .map_err(|e| Error::Keyring(e.to_string()))
    }

    pub fn get_api_key(&self, provider: &str) -> Result<Option<String>> {
        let entry = Entry::new(&self.service, &format!("api_key:{}", provider))
            .map_err(|e| Error::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }

    pub fn delete_api_key(&self, provider: &str) -> Result<()> {
        let entry = Entry::new(&self.service, &format!("api_key:{}", provider))
            .map_err(|e| Error::Keyring(e.to_string()))?;
        entry.delete_credential()
            .map_err(|e| Error::Keyring(e.to_string()))
    }
}
```

**Keyring targets:**
- Linux: `libsecret` via `secret-service` (keyring-rs handles this)
- macOS: Keychain
- Windows: Credential Manager

**Fallback**: If keyring access fails, fall back to an encrypted file in `~/.lazyjob/.credentials` (age-encrypted).

### Database Encryption: age

Database encryption is optional and off by default. When enabled, the entire SQLite file is encrypted at rest using the `age` crate with a user-provided passphrase. This is file-level encryption, not column-level.

```rust
// lazyjob-core/src/security/database.rs

use age::{Encryptor, Decryptor, Identity};

pub struct FileEncryption {
    identity: Option<Identity>,
    passphrase: Option<String>,
}

impl FileEncryption {
    pub fn with_identity(identity: Identity) -> Self {
        Self { identity: Some(identity), passphrase: None }
    }

    pub fn with_passphrase(passphrase: &str) -> Self {
        Self { identity: None, passphrase: Some(passphrase.to_string()) }
    }

    pub fn encrypt_file(&self, input: &Path, output: &Path) -> Result<()> {
        let file = std::fs::File::open(input)?;
        let mut encrypted = std::fs::File::create(output)?;
        let encryptor = match &self.identity {
            Some(id) => Encryptor::with_identities([id].into_iter()),
            None => {
                let pw = self.passphrase.as_ref().unwrap();
                Encryptor::with_passphrase(pw.as_bytes().into())
            }
        };
        let mut writer = encryptor.wrap_output(&mut encrypted)?;
        std::io::copy(&mut std::io::BufReader::new(file), &mut writer)?;
        writer.finish()?;
        Ok(())
    }

    pub fn decrypt_file(&self, input: &Path, output: &Path) -> Result<()> {
        let file = std::fs::File::open(input)?;
        let mut decrypted = std::fs::File::create(output)?;
        let decryptor = Decryptor::new_autodetect(std::io::BufReader::new(file))?;
        match decryptor {
            Decryptor::WithIdentity(id) => {
                let id = self.identity.as_ref().ok_or_else(|| Error::Encryption("No identity for decryption".into()))?;
                let mut reader = id.decrypt(id, None)?;
                std::io::copy(&mut reader, &mut decrypted)?;
            }
            Decryptor::WithPassphrase(pw) => {
                let pw = self.passphrase.as_ref().ok_or_else(|| Error::Encryption("No passphrase for decryption".into()))?;
                let mut reader = pw.decrypt(pw, None)?;
                std::io::copy(&mut reader, &mut decrypted)?;
            }
        }
        Ok(())
    }
}
```

**Encryption workflow:**
1. User sets encryption key: `lazyjob config set encryption.key`
2. On next startup, LazyJob prompts for passphrase (or reads from `LAZYJOB_ENCRYPTION_KEY` env var)
3. Database is encrypted to `lazyjob.db.age` on every write (via SQLITE page hooks or backup-to-encrypted)
4. On startup, passphrase unlocks the `.age` file, decrypted DB is opened

**Note**: For MVP, skip full file encryption. Add a note that `age` encryption can be enabled post-MVP. The keyring for API keys is the immediate requirement.

### Privacy Mode

```rust
// lazyjob-core/src/security/privacy.rs

pub enum PrivacyMode {
    Full,       // All features enabled, network I/O allowed
    Minimal,    // No LLM calls (use cached responses), no platform API calls
    Stealth,    // No SQLite persistence, all in-memory only (data lost on quit)
}

pub struct PrivacySettings {
    pub mode: PrivacyMode,
    pub store_api_keys: bool,      // Store in keyring (default: true)
    pub encrypt_database: bool,    // age encryption (default: false)
    pub allow_analytics: bool,    // No telemetry by default (default: false)
    pub cloud_sync: bool,         // Not implemented yet (default: false)
}

impl PrivacySettings {
    pub fn from_config(config: &Config) -> Self {
        Self {
            mode: config.privacy.mode,
            store_api_keys: config.privacy.store_api_keys.unwrap_or(true),
            encrypt_database: config.privacy.encrypt_database.unwrap_or(false),
            allow_analytics: false,
            cloud_sync: false,
        }
    }
}
```

**`PrivacyMode::Minimal`** disables:
- LLM calls (shows cached results if available)
- Platform API calls (shows error "Enable network I/O to use this feature")
- Ralph loops that require network access

**`PrivacyMode::Stealth`** is for users who want zero persistence:
- All data in memory only
- `export_all()` still works (exports from memory)
- No SQLite file written

### Data Export

```rust
// lazyjob-core/src/security/export.rs

pub async fn export_all(db: &Database, path: &Path) -> Result<ExportReport> {
    // Export is ALWAYS decrypted
    // Even when database encryption is enabled, export produces plaintext JSON
    let export = Export {
        version: "1.0".to_string(),
        exported_at: Utc::now(),
        jobs: db.jobs.list().await?,
        applications: db.applications.list().await?,
        profile_contacts: db.profile_contacts.list().await?,
        companies: db.companies.list().await?,
        life_sheet: db.life_sheet.get().await?,
        interviews: db.interviews.list().await?,
        offers: db.offers.list().await?,
    };
    let json = serde_json::to_string_pretty(&export)
        .context("Failed to serialize export")?;
    tokio::fs::write(path, json).await?;
    Ok(ExportReport { path: path.to_path_buf(), record_counts: export.counts(), exported_at: Utc::now() })
}

pub async fn import_all(db: &Database, path: &Path) -> Result<ImportReport> {
    let json = tokio::fs::read_to_string(path).await?;
    let import: Export = serde_json::from_str(&json)?;
    let mut counts = ImportCounts::default();
    for job in import.jobs { db.jobs.insert(&job).await?; counts.jobs += 1; }
    // ... etc
    Ok(ImportReport { counts, warnings: vec![] })
}
```

### Never-Sync Tables

The following tables contain data that should NEVER be synced to cloud in the SaaS migration:

```rust
// lazyjob-core/src/security/never_sync.rs

pub const NEVER_SYNC_TABLES: &[&str] = &[
    "offer_details",       // May violate offer letter confidentiality
    "token_usage_log",    // Per-user billing data (sync aggregated, not raw)
];
```

## Open Questions

- **Master password**: The spec-inventory notes `16-privacy-security.md` includes a "master password" question. Should LazyJob require a password on startup that unlocks the database? This adds friction but protects against shoulder-surfing. MVP: no master password (local machine security is the boundary). Phase 2: optional master password.
- **Cloud sync**: The future SaaS phase must handle encrypted sync (Supabase with client-side encryption). The Repository trait design should account for this — sync logic must be separate from persistence logic. The spec-inventory notes this in `architecture-config-management.md`.

## Implementation Tasks

- [ ] Implement `CredentialManager` in `lazyjob-core/src/security/credentials.rs` with keyring integration and encrypted-file fallback
- [ ] Add `SECRET_SERVICE` feature flag to `keyring` crate for Linux `libsecret` support
- [ ] Implement `FileEncryption` in `lazyjob-core/src/security/database.rs` with `encrypt_file`/`decrypt_file` for age encryption
- [ ] Add `[privacy]` section to `lazyjob.toml`: `mode = "full" | "minimal" | "stealth"`, `encrypt_database = false`
- [ ] Implement `PrivacySettings::from_config()` to gate LLM calls, platform API calls, and Ralph loops based on privacy mode
- [ ] Implement `export_all()` and `import_all()` in `lazyjob-core/src/security/export.rs`
- [ ] Add `NEVER_SYNC_TABLES` constant, integrate with SaaS migration spec
- [ ] Write integration test: export → delete → import → verify counts match (roundtrip test)
