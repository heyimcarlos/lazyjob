use thiserror::Error;

#[derive(Debug, Error)]
pub enum RalphError {
    #[error("decode error: {0}")]
    Decode(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("run not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, RalphError>;
