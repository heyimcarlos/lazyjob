# SQLite Persistence Layer — Implementation Plan

## Spec Reference
- **Spec file**: `specs/04-sqlite-persistence.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary

Implement a local-first SQLite persistence layer for LazyJob using `sqlx` with async I/O. The persistence layer provides ACID transactions, WAL-mode concurrency for multi-process access (TUI + ralph subprocesses), schema migrations, and backup/recovery. All LazyJob data (jobs, applications, contacts, interviews, offers) lives in SQLite at `~/.lazyjob/lazyjob.db`.

## Problem Statement

LazyJob needs a local-first data persistence layer that:
1. Stores all job search data (jobs, applications, contacts, life sheet)
2. Supports concurrent access from multiple processes (TUI + ralph subprocesses)
3. Provides ACID transactions for data integrity
4. Handles schema evolution through migrations
5. Supports backup and recovery
6. Works offline-first (no network dependency)

## Implementation Phases

### Phase 1: Foundation
1. Create `lazyjob-core/src/persistence/` module structure
2. Define `Database` struct with `SqlitePool`
3. Implement connection initialization with WAL mode and foreign keys
4. Set up `sqlx` migrations directory structure
5. Create initial migration file (`001_initial_schema.sql`)
6. Implement graceful shutdown with WAL checkpoint

### Phase 2: Core Implementation
1. Implement repository structs: `JobRepository`, `CompanyRepository`, `ApplicationRepository`, `ContactRepository`, `InterviewRepository`, `OfferRepository`, `ReminderRepository`
2. Create filter types for each repository (`JobFilter`, `ApplicationFilter`, etc.)
3. Implement CRUD operations with compile-time query checking via `sqlx::query!`
4. Add activity logging for audit trail
5. Implement transaction support for multi-entity operations

### Phase 3: Integration & Polish
1. Implement backup/restore functionality with timestamped files
2. Add auto-backup on startup if WAL file exists (dirty shutdown detection)
3. Implement Ralph subprocess database access (direct SQLite with WAL)
4. Add failure mode handling: lock contention with retry, corruption recovery
5. Integrate with LazyJob error types (`thiserror`/`anyhow`)
6. Add health check endpoint for TUI status display

## Data Model

### New Database Tables

```sql
-- jobs: Core job listing table (from spec schema)
CREATE TABLE jobs (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    company_name TEXT NOT NULL,
    company_id TEXT,
    location TEXT,
    remote TEXT,
    url TEXT,
    description TEXT,
    salary_min INTEGER,
    salary_max INTEGER,
    salary_currency TEXT DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'discovered',
    interest_level INTEGER DEFAULT 3,
    source TEXT,
    discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- companies: Company information
CREATE TABLE companies (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    website TEXT,
    linkedin_url TEXT,
    crunchbase_url TEXT,
    industry TEXT,
    size TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- applications: Job applications tracking
CREATE TABLE applications (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    resume_version TEXT,
    cover_letter_version TEXT,
    submitted_at TEXT,
    last_contact_at TEXT,
    next_follow_up TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE
);

-- contacts: Professional network contacts
CREATE TABLE contacts (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    role TEXT,
    email TEXT,
    phone TEXT,
    linkedin_url TEXT,
    twitter_handle TEXT,
    company_id TEXT,
    relationship TEXT,
    quality INTEGER DEFAULT 3,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (company_id) REFERENCES companies(id) ON DELETE SET NULL
);

-- interviews: Interview scheduling and feedback
CREATE TABLE interviews (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL,
    type TEXT NOT NULL,
    scheduled_at TEXT,
    duration_minutes INTEGER,
    location TEXT,
    meeting_url TEXT,
    interviewer_names TEXT,
    status TEXT NOT NULL DEFAULT 'scheduled',
    feedback TEXT,
    rating INTEGER,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);

-- offers: Job offers and compensation details
CREATE TABLE offers (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL,
    salary INTEGER NOT NULL,
    bonus INTEGER,
    equity TEXT,
    start_date TEXT,
    expires_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);

-- reminders: Follow-up reminders
CREATE TABLE reminders (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    description TEXT,
    due_at TEXT NOT NULL,
    completed INTEGER DEFAULT 0,
    application_id TEXT,
    job_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE SET NULL,
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE SET NULL
);

-- activity_log: Audit trail for all entity changes
CREATE TABLE activity_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    details TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Required indexes
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_company ON jobs(company_id);
CREATE INDEX idx_applications_job ON applications(job_id);
CREATE INDEX idx_applications_status ON applications(status);
CREATE INDEX idx_contacts_company ON contacts(company_id);
CREATE INDEX idx_interviews_application ON interviews(application_id);
CREATE INDEX idx_activity_entity ON activity_log(entity_type, entity_id);
CREATE INDEX idx_reminders_due ON reminders(due_at) WHERE completed = 0;
```

### New Rust Structs

```rust
// lazyjob-core/src/persistence/mod.rs

pub struct Database {
    pool: SqlitePool,
    backup_dir: PathBuf,
}

pub type Result<T> = std::result::Result<T, DbError>;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("not found: {entity} {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("constraint violation: {0}")]
    Constraint(String),
}
```

Repository structs with filter types:
```rust
pub struct JobFilter {
    pub status: Option<String>,
    pub company_id: Option<String>,
    pub source: Option<String>,
    pub interest_level_min: Option<i32>,
}

pub struct ApplicationFilter {
    pub status: Option<String>,
    pub job_id: Option<String>,
}
```

## API Surface

### Module Hierarchy
```
lazyjob-core/src/persistence/
├── mod.rs           # Database struct, connection management, exports
├── error.rs         # DbError enum, Result type alias
├── jobs.rs          # JobRepository
├── companies.rs     # CompanyRepository
├── applications.rs  # ApplicationRepository
├── contacts.rs      # ContactRepository
├── interviews.rs    # InterviewRepository
├── offers.rs        # OfferRepository
├── reminders.rs     # ReminderRepository
├── activity.rs      # ActivityLogRepository
└── backup.rs        # Backup/restore functionality
```

### Public API

```rust
// Database initialization
impl Database {
    pub async fn new(db_path: &Path) -> Result<Self>
    pub async fn with_auto_backup(db_path: &Path, backup_dir: &Path) -> Result<Self>
    pub async fn close(self) -> Result<()>
    pub async fn backup(&self) -> Result<PathBuf>
    pub async fn restore(&self, backup_path: &Path) -> Result<()>
    pub fn pool(&self) -> &SqlitePool
}

// Repository access via Deref
impl Deref for Database {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target { &self.pool }
}

// Each repository provides:
impl JobRepository {
    pub fn new(pool: &SqlitePool) -> Self
    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>>
    pub async fn get(&self, id: &str) -> Result<Option<Job>>
    pub async fn insert(&self, job: &Job) -> Result<()>
    pub async fn update(&self, job: &Job) -> Result<()>
    pub async fn delete(&self, id: &str) -> Result<()>
}
```

### Integration with Other Crates

- **lazyjob-core**: Provides persistence module consumed by TUI and ralph
- **lazyjob-tui**: Uses `Database` for all CRUD operations via repository pattern
- **lazyjob-ralph**: Opens its own connection to SQLite for offline operation; uses same schema

## Key Technical Decisions

### sqlx over rusqlite
**Decision**: Use `sqlx` with async SQLite driver
**Rationale**: LazyJob uses tokio for async operations (LLM calls, file I/O); sqlx provides compile-time query safety without blocking the async executor
**Alternative rejected**: rusqlite with `spawn_blocking` — easy to accidentally block executor, no compile-time query checking

### WAL Mode for Concurrency
**Decision**: Enable WAL journal mode
**Rationale**: Allows concurrent reads from TUI + ralph subprocesses while maintaining ACID properties; readers don't block writers
**Tradeoff**: WAL file adds overhead but enables the multi-process use case

### Ralph Direct Database Access
**Decision**: Ralph subprocesses open their own SQLite connections
**Rationale**: Keeps ralph decoupled from TUI; avoids IPC complexity for MVP
**Tradeoff**: Potential for lock contention; mitigated by busy_timeout and retry logic

### Migration Strategy
**Decision**: Use sqlx's built-in `Migrator` with SQL migration files
**Rationale**: Type-safe, version-tracked migrations with rollback support
**Tradeoff**: Requires migration files in `./migrations/` directory at build time

## File Structure

```
lazyjob-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    └── persistence/
        ├── mod.rs           # Database, exports, repository construction
        ├── error.rs         # DbError, Result alias
        ├── jobs.rs          # JobRepository, Job, JobFilter
        ├── companies.rs     # CompanyRepository, Company
        ├── applications.rs  # ApplicationRepository, Application
        ├── contacts.rs      # ContactRepository, Contact
        ├── interviews.rs    # InterviewRepository, Interview
        ├── offers.rs        # OfferRepository, Offer
        ├── reminders.rs     # ReminderRepository, Reminder
        ├── activity.rs      # ActivityLogRepository
        └── backup.rs        # Backup/restore logic

migrations/
├── 001_initial_schema.sql   # Core tables
├── 002_add_companies.sql     # Company entity
├── 003_add_applications.sql  # Applications with job FK
├── 004_add_contacts.sql      # Contacts with company FK
├── 005_add_interviews.sql    # Interview tracking
├── 006_add_offers.sql        # Offer management
└── 007_add_reminders.sql     # Reminder system
```

## Dependencies

### lazyjob-core/Cargo.toml additions
```toml
[dependencies]
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono"] }
tokio = { version = "1", features = ["full"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }

[dev-dependencies]
sqlx = { features = ["sqlite", "migrate"] }
tempfile = "3"
```

## Testing Strategy

### Unit Tests
- **Repository CRUD**: Test each repository's create/read/update/delete with in-memory SQLite
- **Filter combinations**: Test repository `list()` with various filter permutations
- **Transaction rollback**: Verify changes roll back on error
- **Error handling**: Test constraint violations, not found, foreign key errors

### Integration Tests
- **Database initialization**: Test `Database::new()` creates schema correctly
- **Migration application**: Test applying multiple migrations in sequence
- **Backup/restore cycle**: Create data, backup, restore, verify data integrity
- **Concurrent access**: Spawn multiple tasks querying same database

### Edge Cases
- `SQLITE_BUSY` lock contention: Verify exponential backoff retry works
- Dirty WAL on startup: Verify auto-backup triggers correctly
- Migration failure: Verify graceful error with meaningful message
- Foreign key violations: Test cascading deletes work correctly

## Open Questions

1. **Full-Text Search**: Should we use SQLite FTS5 for job description search? (defer to future iteration)
2. **Query Complexity**: For complex filtering/aggregation, use SQL or in-memory filtering? (start with SQL, optimize if needed)
3. **Backup Retention**: Keep 7 daily + 4 weekly backups? (implement as specified)
4. **Ralph Connection Pooling**: Should ralph subprocesses maintain their own pool or use shared connections? (Ralph uses own pool with shorter busy_timeout)

## Effort Estimate

**Rough estimate**: 5-7 days

**Reasoning**:
- Phase 1 (Foundation): 1-2 days — module structure, connection, migrations
- Phase 2 (Core Repositories): 2-3 days — 7 repositories with full CRUD
- Phase 3 (Integration/Polish): 2 days — backup, error handling, testing

**Dependencies**:
- Requires `01-architecture.md` for overall crate structure
- Schema aligns with `03-life-sheet-data-model.md` but that spec covers a different data model layer