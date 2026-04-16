use async_trait::async_trait;

use crate::error::Result;
use crate::message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage};
use crate::provider::{EmbeddingProvider, LlmProvider};

pub struct MockLlmProvider {
    response: LlmResponse,
}

impl MockLlmProvider {
    pub fn new(response: LlmResponse) -> Self {
        Self { response }
    }

    pub fn with_content(content: &str) -> Self {
        let usage = TokenUsage::new(10, 20);
        Self {
            response: LlmResponse::new(content, "mock-model", usage),
        }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn provider_name(&self) -> &str {
        "mock"
    }

    fn model_name(&self) -> &str {
        &self.response.model
    }

    async fn complete(
        &self,
        _messages: Vec<ChatMessage>,
        _opts: CompletionOptions,
    ) -> Result<LlmResponse> {
        Ok(self.response.clone())
    }
}

pub struct MockEmbeddingProvider {
    embedding: Vec<f32>,
}

impl MockEmbeddingProvider {
    pub fn new(embedding: Vec<f32>) -> Self {
        Self { embedding }
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbeddingProvider {
    fn provider_name(&self) -> &str {
        "mock-embedding"
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(self.embedding.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies async_trait enables dyn dispatch for LlmProvider
    #[tokio::test]
    async fn async_trait_dyn_dispatch() {
        let provider: Box<dyn LlmProvider> = Box::new(MockLlmProvider::with_content("hello dyn"));
        let msgs = vec![ChatMessage::User("hi".into())];
        let resp = provider
            .complete(msgs, CompletionOptions::default())
            .await
            .unwrap();
        assert_eq!(resp.content, "hello dyn");
        assert_eq!(provider.provider_name(), "mock");
    }

    #[tokio::test]
    async fn mock_provider_returns_canned_response() {
        let provider = MockLlmProvider::with_content("canned response");
        let msgs = vec![ChatMessage::User("test".into())];
        let resp = provider
            .complete(msgs, CompletionOptions::default())
            .await
            .unwrap();
        assert_eq!(resp.content, "canned response");
        assert_eq!(resp.model, "mock-model");
        assert_eq!(resp.usage.total_tokens, 30);
    }

    #[test]
    fn mock_provider_with_content_helper() {
        let provider = MockLlmProvider::with_content("test content");
        assert_eq!(provider.provider_name(), "mock");
        assert_eq!(provider.model_name(), "mock-model");
        assert_eq!(provider.response.content, "test content");
    }

    #[tokio::test]
    async fn mock_provider_new_with_custom_response() {
        let usage = TokenUsage::new(5, 15);
        let resp = LlmResponse::new("custom", "gpt-4", usage);
        let provider = MockLlmProvider::new(resp);
        let result = provider
            .complete(
                vec![ChatMessage::System("hi".into())],
                CompletionOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.content, "custom");
        assert_eq!(result.model, "gpt-4");
        assert_eq!(result.usage.total_tokens, 20);
    }

    // learning test: verifies async_trait enables dyn dispatch for EmbeddingProvider
    #[tokio::test]
    async fn async_trait_dyn_dispatch_embedding() {
        let embedding = vec![0.1_f32, 0.2, 0.3, 0.4];
        let provider: Box<dyn EmbeddingProvider> =
            Box::new(MockEmbeddingProvider::new(embedding.clone()));
        let result = provider.embed("some text").await.unwrap();
        assert_eq!(result, embedding);
        assert_eq!(provider.provider_name(), "mock-embedding");
    }

    #[tokio::test]
    async fn mock_embedding_returns_canned_vec() {
        let provider = MockEmbeddingProvider::new(vec![1.0, 2.0, 3.0]);
        let result = provider.embed("anything").await.unwrap();
        assert_eq!(result, vec![1.0, 2.0, 3.0]);
    }

    #[tokio::test]
    async fn mock_provider_called_multiple_times_returns_same_response() {
        let provider = MockLlmProvider::with_content("stable");
        for _ in 0..3 {
            let resp = provider
                .complete(
                    vec![ChatMessage::User("q".into())],
                    CompletionOptions::default(),
                )
                .await
                .unwrap();
            assert_eq!(resp.content, "stable");
        }
    }
}
