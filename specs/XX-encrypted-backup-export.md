# Spec: Encrypted Backup and Export

## Context

LazyJob stores sensitive job search data (resumes, cover letters, contacts, job applications). Users need secure encrypted backups and data export. This spec addresses the encrypted backup format, key management, and secure export.

## Motivation

- **Data security**: Database may be encrypted but backups might not be
- **Data portability**: Users must be able to export their data
- **Key recovery**: Users who lose passwords need backup recovery options
- **Cloud backup safety**: Encrypted backups to cloud services must remain encrypted

## Design

### Backup Format

Backups are encrypted archives using `age` format:

```
lazyjob-backup-2024-04-15-001.blob
```

File structure (before encryption):
```
lazyjob-backup/
├── manifest.json          # Metadata (not encrypted)
├── database.sqlite        # Full SQLite database (encrypted)
├── attachments/           # Any binary attachments
│   ├── resume_001.docx
│   └── cover_letter_001.pdf
└── metadata.yaml          # Backup info, app version
```

**After age encryption**:
```
AGE encrypted blob with armor header
```

### Key Management

#### Backup Encryption Key Derivation

```rust
pub struct BackupKey {
    derived_from: KeyDerivation,
}

pub enum KeyDerivation {
    /// Key derived from user's master password (same as database)
    MasterPassword {
        salt: [u8; 16],
        argon2_params: Argon2Params,
    },
    /// Random key for cloud backup, user receives raw key file
    RandomKey,
}

impl BackupKey {
    pub fn from_master_password(password: &str, salt: [u8; 16]) -> Self {
        let key = argon2::Argon2::default()
            .hash_password_into(password.as_bytes(), &salt)
            .unwrap();
        Self { derived_from: KeyDerivation::MasterPassword { salt, argon2_params: DEFAULT_PARAMS } }
    }
}
```

#### Key Storage Options

1. **Password-derived (default)**: Same master password encrypts backup
2. **Random key file**: For cloud backup, generate random key, user stores it
3. **Hardware key**: YubiKey storage (future)

### Backup Creation

```rust
pub struct BackupService {
    db: Database,
    encryption: EncryptionService,
}

impl BackupService {
    pub async fn create_backup(&self, path: &Path) -> Result<BackupResult> {
        // 1. Create temp directory
        let temp_dir = tempfile::tempdir()?;

        // 2. Export database
        self.db.export_sqlite(temp_dir.path().join("database.sqlite")).await?;

        // 3. Copy attachments
        self.copy_attachments(temp_dir.path())?;

        // 4. Create manifest
        let manifest = self.create_manifest()?;
        std::fs::write(temp_dir.path().join("manifest.json"), manifest)?;

        // 5. Create metadata
        let metadata = self.create_metadata()?;
        std::fs::write(temp_dir.path().join("metadata.yaml"), metadata)?;

        // 6. Create tar archive
        let archive_path = temp_dir.path().join("backup.tar");
        self.create_tar(temp_dir.path(), &archive_path)?;

        // 7. Encrypt with age
        let encrypted_path = path;
        self.encrypt_archive(&archive_path, encrypted_path)?;

        Ok(BackupResult {
            path: encrypted_path.to_path_buf(),
            size_bytes: std::fs::metadata(encrypted_path)?.len(),
            checksum: sha256_file(encrypted_path)?,
        })
    }
}
```

### Backup Restoration

```rust
impl BackupService {
    pub async fn restore_backup(&self, path: &Path, password: &str) -> Result<RestoreResult> {
        // 1. Decrypt archive
        let temp_dir = tempfile::tempdir()?;
        self.decrypt_archive(path, temp_dir.path())?;

        // 2. Extract tar
        let extract_dir = temp_dir.path().join("backup");
        self.extract_tar(temp_dir.path().join("backup.tar"), &extract_dir)?;

        // 3. Verify manifest
        let manifest: Manifest = serde_json::from_str(
            &std::fs::read_to_string(extract_dir.join("manifest.json"))?
        )?;
        self.verify_manifest(&manifest)?;

        // 4. Stop current Ralph/TUI
        // ... (handled before calling restore)

        // 5. Replace database
        let db_path = self.db.path();
        backup_old_db(db_path)?;
        std::fs::copy(extract_dir.join("database.sqlite"), db_path)?;

        // 6. Restore attachments
        self.restore_attachments(&extract_dir)?;

        // 7. Verify database integrity
        self.db.verify_integrity()?;

        Ok(RestoreResult {
            apps_updated: manifest.app_count,
            attachments_restored: manifest.attachment_count,
        })
    }
}
```

### Encrypted Export

For data portability, export in decrypted form (user owns their data):

```rust
impl ExportService {
    pub async fn export_json(&self, path: &Path) -> Result<()> {
        let data = self.collect_all_data().await?;
        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(path, json)?;
    }

    pub async fn export_csv(&self, path: &Path) -> Result<()> {
        // Export tabular data (jobs, applications, contacts) as CSV
        let jobs = self.db.get_all_jobs().await?;
        let mut csv = String::new();
        // ... csv writing
        std::fs::write(path, csv)?;
    }
}
```

**Note**: JSON/CSV export is NOT encrypted - user explicitly has the key by having access to the file.

### Secure Deletion

When user deletes data, temp files must be securely wiped:

```rust
pub fn secure_delete(path: &Path) -> Result<()> {
    // Open file
    let file = std::fs::OpenOptions::new().write(true).open(path)?;

    // Get file size
    let metadata = file.metadata()?;
    let len = metadata.len();

    // Overwrite with zeros
    file.write_all(&vec![0u8; len as usize])?;
    file.sync_all()?;

    // Delete file
    std::fs::remove_file(path)?;
    Ok(())
}

impl BackupService {
    async fn cleanup_temp_files(&self, temp_dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(temp_dir)? {
            let entry = entry?;
            secure_delete(&entry.path())?;
        }
        std::fs::remove_dir(temp_dir)?;
        Ok(())
    }
}
```

### Cloud Backup Integration

For users syncing to Google Drive/Dropbox:

```rust
pub enum CloudBackupProvider {
    GoogleDrive,
    Dropbox,
    NextCloud,
}

pub struct CloudBackupConfig {
    pub provider: CloudBackupProvider,
    pub encrypted: bool,           // Always true for security
    pub auto_backup: bool,
    pub backup_frequency_hours: u32,
}

impl BackupService {
    pub async fn sync_to_cloud(&self, config: &CloudBackupConfig) -> Result<()> {
        // Create local encrypted backup
        let temp_path = tempfile::NamedTempFile::new()?;
        let result = self.create_backup(&temp_path).await?;

        // Upload to cloud (encrypted blob)
        match config.provider {
            CloudBackupProvider::GoogleDrive => {
                self.upload_google_drive(&temp_path, "lazyjob-backup").await?;
            }
            // ...
        }

        // Notify user
        self.notify_backup_complete(result).await?;

        Ok(())
    }
}
```

## Implementation Notes

- **Age encryption** (Rust-native, well-audited) — recommended over age for better Rust integration
- Backups are not compressed (age handles that internally)
- **Age best practices** from official spec:
  - Encrypt to multiple recipients by repeating `-r` flag
  - Use post-quantum hybrid keys with `-pq` flag for quantum-resistant encryption
  - Protect key files with passphrases using `-p` flag
  - Generate keys with `age-keygen -o key.txt`
  - Use `age-inspect` to verify encrypted file metadata without decryption
- **Supported age key types**: age public keys, SSH public keys (ssh-ed25519, ssh-rsa), and hardware tokens like YubiKeys
- Temp files created in RAM-backed filesystem if available
- Backup verification includes SQLite integrity check
- For maximum security, use post-quantum hybrid encryption for cloud backups

## Open Questions

1. **Key escrow**: Should we offer "recovery key" stored separately?
2. **Incremental backup**: Only backup changes (future optimization)
3. **Backup retention**: How many old backups to keep?

## Related Specs

- `16-privacy-security.md` - Encryption design
- `XX-master-password-app-unlock.md` - Master password
- `XX-data-portability-export.md` - Data export