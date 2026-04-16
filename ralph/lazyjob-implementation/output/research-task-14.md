# Research: Task 14 — llm-provider-traits

## Task Summary
Define the core LLM abstraction types and traits in `lazyjob-llm`:
- `LlmProvider` trait: `async complete(messages, opts) -> Result<LlmResponse>`
- `EmbeddingProvider` trait: `async embed(text) -> Result<Vec<f32>>`
- `ChatMessage` struct (System/User/Assistant variants)
- `CompletionOptions` struct (model, temperature, max_tokens, stream flag)
- `LlmResponse` struct (content, usage: TokenUsage)
- `TokenUsage` struct (prompt_tokens, completion_tokens, total_tokens)
- `MockLlmProvider` returning canned responses
- Trait object tests confirming `dyn LlmProvider` works

## Existing Codebase State

### lazyjob-llm crate
- Location: `crates/lazyjob-llm/`
- `src/lib.rs`: only has `pub fn version() -> &'static str`
- `Cargo.toml`: has lazyjob-core, thiserror, anyhow, serde, serde_json, tokio

### Workspace
- Root `Cargo.toml`: members all in `crates/`
- No `async-trait` in workspace deps yet

## Key Design Decisions

### Async fn in Traits + Dynamic Dispatch
In Rust 2024/1.75+, `async fn` in traits is stabilized, but ONLY for static dispatch.
For `dyn LlmProvider`, we need one of:
1. `async_trait` macro (most proven, boxes futures)
2. Explicit `-> Pin<Box<dyn Future + Send>>` return types
3. `dynosaur` crate (newer approach)

**Decision**: Use `async_trait` macro — well-proven, correct, compatible with Rust 2024 edition.

### Module Layout
Following spec exactly:
```
lazyjob-llm/src/
├── lib.rs         — re-exports
├── error.rs       — LlmError
├── message.rs     — ChatMessage, CompletionOptions, LlmResponse, TokenUsage
├── provider.rs    — LlmProvider + EmbeddingProvider traits
└── mock.rs        — MockLlmProvider + MockEmbeddingProvider
```

### Error Type
`LlmError` thiserror enum with variants:
- `Api(String)` — HTTP 4xx/5xx errors
- `Auth(String)` — invalid API key
- `RateLimit` — 429 Too Many Requests
- `Network(String)` — connection/timeout errors
- `ModelNotFound(String)` — unknown model identifier
- `ContextLengthExceeded { max: usize, actual: usize }`
- `Stream(String)` — SSE/streaming errors
- `NotSupported(String)` — e.g. embed() called on Anthropic provider

### ChatMessage
Enum with 3 variants (not a struct with role field) — provides type safety:
```rust
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(String),
}
```
Helper methods: `role() -> &str`, `content() -> &str`

### CompletionOptions
All fields optional with defaults:
```rust
pub struct CompletionOptions {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
}
```

### LlmResponse
```rust
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub stop_reason: Option<String>,
}
```

### MockLlmProvider
Stores a preset `LlmResponse` and returns a clone on each `complete()` call.
`MockEmbeddingProvider` stores `Vec<f32>` and returns a clone on each `embed()` call.

## Dependencies Needed
- `async-trait = "0.1"` — add to workspace deps + lazyjob-llm deps

## Learning Test Plan
- `async_trait_dyn_dispatch` — proves `Box<dyn LlmProvider>` calling `.complete()` returns the expected response. This verifies the async_trait macro enables dynamic dispatch correctly.
