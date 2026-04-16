# Research: Task 9 — Credential Manager

## Task Summary
Implement `CredentialManager` wrapping the `keyring` crate for secure API key storage in the OS keychain. Add CLI subcommands `config set-key` and `config get-key`.

## Key Findings

### keyring crate (v3)
- `Entry::new(service, username)` creates a keyring entry handle
- `entry.set_password(password: &str)` stores a credential
- `entry.get_password()` retrieves a credential as String
- `entry.delete_credential()` removes a credential
- `keyring::Error::NoEntry` for "not found" — used to distinguish missing from real errors
- **Mock support**: `keyring::set_default_credential_builder(keyring::mock::default_credential_builder())` enables in-memory mock for tests
- On Linux without a keyring daemon, falls back to file-based storage

### secrecy crate (v0.8)
- `Secret<String>` (aliased as `SecretString`) wraps sensitive strings
- `ExposeSecret::expose_secret(&self)` provides read access
- `Secret::new(value)` wraps a value
- Implements `Zeroize` on drop automatically

### zeroize crate (v1)
- `Zeroize` trait and derive macro for zeroing memory on drop
- `Zeroizing<T>` wrapper that calls zeroize on drop
- Used for temporary buffers holding sensitive data

### Existing Codebase Context
- `CoreError` in `lazyjob-core/src/error.rs` — needs a `Credential` variant
- `lazyjob-core/src/lib.rs` — needs `pub mod credentials`
- CLI in `lazyjob-cli/src/main.rs` — needs `Config` subcommand with `SetKey`/`GetKey`
- Workspace Cargo.toml — needs keyring, secrecy, zeroize added

### Design Decisions
1. **Flat module, not nested**: Place in `lazyjob-core/src/credentials.rs` (not `security/`) — the full security module is a future task
2. **Service name**: Use `"lazyjob"` as the keyring service name consistently
3. **Key naming**: Store as `api_key:{provider}` in the keyring (e.g., `api_key:anthropic`)
4. **Mock in tests**: Use keyring's built-in mock credential builder for all unit tests
5. **No blob storage yet**: Task only requires API key methods, not blob storage (that's for encryption task later)
