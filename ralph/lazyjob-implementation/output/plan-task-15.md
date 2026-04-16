# Plan: Task 15 — llm-anthropic

## Files to Create or Modify

| File | Action |
|------|--------|
| `Cargo.toml` | Add `reqwest` to `[workspace.dependencies]` |
| `crates/lazyjob-llm/Cargo.toml` | Add `reqwest` dep, add `[features] integration = []` |
| `crates/lazyjob-llm/src/providers/mod.rs` | New: re-export `AnthropicProvider` |
| `crates/lazyjob-llm/src/providers/anthropic.rs` | New: full `AnthropicProvider` implementation |
| `crates/lazyjob-llm/src/lib.rs` | Add `pub mod providers` |

---

## Types and Functions

### `crates/lazyjob-llm/src/providers/anthropic.rs`

```rust
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    default_model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self
    pub fn with_model(self, model: impl Into<String>) -> Self
    pub fn from_credentials(creds: &CredentialManager) -> Result<Self>  // reads "anthropic" key
    
    // internal
    async fn call_non_streaming(&self, req: &AnthropicRequest) -> Result<LlmResponse>
    async fn call_streaming(&self, req: &AnthropicRequest) -> Result<LlmResponse>
    async fn call_with_backoff(&self, req: &AnthropicRequest) -> Result<LlmResponse>
    fn build_request(&self, messages: Vec<ChatMessage>, opts: &CompletionOptions) -> AnthropicRequest
    fn map_http_error(&self, status: reqwest::StatusCode, body: &str) -> LlmError
}

impl LlmProvider for AnthropicProvider { ... }
```

### Internal serde types (not pub)

```rust
struct AnthropicRequest { model, max_tokens, messages, system?, stream? }
struct AnthropicMessage { role, content }
struct AnthropicResponse { content, model, stop_reason, usage }
struct AnthropicContentBlock { #[serde(rename="type")] block_type, text? }
struct AnthropicUsage { input_tokens, output_tokens }
// SSE types:
struct SseEvent { #[serde(rename="type")] event_type, delta? }
struct SseDelta { #[serde(rename="type")] delta_type, text? }
```

---

## Tests to Write

### Learning tests (no new crates — reqwest is a well-known API, but verify key patterns)
- `reqwest_client_builds_with_rustls` — proves `reqwest::Client::builder().use_rustls_tls().build()` works in tests (compile-time verification of rustls feature)
- `reqwest_json_serializes_request_body` — proves a struct with `#[derive(Serialize)]` serializes correctly for JSON body (sanity check before sending to Anthropic)

### Unit tests
- `anthropic_provider_name_and_default_model` — verifies provider metadata
- `build_request_separates_system_messages` — system messages go to `system` field, user/assistant stay in `messages`
- `build_request_with_no_system_message` — `system` field is None when no system message
- `build_request_uses_opts_model_override` — `CompletionOptions::model` overrides default
- `map_http_error_401_returns_auth` — status 401 → `LlmError::Auth`
- `map_http_error_429_returns_rate_limit` — status 429 → `LlmError::RateLimit`
- `map_http_error_500_returns_api` — status 500 → `LlmError::Api`
- `is_retryable_for_rate_limit_and_server_errors` — checks which errors trigger retry
- `parse_non_streaming_response` — deserializes a fixture JSON response into `LlmResponse`
- `parse_sse_stream_accumulates_text` — parses a fixture SSE byte stream into `LlmResponse`

### Integration test (feature-gated)
```rust
#[cfg(feature = "integration")]
#[tokio::test]
async fn anthropic_real_call_to_haiku() {
    // Reads ANTHROPIC_API_KEY env var, calls claude-haiku-4-5-20251001, verifies non-empty response
}
```

---

## Migration Needed
None — this task is purely in-memory, no DB changes.

---

## Backoff Implementation Detail
```rust
const MAX_RETRIES: usize = 3;
const BACKOFF_DELAYS_SECS: [u64; 3] = [1, 2, 4];

async fn call_with_backoff(&self, req: &AnthropicRequest) -> Result<LlmResponse> {
    let mut last_error = LlmError::Api("no attempts made".into());
    for attempt in 0..=MAX_RETRIES {
        match self.call_once(req).await {
            Ok(resp) => return Ok(resp),
            Err(e) if is_retryable(&e) && attempt < MAX_RETRIES => {
                tokio::time::sleep(Duration::from_secs(BACKOFF_DELAYS_SECS[attempt])).await;
                last_error = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_error)
}
```

---

## SSE Parsing Detail
```
1. Collect response bytes via .bytes().await (simpler than streaming for complete-response use case)
2. Split into lines
3. Skip lines starting with "event:" (we use the data type field to discriminate)
4. For lines starting with "data: ", strip prefix, parse JSON
5. Match on event_type:
   - "content_block_delta" → append delta.text to accumulated string
   - "message_delta" → capture stop_reason and output_tokens
   - "message_start" → capture model name and input_tokens
   - "message_stop" → break loop
6. Build LlmResponse from accumulated values
```
