# SQLite Persistence Layer

## Status
Researching

## Problem Statement

LazyJob needs a local-first data persistence layer that:
1. Stores all job search data (jobs, applications, contacts, life sheet)
2. Supports concurrent access from multiple processes (TUI + ralph subprocesses)
3. Provides ACID transactions for data integrity
4. Handles schema evolution through migrations
5. Supports backup and recovery
6. Works offline-first (no network dependency)

SQLite is the natural choice for a local-first desktop/CLI application. This spec covers the persistence architecture.

---

## Research Findings

### rusqlite (Synchronous, Single-Threaded)

The `rusqlite` crate provides ergonomic SQLite bindings:

**Connection Pattern**
```rust
use rusqlite::{Connection, Result};

let conn = Connection::open("lazyjob.db")?;

conn.execute(
    "CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    [],
)?;
```

**Prepared Statements with Params**
```rust
let mut stmt = conn.prepare("INSERT INTO person (name) VALUES (?1)")?;
stmt.execute(params!["Alice"])?;

// Or inline
conn.execute("INSERT INTO person (name) VALUES (?1)", params!["Bob"])?;
```

**Transaction Support**
```rust
let tx = conn.transaction()?;
// All operations in transaction
tx.execute("INSERT INTO ...", [])?;
tx.commit()?;

// Or use savepoint for nested transactions
let sp = conn.savepoint()?;
sp.execute("INSERT INTO ...", [])?;
sp.commit()?;
```

**Key Limitation**: rusqlite is synchronous and single-threaded. For TUI apps using tokio, this requires spawning blocking tasks.

### rusqlite Concurrency Model

**busy_timeout**: Set to handle lock contention
```rust
conn.busy_timeout(Duration::from_millis(5000))?;
```

**WAL Mode**: Enables concurrent reads during writes
```rust
conn.execute_batch("PRAGMA journal_mode=WAL;")?;
```

**WAL Checkpoint Modes**
- `PASSIVE` (0): Do as much as possible without blocking
- `FULL` (1): Wait for writers, then checkpoint
- `RESTART` (2): Like FULL but wait for readers
- `TRUNCATE` (3): Like RESTART but also truncate WAL

### sqlx (Async, Compile-Time Query Checking)

The `sqlx` crate provides async SQL with compile-time checked queries:

**Async Query**
```rust
use sqlx::{SqlitePool, query, query_as};

let pool = SqlitePool::connect("sqlite://lazyjob.db").await?;

let rows = query!("SELECT * FROM jobs WHERE status = ?", status)
    .fetch_all(&pool)
    .await?;
```

**Type-Safe Results**
```rust
let job = query_as!(Job, "SELECT id, title, company FROM jobs WHERE id = ?", id)
    .fetch_one(&pool)
    .await?;
```

**Key Advantage**: Compile-time query verification against actual database schema.

**Key Limitation**: Requires `sqlx prepare` for compile-time checking, or uses `sqlx.toml` for offline mode.

### rusqlite_migration (Schema Migrations)

```rust
use rusqlite_migration::{Migrations, M};

let migrations = Migrations::new(vec![
    M::up("CREATE TABLE animals (name TEXT);").down("DROP TABLE animals;"),
    M::up("CREATE TABLE food (name TEXT);").down("DROP TABLE food;"),
]);

// Apply all migrations
migrations.to_latest(&mut conn).unwrap();

// Rollback to specific version
migrations.to_version(&mut conn, 1);
```

Uses SQLite's `user_version` PRAGMA to track schema version.

### sqlx Migration

```rust
use sqlx::migrate::Migrator;
use std::path::Path;

let m = Migrator::new(Path::new("./migrations")).await?;
m.run(&pool).await?;
```

Migrations stored in `./migrations/{version}_{name}.sql` files.

### Backup and Recovery

**rusqlite Online Backup**
```rust
use rusqlite::backup;

let backup = backup::Backup::new(&src_conn, &mut dst_conn)?;
backup.run_to_completion(5, Duration::from_millis(250), Some(progress))?;
```

**Recommended Backup Strategy**
- WAL mode + periodic `VACUUM`
- Auto-backup on startup if WAL file exists
- Keep last N backups

### Multi-Process Concurrency

SQLite supports multi-process access:
- WAL mode enables concurrent reads from multiple processes
- Writes still acquire exclusive lock
- `busy_timeout` prevents immediate failures on lock

For TUI + ralph subprocesses:
- Main process holds WAL lock for writes
- Ralph subprocesses need short-lived connections
- Consider file-based locking for critical sections

---

## Design Options

### Option A: rusqlite with Blocking in Tokio

**Description**: Use synchronous rusqlite with `tokio::task::spawn_blocking` for database operations.

**Pros**:
- Mature, stable crate
- Rich feature set (WAL, backup, FTS5)
- No async complexity
- Smaller binary (no async runtime overhead)

**Cons**:
- Must wrap all DB calls in `spawn_blocking`
- Easy to accidentally block the async executor
- No compile-time query checking

**Best for**: Simple applications, avoiding async complexity

### Option B: sqlx with SQLite (Recommended)

**Description**: Use async sqlx with SQLite for all database operations.

**Pros**:
- True async I/O, no blocking
- Compile-time query checking (with offline mode)
- Type-safe result mapping
- Migrations built-in
- Connection pooling

**Cons**:
- Larger binary (async runtime)
- Compile-time checking requires `CARGO_BUILD_SQLX` or `sqlx.toml`
- Async adds complexity

**Best for**: Production applications with async architecture

### Option C: Hybrid (rusqlite + ralph, sqlx main)

**Description**: Main app uses rusqlite directly, ralph subprocesses use separate SQLite file or read-only replica.

**Pros**:
- Avoids IPC for database access
- Ralph can be completely independent
- Clear separation of concerns

**Cons**:
- Two database files to sync
- Potential for data inconsistency
- More complex backup strategy

**Best for**: When ralph should be truly decoupled

---

## Recommended Approach

**Option B: sqlx with SQLite** is recommended.

Rationale:
1. LazyJob uses tokio for async operations (LLM calls, file I/O)
2. sqlx provides compile-time safety for queries
3. Migrations are first-class citizens
4. Connection pooling handles concurrent access
5. The TUI main loop is async anyway

---

## Architecture

### Database File Location

```
~/.lazyjob/
├── config.yaml          # User configuration
├── life-sheet.yaml      # Life sheet (user-editable)
├── lazyjob.db           # Main SQLite database
├── lazyjob.db-wal       # WAL file (if WAL mode enabled)
├── lazyjob.db-shm       # Shared memory (if WAL mode enabled)
└── backups/
    ├── 2024-01-15.db    # Dated backups
    └── 2024-01-14.db
```

### Database Schema

See `03-life-sheet-data-model.md` for Life Sheet tables. Here are the core LazyJob tables:

```sql
-- Core Tables

CREATE TABLE jobs (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    company_name TEXT NOT NULL,
    company_id TEXT,
    location TEXT,
    remote TEXT,  -- 'yes', 'no', 'hybrid'
    url TEXT,
    description TEXT,
    salary_min INTEGER,
    salary_max INTEGER,
    salary_currency TEXT DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'discovered',
    interest_level INTEGER DEFAULT 3,  -- 1-5
    source TEXT,  -- 'linkedin', 'indeed', 'greenhouse', 'manual'
    discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE companies (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    website TEXT,
    linkedin_url TEXT,
    crunchbase_url TEXT,
    industry TEXT,
    size TEXT,  -- 'startup', 'small', 'medium', 'large', 'enterprise'
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

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

CREATE TABLE contacts (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    role TEXT,
    email TEXT,
    phone TEXT,
    linkedin_url TEXT,
    twitter_handle TEXT,
    company_id TEXT,
    relationship TEXT,  -- 'recruiter', 'hiring-manager', 'interviewer', 'referral', 'network'
    quality INTEGER DEFAULT 3,  -- 1-5
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (company_id) REFERENCES companies(id) ON DELETE SET NULL
);

CREATE TABLE interviews (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL,
    type TEXT NOT NULL,  -- 'phone-screen', 'technical', 'behavioral', 'onsite', 'final'
    scheduled_at TEXT,
    duration_minutes INTEGER,
    location TEXT,
    meeting_url TEXT,
    interviewer_names TEXT,  -- JSON array
    status TEXT NOT NULL DEFAULT 'scheduled',  -- 'scheduled', 'completed', 'cancelled', 'no-show'
    feedback TEXT,
    rating INTEGER,  -- 1-5
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);

CREATE TABLE offers (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    application_id TEXT NOT NULL,
    salary INTEGER NOT NULL,
    bonus INTEGER,
    equity TEXT,  -- e.g., "0.1%"
    start_date TEXT,
    expires_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending', 'accepted', 'declined', 'withdrawn'
    notes TEXT,
    created_at TEXT NOT DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);

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

CREATE TABLE activity_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    entity_type TEXT NOT NULL,  -- 'job', 'application', 'interview', 'contact'
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,  -- 'created', 'updated', 'status_changed', 'interview_scheduled'
    details TEXT,  -- JSON for additional context
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Schema migrations tracking
CREATE TABLE _sqlx_migrations (
    version TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_company ON jobs(company_id);
CREATE INDEX idx_applications_job ON applications(job_id);
CREATE INDEX idx_applications_status ON applications(status);
CREATE INDEX idx_contacts_company ON contacts(company_id);
CREATE INDEX idx_interviews_application ON interviews(application_id);
CREATE INDEX idx_activity_entity ON activity_log(entity_type, entity_id);
CREATE INDEX idx_reminders_due ON reminders(due_at) WHERE completed = 0;
```

### Connection Management

```rust
// lazyjob-core/src/persistence/mod.rs

use sqlx::{SqlitePool, SqlitePoolOptions, sqlite::SqliteJournalMode};
use std::time::Duration;

pub struct Database {
    pool: SqlitePool,
    backup_dir: PathBuf,
}

impl Database {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Configure connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(5)  // TUI + multiple ralph subprocesses
            .acquire_timeout(Duration::from_secs(10))
            .connect(format!(
                "sqlite:{}?mode=rwc&journal_mode=WAL&foreign_keys=on",
                db_path.display()
            ))
            .await?;

        // Enable foreign keys
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await?;

        // Run migrations
        self.run_migrations(&pool).await?;

        Ok(Self { pool, backup_dir })
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        let migrator = Migrator::new(Path::new("./migrations")).await?;
        migrator.run(pool).await?;
        Ok(())
    }
}

impl Deref for Database {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target { &self.pool }
}
```

### Repository Pattern

```rust
// lazyjob-core/src/persistence/jobs.rs

pub struct JobRepository<'db> {
    pool: &'db SqlitePool,
}

impl<'db> JobRepository<'db> {
    pub fn new(pool: &'db SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>> {
        let mut query = String::from(
            "SELECT id, title, company_name, location, status, ... FROM jobs WHERE 1=1"
        );

        if let Some(status) = &filter.status {
            query.push_str(&format!(" AND status = '{}'", status));
        }
        if let Some(company_id) = &filter.company_id {
            query.push_str(&format!(" AND company_id = '{}'", company_id));
        }

        query.push_str(" ORDER BY discovered_at DESC");

        sqlx::query_as::<_, Job>(&query)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Job>> {
        sqlx::query_as!(
            Job,
            "SELECT * FROM jobs WHERE id = ?",
            id
        )
        .fetch_optional(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn insert(&self, job: &Job) -> Result<()> {
        sqlx::query!(
            "INSERT INTO jobs (id, title, company_name, ...) VALUES (?, ?, ...)",
            job.id,
            job.title,
            job.company_name,
            // ... all fields
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn update(&self, job: &Job) -> Result<()> {
        sqlx::query!(
            "UPDATE jobs SET title = ?, status = ?, updated_at = datetime('now') WHERE id = ?",
            job.title,
            job.status,
            job.id
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query!("DELETE FROM jobs WHERE id = ?", id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
```

### Migration Files

```
migrations/
├── 001_initial_schema.sql
├── 002_add_job_contacts.sql
├── 003_add_interviews.sql
└── 004_add_offers.sql
```

**001_initial_schema.sql**
```sql
-- Jobs table
CREATE TABLE IF NOT EXISTS jobs (
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

CREATE TABLE IF NOT EXISTS companies (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    website TEXT,
    linkedin_url TEXT,
    industry TEXT,
    size TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_company ON jobs(company_id);
```

### Backup Strategy

```rust
// lazyjob-core/src/persistence/backup.rs

impl Database {
    pub async fn backup(&self) -> Result<PathBuf> {
        let backup_dir = self.backup_dir.join(
            chrono::Local::now().format("%Y-%m-%d")
        );
        std::fs::create_dir_all(&backup_dir)?;

        let backup_path = backup_dir.join("lazyjob.db");
        let src = Connection::open(&*self.pool).await?;
        let dst = Connection::open(&backup_path)?;

        backup::Backup::new(&src, &dst)?
            .run_to_completion(5, Duration::from_millis(250), None)?;

        // Also vacuum to compact
        sqlx::query("VACUUM")
            .execute(&*self.pool)
            .await?;

        Ok(backup_path)
    }

    pub async fn restore(&self, backup_path: &Path) -> Result<()> {
        // Verify backup is valid
        let backup_conn = Connection::open(backup_path)?;

        // Replace current database
        sqlx::query("DETACH DATABASE lazyjob")
            .execute(&*self.pool)
            .await?;

        // Copy backup over
        let current_path = PathBuf::from("lazyjob.db");
        std::fs::copy(backup_path, &current_path)?;

        Ok(())
    }
}
```

### Auto-Backup on Startup

```rust
impl Database {
    pub async fn with_auto_backup(db_path: &Path, backup_dir: &Path) -> Result<Self> {
        let db = Self::new(db_path).await?;

        // Check if WAL file exists (indicates dirty shutdown)
        let wal_path = db_path.with_extension("db-wal");
        if wal_path.exists() {
            // Restore from most recent backup or create new backup
            if let Some(latest_backup) = find_latest_backup(backup_dir)? {
                if file_is_newer(&wal_path, latest_backup)? {
                    db.restore(latest_backup).await?;
                }
            }
            db.backup().await?;
        }

        Ok(db)
    }
}
```

### Ralph Subprocess Database Access

For Ralph subprocesses that need to read/write data, options:

**Option 1: Direct SQLite Access**
Ralph subprocesses open their own connection to the same SQLite file. SQLite WAL handles concurrency.

**Option 2: Unix Domain Socket API**
Ralph subprocesses communicate with TUI via IPC, TUI handles all DB operations.

```rust
// lazyjob-ralph/src/ipc/database.rs

pub struct RalphDatabaseProxy {
    socket: UnixStream,
}

impl RalphDatabaseProxy {
    pub async fn query_jobs(&self, filter: JobFilter) -> Result<Vec<Job>> {
        let request = DbRequest::ListJobs { filter };
        self.send_request(request).await
    }

    pub async fn update_job(&self, job: Job) -> Result<()> {
        let request = DbRequest::UpdateJob { job };
        self.send_request(request).await
    }
}
```

**Recommendation**: Option 1 (direct SQLite) for MVP, with WAL mode enabled. Ralph subprocesses should:
- Use shorter `busy_timeout` (1-2 seconds)
- Retry on `SQLITE_BUSY`
- Close connections promptly

---

## Failure Modes

1. **Database Locked**: `SQLITE_BUSY` - exponential backoff retry, max 3 attempts
2. **Corruption**: Restore from backup, show user error message
3. **Disk Full**: Detect before write, warn user, suggest cleanup
4. **Dirty WAL on Startup**: Detect via WAL file timestamp vs DB timestamp, restore or backup
5. **Migration Failure**: Log error, attempt `.down()` migration, fail fast with user message
6. **Foreign Key Violation**: Return user-friendly error with context

---

## Open Questions

1. **Query Complexity**: For complex filtering/aggregation, should we use SQL or in-memory filtering?
2. **Full-Text Search**: Should we use SQLite FTS5 for job description search?
3. **Ralph Connection Pooling**: Should ralph subprocesses maintain their own pool or use shared connections?
4. **Backup Retention**: How many backups to keep? (Default: 7 daily, 4 weekly)

---

## Dependencies

```toml
# lazyjob-core/Cargo.toml
[dependencies]
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }
rusqlite = { version = "0.32", features = ["backup"] }
tokio = { version = "1", features = ["full"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }

[dev-dependencies]
sqlx = { features = ["sqlite", "migrate"] }
```

---

## Sources

- [rusqlite Documentation](https://docs.rs/rusqlite/0.39.0/rusqlite/)
- [sqlx Documentation](https://docs.rs/sqlx/latest/sqlx/)
- [rusqlite_migration crate](https://docs.rs/rusqlite_migration/latest/rusqlite_migration/)
- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [SQLite Backup API](https://www.sqlite.org/backup.html)
- [Tauri SQL Plugin](https://github.com/tauri-apps/tauri-plugin-sql) - reference for sqlx + Tauri
