# Spec: Architecture — Config Management

**JTBD**: A fast, reliable tool that works offline; Keep my data private and portable
**Topic**: Define the configuration management system: TOML config file, API key storage, platform token management, and user preferences
**Domain**: architecture

---

## What

LazyJob's configuration lives in `~/.lazyjob/lazyjob.toml` (TOML format) and API keys/secrets live in the OS keyring. The `Config` struct is the single parsed representation of the TOML file, and all crates access configuration through typed accessors. Platform tokens (Greenhouse board tokens, Adzuna API keys, Apify API keys) are stored in the TOML file (not the keyring — they are not secrets in the same sense as LLM API keys) unless the user specifies keyring storage for sensitive ones.

## Why

A well-designed config system:
- **Discovery**: Users can read `lazyjob.toml` to understand all options
- **Versioning**: Config is a plain file — easy to backup, diff, and restore
- **No hardcoded values**: All thresholds, limits, and feature flags are configurable
- **Separation of secrets**: API keys go in keyring, everything else in TOML
- **Portable**: Copy `~/.lazyjob/` to a new machine, run LazyJob, it works

The alternative (environment variables) is harder to discover, harder to version-control, and harder to present in a TUI settings view. TOML is the right choice for a user-owned config file.

## How

### Config File Location

```
~/.lazyjob/
├── config.toml          # Main configuration (TOML)
├── life-sheet.yaml      # Life sheet (YAML, human-editable)
├── lazyjob.db           # SQLite database
├── .credentials         # Encrypted fallback credentials (if keyring fails)
└── backups/
```

### Config Schema

```toml
# ~/.lazyjob/lazyjob.toml

[general]
data_dir = "~/.lazyjob"        # Can be overridden
theme = "dark"                 # "dark" | "light"
polling_interval_minutes = 60  # Ralph discovery loop interval

[llm]
provider = "anthropic"         # "anthropic" | "openai" | "ollama"
model = "claude-3-5-sonnet-20241022"
max_tokens = 4096

[llm.fallback]
provider = "openai"
model = "gpt-4o"

[llm.ollama]
endpoint = "http://localhost:11434"
model = "llama3.2"

# API keys stored in OS keyring — reference by name in TOML
# lazyjob config key set llm.anthropic.api_key "sk-..."
# lazyjob config key set llm.openai.api_key "sk-..."

[platforms.greenhouse]
enabled = true
board_tokens = ["stripe", "notion", "figma", "linear"]

[platforms.lever]
enabled = true
company_ids = ["stripe", "notion"]

[platforms.adzuna]
enabled = false
app_id = ""       # Leave empty if not using
app_key = ""
country = "us"

[platforms.apify]
enabled = false
api_key = ""

[networking]
max_weekly_new_contacts = 5      # From networking-outreach-drafting.md
max_follow_up_reminders_per_contact = 2  # From networking-referral-management.md
referral_ghost_score_threshold = 0.6   # Block referral suggests if ghost_score > 0.6

[privacy]
mode = "full"                    # "full" | "minimal" | "stealth"
encrypt_database = false

[job_search]
min_match_score = 0.3            # Don't surface jobs below this score
max_ghost_score = 0.7            # Flag jobs above this ghost score
recency_decay_days = 14          # Halflife for job freshness in feed ranking

[ratelimit]
requests_per_minute = 30         # Shared across all platform clients
```

### Config Parsing

```rust
// lazyjob-core/src/config/mod.rs

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub llm: LlmConfig,
    pub platforms: PlatformsConfig,
    pub networking: NetworkingConfig,
    pub privacy: PrivacyConfig,
    pub job_search: JobSearchConfig,
    pub ratelimit: RateLimitConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    pub data_dir: PathBuf,
    pub theme: String,
    pub polling_interval_minutes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub fallback: Option<LlmFallbackConfig>,
    pub ollama: Option<OllamaConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformsConfig {
    pub greenhouse: GreenhouseConfig,
    pub lever: LeverConfig,
    pub adzuna: AdzunaConfig,
    pub apify: ApifyConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkingConfig {
    pub max_weekly_new_contacts: usize,
    pub max_follow_up_reminders_per_contact: usize,
    pub referral_ghost_score_threshold: f64,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(&path)?;
        let config: Config = toml_edit::from_str(&contents)?;
        Ok(config)
    }

    pub fn config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or(Error::Config("No home directory".into()))?;
        Ok(home.join(".lazyjob").join("config.toml"))
    }

    pub fn ensure_exists() -> Result<()> {
        let path = Self::config_path()?;
        if !path.exists() {
            let default = Self::default();
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, toml_edit::to_string(&default)?)?;
        }
        Ok(())
    }

    pub fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            general: GeneralConfig {
                data_dir: home.join(".lazyjob"),
                theme: "dark".to_string(),
                polling_interval_minutes: 60,
            },
            llm: LlmConfig {
                provider: "anthropic".to_string(),
                model: "claude-3-5-sonnet-20241022".to_string(),
                fallback: None,
                ollama: None,
            },
            platforms: PlatformsConfig {
                greenhouse: GreenhouseConfig { enabled: false, board_tokens: vec![] },
                lever: LeverConfig { enabled: false, company_ids: vec![] },
                adzuna: AdzunaConfig { enabled: false, app_id: String::new(), app_key: String::new(), country: "us".to_string() },
                apify: ApifyConfig { enabled: false, api_key: String::new() },
            },
            networking: NetworkingConfig {
                max_weekly_new_contacts: 5,
                max_follow_up_reminders_per_contact: 2,
                referral_ghost_score_threshold: 0.6,
            },
            privacy: PrivacyConfig {
                mode: "full".to_string(),
                encrypt_database: false,
            },
            job_search: JobSearchConfig {
                min_match_score: 0.3,
                max_ghost_score: 0.7,
                recency_decay_days: 14,
            },
            ratelimit: RateLimitConfig {
                requests_per_minute: 30,
            },
        }
    }
}
```

### API Key Storage: Keyring + TOML Reference

API keys are stored in the OS keyring, not in TOML. The TOML file contains a reference name that maps to the keyring entry:

```toml
[llm.anthropic]
api_key_ref = "anthropic"  # Maps to keyring entry "api_key:anthropic"

[llm.openai]
api_key_ref = "openai"      # Maps to keyring entry "api_key:openai"
```

```rust
// lazyjob-core/src/config/keys.rs

impl LlmConfig {
    pub async fn resolve_api_key(&self) -> Result<Option<String>> {
        match self.provider.as_str() {
            "anthropic" => {
                let key_ref = self.anthropic.as_ref()
                    .and_then(|c| c.api_key_ref.as_ref())
                    .unwrap_or(&"anthropic".to_string());
                get_api_key(key_ref).await
            }
            "openai" => {
                let key_ref = self.openai.as_ref()
                    .and_then(|c| c.api_key_ref.as_ref())
                    .unwrap_or(&"openai".to_string());
                get_api_key(key_ref).await
            }
            _ => Ok(None)
        }
    }
}
```

### TUI Config Editing

The TUI settings view (`lazyjob-tui/src/views/settings.rs`) provides a config editor:

```rust
// Settings view shows TOML config as structured form
// User edits values, presses Save → writes back to lazyjob.toml
// Sensitive values (API keys) show masked "●●●●●●●●" and Change button
// Change button → calls CredentialManager::store_api_key() → updates keyring
```

### Config Migration

When the config schema evolves, LazyJob must migrate old configs:

```rust
// lazyjob-core/src/config/migration.rs

impl Config {
    pub fn migrate_if_needed() -> Result<()> {
        let path = Self::config_path()?;
        if !path.exists() {
            Self::ensure_exists()?;
            return Ok(());
        }

        let current = Self::load()?;
        if current.general.config_version < CONFIG_VERSION {
            let migrated = Self::migrate(&current)?;
            std::fs::write(&path, toml_edit::to_string(&migrated)?)?;
        }
        Ok(())
    }

    fn migrate(current: &Config) -> Result<Config> {
        let mut next = current.clone();
        // Add new fields with defaults for old configs
        if next.platforms.adzuna.country.is_empty() {
            next.platforms.adzuna.country = "us".to_string();
        }
        next.general.config_version = CONFIG_VERSION;
        Ok(next)
    }
}
```

## Open Questions

- **Environment variable overrides**: Should `LAZYJOB_LLM_API_KEY` env vars override TOML/keyring for development? MVP: no env var overrides — keep it simple. Phase 2: document `LAZYJOB_DATA_DIR` override at minimum.
- **Config hot-reload**: Should changing `lazyjob.toml` while LazyJob is running cause a config reload without restart? Technically possible with file watcher. MVP: require restart. Phase 2: add `--reload-config` signal.
- **`data_dir` relocation**: Moving the data directory is a one-time setup operation. Should the TUI have a "relocate data" flow that moves the SQLite file and all backups? Phase 2.

## Implementation Tasks

- [ ] Define `Config` struct with all sections in `lazyjob-core/src/config/mod.rs` using `toml_edit` for serialization
- [ ] Implement `Config::load()`, `Config::ensure_exists()`, `Config::default()`, `Config::config_path()` for TOML file management
- [ ] Implement API key resolution via `CredentialManager` for `[llm.anthropic]`, `[llm.openai]` references in TOML
- [ ] Add `toml_edit` to `lazyjob-core` dependencies (for config read/write)
- [ ] Add `dirs` crate for `~/.lazyjob` path resolution
- [ ] Implement `[networking]` section: `max_weekly_new_contacts`, `max_follow_up_reminders_per_contact`, `referral_ghost_score_threshold` — wire these values into the networking and referral specs
- [ ] Add `config_version` field to `[general]` for schema migration
- [ ] Implement `Config::migrate_if_needed()` with version-based migration
- [ ] Write TUI settings view (`lazyjob-tui/src/views/settings.rs`) with form fields for all config sections, Save button writes back to TOML, API key Change button updates keyring
- [ ] Write CLI subcommand `lazyjob config get/set/list` for command-line config management
