# Spec: LLM Provider Abstraction

**JTBD**: Let AI handle tedious job search work autonomously while I focus on high-signal decisions
**Topic**: A provider-agnostic trait and three implementations for all LLM calls in ralph workers
**Domain**: agentic

---

## What

`lazyjob-llm` is a dedicated crate that defines the `LlmProvider` trait and implements it for three backends: Anthropic (`claude-sonnet-4-6`), OpenAI (`gpt-4o`), and Ollama (local models, default `llama3.2`). The trait covers chat completions (blocking and SSE streaming), embeddings, and token usage tracking. All ralph workers receive a `Arc<dyn LlmProvider>` and call through this interface; they never reference provider-specific types. Embeddings use a separate `EmbeddingProvider` sub-trait because Anthropic does not offer embeddings — the two concerns must be independently configurable.

## Why

LazyJob's key value proposition for Audience 4 (Tool Author / SaaS Operator) is JTBD D-2: offer premium AI features without requiring users to manage API keys. The loom pattern (server-side proxy + provider-agnostic trait) is the architecture that enables this: locally, users bring their own keys; in SaaS mode, the proxy routes to the cheapest capable provider. Without a clean trait boundary, every provider change would require editing all seven loop types. With the trait, swapping Anthropic for a new model is a one-line config change.

The offline-first constraint also requires Ollama: users without API keys or internet access must be able to run at least some features (e.g., embedding-based job scoring) locally. The trait lets Ollama serve as a graceful degradation path.

## How

### Crate layout: `lazyjob-llm`

```
lazyjob-llm/
├── lib.rs              # re-exports: LlmProvider, EmbeddingProvider, LlmError, types
├── error.rs
├── types.rs            # ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage
├── provider.rs         # LlmProvider + EmbeddingProvider traits
├── registry.rs         # ProviderRegistry (keyed by name string)
├── builder.rs          # LlmBuilder (from lazyjob.toml config)
├── cost.rs             # Per-provider cost-per-token constants, TokenBudget
└── providers/
    ├── mod.rs
    ├── anthropic.rs    # AnthropicProvider (reqwest, SSE parsing)
    ├── openai.rs       # OpenAIProvider (async-openai crate)
    └── ollama.rs       # OllamaProvider (ollama-rs crate)
```

### Core trait

```rust
// lazyjob-llm/src/provider.rs

use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn context_length(&self) -> u32;

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<ChatResponse, LlmError>;

    async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LlmError>> + Send>>, LlmError>;

    /// Convenience: non-streaming single-message completion.
    async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let messages = vec![ChatMessage::user(prompt.to_string())];
        self.chat(messages).await.map(|r| r.content)
    }
}

/// Separate sub-trait for embedding-capable providers.
/// Only OpenAI (text-embedding-3-small) and Ollama (nomic-embed-text) implement this.
/// Anthropic does NOT implement EmbeddingProvider.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError>;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError>;
    fn embedding_dimensions(&self) -> usize;
}
```

### Key message types

```rust
// lazyjob-llm/src/types.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole { System, User, Assistant }

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self { ... }
    pub fn user(content: impl Into<String>) -> Self { ... }
    pub fn assistant(content: impl Into<String>) -> Self { ... }
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatStreamChunk {
    pub delta: String,
    pub finish_reason: Option<String>,
}
```

### Error type

```rust
// lazyjob-llm/src/error.rs

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("API authentication failed: {0}")]
    Auth(String),
    #[error("Rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Context length exceeded (max {max_tokens} tokens)")]
    ContextLengthExceeded { max_tokens: u32 },
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Streaming error: {0}")]
    Stream(String),
    #[error("Provider unavailable: {0}")]
    Unavailable(String),
    #[error("Embeddings not supported by this provider")]
    EmbeddingsNotSupported,
    #[error("Other LLM error: {0}")]
    Other(String),
}
```

### Provider implementations (key decisions)

**AnthropicProvider** (`lazyjob-llm/src/providers/anthropic.rs`):
- HTTP client: `reqwest` with custom SSE parsing (no Anthropic Rust SDK exists as of 2026)
- SSE parsing: reads `event: content_block_delta` events, extracts `delta.text` from JSON body
- Required header: `anthropic-version: 2023-06-01`, `x-api-key: <key>`
- Default model: `claude-sonnet-4-6`
- Does NOT implement `EmbeddingProvider` — return `LlmError::EmbeddingsNotSupported`
- Retry logic: 3 retries on `RateLimited` with exponential backoff (base 2s)
- Context length: 200,000 tokens for all current Claude models

**OpenAIProvider** (`lazyjob-llm/src/providers/openai.rs`):
- Uses `async-openai = "0.34"` crate
- Streaming: `client.chat().create_stream(request)` → map chunks to `ChatStreamChunk`
- Implements `EmbeddingProvider` using `text-embedding-3-small` (1536 dims)
- Default model: `gpt-4o`
- Context length: 128,000 for gpt-4o

**OllamaProvider** (`lazyjob-llm/src/providers/ollama.rs`):
- Uses `ollama-rs = "0.3"` crate
- Default chat model: `llama3.2`
- Implements `EmbeddingProvider` using `nomic-embed-text` (768 dims, used for offline job matching)
- Assumes `http://localhost:11434` unless overridden in config
- No auth; errors if Ollama is not running map to `LlmError::Unavailable`
- Context length: conservative 4,096 (Ollama handles internally, but we must not over-fill)

### Provider registry and builder

```rust
// lazyjob-llm/src/registry.rs
pub struct ProviderRegistry {
    chat_providers: HashMap<String, Arc<dyn LlmProvider>>,
    embedding_providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default_chat: Option<String>,
    default_embedding: Option<String>,
}

impl ProviderRegistry {
    pub fn default_chat(&self) -> Option<Arc<dyn LlmProvider>>;
    pub fn default_embedding(&self) -> Option<Arc<dyn EmbeddingProvider>>;
    pub fn get_chat(&self, name: &str) -> Option<Arc<dyn LlmProvider>>;
    pub fn get_embedding(&self, name: &str) -> Option<Arc<dyn EmbeddingProvider>>;
}

// lazyjob-llm/src/builder.rs
pub struct LlmBuilder {
    anthropic_key: Option<String>,
    openai_key: Option<String>,
    ollama_url: Option<String>,
    default_chat_provider: Option<String>,   // "anthropic" | "openai" | "ollama"
    default_embedding_provider: Option<String>, // "openai" | "ollama"
}

impl LlmBuilder {
    pub fn from_config(config: &LlmConfig) -> Self;
    pub fn build(self) -> Result<ProviderRegistry, LlmError>;
}
```

`LlmConfig` is read from `lazyjob.toml`:

```toml
[llm]
default_chat_provider = "anthropic"     # or "openai" | "ollama"
default_embedding_provider = "ollama"   # offline-first default

[llm.anthropic]
model = "claude-sonnet-4-6"
# api_key read from OS keyring, not this file

[llm.openai]
model = "gpt-4o"
# api_key read from OS keyring

[llm.ollama]
url = "http://localhost:11434"
chat_model = "llama3.2"
embedding_model = "nomic-embed-text"
```

### Token usage tracking

All `ChatResponse` objects carry `TokenUsage`. The caller (ralph worker) is responsible for accumulating usage and writing to `token_usage_log` table:

```sql
CREATE TABLE IF NOT EXISTS token_usage_log (
    id           TEXT PRIMARY KEY,
    loop_id      TEXT REFERENCES ralph_loop_runs(id),
    provider     TEXT NOT NULL,
    model        TEXT NOT NULL,
    prompt_tokens   INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens    INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd_micro INTEGER,  -- cost in microdollars (integer to avoid float)
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Cost computation lives in `lazyjob-llm/src/cost.rs`:

```rust
pub struct ModelCost {
    pub prompt_per_1k: f64,      // USD per 1K prompt tokens
    pub completion_per_1k: f64,  // USD per 1K completion tokens
}

pub fn estimate_cost(usage: &TokenUsage, provider: &str, model: &str) -> u64 {
    // Returns microdollars (u64) to avoid floating point in DB
}
```

### Streaming in TUI context

When a ralph worker calls `chat_stream()`, it forwards the stream chunks back to the TUI via `WorkerEvent::ResultChunk { summary }` (which contains the partially-accumulated text). The TUI's Ralph panel renders the accumulated text in a scrollable preview pane. Workers must handle `WorkerCommand::Cancel` while mid-stream by dropping the stream future.

### Offline degradation

If `default_chat_provider = "anthropic"` and no API key is configured, the builder falls back to Ollama. The fallback chain:

1. Configured default (from `lazyjob.toml`)
2. Ollama at `localhost:11434` (if running)
3. `LlmError::Unavailable("No LLM provider configured or reachable")`

The TUI displays a banner when running in degraded mode.

## Interface

```rust
// lazyjob-llm public API (lib.rs re-exports)
pub use types::{ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage, MessageRole};
pub use error::LlmError;
pub use provider::{LlmProvider, EmbeddingProvider};
pub use registry::ProviderRegistry;
pub use builder::LlmBuilder;
pub use cost::estimate_cost;
```

## Open Questions

- Should `EmbeddingProvider` support batching natively (embed multiple texts in one HTTP call), or always call `embed()` per-text and let callers batch at a higher level? Batching matters for bulk job scoring.
- For the SaaS proxy (JTBD D-2), the `LlmProvider` trait must route through an HTTP server instead of directly to provider APIs. Should the proxy be a fourth `LlmProvider` impl (e.g., `LoomProxyProvider`), or should the proxy transparently replace the provider at `LlmBuilder::build()` time based on a `[llm.proxy]` config section?
- `token_usage_log.estimated_cost_usd_micro` uses the `cost.rs` pricing table. When prices change, the historical log will be wrong. Should we log the pricing constants used at the time, or accept that historical cost estimates are approximate?

## Implementation Tasks

- [ ] Define `LlmProvider` and `EmbeddingProvider` traits in `lazyjob-llm/src/provider.rs` with `async_trait` + `Send + Sync` bounds
- [ ] Define `ChatMessage`, `ChatResponse`, `ChatStreamChunk`, `TokenUsage` in `lazyjob-llm/src/types.rs` with serde derives
- [ ] Implement `AnthropicProvider` in `lazyjob-llm/src/providers/anthropic.rs` using reqwest with manual SSE parsing, 3-retry backoff, 200K context
- [ ] Implement `OpenAIProvider` in `lazyjob-llm/src/providers/openai.rs` using `async-openai`, including `EmbeddingProvider` with `text-embedding-3-small`
- [ ] Implement `OllamaProvider` in `lazyjob-llm/src/providers/ollama.rs` using `ollama-rs`, including `EmbeddingProvider` with `nomic-embed-text` (768 dims)
- [ ] Implement `ProviderRegistry` and `LlmBuilder` with `from_config()` constructor and Ollama fallback chain
- [ ] Create `token_usage_log` SQLite table DDL and `cost.rs` microdollar cost estimator
- [ ] Write mockall-based `MockLlmProvider` in `[dev-dependencies]` for use across all worker integration tests
