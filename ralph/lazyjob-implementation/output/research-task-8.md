# Research: Task 8 — config-management

## Task Description
Implement Config struct in lazyjob-core/src/config.rs with TOML serialization. Reads from ~/.lazyjob/lazyjob.toml, creates defaults if missing. DATABASE_URL env var overrides config file.

## Key Findings

### Existing Patterns
- `lazyjob-core::db::DEFAULT_DATABASE_URL` = "postgresql://localhost/lazyjob" — Config should use this same default
- `CoreError` in error.rs has From impls for serde_json, serde_yaml, sqlx — need to add toml deserialization error
- CLI currently reads `--database-url` flag or env var, falls back to DEFAULT_DATABASE_URL — Config will sit between these layers
- `specs/rust-patterns.md` mentions "Layered configuration (config crate with TOML + env overrides)" as a future pattern

### New Dependency: toml
- Need `toml` crate for TOML ser/de. Version 0.8 is current stable.
- Uses serde Serialize/Deserialize — our types already derive these.

### Config Fields (from task description)
- `database_url`: String, default "postgresql://localhost/lazyjob"
- `life_sheet_path`: PathBuf, default "~/.lazyjob/life-sheet.yaml"
- `default_llm_provider`: Option<String>, default None
- `theme`: String, default "dark"
- `keybindings`: HashMap<String, String>, default empty (overrides)

### Config Resolution Order
1. Env var DATABASE_URL overrides config file's database_url
2. CLI --database-url flag overrides everything (handled in CLI, not Config)
3. Config file values are defaults
4. If no config file exists, create with defaults via ensure_exists()

### File Location
- Config dir: `~/.lazyjob/` (use dirs::home_dir() or std::env for HOME)
- Config file: `~/.lazyjob/lazyjob.toml`
- Need `dirs` crate for cross-platform home directory resolution

### Integration Points
- CLI main.rs will call Config::load() at startup
- Config.database_url replaces the hardcoded DEFAULT_DATABASE_URL fallback
- TUI will use Config for theme and keybinding overrides
- LLM registry will use Config.default_llm_provider
