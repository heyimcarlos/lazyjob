# Implementation Plan: Platform API Integrations

## Status
Draft

## Related Spec
`specs/11-platform-api-integrations.md`

## Overview

LazyJob needs to pull job listings from multiple external sources and normalize them into
a single internal `DiscoveredJob` model stored in SQLite. The sources span two categories:
structured REST APIs (Greenhouse, Lever) and JavaScript-heavy sites that require browser
automation (Workday). LinkedIn is explicitly excluded from production use due to ToS
violations.

The integration layer is built in `lazyjob-core` because it is consumed by multiple
executors: the TUI can trigger on-demand refreshes, Ralph subprocesses run background
discovery loops, and the CLI binary orchestrates scheduled sweeps. A `PlatformClient`
async trait provides the uniform interface; a `PlatformRegistry` manages client instances
keyed by platform name; a `JobIngestionService` drives the full fetch → normalize →
deduplicate → persist pipeline.

Rate limiting is per-client using a token-bucket algorithm (`governor` crate). Retry
logic uses exponential backoff with jitter (`backoff` crate). Credentials for
authenticated sources are stored in the OS keychain (`keyring` crate) and accessed
through newtype wrappers that implement `zeroize::Zeroize`. Browser automation uses
`headless_chrome` for Workday until a more complete Playwright-Rust binding stabilizes.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `SqlitePool`, migrations
- `specs/02-llm-provider-abstraction-implementation-plan.md` — `LLMProvider` trait (used for HTML→text extraction fallback)

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
reqwest     = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip"] }
governor    = "0.6"          # token-bucket rate limiter
backoff     = { version = "0.4", features = ["tokio"] }
scraper     = "0.19"         # CSS selector HTML parsing
html2text   = "0.12"         # HTML → plain text
keyring     = "2"            # OS keychain (libsecret/Keychain/DPAPI)
secrecy     = "0.8"          # Secret<T> wrapper
zeroize     = { version = "1", features = ["derive"] }
async-trait = "0.1"
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4"] }
tracing     = "0.1"
thiserror   = "2"
anyhow      = "1"
tokio       = { version = "1", features = ["macros", "rt-multi-thread", "time"] }

# Browser automation — Workday / JS-heavy boards
headless_chrome = "1"

[dev-dependencies]
wiremock    = "0.6"          # HTTP mock server for unit tests
tempfile    = "3"
tokio       = { version = "1", features = ["full"] }
```

## Architecture

### Crate Placement

All platform integration code lives in `lazyjob-core/src/platforms/`. This keeps it
accessible to both `lazyjob-tui` (on-demand fetch) and `lazyjob-ralph` (background
discovery loops) without introducing a new crate.

The `JobIngestionService` (persistence side) also lives in `lazyjob-core/src/platforms/`
alongside the clients, but depends on the `persistence::JobRepository` from the same crate.

### Core Types

```rust
// lazyjob-core/src/platforms/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Canonical job record produced by every platform client after normalization.
/// This maps 1:1 to the `jobs` SQLite table (minus generated fields like `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredJob {
    /// Platform identifier, e.g. "greenhouse", "lever", "workday".
    pub source: String,
    /// The platform's own opaque job ID (used for deduplication).
    pub source_id: String,
    pub title: String,
    pub company_name: String,
    /// Optional FK into `companies.id` — resolved by `JobIngestionService`.
    pub company_id: Option<String>,
    pub location: Option<String>,
    pub remote: Option<RemoteType>,
    pub url: String,
    /// Plain text, HTML stripped. May be truncated to 64 KiB.
    pub description: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub salary_currency: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteType {
    Remote,
    Hybrid,
    OnSite,
    Unknown,
}

/// Per-client configuration loaded from the LazyJob config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
    /// How long to wait between full refreshes, in seconds.
    pub refresh_interval_secs: u64,
    /// Source-specific settings (board_token, company_id, base_url overrides, etc.).
    pub settings: serde_json::Value,
}

/// Typed wrapper for platform credentials stored in the OS keychain.
/// Implements Zeroize so the secret is wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PlatformCredential(pub secrecy::Secret<String>);
```

### Trait Definitions

```rust
// lazyjob-core/src/platforms/client.rs

use async_trait::async_trait;
use super::types::{DiscoveredJob, PlatformConfig};
use super::error::PlatformError;

/// Uniform interface that every job-board integration must implement.
/// Implementations are constructed once and stored in `PlatformRegistry`.
#[async_trait]
pub trait PlatformClient: Send + Sync {
    /// Unique name for this platform, e.g. `"greenhouse"`.
    fn name(&self) -> &'static str;

    /// Fetch **all** currently open jobs for the configured company/board.
    /// Returns a deduplicated list of normalized `DiscoveredJob` records.
    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>, PlatformError>;

    /// Fetch a single job by the platform's own identifier.
    async fn fetch_job(&self, source_id: &str) -> Result<DiscoveredJob, PlatformError>;

    /// Health check — returns true if the platform endpoint is reachable.
    async fn health_check(&self) -> bool;
}

/// Extension trait for platforms that support credential-based (authenticated) access.
#[async_trait]
pub trait AuthenticatedPlatformClient: PlatformClient {
    /// Load credentials from the OS keychain and validate them.
    async fn authenticate(&mut self) -> Result<(), PlatformError>;

    /// Refresh an expired session/token.
    async fn refresh_credentials(&mut self) -> Result<(), PlatformError>;
}
```

### SQLite Schema

```sql
-- Migration: 010_platform_integrations.sql

-- Tracks which platform+company combinations are configured for discovery.
CREATE TABLE platform_sources (
    id          TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    platform    TEXT NOT NULL,           -- "greenhouse", "lever", "workday"
    company_id  TEXT,                    -- FK to companies.id (nullable)
    company_name TEXT NOT NULL,
    config      TEXT NOT NULL,           -- JSON: board_token, company_slug, etc.
    enabled     INTEGER NOT NULL DEFAULT 1,
    last_fetched_at TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (company_id) REFERENCES companies(id) ON DELETE SET NULL,
    UNIQUE (platform, company_name)
);

-- Deduplication table: one row per (source, source_id) pair.
-- Prevents the same job from being inserted twice across fetch cycles.
CREATE TABLE platform_job_index (
    source      TEXT NOT NULL,
    source_id   TEXT NOT NULL,
    job_id      TEXT NOT NULL,           -- FK to jobs.id
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (source, source_id),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE
);

-- Rate-limit audit log — useful for debugging throttle decisions.
CREATE TABLE platform_request_log (
    id          TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    platform    TEXT NOT NULL,
    url         TEXT NOT NULL,
    status_code INTEGER,
    latency_ms  INTEGER,
    error       TEXT,
    requested_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_platform_sources_platform ON platform_sources(platform);
CREATE INDEX idx_platform_job_index_job ON platform_job_index(job_id);
CREATE INDEX idx_platform_request_log_platform ON platform_request_log(platform, requested_at);
```

### Module Structure

```
lazyjob-core/
  src/
    platforms/
      mod.rs          # PlatformRegistry, JobIngestionService, public re-exports
      client.rs       # PlatformClient + AuthenticatedPlatformClient traits
      types.rs        # DiscoveredJob, RemoteType, PlatformConfig, PlatformCredential
      error.rs        # PlatformError enum
      rate_limit.rs   # RateLimiter wrapper around governor
      registry.rs     # PlatformRegistry
      ingestion.rs    # JobIngestionService (fetch → normalize → dedup → persist)
      greenhouse/
        mod.rs        # GreenhouseClient impl
        types.rs      # Raw Greenhouse API response shapes
      lever/
        mod.rs        # LeverClient impl
        types.rs      # Raw Lever API response shapes
      workday/
        mod.rs        # WorkdayClient impl (headless_chrome)
        types.rs      # Parsed Workday job shapes
      html.rs         # strip_html(), extract_salary_range() helpers
```

## Implementation Phases

---

### Phase 1 — Core Scaffold (MVP: Greenhouse + Lever)

#### Step 1.1 — Module skeleton and error type

Create `lazyjob-core/src/platforms/mod.rs`, `client.rs`, `types.rs`, and `error.rs`.

```rust
// lazyjob-core/src/platforms/error.rs

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PlatformError>;

#[derive(Error, Debug)]
pub enum PlatformError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API returned {status} for {url}: {body}")]
    ApiError { status: u16, url: String, body: String },

    #[error("rate limit exceeded for platform {platform}")]
    RateLimited { platform: String },

    #[error("authentication failed for platform {platform}: {reason}")]
    AuthFailed { platform: String, reason: String },

    #[error("credential not found in keychain: {service}/{account}")]
    CredentialNotFound { service: String, account: String },

    #[error("response deserialization failed: {0}")]
    Deserialize(#[from] serde_json::Error),

    #[error("browser automation failed: {0}")]
    BrowserError(String),

    #[error("platform {platform} is not registered")]
    UnknownPlatform { platform: String },

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

**Verification**: `cargo check` compiles the module tree; error variants are exhaustive.

---

#### Step 1.2 — Rate limiter wrapper

```rust
// lazyjob-core/src/platforms/rate_limit.rs

use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorLimiter,
};
use nonzero_ext::nonzero;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::time::sleep;

pub struct RateLimiter {
    inner: Arc<GovernorLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
}

impl RateLimiter {
    /// Create a new limiter capped at `requests_per_minute`.
    pub fn new(requests_per_minute: u32) -> Self {
        let rpm = NonZeroU32::new(requests_per_minute.max(1)).unwrap();
        let quota = Quota::per_minute(rpm);
        Self {
            inner: Arc::new(GovernorLimiter::direct(quota)),
        }
    }

    /// Block (async) until a token is available.
    pub async fn acquire(&self) {
        loop {
            match self.inner.check() {
                Ok(_) => return,
                Err(not_until) => {
                    let wait = not_until.wait_time_from(governor::clock::DefaultClock::default().now());
                    sleep(wait).await;
                }
            }
        }
    }
}
```

**Crate APIs used:**
- `governor::RateLimiter::direct(quota)` — creates a token bucket
- `governor::Quota::per_minute(n)` — sets burst+fill rate to `n` per minute
- `not_until.wait_time_from(now)` — returns `std::time::Duration` to sleep

**Verification**: Write a unit test that creates a 60 RPM limiter, calls `acquire()` 5 times rapidly, and asserts total elapsed time is < 5 seconds (burst allowance).

---

#### Step 1.3 — HTTP client base with retry

```rust
// lazyjob-core/src/platforms/http.rs

use backoff::{ExponentialBackoff, Error as BackoffError};
use reqwest::{Client, Response, StatusCode};
use crate::platforms::error::PlatformError;

pub struct PlatformHttpClient {
    inner: Client,
    platform: &'static str,
}

impl PlatformHttpClient {
    pub fn new(platform: &'static str) -> Self {
        let inner = Client::builder()
            .user_agent("LazyJob/0.1 (+https://github.com/lazyjob/lazyjob)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { inner, platform }
    }

    /// GET with exponential backoff on transient errors (429, 5xx, network errors).
    pub async fn get_with_retry(&self, url: &str) -> Result<Response, PlatformError> {
        let client = self.inner.clone();
        let url = url.to_string();
        let platform = self.platform;

        let op = || async {
            let resp = client.get(&url).send().await.map_err(|e| {
                BackoffError::Transient { err: PlatformError::Http(e), retry_after: None }
            })?;

            match resp.status() {
                s if s.is_success() => Ok(resp),
                StatusCode::TOO_MANY_REQUESTS => {
                    Err(BackoffError::Transient {
                        err: PlatformError::RateLimited { platform: platform.to_string() },
                        retry_after: None,
                    })
                }
                s if s.is_server_error() => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(BackoffError::Transient {
                        err: PlatformError::ApiError { status: s.as_u16(), url: url.clone(), body },
                        retry_after: None,
                    })
                }
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(BackoffError::Permanent(
                        PlatformError::ApiError { status: s.as_u16(), url: url.clone(), body }
                    ))
                }
            }
        };

        backoff::future::retry(ExponentialBackoff::default(), op)
            .await
            .map_err(|e| match e {
                BackoffError::Permanent(e) | BackoffError::Transient { err: e, .. } => e,
            })
    }
}
```

**Crate APIs used:**
- `backoff::future::retry(policy, async_closure)` — drives exponential backoff
- `backoff::ExponentialBackoff::default()` — 500ms initial, 2× multiplier, max 1min, max 3 retries
- `reqwest::Client::builder().user_agent(...).timeout(...)` — configure client once

---

#### Step 1.4 — HTML utility helpers

```rust
// lazyjob-core/src/platforms/html.rs

use scraper::{Html, Selector};

/// Strip HTML tags and return plain text, truncated to `max_bytes`.
pub fn strip_html(html: &str, max_bytes: usize) -> String {
    let text = html2text::from_read(html.as_bytes(), 120);
    if text.len() > max_bytes {
        text[..max_bytes].to_string()
    } else {
        text
    }
}

/// Attempt to parse a salary range from a free-text string like "$120k-$160k".
/// Returns (min_cents, max_cents) if parseable, otherwise None.
pub fn extract_salary(text: &str) -> Option<(i64, i64)> {
    // Simple regex-free heuristic: find $ amounts
    let text = text.replace(",", "");
    let mut amounts: Vec<i64> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            let mut num = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() || d == '.' {
                    num.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            let multiplier = if text[text.find('$').unwrap_or(0)..].contains('k') || text.contains('K') { 1000 } else { 1 };
            if let Ok(n) = num.parse::<f64>() {
                amounts.push((n * multiplier as f64) as i64);
            }
        }
    }
    match amounts.as_slice() {
        [min, max, ..] => Some((*min, *max)),
        [single] => Some((*single, *single)),
        _ => None,
    }
}
```

---

#### Step 1.5 — Greenhouse client

```rust
// lazyjob-core/src/platforms/greenhouse/types.rs

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct GreenhouseJobListResponse {
    pub jobs: Vec<GreenhouseJob>,
    pub meta: Option<GreenhouseMeta>,
}

#[derive(Deserialize, Debug)]
pub struct GreenhouseJob {
    pub id: u64,
    pub title: String,
    pub absolute_url: String,
    pub location: Option<GreenhouseLocation>,
    pub content: Option<String>,  // HTML description
    pub departments: Vec<GreenhouseDept>,
    pub offices: Vec<GreenhouseOffice>,
    pub updated_at: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct GreenhouseLocation { pub name: String }

#[derive(Deserialize, Debug)]
pub struct GreenhouseDept { pub name: String }

#[derive(Deserialize, Debug)]
pub struct GreenhouseOffice { pub name: String }

#[derive(Deserialize, Debug)]
pub struct GreenhouseMeta { pub total: Option<u64> }
```

```rust
// lazyjob-core/src/platforms/greenhouse/mod.rs

use async_trait::async_trait;
use chrono::Utc;
use tracing::instrument;

use crate::platforms::{
    client::PlatformClient,
    error::{PlatformError, Result},
    html::strip_html,
    rate_limit::RateLimiter,
    types::{DiscoveredJob, RemoteType},
    http::PlatformHttpClient,
};
use super::types::{GreenhouseJobListResponse, GreenhouseJob};

pub struct GreenhouseClient {
    http: PlatformHttpClient,
    limiter: RateLimiter,
    board_token: String,
    company_name: String,
}

impl GreenhouseClient {
    pub fn new(board_token: impl Into<String>, company_name: impl Into<String>) -> Self {
        Self {
            http: PlatformHttpClient::new("greenhouse"),
            limiter: RateLimiter::new(60),
            board_token: board_token.into(),
            company_name: company_name.into(),
        }
    }

    fn normalize(&self, raw: GreenhouseJob) -> DiscoveredJob {
        let description = raw.content
            .as_deref()
            .map(|html| strip_html(html, 65536));

        let remote = raw.location.as_ref()
            .map(|l| classify_remote(&l.name))
            .unwrap_or(RemoteType::Unknown);

        DiscoveredJob {
            source: "greenhouse".to_string(),
            source_id: raw.id.to_string(),
            title: raw.title,
            company_name: self.company_name.clone(),
            company_id: None,
            location: raw.location.map(|l| l.name),
            remote: Some(remote),
            url: raw.absolute_url,
            description,
            department: raw.departments.into_iter().next().map(|d| d.name),
            employment_type: None,
            salary_min: None,
            salary_max: None,
            salary_currency: None,
            posted_at: raw.updated_at.and_then(|s| s.parse().ok()),
            fetched_at: Utc::now(),
        }
    }
}

fn classify_remote(location: &str) -> RemoteType {
    let lower = location.to_lowercase();
    if lower.contains("remote") && lower.contains("hybrid") { RemoteType::Hybrid }
    else if lower.contains("remote") { RemoteType::Remote }
    else if lower.is_empty() { RemoteType::Unknown }
    else { RemoteType::OnSite }
}

#[async_trait]
impl PlatformClient for GreenhouseClient {
    fn name(&self) -> &'static str { "greenhouse" }

    #[instrument(skip(self), fields(board_token = %self.board_token))]
    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>> {
        self.limiter.acquire().await;

        let url = format!(
            "https://boards-api.greenhouse.io/v1/boards/{}/jobs?content=true",
            self.board_token
        );
        tracing::debug!(%url, "fetching greenhouse jobs");

        let resp = self.http.get_with_retry(&url).await?;
        let data: GreenhouseJobListResponse = resp.json().await
            .map_err(PlatformError::Deserialize)?;

        tracing::info!(count = data.jobs.len(), board = %self.board_token, "greenhouse fetch complete");
        Ok(data.jobs.into_iter().map(|j| self.normalize(j)).collect())
    }

    #[instrument(skip(self))]
    async fn fetch_job(&self, source_id: &str) -> Result<DiscoveredJob> {
        self.limiter.acquire().await;

        let url = format!(
            "https://boards-api.greenhouse.io/v1/boards/{}/jobs/{}",
            self.board_token, source_id
        );
        let resp = self.http.get_with_retry(&url).await?;
        let raw: GreenhouseJob = resp.json().await.map_err(PlatformError::Deserialize)?;
        Ok(self.normalize(raw))
    }

    async fn health_check(&self) -> bool {
        let url = format!(
            "https://boards-api.greenhouse.io/v1/boards/{}/jobs",
            self.board_token
        );
        self.http.get_with_retry(&url).await.is_ok()
    }
}
```

**Verification**: Write a `wiremock` test that serves a fixture `greenhouse_jobs.json` and asserts the client returns 3 `DiscoveredJob` records with correct source IDs.

---

#### Step 1.6 — Lever client

```rust
// lazyjob-core/src/platforms/lever/types.rs

use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LeverPosting {
    pub id: String,
    pub text: String,           // job title
    pub categories: LeverCategories,
    pub description: String,    // HTML
    pub descriptionPlain: Option<String>,
    pub additional: Option<String>,
    pub hosted_url: String,
    pub created_at: Option<i64>, // epoch ms
}

#[derive(Deserialize, Debug, Default)]
pub struct LeverCategories {
    pub team: Option<String>,
    pub department: Option<String>,
    pub location: Option<String>,
    pub commitment: Option<String>, // e.g. "Full-time"
    pub level: Option<String>,
}

// Lever returns a plain JSON array, not an object.
pub type LeverResponse = Vec<LeverPosting>;
```

```rust
// lazyjob-core/src/platforms/lever/mod.rs

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use tracing::instrument;

use crate::platforms::{
    client::PlatformClient,
    error::{PlatformError, Result},
    html::strip_html,
    rate_limit::RateLimiter,
    types::{DiscoveredJob, RemoteType},
    http::PlatformHttpClient,
};
use super::types::LeverResponse;

pub struct LeverClient {
    http: PlatformHttpClient,
    limiter: RateLimiter,
    company_slug: String,
}

impl LeverClient {
    pub fn new(company_slug: impl Into<String>) -> Self {
        Self {
            http: PlatformHttpClient::new("lever"),
            limiter: RateLimiter::new(60),
            company_slug: company_slug.into(),
        }
    }
}

#[async_trait]
impl PlatformClient for LeverClient {
    fn name(&self) -> &'static str { "lever" }

    #[instrument(skip(self), fields(company = %self.company_slug))]
    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>> {
        self.limiter.acquire().await;

        let url = format!(
            "https://api.lever.co/v0/postings/{}?mode=json",
            self.company_slug
        );
        let resp = self.http.get_with_retry(&url).await?;
        let postings: LeverResponse = resp.json().await.map_err(PlatformError::Deserialize)?;

        tracing::info!(count = postings.len(), company = %self.company_slug, "lever fetch complete");

        let jobs = postings.into_iter().map(|p| {
            let description = p.descriptionPlain
                .or_else(|| Some(strip_html(&p.description, 65536)));

            let remote = p.categories.location.as_deref()
                .map(|l| {
                    let lower = l.to_lowercase();
                    if lower.contains("remote") { RemoteType::Remote }
                    else { RemoteType::OnSite }
                })
                .unwrap_or(RemoteType::Unknown);

            let posted_at: Option<DateTime<Utc>> = p.created_at
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single());

            DiscoveredJob {
                source: "lever".to_string(),
                source_id: p.id,
                title: p.text,
                company_name: self.company_slug.clone(),
                company_id: None,
                location: p.categories.location,
                remote: Some(remote),
                url: p.hosted_url,
                description,
                department: p.categories.department,
                employment_type: p.categories.commitment,
                salary_min: None,
                salary_max: None,
                salary_currency: None,
                posted_at,
                fetched_at: Utc::now(),
            }
        }).collect();

        Ok(jobs)
    }

    #[instrument(skip(self))]
    async fn fetch_job(&self, source_id: &str) -> Result<DiscoveredJob> {
        self.limiter.acquire().await;
        let url = format!(
            "https://api.lever.co/v0/postings/{}/{}?mode=json",
            self.company_slug, source_id
        );
        let resp = self.http.get_with_retry(&url).await?;
        let posting: crate::platforms::lever::types::LeverPosting = resp.json()
            .await.map_err(PlatformError::Deserialize)?;

        let jobs = self.fetch_jobs().await?;  // simple approach: get list, find by ID
        jobs.into_iter()
            .find(|j| j.source_id == source_id)
            .ok_or_else(|| PlatformError::ApiError {
                status: 404,
                url,
                body: "job not found in listing".to_string(),
            })
    }

    async fn health_check(&self) -> bool {
        let url = format!("https://api.lever.co/v0/postings/{}", self.company_slug);
        self.http.get_with_retry(&url).await.is_ok()
    }
}
```

---

### Phase 2 — Platform Registry and Ingestion Pipeline

#### Step 2.1 — PlatformRegistry

```rust
// lazyjob-core/src/platforms/registry.rs

use std::collections::HashMap;
use std::sync::Arc;

use crate::platforms::{
    client::PlatformClient,
    error::{PlatformError, Result},
};

pub struct PlatformRegistry {
    clients: HashMap<String, Arc<dyn PlatformClient>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self { clients: HashMap::new() }
    }

    pub fn register(&mut self, client: Arc<dyn PlatformClient>) {
        self.clients.insert(client.name().to_string(), client);
    }

    pub fn get(&self, name: &str) -> Result<Arc<dyn PlatformClient>> {
        self.clients.get(name).cloned().ok_or_else(|| PlatformError::UnknownPlatform {
            platform: name.to_string(),
        })
    }

    pub fn all(&self) -> impl Iterator<Item = Arc<dyn PlatformClient>> + '_ {
        self.clients.values().cloned()
    }

    pub fn names(&self) -> Vec<&str> {
        self.clients.keys().map(String::as_str).collect()
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self { Self::new() }
}
```

---

#### Step 2.2 — JobIngestionService

The ingestion service orchestrates: fetch → normalize → dedup → persist.

```rust
// lazyjob-core/src/platforms/ingestion.rs

use std::sync::Arc;
use sqlx::SqlitePool;
use tracing::instrument;

use crate::platforms::{
    client::PlatformClient,
    error::{PlatformError, Result},
    types::DiscoveredJob,
};
use crate::persistence::jobs::{Job, JobRepository};

pub struct JobIngestionService {
    pool: SqlitePool,
}

impl JobIngestionService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Fetch all jobs from a single client, deduplicate, and persist new ones.
    /// Returns the count of newly inserted jobs.
    #[instrument(skip(self, client))]
    pub async fn ingest_from(&self, client: &dyn PlatformClient) -> Result<usize> {
        let jobs = client.fetch_jobs().await?;
        let mut inserted = 0usize;

        for discovered in jobs {
            match self.upsert_job(&discovered).await {
                Ok(true) => inserted += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        source = %discovered.source,
                        source_id = %discovered.source_id,
                        error = %e,
                        "failed to ingest job, skipping"
                    );
                }
            }
        }

        // Update last_fetched_at in platform_sources
        sqlx::query!(
            "UPDATE platform_sources SET last_fetched_at = datetime('now') WHERE platform = ?",
            client.name()
        )
        .execute(&self.pool)
        .await
        .ok(); // non-fatal

        tracing::info!(
            platform = client.name(),
            inserted,
            "ingestion complete"
        );
        Ok(inserted)
    }

    /// Insert if not already in `platform_job_index`. Returns true if newly inserted.
    async fn upsert_job(&self, job: &DiscoveredJob) -> Result<bool> {
        let existing = sqlx::query_scalar!(
            "SELECT job_id FROM platform_job_index WHERE source = ? AND source_id = ?",
            job.source,
            job.source_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| PlatformError::Unexpected(e.into()))?;

        if existing.is_some() {
            return Ok(false);
        }

        // Insert into jobs table
        let job_id = uuid::Uuid::new_v4().to_string();
        sqlx::query!(
            r#"
            INSERT INTO jobs (
                id, title, company_name, location, url, description,
                source, status, discovered_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 'discovered', ?, datetime('now'), datetime('now'))
            "#,
            job_id,
            job.title,
            job.company_name,
            job.location,
            job.url,
            job.description,
            job.source,
            job.fetched_at,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| PlatformError::Unexpected(e.into()))?;

        // Record in dedup index
        sqlx::query!(
            "INSERT INTO platform_job_index (source, source_id, job_id) VALUES (?, ?, ?)",
            job.source,
            job.source_id,
            job_id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| PlatformError::Unexpected(e.into()))?;

        Ok(true)
    }
}
```

**Crate APIs used:**
- `sqlx::query!` — compile-time SQL checking
- `sqlx::query_scalar!` — returns a single scalar value
- `uuid::Uuid::new_v4().to_string()` — generate job IDs

**Verification**: Write an integration test using an in-memory SQLite DB. Register a mock `PlatformClient` that returns 5 jobs. Call `ingest_from()` twice. Assert first call returns 5, second call returns 0 (dedup).

---

### Phase 3 — Workday Browser Automation

Workday has no public API. Jobs are rendered via JavaScript-heavy React/Angular SPAs.
The `headless_chrome` crate drives a system-installed Chromium instance.

```rust
// lazyjob-core/src/platforms/workday/mod.rs

use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions, Tab};
use std::sync::Arc;
use std::time::Duration;
use tracing::instrument;

use crate::platforms::{
    client::PlatformClient,
    error::{PlatformError, Result},
    html::strip_html,
    types::{DiscoveredJob, RemoteType},
};
use super::types::WorkdayJob;

pub struct WorkdayClient {
    base_url: String,
    company_name: String,
    /// Wait up to this duration for the job list to render.
    render_timeout: Duration,
}

impl WorkdayClient {
    pub fn new(
        base_url: impl Into<String>,
        company_name: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            company_name: company_name.into(),
            render_timeout: Duration::from_secs(15),
        }
    }

    fn launch_browser() -> Result<Browser> {
        Browser::new(
            LaunchOptions::default_builder()
                .headless(true)
                .sandbox(false)  // required in containerized environments
                .build()
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?,
        )
        .map_err(|e| PlatformError::BrowserError(e.to_string()))
    }

    fn extract_jobs_from_page(tab: &Arc<Tab>) -> Result<Vec<WorkdayJob>> {
        // Workday renders jobs into `[data-automation-id="job-title"]` elements.
        // We inject a script to collect all visible job data into a JSON array.
        let json = tab.evaluate(r#"
            JSON.stringify(
                Array.from(document.querySelectorAll('[data-automation-id="compositeContainer"]')).map(el => ({
                    title: el.querySelector('[data-automation-id="jobTitle"]')?.innerText ?? '',
                    location: el.querySelector('[data-automation-id="location"]')?.innerText ?? '',
                    posted: el.querySelector('[data-automation-id="postedOn"]')?.innerText ?? '',
                    url: el.querySelector('a[data-automation-id="jobTitle"]')?.href ?? '',
                    id: el.querySelector('a')?.href?.split('/').pop() ?? '',
                }))
            )
        "#, false)
        .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

        let jobs_json = json.value
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| PlatformError::BrowserError("no JSON returned from page".into()))?;

        serde_json::from_str::<Vec<WorkdayJob>>(&jobs_json)
            .map_err(PlatformError::Deserialize)
    }
}

#[async_trait]
impl PlatformClient for WorkdayClient {
    fn name(&self) -> &'static str { "workday" }

    #[instrument(skip(self))]
    async fn fetch_jobs(&self) -> Result<Vec<DiscoveredJob>> {
        let base_url = self.base_url.clone();
        let company_name = self.company_name.clone();
        let timeout = self.render_timeout;

        // headless_chrome is sync; spawn_blocking to avoid blocking the tokio executor.
        let raw_jobs = tokio::task::spawn_blocking(move || -> Result<Vec<WorkdayJob>> {
            let browser = WorkdayClient::launch_browser()?;
            let tab = browser.new_tab()
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            tab.navigate_to(&base_url)
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            tab.wait_for_element_with_custom_timeout(
                "[data-automation-id=\"jobTitle\"]",
                timeout,
            ).map_err(|e| PlatformError::BrowserError(
                format!("timed out waiting for job list: {e}")
            ))?;

            WorkdayClient::extract_jobs_from_page(&tab)
        })
        .await
        .map_err(|e| PlatformError::BrowserError(format!("spawn_blocking panicked: {e}")))?;

        let raw_jobs = raw_jobs?;

        Ok(raw_jobs.into_iter().map(|raw| DiscoveredJob {
            source: "workday".to_string(),
            source_id: raw.id,
            title: raw.title,
            company_name: company_name.clone(),
            company_id: None,
            location: Some(raw.location).filter(|l| !l.is_empty()),
            remote: Some(RemoteType::Unknown),
            url: raw.url,
            description: None, // fetched lazily via fetch_job()
            department: None,
            employment_type: None,
            salary_min: None,
            salary_max: None,
            salary_currency: None,
            posted_at: None,
            fetched_at: chrono::Utc::now(),
        }).collect())
    }

    async fn fetch_job(&self, source_id: &str) -> Result<DiscoveredJob> {
        // Navigate directly to the job detail page and scrape description.
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), source_id);
        let company_name = self.company_name.clone();

        let (description, title) = tokio::task::spawn_blocking(move || -> Result<(String, String)> {
            let browser = WorkdayClient::launch_browser()?;
            let tab = browser.new_tab()
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            tab.navigate_to(&url)
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            let desc_elem = tab.wait_for_element("[data-automation-id=\"jobPostingDescription\"]")
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;
            let description = desc_elem.get_inner_text()
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            let title_elem = tab.find_element("[data-automation-id=\"jobPostingHeader\"]")
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;
            let title = title_elem.get_inner_text()
                .map_err(|e| PlatformError::BrowserError(e.to_string()))?;

            Ok((description, title))
        })
        .await
        .map_err(|e| PlatformError::BrowserError(e.to_string()))??;

        Ok(DiscoveredJob {
            source: "workday".to_string(),
            source_id: source_id.to_string(),
            title,
            company_name,
            company_id: None,
            location: None,
            remote: Some(RemoteType::Unknown),
            url: format!("{}/{}", self.base_url.trim_end_matches('/'), source_id),
            description: Some(strip_html(&description, 65536)),
            department: None,
            employment_type: None,
            salary_min: None,
            salary_max: None,
            salary_currency: None,
            posted_at: None,
            fetched_at: chrono::Utc::now(),
        })
    }

    async fn health_check(&self) -> bool {
        // For Workday, health = can launch browser and navigate to base URL without error.
        let base_url = self.base_url.clone();
        tokio::task::spawn_blocking(move || {
            WorkdayClient::launch_browser()
                .and_then(|b| {
                    b.new_tab()
                        .map_err(|e| PlatformError::BrowserError(e.to_string()))
                        .and_then(|t| {
                            t.navigate_to(&base_url)
                                .map_err(|e| PlatformError::BrowserError(e.to_string()))
                                .map(|_| ())
                        })
                })
                .is_ok()
        })
        .await
        .unwrap_or(false)
    }
}
```

**Crate APIs used:**
- `headless_chrome::Browser::new(LaunchOptions)` — launch browser instance
- `browser.new_tab()` → `Arc<Tab>`
- `tab.navigate_to(&url)` — navigate page
- `tab.wait_for_element_with_custom_timeout(selector, Duration)` — wait for render
- `tab.evaluate(js, false)` — execute JavaScript, get return value
- `tokio::task::spawn_blocking(|| ...)` — offload sync work to blocking thread pool

---

### Phase 4 — Credential Storage and Authenticated Sources

Some future platforms (e.g., an internal corporate ATS) require authentication. The
`keyring` crate stores credentials in the OS keychain.

```rust
// lazyjob-core/src/platforms/credentials.rs

use keyring::Entry;
use secrecy::Secret;
use zeroize::Zeroize;

use crate::platforms::error::{PlatformError, Result};

const SERVICE: &str = "lazyjob";

/// Store a credential (e.g., API token) for a given platform in the OS keychain.
pub fn store_credential(platform: &str, account: &str, secret: &str) -> Result<()> {
    Entry::new(SERVICE, &format!("{platform}/{account}"))
        .map_err(|e| PlatformError::AuthFailed {
            platform: platform.to_string(),
            reason: e.to_string(),
        })?
        .set_password(secret)
        .map_err(|e| PlatformError::AuthFailed {
            platform: platform.to_string(),
            reason: e.to_string(),
        })
}

/// Load a credential from the OS keychain. The returned secret is wrapped in
/// `secrecy::Secret` to prevent accidental logging.
pub fn load_credential(platform: &str, account: &str) -> Result<Secret<String>> {
    let value = Entry::new(SERVICE, &format!("{platform}/{account}"))
        .map_err(|e| PlatformError::CredentialNotFound {
            service: SERVICE.to_string(),
            account: format!("{platform}/{account}"),
        })?
        .get_password()
        .map_err(|_| PlatformError::CredentialNotFound {
            service: SERVICE.to_string(),
            account: format!("{platform}/{account}"),
        })?;

    Ok(Secret::new(value))
}

/// Delete a stored credential.
pub fn delete_credential(platform: &str, account: &str) -> Result<()> {
    Entry::new(SERVICE, &format!("{platform}/{account}"))
        .map_err(|e| PlatformError::AuthFailed {
            platform: platform.to_string(),
            reason: e.to_string(),
        })?
        .delete_password()
        .map_err(|e| PlatformError::AuthFailed {
            platform: platform.to_string(),
            reason: e.to_string(),
        })
}
```

**Crate APIs used:**
- `keyring::Entry::new(service, account)` — handle to a keychain entry
- `entry.set_password(secret)` — write credential
- `entry.get_password()` — read credential
- `entry.delete_password()` — remove credential
- `secrecy::Secret::new(value)` — wrap to prevent logging

---

### Phase 5 — TUI Integration

The TUI needs two entry points:
1. A **status indicator** showing when the last platform sweep ran and how many jobs were found.
2. A **manual refresh trigger** (keybind `R` in the Jobs List view) that fires an on-demand ingestion cycle.

```rust
// lazyjob-tui/src/views/jobs_list.rs (addition)

use tokio::sync::mpsc;

pub enum JobsListAction {
    RefreshPlatforms,
    // ...existing actions...
}

// In the event loop, handle RefreshPlatforms:
//   1. Disable the R keybind (show spinner)
//   2. Spawn a tokio task: ingestion_service.ingest_from(client).await
//   3. Send result count back via mpsc channel
//   4. Re-enable keybind, update status bar with "Fetched N new jobs"
```

The `PlatformRegistry` and `JobIngestionService` are constructed once in `lazyjob-cli/src/main.rs` and passed to the `App` struct via `Arc<>`.

```rust
// lazyjob-cli/src/main.rs (pseudostructure — not pseudocode)

let mut registry = PlatformRegistry::new();
// Load platform_sources from SQLite and hydrate registry
for source in db.list_platform_sources().await? {
    let client: Arc<dyn PlatformClient> = match source.platform.as_str() {
        "greenhouse" => Arc::new(GreenhouseClient::new(
            source.config["board_token"].as_str().unwrap(),
            &source.company_name,
        )),
        "lever" => Arc::new(LeverClient::new(
            source.config["company_slug"].as_str().unwrap(),
        )),
        _ => continue,
    };
    registry.register(client);
}

let ingestion = Arc::new(JobIngestionService::new(db.pool().clone()));
let app = App::new(db, registry, ingestion);
```

---

## Key Crate APIs

| Crate | API | Usage |
|---|---|---|
| `reqwest` | `Client::builder().rustls_tls().build()` | HTTP client for REST APIs |
| `reqwest` | `client.get(url).send().await` | Fire GET request |
| `reqwest` | `response.json::<T>().await` | Deserialize JSON body |
| `governor` | `RateLimiter::direct(Quota::per_minute(n))` | Token bucket rate limiter |
| `governor` | `limiter.check()` → `Ok(())` or `Err(NotUntil)` | Check token availability |
| `backoff` | `backoff::future::retry(ExponentialBackoff::default(), op)` | Retry with jitter |
| `headless_chrome` | `Browser::new(LaunchOptions)` | Launch headless Chrome |
| `headless_chrome` | `tab.navigate_to(url)` | Navigate to URL |
| `headless_chrome` | `tab.wait_for_element_with_custom_timeout(sel, dur)` | Wait for render |
| `headless_chrome` | `tab.evaluate(js, false)` | Execute JavaScript |
| `html2text` | `html2text::from_read(bytes, width)` | Strip HTML to plain text |
| `scraper` | `Html::parse_document(html)` + `Selector::parse(css)` | CSS selector parsing |
| `keyring` | `Entry::new(service, account).get_password()` | Read OS keychain |
| `secrecy` | `Secret::new(value)` + `.expose_secret()` | Safe secret handling |
| `sqlx` | `query_scalar!("SELECT ...", ...)` | Check dedup index |
| `sqlx` | `query!("INSERT INTO ...", ...)` | Persist ingested jobs |
| `uuid` | `Uuid::new_v4().to_string()` | Generate job IDs |
| `tokio` | `task::spawn_blocking(|| ...)` | Offload sync browser ops |

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum PlatformError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API returned {status} for {url}: {body}")]
    ApiError { status: u16, url: String, body: String },

    #[error("rate limit exceeded for platform {platform}")]
    RateLimited { platform: String },

    #[error("authentication failed for platform {platform}: {reason}")]
    AuthFailed { platform: String, reason: String },

    #[error("credential not found in keychain: {service}/{account}")]
    CredentialNotFound { service: String, account: String },

    #[error("response deserialization failed: {0}")]
    Deserialize(#[from] serde_json::Error),

    #[error("browser automation failed: {0}")]
    BrowserError(String),

    #[error("platform {platform} is not registered")]
    UnknownPlatform { platform: String },

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, PlatformError>;
```

Callers match on `PlatformError::RateLimited` to implement soft backoff at the
orchestration layer, and on `PlatformError::AuthFailed` to prompt the user to re-enter
credentials via the TUI.

## Testing Strategy

### Unit Tests

**File**: `lazyjob-core/src/platforms/greenhouse/mod.rs` (inline `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};
    use super::*;

    #[tokio::test]
    async fn greenhouse_normalizes_jobs() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/boards/acme-corp/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 12345,
                    "title": "Senior Rust Engineer",
                    "absolute_url": "https://boards.greenhouse.io/acme/jobs/12345",
                    "location": { "name": "Remote" },
                    "content": "<p>We are hiring a Rust engineer...</p>",
                    "departments": [{ "name": "Engineering" }],
                    "offices": [],
                    "updated_at": null
                }],
                "meta": { "total": 1 }
            })))
            .mount(&server)
            .await;

        let client = GreenhouseClient::new_with_base_url(
            "acme-corp", "Acme Corp", server.uri()
        );
        let jobs = client.fetch_jobs().await.unwrap();

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].source_id, "12345");
        assert_eq!(jobs[0].title, "Senior Rust Engineer");
        assert_eq!(jobs[0].remote, Some(RemoteType::Remote));
        assert!(jobs[0].description.as_deref().unwrap().contains("Rust engineer"));
    }

    #[tokio::test]
    async fn greenhouse_retries_on_500() {
        let server = MockServer::start().await;
        // First call returns 500, second returns 200
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [], "meta": null
            })))
            .mount(&server)
            .await;

        let client = GreenhouseClient::new_with_base_url("acme", "Acme", server.uri());
        let result = client.fetch_jobs().await;
        assert!(result.is_ok());
    }
}
```

**Lever**: Same pattern — `wiremock` serves a JSON array fixture.

**Rate limiter**: Test that a 2-RPM limiter, called 3 times, takes at least 60 seconds
(skip in CI with `#[ignore]`; run locally for correctness).

**Dedup**: In-memory SQLite, ingest same source_id twice, assert second returns `Ok(false)`.

### Integration Tests

**File**: `lazyjob-core/tests/platform_ingestion.rs`

```rust
#[tokio::test]
async fn full_ingest_cycle() {
    // 1. Create in-memory SQLite with all migrations applied
    // 2. Mount wiremock for Greenhouse with 10 job fixtures
    // 3. Run ingest_from() → assert 10 new jobs in jobs table
    // 4. Run ingest_from() again → assert 0 new jobs (dedup)
    // 5. Query jobs table directly, assert all have source="greenhouse"
}
```

### TUI Tests

The TUI refresh trigger is tested by:
1. Creating a mock `PlatformClient` that returns a fixed list
2. Calling the `JobsListAction::RefreshPlatforms` handler
3. Asserting the status bar text updates to "Fetched N new jobs"

No headless browser tests in CI — `WorkdayClient` is integration-tested manually.

## Open Questions

1. **Pagination**: Greenhouse returns all jobs in a single response; Lever does too (no paging). For companies with >200 jobs, Greenhouse supports `?page=N`. Should we add pagination now or defer?
   - Recommendation: defer; add `page` loop when a Greenhouse board exceeds 200 results.

2. **Workday automation stability**: `headless_chrome` relies on Chrome DevTools Protocol. Workday's DOM selectors (`data-automation-id`) are stable but may change. Should we make the CSS selectors configurable per-company in `platform_sources.config`?
   - Recommendation: yes — store selectors as JSON in the config column.

3. **LinkedIn**: ToS prohibits scraping. Exclude entirely. If users explicitly ask, document the risk and provide no official support.

4. **Session persistence for authenticated platforms**: Should cookies from Workday login be persisted across restarts, or re-login each time?
   - Recommendation: persist cookie JSON to keychain, refresh on `401`.

5. **Concurrent multi-platform fetch**: Should `JobIngestionService` fetch all platforms in parallel via `tokio::join!`? Current design is sequential.
   - Recommendation: yes, use `futures::future::join_all(clients.map(|c| ingest(c)))` in Phase 3.

6. **Company resolution**: `DiscoveredJob.company_id` is always `None` after normalization. A separate pass should look up/create `companies` rows and back-fill `jobs.company_id`. Scope this to a follow-on task.

## Related Specs

- `specs/05-job-discovery-layer-implementation-plan.md` — orchestrates when and how platforms are polled
- `specs/16-privacy-security.md` — credential encryption at rest
- `specs/XX-authenticated-job-sources.md` — OAuth flows and session refresh
- `specs/XX-application-cross-source-deduplication.md` — deduplication strategy beyond source_id hashing
- `specs/09-tui-design-keybindings-implementation-plan.md` — keybind for manual refresh trigger
