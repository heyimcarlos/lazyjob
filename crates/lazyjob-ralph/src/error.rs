use thiserror::Error;

#[derive(Debug, Error)]
pub enum RalphError {
    #[error("decode error: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, RalphError>;
