# Research: Task 17 — llm-registry

## Task Summary
Implement `ProviderRegistry`, `LlmBuilder::from_config`, `LoggingProvider` (token_usage_log insert), and `cost.rs` in `lazyjob-llm`.

---

## Existing Codebase State

### lazyjob-llm structure
- `src/error.rs` — `LlmError` enum, `type Result<T>`
- `src/message.rs` — `ChatMessage`, `CompletionOptions`, `LlmResponse`, `TokenUsage`
- `src/provider.rs` — `LlmProvider`, `EmbeddingProvider` traits (async_trait)
- `src/mock.rs` — `MockLlmProvider`, `MockEmbeddingProvider`
- `src/providers/anthropic.rs` — `AnthropicProvider::new(api_key)`, `from_credentials(creds)`
- `src/providers/openai.rs` — `OpenAiProvider::new(api_key)`, `from_credentials(creds)`
- `src/providers/ollama.rs` — `OllamaProvider::new()` (no credentials needed)

### LlmProvider trait interface
```rust
pub trait LlmProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    async fn complete(&self, messages: Vec<ChatMessage>, opts: CompletionOptions) -> Result<LlmResponse>;
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

### TokenUsage
```rust
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
```

### Config struct (lazyjob-core)
Key fields relevant to registry:
- `default_llm_provider: Option<String>` — "anthropic", "openai", or "ollama"

### CredentialManager (lazyjob-core)
- `CredentialManager::with_store(Box<dyn CredentialStore>)` — used in tests
- `InMemoryStore::new()` — test credential store
- `get_api_key(provider: &str) -> core::Result<SecretString>`
- Returns `Err(CoreError::Credential(...))` when no key is set

### from_credentials pattern
Each provider's `from_credentials` returns `LlmError::Auth(...)` when no key is set.
This enables clean fallback with `if let Ok(p) = Provider::from_credentials(creds)`.

### token_usage_log schema
```sql
CREATE TABLE token_usage_log (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_microdollars BIGINT NOT NULL DEFAULT 0,
    operation TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### sqlx usage pattern (from existing repos)
- Uses runtime `sqlx::query(...)` NOT compile-time `sqlx::query!(...)` to avoid needing DATABASE_URL at build time
- `PgPool` from `sqlx::postgres::PgPool`

---

## Design Decisions

### 1. cost.rs — estimate_cost(model, tokens) -> u64
- Returns **microdollars** (1 microdollar = $0.000001)
- Single total `tokens: u32` parameter (blended input+output rate)
- Match model name by substring for flexibility (provider variants like claude-haiku-4-5-...)
- Pricing table based on current public list prices (blended input/output)

### 2. ProviderRegistry
- `HashMap<String, Arc<dyn LlmProvider>>` keyed by provider name
- Methods: `new()`, `add(name, provider)`, `get(name) -> Option<Arc<dyn LlmProvider>>`, `default_provider() -> Option<Arc<dyn LlmProvider>>`, `all() -> Vec<...>`, `set_default(name)`
- `Arc<dyn LlmProvider>` for cheap cloning and shared ownership

### 3. LlmBuilder::from_config
- Fallback chain: configured provider → Anthropic (if key set) → OpenAI (if key set) → Ollama (always)
- Returns `Result<Box<dyn LlmProvider>>` (owned, not Arc)
- Uses `if let Ok(p) = Provider::from_credentials(creds)` pattern for clean fallback

### 4. LoggingProvider
- Wraps `Arc<dyn LlmProvider>` and `sqlx::PgPool`
- Implements `LlmProvider`: delegates `complete()`, logs to token_usage_log table
- Uses fire-and-forget pattern for DB insert (logs errors with tracing, doesn't fail on log failure)
- Constructor: `LoggingProvider::new(provider: Arc<dyn LlmProvider>, pool: sqlx::PgPool)`
- Optional: `with_operation(op: &str)` builder for labeling the operation context

### 5. sqlx dependency
- Add `sqlx = { workspace = true }` to `lazyjob-llm/Cargo.toml`
- Already in workspace, needed for `PgPool` type and runtime queries

---

## Rust Patterns Used

- **Decorator pattern** (`LoggingProvider`): wraps existing trait object, adds behavior
- **Builder pattern** (`LlmBuilder::from_config`): constructs best provider from config
- **Arc<dyn Trait>**: for shared, clone-able trait objects in the registry
- No new external crates needed (sqlx already in workspace)

---

## Test Strategy

### Unit tests (no DB needed)
- `estimate_cost_known_models` — verify pricing for claude-haiku, gpt-4o, ollama
- `estimate_cost_zero_tokens` — edge case: 0 tokens → 0 microdollars
- `estimate_cost_unknown_model` — falls through to default rate
- `registry_add_and_get` — add MockLlmProvider, retrieve by name
- `registry_default_provider` — set_default, verify default_provider returns it
- `registry_get_missing_returns_none` — get unknown name
- `builder_falls_back_to_ollama_with_no_creds` — InMemoryStore with no keys → Ollama
- `builder_uses_anthropic_when_key_set` — set anthropic key → returns Anthropic
- `builder_respects_config_override` — config.default_llm_provider = "ollama" → Ollama
- `logging_provider_delegates_to_inner` — LoggingProvider without DB (test without pool)

### Integration tests (need DATABASE_URL)
- `logging_provider_inserts_usage_log` — verify token_usage_log row created after complete

No new learning tests needed (sqlx already proven in tasks 3-4; async_trait proven in tasks 14-15).
