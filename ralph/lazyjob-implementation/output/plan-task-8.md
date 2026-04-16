# Plan: Task 8 — config-management

## Files to Create/Modify

### Create
- `lazyjob-core/src/config.rs` — Config struct and all logic

### Modify
- `Cargo.toml` — add `toml` and `dirs` to workspace deps
- `lazyjob-core/Cargo.toml` — add `toml` and `dirs` deps
- `lazyjob-core/src/lib.rs` — add `pub mod config`
- `lazyjob-core/src/error.rs` — add From<toml> error variants

## Types/Functions

### Config struct
```rust
pub struct Config {
    pub database_url: String,
    pub life_sheet_path: PathBuf,
    pub default_llm_provider: Option<String>,
    pub theme: String,
    pub keybindings: HashMap<String, String>,
}
```

### Functions
- `Config::default()` — sensible defaults
- `Config::load()` -> Result<Config> — reads ~/.lazyjob/lazyjob.toml, applies env overrides
- `Config::load_from(path)` -> Result<Config> — reads from specific path (for testing)
- `Config::save(&self)` -> Result<()> — writes to ~/.lazyjob/lazyjob.toml
- `Config::save_to(&self, path)` -> Result<()> — writes to specific path (for testing)
- `Config::ensure_exists()` -> Result<PathBuf> — creates config dir + default file if missing
- `Config::config_dir()` -> PathBuf — returns ~/.lazyjob/
- `Config::config_path()` -> PathBuf — returns ~/.lazyjob/lazyjob.toml
- `fn apply_env_overrides(&mut Config)` — DATABASE_URL env var override

## Tests

### Learning Tests
- `toml_serialize_round_trip` — proves toml crate can serialize/deserialize a Config-like struct
- `toml_optional_fields_default` — proves missing optional fields in TOML use serde defaults

### Unit Tests
- `default_config_has_expected_values` — verify defaults
- `load_from_file` — write a temp TOML, load it, verify fields
- `save_and_reload_round_trip` — save config, reload, compare
- `env_override_database_url` — set DATABASE_URL env, verify it overrides file value
- `ensure_exists_creates_file` — verify file creation in temp dir
- `partial_toml_uses_defaults` — TOML with only some fields still loads with defaults for rest

## No Migrations Needed
