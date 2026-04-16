# Spec: Master Password for Application Unlock

## Context

LazyJob stores sensitive data (resumes, cover letters, contacts, job applications, salary data). Without authentication, anyone with file system access can read all data. This spec addresses master password authentication on app launch.

## Motivation

- **Data protection**: Prevent unauthorized access to sensitive job search data
- **Device theft protection**: If laptop stolen, data not immediately accessible
- **Multi-user households**: Multiple users on same machine need separate data

## Design

### Password-Derived Encryption Key

```rust
pub struct MasterPasswordService {
    kdf: Argon2,
    salt_storage: SaltStorage,
}

impl MasterPasswordService {
    pub fn derive_key(&self, password: &str) -> Result<DerivedKey> {
        let salt = self.salt_storage.get_or_create_salt();

        let key = self.kdf
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| Error::KDFFailed)?;

        Ok(DerivedKey {
            key: key.as_bytes()[..32].to_vec(),  // 256-bit key
            salt,
        })
    }

    pub fn verify_password(&self, password: &str, stored_hash: &str) -> bool {
        // Use argon2 to verify (not store raw password)
        let key = self.derive_key(password).unwrap();
        // Compare hash
        verify_hash(&stored_hash, &key.to_bytes())
    }
}
```

### App Unlock Flow

```rust
pub enum AppState {
    Locked {
        attempts_remaining: u8,
        lockout_until: Option<DateTime<Utc>>,
    },
    Unlocked {
        session_id: Uuid,
        expires_at: DateTime<Utc>,
    },
}

pub struct UnlockFlow {
    max_attempts: u8 = 5,
    lockout_duration: Duration = Duration::minutes(5),
}

impl UnlockFlow {
    pub async fn handle_password_attempt(
        &self,
        password: &str,
        state: &AppState,
    ) -> Result<UnlockResult> {
        if let AppState::Locked { attempts_remaining, lockout_until } = state {
            if let Some(until) = lockout_until {
                if Utc::now() < *until {
                    return Err(UnlockError::AccountLocked(*until));
                }
            }
            if *attempts_remaining == 0 {
                return Err(UnlockError::NoAttemptsRemaining);
            }
        }

        let key = MASTER_PASSWORD_SERVICE.derive_key(password)?;
        let stored_hash = DATABASE.get_stored_hash()?;

        if MASTER_PASSWORD_SERVICE.verify_password(password, &stored_hash)? {
            // Success - create session
            let session = self.create_session(key).await?;
            Ok(UnlockResult::Success(session))
        } else {
            // Failed attempt
            self.record_failed_attempt(state).await?;
            Err(UnlockError::InvalidPassword)
        }
    }
}
```

### Session Management

```rust
pub struct Session {
    id: Uuid,
    encryption_key: [u8; 32],
    created_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    timeout_minutes: u32,
}

impl Session {
    pub fn is_expired(&self) -> bool {
        let elapsed = self.last_activity - self.created_at;
        elapsed > Duration::minutes(self.timeout_minutes as i64)
    }

    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }
}
```

Default session timeout: **30 minutes of inactivity**.

### Biometric Unlock (macOS)

```rust
pub struct BiometricUnlock {
    enabled: bool,
}

impl BiometricUnlock {
    pub async fn try_unlock(&self) -> Result<Option<EncryptionKey>> {
        #[cfg(target_os = "macos")]
        {
            use security_framework::SecureBuffer;

            let access = AccessControl::create()
                .with_reason("Unlock LazyJob")
                .build()?;

            let mut buffer = SecureBuffer::with_capacity(32);
            let result = buffer.unlock_with_access_control(&access);

            if result.is_ok() {
                return Ok(Some(buffer.to_vec()));
            }
        }

        Ok(None)  // Biometric not available
    }
}
```

**Requirements**:
- User must have set up master password first
- Key stored in macOS Keychain, protected by Touch ID
- Fallback to password if Touch ID fails

### Password Change

```rust
impl MasterPasswordService {
    pub async fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<()> {
        // 1. Verify current password
        let current_key = self.derive_key(current_password)?;
        if !self.verify(current_password)? {
            return Err(PasswordError::InvalidCurrent);
        }

        // 2. Re-encrypt database with new key
        let new_key = self.derive_key(new_password)?;
        self.database.rekey(&current_key, &new_key).await?;

        // 3. Update stored hash
        let new_hash = self.hash_password(new_password)?;
        self.database.update_password_hash(&new_hash)?;

        Ok(())
    }
}
```

### Password Recovery

**Critical constraint**: Local-only data = no recovery. This must be communicated clearly:

```
┌─────────────────────────────────────────────────────────────────┐
│  ⚠️  Password Recovery Not Possible                             │
│                                                                 │
│  LazyJob stores your data locally on this device.              │
│  If you forget your master password, your data cannot be        │
│  recovered.                                                     │
│                                                                 │
│  We strongly recommend:                                         │
│  • Use a password manager to store your master password        │
│  • Keep a physical backup of your recovery key                  │
│                                                                 │
│  [Create Recovery Key]  [I Understand, Continue]                │
└─────────────────────────────────────────────────────────────────┘
```

Recovery key is a random 256-bit key, encrypted with the password and stored separately.

### Password Strength Requirements

```rust
pub fn validate_password_strength(password: &str) -> PasswordValidation {
    let length_ok = password.len() >= 12;
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());

    let score = [length_ok, has_upper, has_lower, has_digit, has_special]
        .iter().filter(|&&x| x).count() as u8;

    PasswordValidation {
        valid: score >= 4,
        score,
        feedback: vec![
            if !length_ok { "Must be at least 12 characters" } else { "" },
            if !has_upper { "Add uppercase letter" } else { "" },
            // ...
        ].into_iter().filter(|s| !s.is_empty()).collect(),
    }
}
```

Minimum: 12 characters, mixed case, number, or special character.

## Implementation Notes

- **Argon2id** is the recommended KDF (RFC 9106, OWASP guidance) — hybrid approach provides resistance to both GPU cracking and side-channel attacks
- **Recommended parameters** (OWASP, balanced security): 19 MiB memory, 2 iterations, 1 parallelism
- **Alternative RFC 9106 profiles**:
  - Default: 2 GiB memory, 1 iteration, 4 parallelism (high-security server)
  - Memory-constrained: 64 MiB memory, 3 iterations, 4 parallelism (mobile/embedded)
- Salt stored in separate file from password hash
- Key never stored in plaintext, only in session memory
- Session stored in memory only (not disk)
- Lockout after 5 failed attempts, 5-minute cooldown

## Open Questions

1. **Enterprise mode**: Skip password for corporate-managed devices?
2. **Password hint**: Safe to allow hint without helping attackers?
3. **Emergency access**: For estate planning, allow trusted party access?

## Related Specs

- `16-privacy-security.md` - Encryption design
- `XX-encrypted-backup-export.md` - Backup encryption
- `XX-tui-accessibility.md` - Accessibility for lock screen