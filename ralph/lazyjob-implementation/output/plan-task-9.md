# Plan: Task 9 — Credential Manager

## Files to Create/Modify

### New Files
- `lazyjob-core/src/credentials.rs` — CredentialManager struct with set/get/delete API key methods

### Modified Files
- `Cargo.toml` — add keyring, secrecy, zeroize to workspace deps
- `lazyjob-core/Cargo.toml` — add keyring, secrecy, zeroize to crate deps
- `lazyjob-core/src/error.rs` — add CoreError::Credential variant
- `lazyjob-core/src/lib.rs` — add `pub mod credentials`
- `lazyjob-cli/src/main.rs` — add Config subcommand with SetKey/GetKey

## Types/Functions/Structs

### `lazyjob-core/src/credentials.rs`
- `const SERVICE: &str = "lazyjob"` — keyring service namespace
- `pub struct CredentialManager` — unit struct wrapping keyring operations
- `CredentialManager::new() -> Self`
- `CredentialManager::set_api_key(&self, provider: &str, key: &SecretString) -> Result<()>`
- `CredentialManager::get_api_key(&self, provider: &str) -> Result<Option<SecretString>>`
- `CredentialManager::delete_api_key(&self, provider: &str) -> Result<()>`

### `lazyjob-core/src/error.rs`
- `CoreError::Credential(String)` — wraps keyring errors as strings

### `lazyjob-cli/src/main.rs`
- `Commands::Config(ConfigArgs)` — new top-level subcommand
- `ConfigCommand::SetKey { provider, key }` — store API key
- `ConfigCommand::GetKey { provider }` — retrieve API key (masked)
- `ConfigCommand::DeleteKey { provider }` — remove API key

## Tests

### Learning Tests
- `keyring_mock_round_trip` — proves keyring mock credential store works for set/get/delete
- `keyring_mock_no_entry` — proves NoEntry error is returned for missing keys
- `secrecy_expose_secret` — proves SecretString wraps and exposes correctly

### Unit Tests
- `set_and_get_api_key` — round-trip store and retrieve
- `get_missing_key_returns_none` — missing provider returns Ok(None)
- `delete_api_key` — store then delete then get returns None
- `delete_missing_key_is_ok` — deleting non-existent key doesn't error

### CLI Tests
- `parse_config_set_key` — clap parses `config set-key --provider anthropic --key sk-test`
- `parse_config_get_key` — clap parses `config get-key --provider anthropic`
- `parse_config_delete_key` — clap parses `config delete-key --provider anthropic`

## Migrations
None needed.
