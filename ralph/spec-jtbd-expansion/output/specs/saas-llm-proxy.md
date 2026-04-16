# Spec: SaaS — LLM Proxy

**JTBD**: Offer premium AI features without requiring users to manage API keys
**Topic**: Define the server-side LLM proxy that routes requests from LazyJob clients through a tier-gated, usage-tracked proxy for SaaS deployment
**Domain**: saas

---

## What

In SaaS mode, LazyJob operates a server-side LLM proxy (the loom pattern) that:
1. Receives LLM requests from LazyJob clients (TUI + Ralph loops)
2. Gates access behind subscription tier checks
3. Routes requests to the cheapest capable provider at runtime (Anthropic for Pro, OpenAI for Free tier)
4. Tracks per-user token usage for billing
5. Streams responses back to clients via SSE

In local mode, users bring their own API keys. In SaaS mode, LazyJob's server pays for LLM requests and bills users through subscription tiers.

## Why

The LLM proxy enables:
- **Subscription tier gating**: Free users get limited LLM calls; Pro users get unlimited
- **Cost control**: Route to the cheapest capable model — Ollama (free if self-hosted) > OpenAI > Anthropic
- **Token usage tracking**: Per-user billing for overage charges
- **No API key exposure**: Users don't need to manage API keys in SaaS mode — LazyJob's server handles billing
- **Consistent UX**: Same `LlmProvider` interface in both local and SaaS modes; the `LoomProxyProvider` impl replaces direct provider calls transparently

## How

### Architecture

```
Local Mode (current):
  lazyjob-llm → LlmProvider (direct to Anthropic/OpenAI/Ollama)
                ↑ API keys in keyring

SaaS Mode:
  lazyjob-tui → lazyjob-ralph → lazyjob-llm → LlmBuilder
                                            ↓
                                    LoomProxyProvider ──────────────────┐
                                            │                           │
                              ┌─────────────▼────────────┐              │
                              │   Loom LLM Proxy Server  │              │
                              │   (lazyjob-proxy crate) │              │
                              │                         │              │
                              │  ┌───────────────────┐  │              │
                              │  │ TierGating        │  │              │
                              │  │ UsageTracker      │  │              │
                              │  │ LlmRouter         │  │              │
                              │  └───────────────────┘  │              │
                              └────────────┬────────────┘              │
                                           ▼                            │
                              ┌────────────────────────────┐            │
                              │ Anthropic / OpenAI / Ollama│◄───────────┘
                              └────────────────────────────┘
```

### LlmBuilder in SaaS Mode

```rust
// lazyjob-llm/src/builder.rs

pub struct LlmBuilder {
    config: LlmConfig,
    credential_store: CredentialStore,
}

pub enum LlmProviderVariant {
    Direct(Box<dyn LlmProvider>),        // Local mode: real provider
    LoomProxy(LoomProxyProvider),        // SaaS mode: proxy provider
}

impl LlmBuilder {
    pub async fn build(&self) -> Result<LlmProviderVariant> {
        if self.config.proxy_url.is_some() {
            // SaaS mode: use server-side proxy
            let proxy_url = self.config.proxy_url.as_ref().unwrap();
            Ok(LlmProviderVariant::LoomProxy(LoomProxyProvider::new(proxy_url)))
        } else {
            // Local mode: use direct provider
            let api_key = self.credential_store.get_api_key(&self.config.provider).await?;
            match self.config.provider.as_str() {
                "anthropic" => Ok(LlmProviderVariant::Direct(
                    AnthropicProvider::new(api_key.unwrap())?
                )),
                "openai" => Ok(LlmProviderVariant::Direct(
                    OpenAIProvider::new(api_key.unwrap())?
                )),
                "ollama" => Ok(LlmProviderVariant::Direct(
                    OllamaProvider::new(&self.config.ollama.as_ref().unwrap().endpoint)?
                )),
                _ => Err(Error::UnknownLlmProvider(self.config.provider.clone())),
            }
        }
    }
}
```

### LoomProxyProvider

```rust
// lazyjob-proxy/src/lib.rs (new crate)

pub struct LoomProxyProvider {
    client: reqwest::Client,
    base_url: String,
    auth_token: String,
}

#[async_trait::async_trait]
impl LlmProvider for LoomProxyProvider {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse> {
        // 1. Check tier entitlements
        self.check_tier_quota().await?;

        // 2. Route to cheapest capable provider
        let target_provider = self.router.cheapest_capable(&messages).await?;

        // 3. Forward request
        let response = self.client
            .post(&format!("{}/v1/chat", self.base_url))
            .bearer_auth(&self.auth_token)
            .json(&ChatRequest { messages, provider: target_provider })
            .send()
            .await?;

        // 4. Track usage
        let usage = response.usage();
        self.usage_tracker.record(&self.user_id, &target_provider, &usage).await?;

        Ok(response.into())
    }

    async fn complete(&self, prompt: &str) -> Result<String> {
        self.chat(vec![ChatMessage::user(prompt)]).map(|r| r.content)
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // Embedding goes directly to Ollama in SaaS (cheapest for embeddings)
        self.client
            .post("https://api.ollama.com/api/embeddings")
            .bearer_auth(&self.auth_token)
            .json(&serde_json::json!({ "model": "nomic-embed-text", "text": text }))
            .send()
            .await?
            .json()
            .await
            .map_err(Into::into)
    }
}
```

### Tier Gating

```rust
// lazyjob-proxy/src/tier.rs

pub enum SubscriptionTier {
    Free,
    Pro,
    Team,
    Enterprise,
}

impl SubscriptionTier {
    pub fn max_tokens_per_month(&self) -> Option<i64> {
        match self {
            SubscriptionTier::Free => Some(100_000),    // ~$5 worth
            SubscriptionTier::Pro => None,               // Unlimited
            SubscriptionTier::Team => None,
            SubscriptionTier::Enterprise => None,
        }
    }

    pub fn allowed_models(&self) -> Vec<&'static str> {
        match self {
            SubscriptionTier::Free => vec!["gpt-4o-mini", "llama3.2"],
            SubscriptionTier::Pro => vec!["claude-3-5-sonnet", "gpt-4o", "gpt-4o-mini", "llama3.2"],
            SubscriptionTier::Team => vec!["claude-3-5-sonnet", "gpt-4o", "gpt-4o-mini", "llama3.2", "claude-3-opus"],
            SubscriptionTier::Enterprise => vec!["*"],   // All models
        }
    }
}

impl LoomProxyProvider {
    async fn check_tier_quota(&self) -> Result<()> {
        let usage = self.usage_tracker.get_monthly_usage(&self.user_id).await?;
        let limit = self.user_tier.max_tokens_per_month();
        if let Some(limit) = limit {
            if usage >= limit {
                return Err(Error::QuotaExceeded { used: usage, limit });
            }
        }
        Ok(())
    }
}
```

### Token Usage Tracking

```rust
// lazyjob-proxy/src/usage.rs

pub struct UsageTracker {
    pool: SqlitePool, // Or PostgresPool in SaaS
}

impl UsageTracker {
    pub async fn record(
        &self,
        user_id: &Uuid,
        provider: &str,
        usage: &TokenUsage,
    ) -> Result<()> {
        let cost = self.calculate_cost(provider, usage);
        sqlx::query!(
            "INSERT INTO token_usage_log (id, user_id, provider, model, input_tokens, output_tokens, cost_microdollars, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            Uuid::new_v4().to_string(),
            user_id.to_string(),
            provider,
            usage.model,
            usage.input_tokens as i64,
            usage.output_tokens as i64,
            cost,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_monthly_usage(&self, user_id: &Uuid) -> Result<i64> {
        sqlx::query_scalar!(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0) FROM token_usage_log
             WHERE user_id = ? AND created_at > datetime('now', 'start of month')",
            user_id.to_string()
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    fn calculate_cost(&self, provider: &str, usage: &TokenUsage) -> i64 {
        // Microdollars: Anthropic Claude 3.5 Sonnet = $3/M input, $15/M output
        // OpenAI GPT-4o = $2.5/M input, $10/M output
        match provider {
            "anthropic" => {
                (usage.input_tokens as i64 * 3) + (usage.output_tokens as i64 * 15)
            }
            "openai" => {
                (usage.input_tokens as i64 * 25 / 10) + (usage.output_tokens as i64 * 100 / 10)
            }
            _ => 0,
        }
    }
}
```

### LLM Provider Selection Logic

```rust
// lazyjob-proxy/src/router.rs

pub struct LlmRouter {
    providers: HashMap<&'static str, Box<dyn LlmProvider>>,
}

impl LlmRouter {
    pub fn cheapest_capable(&self, messages: &[ChatMessage]) -> &'static str {
        // If messages require function calling or vision: route to Anthropic
        if messages.iter().any(|m| m.has_attachments()) {
            return "anthropic";
        }

        // If messages are short: route to OpenAI GPT-4o-mini (cheaper for short context)
        let total_tokens = messages.iter().map(|m| m.token_count()).sum::<u64>();
        if total_tokens < 2000 {
            return "openai";  // GPT-4o-mini is 10x cheaper for small inputs
        }

        // Default to Anthropic for complex reasoning tasks
        "anthropic"
    }
}
```

## Open Questions

- **`LoomProxyProvider` fallback**: If the loom proxy server is down, should `LoomProxyProvider::chat()` fall back to local Ollama if available? Or fail fast? MVP: fail fast with a clear error message.
- **Ollama for embeddings in SaaS**: Should SaaS deployments require users to self-host Ollama for embeddings (to avoid API costs on the most token-heavy operation)? Or should LazyJob maintain a shared embedding service? MVP: shared Ollama endpoint with per-user rate limiting.

## Implementation Tasks

- [ ] Create `lazyjob-proxy/` crate scaffold: `lazyjob-proxy/src/lib.rs`, `Cargo.toml`, endpoint handler
- [ ] Implement `LoomProxyProvider` in `lazyjob-llm/src/loom_proxy.rs` that implements `LlmProvider` and routes HTTP requests to the proxy server
- [ ] Add `proxy_url` field to `[llm]` config section in `lazyjob.toml`
- [ ] Implement `LlmBuilder::build()` with `LoomProxyProvider` variant when `proxy_url` is set
- [ ] Implement `TierGating` in `lazyjob-proxy/src/tier.rs` with `SubscriptionTier`, `max_tokens_per_month()`, `allowed_models()`
- [ ] Implement `UsageTracker` in `lazyjob-proxy/src/usage.rs` with `record()`, `get_monthly_usage()`, `calculate_cost()`
- [ ] Implement `LlmRouter::cheapest_capable()` for automatic model selection based on message complexity
- [ ] Add SSE streaming response type to `LoomProxyProvider` for real-time token streaming to TUI
- [ ] Implement `token_usage_log` billing query endpoint (`GET /api/usage/:user_id`) for user-facing usage dashboard
- [ ] Add `cost_microdollars` column to `token_usage_log` and wire `calculate_cost()` from both SaaS and local LLM calls
