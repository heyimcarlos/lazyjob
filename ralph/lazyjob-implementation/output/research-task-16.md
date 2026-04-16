# Research: Task 16 — llm-openai

## Task Description
Implement `OpenAiProvider` using `async-openai` crate (chat completions + embeddings) and `OllamaProvider` using `ollama-rs` (chat + local embeddings with `nomic-embed-text`). Both read API keys/config from `CredentialManager` where applicable.

## Codebase State

### Existing lazyjob-llm crate
- `src/error.rs` — `LlmError` with Api, Auth, RateLimit, Network, ModelNotFound, ContextLengthExceeded, Stream, NotSupported variants
- `src/message.rs` — `ChatMessage` enum (System/User/Assistant), `CompletionOptions`, `LlmResponse`, `TokenUsage`
- `src/provider.rs` — `LlmProvider` and `EmbeddingProvider` async traits (with `async_trait`)
- `src/mock.rs` — `MockLlmProvider` and `MockEmbeddingProvider`
- `src/providers/anthropic.rs` — Full `AnthropicProvider` with reqwest, SSE streaming, backoff
- `src/providers/mod.rs` — currently re-exports only `AnthropicProvider`
- `src/lib.rs` — re-exports all public types + `AnthropicProvider`

### Workspace dependencies already present
- `async-trait = "0.1"`, `reqwest = "0.12"` (rustls-tls), `secrecy = "0.8"`
- `lazyjob-core` (for `CredentialManager`)

### Missing workspace deps (must add)
- `async-openai = "0.34"` — OpenAI official Rust SDK
- `ollama-rs = "0.3"` — Ollama Rust client

## API Research

### async-openai 0.34

**Client creation with explicit API key:**
```rust
use async_openai::{Client, config::OpenAIConfig};
let config = OpenAIConfig::new().with_api_key("sk-...");
let client: Client<OpenAIConfig> = Client::with_config(config);
```

**Chat completion:**
```rust
use async_openai::types::{
    CreateChatCompletionRequestArgs,
    ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestAssistantMessageArgs,
};

let request = CreateChatCompletionRequestArgs::default()
    .model("gpt-4o-mini")
    .max_tokens(4096u32)
    .messages(api_messages)
    .build()?;

let response = client.chat().create(request).await?;
// Text: response.choices[0].message.content: Option<String>
// Usage: response.usage: Option<CompletionUsage>
//   usage.prompt_tokens: u32
//   usage.completion_tokens: u32
// Model: response.model: String
```

**Embeddings (text-embedding-3-small, 1536 dims):**
```rust
use async_openai::types::CreateEmbeddingRequestArgs;
let request = CreateEmbeddingRequestArgs::default()
    .model("text-embedding-3-small")
    .input(text)
    .build()?;
let response = client.embeddings().create(request).await?;
// response.data[0].embedding: Vec<f32>
```

### ollama-rs 0.3

**Client:**
```rust
use ollama_rs::Ollama;
let ollama = Ollama::new("http://localhost", 11434);  // or Ollama::default()
```

**Chat:**
```rust
use ollama_rs::generation::chat::{ChatMessage as OllamaMsg, request::ChatMessageRequest};
let request = ChatMessageRequest::new(model, messages);
let response = ollama.send_chat_messages(request).await?;
// response.message.content: String
// response.model: String (or Option<String> — use unwrap_or_default)
// response.final_data: Option<ChatMessageFinalResponseData>
//   final_data.prompt_eval_count: u64
//   final_data.eval_count: u64
```

**Embeddings (nomic-embed-text, 768 dims):**
```rust
use ollama_rs::generation::embeddings::request::{GenerateEmbeddingsRequest, EmbeddingsInput};
let request = GenerateEmbeddingsRequest::new(model, EmbeddingsInput::Single(text.to_string()));
let response = ollama.generate_embeddings(request).await?;
// response.embeddings: Vec<Vec<f32>>
```

## Key Design Decisions

1. **OpenAiProvider implements both `LlmProvider` and `EmbeddingProvider`** — OpenAI has both chat and embeddings APIs.
2. **OllamaProvider implements both `LlmProvider` and `EmbeddingProvider`** — Ollama supports both locally.
3. **AnthropicProvider does NOT implement `EmbeddingProvider`** — Anthropic has no embeddings API (matches spec).
4. **Message type aliasing** — `ollama_rs::generation::chat::ChatMessage` conflicts with our `crate::message::ChatMessage`; aliased as `OllamaMessage`.
5. **Error mapping** — OpenAI SDK errors are strings; parse for 401/429 to map to Auth/RateLimit variants.
6. **No backoff for OpenAI/Ollama** — async-openai handles retries internally; Ollama is local (no rate limiting needed). Follows simplicity principle for this iteration.
7. **Learning tests** — Two new crates (async-openai, ollama-rs) both require learning tests.

## Files To Create/Modify
- `Cargo.toml` (add async-openai, ollama-rs to workspace deps)
- `crates/lazyjob-llm/Cargo.toml` (add async-openai, ollama-rs deps)
- `crates/lazyjob-llm/src/providers/openai.rs` (NEW)
- `crates/lazyjob-llm/src/providers/ollama.rs` (NEW)
- `crates/lazyjob-llm/src/providers/mod.rs` (add re-exports)
- `crates/lazyjob-llm/src/lib.rs` (add re-exports)
