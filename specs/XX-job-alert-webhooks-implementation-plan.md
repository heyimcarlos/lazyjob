# Implementation Plan: Real-Time Job Alert Webhooks

## Status
Draft

## Related Spec
`specs/XX-job-alert-webhooks.md`

## Overview

The Job Alert Webhooks system extends LazyJob's polling-based job discovery with a
push-based real-time ingestion channel. When a Greenhouse or Lever company board pushes
a `job_created` or `job_updated` webhook, LazyJob's embedded `axum` HTTP server receives
it, verifies the HMAC signature, normalizes the payload into the canonical `Job` model,
and hands it off to the existing `JobIngestionService` for deduplication and persistence.

This plan adapts the spec's Redis-based retry queue to SQLite (consistent with the
local-first architecture) and replaces the spec's in-memory `HashSet` dedup with the
existing `platform_job_index` unique constraint. The spec's `WebhookRetryQueue` struct
is re-designed as a background `tokio` task backed by a `webhook_delivery_log` SQLite
table. The spec's email fallback is implemented as `EmailAlertWatcher` using the
`async-imap` crate.

An alert rule engine sits between incoming webhook payloads and the notification layer:
users define keyword/company/location filter criteria in `config.toml`; matching jobs
trigger desktop notifications immediately via `notify-rust`. This separates the webhook
plumbing (always on) from alert configuration (user-controlled).

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, migrations scaffold
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobIngestionService`, `platform_job_index`
- `specs/11-platform-api-integrations-implementation-plan.md` — `GreenhouseClient`, normalized `Job` model
- `specs/application-pipeline-metrics-implementation-plan.md` — `NotificationScheduler` (for alert delivery)

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml (new additions)
axum         = { version = "0.7", features = ["macros"] }
tower        = { version = "0.4", features = ["limit"] }
tower-http   = { version = "0.5", features = ["cors", "trace"] }
hmac         = "0.12"
sha2         = "0.10"
hex          = "0.4"
governor     = "0.6"          # already present from platform integrations
notify-rust  = "4"            # already present from notifications plan

# Email fallback (Phase 4)
async-imap   = "0.9"
async-native-tls = "0.5"     # TLS for IMAP (or native-tls)
mailparse    = "0.14"         # Parse RFC 2822 email messages
```

## Architecture

### Crate Placement

All webhook code lives in `lazyjob-core/src/webhooks/`. The `WebhookServer` is
constructed and spawned by `lazyjob-cli/src/main.rs` (or by the TUI startup path in
`lazyjob-tui`) as an optional background tokio task. The server writes incoming jobs
directly to SQLite via the existing `JobIngestionService` — it does not create a new
persistence path.

```
lazyjob-core/
  src/
    webhooks/
      mod.rs           # pub use surface
      server.rs        # WebhookServer, axum router, graceful shutdown
      handlers/
        mod.rs
        greenhouse.rs  # handle_greenhouse handler
        lever.rs       # handle_lever handler
        email.rs       # handle_forwarded_email handler
      security.rs      # SignatureVerifier, IpAllowlist, RateLimiterState
      alert_rules.rs   # AlertRule, AlertRuleEngine, AlertRuleConfig
      retry.rs         # RetryWorker, RetryPolicy, webhook_delivery_log queries
      email_watcher.rs # EmailAlertWatcher (Phase 4)
      error.rs         # WebhookError enum
```

### Core Types

```rust
// webhooks/server.rs

pub struct WebhookServer {
    config: WebhookConfig,
    db: Arc<Database>,
    ingestion: Arc<JobIngestionService>,
    alert_engine: Arc<AlertRuleEngine>,
    shutdown_tx: watch::Sender<bool>,
}

pub struct WebhookConfig {
    /// Listening port. Default: 9731.
    pub port: u16,
    /// URL path prefix for all webhook routes. Default: "/webhooks".
    pub path_prefix: String,
    /// Per-source HMAC secrets. Key = source name ("greenhouse" | "lever").
    pub secrets: HashMap<String, secrecy::Secret<String>>,
    /// IP allowlist. Empty = allow all (development mode). Production: set platform IPs.
    pub ip_allowlist: Vec<IpAddr>,
    /// Max requests per minute per source IP. Default: 60.
    pub rate_limit_rpm: u32,
    /// Whether the webhook server is enabled at all.
    pub enabled: bool,
}

/// A single raw webhook delivery, logged to SQLite before processing.
pub struct WebhookDelivery {
    pub id: Uuid,
    pub source: WebhookSource,
    pub received_at: DateTime<Utc>,
    pub payload_bytes: Vec<u8>,
    pub signature_header: String,
    pub status: DeliveryStatus,
    pub attempts: u8,
    pub last_error: Option<String>,
    pub processed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum WebhookSource {
    Greenhouse,
    Lever,
    Email,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Processing,
    Succeeded,
    Failed,
    Abandoned,   // max retries exhausted
}
```

### Trait Definitions

```rust
// webhooks/handlers/mod.rs

/// Each webhook source implements this to normalize its payload into the canonical Job.
#[async_trait::async_trait]
pub trait WebhookHandler: Send + Sync + 'static {
    fn source(&self) -> WebhookSource;

    /// Verify the source-specific signature header against the raw body bytes.
    fn verify_signature(&self, headers: &HeaderMap, body: &Bytes) -> Result<(), WebhookError>;

    /// Parse the raw bytes into one or more job-level events.
    fn parse_events(&self, body: &Bytes) -> Result<Vec<WebhookJobEvent>, WebhookError>;
}

pub enum WebhookJobEvent {
    Created { job: Job },
    Updated { source_id: String },
    Closed  { source_id: String },
    Other   { action: String },    // silently ignored
}

// webhooks/alert_rules.rs

pub trait AlertMatcher: Send + Sync {
    fn matches(&self, job: &Job) -> bool;
}
```

### SQLite Schema

```sql
-- Migration: 018_webhook_delivery_log.sql

CREATE TABLE IF NOT EXISTS webhook_delivery_log (
    id              TEXT PRIMARY KEY,               -- UUID v4
    source          TEXT NOT NULL,                  -- 'greenhouse' | 'lever' | 'email'
    received_at     TEXT NOT NULL,                  -- ISO 8601 UTC
    payload_bytes   BLOB NOT NULL,
    signature_header TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'pending',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    processed_at    TEXT,
    CONSTRAINT chk_status CHECK (
        status IN ('pending', 'processing', 'succeeded', 'failed', 'abandoned')
    )
);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_status
    ON webhook_delivery_log (status, received_at)
    WHERE status IN ('pending', 'failed');

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_received
    ON webhook_delivery_log (received_at DESC);

-- Alert rule storage (user-defined filters)
CREATE TABLE IF NOT EXISTS webhook_alert_rules (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,     -- BOOLEAN
    keyword_patterns TEXT NOT NULL DEFAULT '[]',    -- JSON array of strings
    company_patterns TEXT NOT NULL DEFAULT '[]',    -- JSON array of strings
    location_patterns TEXT NOT NULL DEFAULT '[]',   -- JSON array of strings
    min_salary_cents INTEGER,                        -- NULL = no filter
    notify_desktop  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
```

### Module Structure

```
lazyjob-core/
  src/
    webhooks/
      mod.rs
      server.rs
      error.rs
      security.rs
      alert_rules.rs
      retry.rs
      email_watcher.rs    (Phase 4)
      handlers/
        mod.rs
        greenhouse.rs
        lever.rs
        email.rs          (Phase 4)
```

## Implementation Phases

### Phase 1 — Core Receiver and Signature Verification (MVP)

**Step 1.1 — Define error types and migration**

File: `lazyjob-core/src/webhooks/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum WebhookError {
    #[error("invalid HMAC signature")]
    InvalidSignature,
    #[error("missing signature header: {header}")]
    MissingSignatureHeader { header: &'static str },
    #[error("unauthorized IP address: {ip}")]
    UnauthorizedIp { ip: IpAddr },
    #[error("rate limit exceeded")]
    RateLimitExceeded,
    #[error("payload parse error: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("unknown webhook source")]
    UnknownSource,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type WebhookResult<T> = Result<T, WebhookError>;
```

Apply migration `018_webhook_delivery_log.sql` via the existing `sqlx::migrate!` runner.

**Step 1.2 — Signature verification**

File: `lazyjob-core/src/webhooks/security.rs`

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct SignatureVerifier;

impl SignatureVerifier {
    /// Greenhouse sends `X-Greenhouse-Signature` as hex-encoded HMAC-SHA256.
    pub fn verify_greenhouse(secret: &[u8], body: &[u8], sig_header: &str) -> bool {
        let mut mac = HmacSha256::new_from_slice(secret)
            .expect("HMAC accepts any key length");
        mac.update(body);
        let expected = hex::encode(mac.finalize().into_bytes());
        // Constant-time comparison to prevent timing attacks.
        // The `hmac` crate's `verify_slice` operates on raw bytes; hex-compare is safe
        // here since both sides are fixed-length hex strings produced by the same function.
        hmac::subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), sig_header.trim().as_bytes())
            .into()
    }

    /// Lever sends `X-Lever-Signature-256` as `sha256=<hex>`.
    pub fn verify_lever(secret: &[u8], body: &[u8], sig_header: &str) -> bool {
        let hex_part = sig_header.strip_prefix("sha256=").unwrap_or(sig_header);
        Self::verify_greenhouse(secret, body, hex_part)
    }
}

/// Rate limiter state shared across all axum handlers via Extension.
pub struct RateLimiterState {
    limiter: governor::RateLimiter<
        IpAddr,
        governor::state::keyed::DefaultKeyedStateStore<IpAddr>,
        governor::clock::DefaultClock,
    >,
}

impl RateLimiterState {
    pub fn new(rpm: u32) -> Self {
        let quota = governor::Quota::per_minute(NonZeroU32::new(rpm).unwrap());
        Self { limiter: governor::RateLimiter::keyed(quota) }
    }

    pub fn check(&self, ip: IpAddr) -> bool {
        self.limiter.check_key(&ip).is_ok()
    }
}
```

**Step 1.3 — Greenhouse handler**

File: `lazyjob-core/src/webhooks/handlers/greenhouse.rs`

Greenhouse webhook payload (abridged — only fields LazyJob uses):

```rust
#[derive(serde::Deserialize)]
pub struct GreenhouseWebhookPayload {
    pub action: String,
    pub payload: GreenhousePayloadInner,
}

#[derive(serde::Deserialize)]
pub struct GreenhousePayloadInner {
    pub job: Option<GreenhouseJobPayload>,
}

#[derive(serde::Deserialize)]
pub struct GreenhouseJobPayload {
    pub id: i64,
    pub title: String,
    pub status: String,    // "open" | "closed"
    pub content: String,   // HTML description
    pub departments: Vec<GreenhouseDepartment>,
    pub offices: Vec<GreenhouseOffice>,
    pub updated_at: String, // ISO 8601
}
```

Handler (axum 0.7 style):

```rust
pub async fn handle_greenhouse(
    State(state): State<Arc<WebhookServerState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. IP allowlist check
    if !state.config.ip_allowlist.is_empty()
        && !state.config.ip_allowlist.contains(&addr.ip())
    {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }

    // 2. Rate limit
    if !state.rate_limiter.check(addr.ip()) {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    }

    // 3. Log delivery before any processing (ensures we can retry on crash)
    let delivery_id = Uuid::new_v4();
    if let Err(e) = state.repo.log_delivery_received(
        delivery_id, WebhookSource::Greenhouse, &body,
        headers.get("X-Greenhouse-Signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    ).await {
        tracing::error!(?e, "failed to log webhook delivery");
        return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
    }

    // 4. Verify signature
    let secret_ref = match state.config.secrets.get("greenhouse") {
        Some(s) => s.expose_secret().as_bytes().to_vec(),
        None => {
            tracing::warn!("greenhouse webhook received but no secret configured — accepting");
            vec![]
        }
    };
    let sig = headers.get("X-Greenhouse-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secret_ref.is_empty() && !SignatureVerifier::verify_greenhouse(&secret_ref, &body, sig) {
        let _ = state.repo.mark_delivery_failed(delivery_id, "invalid signature").await;
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // 5. Dispatch to async processing task (return 200 immediately)
    let state_clone = state.clone();
    tokio::spawn(async move {
        process_greenhouse_delivery(state_clone, delivery_id, body).await;
    });

    StatusCode::OK.into_response()
}

async fn process_greenhouse_delivery(
    state: Arc<WebhookServerState>,
    delivery_id: Uuid,
    body: Bytes,
) {
    let _ = state.repo.mark_delivery_processing(delivery_id).await;

    let result: anyhow::Result<()> = async {
        let payload: GreenhouseWebhookPayload = serde_json::from_slice(&body)?;
        let handler = GreenhouseWebhookHandler::new(state.config.clone());
        let events = handler.parse_events_from(&payload)?;

        for event in events {
            match event {
                WebhookJobEvent::Created { job } | WebhookJobEvent::Updated { job } => {
                    state.alert_engine.evaluate_and_notify(&job).await;
                    state.ingestion.ingest_job(job).await?;
                }
                WebhookJobEvent::Closed { source_id } => {
                    state.ingestion.mark_closed("greenhouse", &source_id).await?;
                }
                WebhookJobEvent::Other { action } => {
                    tracing::debug!(%action, "ignored greenhouse webhook action");
                }
            }
        }
        Ok(())
    }.await;

    match result {
        Ok(()) => {
            let _ = state.repo.mark_delivery_succeeded(delivery_id).await;
        }
        Err(e) => {
            tracing::warn!(?e, %delivery_id, "greenhouse webhook processing failed");
            let _ = state.repo.mark_delivery_failed(delivery_id, &e.to_string()).await;
            // Retry worker picks this up — no immediate retry here.
        }
    }
}
```

**Step 1.4 — Lever handler**

File: `lazyjob-core/src/webhooks/handlers/lever.rs`

Lever webhook payload structure:

```rust
#[derive(serde::Deserialize)]
pub struct LeverWebhookPayload {
    #[serde(rename = "type")]
    pub event_type: String,   // "posting_created" | "posting_updated" | "posting_deleted"
    pub data: LeverPosting,
}

#[derive(serde::Deserialize)]
pub struct LeverPosting {
    pub id: String,           // UUID
    pub text: String,         // Job title
    pub state: String,        // "published" | "closed"
    pub content: LeverContent,
    pub tags: Vec<String>,
    pub created_at: i64,      // Unix milliseconds
    pub updated_at: i64,
}

#[derive(serde::Deserialize)]
pub struct LeverContent {
    pub description: String,  // HTML
    pub lists: Vec<LeverList>,
}
```

`handle_lever` mirrors `handle_greenhouse` with:
- Header: `X-Lever-Signature-256` (format: `sha256=<hex>`)
- Signature verifier: `SignatureVerifier::verify_lever`
- Config key: `"lever"`

Normalizer maps `LeverPosting` → canonical `Job`:

```rust
pub fn normalize_lever_posting(posting: &LeverPosting, board_slug: &str) -> Job {
    Job {
        source: "lever".to_string(),
        source_id: posting.id.clone(),
        title: posting.text.clone(),
        company: board_slug.to_string(),  // Board slug acts as company identifier
        description_html: posting.content.description.clone(),
        location: posting.tags.first().cloned().unwrap_or_default(),
        url: format!("https://jobs.lever.co/{}/{}", board_slug, posting.id),
        posted_at: DateTime::from_timestamp_millis(posting.created_at)
            .unwrap_or_else(Utc::now),
        ..Job::default()
    }
}
```

**Step 1.5 — axum Router assembly**

File: `lazyjob-core/src/webhooks/server.rs`

```rust
pub struct WebhookServerState {
    pub config: WebhookConfig,
    pub db: Arc<Database>,
    pub ingestion: Arc<JobIngestionService>,
    pub alert_engine: Arc<AlertRuleEngine>,
    pub repo: Arc<WebhookDeliveryRepository>,
    pub rate_limiter: Arc<RateLimiterState>,
}

impl WebhookServer {
    pub async fn run(self) -> anyhow::Result<()> {
        if !self.config.enabled {
            tracing::info!("webhook server disabled — skipping startup");
            return Ok(());
        }

        let state = Arc::new(WebhookServerState { /* ... */ });

        let app = Router::new()
            .route(
                &format!("{}/greenhouse", self.config.path_prefix),
                post(handle_greenhouse),
            )
            .route(
                &format!("{}/lever", self.config.path_prefix),
                post(handle_lever),
            )
            .with_state(state)
            .layer(
                tower_http::trace::TraceLayer::new_for_http()
                    .make_span_with(|req: &Request<_>| {
                        tracing::info_span!("webhook", method = %req.method(), uri = %req.uri())
                    }),
            );

        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));
        tracing::info!(%addr, "webhook server listening");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await?;

        Ok(())
    }
}
```

**Verification:** Start the server on port 9731. Use `curl -X POST http://localhost:9731/webhooks/greenhouse -H "Content-Type: application/json" -d '{"action":"job_created","payload":{"job":...}}'`. Confirm a new row in `webhook_delivery_log` with `status = 'succeeded'` and a new row in `jobs` via `SELECT * FROM jobs WHERE source='greenhouse'`.

### Phase 2 — Alert Rules Engine

**Step 2.1 — AlertRule types**

File: `lazyjob-core/src/webhooks/alert_rules.rs`

```rust
/// A single user-defined filter. A job passes if ALL non-empty criteria match.
pub struct AlertRule {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub keyword_patterns: Vec<String>,   // OR — any keyword must appear in title/description
    pub company_patterns: Vec<String>,   // OR — any company name substring must match
    pub location_patterns: Vec<String>,  // OR — any location substring must match
    pub min_salary_cents: Option<i64>,
    pub notify_desktop: bool,
}

impl AlertMatcher for AlertRule {
    fn matches(&self, job: &Job) -> bool {
        if !self.enabled {
            return false;
        }

        // All non-empty criteria must individually pass (AND between criteria types)
        let keyword_pass = self.keyword_patterns.is_empty()
            || self.keyword_patterns.iter().any(|kw| {
                let kw_lower = kw.to_lowercase();
                job.title.to_lowercase().contains(&kw_lower)
                    || job.description_sanitized.to_lowercase().contains(&kw_lower)
            });

        let company_pass = self.company_patterns.is_empty()
            || self.company_patterns.iter().any(|pat| {
                job.company.to_lowercase().contains(&pat.to_lowercase())
            });

        let location_pass = self.location_patterns.is_empty()
            || self.location_patterns.iter().any(|pat| {
                job.location.to_lowercase().contains(&pat.to_lowercase())
            });

        let salary_pass = match (self.min_salary_cents, job.salary_min_cents) {
            (Some(min), Some(actual)) => actual >= min,
            (Some(_), None) => false,    // salary required but unknown
            (None, _) => true,
        };

        keyword_pass && company_pass && location_pass && salary_pass
    }
}

pub struct AlertRuleEngine {
    rules: RwLock<Vec<AlertRule>>,
}

impl AlertRuleEngine {
    pub fn new(rules: Vec<AlertRule>) -> Self {
        Self { rules: RwLock::new(rules) }
    }

    pub async fn evaluate_and_notify(&self, job: &Job) {
        let rules = self.rules.read().await;
        for rule in rules.iter().filter(|r| r.matches(job)) {
            if rule.notify_desktop {
                Self::send_desktop_notification(job, &rule.name);
            }
        }
    }

    fn send_desktop_notification(job: &Job, rule_name: &str) {
        let body = format!("{} @ {}\nRule: {}", job.title, job.company, rule_name);
        if let Err(e) = notify_rust::Notification::new()
            .summary("LazyJob: New Job Alert")
            .body(&body)
            .timeout(notify_rust::Timeout::Milliseconds(8000))
            .show()
        {
            tracing::warn!(?e, "desktop notification failed");
        }
    }

    pub async fn reload(&self, rules: Vec<AlertRule>) {
        let mut guard = self.rules.write().await;
        *guard = rules;
    }
}
```

**Step 2.2 — AlertRuleRepository**

```rust
impl WebhookDeliveryRepository {
    pub async fn list_alert_rules(&self) -> Result<Vec<AlertRule>, WebhookError> {
        let rows = sqlx::query_as!(
            AlertRuleRow,
            "SELECT * FROM webhook_alert_rules ORDER BY created_at"
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(AlertRuleRow::into_domain).collect()
    }

    pub async fn upsert_alert_rule(&self, rule: &AlertRule) -> Result<(), WebhookError> {
        sqlx::query!(
            r#"INSERT INTO webhook_alert_rules
               (id, name, enabled, keyword_patterns, company_patterns,
                location_patterns, min_salary_cents, notify_desktop,
                created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   enabled = excluded.enabled,
                   keyword_patterns = excluded.keyword_patterns,
                   company_patterns = excluded.company_patterns,
                   location_patterns = excluded.location_patterns,
                   min_salary_cents = excluded.min_salary_cents,
                   notify_desktop = excluded.notify_desktop,
                   updated_at = excluded.updated_at"#,
            rule.id.to_string(),
            rule.name,
            rule.enabled as i64,
            serde_json::to_string(&rule.keyword_patterns)?,
            serde_json::to_string(&rule.company_patterns)?,
            serde_json::to_string(&rule.location_patterns)?,
            rule.min_salary_cents,
            rule.notify_desktop as i64,
            Utc::now().to_rfc3339(),
            Utc::now().to_rfc3339(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

**Verification:** Insert a rule, deliver a webhook payload that matches it. Confirm `notify-rust` fires. Deliver a non-matching payload. Confirm no notification.

### Phase 3 — Retry Worker (SQLite-backed)

The spec mentions Redis for retry storage. LazyJob is local-first; the retry queue is
a `webhook_delivery_log` table + background `tokio::time::interval` task.

**Step 3.1 — RetryPolicy**

File: `lazyjob-core/src/webhooks/retry.rs`

```rust
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub backoff: [Duration; 3],  // Fixed backoffs: [1s, 5s, 30s]
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: [
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30),
            ],
        }
    }
}

pub struct RetryWorker {
    policy: RetryPolicy,
    repo: Arc<WebhookDeliveryRepository>,
    ingestion: Arc<JobIngestionService>,
    alert_engine: Arc<AlertRuleEngine>,
    interval: Duration,   // How often to poll for failed deliveries. Default: 10s.
}

impl RetryWorker {
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = self.process_pending().await {
                        tracing::warn!(?e, "retry worker sweep failed");
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    }

    async fn process_pending(&self) -> anyhow::Result<()> {
        let failed = self.repo.list_retryable_deliveries(self.policy.max_attempts).await?;

        for delivery in failed {
            let next_attempt = delivery.attempts as usize;
            if next_attempt >= self.policy.backoff.len() {
                self.repo.mark_delivery_abandoned(delivery.id).await?;
                tracing::warn!(delivery_id = %delivery.id, "webhook delivery abandoned after max retries");
                continue;
            }

            // Wait for backoff before retrying (spawn so we don't block the sweep)
            let backoff = self.policy.backoff[next_attempt];
            let repo = self.repo.clone();
            let ingestion = self.ingestion.clone();
            let alert_engine = self.alert_engine.clone();
            let delivery_id = delivery.id;
            let source = delivery.source;
            let payload_bytes = delivery.payload_bytes.clone();

            tokio::spawn(async move {
                tokio::time::sleep(backoff).await;
                retry_delivery(repo, ingestion, alert_engine, delivery_id, source, &payload_bytes).await;
            });
        }

        Ok(())
    }
}
```

**Step 3.2 — List retryable deliveries query**

```rust
pub async fn list_retryable_deliveries(
    &self,
    max_attempts: u8,
) -> Result<Vec<WebhookDelivery>, WebhookError> {
    sqlx::query_as!(
        WebhookDeliveryRow,
        r#"SELECT * FROM webhook_delivery_log
           WHERE status = 'failed'
             AND attempts < ?
           ORDER BY received_at ASC
           LIMIT 50"#,
        max_attempts as i64,
    )
    .fetch_all(&self.pool)
    .await
    .map_err(WebhookError::from)
    .map(|rows| rows.into_iter().map(Into::into).collect())
}
```

**Verification:** Deliver a webhook while SQLite is temporarily locked (simulate by holding a transaction). Confirm `status = 'failed'`. Wait >10 seconds. Confirm retry increments `attempts` and eventually `status = 'succeeded'`. After 3 failures with a permanently broken payload, confirm `status = 'abandoned'`.

### Phase 4 — Email Alert Fallback

For platforms (LinkedIn, Indeed) that don't support webhooks, users configure job alert
emails to forward to a local IMAP inbox. `EmailAlertWatcher` polls that inbox on a
5-minute interval and passes found jobs through the same ingestion path.

**Step 4.1 — IMAP configuration**

```toml
# ~/.config/lazyjob/config.toml
[webhooks.email_fallback]
enabled = false
imap_host = "imap.gmail.com"
imap_port = 993
imap_username = "user@gmail.com"
# Password stored in OS keychain under key "lazyjob::webhook_email"
mailbox = "INBOX"
sender_filter = "@linkedin.com"  # Only process emails from this sender
poll_interval_secs = 300
```

**Step 4.2 — EmailAlertWatcher**

File: `lazyjob-core/src/webhooks/email_watcher.rs`

```rust
pub struct EmailAlertWatcher {
    config: EmailFallbackConfig,
    ingestion: Arc<JobIngestionService>,
    alert_engine: Arc<AlertRuleEngine>,
}

impl EmailAlertWatcher {
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(
            Duration::from_secs(self.config.poll_interval_secs)
        );
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = self.check_inbox().await {
                        tracing::warn!(?e, "email alert watcher poll failed");
                    }
                }
                _ = shutdown.changed() => break,
            }
        }
    }

    async fn check_inbox(&self) -> anyhow::Result<()> {
        let password = keyring::Entry::new("lazyjob", "webhook_email")?
            .get_password()?;

        // async-imap connection
        let tls = async_native_tls::TlsConnector::new();
        let client = async_imap::connect(
            (self.config.imap_host.as_str(), self.config.imap_port),
            &self.config.imap_host,
            tls,
        ).await?;

        let mut session = client
            .login(&self.config.imap_username, &password)
            .await
            .map_err(|(e, _)| e)?;

        session.select(&self.config.mailbox).await?;

        // Fetch unseen messages from the configured sender
        let query = format!("UNSEEN FROM \"{}\"", self.config.sender_filter);
        let uids = session.uid_search(&query).await?;

        for uid in uids {
            let raw = session
                .uid_fetch(uid.to_string(), "(RFC822)")
                .await?
                .into_iter()
                .next();

            if let Some(fetch) = raw {
                if let Some(body) = fetch.body() {
                    if let Err(e) = self.process_email(body).await {
                        tracing::warn!(?e, %uid, "failed to process alert email");
                    }
                }
            }

            // Mark as seen to avoid reprocessing
            session.uid_store(uid.to_string(), "+FLAGS (\\Seen)").await?;
        }

        session.logout().await?;
        Ok(())
    }

    async fn process_email(&self, raw: &[u8]) -> anyhow::Result<()> {
        let parsed = mailparse::parse_mail(raw)?;
        let body_html = parsed.get_body()?;

        // LLM-assisted extraction (falls back to regex for common formats)
        let jobs = extract_jobs_from_email_html(&body_html).await?;

        for job in jobs {
            self.alert_engine.evaluate_and_notify(&job).await;
            self.ingestion.ingest_job(job).await?;
        }

        Ok(())
    }
}

/// Attempt to parse LinkedIn/Indeed job alert email HTML using regex patterns.
/// Falls back to empty Vec on failure (no panic, no LLM in Phase 4 MVP).
pub fn extract_jobs_from_email_html(html: &str) -> Vec<Job> {
    use once_cell::sync::Lazy;
    use regex::Regex;

    // LinkedIn job alert format: <a class="job-card-list__title"> ... </a>
    static LINKEDIN_JOB_TITLE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"class="job-card-list__title"[^>]*>([^<]+)<"#).unwrap()
    });
    static LINKEDIN_COMPANY: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"class="job-card-container__company-name"[^>]*>([^<]+)<"#).unwrap()
    });

    let titles: Vec<&str> = LINKEDIN_JOB_TITLE
        .captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| m.as_str().trim()))
        .collect();

    let companies: Vec<&str> = LINKEDIN_COMPANY
        .captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| m.as_str().trim()))
        .collect();

    titles.into_iter().zip(companies.into_iter())
        .map(|(title, company)| Job {
            source: "email_alert".to_string(),
            source_id: format!("email-{}", uuid::Uuid::new_v4()),
            title: title.to_string(),
            company: company.to_string(),
            ..Job::default()
        })
        .collect()
}
```

**Verification:** Configure Gmail IMAP credentials. Forward a LinkedIn job alert email to the inbox. After one poll cycle, confirm jobs appear in `jobs` table with `source = 'email_alert'`.

### Phase 5 — TUI Configuration and Log Viewer

**Step 5.1 — Webhook config section in config.toml**

```toml
[webhooks]
enabled = false                   # Must be explicitly enabled
port = 9731
path_prefix = "/webhooks"
ip_allowlist = []                 # Empty = allow all (dev mode)
rate_limit_rpm = 60

[webhooks.secrets]
greenhouse = ""                   # Secret goes in keychain: lazyjob::webhook::greenhouse
lever = ""

[webhooks.email_fallback]
enabled = false
```

Secrets are stored in OS keychain using the existing `keyring::Entry` pattern:

```rust
// On first configuration:
keyring::Entry::new("lazyjob", "webhook::greenhouse")?
    .set_password(&secret_value)?;

// At runtime:
let secret = keyring::Entry::new("lazyjob", "webhook::greenhouse")?
    .get_password()
    .unwrap_or_default();
```

**Step 5.2 — Webhook log viewer TUI**

File: `lazyjob-tui/src/views/webhook_log.rs`

A simple `List`-based view accessible at `lazyjob webhooks log` or via a TUI keybind:

```
Webhook Delivery Log                             [r]efresh  [q]uit
──────────────────────────────────────────────────────────────────
ID            Source       Received At         Status     Attempts
──────────────────────────────────────────────────────────────────
d4a1...9f3c  greenhouse   2026-04-16 14:32:11  succeeded  1
f7b2...1e9a  lever        2026-04-16 14:28:47  failed     2
a3c0...8d12  greenhouse   2026-04-16 13:55:02  abandoned  3
```

Key bindings:
- `r` — refresh
- `Enter` — expand selected delivery (show payload bytes as pretty JSON, last error)
- `d` — delete selected delivery row from log
- `q` — close view

Status colors:
- `succeeded` → green
- `pending` / `processing` → yellow
- `failed` → red
- `abandoned` → dark gray

**Step 5.3 — Alert rule editor TUI**

A form-based view for CRUD on `webhook_alert_rules`:

```
Alert Rules                              [n]ew  [e]dit  [d]elete  [q]uit
─────────────────────────────────────────────────────────────────────────
  Name              Keywords                 Companies   Status
  ──────────────────────────────────────────────────────────────
  Senior Rust Jobs  rust, systems, tokio     <any>       enabled
  FAANG Roles       staff engineer, L6       Google,...  enabled
```

Edit modal inputs (validated on submit):
- Name (required)
- Keywords (comma-separated)
- Companies (comma-separated, empty = any)
- Locations (comma-separated, empty = any)
- Min salary ($k notation, e.g. `120k`)
- Desktop notifications (checkbox)

**Verification:** Create an alert rule for `"rust"`. Start the webhook server. POST a Greenhouse payload with a Rust job. Confirm desktop notification fires. Edit the rule to disable it. Re-POST. Confirm no notification.

## Key Crate APIs

- `axum::Router::new().route(path, post(handler))` — route registration
- `axum::extract::ConnectInfo::<SocketAddr>` — peer IP for allowlist check
- `axum::extract::State::<T>` — shared application state
- `hmac::{Hmac, Mac}` + `sha2::Sha256` — signature computation
- `hmac::subtle::ConstantTimeEq` — timing-safe comparison
- `hex::encode(bytes)` — hex string from HMAC output
- `governor::RateLimiter::keyed(quota)` — per-IP keyed rate limiter
- `governor::Quota::per_minute(NonZeroU32)` — rate quota definition
- `tower_http::trace::TraceLayer::new_for_http()` — request tracing middleware
- `axum::serve(listener, app).with_graceful_shutdown(future)` — graceful stop
- `tokio::sync::watch::channel(false)` — shutdown signal
- `tokio::time::interval(duration).set_missed_tick_behavior(Skip)` — retry loop
- `async_imap::connect((host, port), host, tls_connector)` — IMAP connection
- `session.uid_search("UNSEEN FROM ...")` — fetch unseen messages
- `mailparse::parse_mail(raw_bytes)` — RFC 2822 email parsing
- `notify_rust::Notification::new().summary(s).body(b).show()` — desktop alert
- `sqlx::query_as!(Row, sql, params...)` — typed query
- `keyring::Entry::new(service, key).get_password()` — OS keychain read

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum WebhookError {
    #[error("invalid HMAC signature")]
    InvalidSignature,

    #[error("missing required header: {header}")]
    MissingSignatureHeader { header: &'static str },

    #[error("request from unauthorized IP: {ip}")]
    UnauthorizedIp { ip: IpAddr },

    #[error("rate limit exceeded for IP: {ip}")]
    RateLimitExceeded { ip: IpAddr },

    #[error("payload parse failed: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("unknown webhook source")]
    UnknownSource,

    #[error("job ingestion failed: {0}")]
    Ingestion(#[from] anyhow::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("email inbox error: {0}")]
    Imap(String),

    #[error("keychain access failed: {0}")]
    Keychain(String),
}

pub type WebhookResult<T> = std::result::Result<T, WebhookError>;
```

HTTP response mapping:
- `InvalidSignature` → 401
- `UnauthorizedIp` → 403
- `RateLimitExceeded` → 429
- `ParseError` → 400
- `Database` / `Ingestion` → 500 (always log; return 200 to prevent retries from the platform if the payload is valid but we have an internal problem — see note below)

> **Note on 500 vs 200**: Greenhouse and Lever will retry a delivery if they receive a
> non-2xx response. If processing fails due to a SQLite write error (transient), returning
> 500 causes the platform to retry — useful. If processing fails because the payload is
> malformed, returning 500 causes infinite retries — harmful. Strategy: log everything,
> return 200 immediately after signature verification passes, process asynchronously, and
> let the internal `RetryWorker` handle transient failures. Only return 4xx for
> authentication errors (signature/IP) where no retry should occur.

## Testing Strategy

### Unit Tests

**`security.rs` — HMAC verification:**
```rust
#[test]
fn test_verify_greenhouse_signature() {
    let secret = b"test-secret";
    let body = b"test-body";
    // Compute expected using same function
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());
    assert!(SignatureVerifier::verify_greenhouse(secret, body, &expected));
    assert!(!SignatureVerifier::verify_greenhouse(secret, body, "bad"));
}
```

**`alert_rules.rs` — matcher:**
```rust
#[test]
fn test_alert_rule_matches_keyword() {
    let rule = AlertRule {
        keyword_patterns: vec!["rust".to_string()],
        ..AlertRule::default()
    };
    let job = Job { title: "Senior Rust Engineer".to_string(), ..Job::default() };
    assert!(rule.matches(&job));

    let job2 = Job { title: "Senior Python Engineer".to_string(), ..Job::default() };
    assert!(!rule.matches(&job2));
}

#[test]
fn test_alert_rule_empty_criteria_matches_all() {
    let rule = AlertRule { enabled: true, ..AlertRule::default() };
    let job = Job { title: "Any Job".to_string(), ..Job::default() };
    assert!(rule.matches(&job));
}
```

### Integration Tests (sqlx::test)

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_webhook_delivery_log_crud(pool: PgPool) {
    let repo = WebhookDeliveryRepository::new(Arc::new(pool));
    let id = Uuid::new_v4();

    repo.log_delivery_received(id, WebhookSource::Greenhouse, b"payload", "sig").await.unwrap();

    let deliveries = repo.list_retryable_deliveries(3).await.unwrap();
    // Should be empty — status is 'pending', not 'failed'
    assert!(deliveries.is_empty());

    repo.mark_delivery_failed(id, "test error").await.unwrap();
    let deliveries = repo.list_retryable_deliveries(3).await.unwrap();
    assert_eq!(deliveries.len(), 1);
}
```

### Handler Integration Tests (wiremock pattern — server responds to test client)

Because the webhook server is *inbound*, the test approach is reversed: spin up the `WebhookServer` on a random port, send POST requests using `reqwest::Client`, and assert on SQLite state:

```rust
#[tokio::test]
async fn test_greenhouse_handler_valid_signature() {
    let db = setup_test_db().await;
    let ingestion = Arc::new(mock_ingestion_service(db.clone()));
    let server = WebhookServer::new(
        WebhookConfig {
            port: 0,   // OS assigns random port
            secrets: [("greenhouse".into(), Secret::new("test-secret".into()))].into(),
            ip_allowlist: vec![],
            ..Default::default()
        },
        db.clone(),
        ingestion.clone(),
        Arc::new(AlertRuleEngine::new(vec![])),
    );

    let port = server.local_addr().port();
    tokio::spawn(async move { server.run().await.unwrap() });

    let payload = serde_json::json!({
        "action": "job_created",
        "payload": { "job": { "id": 123, "title": "Rust Engineer", ... } }
    });
    let body = serde_json::to_vec(&payload).unwrap();

    let sig = compute_greenhouse_signature(b"test-secret", &body);
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{}/webhooks/greenhouse", port))
        .header("X-Greenhouse-Signature", sig)
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    // Give async processor time to run
    tokio::time::sleep(Duration::from_millis(100)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE source='greenhouse'")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(count, 1);
}
```

## Open Questions

1. **Cloud relay for local-mode users**: Users not on the SaaS plan cannot receive
   webhooks from Greenhouse/Lever without a public IP. The spec asks about a "cloud relay".
   One option: a free relay service at `hooks.lazyjob.app` that buffers and forwards
   payloads over a WebSocket or SSE connection to the local server. This requires the
   SaaS migration path and is deferred post-MVP.

2. **Greenhouse webhook availability**: Greenhouse's real-time webhook API (`Recruiting
   Webhooks`) is part of their enterprise/API tier — it requires an organization-level
   API key and is not available on all plans. The public Job Board API
   (`boards-api.greenhouse.io`) is always polling-based. Clarify with product: is the
   target audience enterprise Greenhouse customers, or should we stick with polling as
   the primary mechanism and treat webhooks as opt-in enhancement?

3. **Lever webhook endpoint discovery**: Lever provides webhooks at the organization
   level (configured in Settings → Integrations → Webhooks). The endpoint URL must be
   reachable from the internet. Document in onboarding: use ngrok in dev, a reverse
   proxy (Caddy/nginx) in production self-hosted mode.

4. **Email regex fragility**: LinkedIn and Indeed change their email alert HTML
   frequently. The `extract_jobs_from_email_html` regex approach will break silently
   (returning no jobs). Phase 4.5 should add an LLM fallback extraction step (similar
   to the offer letter parser in spec 08-gaps-salary-tui) to handle layout changes.

5. **IP allowlist for Greenhouse/Lever**: Neither platform publishes a stable list of
   egress IPs for their webhook senders. The `ip_allowlist` feature should default to
   empty (HMAC signature verification is the primary security mechanism) and document
   clearly that enabling the allowlist requires monitoring webhook delivery failures
   when the platform changes IPs.

6. **Port exposure**: The default port 9731 is a local port. For self-hosted users who
   want real-time webhooks, document that a reverse proxy (Caddy/nginx) must be configured
   to terminate TLS and forward to localhost:9731. Provide a sample `Caddyfile` snippet.

## Related Specs

- `specs/job-search-discovery-engine.md` — primary job ingestion path that webhooks feed
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobIngestionService` interface
- `specs/11-platform-api-integrations.md` — `GreenhouseClient`, `LeverClient`, credential storage
- `specs/XX-authenticated-job-sources.md` — credential management for platform sources
- `specs/application-pipeline-metrics.md` — `NotificationScheduler` (reused for alert delivery)
- `specs/18-saas-migration-path.md` — cloud relay for non-SaaS users (deferred dependency)
