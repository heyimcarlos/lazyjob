# SaaS Migration Path

## Status
Researching

## Problem Statement

LazyJob starts as a local-first CLI tool. But the goal is a SaaS product. This spec covers the architectural decisions that enable a smooth transition from local app to cloud service.

---

## Research Findings

### Local-First Architecture Patterns

**Key Principles**:
1. All data stored locally first
2. Sync to cloud as secondary concern
3. Works fully offline
4. User owns their data

**Examples**:
- Obsidian: Local Markdown files, optional sync
- Linear: Always cloud, but excellent offline support
- Notion: Cloud-first, offline limited
- 1Password: Local vault, cloud sync

### Supabase Migration Path

Supabase is a Firebase alternative built on PostgreSQL. Local-first apps often use Supabase as the backend.

**Migration Strategy**:
1. Keep SQLite for local
2. Add Supabase as optional sync target
3. Conflict resolution via "last write wins" or CRDT
4. Sync happens in background

### PostgreSQL vs SQLite

For SaaS, PostgreSQL provides:
- Row-level security
- Better concurrency
- Cloud-native deployment
- Richer query language

**Hybrid Approach**:
- Local: SQLite
- Cloud: PostgreSQL (via Supabase or self-hosted)
- Sync: Custom sync layer or Fivetran

### Real-Time Considerations

**For SaaS Features**:
- Real-time collaboration (multiple users)
- Live notifications
- Presence indicators

**Technology Options**:
- Supabase Realtime (PostgreSQL + WebSocket)
- Firebase Realtime / Firestore
- Ably / Pusher
- Custom WebSocket server

---

## Migration Architecture

### Phase 1: Local-First (Current)

```
┌─────────────────────────────────────────┐
│           LazyJob TUI (Local)              │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │ SQLite  │  │ LLM     │  │ Ralph   │   │
│  │ DB      │  │ Client  │  │ Loops   │   │
│  └─────────┘  └─────────┘  └─────────┘   │
│                                             │
└─────────────────────────────────────────┘
```

### Phase 2: Add Sync Option

```
┌─────────────────────────────────────────┐
│           LazyJob TUI (Local)              │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │ SQLite  │  │ LLM     │  │ Ralph   │   │
│  │ DB      │  │ Client  │  │ Loops   │   │
│  └────┬────┘  └─────────┘  └─────────┘   │
│       │                                      │
│       ▼                                      │
│  ┌──────────────────────────────────┐      │
│  │        Sync Layer                  │      │
│  │  (Optional background sync)         │      │
│  └──────────────┬───────────────────┘      │
└─────────────────┼─────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│         Cloud Storage (Optional)         │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │Jobs     │  │ Life    │  │Auth     │   │
│  │Table    │  │ Sheet   │  │Users    │   │
│  └─────────┘  └─────────┘  └─────────┘   │
│                                             │
│  Supabase / PostgreSQL                      │
└─────────────────────────────────────────┘
```

### Phase 3: Full SaaS

```
┌─────────────────────────────────────────┐
│           LazyJob Web (SaaS)               │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │ React   │  │ LLM     │  │ API     │   │
│  │ App      │  │ Proxy  │  │ Gateway │   │
│  └─────────┘  └─────────┘  └─────────┘   │
└─────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│         Backend Services                   │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐   │
│  │ Auth    │  │ Jobs    │  │ Ralph   │   │
│  │ Service │  │ Service │  │ Service │   │
│  └─────────┘  └─────────┘  └─────────┘   │
│                                             │
│  ┌──────────────────────────────────┐      │
│  │        PostgreSQL                  │      │
│  │  (Row-level security)              │      │
│  └──────────────────────────────────┘      │
└─────────────────────────────────────────┘
```

---

## Key Technical Decisions

### 1. Shared Data Model

```rust
// Both SQLite and PostgreSQL use the same entities
// Schema differences handled by repository pattern

pub trait JobRepository {
    async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>>;
    async fn get(&self, id: &Uuid) -> Result<Option<Job>>;
    async fn insert(&self, job: &Job) -> Result<()>;
    async fn update(&self, job: &Job) -> Result<()>;
    async fn delete(&self, id: &Uuid) -> Result<()>;
}

// Implement for SQLite (local)
impl JobRepository for SqliteJobRepository { ... }

// Implement for PostgreSQL (cloud)
impl JobRepository for PostgresJobRepository { ... }
```

### 2. Authentication

**Local**: No auth required (single user on local machine)

**SaaS**:
- OAuth providers (Google, GitHub)
- Email magic links
- SSO for enterprise

```rust
pub enum AuthProvider {
    Google,
    GitHub,
    EmailMagicLink,
    EnterpriseSSO,
}

pub struct User {
    pub id: Uuid,
    pub email: String,
    pub auth_provider: AuthProvider,
    pub created_at: DateTime<Utc>,
}
```

### 3. Multi-tenancy

```rust
pub struct Tenant {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub plan: Plan,
    pub storage_bytes: i64,
}

pub enum Plan {
    Free,
    Pro,
    Team,
    Enterprise,
}

// Row-level security in PostgreSQL
// Every table has tenant_id column
```

### 4. Sync Protocol

```rust
pub enum SyncOperation {
    Insert { table: String, data: serde_json::Value },
    Update { table: String, id: Uuid, data: serde_json::Value },
    Delete { table: String, id: Uuid },
}

pub struct SyncState {
    pub last_sync: DateTime<Utc>,
    pub cursor: String,  // For pagination
    pub pending: Vec<SyncOperation>,
}

impl SyncProtocol {
    // Conflict resolution: Last-write-wins with vector clocks
    // Or: CRDT for specific data types
}
```

### 5. Ralph as a Service

**Local**: Ralph runs on user's machine

**SaaS**: Ralph runs on server, accessed via API

```rust
// Cloud Ralph API
pub async fn start_ralph_loop(
    auth: AuthenticatedUser,
    loop_type: LoopType,
    params: serde_json::Value,
) -> Result<RalphLoopHandle> {
    // Start Ralph loop on server
    // Return handle for polling/status
}

// WebSocket for streaming output
pub async fn ralph_output_stream(
    auth: AuthenticatedUser,
    loop_id: Uuid,
) -> Result<WebSocketStream> {
    // Stream Ralph loop output to client
}
```

---

## SaaS Pricing Tiers

| Feature | Free | Pro | Team | Enterprise |
|---------|------|-----|------|------------|
| Local SQLite | ✓ | ✓ | ✓ | ✓ |
| Cloud Sync | - | ✓ | ✓ | ✓ |
| Ralph Loops | Limited | ✓ | ✓ | ✓ |
| Job Applications | 20 | Unlimited | Unlimited | Unlimited |
| Team Collaboration | - | - | ✓ | ✓ |
| SSO | - | - | - | ✓ |
| Custom Branding | - | - | - | ✓ |
| API Access | - | - | ✓ | ✓ |
| SLA | - | Best effort | 99.9% | 99.99% |

---

## Migration Checklist

### Code Changes

- [ ] Extract repository interfaces
- [ ] Implement PostgreSQL repositories
- [ ] Add authentication layer
- [ ] Implement multi-tenancy
- [ ] Build sync protocol
- [ ] Create API server
- [ ] Build web UI (or maintain TUI for local)

### Infrastructure

- [ ] Set up PostgreSQL (Supabase or self-hosted)
- [ ] Configure auth providers
- [ ] Set up CDN for static assets
- [ ] Configure domain/DNS
- [ ] Set up monitoring (Sentry, Datadog)
- [ ] Set up CI/CD

### Compliance

- [ ] GDPR compliance (data export, deletion)
- [ ] SOC 2 (if enterprise)
- [ ] Privacy policy
- [ ] Terms of service

---

## Open Questions

1. **TUI vs Web UI**: Should SaaS version maintain TUI or switch to web?
2. **Data Portability**: Can users export their cloud data anytime?
3. **Pricing**: How to price without hurting local-first brand?

---

## Sources

- [Supabase Documentation](https://supabase.com/docs)
- [Local-first Software](https://www.inkandswitch.com/local-first/)
- [Linear's Migration Story](https://linear.app/blog/building-linear)
