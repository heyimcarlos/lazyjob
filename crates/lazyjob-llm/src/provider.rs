use async_trait::async_trait;

use crate::error::Result;
use crate::message::{ChatMessage, CompletionOptions, LlmResponse};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;

    async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        opts: CompletionOptions,
    ) -> Result<LlmResponse>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;

    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}
