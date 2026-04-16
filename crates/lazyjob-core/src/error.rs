pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("credential error: {0}")]
    Credential(String),

    #[error("HTTP error: {0}")]
    Http(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Serialization(e.to_string())
    }
}

impl From<serde_yaml::Error> for CoreError {
    fn from(e: serde_yaml::Error) -> Self {
        CoreError::Serialization(e.to_string())
    }
}

impl From<toml::de::Error> for CoreError {
    fn from(e: toml::de::Error) -> Self {
        CoreError::Serialization(e.to_string())
    }
}

impl From<toml::ser::Error> for CoreError {
    fn from(e: toml::ser::Error) -> Self {
        CoreError::Serialization(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let err = CoreError::NotFound {
            entity: "Job",
            id: "abc-123".into(),
        };
        assert_eq!(err.to_string(), "Job not found: abc-123");
    }

    #[test]
    fn validation_display() {
        let err = CoreError::Validation("title cannot be empty".into());
        assert_eq!(err.to_string(), "validation error: title cannot be empty");
    }
}
