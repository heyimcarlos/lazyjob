# Plan: Task 4 — Repositories

## Files to Create/Modify

### Modify
- `lazyjob-core/src/domain/ids.rs` — Add `sqlx::Type` (transparent) to `define_id!` macro
- `lazyjob-core/src/domain/application.rs` — Add `as_str()` and `TryFrom<&str>` for ApplicationStage
- `lazyjob-core/src/lib.rs` — Add `pub mod repositories`

### Create
- `lazyjob-core/src/repositories/mod.rs` — Pagination struct, re-exports
- `lazyjob-core/src/repositories/job.rs` — JobRepository with CRUD
- `lazyjob-core/src/repositories/application.rs` — ApplicationRepository with CRUD
- `lazyjob-core/src/repositories/company.rs` — CompanyRepository with CRUD
- `lazyjob-core/src/repositories/contact.rs` — ContactRepository with CRUD

## Types/Functions to Define

### Pagination (repositories/mod.rs)
- `Pagination { limit: i64, offset: i64 }` with `Default` (50, 0)

### JobRepository
- `new(pool: PgPool) -> Self`
- `insert(job: &Job) -> Result<()>`
- `find_by_id(id: &JobId) -> Result<Option<Job>>`
- `list(pagination: &Pagination) -> Result<Vec<Job>>`
- `update(job: &Job) -> Result<()>`
- `delete(id: &JobId) -> Result<()>`

### ApplicationRepository
- Same CRUD pattern; stage stored/retrieved as string with conversion

### CompanyRepository
- Same CRUD pattern; tech_stack/culture_keywords as Vec<String> ↔ TEXT[]

### ContactRepository
- Same CRUD pattern

## Tests

### Unit Tests
- ApplicationStage::as_str() round-trips for all 9 variants
- ApplicationStage::try_from invalid string returns error
- Pagination default values

### Integration Tests (skip when DATABASE_URL not set)
- Job CRUD: insert → find_by_id → update → list → delete
- Application CRUD with FK to job
- Company CRUD with TEXT[] fields
- Contact CRUD with FK to company
- find_by_id returns None for non-existent ID
- delete is idempotent (no error on missing ID)

## Migrations
- None needed (all tables exist from 001_initial_schema.sql)
