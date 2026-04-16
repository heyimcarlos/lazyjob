use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{LlmError, Result};
use crate::message::{ChatMessage, CompletionOptions, LlmResponse, TokenUsage};
use crate::provider::LlmProvider;

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
const BACKOFF_DELAYS_SECS: [u64; 3] = [1, 2, 4];

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    default_model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("failed to build reqwest client"),
            api_key: api_key.into(),
            default_model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub fn from_credentials(creds: &lazyjob_core::credentials::CredentialManager) -> Result<Self> {
        use secrecy::ExposeSecret;
        let secret = creds
            .get_api_key("anthropic")
            .map_err(|e| LlmError::Auth(e.to_string()))?
            .ok_or_else(|| LlmError::Auth("no API key set for provider 'anthropic'".into()))?;
        Ok(Self::new(secret.expose_secret().to_owned()))
    }

    fn build_request(
        &self,
        messages: Vec<ChatMessage>,
        opts: &CompletionOptions,
    ) -> AnthropicRequest {
        let model = opts
            .model
            .as_deref()
            .unwrap_or(&self.default_model)
            .to_string();
        let max_tokens = opts.max_tokens.unwrap_or(4096);

        let mut system: Option<String> = None;
        let mut api_messages: Vec<AnthropicMessage> = Vec::new();

        for msg in messages {
            match msg {
                ChatMessage::System(content) => {
                    system = Some(content);
                }
                ChatMessage::User(content) => {
                    api_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                ChatMessage::Assistant(content) => {
                    api_messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
            }
        }

        AnthropicRequest {
            model,
            max_tokens,
            system,
            messages: api_messages,
            stream: opts.stream,
        }
    }

    fn map_http_error(&self, status: reqwest::StatusCode, body: &str) -> LlmError {
        match status.as_u16() {
            401 => LlmError::Auth(body.to_string()),
            429 => LlmError::RateLimit,
            _ => LlmError::Api(format!("HTTP {}: {}", status, body)),
        }
    }

    fn is_retryable(err: &LlmError) -> bool {
        matches!(err, LlmError::RateLimit | LlmError::Api(_))
    }

    async fn call_once(&self, req: &AnthropicRequest) -> Result<LlmResponse> {
        if req.stream {
            self.call_streaming(req).await
        } else {
            self.call_non_streaming(req).await
        }
    }

    async fn call_non_streaming(&self, req: &AnthropicRequest) -> Result<LlmResponse> {
        let response = self
            .client
            .post(ANTHROPIC_API_BASE)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(req)
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(self.map_http_error(status, &body));
        }

        let api_resp: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| LlmError::Api(format!("failed to parse response: {}", e)))?;

        let content = api_resp
            .content
            .into_iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        let usage = TokenUsage::new(api_resp.usage.input_tokens, api_resp.usage.output_tokens);
        let mut resp = LlmResponse::new(content, api_resp.model, usage);
        resp.stop_reason = api_resp.stop_reason;
        Ok(resp)
    }

    async fn call_streaming(&self, req: &AnthropicRequest) -> Result<LlmResponse> {
        let response = self
            .client
            .post(ANTHROPIC_API_BASE)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(req)
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(self.map_http_error(status, &body));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| LlmError::Stream(e.to_string()))?;

        parse_sse_response(&bytes)
    }

    async fn call_with_backoff(&self, req: &AnthropicRequest) -> Result<LlmResponse> {
        let mut delays = BACKOFF_DELAYS_SECS.iter();
        loop {
            match self.call_once(req).await {
                Ok(resp) => return Ok(resp),
                Err(e) if Self::is_retryable(&e) => {
                    if let Some(&delay) = delays.next() {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model_name(&self) -> &str {
        &self.default_model
    }

    async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        opts: CompletionOptions,
    ) -> Result<LlmResponse> {
        let req = self.build_request(messages, &opts);
        self.call_with_backoff(&req).await
    }
}

fn parse_sse_response(bytes: &[u8]) -> Result<LlmResponse> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| LlmError::Stream(format!("invalid UTF-8 in SSE stream: {e}")))?;

    let mut content = String::new();
    let mut model = String::new();
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut stop_reason: Option<String> = None;

    for line in text.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }

        let Ok(event) = serde_json::from_str::<SseEvent>(data) else {
            continue;
        };

        match event.event_type.as_str() {
            "message_start" => {
                if let Some(msg) = event.message {
                    model = msg.model.unwrap_or_default();
                    input_tokens = msg.usage.map(|u| u.input_tokens).unwrap_or(0);
                }
            }
            "content_block_delta" => {
                if let Some(delta) = event.delta
                    && delta.delta_type.as_deref() == Some("text_delta")
                    && let Some(text) = delta.text
                {
                    content.push_str(&text);
                }
            }
            "message_delta" => {
                if let Some(delta) = event.delta {
                    stop_reason = delta.stop_reason;
                }
                if let Some(usage) = event.usage {
                    output_tokens = usage.output_tokens;
                }
            }
            _ => {}
        }
    }

    let usage = TokenUsage::new(input_tokens, output_tokens);
    let mut resp = LlmResponse::new(content, model, usage);
    resp.stop_reason = stop_reason;
    Ok(resp)
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    message: Option<SseMessage>,
    delta: Option<SseDelta>,
    usage: Option<SseUsage>,
}

#[derive(Debug, Deserialize)]
struct SseMessage {
    model: Option<String>,
    usage: Option<SseUsage>,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies reqwest::Client builds successfully with rustls-tls feature
    #[test]
    fn reqwest_client_builds_with_rustls() {
        let client = reqwest::Client::builder().use_rustls_tls().build();
        assert!(client.is_ok(), "reqwest client with rustls should build");
    }

    // learning test: verifies serde serializes AnthropicRequest JSON correctly
    #[test]
    fn reqwest_json_serializes_request_body() {
        let req = AnthropicRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            max_tokens: 1024,
            system: Some("You are helpful.".to_string()),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            }],
            stream: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\""));
        assert!(json.contains("claude-haiku-4-5-20251001"));
        assert!(json.contains("\"system\""));
        assert!(json.contains("You are helpful."));
        assert!(json.contains("\"messages\""));
        assert!(json.contains("\"stream\":false"));
    }

    #[test]
    fn anthropic_provider_name_and_default_model() {
        let provider = AnthropicProvider::new("test-key");
        assert_eq!(provider.provider_name(), "anthropic");
        assert_eq!(provider.model_name(), DEFAULT_MODEL);
    }

    #[test]
    fn with_model_overrides_default() {
        let provider = AnthropicProvider::new("key").with_model("claude-opus-4-6");
        assert_eq!(provider.model_name(), "claude-opus-4-6");
    }

    #[test]
    fn build_request_separates_system_messages() {
        let provider = AnthropicProvider::new("key");
        let messages = vec![
            ChatMessage::System("You are a helpful assistant.".into()),
            ChatMessage::User("Hello".into()),
        ];
        let req = provider.build_request(messages, &CompletionOptions::default());
        assert_eq!(req.system.as_deref(), Some("You are a helpful assistant."));
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
        assert_eq!(req.messages[0].content, "Hello");
    }

    #[test]
    fn build_request_with_no_system_message() {
        let provider = AnthropicProvider::new("key");
        let messages = vec![ChatMessage::User("Hi".into())];
        let req = provider.build_request(messages, &CompletionOptions::default());
        assert!(req.system.is_none());
        assert_eq!(req.messages.len(), 1);
    }

    #[test]
    fn build_request_uses_opts_model_override() {
        let provider = AnthropicProvider::new("key");
        let opts = CompletionOptions {
            model: Some("claude-opus-4-6".to_string()),
            ..CompletionOptions::default()
        };
        let req = provider.build_request(vec![ChatMessage::User("hi".into())], &opts);
        assert_eq!(req.model, "claude-opus-4-6");
    }

    #[test]
    fn map_http_error_401_returns_auth() {
        let provider = AnthropicProvider::new("key");
        let err = provider.map_http_error(reqwest::StatusCode::UNAUTHORIZED, "bad key");
        assert!(matches!(err, LlmError::Auth(_)));
    }

    #[test]
    fn map_http_error_429_returns_rate_limit() {
        let provider = AnthropicProvider::new("key");
        let err = provider.map_http_error(reqwest::StatusCode::TOO_MANY_REQUESTS, "slow down");
        assert!(matches!(err, LlmError::RateLimit));
    }

    #[test]
    fn map_http_error_500_returns_api() {
        let provider = AnthropicProvider::new("key");
        let err = provider.map_http_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "oops");
        assert!(matches!(err, LlmError::Api(_)));
    }

    #[test]
    fn is_retryable_for_rate_limit_and_api_errors() {
        assert!(AnthropicProvider::is_retryable(&LlmError::RateLimit));
        assert!(AnthropicProvider::is_retryable(&LlmError::Api(
            "server error".into()
        )));
        assert!(!AnthropicProvider::is_retryable(&LlmError::Auth(
            "bad key".into()
        )));
        assert!(!AnthropicProvider::is_retryable(&LlmError::Network(
            "timeout".into()
        )));
    }

    #[test]
    fn parse_non_streaming_response() {
        let json = r#"{
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello, world!"}],
            "model": "claude-haiku-4-5-20251001",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {"input_tokens": 25, "output_tokens": 10}
        }"#;
        let api_resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        let content: String = api_resp
            .content
            .into_iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(content, "Hello, world!");
        assert_eq!(api_resp.model, "claude-haiku-4-5-20251001");
        assert_eq!(api_resp.usage.input_tokens, 25);
        assert_eq!(api_resp.usage.output_tokens, 10);
        assert_eq!(api_resp.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_sse_stream_accumulates_text() {
        let sse_data = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-haiku-4-5-20251001\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":25,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\", world!\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":15}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let resp = parse_sse_response(sse_data.as_bytes()).unwrap();
        assert_eq!(resp.content, "Hello, world!");
        assert_eq!(resp.model, "claude-haiku-4-5-20251001");
        assert_eq!(resp.usage.prompt_tokens, 25);
        assert_eq!(resp.usage.completion_tokens, 15);
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_sse_stream_with_empty_data_lines_skipped() {
        let sse_data = "data: {\"type\":\"ping\"}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        let resp = parse_sse_response(sse_data.as_bytes()).unwrap();
        assert_eq!(resp.content, "");
    }

    #[cfg(feature = "integration")]
    #[tokio::test]
    async fn anthropic_real_call_to_haiku() {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY must be set for integration tests");
        let provider = AnthropicProvider::new(api_key);
        let messages = vec![
            ChatMessage::System("You are a concise assistant.".into()),
            ChatMessage::User("Say hello in exactly 3 words.".into()),
        ];
        let opts = CompletionOptions {
            max_tokens: Some(20),
            ..CompletionOptions::default()
        };
        let resp = provider.complete(messages, opts).await.unwrap();
        assert!(
            !resp.content.is_empty(),
            "response content should not be empty"
        );
        assert!(!resp.model.is_empty(), "model should be set");
        assert!(
            resp.usage.total_tokens > 0,
            "token usage should be non-zero"
        );
    }
}
