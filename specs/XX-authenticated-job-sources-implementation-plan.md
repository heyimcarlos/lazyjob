# Implementation Plan: Authenticated Job Sources (LinkedIn / Indeed / Glassdoor)

## Status
Draft

## Related Spec
`specs/XX-authenticated-job-sources.md`

## Overview

LazyJob's job discovery currently relies on public APIs (Greenhouse, Lever) that expose
jobs without authentication. The three highest-volume consumer platforms — LinkedIn,
Indeed, and Glassdoor — gate their data behind login sessions. This plan implements
cookie-based session management for these sources using user-supplied browser cookies,
encrypted at rest with the same AES-GCM key as the LazyJob database.

The design does **not** automate login or credentials entry. The user imports a cookie
file exported from a browser extension (e.g., "Get cookies.txt LOCALLY" for LinkedIn),
LazyJob validates the session, encrypts and stores the cookie jar in the OS keychain,
and uses it for subsequent discovery requests. When a session expires, LazyJob pauses
discovery for that source, emits a TUI notification, and prompts the user to re-import
fresh cookies.

Rate limiting (1 req/2 s per source), random inter-request delays, and rotating User-Agent
strings are built into the `AuthenticatedJobSource` trait implementations to reduce the
risk of triggering rate-limit responses. Captcha challenges result in a hard pause for
the affected source — no auto-solving is attempted, consistent with ToS policy.

## Prerequisites

### Implementation Plans Required First
- `specs/11-platform-api-integrations-implementation-plan.md` — `PlatformClient` trait,
  `JobIngestionService`, `platform_sources` SQLite table
- `specs/16-privacy-security-implementation-plan.md` — `keyring::Entry` credential
  storage pattern, `Zeroizing<[u8; 32]>` session key, `age` file encryption
- `specs/XX-master-password-app-unlock-implementation-plan.md` — `Session` struct
  carrying the derived encryption key

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
# Cookie management
cookie          = { version = "0.18", features = ["percent-encode"] }
cookie_store    = { version = "0.21", features = ["reqwest"] }
reqwest         = { version = "0.12", default-features = false, features = [
                    "rustls-tls", "json", "gzip", "cookies"
                ] }

# Secure memory
secrecy         = "0.8"
zeroize         = { version = "1", features = ["derive"] }

# Keyring
keyring         = "2"

# Encryption for cookie jar at rest
aes-gcm         = "0.10"    # AES-256-GCM authenticated encryption
rand            = "0.8"     # nonce generation

# HTML scraping (reuse existing if already in lazyjob-core)
scraper         = "0.19"

# Rate limiting
governor        = "0.6"
nonzero_ext     = "0.3"     # required by governor's const constructors

# Delays
tokio           = { version = "1", features = ["time", "macros", "rt-multi-thread"] }

[dev-dependencies]
wiremock        = "0.6"
tempfile        = "3"
```

## Architecture

### Crate Placement

All authenticated-source code lives in `lazyjob-core/src/auth_sources/`. It is peer to
`lazyjob-core/src/platforms/` (public job sources). The `JobIngestionService` from the
platforms plan drives both families of sources through the same `PlatformClient` trait.

```
lazyjob-core/
  src/
    auth_sources/
      mod.rs            # pub use, module registry
      types.rs          # PlatformCredentials, CookieJar, AuthenticatedSession
      error.rs          # AuthSourceError
      secure_jar.rs     # SecureCookieJar: encrypt/decrypt at rest
      keyring.rs        # CredentialStore wrapping keyring::Entry
      cookie_parser.rs  # parse_netscape_cookies()
      session_health.rs # SessionHealthMonitor background task
      rate_limiter.rs   # PerSourceRateLimiter
      linkedin/
        mod.rs
        client.rs       # LinkedInSource: PlatformClient impl
        auth.rs         # LinkedInAuth: cookie import + session validation
        parser.rs       # parse voyager API job responses
      indeed/
        mod.rs
        client.rs       # IndeedSource: PlatformClient impl
        auth.rs         # IndeedAuth
        parser.rs
      glassdoor/
        mod.rs
        client.rs       # GlassdoorSource: PlatformClient impl
        auth.rs         # GlassdoorAuth
        parser.rs
```

### Core Types

```rust
// lazyjob-core/src/auth_sources/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use secrecy::Secret;

/// Top-level enum for all authentication modes across platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CredentialType {
    /// Serialized Netscape cookie jar, encrypted at rest.
    CookieJar { encrypted_blob: Vec<u8>, nonce: [u8; 12] },
    /// OAuth2 Bearer token (LinkedIn future path only).
    OAuthToken(Secret<String>),
    /// Basic credentials (not used by MVP platforms).
    UsernamePassword { user: String, pass: Secret<String> },
}

/// Identifies a platform for keyring namespacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthPlatform {
    LinkedIn,
    Indeed,
    Glassdoor,
}

impl AuthPlatform {
    /// Returns the keyring service name for this platform.
    pub fn keyring_service(&self) -> &'static str {
        match self {
            Self::LinkedIn  => "lazyjob::auth_source::linkedin",
            Self::Indeed    => "lazyjob::auth_source::indeed",
            Self::Glassdoor => "lazyjob::auth_source::glassdoor",
        }
    }

    /// Human-readable display name used in TUI messages.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::LinkedIn  => "LinkedIn",
            Self::Indeed    => "Indeed",
            Self::Glassdoor => "Glassdoor",
        }
    }

    /// Base URL used for session validation.
    pub fn session_check_url(&self) -> &'static str {
        match self {
            Self::LinkedIn  => "https://www.linkedin.com/voyager/api/me",
            Self::Indeed    => "https://www.indeed.com/account/view",
            Self::Glassdoor => "https://www.glassdoor.com/member/home/index.htm",
        }
    }

    /// Rate limit: minimum milliseconds between requests.
    pub fn min_request_delay_ms(&self) -> u64 {
        2_000 // 1 req/2s per spec
    }

    /// Jitter range added on top of min_request_delay_ms.
    pub fn jitter_ms_range(&self) -> u64 {
        1_500 // [0, 1500) random extra ms
    }
}

/// A validated, decrypted cookie jar ready for use in an HTTP client.
///
/// Wraps `cookie_store::CookieStore` and adds platform metadata.
/// Holds a `Zeroizing<Vec<u8>>` backing buffer for the decrypted bytes
/// so they are wiped when this struct is dropped.
pub struct AuthenticatedSession {
    pub platform: AuthPlatform,
    pub cookie_store: Arc<reqwest::cookie::Jar>,
    pub imported_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Raw decrypted cookie text — zeroed on drop.
    _decrypted_buf: zeroize::Zeroizing<Vec<u8>>,
}

/// Signals emitted by the session health monitor.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    SessionExpired(AuthPlatform),
    CaptchaDetected(AuthPlatform),
    RateLimitHit(AuthPlatform),
    SessionHealthy(AuthPlatform),
}
```

### Trait Definitions

```rust
// Extends the existing PlatformClient trait for cookie-authenticated sources.
// Full signature of PlatformClient is in lazyjob-core/src/platforms/traits.rs.

#[async_trait::async_trait]
pub trait AuthenticatedJobSource: Send + Sync {
    fn platform(&self) -> AuthPlatform;

    /// Validate and import cookies from a Netscape .txt export file.
    /// Returns a decrypted session ready to use.
    async fn import_cookies(
        &self,
        cookie_file: &std::path::Path,
        encryption_key: &[u8; 32],
    ) -> Result<AuthenticatedSession, AuthSourceError>;

    /// Test whether a session cookie jar is still valid against the platform.
    async fn test_session(
        &self,
        session: &AuthenticatedSession,
    ) -> Result<SessionStatus, AuthSourceError>;

    /// Fetch jobs for a given search query using an authenticated session.
    async fn fetch_jobs_authenticated(
        &self,
        query: &AuthenticatedSearchQuery,
        session: &AuthenticatedSession,
    ) -> Result<Vec<crate::platforms::types::DiscoveredJob>, AuthSourceError>;
}

/// Search parameters for authenticated discovery.
#[derive(Debug, Clone)]
pub struct AuthenticatedSearchQuery {
    pub keywords: String,
    pub location: Option<String>,
    pub remote_only: bool,
    pub max_results: usize,
}

/// Result of a session liveness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Valid { expires_at: Option<DateTime<Utc>> },
    Expired,
    CaptchaChallenge,
    RateLimited { retry_after_secs: Option<u64> },
    UnknownError(String),
}
```

### SQLite Schema

```sql
-- Migration: 020_auth_sources.sql

-- Stores metadata about each platform credential set.
-- The actual cookie bytes are stored encrypted in the OS keychain,
-- NOT in SQLite, to avoid leaking them in database backups.
CREATE TABLE IF NOT EXISTS auth_source_credentials (
    id              TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    platform        TEXT NOT NULL,       -- 'linkedin', 'indeed', 'glassdoor'
    imported_at     TEXT NOT NULL,       -- ISO 8601 UTC
    last_validated  TEXT,               -- ISO 8601 UTC
    session_status  TEXT NOT NULL DEFAULT 'unknown',
                                        -- 'valid', 'expired', 'captcha', 'rate_limited'
    expires_at      TEXT,               -- NULL if unknown
    notes           TEXT,               -- user-visible notes
    UNIQUE(platform)
);

-- Discovery job runs for authenticated sources.
-- Reuses the existing platform_sources table but with auth_source_credential_id FK.
ALTER TABLE platform_sources
    ADD COLUMN auth_credential_id TEXT REFERENCES auth_source_credentials(id);

-- Track captcha events for user notifications.
CREATE TABLE IF NOT EXISTS auth_source_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    platform        TEXT NOT NULL,
    event_type      TEXT NOT NULL,  -- 'captcha', 'expired', 'rate_limit', 'healthy'
    occurred_at     TEXT NOT NULL,
    detail          TEXT
);

CREATE INDEX IF NOT EXISTS idx_auth_source_events_platform
    ON auth_source_events(platform, occurred_at DESC);
```

### Module Structure

```
lazyjob-core/
  src/
    auth_sources/
      mod.rs
      types.rs
      error.rs
      secure_jar.rs
      keyring.rs
      cookie_parser.rs
      session_health.rs
      rate_limiter.rs
      linkedin/
        mod.rs
        client.rs
        auth.rs
        parser.rs
      indeed/
        mod.rs
        client.rs
        auth.rs
        parser.rs
      glassdoor/
        mod.rs
        client.rs
        auth.rs
        parser.rs
```

## Implementation Phases

### Phase 1 — Credential Infrastructure (MVP)

#### Step 1.1 — Cookie File Parser

File: `lazyjob-core/src/auth_sources/cookie_parser.rs`

Parse Netscape `cookies.txt` format used by browser export extensions. Each line is:
```
domain  include_subdomains  path  secure  expiry_epoch  name  value
```

```rust
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ParsedCookie {
    pub domain: String,
    pub include_subdomains: bool,
    pub path: String,
    pub secure: bool,
    pub expires: Option<DateTime<Utc>>,
    pub name: String,
    pub value: String,
}

/// Parse a Netscape cookie export file.
///
/// Lines starting with `#` are comments and are skipped.
/// Returns `Err` if the file contains no recognizable cookie lines.
pub fn parse_netscape_cookies(content: &str) -> Result<Vec<ParsedCookie>, CookieParseError> {
    // Implementation detail: split on '\n', strip '\r', skip comment lines.
    // Each data line is split on '\t' into exactly 7 fields.
    // Expiry epoch 0 or missing → None (session cookie).
}

/// Validate that a parsed cookie set contains the required session cookies.
pub fn validate_linkedin_cookies(cookies: &[ParsedCookie]) -> Result<(), AuthSourceError> {
    let has_li_at = cookies.iter().any(|c| c.name == "li_at" && c.domain.contains("linkedin"));
    if !has_li_at {
        return Err(AuthSourceError::MissingRequiredCookie {
            platform: AuthPlatform::LinkedIn,
            cookie_name: "li_at".into(),
        });
    }
    Ok(())
}

pub fn validate_indeed_cookies(cookies: &[ParsedCookie]) -> Result<(), AuthSourceError> {
    // Indeed session cookies: "CTK" (click tracking), "indeed_rcc" (session)
    // At minimum "indeed_rcc" must be present.
}

pub fn validate_glassdoor_cookies(cookies: &[ParsedCookie]) -> Result<(), AuthSourceError> {
    // Glassdoor session cookie: "GSESSIONID"
}
```

**Key APIs:**
- `str::split('\t')` for tab-delimited parsing
- `i64::from_str` → `DateTime::from_timestamp(epoch, 0)` for expiry
- `reqwest::cookie::Jar::add_cookie_str(&format!("{name}={value}"), &url)` to load cookies

**Verification:** Unit test with a known LinkedIn cookie fixture file containing `li_at` passes `validate_linkedin_cookies`. Test with `li_at` missing returns `MissingRequiredCookie`.

---

#### Step 1.2 — Secure Cookie Jar Encryption

File: `lazyjob-core/src/auth_sources/secure_jar.rs`

Serialized cookie text is encrypted with AES-256-GCM using the session encryption key
derived from the master password (from the privacy-security plan). The nonce is random
(12 bytes) and stored alongside the ciphertext.

```rust
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use zeroize::Zeroizing;

/// Serialize a cookie list to bytes and encrypt with AES-256-GCM.
///
/// Returns `(ciphertext, nonce)` both owned.
pub fn encrypt_cookies(
    cookies: &[ParsedCookie],
    key: &[u8; 32],
) -> Result<(Vec<u8>, [u8; 12]), AuthSourceError> {
    // 1. Serialize cookies to a simple text format: one "name=value" per line.
    let plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(serialize_cookies(cookies));

    // 2. Build cipher and random nonce.
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = Aes256Gcm::generate_nonce(&mut OsRng);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 3. Encrypt.
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| AuthSourceError::EncryptionFailed)?;

    Ok((ciphertext, nonce_bytes.into()))
}

/// Decrypt and deserialize a cookie jar.
/// Returns a `Zeroizing<Vec<u8>>` so the caller can pin it in memory.
pub fn decrypt_cookies(
    ciphertext: &[u8],
    nonce: &[u8; 12],
    key: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AuthSourceError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map(Zeroizing::new)
        .map_err(|_| AuthSourceError::DecryptionFailed)
}
```

**Verification:** Round-trip test: encrypt then decrypt returns identical cookie text.
Tampered ciphertext (flip one byte) returns `DecryptionFailed`.

---

#### Step 1.3 — Credential Store (Keyring)

File: `lazyjob-core/src/auth_sources/keyring.rs`

The encrypted cookie blob + nonce is stored in the OS keyring under the namespaced key
`lazyjob::auth_source::{platform}`. The `auth_source_credentials` SQLite table stores
only metadata (platform, status, timestamps) — the actual encrypted blob never touches
SQLite.

```rust
use keyring::Entry;

pub struct CredentialStore;

impl CredentialStore {
    /// Keyring key for a platform's encrypted cookie blob.
    fn entry(platform: AuthPlatform) -> keyring::Result<Entry> {
        Entry::new(platform.keyring_service(), "cookies")
    }

    /// Store base64-encoded "{nonce_hex}:{ciphertext_hex}" string in keyring.
    pub fn store(
        platform: AuthPlatform,
        ciphertext: &[u8],
        nonce: &[u8; 12],
    ) -> Result<(), AuthSourceError> {
        let blob = format!("{}:{}", hex::encode(nonce), hex::encode(ciphertext));
        Self::entry(platform)?.set_password(&blob)?;
        Ok(())
    }

    /// Retrieve and split the stored blob back into (ciphertext, nonce).
    pub fn load(platform: AuthPlatform) -> Result<(Vec<u8>, [u8; 12]), AuthSourceError> {
        let blob = Self::entry(platform)?.get_password()?;
        let (nonce_hex, ct_hex) = blob.split_once(':')
            .ok_or(AuthSourceError::CorruptCredential(platform))?;
        let nonce: [u8; 12] = hex::decode(nonce_hex)?
            .try_into()
            .map_err(|_| AuthSourceError::CorruptCredential(platform))?;
        let ciphertext = hex::decode(ct_hex)?;
        Ok((ciphertext, nonce))
    }

    pub fn delete(platform: AuthPlatform) -> Result<(), AuthSourceError> {
        Self::entry(platform)?.delete_password()?;
        Ok(())
    }

    pub fn exists(platform: AuthPlatform) -> bool {
        Self::entry(platform)
            .and_then(|e| e.get_password())
            .is_ok()
    }
}
```

**Key APIs:**
- `keyring::Entry::new(service, username)` → `Entry`
- `entry.set_password(&str)` / `entry.get_password()` / `entry.delete_password()`
- `hex::encode` / `hex::decode` from the `hex = "0.4"` crate

**Verification:** Store → load → decode gives identical bytes. Delete → load returns `keyring::Error::NoEntry`.

---

#### Step 1.4 — Rate Limiter

File: `lazyjob-core/src/auth_sources/rate_limiter.rs`

Uses `governor` token-bucket for the 1-req/2s limit, plus a `rand`-based jitter sleep
to randomize timing.

```rust
use governor::{Quota, RateLimiter};
use governor::state::{NotKeyed, InMemoryState};
use governor::clock::DefaultClock;
use nonzero_ext::nonzero;
use std::num::NonZeroU32;
use tokio::time::{sleep, Duration};
use rand::Rng;

pub struct PerSourceRateLimiter {
    limiter: RateLimiter<NotKeyed, InMemoryState, DefaultClock>,
    platform: AuthPlatform,
}

impl PerSourceRateLimiter {
    pub fn new(platform: AuthPlatform) -> Self {
        // 1 request per 2 seconds = 30 per minute
        let quota = Quota::per_minute(nonzero!(30u32));
        Self {
            limiter: RateLimiter::direct(quota),
            platform,
        }
    }

    /// Acquire a rate-limit slot, then add platform-specific random jitter.
    pub async fn acquire(&self) {
        self.limiter.until_ready().await;

        // Random extra delay on top of the rate limit bucket.
        let jitter_ms = rand::thread_rng()
            .gen_range(0..self.platform.jitter_ms_range());
        if jitter_ms > 0 {
            sleep(Duration::from_millis(jitter_ms)).await;
        }
    }
}
```

**Verification:** Calling `acquire()` 5 times in a tight loop takes at least 8s for LinkedIn (2s × 4 gaps). Jitter is observable with a histogram of inter-request gaps.

---

### Phase 2 — LinkedIn Source

File: `lazyjob-core/src/auth_sources/linkedin/auth.rs`

LinkedIn's undocumented Voyager API is used rather than the public Marketing API (which
has no job search endpoint). The session cookie `li_at` identifies the user session.

#### Step 2.1 — Authentication Flow

```rust
pub struct LinkedInAuth {
    rate_limiter: Arc<PerSourceRateLimiter>,
}

impl LinkedInAuth {
    pub fn new() -> Self {
        Self {
            rate_limiter: Arc::new(PerSourceRateLimiter::new(AuthPlatform::LinkedIn)),
        }
    }

    /// Import and validate a Netscape cookie file, encrypt, and store in keyring.
    /// Returns a ready-to-use `AuthenticatedSession`.
    pub async fn import_cookies(
        &self,
        cookie_file: &Path,
        encryption_key: &[u8; 32],
    ) -> Result<AuthenticatedSession, AuthSourceError> {
        let content = tokio::fs::read_to_string(cookie_file).await
            .map_err(|e| AuthSourceError::FileRead(e))?;

        let cookies = parse_netscape_cookies(&content)?;
        validate_linkedin_cookies(&cookies)?;

        let jar = build_reqwest_jar(&cookies);
        let session = AuthenticatedSession {
            platform: AuthPlatform::LinkedIn,
            cookie_store: Arc::new(jar),
            imported_at: Utc::now(),
            expires_at: earliest_expiry(&cookies),
            _decrypted_buf: Zeroizing::new(content.into_bytes()),
        };

        // Validate live before persisting.
        let status = self.test_session(&session).await?;
        if !matches!(status, SessionStatus::Valid { .. }) {
            return Err(AuthSourceError::SessionInvalid {
                platform: AuthPlatform::LinkedIn,
                status,
            });
        }

        // Encrypt and store in keyring.
        let (ct, nonce) = encrypt_cookies(&cookies, encryption_key)?;
        CredentialStore::store(AuthPlatform::LinkedIn, &ct, &nonce)?;

        Ok(session)
    }

    /// Test session liveness by calling `/voyager/api/me`.
    pub async fn test_session(
        &self,
        session: &AuthenticatedSession,
    ) -> Result<SessionStatus, AuthSourceError> {
        self.rate_limiter.acquire().await;

        let client = build_client_with_session(session, LINKEDIN_UA);
        let resp = client
            .get("https://www.linkedin.com/voyager/api/me")
            .header("Csrf-Token", extract_csrf_token(session))
            .send()
            .await?;

        match resp.status().as_u16() {
            200 => Ok(SessionStatus::Valid { expires_at: session.expires_at }),
            401 | 403 => {
                let body = resp.text().await.unwrap_or_default();
                if body.contains("captcha") || body.contains("CAPTCHA") {
                    Ok(SessionStatus::CaptchaChallenge)
                } else {
                    Ok(SessionStatus::Expired)
                }
            }
            429 => Ok(SessionStatus::RateLimited {
                retry_after_secs: resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok()),
            }),
            code => Ok(SessionStatus::UnknownError(format!("HTTP {code}"))),
        }
    }
}

/// The CSRF token required by Voyager API is the `JSESSIONID` cookie value.
fn extract_csrf_token(session: &AuthenticatedSession) -> String {
    // In practice: query the cookie jar for domain ".linkedin.com", name "JSESSIONID".
    // reqwest::cookie::Jar does not expose a public getter, so we store the CSRF token
    // separately in `AuthenticatedSession.csrf_token: Option<String>`.
    session.csrf_token.clone().unwrap_or_default()
}
```

#### Step 2.2 — LinkedIn Job Search Client

File: `lazyjob-core/src/auth_sources/linkedin/client.rs`

The Voyager API endpoint for job search:
```
GET https://www.linkedin.com/voyager/api/jobs/search
    ?keywords=rust+engineer
    &location=San+Francisco
    &count=25
    &start=0
```

Response is a `voyager.dash.jobs.JobPostingCollection` JSON blob. Key fields:

```rust
// Minimal serde structs for the Voyager job search response.
#[derive(Deserialize)]
struct VoyagerJobsResponse {
    elements: Vec<VoyagerJobElement>,
}

#[derive(Deserialize)]
struct VoyagerJobElement {
    #[serde(rename = "entityUrn")]
    entity_urn: String,          // "urn:li:fsd_jobPosting:1234567"
    title: String,
    #[serde(rename = "companyDetails")]
    company_details: Option<VoyagerCompanyDetails>,
    #[serde(rename = "formattedLocation")]
    formatted_location: Option<String>,
    #[serde(rename = "listedAt")]
    listed_at: Option<i64>,      // Unix epoch milliseconds
    #[serde(rename = "workRemoteAllowed")]
    work_remote_allowed: Option<bool>,
    description: Option<VoyagerJobDescription>,
}

#[derive(Deserialize)]
struct VoyagerCompanyDetails {
    company: Option<String>,     // company name
}

#[derive(Deserialize)]
struct VoyagerJobDescription {
    text: Option<String>,
}
```

Parser (`linkedin/parser.rs`):

```rust
pub fn parse_voyager_jobs(
    raw: &VoyagerJobsResponse,
) -> Vec<crate::platforms::types::DiscoveredJob> {
    raw.elements.iter().filter_map(|el| {
        let source_id = extract_job_id_from_urn(&el.entity_urn)?;
        let company_name = el.company_details
            .as_ref()
            .and_then(|c| c.company.clone())
            .unwrap_or_default();

        Some(DiscoveredJob {
            source: "linkedin".into(),
            source_id,
            title: el.title.clone(),
            company_name,
            company_id: None, // resolved by JobIngestionService
            location: el.formatted_location.clone(),
            remote: el.work_remote_allowed.unwrap_or(false),
            description_html: el.description.as_ref().and_then(|d| d.text.clone()),
            posted_at: el.listed_at.and_then(|ms| DateTime::from_timestamp_millis(ms)),
            salary_raw: None, // LinkedIn salary data requires additional API calls
            url: Some(format!(
                "https://www.linkedin.com/jobs/view/{}/",
                extract_job_id_from_urn(&el.entity_urn).unwrap_or_default()
            )),
        })
    }).collect()
}

fn extract_job_id_from_urn(urn: &str) -> Option<String> {
    // "urn:li:fsd_jobPosting:1234567" -> "1234567"
    urn.split(':').last().map(String::from)
}
```

Full client:

```rust
pub struct LinkedInSource {
    auth: LinkedInAuth,
    rate_limiter: Arc<PerSourceRateLimiter>,
    session: Arc<tokio::sync::RwLock<Option<AuthenticatedSession>>>,
}

impl LinkedInSource {
    /// Paginate through job search results up to `query.max_results`.
    pub async fn fetch_jobs_authenticated(
        &self,
        query: &AuthenticatedSearchQuery,
    ) -> Result<Vec<DiscoveredJob>, AuthSourceError> {
        let session_guard = self.session.read().await;
        let session = session_guard.as_ref()
            .ok_or(AuthSourceError::NoSession(AuthPlatform::LinkedIn))?;

        let client = build_client_with_session(session, LINKEDIN_UA);
        let mut results = Vec::new();
        let mut start = 0usize;
        let page_size = 25usize;

        loop {
            self.rate_limiter.acquire().await;

            let resp = client
                .get("https://www.linkedin.com/voyager/api/jobs/search")
                .query(&[
                    ("keywords", query.keywords.as_str()),
                    ("count",    &page_size.to_string()),
                    ("start",    &start.to_string()),
                ])
                .header("Csrf-Token", extract_csrf_token(session))
                .header("X-Li-Lang", "en_US")
                .send()
                .await?;

            match detect_session_problem(&resp) {
                Some(SessionStatus::CaptchaChallenge) =>
                    return Err(AuthSourceError::CaptchaRequired(AuthPlatform::LinkedIn)),
                Some(SessionStatus::Expired) =>
                    return Err(AuthSourceError::SessionExpired(AuthPlatform::LinkedIn)),
                Some(SessionStatus::RateLimited { retry_after_secs }) =>
                    return Err(AuthSourceError::RateLimited {
                        platform: AuthPlatform::LinkedIn,
                        retry_after_secs,
                    }),
                _ => {}
            }

            let body: VoyagerJobsResponse = resp.json().await?;
            let page = parse_voyager_jobs(&body);
            let fetched = page.len();
            results.extend(page);

            if fetched < page_size || results.len() >= query.max_results {
                break;
            }
            start += page_size;
        }

        results.truncate(query.max_results);
        Ok(results)
    }
}

const LINKEDIN_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
```

**Verification:** `wiremock`-based test with `with_base_url()` constructor returns parsed jobs. A `403` response triggers `AuthSourceError::SessionExpired`.

---

### Phase 3 — Indeed Source

File: `lazyjob-core/src/auth_sources/indeed/`

Indeed exposes a public search API at `https://www.indeed.com/jobs?q=...&l=...` (HTML
scraping) and a semi-public REST API at `https://api.indeed.com/ads/apisearch` (requires
publisher ID, not generally available). For the MVP, scraping the HTML with `scraper` is
used. Authentication unlocks salary data and some employer-posted listings.

```rust
pub struct IndeedSource {
    rate_limiter: Arc<PerSourceRateLimiter>,
    session: Arc<tokio::sync::RwLock<Option<AuthenticatedSession>>>,
}
```

Key scraping selectors (from the current Indeed DOM, subject to change):
- Job cards: `div[data-jk]` — `data-jk` attribute is the job ID
- Title: `.jobTitle > span[title]`
- Company: `[data-testid="company-name"]`
- Location: `[data-testid="text-location"]`
- Salary: `[data-testid="attribute_snippet_testid"]` containing `$`

```rust
// indeed/parser.rs
use scraper::{Html, Selector};

pub fn parse_indeed_search_page(html: &str) -> Vec<DiscoveredJob> {
    let doc = Html::parse_document(html);
    let card_sel = Selector::parse("div[data-jk]").unwrap();
    // Extract fields per card using named selectors.
    // Return DiscoveredJob with source="indeed".
}
```

Session cookie required: `indeed_rcc`. The auth flow follows the same Netscape import
pattern as LinkedIn but with `validate_indeed_cookies()`.

**Verification:** Fixture HTML file from a real Indeed search result parses to N jobs with expected fields.

---

### Phase 4 — Glassdoor Source

File: `lazyjob-core/src/auth_sources/glassdoor/`

Glassdoor has an undocumented GraphQL API at `https://www.glassdoor.com/graph` used by
their SPA. Session cookie is `GSESSIONID`.

```rust
const GLASSDOOR_GRAPHQL_URL: &str = "https://www.glassdoor.com/graph";

const JOB_SEARCH_QUERY: &str = r#"
    query JobSearchQuery($keyword: String!, $locationId: Int, $numResults: Int) {
        jobListings(
            contextHolder: {
                searchParams: {
                    keyword: $keyword,
                    locationId: $locationId,
                    numResults: $numResults
                }
            }
        ) {
            jobListings {
                jobview {
                    job {
                        listingId
                        jobTitleText
                        employerNameFromSearch
                        locationName
                        salarySourceModel {
                            salaryLow
                            salaryHigh
                        }
                    }
                }
            }
        }
    }
"#;
```

**Note:** Glassdoor's GraphQL schema is undocumented and can change without notice.
The implementation must handle `serde_json::Error` gracefully per call and degrade to
an empty result set rather than aborting discovery. This is explicitly flagged in the
error as `AuthSourceError::ParseFailed { source: "glassdoor", .. }`.

**Verification:** `wiremock` fixture of the GraphQL response parses to expected `DiscoveredJob` list.

---

### Phase 5 — Session Health Monitor

File: `lazyjob-core/src/auth_sources/session_health.rs`

A background tokio task polls each configured platform session every 30 minutes and emits
`SessionEvent` on a `tokio::sync::broadcast::Sender<SessionEvent>`. The TUI subscribes to
display a notification badge when a session expires.

```rust
pub struct SessionHealthMonitor {
    db: Arc<Database>,
    linkedin: Arc<LinkedInAuth>,
    indeed: Arc<IndeedAuth>,
    glassdoor: Arc<GlassdoorAuth>,
    event_tx: broadcast::Sender<SessionEvent>,
    encryption_key: Arc<Zeroizing<[u8; 32]>>,
}

impl SessionHealthMonitor {
    /// Spawn a background task that checks session health every 30 minutes.
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1800));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                self.check_all().await;
            }
        })
    }

    async fn check_all(&self) {
        for platform in [AuthPlatform::LinkedIn, AuthPlatform::Indeed, AuthPlatform::Glassdoor] {
            if !CredentialStore::exists(platform) {
                continue;
            }
            match self.check_one(platform).await {
                Ok(event) => {
                    let _ = self.event_tx.send(event.clone());
                    self.record_event(&event).await;
                }
                Err(e) => {
                    tracing::warn!(%platform, err = %e, "session health check failed");
                }
            }
        }
    }

    async fn check_one(&self, platform: AuthPlatform) -> Result<SessionEvent, AuthSourceError> {
        let session = self.load_session(platform).await?;
        let status = match platform {
            AuthPlatform::LinkedIn  => self.linkedin.test_session(&session).await?,
            AuthPlatform::Indeed    => self.indeed.test_session(&session).await?,
            AuthPlatform::Glassdoor => self.glassdoor.test_session(&session).await?,
        };
        match status {
            SessionStatus::Valid { .. }      => Ok(SessionEvent::SessionHealthy(platform)),
            SessionStatus::Expired           => Ok(SessionEvent::SessionExpired(platform)),
            SessionStatus::CaptchaChallenge  => Ok(SessionEvent::CaptchaDetected(platform)),
            SessionStatus::RateLimited { .. }=> Ok(SessionEvent::RateLimitHit(platform)),
            SessionStatus::UnknownError(msg) => {
                tracing::warn!(%platform, %msg, "unknown session status");
                Ok(SessionEvent::SessionHealthy(platform)) // treat as healthy, log only
            }
        }
    }

    /// Load and decrypt a session from keyring using the session key.
    async fn load_session(&self, platform: AuthPlatform) -> Result<AuthenticatedSession, AuthSourceError> {
        let (ct, nonce) = CredentialStore::load(platform)?;
        let decrypted = decrypt_cookies(&ct, &nonce, &self.encryption_key)?;
        let content = String::from_utf8(decrypted.to_vec())
            .map_err(|_| AuthSourceError::CorruptCredential(platform))?;
        let cookies = parse_netscape_cookies(&content)?;
        let jar = build_reqwest_jar(&cookies);
        Ok(AuthenticatedSession {
            platform,
            cookie_store: Arc::new(jar),
            imported_at: Utc::now(), // approximation for health checks
            expires_at: earliest_expiry(&cookies),
            _decrypted_buf: Zeroizing::new(content.into_bytes()),
        })
    }

    async fn record_event(&self, event: &SessionEvent) {
        // INSERT INTO auth_source_events
        // UPDATE auth_source_credentials SET session_status = ...
    }
}
```

**Verification:** Integration test: mock `test_session` returning `SessionStatus::Expired` → `SessionEvent::SessionExpired` emitted on channel, SQLite row updated.

---

### Phase 6 — TUI Integration

#### Cookie Import Flow

The TUI exposes a per-platform setup panel in the Sources Settings view. The flow:

1. User presses `[i]` on the LinkedIn row → `CookieImportDialog` opens.
2. User pastes a file path to the cookie file (or presses `[f]` to open a file picker).
3. LazyJob calls `LinkedInAuth::import_cookies(path, encryption_key).await`.
4. On success: dialog closes, row shows `● Session active` in green.
5. On `MissingRequiredCookie`: dialog shows `"Cookie 'li_at' not found. Export all cookies for linkedin.com."`.
6. On `SessionInvalid`: dialog shows `"Session has expired. Please log in to LinkedIn in your browser and re-export cookies."`.

#### Session Expired Notification

When `SessionHealthMonitor` emits `SessionEvent::SessionExpired(platform)`, the TUI
status bar shows a `⚠ LinkedIn session expired` badge in yellow. Pressing `[e]` on that
badge jumps directly to the cookie import dialog for that platform.

```rust
// lazyjob-tui/src/views/sources_settings.rs

pub struct SourcesSettingsView {
    platforms: Vec<PlatformRow>,
    selected: usize,
    import_dialog: Option<CookieImportDialog>,
    session_events: broadcast::Receiver<SessionEvent>,
}

pub struct PlatformRow {
    pub platform: AuthPlatform,
    pub status: PlatformStatus,  // Unconfigured / Active / Expired / CaptchaRequired
}

pub enum PlatformStatus {
    Unconfigured,
    Active { last_validated: DateTime<Utc> },
    Expired,
    CaptchaRequired,
    RateLimited { retry_after: Option<DateTime<Utc>> },
}
```

Widget rendering:
- `PlatformStatus::Active` → `●` in green + `"Session active (validated 2h ago)"`
- `PlatformStatus::Expired` → `⚠` in yellow + `"Session expired — press [i] to re-import"`
- `PlatformStatus::CaptchaRequired` → `✗` in red + `"Captcha required — solve in browser, then re-import"`

**Verification:** Manual test: import a valid LinkedIn cookie file → status shows Active.
Import a file missing `li_at` → error message in dialog. Kill the session by tampering
the keyring entry → monitor emits Expired on next check → TUI badge appears.

---

## Key Crate APIs

| Operation | API |
|-----------|-----|
| Parse Netscape cookies | Manual `str::split('\t')` — no crate available |
| Build reqwest cookie jar | `reqwest::cookie::Jar::new()` + `jar.add_cookie_str(str, &url)` |
| Authenticated HTTP client | `reqwest::Client::builder().cookie_provider(Arc::clone(&jar)).user_agent(ua).build()` |
| AES-256-GCM encrypt | `aes_gcm::Aes256Gcm::new(&key).encrypt(&nonce, plaintext)` |
| AES-256-GCM decrypt | `cipher.decrypt(&nonce, ciphertext)` |
| Random nonce | `Aes256Gcm::generate_nonce(&mut aes_gcm::aead::OsRng)` |
| Zeroize secrets | `zeroize::Zeroizing::new(buf)` — zeroed on drop |
| OS keyring store | `keyring::Entry::new(service, username).set_password(&str)` |
| OS keyring load | `entry.get_password() -> Result<String, keyring::Error>` |
| Rate limiting | `governor::RateLimiter::direct(quota).until_ready().await` |
| Jitter | `rand::thread_rng().gen_range(0..max_ms)` → `tokio::time::sleep(Duration::from_millis(ms)).await` |
| HTML scraping | `scraper::Html::parse_document(&str)` + `Selector::parse("css")` |
| Background task | `tokio::spawn(async move { loop { interval.tick().await; ... } })` |
| Session event channel | `tokio::sync::broadcast::channel::<SessionEvent>(32)` |

## Error Handling

```rust
// lazyjob-core/src/auth_sources/error.rs

#[derive(thiserror::Error, Debug)]
pub enum AuthSourceError {
    #[error("missing required cookie '{cookie_name}' for {platform:?}")]
    MissingRequiredCookie {
        platform: AuthPlatform,
        cookie_name: String,
    },

    #[error("{platform:?} session is not valid: {status:?}")]
    SessionInvalid {
        platform: AuthPlatform,
        status: SessionStatus,
    },

    #[error("{platform:?} session has expired — re-import cookies")]
    SessionExpired(AuthPlatform),

    #[error("{platform:?} requires captcha — solve in browser then re-import cookies")]
    CaptchaRequired(AuthPlatform),

    #[error("{platform:?} rate limited; retry after {retry_after_secs:?}s")]
    RateLimited {
        platform: AuthPlatform,
        retry_after_secs: Option<u64>,
    },

    #[error("no session configured for {0:?}")]
    NoSession(AuthPlatform),

    #[error("corrupt or missing credential for {0:?}")]
    CorruptCredential(AuthPlatform),

    #[error("encryption failed")]
    EncryptionFailed,

    #[error("decryption failed — wrong key or corrupt data")]
    DecryptionFailed,

    #[error("cookie parse failed: {0}")]
    CookieParse(String),

    #[error("failed to read cookie file: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("response parse failed for source '{source}': {reason}")]
    ParseFailed { source: String, reason: String },

    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
}
```

## Testing Strategy

### Unit Tests

1. **`cookie_parser.rs`** — Round-trip: serialize → parse a known cookie fixture.
   Test `validate_linkedin_cookies` with/without `li_at`. Test expiry epoch parsing.

2. **`secure_jar.rs`** — Encrypt then decrypt returns identical plaintext.
   Tampered ciphertext returns `DecryptionFailed`. Zero-length plaintext succeeds.

3. **`rate_limiter.rs`** — Call `acquire()` 3 times, measure elapsed ≥ 4s for LinkedIn.

4. **`linkedin/parser.rs`** — Feed a captured Voyager JSON fixture, assert parsed job count,
   title, company, and source_id extraction from URN.

5. **`indeed/parser.rs`** — Feed a captured Indeed search HTML fixture, assert N jobs extracted.

6. **`glassdoor/parser.rs`** — Feed a captured GraphQL JSON fixture, assert parsed jobs.

### Integration Tests

1. **LinkedIn import flow** — Use `wiremock` to mock `https://www.linkedin.com/voyager/api/me`
   returning `200`. Call `import_cookies()` with a fixture cookie file. Assert session stored
   in `CredentialStore` (checked by `exists()`).

2. **LinkedIn expired session** — Mock `/voyager/api/me` returning `401`. Assert `import_cookies`
   returns `AuthSourceError::SessionInvalid`.

3. **Captcha detection** — Mock returning `403` with body containing "captcha". Assert
   `test_session` returns `SessionStatus::CaptchaChallenge`.

4. **Rate limit response** — Mock returning `429` with `Retry-After: 60`. Assert
   `SessionStatus::RateLimited { retry_after_secs: Some(60) }`.

5. **Session health monitor** — Spin up monitor with mocked auth; call `check_all()`.
   Assert `SessionEvent::SessionExpired` broadcast when mock returns `401`.

6. **SQLite event recording** — After `check_all()` detects expiry, query `auth_source_events`
   and assert a row inserted with `event_type = 'expired'` for the platform.

### TUI Tests

Not automated in Phase 1. Manual verification plan:
- Import valid LinkedIn cookies → status row shows Active.
- Import cookie file missing `li_at` → error message in dialog, no keyring change.
- Trigger expired event manually (tamper keyring) → badge in status bar.

## Open Questions

1. **Voyager API stability**: LinkedIn's Voyager API is undocumented and has changed
   significantly before. The parser should defensively handle missing optional fields
   via `Option<T>` rather than hard-failing on schema drift. Consider adding a
   `voyager_api_version` to `auth_source_credentials` for debugging.

2. **2FA expiry**: LinkedIn sessions from 2FA-protected accounts may expire in 24-72h.
   The health monitor checks every 30 minutes, but users who run LazyJob infrequently
   may find sessions already expired. Consider a startup check before the first
   discovery run, independent of the monitor.

3. **Cookie format variants**: Some browser export extensions use `#HttpOnly_` prefixed
   domain lines. The parser must handle this prefix (strip it, set `http_only = true`).
   This is easy to add but should be tested with each supported browser extension.

4. **ToS risk disclosure**: The TUI import dialog must display a one-time ToS risk
   warning: "Using session cookies may violate LinkedIn's Terms of Service. LazyJob
   is not responsible for account restrictions." The user must confirm before importing.
   This disclosure is stored as `tos_acknowledged_at` in `auth_source_credentials`.

5. **Indeed salary data**: Authenticated Indeed sessions expose salary ranges not shown
   to anonymous visitors. The scraper should be enhanced to extract salary from the
   employer-posted salary field in the authenticated job card, separate from the
   estimated salary model Indeed shows to anonymous users.

6. **Glassdoor GraphQL schema drift**: The query in Phase 4 is based on reverse-engineering
   the Glassdoor SPA. It is likely to break with site updates. Consider storing the raw
   JSON response in a `debug_payload` column in `auth_source_events` (only for `ParseFailed`
   events) to facilitate debugging when schema drift occurs.

## Related Specs
- `specs/11-platform-api-integrations.md` — public (non-auth) sources
- `specs/16-privacy-security.md` — keyring integration, encryption key management
- `specs/XX-master-password-app-unlock.md` — session encryption key derivation
- `specs/XX-ralph-process-orphan-cleanup.md` — startup cleanup before discovery runs
- `specs/job-search-discovery-engine.md` — JobIngestionService that consumes these sources
