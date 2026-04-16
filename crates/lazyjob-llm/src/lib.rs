pub mod error;
pub mod message;
pub mod mock;
pub mod provider;
pub mod providers;

pub use error::{LlmError, Result};
pub use message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage};
pub use mock::{MockEmbeddingProvider, MockLlmProvider};
pub use provider::{EmbeddingProvider, LlmProvider};
pub use providers::AnthropicProvider;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
