use super::types::LoopType;

#[derive(thiserror::Error, Debug)]
pub enum TemplateError {
    #[error("missing required variable '{name}' in template '{template}'")]
    MissingVariable { name: String, template: String },

    #[error("template parse error in '{file}': {source}")]
    ParseError {
        file: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("user override TOML is invalid in '{path}': {source}")]
    OverrideParseError {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("no template registered for {0:?}")]
    NotRegistered(LoopType),

    #[error("output validation failed: {0}")]
    ValidationFailed(String),
}

pub type Result<T> = std::result::Result<T, TemplateError>;
