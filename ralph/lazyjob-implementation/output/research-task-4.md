# Research: Task 4 — Repositories

## Domain Types → DB Column Mapping

### Job (jobs table)
- id: UUID (JobId newtype)
- title: TEXT NOT NULL
- company_id: UUID (nullable, FK → companies)
- company_name: TEXT (nullable, denormalized)
- location, url, description: TEXT (nullable)
- salary_min, salary_max: BIGINT (nullable)
- source, source_id: TEXT (nullable)
- match_score, ghost_score: DOUBLE PRECISION (nullable)
- discovered_at: TIMESTAMPTZ NOT NULL
- notes: TEXT (nullable)
- created_at, updated_at: TIMESTAMPTZ (DB-managed defaults)

### Application (applications table)
- id: UUID (ApplicationId)
- job_id: UUID NOT NULL (FK → jobs)
- stage: TEXT NOT NULL (stored as snake_case string, e.g. "phone_screen")
- submitted_at: TIMESTAMPTZ (nullable)
- updated_at: TIMESTAMPTZ NOT NULL
- resume_version, cover_letter_version, notes: TEXT (nullable)

### Company (companies table)
- id: UUID (CompanyId)
- name: TEXT NOT NULL
- website, industry, size: TEXT (nullable)
- tech_stack: TEXT[] NOT NULL DEFAULT '{}'
- culture_keywords: TEXT[] NOT NULL DEFAULT '{}'
- notes: TEXT (nullable)

### Contact (contacts table)
- id: UUID (ContactId)
- name: TEXT NOT NULL
- role, email, linkedin_url: TEXT (nullable)
- company_id: UUID (nullable, FK → companies)
- relationship, notes: TEXT (nullable)

## Key Technical Decisions

1. **Runtime queries over compile-time macros**: Task 3 established this pattern. Use `sqlx::query()` / `sqlx::query_as()` with runtime binding. Avoids requiring DATABASE_URL at build time.

2. **sqlx::Type on ID newtypes**: Add `#[derive(sqlx::Type)]` with `#[sqlx(transparent)]` to all ID newtypes via the `define_id!` macro. This lets sqlx directly bind/extract our newtype UUIDs.

3. **ApplicationStage string conversion**: Stage is stored as TEXT in PG. Need `as_str() -> &str` and `TryFrom<&str>` on ApplicationStage to convert between Rust enum and DB string representation.

4. **Row mapping**: Use `sqlx::query()` with manual `.get()` for rows containing ApplicationStage (needs string conversion). Use `sqlx::query_as::<_, DomainType>()` for types where all fields have sqlx::Type (Job, Company, Contact — once IDs have sqlx::Type).

5. **Pagination**: Simple `Pagination { limit: i64, offset: i64 }` struct with sensible defaults (limit=50, offset=0).

6. **TEXT[] handling**: sqlx postgres driver natively supports `Vec<String>` for TEXT[] columns.

## Dependencies
- sqlx already in workspace with `uuid`, `chrono`, `postgres` features
- No new crates needed
