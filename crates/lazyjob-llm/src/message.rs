use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", content = "content", rename_all = "lowercase")]
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(String),
}

impl ChatMessage {
    pub fn role(&self) -> &str {
        match self {
            Self::System(_) => "system",
            Self::User(_) => "user",
            Self::Assistant(_) => "assistant",
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Self::System(c) | Self::User(c) | Self::Assistant(c) => c,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionOptions {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            model: None,
            temperature: Some(0.7),
            max_tokens: Some(4096),
            stream: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl TokenUsage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}

impl LlmResponse {
    pub fn new(content: impl Into<String>, model: impl Into<String>, usage: TokenUsage) -> Self {
        Self {
            content: content.into(),
            model: model.into(),
            usage,
            stop_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_role_and_content() {
        let system = ChatMessage::System("You are helpful.".into());
        assert_eq!(system.role(), "system");
        assert_eq!(system.content(), "You are helpful.");

        let user = ChatMessage::User("Hello!".into());
        assert_eq!(user.role(), "user");
        assert_eq!(user.content(), "Hello!");

        let assistant = ChatMessage::Assistant("Hi there!".into());
        assert_eq!(assistant.role(), "assistant");
        assert_eq!(assistant.content(), "Hi there!");
    }

    #[test]
    fn completion_options_default() {
        let opts = CompletionOptions::default();
        assert!(opts.model.is_none());
        assert_eq!(opts.temperature, Some(0.7));
        assert_eq!(opts.max_tokens, Some(4096));
        assert!(!opts.stream);
    }

    #[test]
    fn token_usage_total_is_sum() {
        let usage = TokenUsage::new(100, 50);
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn llm_response_construction() {
        let usage = TokenUsage::new(10, 20);
        let resp = LlmResponse::new("Hello!", "claude-3-haiku", usage);
        assert_eq!(resp.content, "Hello!");
        assert_eq!(resp.model, "claude-3-haiku");
        assert_eq!(resp.usage.total_tokens, 30);
        assert!(resp.stop_reason.is_none());
    }

    #[test]
    fn chat_message_serde_round_trip() {
        let msg = ChatMessage::User("test".into());
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role(), "user");
        assert_eq!(back.content(), "test");
    }
}
