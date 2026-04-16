# Plan: Task 17 — llm-registry

## Files to Create
1. `crates/lazyjob-llm/src/cost.rs` — pricing table, `estimate_cost(model, tokens) -> u64`
2. `crates/lazyjob-llm/src/registry.rs` — `ProviderRegistry`, `LlmBuilder`, `LoggingProvider`

## Files to Modify
1. `crates/lazyjob-llm/Cargo.toml` — add `sqlx = { workspace = true }`
2. `crates/lazyjob-llm/src/lib.rs` — add `pub mod cost`, `pub mod registry`, re-exports

## Types to Define

### cost.rs
```rust
pub fn estimate_cost(model: &str, tokens: u32) -> u64
// Returns total microdollars cost
// Internal: PRICING table as const array of (&str, u64) tuples
// Pricing in microdollars per 1000 tokens (blended input/output)
```

### registry.rs
```rust
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default: Option<String>,
}
impl ProviderRegistry {
    pub fn new() -> Self
    pub fn add(&mut self, name: impl Into<String>, provider: Arc<dyn LlmProvider>)
    pub fn get(&self, name: &str) -> Option<Arc<dyn LlmProvider>>
    pub fn default_provider(&self) -> Option<Arc<dyn LlmProvider>>
    pub fn all(&self) -> Vec<(String, Arc<dyn LlmProvider>)>
    pub fn set_default(&mut self, name: impl Into<String>)
}
impl Default for ProviderRegistry

pub struct LlmBuilder;
impl LlmBuilder {
    pub fn from_config(config: &Config, creds: &CredentialManager) -> Result<Box<dyn LlmProvider>>
}

pub struct LoggingProvider {
    inner: Arc<dyn LlmProvider>,
    pool: sqlx::PgPool,
    operation: Option<String>,
}
impl LoggingProvider {
    pub fn new(provider: Arc<dyn LlmProvider>, pool: sqlx::PgPool) -> Self
    pub fn with_operation(self, op: impl Into<String>) -> Self
    async fn log_usage(&self, response: &LlmResponse) -> ()
}
#[async_trait] impl LlmProvider for LoggingProvider
```

## Tests to Write

### cost.rs tests (unit, no DB)
- `estimate_cost_zero_tokens` — 0 tokens → 0 microdollars
- `estimate_cost_claude_haiku` — known rate verification
- `estimate_cost_gpt4o_mini` — known rate verification
- `estimate_cost_ollama_is_free` — 0 cost for local models
- `estimate_cost_unknown_model_uses_default` — unknown model gets default rate

### registry.rs tests (unit, no DB)
- `registry_add_and_get` — add provider, retrieve by name
- `registry_get_missing_returns_none`
- `registry_default_provider_after_set_default`
- `registry_all_returns_all_providers`
- `builder_falls_back_to_ollama_with_no_creds`
- `builder_uses_anthropic_when_key_set`
- `builder_uses_openai_when_anthropic_missing_but_openai_set`
- `builder_respects_configured_provider_ollama`
- `logging_provider_delegates_provider_name`
- `logging_provider_delegates_model_name`

## No New Migrations
The `token_usage_log` table already exists from migration 001.

## Implementation Order
1. `cost.rs` (no deps beyond `crate::message::TokenUsage`)
2. `registry.rs` (depends on cost.rs, providers, lazyjob-core types)
3. Update `Cargo.toml` and `lib.rs`
4. Run full build + test cycle
