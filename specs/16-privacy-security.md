# Privacy & Security

## Status
Researching

## Problem Statement

LazyJob stores sensitive personal and professional data:
1. **Personal Info**: Name, email, phone, address
2. **Career Data**: Work history, salary expectations, resume content
3. **Credentials**: API keys for LLM providers
4. **Job Search Activity**: Which companies, what positions

This data must be:
1. **Encrypted at rest**: SQLite database encrypted
2. **Secured in transit**: API calls over HTTPS
3. **Protected from access**: Keychain/credential store for secrets
4. **Exportable**: User owns their data

---

## Research Findings

### Encryption Approaches

**SQLite Encryption Options**:

1. **SQLite Encryption Extension (SEE)**: Proprietary, requires license
2. **SQLCipher**: OpenSSL-based, requires compilation
3. **wxSQLite3**: Wraps SEE
4. **age Encryption**: File-level encryption (post-write)

**Rust Encryption Crates**:
- `age` - Modern, simple file encryption
- `ring` - Low-level crypto primitives
- `rusqlite` with `bundled` feature uses system SQLite

### Keyring Integration

**Linux**: `libsecret` via `secret-service`
**macOS**: Keychain via `security` CLI
**Windows**: Credential Manager

**Rust Crates**:
- `keyring-rs` - Cross-platform keyring
- `secret-service` - Linux secret-service
- `macos-keychain` - macOS Keychain

### Secure Credential Storage

```rust
use keyring::Entry;

pub struct SecureStorage {
    service: String,
}

impl SecureStorage {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry = Entry::new(&self.service, key)
            .map_err(|e| Error::Keyring(e.to_string()))?;

        entry.set_password(value)
            .map_err(|e| Error::Keyring(e.to_string()))
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = Entry::new(&self.service, key)
            .map_err(|e| Error::Keyring(e.to_string()))?;

        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        let entry = Entry::new(&self.service, key)
            .map_err(|e| Error::Keyring(e.to_string()))?;

        entry.delete_credential()
            .map_err(|e| Error::Keyring(e.to_string()))
    }
}
```

### age Encryption

For file-level encryption:

```rust
use age::{Encryptor, Decryptor, Identity};
use std::io::{Read, Write};

pub struct FileEncryption {
    identity: Identity,
}

impl FileEncryption {
    pub fn generate_identity() -> Result<(Identity, String)> {
        let identity = age::Identity::generate();
        let public_key = identity.to_public();
        Ok((identity, public_key.to_string()))
    }

    pub fn encrypt_file(
        &self,
        input: &Path,
        output: &Path,
        recipients: &[String],
    ) -> Result<()> {
        let file = std::fs::File::open(input)?;
        let mut encrypted = std::fs::File::create(output)?;

        let encryptor = Encryptor::with_recipients(
            recipients.iter().map(|r| r.parse().unwrap()).collect::<Vec<_>>(),
        );

        let mut writer = encryptor.wrap_output(&mut encrypted)?;
        std::io::copy(&mut std::io::BufReader::new(file), &mut writer)?;
        writer.finish()?;

        Ok(())
    }

    pub fn decrypt_file(
        &self,
        input: &Path,
        output: &Path,
    ) -> Result<()> {
        let file = std::fs::File::open(input)?;
        let mut decrypted = std::fs::File::create(output)?;

        let decryptor = Decryptor::new_autodetect(&mut std::io::BufReader::new(file))?;

        if let Some(identity_key) = decryptor.into_identity_decryptor() {
            let mut reader = identity_key.decrypt(&self.identity, None)?;
            std::io::copy(&mut reader, &mut decrypted)?;
        }

        Ok(())
    }
}
```

### 1Password/Bitwarden Patterns

**Local-first apps typically**:
1. Store master password locally (never transmitted)
2. Use derived key for local encryption
3. Optionally sync encrypted vault to cloud

**For LazyJob**:
- API keys stored in system keyring
- SQLite database optionally encrypted with age
- Export always decrypts first

---

## Design

### Credential Management

```rust
// lazyjob-core/src/security/credentials.rs

pub struct CredentialManager {
    keyring: SecureStorage,
}

impl CredentialManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            keyring: SecureStorage::new("lazyjob")?,
        })
    }

    pub fn store_api_key(&self, provider: &str, key: &str) -> Result<()> {
        self.keyring.set(&format!("api_key:{}", provider), key)
    }

    pub fn get_api_key(&self, provider: &str) -> Result<Option<String>> {
        self.keyring.get(&format!("api_key:{}", provider))
    }

    pub fn delete_api_key(&self, provider: &str) -> Result<()> {
        self.keyring.delete(&format!("api_key:{}", provider))
    }
}
```

### Database Encryption

```rust
// lazyjob-core/src/security/database.rs

pub struct EncryptedDatabase {
    pool: SqlitePool,
    encryption: Option<FileEncryption>,
}

impl EncryptedDatabase {
    pub async fn open(
        path: &Path,
        encryption_key: Option<&str>,
    ) -> Result<Self> {
        let pool = SqlitePool::connect(&format!(
            "sqlite:{}?mode=rwc",
            path.display()
        )).await?;

        let encryption = if let Some(key) = encryption_key {
            Some(FileEncryption::new(key)?)
        } else {
            None
        };

        Ok(Self { pool, encryption })
    }

    pub async fn backup(&self, dest: &Path) -> Result<()> {
        // Decrypt to temp, copy, re-encrypt if needed
        // For now, unencrypted backup
        let backup = backup::Backup::new(&self.pool, dest)?;
        backup.run_to_completion(5, Duration::from_millis(250), None)?;
        Ok(())
    }
}
```

### Data Export (Always Decrypted)

```rust
impl Database {
    pub async fn export_all(&self, path: &Path) -> Result<ExportReport> {
        // Export is always decrypted - user has full access
        let export = Export {
            version: "1.0".to_string(),
            exported_at: Utc::now(),
            jobs: self.jobs.list().await?,
            applications: self.applications.list().await?,
            contacts: self.contacts.list().await?,
            life_sheet: self.life_sheet.get().await?,
            settings: self.settings.get().await?,
        };

        let json = serde_json::to_string_pretty(&export)
            .context("Failed to serialize export")?;

        tokio::fs::write(path, json).await?;

        Ok(ExportReport {
            path: path.to_path_buf(),
            record_counts: export.record_counts(),
            exported_at: Utc::now(),
        })
    }

    pub async fn import(&self, path: &Path) -> Result<ImportReport> {
        let json = tokio::fs::read_to_string(path).await?;
        let import: Export = serde_json::from_str(&json)?;

        let mut counts = ImportCounts::default();

        for job in import.jobs {
            self.jobs.insert(&job).await?;
            counts.jobs += 1;
        }

        for application in import.applications {
            self.applications.insert(&application).await?;
            counts.applications += 1;
        }

        // ... etc

        Ok(ImportReport { counts, warnings: vec![] })
    }
}
```

### Privacy Modes

```rust
pub enum PrivacyMode {
    Full,        // All features, stores everything
    Minimal,    // No API calls, no cloud sync
    Stealth,    // No local persistence, all in memory
}

pub struct PrivacySettings {
    pub mode: PrivacyMode,
    pub store_api_keys: bool,
    pub encrypt_database: bool,
    pub allow_analytics: bool,
    pub cloud_sync: bool,
}
```

---

## Failure Modes

1. **Keyring Access Denied**: Fall back to encrypted file storage
2. **Lost Encryption Key**: Data is unrecoverable - warn user clearly
3. **Memory Exposure**: Sensitive data in memory could be swapped to disk - minimize sensitive data in memory
4. **Export Leak**: When exporting, ensure temp files are securely deleted

---

## Open Questions

1. **Cloud Sync**: Should LazyJob eventually sync encrypted data to cloud?
2. **Master Password**: Should we require a master password for the app?
3. **API Key Sharing**: How to share credentials between devices?

---

## Dependencies

```toml
[dependencies]
keyring = "3"           # Cross-platform keyring
age = "0.9"             # File encryption
rusqlite = { version = "0.32", features = ["backup"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }
thiserror = "2"
anyhow = "1"
```

---

## Sources

- [keyring-rs Documentation](https://docs.rs/keyring/latest/keyring/)
- [age Encryption](https://age-encryption.org/)
- [SQLite Encryption Options](https://www.sqlite.org/see/doc/release/www/index.wiki)
