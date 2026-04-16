# Research: Task 15 — llm-anthropic

## Task Description
Implement `AnthropicProvider` in `lazyjob-llm/src/providers/anthropic.rs` using `reqwest` (rustls-tls). Support both streaming (SSE) and non-streaming. Implement 3-attempt exponential backoff (1s, 2s, 4s) on 429/500/503. Map Anthropic API response to `LlmResponse`. Read API key from CredentialManager. Write integration test gated behind `#[cfg(feature="integration")]`.

---

## Existing Code State

### crates/lazyjob-llm/src/lib.rs
Exports: `LlmError`, `Result`, `ChatMessage`, `CompletionOptions`, `LlmResponse`, `TokenUsage`, `MockLlmProvider`, `MockEmbeddingProvider`, `LlmProvider`, `EmbeddingProvider`.

### crates/lazyjob-llm/src/provider.rs
```rust
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    async fn complete(messages: Vec<ChatMessage>, opts: CompletionOptions) -> Result<LlmResponse>;
}
pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn embed(text: &str) -> Result<Vec<f32>>;
}
```

### crates/lazyjob-llm/src/message.rs
- `ChatMessage`: System(String), User(String), Assistant(String)
- `CompletionOptions`: model (Option), temperature (Some(0.7)), max_tokens (Some(4096)), stream (false)
- `LlmResponse`: content, model, usage, stop_reason
- `TokenUsage::new(prompt, completion)` auto-computes total

### crates/lazyjob-llm/src/error.rs
`LlmError` variants: Api, Auth, RateLimit, Network, ModelNotFound, ContextLengthExceeded, Stream, NotSupported

### Workspace deps (Cargo.toml)
Already present: async-trait, tokio (full), serde, serde_json
Missing: reqwest (needs to be added)

---

## Anthropic API Details

### Endpoint
`POST https://api.anthropic.com/v1/messages`

### Headers
```
x-api-key: {api_key}
anthropic-version: 2023-06-01
content-type: application/json
```

### Non-streaming request body
```json
{
  "model": "claude-haiku-4-5-20251001",
  "max_tokens": 4096,
  "system": "system message text",
  "messages": [
    {"role": "user", "content": "Hello"},
    {"role": "assistant", "content": "Hi"},
    {"role": "user", "content": "How are you?"}
  ]
}
```
Notes:
- System messages are extracted and sent as top-level `system` field
- Only user/assistant messages go in `messages` array
- Anthropic requires at least one user message

### Non-streaming response
```json
{
  "id": "msg_...",
  "type": "message",
  "role": "assistant",
  "content": [{"type": "text", "text": "Response text here"}],
  "model": "claude-haiku-4-5-20251001",
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {
    "input_tokens": 25,
    "output_tokens": 100
  }
}
```

### Streaming (SSE) request body
Same as above, plus `"stream": true`.

### Streaming response (SSE event stream)
```
event: message_start
data: {"type":"message_start","message":{"id":"msg_...","type":"message","role":"assistant","content":[],"model":"claude-haiku-4-5-20251001","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: ping
data: {"type":"ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":100}}

event: message_stop
data: {"type":"message_stop"}
```

### Error responses
- `401 Unauthorized` → `LlmError::Auth`
- `429 Too Many Requests` → `LlmError::RateLimit` (retryable)
- `500 Internal Server Error` → `LlmError::Api` (retryable)
- `503 Service Unavailable` → `LlmError::Api` (retryable)
- Other 4xx → `LlmError::Api` (non-retryable)

---

## Backoff Strategy
- Maximum 3 retries (4 total attempts) on retryable errors (429, 500, 503)
- Delays: [1s, 2s, 4s] between attempts
- If all attempts fail, return last error

---

## Dependencies Needed
- `reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"], default-features = false }` in workspace
- `reqwest = { workspace = true }` in lazyjob-llm/Cargo.toml
- Integration feature gate: `[features] integration = []` in lazyjob-llm/Cargo.toml

---

## Key Design Decisions
1. API key passed directly to constructor (`new(api_key: String)`) — keeps provider simple and testable
2. `from_credentials(creds: &CredentialManager)` alternative constructor reads from keyring
3. Default model: `claude-haiku-4-5-20251001` (cheapest, fastest; overridden by `CompletionOptions::model`)
4. SSE streaming accumulated into final `LlmResponse` (trait returns complete response, not stream)
5. System messages extracted from messages vector and sent as top-level `system` field
6. `EmbeddingProvider` not implemented (Anthropic has no embeddings API) — returns `LlmError::NotSupported`
