# Plan: Task 3 — postgres-migrations

## Files to Create/Modify

### New files:
1. `lazyjob-core/migrations/001_initial_schema.sql` — All DDL for initial tables + indexes
2. `lazyjob-core/src/db.rs` — Database struct, connect(), pool()

### Modified files:
1. `Cargo.toml` (workspace) — Add sqlx to workspace dependencies
2. `lazyjob-core/Cargo.toml` — Add sqlx dependency
3. `lazyjob-core/src/lib.rs` — Add `pub mod db;`
4. `lazyjob-core/src/error.rs` — Add `From<sqlx::Error>` for CoreError

## Types/Functions to Define

### `lazyjob-core/src/db.rs`
```rust
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(database_url: &str) -> crate::error::Result<Self>
    pub fn pool(&self) -> &PgPool
}
```

## Migration: 001_initial_schema.sql
Tables (all using UUID PKs, TIMESTAMPTZ timestamps):
- jobs
- companies
- applications (FK → jobs)
- application_transitions (FK → applications)
- contacts (FK → companies)
- interviews (FK → applications)
- offers (FK → applications)
- life_sheet_items
- token_usage_log
- ralph_loop_runs

## Tests

### Learning tests (in db.rs #[cfg(test)]):
- `sqlx_pg_pool_from_url_format` — verifies PgPoolOptions can be configured (no actual connection needed)
- `migration_sql_files_embedded` — verifies sqlx::migrate!() compiles and the migrator has expected migration count

### Unit tests:
- `database_default_url` — test that a sensible default URL constant exists

### Integration tests (gated behind DATABASE_URL env var):
- `connect_and_migrate` — connects to real PG, runs migration, verifies tables exist
- `tables_exist_after_migration` — queries pg_tables to confirm all expected tables are created

## Migrations
- `lazyjob-core/migrations/001_initial_schema.sql` — full initial schema
