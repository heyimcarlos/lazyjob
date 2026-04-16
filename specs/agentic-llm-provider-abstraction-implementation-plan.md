# Implementation Plan: Agentic LLM Provider Abstraction

## Status
Draft

## Related Spec
[specs/agentic-llm-provider-abstraction.md](./agentic-llm-provider-abstraction.md)

## Overview

`lazyjob-llm` is the dedicated crate that defines the provider-agnostic LLM interface for
every ralph worker and pipeline stage in LazyJob. It owns two async traits — `LlmProvider`
for chat/completion and `EmbeddingProvider` for vector generation — plus three concrete
implementations (Anthropic, OpenAI, Ollama), a `ProviderRegistry`, an `LlmBuilder` that
reads `lazyjob.toml`, a cost estimator, and a `MockLlmProvider` for testing.

The split between `LlmProvider` and `EmbeddingProvider` is load-bearing: Anthropic's API
has no embeddings endpoint. Keeping the two concerns in separate traits lets callers request
only what they need without tying themselves to a specific provider combination.

This crate has no SQLite or TUI dependency. It is a pure async library that other crates
(`lazyjob-ralph`, `lazyjob-core`) depend on via `Arc<dyn LlmProvider>` injection. The
`token_usage_log` DDL is defined here and applied by `lazyjob-core`'s migration runner.

## Prerequisites

### Must be implemented first
- None — this is a leaf crate with no lazyjob-* dependencies.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
async-trait       = "0.1"
reqwest           = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }
tokio             = { version = "1", features = ["macros", "rt-multi-thread"] }
tokio-stream      = "0.1"
futures-util      = "0.3"
serde             = { version = "1", features = ["derive"] }
serde_json        = "1"
thiserror         = "1"
anyhow            = "1"
tracing           = "0.1"
secrecy           = "0.8"
async-openai      = "0.28"
ollama-rs         = { version = "0.2", default-features = false, features = ["stream"] }
backoff           = { version = "0.4", features = ["tokio"] }
bytes             = "1"
```

In `lazyjob-llm/Cargo.toml`:

```toml
[package]
name = "lazyjob-llm"
version.workspace = true
edition.workspace = true

[dependencies]
async-trait.workspace = true
reqwest.workspace = true
tokio.workspace = true
tokio-stream.workspace = true
futures-util.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
secrecy.workspace = true
async-openai.workspace = true
ollama-rs.workspace = true
backoff.workspace = true
bytes.workspace = true

[dev-dependencies]
tokio    = { workspace = true, features = ["test-util"] }
mockall  = "0.13"
wiremock = "0.6"
```

---

## Architecture

### Crate Placement

Everything lives in `lazyjob-llm`. No other crate is touched in Phase 1 beyond updating
workspace `Cargo.toml`. Downstream crates (`lazyjob-ralph`, `lazyjob-core`) receive
`Arc<dyn LlmProvider>` through dependency injection — they never reference provider-specific
types.

### Module Structure

```
lazyjob-llm/
  src/
    lib.rs            # re-exports public surface
    error.rs          # LlmError (thiserror)
    types.rs          # ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage, MessageRole
    provider.rs       # LlmProvider + EmbeddingProvider traits
    registry.rs       # ProviderRegistry
    builder.rs        # LlmBuilder (reads LlmConfig)
    config.rs         # LlmConfig, AnthropicConfig, OpenAiConfig, OllamaConfig
    cost.rs           # ModelCost, estimate_cost(), PRICING table
    providers/
      mod.rs
      anthropic.rs    # AnthropicProvider
      openai.rs       # OpenAiProvider
      ollama.rs       # OllamaProvider
  tests/
    integration_anthropic.rs
    integration_openai.rs
    integration_ollama.rs
```

### Core Types

```rust
// lazyjob-llm/src/types.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: MessageRole::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: MessageRole::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: MessageRole::Assistant, content: content.into() }
    }
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// Full text of the assistant message.
    pub content: String,
    /// Model identifier echoed from the provider (e.g. "claude-sonnet-4-6").
    pub model: String,
    pub usage: TokenUsage,
    /// Provider stop reason: "end_turn", "max_tokens", "stop_sequence", etc.
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Emitted by chat_stream() on each SSE delta.
#[derive(Debug, Clone)]
pub struct ChatStreamChunk {
    /// Incremental text delta (may be empty on final chunk).
    pub delta: String,
    /// Set on the final chunk: "end_turn" | "max_tokens" | etc.
    pub finish_reason: Option<String>,
}
```

### Trait Definitions

```rust
// lazyjob-llm/src/provider.rs

use std::pin::Pin;
use async_trait::async_trait;
use tokio_stream::Stream;
use crate::{ChatMessage, ChatResponse, ChatStreamChunk, LlmError};

/// Primary trait for chat/completion. All ralph workers use Arc<dyn LlmProvider>.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    /// Maximum context window in tokens.
    fn context_length(&self) -> u32;

    /// Blocking multi-turn chat. Returns the full assistant message.
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<ChatResponse, LlmError>;

    /// Streaming multi-turn chat. Returns a stream of delta chunks.
    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError>;

    /// Convenience: single-message completion (non-streaming).
    async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let messages = vec![ChatMessage::user(prompt.to_string())];
        self.chat(messages).await.map(|r| r.content)
    }
}

/// Separate trait for embedding-capable providers.
/// Anthropic does NOT implement this; OpenAI and Ollama do.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text. Returns a unit-normalized f32 vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError>;

    /// Embed a batch. Concrete impls must use provider-native batch endpoints where
    /// available (OpenAI supports up to 2048 inputs per request).
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError>;

    /// Dimensionality of the embedding vectors (e.g. 1536 for text-embedding-3-small).
    fn embedding_dimensions(&self) -> usize;
}
```

### SQLite Schema

The table is defined in this crate as a migration fragment but applied by `lazyjob-core`'s
migration runner. The DDL is exported from this crate as a `const` string.

```rust
// lazyjob-llm/src/cost.rs
pub const TOKEN_USAGE_LOG_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS token_usage_log (
    id                      TEXT PRIMARY KEY,           -- UUIDv7
    loop_id                 TEXT REFERENCES ralph_loop_runs(id) ON DELETE SET NULL,
    provider                TEXT NOT NULL,              -- "anthropic" | "openai" | "ollama"
    model                   TEXT NOT NULL,
    prompt_tokens           INTEGER NOT NULL DEFAULT 0,
    completion_tokens       INTEGER NOT NULL DEFAULT 0,
    total_tokens            INTEGER NOT NULL DEFAULT 0,
    -- Cost stored as microdollars (integer) to avoid floating-point precision errors.
    estimated_cost_usd_micro INTEGER,
    created_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_token_usage_loop ON token_usage_log(loop_id);
CREATE INDEX IF NOT EXISTS idx_token_usage_created ON token_usage_log(created_at);
"#;
```

---

## Implementation Phases

### Phase 1 — Core Skeleton (MVP)

**Goal**: The crate compiles, all types are defined, and the trait is usable from downstream
crates even before any provider is implemented.

#### Step 1.1 — Crate scaffold

Create `lazyjob-llm/Cargo.toml` and `lazyjob-llm/src/lib.rs`. Add the crate to the
workspace `Cargo.toml` `[workspace.members]`.

```toml
# lazyjob-llm/Cargo.toml (abbreviated)
[package]
name    = "lazyjob-llm"
version = "0.1.0"
edition = "2024"
```

`lib.rs` re-exports:

```rust
// lazyjob-llm/src/lib.rs
pub mod config;
pub mod cost;
pub mod error;
pub mod provider;
pub mod providers;
pub mod registry;
pub mod builder;
pub mod types;

pub use error::LlmError;
pub use provider::{EmbeddingProvider, LlmProvider};
pub use registry::ProviderRegistry;
pub use builder::LlmBuilder;
pub use types::{ChatMessage, ChatResponse, ChatStreamChunk, MessageRole, TokenUsage};
pub use cost::estimate_cost;

pub type LlmResult<T> = Result<T, LlmError>;
```

**Verification**: `cargo build -p lazyjob-llm` compiles without errors.

#### Step 1.2 — Error type

```rust
// lazyjob-llm/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("API authentication failed: {0}")]
    Auth(String),

    #[error("rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("context length exceeded (max {max_tokens} tokens)")]
    ContextLengthExceeded { max_tokens: u32 },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("streaming error: {0}")]
    Stream(String),

    #[error("provider unavailable: {0}")]
    Unavailable(String),

    #[error("embeddings not supported by this provider")]
    EmbeddingsNotSupported,

    #[error("JSON deserialization failed: {0}")]
    Deserialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
```

#### Step 1.3 — Types (`types.rs`)

Implement the full `ChatMessage`, `ChatResponse`, `ChatStreamChunk`, `TokenUsage` structs
shown above plus their `impl` blocks (convenience constructors). All types derive `Debug`,
`Clone`, `Serialize`, `Deserialize`.

#### Step 1.4 — Config types

```rust
// lazyjob-llm/src/config.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LlmConfig {
    pub default_chat_provider: Option<String>,       // "anthropic" | "openai" | "ollama"
    pub default_embedding_provider: Option<String>,  // "openai" | "ollama"
    pub anthropic: Option<AnthropicConfig>,
    pub openai: Option<OpenAiConfig>,
    pub ollama: Option<OllamaConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_anthropic_model")]
    pub model: String,
    /// If Some, overrides keyring lookup (for CI/testing only — not stored in config file).
    #[serde(skip)]
    pub api_key_override: Option<String>,
}

fn default_anthropic_model() -> String { "claude-sonnet-4-6".to_string() }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_openai_model")]
    pub model: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(skip)]
    pub api_key_override: Option<String>,
}

fn default_openai_model()    -> String { "gpt-4o".to_string() }
fn default_embedding_model() -> String { "text-embedding-3-small".to_string() }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_ollama_chat_model")]
    pub chat_model: String,
    #[serde(default = "default_ollama_embed_model")]
    pub embedding_model: String,
}

fn default_ollama_url()        -> String { "http://localhost:11434".to_string() }
fn default_ollama_chat_model() -> String { "llama3.2".to_string() }
fn default_ollama_embed_model()-> String { "nomic-embed-text".to_string() }
```

**Verification**: `cargo test -p lazyjob-llm` (unit tests on config defaults pass).

---

### Phase 2 — Anthropic Provider

**Goal**: `AnthropicProvider` passes non-streaming and streaming unit tests backed by a
`wiremock` mock server.

#### Step 2.1 — Provider struct

```rust
// lazyjob-llm/src/providers/anthropic.rs

use secrecy::{ExposeSecret, Secret};
use reqwest::Client;

pub struct AnthropicProvider {
    client: Client,
    api_key: Secret<String>,
    model: String,
    base_url: String,   // injectable for tests: "https://api.anthropic.com/v1"
}

impl AnthropicProvider {
    pub fn new(api_key: Secret<String>, model: impl Into<String>) -> Self {
        Self::with_base_url(api_key, model, "https://api.anthropic.com/v1")
    }

    /// Test constructor — allows pointing at a wiremock server.
    pub fn with_base_url(
        api_key: Secret<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let client = Client::builder()
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("anthropic-version",
                    "2023-06-01".parse().unwrap());
                h
            })
            .build()
            .expect("reqwest client");
        Self {
            client,
            api_key,
            model: model.into(),
            base_url: base_url.into(),
        }
    }
}
```

#### Step 2.2 — Non-streaming `chat()`

Build the request body from `Vec<ChatMessage>`. Separate `system` messages from the
`messages` array (Anthropic requires this):

```rust
#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn provider_name(&self) -> &str { "anthropic" }
    fn model_name(&self) -> &str { &self.model }
    fn context_length(&self) -> u32 { 200_000 }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LlmError> {
        let system: String = messages.iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let turns: Vec<_> = messages.into_iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            }))
            .collect();

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": turns,
        });

        let resp = self.client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", self.api_key.expose_secret())
            .json(&body)
            .send()
            .await?;

        match resp.status().as_u16() {
            401 => return Err(LlmError::Auth("invalid API key".into())),
            429 => {
                let retry = resp.headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30);
                return Err(LlmError::RateLimited { retry_after_secs: retry });
            }
            200 => {}
            code => return Err(LlmError::Other(format!("HTTP {code}"))),
        }

        let json: serde_json::Value = resp.json().await?;
        Self::parse_response(json)
    }
    // ...
}
```

`parse_response` extracts the content array's first `text` block and the usage fields.

#### Step 2.3 — SSE streaming `chat_stream()`

Anthropic SSE events follow the pattern:

```
event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}
```

Implementation uses `reqwest` byte streaming with manual line parsing:

```rust
async fn chat_stream(
    &self,
    messages: Vec<ChatMessage>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError> {
    let body = self.build_request_body(&messages, true);
    let response = self.client
        .post(format!("{}/messages", self.base_url))
        .header("x-api-key", self.api_key.expose_secret())
        .json(&body)
        .send()
        .await?;

    let byte_stream = response.bytes_stream();
    let text_stream = futures_util::stream::try_unfold(
        (byte_stream, String::new()),
        |(mut stream, mut buf)| async move {
            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|e| LlmError::Network(e))?;
                buf.push_str(&String::from_utf8_lossy(&bytes));
                if let Some(chunk) = parse_sse_chunk(&mut buf)? {
                    return Ok(Some((chunk, (stream, buf))));
                }
            }
            Ok(None)
        }
    );

    Ok(Box::pin(text_stream))
}
```

`parse_sse_chunk` consumes lines from `buf`, looking for `data:` lines after
`event: content_block_delta`, returning `ChatStreamChunk` or `None` if no full event yet.

#### Step 2.4 — Retry with exponential backoff

Wrap `chat()` with the `backoff` crate for `RateLimited` errors:

```rust
use backoff::{ExponentialBackoff, future::retry};

// Retry up to 3 attempts on RateLimited; all other errors are permanent.
async fn chat_with_retry(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LlmError> {
    let backoff = ExponentialBackoff {
        max_elapsed_time: Some(std::time::Duration::from_secs(60)),
        initial_interval: std::time::Duration::from_secs(2),
        multiplier: 2.0,
        max_interval: std::time::Duration::from_secs(16),
        ..Default::default()
    };
    retry(backoff, || async {
        self.chat_inner(messages.clone()).await.map_err(|e| match e {
            LlmError::RateLimited { .. } => backoff::Error::transient(e),
            other => backoff::Error::permanent(other),
        })
    }).await
}
```

**Verification**: `cargo test -p lazyjob-llm anthropic` — tests against wiremock stubs for
200, 401, 429, and streaming responses all pass.

---

### Phase 3 — OpenAI Provider

**Goal**: `OpenAiProvider` implements `LlmProvider` + `EmbeddingProvider` using `async-openai`.

#### Step 3.1 — Provider struct

```rust
// lazyjob-llm/src/providers/openai.rs

use async_openai::{Client as OpenAiClient, config::OpenAIConfig};

pub struct OpenAiProvider {
    client: OpenAiClient<OpenAIConfig>,
    model: String,
    embedding_model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, model: impl Into<String>, embedding_model: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: OpenAiClient::with_config(config),
            model: model.into(),
            embedding_model: embedding_model.into(),
        }
    }
}
```

#### Step 3.2 — `LlmProvider` impl

Map `Vec<ChatMessage>` → `async_openai::types::ChatCompletionRequestMessage`, call
`client.chat().create(request).await`, unpack the response:

```rust
use async_openai::types::{
    ChatCompletionRequestMessage,
    ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestAssistantMessageArgs,
    CreateChatCompletionRequestArgs,
};

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn provider_name(&self) -> &str { "openai" }
    fn model_name(&self) -> &str { &self.model }
    fn context_length(&self) -> u32 { 128_000 }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LlmError> {
        let oai_msgs: Vec<ChatCompletionRequestMessage> = messages
            .into_iter()
            .map(oai_message_from)
            .collect::<Result<_, _>>()?;

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(oai_msgs)
            .build()
            .map_err(|e| LlmError::Other(e.to_string()))?;

        let resp = self.client.chat().create(request).await
            .map_err(|e| LlmError::Other(e.to_string()))?;

        let choice = resp.choices.into_iter().next()
            .ok_or_else(|| LlmError::Other("empty choices".into()))?;
        let content = choice.message.content
            .unwrap_or_default();
        let usage = resp.usage.unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: TokenUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
            stop_reason: choice.finish_reason.map(|r| format!("{r:?}")),
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError> {
        use futures_util::StreamExt;
        let oai_msgs: Vec<_> = messages.into_iter().map(oai_message_from).collect::<Result<_,_>>()?;
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(oai_msgs)
            .stream(true)
            .build()
            .map_err(|e| LlmError::Other(e.to_string()))?;

        let mut raw_stream = self.client.chat().create_stream(request).await
            .map_err(|e| LlmError::Other(e.to_string()))?;

        let mapped = async_stream::stream! {
            while let Some(result) = raw_stream.next().await {
                match result {
                    Ok(resp) => {
                        for choice in resp.choices {
                            let delta = choice.delta.content.unwrap_or_default();
                            let finish_reason = choice.finish_reason.map(|r| format!("{r:?}"));
                            yield Ok(ChatStreamChunk { delta, finish_reason });
                        }
                    }
                    Err(e) => yield Err(LlmError::Other(e.to_string())),
                }
            }
        };
        Ok(Box::pin(mapped))
    }
}
```

#### Step 3.3 — `EmbeddingProvider` impl

```rust
use async_openai::types::{CreateEmbeddingRequestArgs, EncodingFormat};

#[async_trait]
impl EmbeddingProvider for OpenAiProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let batch = self.embed_batch(vec![text.to_string()]).await?;
        batch.into_iter().next().ok_or_else(|| LlmError::Other("empty embedding".into()))
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError> {
        let request = CreateEmbeddingRequestArgs::default()
            .model(&self.embedding_model)
            .input(texts)
            .encoding_format(EncodingFormat::Float)
            .build()
            .map_err(|e| LlmError::Other(e.to_string()))?;

        let resp = self.client.embeddings().create(request).await
            .map_err(|e| LlmError::Other(e.to_string()))?;

        Ok(resp.data.into_iter().map(|e| e.embedding).collect())
    }

    fn embedding_dimensions(&self) -> usize { 1536 } // text-embedding-3-small
}
```

**Verification**: Unit test with `mockall` stubs; integration test gated on `OPENAI_API_KEY`
env var.

---

### Phase 4 — Ollama Provider

**Goal**: `OllamaProvider` implements `LlmProvider` + `EmbeddingProvider` using `ollama-rs`.
Errors when the Ollama daemon is not reachable map to `LlmError::Unavailable`.

#### Step 4.1 — Provider struct

```rust
// lazyjob-llm/src/providers/ollama.rs
use ollama_rs::Ollama;

pub struct OllamaProvider {
    client: Ollama,
    chat_model: String,
    embedding_model: String,
}

impl OllamaProvider {
    pub fn new(
        url: impl Into<String>,
        chat_model: impl Into<String>,
        embedding_model: impl Into<String>,
    ) -> Result<Self, LlmError> {
        let url = url.into();
        let parsed = url.parse::<reqwest::Url>()
            .map_err(|e| LlmError::Other(format!("invalid Ollama URL: {e}")))?;
        let host = format!("{}://{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or("localhost"),
        );
        let port = parsed.port().unwrap_or(11434);
        Ok(Self {
            client: Ollama::new(host, port),
            chat_model: chat_model.into(),
            embedding_model: embedding_model.into(),
        })
    }
}
```

#### Step 4.2 — `LlmProvider` impl

```rust
use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage as OllamaChatMsg, MessageRole as OllamaRole};

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn provider_name(&self) -> &str { "ollama" }
    fn model_name(&self) -> &str { &self.chat_model }
    fn context_length(&self) -> u32 { 4_096 }

    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LlmError> {
        let ollama_msgs: Vec<OllamaChatMsg> = messages.iter().map(|m| {
            let role = match m.role {
                MessageRole::System    => OllamaRole::System,
                MessageRole::User      => OllamaRole::User,
                MessageRole::Assistant => OllamaRole::Assistant,
            };
            OllamaChatMsg::new(role, m.content.clone())
        }).collect();

        let request = ChatMessageRequest::new(self.chat_model.clone(), ollama_msgs);
        let resp = self.client.send_chat_messages(request).await
            .map_err(|e| LlmError::Unavailable(e.to_string()))?;

        Ok(ChatResponse {
            content: resp.message.map(|m| m.content).unwrap_or_default(),
            model: self.chat_model.clone(),
            usage: TokenUsage {
                prompt_tokens: resp.prompt_eval_count.unwrap_or(0) as u32,
                completion_tokens: resp.eval_count.unwrap_or(0) as u32,
                total_tokens: (resp.prompt_eval_count.unwrap_or(0)
                    + resp.eval_count.unwrap_or(0)) as u32,
            },
            stop_reason: resp.done_reason,
        })
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError> {
        // ollama-rs v0.2 stream API
        use futures_util::StreamExt;
        let ollama_msgs = messages_to_ollama(messages);
        let request = ChatMessageRequest::new(self.chat_model.clone(), ollama_msgs);
        let raw = self.client.send_chat_messages_stream(request).await
            .map_err(|e| LlmError::Unavailable(e.to_string()))?;

        let mapped = raw.map(|res| res.map(|r| ChatStreamChunk {
            delta: r.message.map(|m| m.content).unwrap_or_default(),
            finish_reason: if r.done { r.done_reason } else { None },
        }).map_err(|e| LlmError::Stream(e.to_string())));

        Ok(Box::pin(mapped))
    }
}
```

#### Step 4.3 — `EmbeddingProvider` impl

```rust
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let req = GenerateEmbeddingsRequest::new(
            self.embedding_model.clone(),
            ollama_rs::generation::embeddings::request::EmbeddingsInput::Single(text.to_string()),
        );
        let resp = self.client.generate_embeddings(req).await
            .map_err(|e| LlmError::Unavailable(e.to_string()))?;
        Ok(resp.embeddings.into_iter().next().unwrap_or_default())
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError> {
        // Ollama does not batch; call sequentially.
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed(&t).await?);
        }
        Ok(results)
    }

    fn embedding_dimensions(&self) -> usize { 768 } // nomic-embed-text
}
```

**Verification**: Integration test gated on Ollama running locally (`OLLAMA_URL` env var or
skip annotation).

---

### Phase 5 — Registry and Builder

**Goal**: `LlmBuilder::from_config(config).build()` constructs a `ProviderRegistry` with
the configured providers, reads API keys from keyring, and falls back to Ollama.

#### Step 5.1 — ProviderRegistry

```rust
// lazyjob-llm/src/registry.rs
use std::collections::HashMap;
use std::sync::Arc;
use crate::{EmbeddingProvider, LlmProvider};

pub struct ProviderRegistry {
    chat: HashMap<String, Arc<dyn LlmProvider>>,
    embedding: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default_chat: Option<String>,
    default_embedding: Option<String>,
}

impl ProviderRegistry {
    pub fn default_chat(&self) -> Option<Arc<dyn LlmProvider>> {
        self.default_chat.as_ref()
            .and_then(|n| self.chat.get(n))
            .cloned()
    }

    pub fn default_embedding(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.default_embedding.as_ref()
            .and_then(|n| self.embedding.get(n))
            .cloned()
    }

    pub fn get_chat(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.chat.get(name).cloned()
    }

    pub fn get_embedding(&self, name: &str) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embedding.get(name).cloned()
    }

    pub fn chat_providers(&self) -> impl Iterator<Item = &str> {
        self.chat.keys().map(String::as_str)
    }

    /// True when no provider is configured (offline/unconfigured state).
    pub fn is_empty(&self) -> bool {
        self.chat.is_empty()
    }
}
```

#### Step 5.2 — LlmBuilder

```rust
// lazyjob-llm/src/builder.rs
use secrecy::Secret;
use crate::{config::LlmConfig, LlmError, providers::*};

pub struct LlmBuilder {
    config: LlmConfig,
}

impl LlmBuilder {
    pub fn from_config(config: LlmConfig) -> Self {
        Self { config }
    }

    /// Build a ProviderRegistry from the config.
    /// API keys are read from the OS keyring unless an override is set.
    /// Falls back to Ollama if the configured default is unreachable.
    pub fn build(self) -> Result<ProviderRegistry, LlmError> {
        use std::collections::HashMap;
        use std::sync::Arc;

        let mut chat: HashMap<String, Arc<dyn crate::LlmProvider>> = HashMap::new();
        let mut embedding: HashMap<String, Arc<dyn crate::EmbeddingProvider>> = HashMap::new();

        // Anthropic
        if let Some(ref anthropic_cfg) = self.config.anthropic {
            let key = anthropic_cfg.api_key_override.clone()
                .or_else(|| keyring_get("anthropic_api_key"));
            if let Some(k) = key {
                let provider = anthropic::AnthropicProvider::new(
                    Secret::new(k),
                    &anthropic_cfg.model,
                );
                chat.insert("anthropic".into(), Arc::new(provider));
                tracing::debug!("registered Anthropic chat provider");
            }
        }

        // OpenAI
        if let Some(ref openai_cfg) = self.config.openai {
            let key = openai_cfg.api_key_override.clone()
                .or_else(|| keyring_get("openai_api_key"));
            if let Some(k) = key {
                let provider = Arc::new(openai::OpenAiProvider::new(
                    &k,
                    &openai_cfg.model,
                    &openai_cfg.embedding_model,
                ));
                chat.insert("openai".into(), provider.clone() as Arc<dyn crate::LlmProvider>);
                embedding.insert("openai".into(), provider as Arc<dyn crate::EmbeddingProvider>);
                tracing::debug!("registered OpenAI chat+embedding provider");
            }
        }

        // Ollama (no auth required)
        if let Some(ref ollama_cfg) = self.config.ollama {
            match ollama::OllamaProvider::new(
                &ollama_cfg.url,
                &ollama_cfg.chat_model,
                &ollama_cfg.embedding_model,
            ) {
                Ok(p) => {
                    let p = Arc::new(p);
                    chat.insert("ollama".into(), p.clone() as Arc<dyn crate::LlmProvider>);
                    embedding.insert("ollama".into(), p as Arc<dyn crate::EmbeddingProvider>);
                    tracing::debug!("registered Ollama provider at {}", ollama_cfg.url);
                }
                Err(e) => tracing::warn!("failed to configure Ollama provider: {e}"),
            }
        }

        // Resolve defaults and apply fallback chain
        let default_chat = self.resolve_default_chat(&chat);
        let default_embedding = self.resolve_default_embedding(&embedding);

        Ok(ProviderRegistry { chat, embedding, default_chat, default_embedding })
    }

    fn resolve_default_chat(
        &self,
        chat: &HashMap<String, Arc<dyn crate::LlmProvider>>,
    ) -> Option<String> {
        // 1. Configured default
        if let Some(ref name) = self.config.default_chat_provider {
            if chat.contains_key(name.as_str()) {
                return Some(name.clone());
            }
            tracing::warn!(
                "configured default_chat_provider '{}' is not available; falling back",
                name
            );
        }
        // 2. Anthropic
        if chat.contains_key("anthropic") { return Some("anthropic".into()); }
        // 3. OpenAI
        if chat.contains_key("openai") { return Some("openai".into()); }
        // 4. Ollama
        if chat.contains_key("ollama") { return Some("ollama".into()); }
        None
    }

    fn resolve_default_embedding(
        &self,
        embedding: &HashMap<String, Arc<dyn crate::EmbeddingProvider>>,
    ) -> Option<String> {
        if let Some(ref name) = self.config.default_embedding_provider {
            if embedding.contains_key(name.as_str()) {
                return Some(name.clone());
            }
        }
        // Prefer OpenAI, fall back to Ollama (offline-capable)
        if embedding.contains_key("openai") { return Some("openai".into()); }
        if embedding.contains_key("ollama") { return Some("ollama".into()); }
        None
    }
}

fn keyring_get(service: &str) -> Option<String> {
    keyring::Entry::new("lazyjob", service)
        .ok()
        .and_then(|e| e.get_password().ok())
}
```

Add `keyring = "2"` to `lazyjob-llm/Cargo.toml` dependencies.

**Verification**: `cargo test -p lazyjob-llm builder` — tests for fallback chain logic all
pass (no real keyring access, uses config overrides).

---

### Phase 6 — Cost Estimator

**Goal**: `estimate_cost()` converts a `TokenUsage` into microdollars without floating-point
drift.

```rust
// lazyjob-llm/src/cost.rs

/// Static pricing table. Update when provider pricing changes.
/// All prices are in microdollars per 1K tokens (1 USD = 1,000,000 microdollars).
static PRICING: &[(&str, &str, u64, u64)] = &[
    // (provider, model, prompt_per_1k_µ$, completion_per_1k_µ$)
    ("anthropic", "claude-sonnet-4-6",         3_000,  15_000),
    ("anthropic", "claude-opus-4-6",           15_000,  75_000),
    ("openai",    "gpt-4o",                    5_000,  15_000),
    ("openai",    "text-embedding-3-small",      100,       0),
    ("ollama",    "*",                              0,       0), // local — no cost
];

pub struct ModelCost {
    pub prompt_per_1k_microdollars: u64,
    pub completion_per_1k_microdollars: u64,
}

pub fn model_cost(provider: &str, model: &str) -> ModelCost {
    PRICING.iter()
        .find(|(p, m, _, _)| *p == provider && (*m == model || *m == "*"))
        .map(|(_, _, prompt, completion)| ModelCost {
            prompt_per_1k_microdollars: *prompt,
            completion_per_1k_microdollars: *completion,
        })
        .unwrap_or(ModelCost {
            prompt_per_1k_microdollars: 0,
            completion_per_1k_microdollars: 0,
        })
}

/// Returns estimated cost in microdollars (integer). Use integer division.
pub fn estimate_cost(usage: &crate::TokenUsage, provider: &str, model: &str) -> u64 {
    let cost = model_cost(provider, model);
    let prompt_µ = (usage.prompt_tokens as u64) * cost.prompt_per_1k_microdollars / 1000;
    let completion_µ = (usage.completion_tokens as u64)
        * cost.completion_per_1k_microdollars / 1000;
    prompt_µ + completion_µ
}
```

Also export the DDL constant (shown in the SQLite Schema section above).

**Verification**: Unit test: `estimate_cost(&usage, "anthropic", "claude-sonnet-4-6")` for
known token counts produces the expected microdollar value.

---

### Phase 7 — Mock Provider (Test Infrastructure)

**Goal**: Provide a `MockLlmProvider` and `MockEmbeddingProvider` in `dev-dependencies` for
all downstream test suites.

```rust
// lazyjob-llm/src/providers/mock.rs  (cfg(test) or feature-gated)
use crate::{ChatMessage, ChatResponse, ChatStreamChunk, LlmError, TokenUsage};
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// Deterministic mock: always returns `response`.
pub struct MockLlmProvider {
    pub response: String,
    pub model: String,
}

#[async_trait]
impl crate::LlmProvider for MockLlmProvider {
    fn provider_name(&self) -> &str { "mock" }
    fn model_name(&self) -> &str { &self.model }
    fn context_length(&self) -> u32 { 100_000 }

    async fn chat(&self, _: Vec<ChatMessage>) -> Result<ChatResponse, LlmError> {
        Ok(ChatResponse {
            content: self.response.clone(),
            model: self.model.clone(),
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
            stop_reason: Some("end_turn".into()),
        })
    }

    async fn chat_stream(
        &self,
        _: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError> {
        let words: Vec<_> = self.response
            .split_whitespace()
            .map(|w| Ok(ChatStreamChunk {
                delta: format!("{w} "),
                finish_reason: None,
            }))
            .collect();
        let last_idx = words.len().saturating_sub(1);
        let chunks: Vec<_> = words.into_iter().enumerate().map(|(i, mut c)| {
            if i == last_idx {
                if let Ok(ref mut chunk) = c {
                    chunk.finish_reason = Some("end_turn".into());
                }
            }
            c
        }).collect();
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

/// Always returns the same embedding vector.
pub struct MockEmbeddingProvider {
    pub dimensions: usize,
}

#[async_trait]
impl crate::EmbeddingProvider for MockEmbeddingProvider {
    async fn embed(&self, _: &str) -> Result<Vec<f32>, LlmError> {
        Ok(vec![0.1_f32; self.dimensions])
    }
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError> {
        Ok(texts.iter().map(|_| vec![0.1_f32; self.dimensions]).collect())
    }
    fn embedding_dimensions(&self) -> usize { self.dimensions }
}
```

Export from `crate::providers::mock` (gated with `#[cfg(any(test, feature = "mock"))]`).

---

### Phase 8 — Streaming TUI Integration Pattern

**Goal**: Document and enforce the pattern for forwarding stream chunks to the TUI without
blocking the async runtime.

Ralph workers that need streaming call `chat_stream()` and forward chunks as
`WorkerEvent::ResultChunk`:

```rust
// lazyjob-ralph/src/worker.rs  (NOT in lazyjob-llm — this is a usage pattern)

use lazyjob_llm::{LlmProvider, ChatMessage};
use futures_util::StreamExt;
use tokio::sync::mpsc;

async fn run_streaming_task(
    provider: Arc<dyn LlmProvider>,
    messages: Vec<ChatMessage>,
    event_tx: mpsc::Sender<WorkerEvent>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<String> {
    let mut stream = provider.chat_stream(messages).await?;
    let mut accumulated = String::new();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(c)) => {
                        accumulated.push_str(&c.delta);
                        let _ = event_tx.send(WorkerEvent::ResultChunk {
                            partial: accumulated.clone(),
                        }).await;
                        if c.finish_reason.is_some() { break; }
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
            _ = &mut cancel_rx => {
                tracing::info!("stream cancelled by TUI");
                break;
            }
        }
    }
    Ok(accumulated)
}
```

This pattern is documented as a required convention — all streaming workers must use
`tokio::select!` over the stream and a cancel receiver so the TUI can abort mid-stream.

---

## Key Crate APIs

| Crate | API | Usage |
|-------|-----|-------|
| `reqwest` | `Client::post(url).header(k,v).json(&body).send().await` | Anthropic HTTP requests |
| `reqwest` | `Response::bytes_stream()` | Anthropic SSE streaming |
| `backoff` | `retry(ExponentialBackoff, || async { ... })` | Rate-limited retry |
| `async-openai` | `Client::chat().create(req).await` | OpenAI chat |
| `async-openai` | `Client::chat().create_stream(req).await` | OpenAI streaming |
| `async-openai` | `Client::embeddings().create(req).await` | OpenAI embeddings |
| `async-openai` | `CreateChatCompletionRequestArgs::default().model().messages().build()` | Request builder |
| `ollama-rs` | `Ollama::new(host, port)` | Ollama client construction |
| `ollama-rs` | `Ollama::send_chat_messages(req).await` | Ollama chat |
| `ollama-rs` | `Ollama::send_chat_messages_stream(req).await` | Ollama streaming |
| `ollama-rs` | `Ollama::generate_embeddings(req).await` | Ollama embeddings |
| `secrecy` | `Secret::new(s)` / `s.expose_secret()` | API key protection |
| `keyring` | `Entry::new("lazyjob", service).get_password()` | OS keychain read |
| `tokio-stream` | `StreamExt::next()` | Stream consumption |
| `futures_util` | `stream::try_unfold(state, closure)` | Custom stream construction |

---

## Error Handling

```rust
// Full error enum (already shown above, repeated here for completeness)
#[derive(Error, Debug)]
pub enum LlmError {
    #[error("API authentication failed: {0}")]
    Auth(String),

    #[error("rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("context length exceeded (max {max_tokens} tokens)")]
    ContextLengthExceeded { max_tokens: u32 },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("streaming error: {0}")]
    Stream(String),

    #[error("provider unavailable: {0}")]
    Unavailable(String),

    #[error("embeddings not supported by this provider")]
    EmbeddingsNotSupported,

    #[error("JSON deserialization failed: {0}")]
    Deserialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
```

All provider errors are mapped to this enum before leaving `lazyjob-llm`. Callers never see
`reqwest::Error` or provider-SDK errors directly. The `#[from] reqwest::Error` on `Network`
is the only exception — allowed because `reqwest::Error` implements `std::error::Error` and
doesn't leak provider-internal state.

---

## Testing Strategy

### Unit Tests (no network, no keyring)

```
lazyjob-llm/src/providers/anthropic.rs
  test_parse_chat_response_success()        — happy-path JSON fixture parsing
  test_parse_rate_limited_response()        — 429 response → RateLimited variant
  test_parse_auth_failure()                 — 401 response → Auth variant
  test_sse_chunk_parsing()                  — single SSE buffer with two events

lazyjob-llm/src/providers/openai.rs
  test_oai_message_mapping()                — ChatMessage → async-openai type mapping

lazyjob-llm/src/cost.rs
  test_estimate_cost_anthropic_sonnet()     — known values → correct microdollar amount
  test_estimate_cost_ollama_zero()          — Ollama always returns 0

lazyjob-llm/src/builder.rs
  test_builder_fallback_to_ollama()         — Anthropic key absent, Ollama resolves
  test_builder_no_providers_empty_registry()
  test_builder_explicit_default_respected()
```

### Integration Tests (require real services)

```
lazyjob-llm/tests/integration_anthropic.rs
  #[tokio::test]
  #[ignore = "requires ANTHROPIC_API_KEY"]
  async fn anthropic_chat_roundtrip() { ... }

  #[tokio::test]
  #[ignore = "requires ANTHROPIC_API_KEY"]
  async fn anthropic_streaming_accumulates_full_response() { ... }

lazyjob-llm/tests/integration_openai.rs
  #[ignore = "requires OPENAI_API_KEY"]
  async fn openai_embed_batch_dimensions() { ... }

lazyjob-llm/tests/integration_ollama.rs
  #[ignore = "requires running Ollama daemon"]
  async fn ollama_nomic_embed_text_dims() { ... }
```

All integration tests check for their required env var at runtime and skip cleanly if absent:

```rust
fn skip_without_key(var: &str) {
    if std::env::var(var).is_err() {
        eprintln!("skipping — {var} not set");
        return;
    }
}
```

### Wiremock-based HTTP Tests

```rust
// lazyjob-llm/src/providers/anthropic.rs  #[cfg(test)]

#[tokio::test]
async fn test_anthropic_chat_with_wiremock() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path, header};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello!"}],
            "model": "claude-sonnet-4-6",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url(
        secrecy::Secret::new("test-key".into()),
        "claude-sonnet-4-6",
        server.uri(),
    );
    let resp = provider.chat(vec![ChatMessage::user("hi")]).await.unwrap();
    assert_eq!(resp.content, "Hello!");
    assert_eq!(resp.usage.prompt_tokens, 10);
}
```

---

## Open Questions

1. **`embed_batch` for Ollama**: Ollama's HTTP API as of v0.2 does not expose a native batch
   embeddings endpoint. The current plan calls `embed()` in a loop. This is acceptable for
   small batches but will be slow for bulk job scoring (500+ jobs). If Ollama adds a batch
   endpoint, `OllamaProvider::embed_batch` should be updated without interface changes.

2. **SaaS proxy provider**: When running in SaaS mode (spec 18), LLM calls should route
   through `lazyjob-api` instead of directly to Anthropic/OpenAI. The cleanest approach is
   a fourth provider impl `LoomProxyProvider` that speaks the same JSON envelope as Anthropic
   but points at `https://api.lazyjob.app/v1/messages`. This defers provider abstraction
   responsibility to the proxy, which internally selects the cheapest capable model. The
   decision on whether to implement this as a new provider or as a transparent URL swap in
   `LlmBuilder` is deferred to the SaaS migration implementation plan.

3. **Historical cost accuracy**: `token_usage_log.estimated_cost_usd_micro` is computed at
   call time using the PRICING constant. When pricing changes, old rows become inaccurate.
   Accept this limitation — the log is informational, not financial. Consider adding a
   `pricing_snapshot` JSON column in a future migration if auditability becomes important.

4. **`async-trait` vs native async fn in traits**: Rust 1.75+ supports `async fn` in traits
   but only for non-dyn uses. Since LazyJob uses `Arc<dyn LlmProvider>`, `async-trait` is
   still required. Remove it only after RPITIT/dyn-async-traits land stably and `dyn` usage
   is broadly supported.

5. **Context-length enforcement**: Should the builder or caller be responsible for checking
   message length against `context_length()`? Currently the spec expects the provider to
   return `ContextLengthExceeded` from the API. Consider adding a pre-flight token count
   using a tokenizer crate (`tiktoken-rs` or `anthropic-tokenizer`) to fail fast before
   making the API call. Deferred — adds build complexity for limited benefit in V1.

---

## Related Specs

- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — consumers of
  `Arc<dyn LlmProvider>` via dependency injection
- [specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) —
  streaming chunks forwarded as `WorkerEvent::ResultChunk`
- [specs/17-ralph-prompt-templates.md](./17-ralph-prompt-templates.md) — templates assembled
  into `Vec<ChatMessage>` before calling `LlmProvider::chat()`
- [specs/18-saas-migration-path.md](./18-saas-migration-path.md) — SaaS proxy provider
  design
- [specs/XX-llm-cost-budget-management.md](./XX-llm-cost-budget-management.md) — reads
  `token_usage_log` table populated by callers using `estimate_cost()`
- [specs/02-llm-provider-abstraction.md](./02-llm-provider-abstraction.md) — earlier spec
  with overlapping content; this plan supersedes it for the agentic domain
