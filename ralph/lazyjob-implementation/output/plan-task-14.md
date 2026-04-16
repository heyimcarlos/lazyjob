# Plan: Task 14 — llm-provider-traits

## Files to Create/Modify

### Modify
1. `Cargo.toml` (workspace root) — add `async-trait = "0.1"` to `[workspace.dependencies]`
2. `crates/lazyjob-llm/Cargo.toml` — add `async-trait = { workspace = true }`
3. `crates/lazyjob-llm/src/lib.rs` — rewrite with module declarations + re-exports

### Create
4. `crates/lazyjob-llm/src/error.rs` — `LlmError` enum, `Result<T>` alias
5. `crates/lazyjob-llm/src/message.rs` — `ChatMessage`, `CompletionOptions`, `LlmResponse`, `TokenUsage`
6. `crates/lazyjob-llm/src/provider.rs` — `LlmProvider` trait, `EmbeddingProvider` trait
7. `crates/lazyjob-llm/src/mock.rs` — `MockLlmProvider`, `MockEmbeddingProvider`

## Types and Functions

### error.rs
```rust
pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    Api(String),
    Auth(String),
    RateLimit,
    Network(String),
    ModelNotFound(String),
    ContextLengthExceeded { max: usize, actual: usize },
    Stream(String),
    NotSupported(String),
}
```

### message.rs
```rust
pub enum ChatMessage { System(String), User(String), Assistant(String) }
impl ChatMessage { pub fn role(&self) -> &str; pub fn content(&self) -> &str; }

pub struct CompletionOptions { model, temperature, max_tokens, stream }
impl Default for CompletionOptions { ... }

pub struct TokenUsage { pub prompt_tokens: u32, pub completion_tokens: u32, pub total_tokens: u32 }
impl TokenUsage { pub fn new(prompt, completion) -> Self }

pub struct LlmResponse { pub content: String, pub model: String, pub usage: TokenUsage, pub stop_reason: Option<String> }
```

### provider.rs
```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    async fn complete(&self, messages: Vec<ChatMessage>, opts: CompletionOptions) -> Result<LlmResponse>;
}

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}
```

### mock.rs
```rust
pub struct MockLlmProvider { response: LlmResponse }
impl MockLlmProvider { pub fn new(response: LlmResponse) -> Self; pub fn with_content(content: &str) -> Self; }
// impl LlmProvider

pub struct MockEmbeddingProvider { embedding: Vec<f32> }
impl MockEmbeddingProvider { pub fn new(embedding: Vec<f32>) -> Self; }
// impl EmbeddingProvider
```

## Tests

### Learning tests (in provider.rs or mock.rs)
- `async_trait_dyn_dispatch` — creates `Box<dyn LlmProvider>` from MockLlmProvider, calls `.complete()` via dyn dispatch, asserts response content matches. Proves async_trait enables dyn dispatch.

### Unit tests (in message.rs)
- `chat_message_role_and_content` — tests all 3 ChatMessage variants' role() and content()
- `completion_options_default` — verifies Default produces expected values (temp=0.7, max_tokens=4096, stream=false)
- `token_usage_total_is_sum` — verifies total = prompt + completion
- `llm_response_construction` — builds LlmResponse, checks fields

### Unit tests (in error.rs)
- `llm_error_display` — spot-checks Display output of a few variants

### Unit tests (in mock.rs)
- `mock_provider_returns_canned_response` — calls complete(), checks content
- `mock_provider_with_content_helper` — tests with_content() constructor
- `mock_embedding_returns_canned_vec` — calls embed(), checks returned vector

## Migrations
None required — this task has no DB changes.
