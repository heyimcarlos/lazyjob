use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lazyjob_core::config::Config;
use lazyjob_core::credentials::CredentialManager;
use sqlx::PgPool;

use crate::Result;
use crate::cost::estimate_cost;
use crate::message::{ChatMessage, CompletionOptions, LlmResponse};
use crate::provider::LlmProvider;
use crate::providers::{AnthropicProvider, OllamaProvider, OpenAiProvider};

pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default: Option<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default: None,
        }
    }

    pub fn add(&mut self, name: impl Into<String>, provider: Arc<dyn LlmProvider>) {
        let name = name.into();
        if self.default.is_none() {
            self.default = Some(name.clone());
        }
        self.providers.insert(name, provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(name).cloned()
    }

    pub fn default_provider(&self) -> Option<Arc<dyn LlmProvider>> {
        self.default
            .as_ref()
            .and_then(|n| self.providers.get(n))
            .cloned()
    }

    pub fn all(&self) -> Vec<(String, Arc<dyn LlmProvider>)> {
        self.providers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn set_default(&mut self, name: impl Into<String>) {
        self.default = Some(name.into());
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LlmBuilder;

impl LlmBuilder {
    pub fn from_config(config: &Config, creds: &CredentialManager) -> Result<Box<dyn LlmProvider>> {
        if let Some(ref name) = config.default_llm_provider {
            match name.as_str() {
                "anthropic" => {
                    if let Ok(p) = AnthropicProvider::from_credentials(creds) {
                        return Ok(Box::new(p));
                    }
                }
                "openai" => {
                    if let Ok(p) = OpenAiProvider::from_credentials(creds) {
                        return Ok(Box::new(p));
                    }
                }
                "ollama" => return Ok(Box::new(OllamaProvider::new())),
                _ => {}
            }
        }

        if let Ok(p) = AnthropicProvider::from_credentials(creds) {
            return Ok(Box::new(p));
        }

        if let Ok(p) = OpenAiProvider::from_credentials(creds) {
            return Ok(Box::new(p));
        }

        Ok(Box::new(OllamaProvider::new()))
    }
}

pub struct LoggingProvider {
    inner: Arc<dyn LlmProvider>,
    pool: Option<PgPool>,
    operation: Option<String>,
}

impl LoggingProvider {
    pub fn new(provider: Arc<dyn LlmProvider>, pool: PgPool) -> Self {
        Self {
            inner: provider,
            pool: Some(pool),
            operation: None,
        }
    }

    pub fn without_pool(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner: provider,
            pool: None,
            operation: None,
        }
    }

    pub fn with_operation(mut self, op: impl Into<String>) -> Self {
        self.operation = Some(op.into());
        self
    }

    async fn log_usage(&self, response: &LlmResponse) {
        let Some(ref pool) = self.pool else {
            return;
        };
        let cost = estimate_cost(&response.model, response.usage.total_tokens);
        let _ = sqlx::query(
            "INSERT INTO token_usage_log (id, provider, model, input_tokens, output_tokens, cost_microdollars, operation) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(self.inner.provider_name())
        .bind(&response.model)
        .bind(response.usage.prompt_tokens as i32)
        .bind(response.usage.completion_tokens as i32)
        .bind(cost as i64)
        .bind(self.operation.as_deref())
        .execute(pool)
        .await;
    }
}

#[async_trait]
impl LlmProvider for LoggingProvider {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        opts: CompletionOptions,
    ) -> Result<LlmResponse> {
        let response = self.inner.complete(messages, opts).await?;
        self.log_usage(&response).await;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyjob_core::credentials::InMemoryStore;

    use crate::mock::MockLlmProvider;

    fn no_creds() -> CredentialManager {
        CredentialManager::with_store(Box::new(InMemoryStore::new()))
    }

    fn default_config() -> Config {
        Config::default()
    }

    #[test]
    fn registry_add_and_get() {
        let mut reg = ProviderRegistry::new();
        let mock = Arc::new(MockLlmProvider::with_content("hello"));
        reg.add("mock", mock as Arc<dyn LlmProvider>);
        assert!(reg.get("mock").is_some());
    }

    #[test]
    fn registry_get_missing_returns_none() {
        let reg = ProviderRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn registry_default_provider_after_set_default() {
        let mut reg = ProviderRegistry::new();
        let mock = Arc::new(MockLlmProvider::with_content("hi"));
        reg.add("mock", mock as Arc<dyn LlmProvider>);
        reg.set_default("mock");
        assert!(reg.default_provider().is_some());
    }

    #[test]
    fn registry_first_added_becomes_default() {
        let mut reg = ProviderRegistry::new();
        let mock = Arc::new(MockLlmProvider::with_content("hi"));
        reg.add("first", mock.clone() as Arc<dyn LlmProvider>);
        reg.add("second", mock as Arc<dyn LlmProvider>);
        let def = reg.default_provider().unwrap();
        assert_eq!(def.provider_name(), "mock");
    }

    #[test]
    fn registry_all_returns_all_providers() {
        let mut reg = ProviderRegistry::new();
        let mock = Arc::new(MockLlmProvider::with_content("hi"));
        reg.add("a", mock.clone() as Arc<dyn LlmProvider>);
        reg.add("b", mock as Arc<dyn LlmProvider>);
        assert_eq!(reg.all().len(), 2);
    }

    #[test]
    fn registry_default_is_none_when_empty() {
        let reg = ProviderRegistry::new();
        assert!(reg.default_provider().is_none());
    }

    #[test]
    fn builder_falls_back_to_ollama_with_no_creds() {
        let config = default_config();
        let creds = no_creds();
        let provider = LlmBuilder::from_config(&config, &creds).unwrap();
        assert_eq!(provider.provider_name(), "ollama");
    }

    #[test]
    fn builder_uses_anthropic_when_key_set() {
        use lazyjob_core::credentials::CredentialStore;

        let store = InMemoryStore::new();
        store.set("api_key:anthropic", "sk-ant-test123").unwrap();
        let creds = CredentialManager::with_store(Box::new(store));
        let mut config = default_config();
        config.default_llm_provider = None;

        let provider = LlmBuilder::from_config(&config, &creds).unwrap();
        assert_eq!(provider.provider_name(), "anthropic");
    }

    #[test]
    fn builder_uses_openai_when_anthropic_missing_but_openai_set() {
        use lazyjob_core::credentials::CredentialStore;

        let store = InMemoryStore::new();
        store.set("api_key:openai", "sk-openai-test123").unwrap();
        let creds = CredentialManager::with_store(Box::new(store));
        let config = default_config();

        let provider = LlmBuilder::from_config(&config, &creds).unwrap();
        assert_eq!(provider.provider_name(), "openai");
    }

    #[test]
    fn builder_respects_configured_provider_ollama() {
        let mut config = default_config();
        config.default_llm_provider = Some("ollama".to_string());
        let creds = no_creds();
        let provider = LlmBuilder::from_config(&config, &creds).unwrap();
        assert_eq!(provider.provider_name(), "ollama");
    }

    #[test]
    fn builder_falls_through_when_configured_provider_missing_key() {
        let mut config = default_config();
        config.default_llm_provider = Some("anthropic".to_string());
        let creds = no_creds();
        // No anthropic key set — should fall through to ollama
        let provider = LlmBuilder::from_config(&config, &creds).unwrap();
        assert_eq!(provider.provider_name(), "ollama");
    }

    #[test]
    fn logging_provider_delegates_provider_name() {
        let mock = Arc::new(MockLlmProvider::with_content("hello"));
        let logging = LoggingProvider::without_pool(mock as Arc<dyn LlmProvider>);
        assert_eq!(logging.provider_name(), "mock");
    }

    #[test]
    fn logging_provider_delegates_model_name() {
        let mock = Arc::new(MockLlmProvider::with_content("hello"));
        let logging = LoggingProvider::without_pool(mock as Arc<dyn LlmProvider>);
        assert_eq!(logging.model_name(), "mock-model");
    }

    #[tokio::test]
    async fn logging_provider_delegates_complete_without_pool() {
        use crate::message::ChatMessage;

        let mock = Arc::new(MockLlmProvider::with_content("answer"));
        let logging = LoggingProvider::without_pool(mock as Arc<dyn LlmProvider>);
        let response = logging
            .complete(
                vec![ChatMessage::User("question".to_string())],
                CompletionOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.content, "answer");
    }

    #[tokio::test]
    async fn logging_provider_with_operation_label() {
        use crate::message::ChatMessage;

        let mock = Arc::new(MockLlmProvider::with_content("result"));
        let logging =
            LoggingProvider::without_pool(mock as Arc<dyn LlmProvider>).with_operation("test-op");
        let response = logging
            .complete(
                vec![ChatMessage::User("q".to_string())],
                CompletionOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.content, "result");
        assert_eq!(logging.operation.as_deref(), Some("test-op"));
    }
}
