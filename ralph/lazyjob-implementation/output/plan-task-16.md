# Plan: Task 16 — llm-openai

## Files to Create/Modify

| File | Action |
|------|--------|
| `Cargo.toml` | Add `async-openai = "0.34"` and `ollama-rs = "0.3"` to `[workspace.dependencies]` |
| `crates/lazyjob-llm/Cargo.toml` | Add `async-openai` and `ollama-rs` to `[dependencies]` |
| `crates/lazyjob-llm/src/providers/openai.rs` | NEW — `OpenAiProvider` |
| `crates/lazyjob-llm/src/providers/ollama.rs` | NEW — `OllamaProvider` |
| `crates/lazyjob-llm/src/providers/mod.rs` | Add `pub mod openai`, `pub mod ollama`, re-exports |
| `crates/lazyjob-llm/src/lib.rs` | Add `pub use providers::{OpenAiProvider, OllamaProvider}` |

## Types/Functions to Define

### `providers/openai.rs`
- `struct OpenAiProvider { client: Client<OpenAIConfig>, default_model: String, embedding_model: String }`
- `OpenAiProvider::new(api_key: impl Into<String>) -> Self`
- `OpenAiProvider::with_model(self, model: impl Into<String>) -> Self`
- `OpenAiProvider::from_credentials(creds: &CredentialManager) -> Result<Self>`
- `impl LlmProvider for OpenAiProvider`
- `impl EmbeddingProvider for OpenAiProvider`

### `providers/ollama.rs`
- `struct OllamaProvider { client: Ollama, default_model: String, embedding_model: String }`
- `OllamaProvider::new() -> Self` (localhost:11434)
- `OllamaProvider::with_host_port(host: impl Into<String>, port: u16) -> Self`
- `OllamaProvider::with_model(self, model: impl Into<String>) -> Self`
- `impl Default for OllamaProvider`
- `impl LlmProvider for OllamaProvider`
- `impl EmbeddingProvider for OllamaProvider`

## Tests to Write

### Learning Tests (marked `// learning test: verifies library behavior`)
1. **`async_openai_client_builds_with_config`** — proves `Client::with_config(OpenAIConfig::new().with_api_key("..."))` compiles and builds without error (no network call)
2. **`async_openai_chat_request_serializes`** — proves `CreateChatCompletionRequestArgs` can build a valid request struct; checks model field is set correctly
3. **`ollama_rs_client_constructs`** — proves `Ollama::new("http://localhost", 11434)` and `Ollama::default()` both construct without error (no network call)
4. **`ollama_rs_message_constructors`** — proves `OllamaMessage::system/user/assistant` constructors work correctly

### Unit Tests — OpenAiProvider
5. **`openai_provider_name_and_model`** — `provider_name()` returns "openai", `model_name()` returns default model
6. **`openai_with_model_override`** — `with_model("gpt-4o")` changes `model_name()`
7. **`openai_message_mapping_system`** — verifies ChatMessage::System maps to system role
8. **`openai_message_mapping_user`** — verifies ChatMessage::User maps to user role

### Unit Tests — OllamaProvider
9. **`ollama_provider_name_and_model`** — `provider_name()` returns "ollama", `model_name()` returns default
10. **`ollama_with_model_override`** — `with_model("mistral:latest")` changes model_name
11. **`ollama_default_constructs`** — `OllamaProvider::default()` builds without panic
12. **`ollama_message_mapping`** — ChatMessage variants correctly map to OllamaMessage variants

## Migrations
None — this task is pure in-memory provider logic.

## Verification Steps
1. `cargo build` — zero errors
2. `cargo clippy -- -D warnings` — zero warnings
3. `cargo test` — all 249+ tests still pass, plus 12 new = 261+ total
4. `cargo fmt --all` — no diff
