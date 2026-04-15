# LLM Provider Abstraction

## Status
Researching

## Problem Statement

LazyJob needs to support multiple LLM providers:
- **Cloud**: Anthropic (Claude), OpenAI (GPT-4)
- **Local**: Ollama (self-hosted models)

These providers have different APIs, authentication schemes, streaming formats, and capability sets. We need a unified abstraction that:
1. Provides a consistent interface for chat, completion, and embedding operations
2. Supports SSE streaming for real-time UI feedback
3. Allows provider-specific configuration
4. Enables fallback when providers fail
5. Tracks token usage for cost management

---

## Research Findings

### async-openai (OpenAI SDK for Rust)

The `async-openai` crate (0.34.0) provides comprehensive OpenAI API support:

**Client Configuration**
```rust
use async_openai::{Client, config::OpenAIConfig};

let config = OpenAIConfig::new()
    .with_api_key("sk-...")
    .with_org_id("org-...")
    .with_api_base("https://api.openai.com"); // or custom for Azure

let client = Client::with_config(config);
```

**Streaming Chat Completions**
```rust
use async_openai::{Client, types::CreateChatCompletionRequest};

let request = CreateChatCompletionRequest {
    model: "gpt-4".to_string(),
    messages: vec![...],
    stream: Some(true),
    ..Default::default()
};

let stream = client.chat().create_stream(request).await?;
```

**SSE Response Format**
Each chunk is a `ChatCompletionChunk` with:
```json
{
  "id": "chatcmpl-123",
  "choices": [{
    "delta": { "content": "partial text" },
    "finish_reason": null
  }]
}
```

**Embeddings**
```rust
let request = CreateEmbeddingRequest {
    model: "text-embedding-ada-002".to_string(),
    input: EmbeddingInput::String("text to embed".to_string()),
    ..Default::default()
};
let response = client.embeddings().create(&request).await?;
```

**Key Limitation**: This crate is OpenAI-specific. No Anthropic support built-in.

### Ollama-rs (Local LLM)

The `ollama-rs` crate provides Ollama API access:

**Client Setup**
```rust
use ollama_rs::Ollama;

let ollama = Ollama::default(); // localhost:11434
let ollama = Ollama::new("http://localhost".to_string(), 11434);
```

**Chat with History**
```rust
use ollama_rs::chat::{ChatMessage, ChatMessageRole};

let history = vec![
    ChatMessage { role: ChatMessageRole::User, content: "Hello".to_string() }
];
let response = ollama
    .send_chat_messages_with_history(history, request)
    .await?;
```

**Streaming**
```rust
let mut stream = ollama.generate_stream(request).await?;
while let Some(response) = stream.next().await {
    // process streamed chunks
}
```

**Embeddings**
```rust
let request = GenerateEmbeddingsRequest::new(model, text);
let response = ollama.generate_embeddings(request).await;
```

**Key Features**:
- Model management (list, show, create, copy, delete)
- Function calling via `Coordinator` and `#[ollama_rs::function]` macro
- Chat history support

### LiteLLM Pattern (Multi-Provider Abstraction)

LiteLLM (Python) is the reference for multi-provider abstraction. Key patterns:
- Unified `completion()` function with `model` parameter (e.g., `model="anthropic/claude-3-5-sonnet"`)
- Transparent handling of provider-specific auth, rate limits, retries
- Cost normalization across providers
- Support for embeddings, image inputs, audio

### Anthropic API (REST)

Anthropic's API differs from OpenAI:
- Uses `messages` endpoint (not `chat/completions`)
- Requires `anthropic-version` header
- Has different streaming format (`event` types: `message_start`, `content_block_delta`, etc.)
- Supports System prompts, but not persistent across calls (must include in messages)

**Anthropic SSE Stream Format**
```
event: message_start
data: {"type":"message_start","message":{"id":"..."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: message_stop
data: {"type":"message_stop"}
```

---

## Design Options

### Option A: Simple Enum with Match

**Description**: Single `enum LLMProvider { Anthropic(...), OpenAI(...), Ollama(...) }` with match arms for each operation.

**Pros**:
- Simple, no trait complexity
- No dynamic dispatch overhead
- Easy to understand and implement

**Cons**:
- Violates Open/Closed principle - adding new providers requires modifying existing code
- Code duplication across providers for similar operations
- Hard to test individual providers
- Cannot be extended by external crates

**Best for**: MVP with fixed set of providers

### Option B: Trait-Based Abstraction (Recommended)

**Description**: Define `LLMProvider` trait with async methods for chat, complete, embed. Implement for each provider. Use a registry/factory pattern.

```rust
pub trait LLMProvider: Send + Sync {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse>;
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<StreamingResponse>;
    async fn complete(&self, prompt: &str) -> Result<String>;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
}
```

**Pros**:
- Clean abstraction with defined interface
- Each provider is tested independently
- Can add new providers without modifying existing code
- Enables dependency injection for testing (mock providers)
-符合 Rust idioms (trait-based polymorphism)

**Cons**:
- Some complexity in trait design
- Streaming requires additional thought (return `Pin<Box<dyn Stream>>`)
- Need to handle provider-specific error types

**Best for**: Production systems with multiple providers and long-term maintenance

### Option C: Actor-Based with Message Passing

**Description**: Each provider runs as an actor with message-passing for requests/responses. Uses `tokio::spawn` and channels.

**Pros**:
- Natural concurrency model
- Provider instances isolated from each other
- Built-in backpressure via channel buffering

**Cons**:
- Heavy for simple use cases
- Message passing overhead
- Complexity in managing actor lifecycle
- Harder to implement streaming responses

**Best for**: Systems requiring strong isolation or distributed deployment

### Option D: Request Router (LiteLLM-style)

**Description**: Single `completion()` function that routes based on model name prefix (e.g., `"anthropic/claude-3-5"` routes to Anthropic).

**Pros**:
- Simple API for consumers
- Easy to add new providers
- Consistent interface

**Cons**:
- Model name conventions become part of the API contract
- Provider configuration is global/router-level
- Harder to have provider-specific options

**Best for**: API gateways, proxy services

---

## Recommended Approach

**Option B: Trait-Based Abstraction** is recommended.

Rationale:
- Clean, idiomatic Rust design
- Enables testing with mock providers
- Clear separation of concerns
- Provider-specific configuration isolated in implementations
- Can be extended without modifying consumers

---

## Trait Design

### Core Provider Trait

```rust
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LLMError {
    #[error("API error: {0}")]
    Api(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Rate limited")]
    RateLimited,
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Context length exceeded")]
    ContextLengthExceeded,
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Streaming error: {0}")]
    Stream(String),
    #[error("Other: {0}")]
    Other(String),
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Provider name (e.g., "anthropic", "openai", "ollama")
    fn provider_name(&self) -> &str;

    /// Model name (e.g., "claude-3-5-sonnet-20241022")
    fn model_name(&self) -> &str;

    /// Send a chat message and get a complete response
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError>;

    /// Send a chat message and stream the response
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>, LLMError>;

    /// Simple text completion (may use chat behind the scenes)
    async fn complete(&self, prompt: &str) -> Result<String, LLMError> {
        let messages = vec![ChatMessage::user(prompt.to_string())];
        let response = self.chat(messages).await?;
        Ok(response.content)
    }

    /// Generate embeddings for text
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError>;

    /// Get maximum context length for this model
    fn context_length(&self) -> u32;
}
```

### Message Types

```rust
pub enum ChatMessage {
    System { content: String },
    User { content: String },
    Assistant { content: String },
}

impl ChatMessage {
    pub fn role(&self) -> &str {
        match self {
            ChatMessage::System { .. } => "system",
            ChatMessage::User { .. } => "user",
            ChatMessage::Assistant { .. } => "assistant",
        }
    }

    pub fn content(&self) -> &str {
        match self {
            ChatMessage::System { content } => content,
            ChatMessage::User { content } => content,
            ChatMessage::Assistant { content } => content,
        }
    }
}

pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}

pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

pub struct ChatStreamChunk {
    pub delta: String,
    pub index: u32,
    pub finish_reason: Option<String>,
}
```

### Provider Configuration

```rust
#[derive(Clone)]
pub struct ProviderConfig {
    pub api_key: Option<String>,      // None for Ollama (local)
    pub api_base: Option<String>,    // Custom endpoint
    pub organization: Option<String>, // OpenAI org
    pub max_retries: u32,
    pub timeout: Duration,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            organization: None,
            max_retries: 3,
            timeout: Duration::from_secs(60),
        }
    }
}

pub struct AnthropicConfig {
    pub api_key: String,
    pub api_base: Option<String>, // defaults to https://api.anthropic.com
    pub max_retries: u32,
    pub timeout: Duration,
}

pub struct OpenAIConfig {
    pub api_key: String,
    pub organization: Option<String>,
    pub api_base: Option<String>,
    pub max_retries: u32,
    pub timeout: Duration,
}

pub struct OllamaConfig {
    pub api_base: String, // e.g., "http://localhost:11434"
    pub model: String,    // default model
}
```

### Provider Implementations

```rust
// lazyjob-llm/src/providers/anthropic.rs
pub struct AnthropicProvider {
    client: reqwest::Client,
    config: AnthropicConfig,
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn provider_name(&self) -> &str { "anthropic" }
    fn model_name(&self) -> &str { &self.config.model }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Convert messages to Anthropic format
        // POST to /v1/messages with anthropic-version header
        // Handle non-streaming response
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>)
        -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>, LLMError> {
        // POST to /v1/messages with stream: true
        // Parse SSE event stream
        // Map to ChatStreamChunk
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError> {
        // Anthropic doesn't have embeddings API (as of 2024)
        // Return error or use separate embedding provider
        Err(LLMError::Other("Embeddings not supported by Anthropic".into()))
    }

    fn context_length(&self) -> u32 {
        match self.config.model.as_str() {
            "claude-3-5-sonnet-20241022" => 200_000,
            "claude-3-opus-20240229" => 200_000,
            "claude-3-haiku-20240307" => 200_000,
            _ => 200_000,
        }
    }
}

// lazyjob-llm/src/providers/openai.rs
pub struct OpenAIProvider {
    client: Client, // from async-openai
    model: String,
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn provider_name(&self) -> &str { "openai" }
    fn model_name(&self) -> &str { &self.model }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Use client.chat().create()
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>)
        -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>, LLMError> {
        // Use client.chat().create_stream()
        // Map SSE chunks to ChatStreamChunk
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError> {
        // Use client.embeddings().create()
    }

    fn context_length(&self) -> u32 {
        match self.model.as_str() {
            "gpt-4o" => 128_000,
            "gpt-4-turbo" => 128_000,
            "gpt-4" => 8_192,
            "gpt-3.5-turbo" => 16_385,
            _ => 8_192,
        }
    }
}

// lazyjob-llm/src/providers/ollama.rs
pub struct OllamaProvider {
    ollama: Ollama,
    model: String,
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    fn provider_name(&self) -> &str { "ollama" }
    fn model_name(&self) -> &str { &self.model }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        // Convert to Ollama format
        // Use ollama.send_chat_messages_with_history()
    }

    async fn chat_stream(&self, messages: Vec<ChatMessage>)
        -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>, LLMError> {
        // Use ollama.generate_stream()
        // Map to ChatStreamChunk
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError> {
        // Use ollama.generate_embeddings()
    }

    fn context_length(&self) -> u32 {
        // Ollama handles context internally, but we need a reasonable default
        4096
    }
}
```

### Provider Registry

```rust
// lazyjob-llm/src/registry.rs
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LLMProvider>>,
    default: Option<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default: None,
        }
    }

    pub fn add<P: LLMProvider + 'static>(&mut self, name: String, provider: P) {
        self.providers.insert(name, Arc::new(provider));
        if self.default.is_none() {
            self.default = Some(name.clone());
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LLMProvider>> {
        self.providers.get(name).cloned()
    }

    pub fn default(&self) -> Option<Arc<dyn LLMProvider>> {
        self.default.as_ref().and_then(|n| self.get(n))
    }

    pub fn all(&self) -> Vec<(String, Arc<dyn LLMProvider>)> {
        self.providers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}
```

### Builder Pattern for Setup

```rust
// lazyjob-llm/src/builder.rs
pub struct LLMBuilder {
    anthropic_key: Option<String>,
    openai_key: Option<String>,
    ollama_url: Option<String>,
    default_model: Option<String>,
}

impl LLMBuilder {
    pub fn new() -> Self {
        Self {
            anthropic_key: None,
            openai_key: None,
            ollama_url: None,
            default_model: None,
        }
    }

    pub fn with_anthropic(mut self, api_key: String) -> Self {
        self.anthropic_key = Some(api_key);
        self
    }

    pub fn with_openai(mut self, api_key: String) -> Self {
        self.openai_key = Some(api_key);
        self
    }

    pub fn with_ollama(mut self, url: String) -> Self {
        self.ollama_url = Some(url);
        self
    }

    pub fn default_model(mut self, model: &str) -> Self {
        self.default_model = Some(model.to_string());
        self
    }

    pub fn build(self) -> Result<ProviderRegistry, LLMError> {
        let mut registry = ProviderRegistry::new();

        if let Some(key) = self.anthropic_key {
            let config = AnthropicConfig {
                api_key: key,
                api_base: None,
                max_retries: 3,
                timeout: Duration::from_secs(120), // Anthropic is slower
            };
            let model = self.default_model.clone()
                .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string());
            registry.add("anthropic".to_string(), AnthropicProvider::new(config, model));
        }

        if let Some(key) = self.openai_key {
            let config = OpenAIConfig {
                api_key: key,
                organization: None,
                api_base: None,
                max_retries: 3,
                timeout: Duration::from_secs(60),
            };
            let model = self.default_model.clone()
                .unwrap_or_else(|| "gpt-4o".to_string());
            registry.add("openai".to_string(), OpenAIProvider::new(config, model));
        }

        if let Some(url) = self.ollama_url {
            let model = self.default_model.clone()
                .unwrap_or_else(|| "llama3.2".to_string());
            registry.add("ollama".to_string(), OllamaProvider::new(url, model));
        }

        Ok(registry)
    }
}
```

### Streaming in TUI Context

The streaming API is critical for UX. In the TUI, streaming responses should:
1. Display incrementally as chunks arrive
2. Allow user to interrupt (Ctrl+C)
3. Show spinner/indicator while streaming
4. Handle errors gracefully mid-stream

```rust
// In TUI event loop
async fn handle_streaming_response(
    &mut self,
    stream: Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>,
) -> Result<String, LLMError> {
    let mut full_response = String::new();
    let mut pending = tokio::time::timeout(Duration::from_secs(120), stream);

    loop {
        tokio::select! {
            chunk = pending => {
                match chunk {
                    Ok(Some(Ok(chunk))) => {
                        // Render incrementally to TUI
                        self.update_streaming_text(&chunk.delta);
                        full_response.push_str(&chunk.delta);
                        if chunk.finish_reason.is_some() {
                            break;
                        }
                    }
                    Ok(Some(Err(e))) => return Err(e),
                    Ok(None) => break,
                    Err(_) => return Err(LLMError::Other("Streaming timeout".into())),
                }
            }
            _ = self.cancel_token.cancelled() => {
                // User pressed Ctrl+C
                return Err(LLMError::Other("Interrupted by user".into()));
            }
        }
    }
    Ok(full_response)
}
```

---

## API Surface

### lazyjob-llm Public API

```rust
// Main exports
pub use provider::{LLMProvider, LLMError, ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage};
pub use registry::ProviderRegistry;
pub use builder::LLMBuilder;
pub mod providers;
```

### Module Hierarchy

```
lazyjob-llm/
├── lib.rs                 # Public API re-exports
├── error.rs              # LLMError type
├── message.rs            # ChatMessage, ChatResponse types
├── provider.rs           # LLMProvider trait
├── registry.rs           # ProviderRegistry
├── builder.rs            # LLMBuilder
├── prompts/              # Prompt templates
│   ├── mod.rs
│   ├── job_discovery.rs
│   ├── company_research.rs
│   ├── resume_tailoring.rs
│   └── cover_letter.rs
└── providers/
    ├── mod.rs
    ├── anthropic.rs
    ├── openai.rs
    └── ollama.rs
```

---

## Failure Modes

1. **API Key Invalid/Missing**: Return `LLMError::Auth` with guidance on setting up API keys
2. **Rate Limiting**: Exponential backoff retry, configurable via `max_retries`
3. **Network Timeout**: Return `LLMError::Network`, user can retry
4. **Model Not Found**: Return `LLMError::ModelNotFound`, suggest available models
5. **Context Length Exceeded**: Return `LLMError::ContextLengthExceeded`, truncate or summarize input
6. **Streaming Interruption**: Handle gracefully, return partial response
7. **Provider Unavailable**: Try fallback provider if configured, else return error

---

## Open Questions

1. **Embedding Provider Strategy**: Anthropic doesn't offer embeddings. Should we always use OpenAI for embeddings, or use Ollama locally?
2. **Cost Tracking**: Should we maintain per-provider cost tracking? LiteLLM does this normalization.
3. **Caching**: Should we cache frequent queries (e.g., company research)? Cache key would be hash of messages.
4. **Rate Limit per Provider**: Different providers have different rate limits. Should we implement a global rate limiter?
5. **Batching**: Should we support batch API calls (e.g., embedding many job descriptions at once)?

---

## Dependencies

```toml
# lazyjob-llm/Cargo.toml
[dependencies]
async-trait = "0.1"           # async fn in traits
reqwest = { version = "0.12", features = ["json", "stream"] }
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
serde = { version = "1", features = ["derive"] }
thiserror = "1"
anyhow = "1"
tracing = "0.1"

# Provider-specific
async-openai = "0.34"         # OpenAI SDK
ollama-rs = "0.3"             # Ollama SDK

[dev-dependencies]
mockall = "0.12"
```

---

## Sources

- [async-openai Documentation](https://docs.rs/async-openai/0.34.0/async_openai/)
- [async-openai GitHub](https://github.com/64bit/async-openai)
- [ollama-rs GitHub](https://github.com/pepperoni21/ollama-rs)
- [Anthropic API Documentation](https://docs.anthropic.com/)
- [OpenAI API Documentation](https://platform.openai.com/docs/api-reference)
- [Ollama API Documentation](https://github.com/ollama/ollama/blob/main/api/README.md)
- [LiteLLM Multi-Provider Pattern](https://github.com/BerriAI/litellm)
