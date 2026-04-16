# Research: Task 3 — postgres-migrations

## Objective
Create a `Database` struct wrapping `sqlx::PgPool`, implement `Database::connect(database_url)` with automatic migration, and create the initial PostgreSQL schema.

## Domain Types → PostgreSQL Schema Mapping

### IDs
All domain IDs (JobId, ApplicationId, etc.) wrap `uuid::Uuid`. PostgreSQL column type: `UUID`. Use `gen_random_uuid()` for defaults where needed, but since Rust generates them, we primarily store them.

### Timestamps
`chrono::DateTime<Utc>` → `TIMESTAMPTZ`

### Vec<String>
`Vec<String>` (e.g., Company.tech_stack, culture_keywords) → `TEXT[]` (PostgreSQL array)

### Tables Needed (from task description)
1. **jobs** — matches `Job` domain type
2. **applications** — matches `Application` domain type  
3. **application_transitions** — audit trail for stage changes (new table, no domain type yet)
4. **companies** — matches `Company` domain type
5. **contacts** — matches `Contact` domain type
6. **interviews** — matches `Interview` domain type
7. **offers** — matches `Offer` domain type
8. **life_sheet_items** — for life sheet data persistence (future task 6)
9. **token_usage_log** — for LLM cost tracking (future task 17)
10. **ralph_loop_runs** — for Ralph crash recovery (task 21, but schema here)

## sqlx Configuration
- Features needed: `runtime-tokio`, `postgres`, `uuid`, `chrono`, `migrate`
- Connection: `PgPool::connect(database_url)` 
- Migrations: `sqlx::migrate!()` macro points to `./migrations/` relative to crate root
- For lazyjob-core, migrations dir: `lazyjob-core/migrations/`

## Key Decisions
1. **UUID PKs** — not SERIAL. Domain types already use UUID, so PG schema should match.
2. **No sqlx compile-time checking** — we use `sqlx::query()` / `sqlx::query_as()` (runtime), not `query!()` macro (needs DATABASE_URL at compile time). This avoids requiring a running DB to build.
3. **Migration path** — `sqlx::migrate!()` embeds SQL files at compile time. Path is relative to the Cargo.toml of the crate.
4. **Test strategy** — Integration tests require a running PostgreSQL. Gate behind `#[cfg(feature = "integration")]` or check for `DATABASE_URL` env var. Unit tests for Database struct construction logic don't need DB.

## Dependencies
- `sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "migrate"] }`
- Already have: tokio, uuid, chrono, thiserror, anyhow, serde, serde_json
