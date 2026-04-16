# LLM Provider Abstraction — Implementation Plan

## Spec Reference
- **Spec file**: `specs/02-llm-provider-abstraction.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
A trait-based abstraction layer enabling LazyJob to use multiple LLM providers (Anthropic, OpenAI, Ollama) interchangeably. The design uses a core `LLMProvider` trait with provider-specific implementations, a `ProviderRegistry` for management, and a builder pattern for configuration. Streaming support is included for real-time TUI feedback.

## Problem Statement
LazyJob needs multi-provider LLM support with consistent APIs for chat, completion, and embeddings. Different providers have different authentication, streaming formats, and capabilities. A unified abstraction enables provider fallback, cost tracking, and testing via mocks.

## Implementation Phases

### Phase 1: Foundation
1. Create `lazyjob-llm` crate structure (`Cargo.toml`, `lib.rs`, module hierarchy)
2. Define `LLMError` enum (thiserror-based, covering API, Auth, RateLimit, Network, ModelNotFound, ContextLengthExceeded, Stream errors)
3. Define message types: `ChatMessage` (System/User/Assistant variants), `ChatResponse`, `TokenUsage`, `ChatStreamChunk`
4. Define `LLMProvider` trait with async methods: `chat()`, `chat_stream()`, `complete()`, `embed()`, `provider_name()`, `model_name()`, `context_length()`
5. Add external dependencies to Cargo.toml

### Phase 2: Provider Implementations
1. Implement `AnthropicProvider` in `providers/anthropic.rs`:
   - REST client via `reqwest` (Anthropic uses `/v1/messages`, not `/chat/completions`)
   - SSE streaming parsing for `message_start`, `content_block_delta`, `message_stop` events
   - Context length mapping per model
2. Implement `OpenAIProvider` in `providers/openai.rs`:
   - Leverage `async-openai` crate for API calls and streaming
   - Embeddings via `client.embeddings().create()`
   - Context length mapping per model
3. Implement `OllamaProvider` in `providers/ollama.rs`:
   - Use `ollama-rs` crate
   - Chat with history, streaming support
   - Local embeddings support
4. Create `providers/mod.rs` re-exporting all providers

### Phase 3: Registry & Builder
1. Implement `ProviderRegistry` in `registry.rs`:
   - HashMap of `String → Arc<dyn LLMProvider>`
   - Methods: `add()`, `get()`, `default()`, `all()`
   - Default provider selection
2. Implement `LLMBuilder` in `builder.rs`:
   - Fluent builder API: `with_anthropic()`, `with_openai()`, `with_ollama()`, `default_model()`
   - Validation that at least one provider is configured
   - Returns `Result<ProviderRegistry, LLMError>`

### Phase 4: Integration & Polish
1. Create `prompts/` module with prompt templates for each Ralph loop type
2. Add `chat_stream()` interruption support (cancel token pattern)
3. TUI streaming integration example in documentation
4. Unit tests for each provider (mock HTTP responses)
5. Integration tests with actual providers (skipped without API keys)

## Data Model
No database changes required. This crate operates in memory only.
- `ChatMessage` enum: System/User/Assistant variants with `role()` and `content()` accessors
- `ChatResponse` struct: content, model, usage, stop_reason
- `TokenUsage` struct: prompt_tokens, completion_tokens, total_tokens
- `ChatStreamChunk` struct: delta, index, finish_reason
- `ProviderConfig` structs: per-provider configuration with sensible defaults

## API Surface

### Public exports from `lazyjob-llm`:
```rust
pub use provider::{LLMProvider, LLMError, ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage};
pub use registry::ProviderRegistry;
pub use builder::LLMBuilder;
pub mod providers;
```

### Module hierarchy:
```
lazyjob-llm/
├── lib.rs                 # Public API re-exports
├── error.rs               # LLMError enum
├── message.rs             # ChatMessage, ChatResponse, TokenUsage, ChatStreamChunk
├── provider.rs            # LLMProvider trait definition
├── registry.rs           # ProviderRegistry
├── builder.rs            # LLMBuilder
├── prompts/              # Prompt templates per Ralph loop
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

## Key Technical Decisions

1. **Trait-based over enum**: Enables dependency injection, mocking, and extensibility without modifying existing code.
2. **reqwest for Anthropic over dedicated crate**: `async-openai` doesn't support Anthropic. Direct REST via reqwest gives full control over Anthropic's SSE format.
3. **async-trait for async fn in traits**: Required since Rust doesn't have async fn in traits natively yet.
4. **Arc<dyn LLMProvider> in registry**: Allows sharing providers across threads and cloning references cheaply.
5. **Streaming returns Pin<Box<dyn Stream>>**: Required for dynamic stream type resolution per provider.
6. **Token tracking via TokenUsage struct**: Enables cost management even though providers report differently.
7. **Embedding fallback**: Anthropic lacks embeddings API — `embed()` returns an error with guidance to use OpenAI or Ollama instead.

### Alternatives Rejected:
- **Option A (Simple Enum)**: Violates Open/Closed; hard to test/mock; fixed provider set only.
- **Option C (Actor-based)**: Too heavy for this use case; added complexity without benefit.
- **Option D (Request Router)**: Global configuration model doesn't suit LazyJob's need for per-provider options.

## File Structure
```
lazyjob/
├── lazyjob-llm/                  # NEW crate
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── error.rs
│   │   ├── message.rs
│   │   ├── provider.rs
│   │   ├── registry.rs
│   │   ├── builder.rs
│   │   ├── prompts/
│   │   │   ├── mod.rs
│   │   │   ├── job_discovery.rs
│   │   │   ├── company_research.rs
│   │   │   ├── resume_tailoring.rs
│   │   │   └── cover_letter.rs
│   │   └── providers/
│   │       ├── mod.rs
│   │       ├── anthropic.rs
│   │       ├── openai.rs
│   │       └── ollama.rs
└── (lazyjob-core, lazyjob-tui, lazyjob-cli, lazyjob-ralph unchanged)
```

## Dependencies
| Crate | Version | Justification |
|-------|---------|---------------|
| async-trait | 0.1 | Async fn in traits (Rust limitation) |
| reqwest | 0.12 | HTTP client for Anthropic REST API |
| tokio | 1 (full) | Async runtime |
| tokio-stream | 0.1 | Stream utilities for streaming responses |
| serde | 1 | JSON serialization |
| thiserror | 1 | Ergonomic error enum derivation |
| anyhow | 1 | Error propagation in builder |
| tracing | 0.1 | Logging |
| async-openai | 0.34 | OpenAI SDK (chat + embeddings) |
| ollama-rs | 0.3 | Ollama local LLM SDK |

Dev: mockall 0.12 for mock provider testing.

## Dependencies on Other Specs
- **06-ralph-loop-integration.md**: Ralph loops consume LLM providers; this crate must be implemented first or as a parallel track.
- **17-ralph-prompt-templates.md**: Prompt templates are referenced in this crate's `prompts/` module.
- No database or TUI dependencies.

## Testing Strategy

### Unit Tests
- `test_chat_message_role_and_content()`: Verify `ChatMessage` accessors
- `test_llm_error_display()`: Verify `LLMError` Display impl
- `test_provider_registry_get_default()`: Registry behavior
- `test_builder_validation()`: Builder fails if no providers configured
- Provider tests with mock HTTP (wiremock or custom mock server)

### Integration Tests (require API keys)
- `test_anthropic_chat()`: Smoke test against real Anthropic API (skip without ANTHROPIC_API_KEY)
- `test_openai_embeddings()`: Verify embedding output shape
- `test_ollama_streaming()`: Verify streaming chunks

### Mock Provider for Testing
```rust
pub struct MockProvider {
    chat_response: ChatResponse,
}

#[async_trait]
impl LLMProvider for MockProvider {
    fn provider_name(&self) -> &str { "mock" }
    fn model_name(&self) -> &str { "mock-model" }
    async fn chat(&self, _: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        Ok(self.chat_response.clone())
    }
    // ... other methods
}
```

## Open Questions

1. **Embedding provider selection**: When Anthropic is primary provider, which provider should handle embeddings? OpenAI is most reliable; Ollama is local. Decision: allow embedding provider to be specified independently, default to OpenAI if available.

2. **Cost normalization**: Should we convert token counts to dollar costs for comparison? Would require maintaining a pricing table. Defer to later iteration.

3. **Request caching**: Should repeated queries be cached? Key would be hash of messages + model. Defer unless performance issues emerge.

4. **Global rate limiting**: Different providers have different limits. Should we coordinate? First version: per-provider retry logic only.

5. **Batch embeddings**: Should `embed_batch(texts: Vec<&str>)` be added? Useful for job description similarity. Add if profiling shows bottleneck.

## Effort Estimate
**Rough: 3-4 days**

- Day 1: Crate setup, error/message types, trait definition
- Day 2: Anthropic provider (trickiest — custom SSE parsing)
- Day 3: OpenAI + Ollama providers, registry/builder
- Day 4: Tests, prompts module, integration polish

The Anthropic SSE streaming format is the main complexity. OpenAI and Ollama are straightforward with their respective SDKs.
