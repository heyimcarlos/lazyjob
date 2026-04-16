# Plan: Task 7 — CLI Skeleton

## Files to Modify
1. `Cargo.toml` — add clap to workspace dependencies
2. `lazyjob-cli/Cargo.toml` — add clap dependency
3. `lazyjob-cli/src/main.rs` — full rewrite with clap derive CLI

## Types/Functions to Define

### In `lazyjob-cli/src/main.rs`:
- `Cli` struct (clap derive Parser) with `command: Commands` and optional `--database-url`
- `Commands` enum: `Jobs(JobsCommand)`, `Profile(ProfileCommand)`, `Tui`
- `JobsCommand` enum: `List`, `Add { title, company, url }`
- `ProfileCommand` enum: `Import { file }`, `Export`
- `async fn run(cli: Cli) -> anyhow::Result<()>` — dispatches to handlers
- `async fn handle_jobs_list(db)` — queries and prints jobs
- `async fn handle_jobs_add(db, title, company, url)` — creates and inserts a Job
- `async fn handle_profile_import(db, file)` — calls import_from_yaml
- `async fn handle_profile_export(db)` — loads from DB, converts to JSON Resume, prints
- `fn handle_tui()` — prints placeholder message

## Tests to Write
- **Learning test**: `clap_derive_nested_subcommands` — proves clap derive parsing works with nested subcommands
- **Unit tests**: 
  - `parse_jobs_list` — verifies `lazyjob jobs list` parses correctly
  - `parse_jobs_add` — verifies `lazyjob jobs add --title X --company Y --url Z` parses
  - `parse_profile_import` — verifies `lazyjob profile import --file path`
  - `parse_profile_export` — verifies `lazyjob profile export`
  - `parse_tui` — verifies `lazyjob tui`
  - `parse_database_url_flag` — verifies `--database-url` global option
  - `cross_crate_version_accessible` — existing test, keep it

## Migrations
None needed.
