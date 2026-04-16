# Spec: Architecture — SQLite Persistence

**JTBD**: A fast, reliable tool that works offline; Keep my data private and portable
**Topic**: Define the SQLite persistence layer: schema, connection management, repository pattern, migrations, and backup
**Domain**: architecture

---

## What

SQLite as the sole persistence engine for LazyJob's local-first architecture. All data lives in `~/.lazyjob/lazyjob.db` with WAL mode enabled for multi-process concurrent access (TUI main process + Ralph subprocesses). The `Database` struct wraps `sqlx::SqlitePool` with repository traits for each domain entity. Migrations are sqlx migration files tracked in `_sqlx_migrations`. Auto-backup runs on startup if the WAL file indicates a dirty shutdown.

## Why

SQLite is the right choice for a local-first CLI tool because:
- Zero configuration — no server process, no port conflicts
- ACID transactions — application state is always consistent
- WAL mode — concurrent reads from TUI and Ralph subprocesses without lock contention
- Portability — single `.db` file is trivially backup-able, exportable, and (if needed) migratable to PostgreSQL via the Repository trait

The Repository trait is the key architectural decision: define the interface once in `lazyjob-core`, implement for SQLite now, implement for PostgreSQL when SaaS migration happens. No business logic changes required.

## How

### Database File Location

```
~/.lazyjob/
├── config.yaml          # User configuration (TOML, not in DB)
├── life-sheet.yaml      # Life sheet (YAML, human-editable)
├── lazyjob.db           # Main SQLite database
├── lazyjob.db-wal       # WAL file (auto-created in WAL mode)
├── lazyjob.db-shm       # Shared memory file (auto-created in WAL mode)
└── backups/
    ├── 2024-01-15.db    # Dated snapshots
    └── ...
```

### Connection Pool Setup

```rust
// lazyjob-core/src/persistence/database.rs

use sqlx::{SqlitePool, SqlitePoolOptions};
use std::time::Duration;

pub struct Database {
    pool: SqlitePool,
    backup_dir: PathBuf,
}

impl Database {
    pub async fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        std::fs::create_dir_all(data_dir.join("backups"))?;

        let db_path = data_dir.join("lazyjob.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(5)  // TUI + multiple Ralph subprocesses
            .acquire_timeout(Duration::from_secs(10))
            .connect(&format!(
                "sqlite:{}?mode=rwc&journal_mode=WAL&foreign_keys=on",
                db_path.display()
            ))
            .await?;

        // Enable foreign keys on every connection
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await?;

        // Run pending migrations
        let migrator = Migrator::new(Path::new("lazyjob-core/src/persistence/migrations"))
            .await?;
        migrator.run(&pool).await?;

        Ok(Self { pool, backup_dir: data_dir.join("backups") })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

impl Deref for Database {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target { &self.pool }
}
```

### Repository Traits

```rust
// lazyjob-core/src/persistence/jobs.rs

pub struct JobRepository<'db> { pool: &'db SqlitePool }

impl<'db> JobRepository<'db> {
    pub fn new(pool: &'db SqlitePool) -> Self { Self { pool } }

    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>> {
        let mut query = "SELECT * FROM jobs WHERE 1=1".to_string();
        // ... filter construction
        sqlx::query_as::<_, Job>(&query).fetch_all(self.pool).await.map_err(Into::into)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Job>> {
        sqlx::query_as!(Job, "SELECT * FROM jobs WHERE id = ?", id)
            .fetch_optional(self.pool).await.map_err(Into::into)
    }

    pub async fn insert(&self, job: &Job) -> Result<()> {
        sqlx::query!(
            "INSERT INTO jobs (id, title, company_name, ...) VALUES (?, ?, ...)",
            job.id, job.title, job.company_name, ...
        ).execute(self.pool).await?;
        Ok(())
    }

    pub async fn update(&self, job: &Job) -> Result<()> { ... }
    pub async fn delete(&self, id: &str) -> Result<()> { ... }

    pub async fn count_new_matches_since(&self, since: DateTime<Utc>) -> Result<i64> {
        sqlx::query_scalar!("SELECT COUNT(*) FROM jobs WHERE discovered_at > ?", since)
            .fetch_one(self.pool).await.map_err(Into::into)
    }
}
```

### Schema: Core Tables

See `specs/04-sqlite-persistence.md` for full DDL. Key tables:

```sql
-- jobs table (note: source_quality added for dedup engine)
CREATE TABLE jobs (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    title TEXT NOT NULL,
    company_name TEXT NOT NULL,
    company_id TEXT REFERENCES companies(id),
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
    source_id TEXT,
    source_quality TEXT DEFAULT 'api' CHECK(source_quality IN ('api', 'aggregated', 'scraped')),
    ghost_score REAL,
    match_score REAL,
    discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_company ON jobs(company_id);
CREATE INDEX idx_jobs_discovered_at ON jobs(discovered_at);
CREATE INDEX idx_jobs_source_quality ON jobs(source_quality);
```

### Ralph Subprocess Database Access

Ralph subprocesses open their own short-lived connection to the same SQLite file:

```rust
// Ralph subprocess database access
// In lazyjob-ralph/src/persistence.rs (or in lazyjob-core for Ralph's use)

pub async fn open Ralph_database(data_dir: &Path) -> Result<SqlitePool> {
    let db_path = data_dir.join("lazyjob.db");
    SqlitePool::connect(&format!(
        "sqlite:{}?mode=rwc&journal_mode=WAL&busy_timeout=2000",
        db_path.display()
    ))
    .await
}
```

**Key settings for Ralph subprocesses**:
- `busy_timeout=2000` (2 seconds, shorter than TUI's 10 seconds — Ralph should fail fast)
- WAL mode ensures reads are not blocked by TUI writes
- Ralph subprocesses should hold connections briefly (query → process → close)

### Migrations

```
lazyjob-core/src/persistence/migrations/
├── 001_initial_schema.sql       # Jobs, applications, contacts, interviews, offers, reminders
├── 002_life_sheet.sql           # LifeSheet tables: personal_info, work_experience, education, skills
├── 003_companies.sql            # Company table
├── 004_jobs_source_quality.sql  # Add source_quality field to jobs table
├── 005_profile_contacts.sql     # profile_contacts table for networking
├── 006_referral_asks.sql        # referral_asks table for networking
├── 007_token_usage_log.sql     # token_usage_log for LLM cost tracking
├── 008_duplicate_log.sql        # duplicate_log for dedup analytics
└── 009_offer_details.sql        # offer_details (excluded from SaaS sync)
```

### Auto-Backup on Startup

```rust
impl Database {
    pub async fn with_auto_backup(data_dir: &Path) -> Result<Self> {
        let db = Self::open(data_dir).await?;

        // Check if WAL file exists (indicates dirty shutdown or crash)
        let wal_path = data_dir.join("lazyjob.db-wal");
        if wal_path.exists() {
            let backup_path = db.backup_dir.join(
                chrono::Local::now().format("%Y-%m-%d_%H-%M-%S.db")
            );
            db.backup(&backup_path).await?;
        }

        Ok(db)
    }

    pub async fn backup(&self, dest: &Path) -> Result<()> {
        let src = SqlitePool::connect(&format!(
            "sqlite:{}?mode=ro",
            self.pool().as_str().replace("mode=rwc", "mode=ro")
        )).await?;
        // Use rusqlite backup API for hot backup
        // ... backup implementation
        Ok(())
    }
}
```

### Data Export (Always Decrypted)

```rust
impl Database {
    pub async fn export_all(&self, path: &Path) -> Result<ExportReport> {
        let export = Export {
            version: "1.0".to_string(),
            exported_at: Utc::now(),
            jobs: self.jobs.list().await?,
            applications: self.applications.list().await?,
            profile_contacts: self.profile_contacts.list().await?,
            life_sheet: self.life_sheet.get().await?,
            companies: self.companies.list().await?,
        };
        let json = serde_json::to_string_pretty(&export)?;
        tokio::fs::write(path, json).await?;
        Ok(ExportReport { path: path.to_path_buf(), record_counts: ..., exported_at: Utc::now() })
    }
}
```

### `token_usage_log` Table

For LLM cost tracking (used by SaaS billing and offline cost estimation):

```sql
CREATE TABLE token_usage_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    provider TEXT NOT NULL,         -- 'anthropic', 'openai', 'ollama'
    model TEXT NOT NULL,            -- 'claude-3-5-sonnet-20241022', etc.
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NULL,
    cost_microdollars INTEGER NOT NULL, -- calculated from model pricing
    loop_type TEXT,                 -- which ralph loop triggered this
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_token_usage_created ON token_usage_log(created_at);
CREATE INDEX idx_token_usage_loop ON token_usage_log(loop_type);
```

## Open Questions

- **sqlx offline mode**: The `#[derive(sqlx::FromRow)]` on domain structs requires `CARGO_BUILD_SQLX` or `sqlx.toml` for compile-time query checking without a live DB connection. The `01-architecture.md` spec notes `sqlx.toml` for offline mode. Need to confirm this is the right approach.
- **WAL vs truncate**: The spec-inventory notes WAL mode with TRUNCATE checkpoint on backup for simplicity. This is fine for MVP. Concurrent reader count is low enough that blocking is not a concern.
- **offer_details exclusion from SaaS sync**: The salary spec formally excludes `offer_details` from sync. The migration `009_offer_details.sql` should note this as a "never sync" table. The SaaS migration spec must list this in its exclusion list.

## Implementation Tasks

- [ ] Implement `Database::open()` in `lazyjob-core/src/persistence/database.rs` with SqlitePool, WAL mode, foreign keys, migrations
- [ ] Implement all repository traits: `JobRepository`, `ApplicationRepository`, `CompanyRepository`, `ProfileContactRepository`, `InterviewRepository`, `OfferRepository`, `LifeSheetRepository`
- [ ] Create `lazyjob-core/src/persistence/migrations/001_initial_schema.sql` with all DDL from spec
- [ ] Add `source_quality TEXT DEFAULT 'api'` to `jobs` table in migration `004_jobs_source_quality.sql`
- [ ] Add `token_usage_log` table in migration `007_token_usage_log.sql`
- [ ] Add `duplicate_log` table in migration `008_duplicate_log.sql`
- [ ] Add `offer_details` table (excluded from SaaS sync) in migration `009_offer_details.sql`
- [ ] Implement `Database::with_auto_backup()` with WAL dirty-shutdown detection and auto-backup
- [ ] Implement `Database::export_all()` for JSON data portability
- [ ] Write `cargo sqlx prepare --all` to `sqlx.toml` for offline query compilation
- [ ] Implement Ralph subprocess database helper (`open Ralph_database()`) with `busy_timeout=2000`
