# Research: Task 7 — CLI Skeleton

## Existing State
- `lazyjob-cli/src/main.rs` has a basic `fn main()` printing version
- lazyjob-cli already depends on: lazyjob-core, lazyjob-tui, anyhow, tokio, tracing, tracing-subscriber
- clap is NOT yet in workspace dependencies — needs to be added

## Available APIs to Wire Up
- `lazyjob_core::db::Database::connect(url)` — connects to PG and runs migrations
- `lazyjob_core::db::DEFAULT_DATABASE_URL` — fallback connection string
- `lazyjob_core::repositories::{JobRepository, ApplicationRepository, CompanyRepository, ContactRepository}`
- `lazyjob_core::domain::{Job, Application, Company, Contact, JobId, ...}`
- `lazyjob_core::life_sheet::{import_from_yaml, parse_yaml, serialize_yaml, load_from_db}`
- `lazyjob_core::life_sheet::JsonResume` — via `sheet.to_json_resume()`

## CLI Subcommands Needed (per task description)
1. `jobs list` — prints jobs from DB
2. `jobs add --title --company --url` — inserts a job
3. `profile import --file` — calls life_sheet import
4. `profile export` — prints JSON Resume to stdout
5. `tui` — placeholder for TUI launch

## Dependencies
- `clap = { version = "4", features = ["derive"] }` — for derive-based CLI parsing
- Already have: tokio (full), anyhow, tracing, tracing-subscriber

## Key Decisions to Make
- Use `#[tokio::main]` async main since DB operations are async
- DATABASE_URL from env var with fallback to DEFAULT_DATABASE_URL
- Use `anyhow::Result` at the CLI boundary (convert CoreError via ? with anyhow)
- tracing-subscriber init for logging
- Jobs list: simple table format to stdout
- Profile export: JSON to stdout (serde_json::to_string_pretty)

## Testing Approach
- Learning test: clap derive parsing works for nested subcommands
- Unit tests: test CLI arg parsing with `try_parse_from`
- Integration tests with `std::process::Command` would require DATABASE_URL — skip those, test parsing only
