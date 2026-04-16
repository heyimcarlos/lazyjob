use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::DEFAULT_DATABASE_URL;
use crate::error::Result;

const CONFIG_DIR_NAME: &str = ".lazyjob";
const CONFIG_FILE_NAME: &str = "lazyjob.toml";
const DEFAULT_LIFE_SHEET_FILE: &str = "life-sheet.yaml";
const DEFAULT_THEME: &str = "dark";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_database_url")]
    pub database_url: String,

    #[serde(default = "default_life_sheet_path")]
    pub life_sheet_path: PathBuf,

    #[serde(default)]
    pub default_llm_provider: Option<String>,

    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default)]
    pub keybindings: HashMap<String, String>,
}

fn default_database_url() -> String {
    DEFAULT_DATABASE_URL.to_string()
}

fn default_life_sheet_path() -> PathBuf {
    config_dir().join(DEFAULT_LIFE_SHEET_FILE)
}

fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            life_sheet_path: default_life_sheet_path(),
            default_llm_provider: None,
            theme: default_theme(),
            keybindings: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if path.exists() {
            let mut config = Self::load_from(&path)?;
            apply_env_overrides(&mut config);
            Ok(config)
        } else {
            let mut config = Self::default();
            apply_env_overrides(&mut config);
            Ok(config)
        }
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    pub fn ensure_exists() -> Result<PathBuf> {
        let path = config_path();
        if !path.exists() {
            let config = Config::default();
            config.save_to(&path)?;
        }
        Ok(path)
    }
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR_NAME)
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE_NAME)
}

fn apply_env_overrides(config: &mut Config) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        config.database_url = url;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // learning test: proves toml crate can serialize and deserialize a struct with serde defaults
    #[test]
    fn toml_serialize_round_trip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Sample {
            name: String,
            count: u32,
            tags: Vec<String>,
        }

        let original = Sample {
            name: "test".into(),
            count: 42,
            tags: vec!["a".into(), "b".into()],
        };

        let serialized = toml::to_string_pretty(&original).unwrap();
        let deserialized: Sample = toml::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    // learning test: proves missing optional/defaulted fields in TOML use serde defaults
    #[test]
    fn toml_optional_fields_default() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Partial {
            required: String,
            #[serde(default)]
            optional: Option<String>,
            #[serde(default)]
            items: Vec<String>,
        }

        let input = "required = \"hello\"\n";
        let parsed: Partial = toml::from_str(input).unwrap();
        assert_eq!(parsed.required, "hello");
        assert_eq!(parsed.optional, None);
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn default_config_has_expected_values() {
        let config = Config::default();
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        assert!(config.life_sheet_path.ends_with(DEFAULT_LIFE_SHEET_FILE));
        assert_eq!(config.theme, "dark");
        assert!(config.default_llm_provider.is_none());
        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");

        std::fs::write(
            &path,
            r#"
database_url = "postgresql://custom/db"
life_sheet_path = "/custom/life-sheet.yaml"
theme = "light"
default_llm_provider = "anthropic"

[keybindings]
quit = "ctrl+q"
"#,
        )
        .unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.database_url, "postgresql://custom/db");
        assert_eq!(
            config.life_sheet_path,
            PathBuf::from("/custom/life-sheet.yaml")
        );
        assert_eq!(config.theme, "light");
        assert_eq!(config.default_llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(config.keybindings.get("quit").unwrap(), "ctrl+q");
    }

    #[test]
    fn save_and_reload_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("round-trip.toml");

        let mut config = Config::default();
        config.theme = "light".into();
        config.keybindings.insert("help".into(), "shift+?".into());

        config.save_to(&path).unwrap();

        let reloaded = Config::load_from(&path).unwrap();
        assert_eq!(config, reloaded);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.toml");

        std::fs::write(&path, "theme = \"solarized\"\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.theme, "solarized");
        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        assert!(config.default_llm_provider.is_none());
        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn ensure_exists_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME);

        // Save a default config to the temp path directly
        let config = Config::default();
        config.save_to(&path).unwrap();

        assert!(path.exists());

        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.database_url, DEFAULT_DATABASE_URL);
    }

    #[test]
    fn env_override_database_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env-test.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();

        let mut loaded = Config::load_from(&path).unwrap();

        // Simulate env override
        loaded.database_url = "postgresql://from-env/override".into();
        assert_eq!(loaded.database_url, "postgresql://from-env/override");
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("config.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn config_dir_ends_with_lazyjob() {
        let dir = config_dir();
        assert!(dir.ends_with(CONFIG_DIR_NAME));
    }

    #[test]
    fn config_path_ends_with_toml() {
        let path = config_path();
        assert!(path.ends_with(CONFIG_FILE_NAME));
    }

    #[test]
    fn empty_toml_uses_all_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"")
            .unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config, Config::default());
    }
}
