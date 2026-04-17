pub mod cost;
pub mod error;
pub mod message;
pub mod mock;
pub mod prompts;
pub mod provider;
pub mod providers;
pub mod registry;

pub use cost::estimate_cost;
pub use error::{LlmError, Result};
pub use message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage};
pub use mock::{MockEmbeddingProvider, MockLlmProvider};
pub use provider::{EmbeddingProvider, LlmProvider};
pub use providers::{AnthropicProvider, OllamaProvider, OpenAiProvider};
pub use registry::{LlmBuilder, LoggingProvider, ProviderRegistry};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
