# Spec: Real-Time Job Alert Webhooks

## Context

Job discovery currently uses polling (checking for new jobs periodically). Real-time webhooks can deliver new job postings immediately, giving users a competitive advantage. This spec addresses webhook receivers and integration.

## Motivation

- **Speed**: Polling misses jobs for up to 59 minutes between checks
- **Competitive advantage**: Being first to apply matters
- **Resource efficiency**: Server-side webhooks more efficient than constant polling

## Design

### Webhook Receiver Architecture

```rust
pub struct WebhookReceiver {
    port: u16,
    path_prefix: String,
    secret_key: [u8; 32],
}

impl WebhookReceiver {
    /// Start HTTP server to receive webhooks
    pub async fn start(&self) -> Result<()> {
        let app = axum::Router::new()
            .route(&format!("{}/greenhouse", self.path_prefix), post(Self::handle_greenhouse))
            .route(&format!("{}/lever", self.path_prefix), post(Self::handle_lever))
            .layer(axum::extract::Extension(self.secret_key));

        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        axum::Server::bind(&addr).serve(app).await?;
        Ok(())
    }

    /// HMAC signature verification
    pub fn verify_signature(&self, payload: &[u8], signature: &str, secret: &str) -> bool {
        let expected = hmac_sha256(secret.as_bytes(), payload);
        format!("{:x}", expected) == signature
    }
}
```

### Supported Webhook Sources

#### Greenhouse Webhook

Greenhouse has a separate Recruiting Webhooks API (different from the public Job Board API). The Job Board API at `boards-api.greenhouse.io/v1/boards/{board_token}/jobs` is public and requires no authentication, but is pull-based (polling). For real-time push webhooks, Greenhouse offers a separate Webhooks product.

```rust
pub async fn handle_greenhouse(
    headers: Headers,
    body: Bytes,
) -> Result<WebhookEvent> {
    // Verify signature
    if !verify_hmac(&headers, &body, &GREENHOUSE_WEBHOOK_SECRET)? {
        return Err(WebhookError::InvalidSignature);
    }

    let payload: GreenhousePayload = serde_json::from_slice(&body)?;

    match payload.action.as_str() {
        "job_created" => {
            let job = payload.job.into_job();
            EVENT_BUS.emit(JobDiscoveryEvent::NewJob { job });
        }
        "job_updated" => {
            EVENT_BUS.emit(JobDiscoveryEvent::JobUpdated { job_id: payload.job.id });
        }
        _ => { /* Ignore other actions */ }
    }

    Ok(WebhookEvent::Processed)
}
```

#### Lever Webhook

Lever's webhook API sends POST requests to a configured endpoint when jobs are created or updated. The webhook payload contains the full job object.

Similar handling for Lever's webhook format.

### Webhook Security

```rust
pub struct WebhookSecurity {
    ip_allowlist: Vec<IpAddr>,
    hmac_secrets: HashMap<String, String>,
    rate_limit: RateLimiter,
}

impl WebhookSecurity {
    pub fn verify_request(&self, req: &Request) -> Result<()> {
        // Check source IP
        let peer = req.peer_addr()?;
        if !self.ip_allowlist.contains(&peer) {
            return Err(WebhookError::UnauthorizedIp);
        }

        // Verify HMAC signature
        let signature = req.headers()
            .get("X-Webhook-Signature")
            .ok_or(WebhookError::MissingSignature)?;

        let secret = self.hmac_secrets.get(req.path()).ok_or(WebhookError::UnknownSource)?;
        if !self.verify_signature(req.body(), signature, secret) {
            return Err(WebhookError::InvalidSignature);
        }

        Ok(())
    }
}
```

### Retry Queue

If webhook processing fails:

```rust
pub struct WebhookRetryQueue {
    redis: RedisPool,
    max_retries: u8 = 3,
    backoff: Vec<Duration>,  // [1s, 5s, 30s]
}

impl WebhookRetryQueue {
    pub async fn enqueue(&self, event: WebhookEvent) -> Result<()> {
        // Store in Redis with retry metadata
        let retry_key = format!("webhook:retry:{}", Uuid::new_v4());
        redis::cmd("SETEX")
            .arg(&retry_key)
            .arg(self.backoff[0].as_secs())
            .arg(serde_json::to_string(&event)?)
            .query_async(&mut self.redis).await?;
        Ok(())
    }
}
```

### Email-Based Fallback

For platforms without webhooks:

```rust
pub struct EmailInboxWatcher {
    imap_client: ImapClient,
    filter_rules: Vec<FilterRule>,
}

pub struct FilterRule {
    pub from_pattern: String,
    pub subject_pattern: String,
    pub extract_job: fn(&str) -> Option<Job>,
}

impl EmailInboxWatcher {
    pub async fn check_new_emails(&self) -> Result<Vec<Job>> {
        let messages = self.imap_client.search("UNSEEN FROM @linkedin.com")?;

        let mut jobs = vec![];
        for msg in messages {
            let content = self.imap_client.fetch(&msg).await?;
            if let Some(job) = self.extract_job(&content) {
                jobs.push(job);
            }
        }

        Ok(jobs)
    }
}
```

User forwards job alert emails from LinkedIn/Indeed to a dedicated LazyJob inbox.

### Duplicate Prevention

```rust
pub struct WebhookDeduplication {
    seen_jobs: HashSet<String>,  // In-memory for session, Redis for cross-instance
    ttl_hours: u64 = 24,
}

impl WebhookDeduplication {
    pub fn is_duplicate(&self, job: &Job) -> bool {
        let key = format!("{}:{}", job.company, job.title_normalized);
        if self.seen_jobs.contains(&key) {
            return true;
        }
        self.seen_jobs.insert(key);
        false
    }
}
```

## Implementation Notes

- Use `axum` for HTTP server (Rust-native, async)
- ngrok for local dev tunnel
- For production: LazyJob cloud relay or user's own server
- Rate limiting: max 60 requests/minute per source

## Open Questions

1. **Cloud relay**: For users not on SaaS, how to receive webhooks?
2. **Platform coverage**: Which platforms support webhooks?
3. **Verification**: How to verify webhook sender is really Greenhouse?

## Related Specs

- `05-job-discovery-layer.md` - Job discovery
- `job-search-discovery-engine.md` - Discovery engine
- `XX-authenticated-job-sources.md` - Authenticated sources