# Spec: SaaS — Migration Path

**JTBD**: Migrate from local-first CLI to cloud SaaS product without rewriting core logic
**Topic**: Define the phased migration from local SQLite to cloud PostgreSQL via Supabase, with Repository trait abstraction, optional sync layer, and multi-tenancy
**Domain**: saas

---

## What

The migration path from local-first LazyJob (SQLite) to SaaS (PostgreSQL via Supabase) follows a three-phase approach: Phase 1 (local-first with sync option), Phase 2 (add cloud sync), Phase 3 (full SaaS with web UI). The key architectural enabler is the Repository trait — defined once, implemented for SQLite now and PostgreSQL later. No business logic changes required when switching persistence backends.

## Why

LazyJob starts as a local CLI tool. But the product vision is a SaaS product with cloud sync, team features, and premium AI features. The migration path must be smooth: the MVP is local-first SQLite, and the SaaS features are additive overlays, not rewrites.

The Repository trait is the key: all data access goes through trait methods. Switching from SQLite to PostgreSQL means swapping implementations, not rewriting business logic. Supabase is the target cloud platform because it provides PostgreSQL + Auth + Realtime + Storage in one managed service.

## How

### Phase Architecture

```
Phase 1 (MVP): Local SQLite → Works offline, no cloud dependency
Phase 2:      Local SQLite + Optional Supabase sync (background, last-write-wins)
Phase 3:      Full SaaS — PostgreSQL/SQLite hybrid, web UI, multi-user, team features
```

### Repository Trait (The Key Abstraction)

```rust
// lazyjob-core/src/persistence/mod.rs

pub trait Repository: Send + Sync {
    type Entity;
    type Id;

    async fn get(&self, id: &Self::Id) -> Result<Option<Self::Entity>>;
    async fn list(&self, filter: &Filter) -> Result<Vec<Self::Entity>>;
    async fn insert(&self, entity: &Self::Entity) -> Result<()>;
    async fn update(&self, entity: &Self::Entity) -> Result<()>;
    async fn delete(&self, id: &Self::Id) -> Result<()>;
}

pub trait JobRepository: Repository {
    type Entity = Job;
    type Id = Uuid;
    async fn list_by_status(&self, status: JobStatus) -> Result<Vec<Job>>;
    async fn list_by_company(&self, company_id: &Uuid) -> Result<Vec<Job>>;
    async fn count_new_matches_since(&self, since: DateTime<Utc>) -> Result<i64>;
}

// SQLite implementation (Phase 1)
impl JobRepository for SqliteJobRepository { ... }

// PostgreSQL implementation (Phase 3)
impl JobRepository for PostgresJobRepository { ... }
```

### Sync Protocol

Phase 2 introduces optional background sync to Supabase. The sync protocol uses last-write-wins with vector clocks:

```rust
// lazyjob-sync/src/lib.rs

pub struct SyncState {
    pub last_sync: DateTime<Utc>,
    pub cursor: String,
    pub pending: Vec<SyncOperation>,
}

pub enum SyncOperation {
    Insert { table: String, data: serde_json::Value },
    Update { table: String, id: Uuid, data: serde_json::Value, updated_at: DateTime<Utc> },
    Delete { table: String, id: Uuid },
}

impl SyncProtocol {
    pub async fn sync(&self, local: &SqlitePool, remote: &PostgresPool) -> Result<SyncReport> {
        // 1. Push local changes since last_sync
        let pending = self.get_pending_operations().await?;
        for op in pending {
            self.apply_remote(&op, remote).await?;
        }

        // 2. Pull remote changes since last_sync
        let remote_changes = self.fetch_remote_changes().await?;
        for op in remote_changes {
            self.apply_local(&op, local).await?;
        }

        // 3. Conflict resolution: last-write-wins
        self.resolve_conflicts(local, remote).await?;
        Ok(SyncReport { pushed: pending.len(), pulled: remote_changes.len() })
    }
}
```

### Supabase Schema

```sql
-- Supabase PostgreSQL schema
-- Maps to SQLite schema with additions for multi-tenancy

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE NOT NULL,
    auth_provider TEXT NOT NULL, -- 'google', 'github', 'magic_link'
    plan TEXT NOT NULL DEFAULT 'free',
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'free',
    storage_bytes BIGINT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Row-level security (tenant isolation)
ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY jobs_tenant_isolation ON jobs USING (tenant_id = current_setting('app.current_tenant_id')::UUID);

-- All tables include tenant_id
ALTER TABLE jobs ADD COLUMN tenant_id UUID REFERENCES tenants(id);
ALTER TABLE applications ADD COLUMN tenant_id UUID REFERENCES tenants(id);
ALTER TABLE profile_contacts ADD COLUMN tenant_id UUID REFERENCES tenants(id);
-- etc.
```

### Never-Sync Tables

The following tables are excluded from cloud sync:
- `offer_details` — contains confidential compensation data (may violate offer letter NDAs)
- `token_usage_log` — per-user billing data synced as aggregate only
- `_sqlx_migrations` — local migration tracking, meaningless in cloud

```rust
// lazyjob-sync/src/never_sync.rs

pub const NEVER_SYNC_TABLES: &[&str] = &[
    "offer_details",
    "token_usage_log",
    "_sqlx_migrations",
];

pub fn syncable_tables() -> Vec<&'static str> {
    let all = &["jobs", "applications", "contacts", "companies", "life_sheet_meta", ...];
    all.iter().filter(|t| !NEVER_SYNC_TABLES.contains(t)).copied().collect()
}
```

### Auth Provider

```rust
// lazyjob-saas/src/auth/mod.rs

pub enum AuthProvider {
    Google,
    GitHub,
    EmailMagicLink,
    EnterpriseSSO,
}

pub struct AuthService {
    supabase: SupabaseClient,
}

impl AuthService {
    pub async fn sign_in(&self, provider: AuthProvider, token: &str) -> Result<User> {
        match provider {
            AuthProvider::Google => self.supabase.auth.sign_in_with_oauth(token),
            AuthProvider::EmailMagicLink => self.supabase.auth.sign_in_with_magic_link(token),
            // ...
        }
    }
}
```

## Open Questions

- **Offline-first conflict resolution**: Last-write-wins is simple but can lose data. CRDT for specific fields (skills, notes) would preserve more data but adds complexity. MVP: last-write-wins, document the limitation.
- **TUI vs Web UI for SaaS**: The Phase 3 spec mentions a web UI. But many users may prefer the TUI even in SaaS mode (cloud sync, but TUI remains the UI). The web UI is for team collaboration (JTBD D-1 mentions team features). Decision: Phase 3 adds team features via web UI, TUI remains single-user.
- **Sync encryption**: When syncing to Supabase, should the data be client-side encrypted before upload? The `offer_details` table being excluded from sync suggests sensitive data needs special handling. MVP: no client-side encryption (Supabase handles security). Phase 2: evaluate client-side encryption for `offer_details` and `token_usage_log`.

## Implementation Tasks

- [ ] Extract all repository traits in `lazyjob-core/src/persistence/` to a `Repository` trait hierarchy (JobRepository, ApplicationRepository, etc.) — no implementation changes, just trait extraction
- [ ] Add `SqliteJobRepository`, `SqliteApplicationRepository`, etc. implementations in the same modules
- [ ] Add `TenantRepository`, `UserRepository` in `lazyjob-core/src/persistence/` for SaaS multi-tenancy (Phase 3)
- [ ] Create `lazyjob-sync/` crate scaffold with `SyncProtocol`, `SyncState`, `SyncOperation` types
- [ ] Add Supabase client crate (`lazyjob-sync`) to workspace with `Repository` implementations for PostgreSQL
- [ ] Add `tenant_id` column to all core tables in a new migration (`010_add_tenant_id.sql`)
- [ ] Implement sync deduplication in `SyncProtocol`: don't re-upload unchanged rows (compare `updated_at`)
- [ ] Implement `offer_details` and `token_usage_log` to `NEVER_SYNC_TABLES` list in `lazyjob-sync/src/never_sync.rs`
- [ ] Write Supabase migration files in `lazyjob-sync/migrations/` for PostgreSQL schema creation
