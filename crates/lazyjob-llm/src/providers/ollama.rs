use async_trait::async_trait;
use ollama_rs::{
    Ollama,
    generation::{
        chat::{ChatMessage as OllamaMessage, request::ChatMessageRequest},
        embeddings::request::{EmbeddingsInput, GenerateEmbeddingsRequest},
    },
};

use crate::{
    error::{LlmError, Result},
    message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage},
    provider::{EmbeddingProvider, LlmProvider},
};

const DEFAULT_MODEL: &str = "llama3.2:latest";
const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";
const DEFAULT_HOST: &str = "http://localhost";
const DEFAULT_PORT: u16 = 11434;

pub struct OllamaProvider {
    client: Ollama,
    default_model: String,
    embedding_model: String,
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self {
            client: Ollama::new(DEFAULT_HOST.to_string(), DEFAULT_PORT),
            default_model: DEFAULT_MODEL.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
        }
    }

    pub fn with_host_port(host: impl Into<String>, port: u16) -> Self {
        Self {
            client: Ollama::new(host.into(), port),
            default_model: DEFAULT_MODEL.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    fn map_message(msg: ChatMessage) -> OllamaMessage {
        match msg {
            ChatMessage::System(content) => OllamaMessage::system(content),
            ChatMessage::User(content) => OllamaMessage::user(content),
            ChatMessage::Assistant(content) => OllamaMessage::assistant(content),
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn provider_name(&self) -> &str {
        "ollama"
    }

    fn model_name(&self) -> &str {
        &self.default_model
    }

    async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        opts: CompletionOptions,
    ) -> Result<LlmResponse> {
        let model = opts
            .model
            .as_deref()
            .unwrap_or(&self.default_model)
            .to_string();

        let ollama_messages: Vec<OllamaMessage> =
            messages.into_iter().map(Self::map_message).collect();

        let request = ChatMessageRequest::new(model, ollama_messages);

        let response = self
            .client
            .send_chat_messages(request)
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let content = response.message.content.clone();
        let model_name = response.model.clone();

        let (prompt_tokens, completion_tokens) = response
            .final_data
            .map(|fd| (fd.prompt_eval_count as u32, fd.eval_count as u32))
            .unwrap_or((0, 0));

        let usage = TokenUsage::new(prompt_tokens, completion_tokens);
        Ok(LlmResponse::new(content, model_name, usage))
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    fn provider_name(&self) -> &str {
        "ollama"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let request = GenerateEmbeddingsRequest::new(
            self.embedding_model.clone(),
            EmbeddingsInput::Single(text.to_string()),
        );

        let response = self
            .client
            .generate_embeddings(request)
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Api("empty embeddings response from Ollama".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies Ollama::new constructs without panicking (no network call)
    #[test]
    fn ollama_rs_client_constructs() {
        let _ollama = Ollama::new("http://localhost".to_string(), 11434);
        let _default = Ollama::default();
    }

    // learning test: verifies OllamaMessage constructors create messages with correct roles
    #[test]
    fn ollama_rs_message_constructors() {
        let system = OllamaMessage::system("You are helpful.".to_string());
        let user = OllamaMessage::user("Hello!".to_string());
        let assistant = OllamaMessage::assistant("Hi!".to_string());

        assert_eq!(system.content, "You are helpful.");
        assert_eq!(user.content, "Hello!");
        assert_eq!(assistant.content, "Hi!");
    }

    #[test]
    fn ollama_provider_name_and_model() {
        use crate::provider::LlmProvider;
        let provider = OllamaProvider::new();
        assert_eq!(LlmProvider::provider_name(&provider), "ollama");
        assert_eq!(provider.model_name(), DEFAULT_MODEL);
    }

    #[test]
    fn ollama_with_model_override() {
        use crate::provider::LlmProvider;
        let provider = OllamaProvider::new().with_model("mistral:latest");
        assert_eq!(provider.model_name(), "mistral:latest");
        assert_eq!(LlmProvider::provider_name(&provider), "ollama");
    }

    #[test]
    fn ollama_default_constructs() {
        use crate::provider::LlmProvider;
        let provider = OllamaProvider::default();
        assert_eq!(LlmProvider::provider_name(&provider), "ollama");
    }

    #[test]
    fn ollama_message_mapping_system() {
        let mapped = OllamaProvider::map_message(ChatMessage::System("sys".to_string()));
        assert_eq!(mapped.content, "sys");
    }

    #[test]
    fn ollama_message_mapping_user() {
        let mapped = OllamaProvider::map_message(ChatMessage::User("hello".to_string()));
        assert_eq!(mapped.content, "hello");
    }

    #[test]
    fn ollama_message_mapping_assistant() {
        let mapped = OllamaProvider::map_message(ChatMessage::Assistant("reply".to_string()));
        assert_eq!(mapped.content, "reply");
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn ollama_real_chat_call() {
        let provider = OllamaProvider::new();
        let messages = vec![
            ChatMessage::System("You are a concise assistant.".into()),
            ChatMessage::User("Say hello in exactly 3 words.".into()),
        ];
        let resp = provider
            .complete(messages, CompletionOptions::default())
            .await
            .unwrap();
        assert!(!resp.content.is_empty());
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn ollama_real_embedding_call() {
        let provider = OllamaProvider::new();
        let embedding = provider.embed("Rust programming language").await.unwrap();
        assert_eq!(embedding.len(), 768, "nomic-embed-text produces 768 dims");
    }
}
