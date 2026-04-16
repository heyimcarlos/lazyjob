use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    types::embeddings::CreateEmbeddingRequestArgs,
};
use async_trait::async_trait;
use secrecy::ExposeSecret;

use crate::{
    error::{LlmError, Result},
    message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage},
    provider::{EmbeddingProvider, LlmProvider},
};

const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";

pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
    default_model: String,
    embedding_model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key.into());
        Self {
            client: Client::with_config(config),
            default_model: DEFAULT_MODEL.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub fn from_credentials(creds: &lazyjob_core::credentials::CredentialManager) -> Result<Self> {
        let secret = creds
            .get_api_key("openai")
            .map_err(|e| LlmError::Auth(e.to_string()))?
            .ok_or_else(|| LlmError::Auth("no API key set for provider 'openai'".into()))?;
        Ok(Self::new(secret.expose_secret().to_owned()))
    }

    fn map_openai_error(err: OpenAIError) -> LlmError {
        let msg = err.to_string();
        if msg.contains("401") || msg.contains("Unauthorized") || msg.contains("invalid_api_key") {
            LlmError::Auth(msg)
        } else if msg.contains("429") || msg.contains("rate_limit") {
            LlmError::RateLimit
        } else {
            LlmError::Api(msg)
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn provider_name(&self) -> &str {
        "openai"
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
        let max_tokens = opts.max_tokens.unwrap_or(4096);

        let mut api_messages = Vec::with_capacity(messages.len());
        for msg in messages {
            let api_msg = match msg {
                ChatMessage::System(content) => ChatCompletionRequestSystemMessageArgs::default()
                    .content(content)
                    .build()
                    .map_err(|e: OpenAIError| LlmError::Api(e.to_string()))?
                    .into(),
                ChatMessage::User(content) => ChatCompletionRequestUserMessageArgs::default()
                    .content(content)
                    .build()
                    .map_err(|e: OpenAIError| LlmError::Api(e.to_string()))?
                    .into(),
                ChatMessage::Assistant(content) => {
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(content)
                        .build()
                        .map_err(|e: OpenAIError| LlmError::Api(e.to_string()))?
                        .into()
                }
            };
            api_messages.push(api_msg);
        }

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder
            .model(model)
            .max_tokens(max_tokens)
            .messages(api_messages);
        if let Some(temp) = opts.temperature {
            builder.temperature(temp);
        }
        let request = builder
            .build()
            .map_err(|e: OpenAIError| LlmError::Api(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(Self::map_openai_error)?;

        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let (prompt_tokens, completion_tokens) = response
            .usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        let usage = TokenUsage::new(prompt_tokens, completion_tokens);
        Ok(LlmResponse::new(content, response.model, usage))
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiProvider {
    fn provider_name(&self) -> &str {
        "openai"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let request = CreateEmbeddingRequestArgs::default()
            .model(&self.embedding_model)
            .input(text)
            .build()
            .map_err(|e: OpenAIError| LlmError::Api(e.to_string()))?;

        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(Self::map_openai_error)?;

        response
            .data
            .into_iter()
            .next()
            .map(|e| e.embedding)
            .ok_or_else(|| LlmError::Api("empty embeddings response from OpenAI".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies async_openai Client::with_config builds without network call
    #[test]
    fn async_openai_client_builds_with_config() {
        let config = OpenAIConfig::new().with_api_key("test-key-does-not-matter");
        let _client: Client<OpenAIConfig> = Client::with_config(config);
    }

    // learning test: verifies CreateChatCompletionRequestArgs can build a request struct
    #[tokio::test]
    async fn async_openai_chat_request_serializes() {
        let msg = ChatCompletionRequestUserMessageArgs::default()
            .content("Hello!")
            .build()
            .unwrap();
        let request = CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini")
            .max_tokens(100u32)
            .messages(vec![msg.into()])
            .build()
            .unwrap();
        assert_eq!(request.model, "gpt-4o-mini");
        assert_eq!(request.messages.len(), 1);
    }

    #[test]
    fn openai_provider_name_and_model() {
        use crate::provider::LlmProvider;
        let provider = OpenAiProvider::new("test-key");
        assert_eq!(LlmProvider::provider_name(&provider), "openai");
        assert_eq!(provider.model_name(), DEFAULT_MODEL);
    }

    #[test]
    fn openai_with_model_override() {
        use crate::provider::LlmProvider;
        let provider = OpenAiProvider::new("test-key").with_model("gpt-4o");
        assert_eq!(provider.model_name(), "gpt-4o");
        assert_eq!(LlmProvider::provider_name(&provider), "openai");
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn openai_real_chat_call() {
        let api_key = std::env::var("OPENAI_API_KEY")
            .expect("OPENAI_API_KEY must be set for integration tests");
        let provider = OpenAiProvider::new(api_key);
        let messages = vec![
            ChatMessage::System("You are a concise assistant.".into()),
            ChatMessage::User("Say hello in exactly 3 words.".into()),
        ];
        let opts = CompletionOptions {
            max_tokens: Some(20),
            ..CompletionOptions::default()
        };
        let resp = provider.complete(messages, opts).await.unwrap();
        assert!(!resp.content.is_empty());
        assert!(resp.usage.total_tokens > 0);
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn openai_real_embedding_call() {
        let api_key = std::env::var("OPENAI_API_KEY")
            .expect("OPENAI_API_KEY must be set for integration tests");
        let provider = OpenAiProvider::new(api_key);
        let embedding = provider.embed("Rust programming language").await.unwrap();
        assert_eq!(
            embedding.len(),
            1536,
            "text-embedding-3-small produces 1536 dims"
        );
    }
}
