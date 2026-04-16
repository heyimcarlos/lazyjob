use thiserror::Error;

pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("API error: {0}")]
    Api(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("rate limit exceeded")]
    RateLimit,

    #[error("network error: {0}")]
    Network(String),

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("context length exceeded: max {max}, actual {actual}")]
    ContextLengthExceeded { max: usize, actual: usize },

    #[error("stream error: {0}")]
    Stream(String),

    #[error("not supported: {0}")]
    NotSupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_error_display() {
        assert_eq!(
            LlmError::Api("bad request".into()).to_string(),
            "API error: bad request"
        );
        assert_eq!(LlmError::RateLimit.to_string(), "rate limit exceeded");
        assert_eq!(
            LlmError::ContextLengthExceeded {
                max: 4096,
                actual: 5000
            }
            .to_string(),
            "context length exceeded: max 4096, actual 5000"
        );
    }

    #[test]
    fn llm_error_auth_variant() {
        let err = LlmError::Auth("invalid key".into());
        assert!(err.to_string().contains("authentication error"));
    }
}
