# Implementation Plan: SaaS Migration Path

## Status
Draft

## Related Spec
`specs/18-saas-migration-path.md`

## Overview

LazyJob begins as a local-first, offline-capable TUI application backed by SQLite. This plan describes the incremental architectural work that enables a gradual, non-destructive migration to a cloud-hosted SaaS product without breaking the local-first experience.

The key insight is that the repository pattern already required by the persistence layer (`specs/04-sqlite-persistence.md`) gives us the seam we need: SQLite repositories today, PostgreSQL repositories tomorrow, with a thin sync layer in between. No "big rewrite" is required — each phase adds optional cloud capability while the local path remains fully functional.

The migration unfolds in four phases: (1) fortify local abstractions so they are swap-ready; (2) introduce an optional sync layer that mirrors local data to a cloud PostgreSQL/Supabase backend; (3) add a REST API + auth layer so the web UI and other clients can talk to the same backend; (4) graduate to full multi-tenant SaaS with pricing enforcement, team collaboration, and LLM proxy (Ralph-as-a-Service).

## Prerequisites

- `specs/04-sqlite-persistence.md` and its implementation plan must be complete — the repository traits are the migration seam.
- `specs/16-privacy-security.md` (master password, at-rest encryption) — sensitive data must be encrypted before it touches a remote server.
- `specs/02-llm-provider-abstraction-implementation-plan.md` — the LLM proxy tier reuses this abstraction.

### Crates to Add

```toml
# Phase 1 — feature flags
[dependencies]
once_cell = "1"

# Phase 2 — sync layer
sqlx = { version = "0.7", features = ["postgres", "sqlite", "runtime-tokio-rustls", "macros", "uuid", "chrono"] }
tokio-retry = "0.3"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }

# Phase 3 — API server
axum = { version = "0.7", features = ["macros", "ws"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace", "auth"] }
jsonwebtoken = "9"
oauth2 = "4"
argon2 = "0.5"          # already in lazyjob-core for master password

# Phase 4 — billing gates
stripe-rust = "0.12"    # or equivalent; kept behind feature flag

[dev-dependencies]
wiremock = "0.6"
testcontainers = "0.15"
```

---

## Architecture

### Crate Placement

| Concern | Crate |
|---|---|
| Repository trait definitions | `lazyjob-core` |
| SQLite repository impls | `lazyjob-core` |
| PostgreSQL repository impls | `lazyjob-sync` (new) |
| Sync engine (local→cloud delta) | `lazyjob-sync` (new) |
| Feature-flag runtime | `lazyjob-core` (config module) |
| REST API server (axum) | `lazyjob-api` (new binary crate) |
| Auth middleware (JWT + OAuth) | `lazyjob-api` |
| Pricing-gate middleware | `lazyjob-api` |
| Web UI | outside this plan (separate repo) |

New crates are added to the Cargo workspace in `Cargo.toml`.

### Core Types

```rust
// lazyjob-core/src/config/features.rs

/// Compile-time-safe runtime feature flags.
/// Loaded once from ~/.config/lazyjob/config.toml at startup; never mutated.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FeatureFlags {
    /// Enable cloud sync to a Supabase/Postgres backend.
    pub cloud_sync_enabled: bool,

    /// Supabase project URL, e.g. "https://xyzxyz.supabase.co"
    pub supabase_url: Option<String>,

    /// Supabase anon key; stored in OS keychain, loaded at startup.
    pub supabase_anon_key_ref: Option<String>,

    /// Whether to expose the local REST API for the web UI.
    pub api_server_enabled: bool,

    /// Port for the local API server (default 7471).
    pub api_server_port: u16,

    /// Pricing plan — enforced by API server and sync layer.
    pub plan: Plan,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            cloud_sync_enabled: false,
            supabase_url: None,
            supabase_anon_key_ref: None,
            api_server_enabled: false,
            api_server_port: 7471,
            plan: Plan::Local,
        }
    }
}

/// Global, immutable config loaded once at startup.
/// Use `once_cell::sync::OnceCell` so tests can override.
static FEATURES: once_cell::sync::OnceCell<FeatureFlags> = once_cell::sync::OnceCell::new();

pub fn features() -> &'static FeatureFlags {
    FEATURES.get_or_init(FeatureFlags::default)
}

pub fn init_features(flags: FeatureFlags) {
    FEATURES.set(flags).ok();
}
```

```rust
// lazyjob-core/src/domain/user.rs  (new for Phase 3)

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: uuid::Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub auth_provider: AuthProvider,
    pub tenant_id: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum AuthProvider {
    Google,
    GitHub,
    EmailMagicLink,
    Local,   // no-auth local session
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct Tenant {
    pub id: uuid::Uuid,
    pub owner_id: uuid::Uuid,
    pub plan: Plan,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum Plan {
    Local,       // single-user, no cloud
    Free,        // cloud sync, limited apps
    Pro,         // unlimited apps + Ralph
    Team,        // collaboration
    Enterprise,  // SSO + SLA
}

impl Plan {
    pub fn max_applications(&self) -> Option<u32> {
        match self {
            Plan::Local | Plan::Pro | Plan::Team | Plan::Enterprise => None,
            Plan::Free => Some(20),
        }
    }

    pub fn ralph_loops_enabled(&self) -> bool {
        matches!(self, Plan::Pro | Plan::Team | Plan::Enterprise)
    }

    pub fn cloud_sync_enabled(&self) -> bool {
        !matches!(self, Plan::Local)
    }

    pub fn team_features(&self) -> bool {
        matches!(self, Plan::Team | Plan::Enterprise)
    }
}
```

```rust
// lazyjob-sync/src/sync/types.rs

/// A single operation that can be applied to the cloud replica.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncEvent {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub table_name: String,
    pub row_id: uuid::Uuid,
    pub operation: SyncOperation,
    /// Monotonically increasing sequence number per (user, table).
    pub seq: i64,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperation {
    Insert,
    Update,
    Delete,
}

/// Tracks the high-water mark of what has been synced.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncCursor {
    pub user_id: uuid::Uuid,
    pub table_name: String,
    /// Highest `seq` that has been successfully pushed to the cloud.
    pub last_pushed_seq: i64,
    /// Highest `seq` received from the cloud during the last pull.
    pub last_pulled_seq: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Result of one sync round-trip.
#[derive(Debug)]
pub struct SyncResult {
    pub pushed: usize,
    pub pulled: usize,
    pub conflicts: Vec<SyncConflict>,
}

#[derive(Debug)]
pub struct SyncConflict {
    pub table_name: String,
    pub row_id: uuid::Uuid,
    pub local_seq: i64,
    pub remote_seq: i64,
    pub resolution: ConflictResolution,
}

#[derive(Debug)]
pub enum ConflictResolution {
    LocalWins,
    RemoteWins,
    /// User must resolve manually (deferred to interactive flow).
    NeedsUserReview,
}
```

```rust
// lazyjob-api/src/auth/types.rs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Claims {
    /// Subject — user UUID.
    pub sub: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub plan: Plan,
    /// Expiry (Unix timestamp).
    pub exp: i64,
    pub iat: i64,
}

/// Extractor placed in axum handler arguments via FromRequestParts.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub plan: Plan,
}

/// API-level rate-limit bucket (enforced by tower middleware).
pub struct RateLimitBucket {
    pub plan: Plan,
    /// Requests per minute.
    pub rpm: u32,
}

impl RateLimitBucket {
    pub fn for_plan(plan: &Plan) -> Self {
        let rpm = match plan {
            Plan::Local | Plan::Free => 30,
            Plan::Pro => 120,
            Plan::Team | Plan::Enterprise => 600,
        };
        Self { plan: plan.clone(), rpm }
    }
}
```

### Trait Definitions

```rust
// lazyjob-sync/src/sync/engine.rs

#[async_trait::async_trait]
pub trait SyncEngine: Send + Sync {
    /// Push locally-generated events since last cursor to the cloud.
    async fn push(&self, table: &str) -> Result<usize, SyncError>;

    /// Pull events from the cloud since last cursor and apply them locally.
    async fn pull(&self, table: &str) -> Result<usize, SyncError>;

    /// Full bidirectional sync for all syncable tables.
    async fn sync_all(&self) -> Result<SyncResult, SyncError>;

    /// Return current cursors for all tables.
    async fn cursors(&self) -> Result<Vec<SyncCursor>, SyncError>;
}

// lazyjob-api/src/auth/provider.rs

#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn authorization_url(&self, state: &str) -> String;
    async fn exchange_code(
        &self,
        code: &str,
        state: &str,
    ) -> Result<OAuthTokenSet, AuthError>;
    async fn user_info(&self, access_token: &str) -> Result<OAuthUserInfo, AuthError>;
}

// lazyjob-core/src/billing/gate.rs

pub trait PricingGate: Send + Sync {
    fn check_application_limit(
        &self,
        plan: &Plan,
        current_count: u32,
    ) -> Result<(), PricingError>;

    fn check_ralph_allowed(&self, plan: &Plan) -> Result<(), PricingError>;

    fn check_cloud_sync_allowed(&self, plan: &Plan) -> Result<(), PricingError>;

    fn check_team_features(&self, plan: &Plan) -> Result<(), PricingError>;
}
```

### SQLite Schema (Sync Metadata Tables)

```sql
-- Migration 020: sync metadata

-- Tracks outgoing events to be pushed to the cloud.
CREATE TABLE sync_outbox (
    id          TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    table_name  TEXT NOT NULL,
    row_id      TEXT NOT NULL,
    operation   TEXT NOT NULL CHECK (operation IN ('insert', 'update', 'delete')),
    payload     TEXT NOT NULL,    -- JSON
    seq         INTEGER NOT NULL,
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    pushed_at   TEXT,             -- NULL until successfully pushed
    retry_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_sync_outbox_unpushed ON sync_outbox (pushed_at)
    WHERE pushed_at IS NULL;

-- Tracks the high-water mark of what we have pulled from the cloud.
CREATE TABLE sync_cursors (
    table_name        TEXT PRIMARY KEY,
    last_pushed_seq   INTEGER NOT NULL DEFAULT 0,
    last_pulled_seq   INTEGER NOT NULL DEFAULT 0,
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Conflict log — rows that could not be auto-resolved.
CREATE TABLE sync_conflicts (
    id          TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    table_name  TEXT NOT NULL,
    row_id      TEXT NOT NULL,
    local_seq   INTEGER NOT NULL,
    remote_seq  INTEGER NOT NULL,
    local_json  TEXT NOT NULL,
    remote_json TEXT NOT NULL,
    resolution  TEXT,             -- NULL until resolved
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT
);
```

```sql
-- Migration 021: user and tenant tables (used in cloud PostgreSQL; mirrored locally for offline JWT validation)

CREATE TABLE tenants (
    id         TEXT PRIMARY KEY,
    owner_id   TEXT NOT NULL,
    plan       TEXT NOT NULL DEFAULT 'local',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    auth_provider TEXT NOT NULL DEFAULT 'local',
    tenant_id     TEXT NOT NULL REFERENCES tenants(id),
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- OAuth sessions
CREATE TABLE oauth_sessions (
    id           TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider     TEXT NOT NULL,
    access_token TEXT NOT NULL,   -- stored encrypted via age
    refresh_token TEXT,
    expires_at   TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_oauth_sessions_user ON oauth_sessions (user_id);
```

```sql
-- PostgreSQL schema additions for multi-tenancy (cloud only)
-- Run by lazyjob-api migrations on cloud PostgreSQL

-- Row-level security on all data tables
ALTER TABLE jobs        ADD COLUMN tenant_id UUID NOT NULL;
ALTER TABLE applications ADD COLUMN tenant_id UUID NOT NULL;
ALTER TABLE contacts    ADD COLUMN tenant_id UUID NOT NULL;
-- ... same pattern for all tables

-- Enable RLS
ALTER TABLE jobs        ENABLE ROW LEVEL SECURITY;
ALTER TABLE applications ENABLE ROW LEVEL SECURITY;

-- Policy: users only see their tenant's rows
CREATE POLICY tenant_isolation ON jobs
    USING (tenant_id = current_setting('app.current_tenant_id')::UUID);
```

### Module Structure

```
lazyjob-core/
  src/
    config/
      mod.rs
      features.rs       ← FeatureFlags, Plan, features() singleton
    billing/
      mod.rs
      gate.rs           ← PricingGate trait + DefaultPricingGate impl
    domain/
      user.rs           ← User, Tenant, AuthProvider types

lazyjob-sync/           ← new crate
  Cargo.toml
  src/
    lib.rs
    sync/
      mod.rs
      engine.rs         ← SyncEngine trait
      sqlite_engine.rs  ← SqliteSyncEngine impl
      postgres_client.rs ← HTTP client talking to Supabase REST API
      outbox.rs         ← OutboxWriter: writes SyncEvent to sync_outbox
      puller.rs         ← CloudPuller: fetches remote events, applies locally
      conflict.rs       ← ConflictResolver: last-write-wins + manual queue
      types.rs          ← SyncEvent, SyncCursor, SyncResult, SyncConflict
    background.rs       ← BackgroundSyncer: tokio task, 30s interval

lazyjob-api/            ← new binary crate
  Cargo.toml
  src/
    main.rs             ← build router, bind port, spawn background tasks
    lib.rs
    router.rs           ← axum Router assembly
    auth/
      mod.rs
      middleware.rs     ← JWT extraction, FromRequestParts for AuthenticatedUser
      jwt.rs            ← encode/decode Claims with jsonwebtoken
      oauth/
        mod.rs
        google.rs       ← GoogleOAuthProvider
        github.rs       ← GitHubOAuthProvider
      provider.rs       ← OAuthProvider trait
      types.rs          ← Claims, AuthenticatedUser
    handlers/
      mod.rs
      jobs.rs           ← GET/POST /api/v1/jobs
      applications.rs   ← CRUD /api/v1/applications
      ralph.rs          ← POST /api/v1/ralph/start, GET /api/v1/ralph/{id}/events (SSE)
      sync.rs           ← POST /api/v1/sync/push, GET /api/v1/sync/pull
      auth.rs           ← /api/v1/auth/login, /callback, /logout, /refresh
      billing.rs        ← /api/v1/billing/plan, /upgrade
    middleware/
      rate_limit.rs     ← tower Layer using governor
      plan_gate.rs      ← tower Layer checking Plan before handler
    error.rs            ← ApiError enum → axum IntoResponse
```

---

## Implementation Phases

### Phase 1 — Feature-Flag Infrastructure (Local; Zero Breaking Changes)

**Goal**: Add the `FeatureFlags` config system so all later phases can be toggled without code changes. No network calls yet.

**Step 1.1 — Add `FeatureFlags` to config**

- File: `lazyjob-core/src/config/features.rs`
- Implement `FeatureFlags` struct (see Core Types above).
- Add `plan: Plan` field to the existing `AppConfig` struct.
- Load flags from `~/.config/lazyjob/config.toml` section `[features]` via `serde::Deserialize`.
- Initialize the `FEATURES` singleton from `main.rs` before anything else.

```rust
// lazyjob-cli/src/main.rs
let config = AppConfig::load()?;
lazyjob_core::config::features::init_features(config.features.clone());
```

**Step 1.2 — Add `Plan` and `PricingGate`**

- File: `lazyjob-core/src/billing/gate.rs`
- `DefaultPricingGate` reads the global `features()` singleton.
- `check_application_limit` returns `PricingError::LimitExceeded` if `plan == Plan::Free && count >= 20`.
- Inject `Arc<dyn PricingGate>` into `ApplicationService`; call before inserting.

**Step 1.3 — Sync outbox trigger hooks in repositories**

- File: `lazyjob-core/src/persistence/outbox.rs`
- Add `OutboxWriter` struct with `write(table, row_id, op, payload)` method.
- Call `OutboxWriter::write(...)` in `JobRepository::insert`, `::update`, `::delete` — **only if** `features().cloud_sync_enabled`.
- This keeps the hot path zero-cost when sync is off.

```rust
// Inside JobRepository::insert
if features().cloud_sync_enabled {
    self.outbox.write("jobs", &job.id, SyncOperation::Insert, serde_json::to_value(&job)?).await?;
}
```

**Verification (Phase 1)**:
- `cargo test -p lazyjob-core` — all existing tests pass.
- Set `cloud_sync_enabled = false` in config; confirm no outbox rows written.
- Set `cloud_sync_enabled = true`; insert a job; confirm a row appears in `sync_outbox`.

---

### Phase 2 — Sync Layer (Local ↔ Cloud)

**Goal**: Ship a background tokio task that pushes local changes to a Supabase PostgreSQL backend and pulls remote changes.

**Step 2.1 — Create `lazyjob-sync` crate**

```toml
# lazyjob-sync/Cargo.toml
[package]
name = "lazyjob-sync"
version = "0.1.0"
edition = "2021"

[dependencies]
lazyjob-core = { path = "../lazyjob-core" }
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "macros"] }
reqwest = { version = "0.11", default-features = false, features = ["rustls-tls", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["time"] }
tokio-retry = "0.3"
thiserror = "1"
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
```

**Step 2.2 — `PostgresClient`: Supabase REST wrapper**

- File: `lazyjob-sync/src/sync/postgres_client.rs`
- Uses Supabase's PostgREST API (no native PostgreSQL driver needed in Phase 2; reduces compile complexity).
- Key API calls:

```rust
pub struct PostgresClient {
    client: reqwest::Client,
    base_url: String,
    anon_key: secrecy::Secret<String>,
}

impl PostgresClient {
    /// Push events: POST /rest/v1/sync_events
    pub async fn push_events(&self, events: &[SyncEvent]) -> Result<(), SyncError>;

    /// Pull events newer than `after_seq`: GET /rest/v1/sync_events?seq=gt.{after_seq}
    pub async fn pull_events(&self, table: &str, after_seq: i64)
        -> Result<Vec<SyncEvent>, SyncError>;
}
```

- Retry with `tokio_retry::strategy::ExponentialBackoff::from_millis(100).take(5)`.
- On non-retryable HTTP errors (4xx except 429), surface `SyncError::CloudRejected`.

**Step 2.3 — `SqliteSyncEngine` impl**

- File: `lazyjob-sync/src/sync/sqlite_engine.rs`
- `push(table)`:
  1. `SELECT * FROM sync_outbox WHERE table_name = ? AND pushed_at IS NULL ORDER BY seq LIMIT 100`.
  2. Call `PostgresClient::push_events(batch)`.
  3. On success: `UPDATE sync_outbox SET pushed_at = datetime('now') WHERE id IN (...)`.
  4. Update `sync_cursors.last_pushed_seq`.
- `pull(table)`:
  1. Read `sync_cursors.last_pulled_seq` for table.
  2. Call `PostgresClient::pull_events(table, last_pulled_seq)`.
  3. For each event, call `ConflictResolver::apply_remote(event)`.
  4. Update `sync_cursors.last_pulled_seq`.
- `sync_all()`:
  1. For each table in `SYNCABLE_TABLES = ["jobs", "applications", "contacts", "life_sheet"]`.
  2. `tokio::join!(self.push(t), self.pull(t))` per table sequentially (avoid concurrent writes to same table from push + pull).

**Step 2.4 — `ConflictResolver`**

- File: `lazyjob-sync/src/sync/conflict.rs`
- Strategy: **last-write-wins** based on `SyncEvent::occurred_at`.
- If `remote.occurred_at > local.updated_at`: apply remote (overwrite local row).
- If `remote.occurred_at < local.updated_at`: local wins, discard remote, log to `sync_conflicts`.
- If equal: flag as `NeedsUserReview`, insert into `sync_conflicts`, emit `tracing::warn!`.

**Step 2.5 — `BackgroundSyncer` tokio task**

- File: `lazyjob-sync/src/background.rs`
- Spawned from `lazyjob-cli/src/main.rs` if `features().cloud_sync_enabled`.

```rust
pub async fn run_background_syncer(engine: Arc<dyn SyncEngine>, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                match engine.sync_all().await {
                    Ok(r) => tracing::info!(pushed = r.pushed, pulled = r.pulled, "sync complete"),
                    Err(e) => tracing::error!(error = %e, "sync failed"),
                }
            }
            _ = shutdown.cancelled() => {
                tracing::info!("background syncer shutting down");
                break;
            }
        }
    }
}
```

**Step 2.6 — Manual sync TUI command**

- In TUI command palette: `:sync` → dispatch `Action::TriggerSync`.
- `AppState` holds `SyncStatus { last_sync: Option<DateTime<Utc>>, in_progress: bool, last_error: Option<String> }`.
- Status bar shows sync status.

**Verification (Phase 2)**:
- Start with a Supabase project in test mode.
- Insert a job locally; confirm it appears in Supabase `jobs` table within 30 seconds.
- Delete the job from Supabase; confirm it disappears locally after next pull.
- Force a conflict (edit same row locally and remotely); confirm `sync_conflicts` row is created.

---

### Phase 3 — REST API Server + Authentication

**Goal**: Expose a local (or cloud-deployed) REST API so a web UI or mobile client can interact with LazyJob data. Add JWT-based auth for the cloud-deployed case.

**Step 3.1 — Create `lazyjob-api` binary crate**

```toml
[package]
name = "lazyjob-api"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "lazyjob-api"
path = "src/main.rs"

[dependencies]
lazyjob-core = { path = "../lazyjob-core" }
lazyjob-sync = { path = "../lazyjob-sync" }
axum = { version = "0.7", features = ["macros", "ws"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
jsonwebtoken = "9"
oauth2 = "4"
reqwest = { version = "0.11", default-features = false, features = ["rustls-tls", "json"] }
thiserror = "1"
tracing = "0.1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
```

**Step 3.2 — Router assembly**

```rust
// lazyjob-api/src/router.rs

pub fn build_router(state: AppState) -> axum::Router {
    axum::Router::new()
        // Auth endpoints — no JWT required
        .route("/api/v1/auth/login",    axum::routing::post(handlers::auth::login))
        .route("/api/v1/auth/callback", axum::routing::get(handlers::auth::oauth_callback))
        .route("/api/v1/auth/refresh",  axum::routing::post(handlers::auth::refresh))
        .route("/api/v1/auth/logout",   axum::routing::post(handlers::auth::logout))
        // Protected endpoints — JWT required
        .route("/api/v1/jobs",          axum::routing::get(handlers::jobs::list)
                                              .post(handlers::jobs::create))
        .route("/api/v1/jobs/:id",      axum::routing::get(handlers::jobs::get)
                                              .put(handlers::jobs::update)
                                              .delete(handlers::jobs::delete))
        .route("/api/v1/applications",  axum::routing::get(handlers::applications::list)
                                              .post(handlers::applications::create))
        .route("/api/v1/ralph/start",   axum::routing::post(handlers::ralph::start))
        .route("/api/v1/ralph/:id/events", axum::routing::get(handlers::ralph::sse_stream))
        .route("/api/v1/sync/push",     axum::routing::post(handlers::sync::push))
        .route("/api/v1/sync/pull",     axum::routing::get(handlers::sync::pull))
        .layer(middleware::from_fn_with_state(state.clone(), auth::middleware::require_auth))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(tower_http::trace::DefaultMakeSpan::default()),
        )
        .layer(tower_http::cors::CorsLayer::permissive())  // tighten in production
        .with_state(state)
}
```

**Step 3.3 — JWT auth middleware**

```rust
// lazyjob-api/src/auth/middleware.rs

pub async fn require_auth(
    State(state): State<AppState>,
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, ApiError> {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized("missing bearer token".into()))?;

    let claims = jwt::decode(token, &state.jwt_secret)
        .map_err(|_| ApiError::Unauthorized("invalid token".into()))?;

    req.extensions_mut().insert(AuthenticatedUser {
        user_id: claims.sub,
        tenant_id: claims.tenant_id,
        plan: claims.plan,
    });
    Ok(next.run(req).await)
}
```

**Step 3.4 — JWT encode/decode**

```rust
// lazyjob-api/src/auth/jwt.rs

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation, Algorithm};

pub fn encode_token(claims: &Claims, secret: &[u8]) -> Result<String, AuthError> {
    encode(&Header::new(Algorithm::HS256), claims, &EncodingKey::from_secret(secret))
        .map_err(AuthError::JwtEncode)
}

pub fn decode(token: &str, secret: &[u8]) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map(|d| d.claims)
        .map_err(AuthError::JwtDecode)
}
```

**Step 3.5 — OAuth providers (Google + GitHub)**

```rust
// lazyjob-api/src/auth/oauth/google.rs

pub struct GoogleOAuthProvider {
    client: oauth2::basic::BasicClient,
    http: reqwest::Client,
}

#[async_trait::async_trait]
impl OAuthProvider for GoogleOAuthProvider {
    fn provider_name(&self) -> &'static str { "google" }

    fn authorization_url(&self, state: &str) -> String {
        let (url, _) = self.client
            .authorize_url(|| oauth2::CsrfToken::new(state.to_string()))
            .add_scope(oauth2::Scope::new("email".into()))
            .add_scope(oauth2::Scope::new("profile".into()))
            .url();
        url.to_string()
    }

    async fn exchange_code(&self, code: &str, _state: &str) -> Result<OAuthTokenSet, AuthError> {
        let token = self.client
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| AuthError::OAuthExchange(e.to_string()))?;

        Ok(OAuthTokenSet {
            access_token: token.access_token().secret().clone(),
            refresh_token: token.refresh_token().map(|t| t.secret().clone()),
            expires_at: token.expires_in().map(|d| chrono::Utc::now() + chrono::Duration::from_std(d).unwrap()),
        })
    }

    async fn user_info(&self, access_token: &str) -> Result<OAuthUserInfo, AuthError> {
        #[derive(serde::Deserialize)]
        struct GoogleUser { email: String, name: Option<String> }
        let user: GoogleUser = self.http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(access_token)
            .send().await?.json().await?;
        Ok(OAuthUserInfo { email: user.email, display_name: user.name })
    }
}
```

**Step 3.6 — Pricing gate middleware (tower `Layer`)**

```rust
// lazyjob-api/src/middleware/plan_gate.rs

/// Tower middleware that checks pricing gate before passing to handler.
/// Placed on routes that require Pro or above.
pub struct RequirePlan {
    required: Plan,
}

impl<S> tower::Layer<S> for RequirePlan {
    type Service = RequirePlanService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RequirePlanService { inner, required: self.required.clone() }
    }
}

// In tower::Service impl:
// Extract AuthenticatedUser from request extensions.
// If user.plan < required: return 403 with PricingError::PlanRequired body.
```

**Step 3.7 — SSE streaming for Ralph output**

```rust
// lazyjob-api/src/handlers/ralph.rs

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::BroadcastStream;

pub async fn sse_stream(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    axum::extract::Path(loop_id): axum::extract::Path<uuid::Uuid>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, axum::Error>>> {
    let rx = state.ralph_manager.subscribe(loop_id, &user.tenant_id).unwrap();
    let stream = BroadcastStream::new(rx).map(|msg| {
        msg.map(|m| Event::default().json_data(m).unwrap())
           .map_err(|e| axum::Error::new(e))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

**Verification (Phase 3)**:
- `cargo build -p lazyjob-api && ./target/debug/lazyjob-api`.
- `curl -X POST /api/v1/auth/login` with a magic-link email → receive JWT.
- `curl -H "Authorization: Bearer <token>" /api/v1/jobs` → 200 with job list.
- `curl /api/v1/ralph/start` with a Free plan token → 403 PlanRequired.

---

### Phase 4 — Full Multi-Tenant SaaS

**Goal**: Tenant isolation in PostgreSQL (RLS), billing plan enforcement from Stripe webhooks, team collaboration features, and Ralph-as-a-Service.

**Step 4.1 — PostgreSQL repository implementations**

- File: `lazyjob-sync/src/sync/pg_repositories.rs`
- Mirror all repository traits from `lazyjob-core` with `sqlx::PgPool` backend.
- Every query includes `WHERE tenant_id = $1` (enforced at Rust level as belt-and-suspenders on top of PostgreSQL RLS).
- Use `sqlx::query_as!` macro for compile-time checked queries against the cloud PG schema.

```rust
pub struct PgJobRepository {
    pool: sqlx::PgPool,
    tenant_id: uuid::Uuid,
}

#[async_trait::async_trait]
impl JobRepository for PgJobRepository {
    async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>, RepositoryError> {
        sqlx::query_as!(
            Job,
            "SELECT * FROM jobs WHERE tenant_id = $1 ORDER BY discovered_at DESC LIMIT $2",
            self.tenant_id,
            filter.limit.unwrap_or(100) as i64
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::Database)
    }
    // ...
}
```

**Step 4.2 — RLS configuration in PostgreSQL**

```sql
-- Run as superuser in cloud Postgres
CREATE FUNCTION app_tenant_id() RETURNS uuid AS $$
    SELECT current_setting('app.current_tenant_id', true)::uuid;
$$ LANGUAGE sql STABLE;

-- Policy on jobs
CREATE POLICY tenant_jobs ON jobs
    USING (tenant_id = app_tenant_id());

-- API server sets the tenant on each connection:
-- SET LOCAL app.current_tenant_id = '<uuid>';
```

In axum handlers, after acquiring a PG connection:
```rust
sqlx::query!("SET LOCAL app.current_tenant_id = $1", tenant_id)
    .execute(&mut *tx).await?;
```

**Step 4.3 — Stripe webhook handler**

- Endpoint: `POST /webhooks/stripe` — no JWT required, validate with Stripe signature.
- On `customer.subscription.updated`: update `tenants.plan` in PostgreSQL.
- Propagate to the local TUI via sync layer on next pull.

```rust
// lazyjob-api/src/handlers/billing.rs

pub async fn stripe_webhook(
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    let sig = headers.get("stripe-signature").ok_or(ApiError::BadRequest("missing sig".into()))?;
    let event = stripe_rust::Webhook::construct_event(
        std::str::from_utf8(&body)?,
        sig.to_str()?,
        &STRIPE_WEBHOOK_SECRET,
    )?;
    match event.type_ {
        stripe_rust::EventType::CustomerSubscriptionUpdated => {
            // extract plan from event metadata, update DB
        }
        _ => {}
    }
    Ok(axum::Json(serde_json::json!({"status": "ok"})))
}
```

**Step 4.4 — Ralph-as-a-Service**

- In `lazyjob-api`, the `RalphManager` spawns `tokio::process::Command` instances (same as local) but constrained to the cloud server's resources.
- Each loop is isolated in a tokio task. Output is broadcast on a `tokio::sync::broadcast::Sender<RalphOutputEvent>`.
- Plan gate: only Pro+ users can call `POST /api/v1/ralph/start`.
- Resource budget: `RalphBudget { max_concurrent: u8, max_tokens_per_loop: u32 }` read from env.

**Step 4.5 — GDPR compliance endpoints**

```
GET  /api/v1/me/export   → streams full user data as JSON-LD archive (zip)
POST /api/v1/me/delete   → schedules full account deletion within 30 days
```

- `DataExporter::export_user(user_id)` queries all tables for the tenant and streams as JSON.
- `AccountDeletion` table records the scheduled deletion date; a cron job completes it.

**Verification (Phase 4)**:
- Sign up two users in different tenants; confirm each can only see their own data.
- Downgrade tenant from Pro to Free via Stripe test webhook; confirm next `POST /api/v1/ralph/start` returns 403.
- Trigger `GET /api/v1/me/export`; confirm ZIP contains all expected tables.

---

## Key Crate APIs

- `once_cell::sync::OnceCell::<FeatureFlags>::set(flags)` — initialize global feature flags singleton.
- `sqlx::query_as!(Row, "SELECT ...", params)` — compile-time query checking for both SQLite and PostgreSQL.
- `reqwest::Client::post(url).bearer_auth(key).json(payload).send().await` — Supabase PostgREST push.
- `tokio_retry::Retry::spawn(ExponentialBackoff::from_millis(100).take(5), || client.push(...))` — retry with backoff.
- `jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret))` — JWT signing.
- `jsonwebtoken::decode::<Claims>(token, &key, &Validation::new(Algorithm::HS256))` — JWT verification.
- `oauth2::basic::BasicClient::new(...)` — OAuth 2.0 client setup.
- `axum::middleware::from_fn_with_state(state, require_auth)` — attach auth middleware.
- `axum::response::sse::Sse::new(stream).keep_alive(KeepAlive::default())` — SSE streaming for Ralph output.
- `tokio_stream::wrappers::BroadcastStream::new(rx)` — convert broadcast channel to stream for SSE.
- `tower_http::cors::CorsLayer::permissive()` — permissive CORS for development; tighten for production.
- `tokio::time::interval(Duration::from_secs(30))` — background sync tick.
- `tokio_util::sync::CancellationToken` — graceful shutdown signal for background tasks.

---

## Error Handling

```rust
// lazyjob-sync/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum SyncError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("cloud rejected request (HTTP {status}): {message}")]
    CloudRejected { status: u16, message: String },

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("sync conflict on table {table} row {row_id}: needs user review")]
    ConflictNeedsReview { table: String, row_id: uuid::Uuid },
}

// lazyjob-api/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found")]
    NotFound,

    #[error("plan required: feature requires {0:?} plan or above")]
    PlanRequired(Plan),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("sync error: {0}")]
    Sync(#[from] SyncError),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, code) = match &self {
            ApiError::Unauthorized(_) => (axum::http::StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::Forbidden(_)    => (axum::http::StatusCode::FORBIDDEN, "forbidden"),
            ApiError::BadRequest(_)   => (axum::http::StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::NotFound        => (axum::http::StatusCode::NOT_FOUND, "not_found"),
            ApiError::PlanRequired(_) => (axum::http::StatusCode::PAYMENT_REQUIRED, "plan_required"),
            _                         => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        let body = serde_json::json!({ "error": code, "message": self.to_string() });
        (status, axum::Json(body)).into_response()
    }
}

// lazyjob-core/src/billing/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PricingError {
    #[error("application limit reached: {plan:?} plan allows a maximum of {limit} applications")]
    LimitExceeded { plan: Plan, limit: u32 },

    #[error("ralph loops require Pro plan or above")]
    RalphNotAllowed,

    #[error("cloud sync requires a paid plan")]
    CloudSyncNotAllowed,

    #[error("team features require Team plan or above")]
    TeamFeaturesNotAllowed,
}
```

---

## Testing Strategy

### Unit Tests

**Feature flags**:
```rust
#[test]
fn plan_application_limit() {
    assert_eq!(Plan::Free.max_applications(), Some(20));
    assert_eq!(Plan::Pro.max_applications(), None);
}

#[test]
fn pricing_gate_blocks_on_free_plan() {
    let gate = DefaultPricingGate;
    assert!(gate.check_application_limit(&Plan::Free, 20).is_err());
    assert!(gate.check_application_limit(&Plan::Free, 19).is_ok());
    assert!(gate.check_application_limit(&Plan::Pro, 999).is_ok());
}
```

**JWT round-trip**:
```rust
#[test]
fn jwt_encode_decode_roundtrip() {
    let secret = b"test-secret-key";
    let claims = Claims { sub: uuid::Uuid::new_v4(), tenant_id: uuid::Uuid::new_v4(),
                           plan: Plan::Pro, exp: 9999999999, iat: 0 };
    let token = encode_token(&claims, secret).unwrap();
    let decoded = decode(&token, secret).unwrap();
    assert_eq!(decoded.sub, claims.sub);
}
```

**Conflict resolver**:
```rust
#[test]
fn last_write_wins_remote() {
    let remote = SyncEvent { occurred_at: Utc::now(), .. };
    let local_updated = Utc::now() - Duration::seconds(10);
    let result = ConflictResolver::resolve(&remote, local_updated);
    assert_eq!(result, ConflictResolution::RemoteWins);
}
```

### Integration Tests

**Sync integration (wiremock)**:
- Start `MockServer` that mimics Supabase PostgREST.
- Create `PostgresClient::new_with_base_url(mock_server.uri())`.
- Insert a job into local SQLite → trigger sync → assert mock received the POST to `/rest/v1/sync_events`.

```rust
#[tokio::test]
async fn push_inserts_to_cloud_mock() {
    let mock_server = wiremock::MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/rest/v1/sync_events"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let client = PostgresClient::new_with_base_url(
        mock_server.uri(),
        Secret::new("test-key".into()),
    );
    let event = make_test_sync_event("jobs");
    client.push_events(&[event]).await.unwrap();
    mock_server.verify().await;
}
```

**API handler integration (testcontainers)**:
- Spin up a real PostgreSQL container via `testcontainers`.
- Run migrations against it.
- Build the axum router with a test `AppState`.
- Use `axum::body::to_bytes` + `tower::ServiceExt::oneshot` to send requests.

```rust
#[tokio::test]
async fn get_jobs_requires_auth() {
    let app = build_test_router().await;
    let response = app.oneshot(
        Request::builder()
            .uri("/api/v1/jobs")
            .body(Body::empty())
            .unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

**Multi-tenant isolation**:
- Create two tenants and two users.
- Insert a job under tenant A.
- Authenticate as tenant B user; assert `GET /api/v1/jobs` returns empty list.

### TUI Tests

- The sync status bar widget is unit-testable: render with `ratatui::backend::TestBackend`, assert text contains "Synced 2s ago".
- The plan-gate error dialog: render with `TestBackend`, assert error message shows "Pro plan required".

---

## Open Questions

1. **CRDT vs. last-write-wins**: Last-write-wins works for most job search data (a job listing, a contact), but structured fields like tags lists should use union-CRDT. Is per-field CRDT worth the complexity in MVP? Recommendation: defer to Phase 5; ship LWW first.

2. **Supabase vs. self-hosted PostgreSQL**: Supabase PostgREST reduces server-side code but adds a vendor dependency. A full axum backend with `sqlx::PgPool` is more flexible but requires more infra. Recommendation: use PostgREST for Phase 2, migrate to native PG driver in Phase 3.

3. **TUI vs. web UI for SaaS**: The spec acknowledges this is unresolved. Recommendation: Phase 3 ships a REST API; the web UI is a separate project (e.g., Next.js SPA consuming the API). The TUI remains the primary interface for local users.

4. **Offline JWT validation**: The local API server cannot call the cloud to validate tokens when offline. Recommendation: embed the JWT public key in the binary (or load from config). Short-lived tokens (1h) are acceptable since the local server is only used when the user is on their machine.

5. **Data portability format**: The spec mentions GDPR export. Should the export be a raw SQL dump, a structured JSON-LD archive, or a custom format? Recommendation: structured JSON with a schema version field for forward compatibility.

6. **Sync granularity**: Should we sync at the row level (current plan) or field level? Row-level is simpler but means a remote update to one field overwrites all local field edits. Field-level sync requires tracking per-field timestamps. Recommendation: row-level for Phase 2, field-level in Phase 4.

---

## Related Specs

- `specs/04-sqlite-persistence.md` — the repository trait seam that sync builds on
- `specs/16-privacy-security.md` — encryption at rest before data leaves the device
- `specs/02-llm-provider-abstraction-implementation-plan.md` — LLM proxy tier reuses this abstraction
- `specs/agentic-ralph-orchestration.md` — Ralph-as-a-Service in Phase 4 extends the local orchestration model
- `specs/20-openapi-mvp.md` — MVP build order; sync and API are post-MVP phases
