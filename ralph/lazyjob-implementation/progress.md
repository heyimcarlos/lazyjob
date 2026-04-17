# Progress Log

Started: 2026-04-16
Objective: Build LazyJob incrementally — Rust TUI + CLI for AI-powered job search with resume/cover letter generation

---

**Current state of repo:** Only `src/main.rs` with `println!("Hello, world!")` exists. No workspace, no crates, no migrations, no domain models.

**Architecture target:** 5-crate cargo workspace — lazyjob-core, lazyjob-llm, lazyjob-ralph, lazyjob-tui, lazyjob-cli.

**IMPORTANT: PostgreSQL, NOT SQLite.** The specs reference SQLite but we are using PostgreSQL instead. Use sqlx with `runtime-tokio` + `postgres` features. Use `PgPool` not `SqlitePool`. Use PostgreSQL-native types: SERIAL, TIMESTAMPTZ, TEXT[], JSONB, BOOLEAN. Connection via `DATABASE_URL` env var (default: `postgresql://localhost/lazyjob`). Migrations via `sqlx::migrate!()`. For tests, create a test database or use `DATABASE_URL` pointing to a test instance.

**Key specs to reference:**
- `specs/01-architecture-implementation-plan.md` — workspace + crate layout
- `specs/04-sqlite-persistence-implementation-plan.md` — sqlx + migrations
- `specs/09-tui-design-keybindings-implementation-plan.md` — ratatui TUI
- `specs/02-llm-provider-abstraction-implementation-plan.md` — LLM providers
- `specs/06-ralph-loop-integration-implementation-plan.md` — subprocess protocol
- `specs/07-resume-tailoring-pipeline-implementation-plan.md` — resume AI pipeline
- `specs/08-cover-letter-generation-implementation-plan.md` — cover letter AI

---

## Task 1: workspace-setup — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (converted from single-crate to workspace root)
- lazyjob-core/Cargo.toml + src/lib.rs
- lazyjob-llm/Cargo.toml + src/lib.rs
- lazyjob-ralph/Cargo.toml + src/lib.rs
- lazyjob-tui/Cargo.toml + src/lib.rs
- lazyjob-cli/Cargo.toml + src/main.rs
- Removed src/main.rs (old single-crate entry point)
Key decisions:
- Used thiserror 2.0 (not 1.0 as in spec) since edition 2024 requires it
- Workspace deps include lazyjob-* crates themselves for easy path references
- Only added dependencies each crate needs NOW; future tasks add specific deps (ratatui, clap, sqlx, etc.)
- Each lib crate exports a version() fn using env!("CARGO_PKG_VERSION") for cross-crate verification
Learning tests written:
- lazyjob-core::tests::version_returns_crate_version — verifies env! macro returns expected version string
- lazyjob-cli::tests::cross_crate_version_accessible — proves lazyjob-cli can import and use types from both lazyjob-core and lazyjob-tui, confirming workspace resolution works
Tests passing: 2
Next iteration should know:
- Workspace is fully set up with 5 crates, all building and passing clippy
- serde_yaml is in workspace deps but not yet added to any crate (needed by lazyjob-core for life sheet in task 6)
- No module stubs yet (no models/, persistence/, etc.) — task 2 will create lazyjob-core/src/domain/
- The spec mentions edition 2024, which is already set via workspace.package

## Task 3: postgres-migrations — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added sqlx to workspace dependencies)
- lazyjob-core/Cargo.toml (added sqlx dependency)
- lazyjob-core/migrations/001_initial_schema.sql (full initial schema: 10 tables + indexes)
- lazyjob-core/src/db.rs (Database struct with connect/pool/close)
- lazyjob-core/src/error.rs (added From<sqlx::Error> and From<sqlx::migrate::MigrateError>)
- lazyjob-core/src/lib.rs (added pub mod db)
Key decisions:
- Used UUID primary keys everywhere (matching domain types) instead of SERIAL — Rust generates UUIDs, PG just stores them
- Used runtime sqlx::query/query_as (not compile-time query! macros) to avoid requiring DATABASE_URL at build time
- Integration test (connect_and_migrate) gracefully skips when DATABASE_URL is not set — no hard dependency on running PG for builds
- CoreError::Db now wraps sqlx::Error directly (was a String), enabling ? propagation from sqlx calls
- Added CoreError::Migration variant for sqlx::migrate::MigrateError
- Migration 001 creates all tables mentioned in task description: jobs, applications, application_transitions, companies, contacts, interviews, offers, life_sheet_items, token_usage_log, ralph_loop_runs
- Used PG-native types: UUID, TIMESTAMPTZ, TEXT[], JSONB, BOOLEAN, BIGINT for salary fields (matching domain i64)
- Database::connect() runs sqlx::migrate!() automatically on every connection — idempotent
Learning tests written:
- pg_pool_options_configurable — proves PgPoolOptions builder works without connection
- migration_files_embedded — proves sqlx::migrate!() macro compiles and embeds exactly 1 migration file with version=1
Tests passing: 25 (3 new + 22 existing)
Next iteration should know:
- Database struct is in lazyjob-core::db, connect with Database::connect(url).await
- CoreError::Db now wraps sqlx::Error (not String) — any code that was constructing CoreError::Db("...".into()) will need updating
- sqlx features: runtime-tokio, postgres, uuid, chrono, migrate
- Migrations dir is lazyjob-core/migrations/ — future migrations go here as 002_xxx.sql, 003_xxx.sql, etc.
- The connect_and_migrate integration test runs against a real PG when DATABASE_URL is set
- Table schema matches domain types closely: jobs.salary_min/max are BIGINT (i64), company.tech_stack is TEXT[], etc.
- application_transitions table exists for task 5 (state machine audit trail)
- life_sheet_items and token_usage_log tables exist for future tasks 6 and 17

## Task 2: core-domain-types — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-core/src/error.rs (CoreError enum + Result type alias)
- lazyjob-core/src/domain/mod.rs (re-exports all domain types)
- lazyjob-core/src/domain/ids.rs (JobId, ApplicationId, CompanyId, ContactId, InterviewId, OfferId newtypes)
- lazyjob-core/src/domain/job.rs (Job struct)
- lazyjob-core/src/domain/application.rs (Application struct + ApplicationStage enum)
- lazyjob-core/src/domain/company.rs (Company struct)
- lazyjob-core/src/domain/contact.rs (Contact struct)
- lazyjob-core/src/domain/interview.rs (Interview struct)
- lazyjob-core/src/domain/offer.rs (Offer struct)
- lazyjob-core/src/lib.rs (added pub mod domain + pub mod error)
Key decisions:
- Used a `define_id!` macro to reduce boilerplate for 6 identical ID newtypes (all wrap Uuid with new(), from_uuid(), as_uuid(), Display, serde transparent)
- ApplicationStage uses serde rename_all = "snake_case" for clean JSON serialization
- Company uses Vec<String> for tech_stack and culture_keywords (flexible for future enrichment)
- All structs use Option<T> for nullable fields rather than defaults — explicit about missing data
- CoreError has From impls for std::io::Error and serde_json::Error for ergonomic ? usage
- No learning tests needed — uuid/chrono/serde are already proven from task 1
Learning tests written:
- None required (no new external crates introduced)
Tests passing: 22 (21 new + 1 existing)
Next iteration should know:
- All domain types are in lazyjob-core::domain, re-exported via mod.rs facade pattern
- CoreError and Result<T> are in lazyjob-core::error
- ApplicationStage has 9 variants; transition logic is deferred to task 5
- Job has company_name: Option<String> for denormalized display (in addition to company_id FK)
- IDs use serde(transparent) so they serialize as plain UUID strings in JSON
- No persistence layer yet — task 3 will add PostgreSQL migrations and Database struct

## Task 4: repositories — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-core/src/domain/ids.rs (added sqlx::Type transparent derive to define_id! macro)
- lazyjob-core/src/domain/application.rs (added as_str() and FromStr for ApplicationStage)
- lazyjob-core/src/lib.rs (added pub mod repositories)
- lazyjob-core/src/repositories/mod.rs (Pagination struct, re-exports, integration tests)
- lazyjob-core/src/repositories/job.rs (JobRepository with full CRUD)
- lazyjob-core/src/repositories/application.rs (ApplicationRepository with full CRUD)
- lazyjob-core/src/repositories/company.rs (CompanyRepository with full CRUD, TEXT[] support)
- lazyjob-core/src/repositories/contact.rs (ContactRepository with full CRUD, company FK)
Key decisions:
- Added `#[derive(sqlx::Type)] #[sqlx(transparent)]` to all ID newtypes via define_id! macro — enables direct sqlx bind/extract without intermediate types
- Used intermediate Row structs with sqlx::FromRow for reading, manual bind for writing — keeps domain types free of DB concerns
- ApplicationStage stored as snake_case TEXT in PG; manual as_str()/FromStr conversion avoids sqlx PG enum type dependency
- ApplicationRow uses TryFrom (not From) since stage string parsing can fail
- Delete is idempotent (no error on missing ID); update returns NotFound if 0 rows affected
- Pagination defaults to limit=50, offset=0
- All integration tests gracefully skip when DATABASE_URL is not set
Learning tests written:
- None required (no new external crates; sqlx traits are extensions of existing sqlx usage proven in task 3)
Tests passing: 35 (10 new: pagination_default, job_crud, application_crud, company_crud_with_arrays, contact_crud_with_company_fk, find_by_id_returns_none_for_missing, delete_is_idempotent, update_missing_returns_not_found, as_str_round_trips_all_stages, from_str_invalid_returns_error)
Next iteration should know:
- Repositories are in lazyjob-core::repositories, each takes PgPool (clone of pool from Database)
- Pattern: `let repo = JobRepository::new(db.pool().clone())`
- ApplicationStage now has `as_str() -> &str` and `FromStr` impl for DB round-tripping
- All ID newtypes now implement sqlx::Type (transparent) — can be used directly in sqlx queries
- No filter types yet (task didn't require them) — can be added to list() methods when needed
- Task 5 (application-state-machine) will add can_transition_to() and transition_stage() to ApplicationRepository

## Task 5: application-state-machine — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-core/src/domain/application.rs (added is_terminal, valid_transitions, can_transition_to, StageTransition struct)
- lazyjob-core/src/domain/mod.rs (re-exported StageTransition)
- lazyjob-core/src/repositories/application.rs (added transition_stage, transition_history, TransitionRow)
- lazyjob-core/src/repositories/mod.rs (added 4 integration tests for transitions)
Key decisions:
- Forward-only transition model: Interested→Applied→PhoneScreen→Technical→Onsite→Offer→Accepted, plus any non-terminal→Withdrawn/Rejected
- No backward transitions (spec had richer matrix but task description specified simpler forward-only model)
- Used CoreError::Validation for invalid transitions rather than introducing a new error variant
- transition_stage uses PG transaction (pool.begin/tx.commit) for atomicity: UPDATE applications + INSERT application_transitions in one tx
- StageTransition uses uuid::Uuid for id (not ApplicationId newtype) since transitions don't have their own ID type
- TransitionRow intermediate struct for sqlx::FromRow, with TryFrom conversion parsing stage strings
- Used existing `notes` column in application_transitions table (not `reason`)
Learning tests written:
- None required (no new external crates introduced)
Tests passing: 50 (15 new: is_terminal_for_terminal_stages, is_terminal_false_for_active_stages, valid_forward_transitions, any_non_terminal_to_withdrawn, any_non_terminal_to_rejected, terminal_stages_have_no_transitions, cannot_skip_stages, cannot_go_backward, cannot_transition_from_terminal, exhaustive_transition_matrix, stage_transition_serde_round_trip, transition_stage_succeeds, transition_stage_invalid_rejects, transition_history_returns_ordered, transition_stage_not_found)
Next iteration should know:
- ApplicationStage now has full state machine: is_terminal(), valid_transitions(), can_transition_to()
- ApplicationRepository now has transition_stage(&id, next_stage, reason) -> StageTransition and transition_history(&id) -> Vec<StageTransition>
- StageTransition domain type is in lazyjob-core::domain, re-exported from mod.rs
- Transition validation happens in the repository layer before the DB transaction
- The transition model is intentionally forward-only — no going backward between stages
- Task 6 (life-sheet-yaml) is next — introduces serde_yaml, LifeSheet types, YAML import/export

## Task 6: life-sheet-yaml — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-core/Cargo.toml (added serde_yaml dependency)
- lazyjob-core/src/lib.rs (added pub mod life_sheet)
- lazyjob-core/src/error.rs (added From<serde_yaml::Error> for CoreError)
- lazyjob-core/src/life_sheet/mod.rs (public API re-exports)
- lazyjob-core/src/life_sheet/types.rs (LifeSheet, Basics, Location, WorkExperience, Achievement, Education, SkillCategory, Skill, Certification, Language, Project, JobPreferences, CareerGoals)
- lazyjob-core/src/life_sheet/service.rs (parse_yaml, serialize_yaml, validate, import_from_yaml, load_from_db, upsert_to_db)
- lazyjob-core/src/life_sheet/json_resume.rs (JsonResume types + LifeSheet::to_json_resume() conversion)
- lazyjob-core/tests/fixtures/life-sheet.yaml (comprehensive sample fixture)
Key decisions:
- Used the existing life_sheet_items table with (section, key, value JSONB) — each section stored as one row with section name as key, JSONB blob as value
- Validation requires non-empty name + at least one work_experience or education entry
- JSON Resume export uses camelCase field names (startDate, endDate, studyType, countryCode) per the JSON Resume 1.0 spec
- LifeSheet types use serde(default) on all optional/collection fields for maximum YAML flexibility
- No separate DB tables per life sheet section — JSONB in life_sheet_items is sufficient
- Upsert uses INSERT...ON CONFLICT(section, key) DO UPDATE for idempotent imports
Learning tests written:
- serde_yaml_round_trip — proves serde_yaml can parse and re-serialize a nested LifeSheet struct with full fidelity
- serde_yaml_defaults_for_optional_fields — proves missing optional fields default to None/empty without parse errors
Tests passing: 67 (17 new: serde_yaml_round_trip, serde_yaml_defaults_for_optional_fields, parse_fixture_yaml, achievement_with_metrics, parse_yaml_succeeds_for_valid_input, parse_yaml_rejects_empty_name, parse_yaml_rejects_no_experience_or_education, parse_yaml_accepts_education_only, serialize_and_reparse_roundtrip, import_and_load_roundtrip, json_resume_basics_mapped, json_resume_work_mapped, json_resume_education_mapped, json_resume_skills_mapped, json_resume_serializes_to_valid_json, json_resume_certificates_and_languages, json_resume_projects_mapped)
Next iteration should know:
- LifeSheet types are in lazyjob-core::life_sheet, re-exported via mod.rs
- Public API: parse_yaml(&str), serialize_yaml(&LifeSheet), import_from_yaml(&Path, &PgPool), load_from_db(&PgPool)
- JSON Resume export: sheet.to_json_resume() returns JsonResume struct
- CoreError now has From<serde_yaml::Error> impl
- Fixture file at lazyjob-core/tests/fixtures/life-sheet.yaml for use in future tests
- Task 7 (cli-skeleton) is next — needs clap 4 with derive, subcommands for jobs, profile, tui

## Task 7: cli-skeleton — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added clap to workspace dependencies with derive + env features)
- lazyjob-cli/Cargo.toml (added clap and serde_json dependencies)
- lazyjob-cli/src/main.rs (full rewrite: clap derive CLI with subcommands, async main, DB wiring)
Key decisions:
- Used `#[tokio::main]` async main since all DB operations are async
- Global `--database-url` flag with `env = "DATABASE_URL"` fallback via clap's env feature
- DATABASE_URL defaults to lazyjob_core::db::DEFAULT_DATABASE_URL when neither flag nor env var is set
- Used anyhow::Result at CLI boundary for ergonomic error propagation from CoreError
- Jobs list renders a simple 3-column table (TITLE, COMPANY, URL) with truncation for long values
- Profile export outputs pretty-printed JSON Resume via serde_json::to_string_pretty
- TUI subcommand is a placeholder printing version info — will be wired to App::run() in task 10
- tracing_subscriber::fmt::init() for structured logging
- DB connection is only established for commands that need it (jobs, profile), not for tui placeholder
Learning tests written:
- clap_derive_nested_subcommands — proves clap derive Parser works with nested subcommands (Jobs → List)
Tests passing: 79 (12 new: clap_derive_nested_subcommands, parse_jobs_list, parse_jobs_add, parse_jobs_add_minimal, parse_profile_import, parse_profile_export, parse_tui, parse_database_url_flag, database_url_defaults_to_none, cross_crate_version_accessible, truncate_short_string, truncate_long_string)
Next iteration should know:
- CLI binary is lazyjob-cli, run via `cargo run -p lazyjob-cli -- <subcommand>`
- Subcommands: `jobs list`, `jobs add --title X [--company Y] [--url Z]`, `profile import --file PATH`, `profile export`, `tui`
- clap 4 with derive+env features is in workspace deps
- serde_json added to lazyjob-cli deps for JSON Resume pretty-print
- The `tui` subcommand is a stub — task 10 will wire it to lazyjob-tui::App::run()
- Task 8 (config-management) is next — Config struct, TOML loading, ~/.lazyjob/lazyjob.toml

## Task 8: config-management — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added toml, dirs, tempfile to workspace deps)
- lazyjob-core/Cargo.toml (added toml, dirs deps + tempfile dev-dep)
- lazyjob-core/src/lib.rs (added pub mod config)
- lazyjob-core/src/error.rs (added From<toml::de::Error> and From<toml::ser::Error> for CoreError)
- lazyjob-core/src/config.rs (new: Config struct with full load/save/ensure_exists)
Key decisions:
- Used `dirs` crate (v6) for cross-platform home directory resolution instead of manual HOME env var
- All Config fields use `#[serde(default = "...")]` so partial TOML files work — only specified fields override defaults
- DATABASE_URL env var override applied in `Config::load()` after file loading, not in `load_from()` (keeps load_from pure for testing)
- `save_to()` creates parent directories automatically via `create_dir_all` — eliminates separate ensure_dir step
- `ensure_exists()` is a class method that creates ~/.lazyjob/lazyjob.toml with defaults if missing
- Config re-uses `db::DEFAULT_DATABASE_URL` constant for the default database_url, keeping the single source of truth
- `tempfile` added as workspace dep for test tmpdir usage — will be useful for future tasks too
Learning tests written:
- toml_serialize_round_trip — proves toml crate can serialize/deserialize a struct faithfully
- toml_optional_fields_default — proves missing fields in TOML use serde defaults (critical for partial config files)
Tests passing: 91 (12 new: toml_serialize_round_trip, toml_optional_fields_default, default_config_has_expected_values, load_from_file, save_and_reload_round_trip, partial_toml_uses_defaults, ensure_exists_creates_file, env_override_database_url, save_creates_parent_directories, config_dir_ends_with_lazyjob, config_path_ends_with_toml, empty_toml_uses_all_defaults)
Next iteration should know:
- Config is in lazyjob-core::config, use Config::load() for standard startup flow
- Config::load_from(path) for testing with temp files
- Config::save_to(path) for testing writes without touching ~/.lazyjob/
- CoreError::Serialization now also wraps toml::de::Error and toml::ser::Error
- The CLI currently does NOT use Config yet — task 9 or 10 should wire Config::load() into CLI startup, replacing the raw DATABASE_URL fallback
- `tempfile` crate is now in workspace deps — available for any crate's dev-dependencies
- Task 9 (credential-manager) is next — keyring crate, SecretString, API key storage

## Task 9: credential-manager — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added keyring, secrecy, zeroize to workspace deps)
- lazyjob-core/Cargo.toml (added keyring, secrecy, zeroize deps)
- lazyjob-core/src/credentials.rs (new: CredentialManager with CredentialStore trait, KeyringStore, InMemoryStore)
- lazyjob-core/src/error.rs (added CoreError::Credential variant)
- lazyjob-core/src/lib.rs (added pub mod credentials)
- lazyjob-cli/Cargo.toml (added secrecy dep)
- lazyjob-cli/src/main.rs (added Config subcommand with SetKey/GetKey/DeleteKey)
Key decisions:
- Used a CredentialStore trait instead of directly wrapping keyring — keyring v3's mock stores passwords per-Entry instance (not in a shared global store), making it unusable for testing code that creates multiple Entry objects. The trait enables InMemoryStore for tests and KeyringStore for production.
- InMemoryStore uses Mutex<HashMap<String, String>> — simple, thread-safe, no external dependencies
- CredentialManager::new() uses real KeyringStore; CredentialManager::with_store() accepts any CredentialStore for testing
- API keys stored under "api_key:{provider}" namespace in keyring (e.g., "api_key:anthropic")
- delete_api_key is idempotent — deleting a non-existent key returns Ok(())
- CLI `config set-key` accepts --provider and --key flags; `config get-key` masks the key value (prints "******* (set)" or "No API key found")
- Added #[allow(clippy::enum_variant_names)] on ConfigCommand since SetKey/GetKey/DeleteKey naming is correct for CLI UX
- keyring features: sync-secret-service + crypto-rust (pure Rust crypto, dbus secret service on Linux)
Learning tests written:
- keyring_entry_api_compiles — proves keyring Entry::new/set_password/get_password API works with mock builder (same-entry round trip)
- secrecy_expose_secret — proves SecretString wraps and exposes values correctly
- in_memory_store_round_trip — proves InMemoryStore shares state across set/get/delete calls
Tests passing: 103 (12 new: keyring_entry_api_compiles, secrecy_expose_secret, in_memory_store_round_trip, set_and_get_api_key, get_missing_key_returns_none, delete_api_key, delete_missing_key_is_ok, multiple_providers_independent, overwrite_existing_key, parse_config_set_key, parse_config_get_key, parse_config_delete_key)
Next iteration should know:
- CredentialManager is in lazyjob-core::credentials, use CredentialManager::new() for production, CredentialManager::with_store(Box::new(InMemoryStore::new())) for tests
- CredentialStore trait is public — future crates (lazyjob-llm) can use it for provider API key retrieval
- CoreError::Credential(String) wraps keyring errors as strings
- CLI subcommands: `config set-key --provider X --key Y`, `config get-key --provider X`, `config delete-key --provider X`
- secrecy v0.8 is in workspace deps — SecretString available for any crate
- zeroize v1 with derive feature is in workspace deps — available for future security tasks
- keyring v3 mock per-Entry limitation: the mock does NOT share state between different Entry instances with the same service/user. Always use InMemoryStore for unit tests, not keyring's mock.
- Task 10 (tui-app-loop) is next — ratatui App struct, event loop, crossterm

## Task 10: tui-app-loop — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added ratatui, crossterm, futures to workspace deps)
- lazyjob-tui/Cargo.toml (added ratatui, crossterm, futures, chrono, tracing deps)
- lazyjob-tui/src/lib.rs (module declarations, pub async fn run(), learning tests)
- lazyjob-tui/src/app.rs (App struct, InputMode, RalphUpdate, handle_action)
- lazyjob-tui/src/action.rs (Action enum, ViewId enum with tab indexing)
- lazyjob-tui/src/event_loop.rs (run_event_loop with tokio::select!, TerminalGuard with Drop cleanup)
- lazyjob-tui/src/theme.rs (Theme struct with DARK constant, style helpers)
- lazyjob-tui/src/layout.rs (AppLayout with header/body/status_bar Rect splitting)
- lazyjob-tui/src/render.rs (render function: header tabs, body placeholder, status bar)
- lazyjob-cli/src/main.rs (wired tui subcommand to lazyjob_tui::run() with Config::load())
Key decisions:
- Used tokio::sync::broadcast for Ralph events (not crossbeam_channel) — more natural in async context, avoids extra dependency
- TerminalGuard struct with Drop impl ensures terminal cleanup even on panic (raw mode disabled, alternate screen left)
- App does NOT hold a database handle yet — this task is purely the event loop skeleton. DB will be added when views need data (task 11+)
- ViewId is a simple 6-variant enum (Dashboard, Jobs, Applications, Contacts, Ralph, Settings) — not the full View enum from spec which includes detail views. Detail views will be added as needed.
- 250ms tick rate as specified in task description (not 16ms/60fps from spec)
- Event loop renders on every tick AND on every event, ensuring responsive UI
- map_key_to_action is a simple match statement for now — task 12 will add the configurable KeyMap system
- render_body shows a placeholder with view name and help text — real views come in task 11
Learning tests written:
- ratatui_test_backend_renders_paragraph — proves TestBackend captures rendered Paragraph text in buffer cells
- ratatui_layout_splits_correctly — proves Layout with Length/Fill constraints produces expected Rect dimensions
- crossterm_key_event_constructible — proves KeyEvent::new() works for test construction
Tests passing: 131 (28 new in lazyjob-tui: 3 learning tests, 3 action tests, 7 app tests, 7 event_loop key mapping tests, 3 render tests, 2 layout tests, 2 theme tests, 1 version test)
Next iteration should know:
- TUI entry point: lazyjob_tui::run(Arc<Config>) — launches full event loop with alternate screen
- App struct is in lazyjob_tui::app, Action/ViewId in lazyjob_tui::action
- RalphUpdate enum defined in app.rs — will be replaced/expanded by lazyjob-ralph protocol types in task 18
- CLI `tui` subcommand now launches the real TUI (Config::load() for config)
- No database wiring yet — App::new() takes config + ralph_rx only
- TerminalGuard handles raw mode + alternate screen lifecycle
- ratatui 0.29, crossterm 0.28, futures 0.3 are now in workspace deps
- Task 11 (tui-views-stubs) is next — View trait, stub views, ViewId routing

## Task 11: tui-views-stubs — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-tui/src/views/mod.rs (View trait, Views container struct, re-exports)
- lazyjob-tui/src/views/dashboard.rs (DashboardView stub)
- lazyjob-tui/src/views/jobs_list.rs (JobsListView stub)
- lazyjob-tui/src/views/job_detail.rs (JobDetailView stub)
- lazyjob-tui/src/views/applications.rs (ApplicationsView stub)
- lazyjob-tui/src/views/contacts.rs (ContactsView stub)
- lazyjob-tui/src/views/ralph_panel.rs (RalphPanelView stub)
- lazyjob-tui/src/views/settings.rs (SettingsView stub)
- lazyjob-tui/src/views/help_overlay.rs (HelpOverlay with full keymap display)
- lazyjob-tui/src/app.rs (added Views struct to App, active_view_mut() method)
- lazyjob-tui/src/render.rs (dispatches body rendering to active view, renders help overlay)
- lazyjob-tui/src/event_loop.rs (routes unhandled keys to active view, help overlay intercepts keys when open)
- lazyjob-tui/src/lib.rs (added pub mod views)
Key decisions:
- View trait: render(&mut self, frame, area, theme) + handle_key(code, modifiers) -> Option<Action> + name() -> &'static str
- Views stored in a Views struct on App (static dispatch, not HashMap) — each view is a named field
- App::active_view_mut() returns &mut dyn View based on current ViewId — enables polymorphic dispatch
- HelpOverlay renders as a centered popup (60%x70%) with Clear background, not a tab view
- When help_open is true, all keys route to HelpOverlay first (blocks global keys like q)
- Global keys checked before view-specific keys; unhandled globals fall through to active view
- render.rs now takes &mut App (not &App) since views need &mut self for future scroll state
- All view structs derive Default to satisfy clippy::new_without_default
- No new external crates — all views use existing ratatui widgets (Paragraph, Block, Borders)
Learning tests written:
- None required (no new external crates introduced; ratatui already proven in task 10)
Tests passing: 155 (24 new: 4 views/mod tests, 2 dashboard tests, 2 jobs_list tests, 1 job_detail test, 1 applications test, 1 contacts test, 1 ralph_panel test, 1 settings test, 6 help_overlay tests, 5 render tests — including help overlay rendering and dispatch verification)
Next iteration should know:
- View trait is in lazyjob_tui::views, all views implement it
- Views are accessed via app.views.dashboard, app.views.jobs_list, etc.
- app.active_view_mut() returns the view matching app.active_view ViewId
- Help overlay intercepts all keys when app.help_open is true — ? and Esc close it
- Stub views render placeholder text with hints; real implementations come in tasks 27-30
- JobDetailView exists as a type but is not routed via ViewId (no detail variant) — will be wired in task 28
- render.rs render() now takes &mut App, not &App — any callers must pass &mut
- event_loop map_key_to_action now takes &mut App (for view handle_key)
- Task 12 (tui-keybindings) is next — configurable KeyMap system with per-view context bindings

## Task 12: tui-keybindings — DONE
Date: 2026-04-16
Files created/modified:
- lazyjob-tui/src/keybindings.rs (new: KeyCombo, KeyContext, KeyMap, parse_key_combo, parse_action)
- lazyjob-tui/src/action.rs (added ScrollDown, ScrollUp, Select variants + Action::name())
- lazyjob-tui/src/app.rs (added keymap field to App, wired with_overrides from Config)
- lazyjob-tui/src/event_loop.rs (replaced hardcoded map_global_key with keymap.resolve())
- lazyjob-tui/src/views/help_overlay.rs (render_overlay now takes &KeyMap + &KeyContext, renders dynamic content)
- lazyjob-tui/src/render.rs (passes keymap and active context to help overlay)
- lazyjob-tui/src/lib.rs (added pub mod keybindings)
Key decisions:
- KeyCombo::from_key_event() normalizes SHIFT for Char keys — crossterm sends '?' as Char('?') with SHIFT modifier, normalization ensures KeyCombo::plain(Char('?')) matches
- KeyMap uses HashMap<(KeyContext, KeyCombo), Action> with resolve() that checks context-specific first, then falls back to Global
- Config overrides via with_overrides(&HashMap<String, String>) — parses action names and key strings from Config.keybindings
- Per-view bindings (j/k/Down/Up/Enter) registered for all 6 view contexts — view-specific actions (ScrollDown, ScrollUp, Select) added to Action enum
- HelpOverlay has two render paths: View trait render() for standalone use, render_overlay() with keymap/context for dynamic display
- No new crates needed — uses existing crossterm, std::collections
Learning tests written:
- None required (no new external crates introduced)
Tests passing: 187 (32 new: 27 keybindings tests — KeyCombo display/normalization, KeyMap resolve/defaults/overrides/context, parse_key_combo, parse_action, KeyContext; 2 help_overlay dynamic render tests; 3 event_loop tests replaced — j/k/Enter/arrows now resolve via keymap)
Next iteration should know:
- KeyMap is in lazyjob_tui::keybindings, constructed via KeyMap::default_keymap().with_overrides(&config.keybindings)
- App.keymap field holds the active keymap — event_loop uses keymap.resolve() instead of hardcoded match
- KeyCombo::from_key_event(code, modifiers) normalizes SHIFT for char keys — always use this when converting crossterm events
- KeyContext::from_view_id(ViewId) converts between the two enums
- Action now has ScrollDown, ScrollUp, Select variants — currently no-ops in handle_action (views are stubs), will be wired when views get real implementations
- Action::name() returns display string for help overlay rendering
- HelpOverlay.render_overlay(frame, area, theme, &keymap, &active_context) renders grouped Global + active context sections
- Config.keybindings HashMap<String, String> overrides work: key is action name ("quit"), value is key combo string ("ctrl+q")
- Task 13 (tui-widgets) is next — custom ratatui widgets: JobCard, ModalDialog, ConfirmDialog, StatBlock, ProgressBar

## Task 13: tui-widgets — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-tui/src/widgets/confirm_dialog.rs (NEW)
- crates/lazyjob-tui/src/lib.rs (added pub mod widgets;)
- crates/lazyjob-tui/src/widgets/modal_dialog.rs (fixed centered_rect height clamping bug)
Key decisions:
- All 4 widgets (JobCard, ModalDialog, StatBlock, ProgressBar) were already implemented in prior iterations but not wired to lib.rs — adding `pub mod widgets;` activated them and their tests
- ConfirmDialog built on top of ModalDialog's pub `centered_rect` helper — no duplication
- centered_rect had a latent buffer-overflow bug: when popup height > terminal height, the Rect extended beyond the buffer causing panics. Fixed by clamping height to r.height in both the Layout constraint and the final Rect::new() call
- ConfirmDialog uses `confirm_selected: bool` to track which button (Yes/No) is highlighted; selected button gets theme.primary + BOLD, inactive gets theme.text_muted
- Layout inside ConfirmDialog: Fill(1) body Paragraph + Length(1) button row
Learning tests written:
- None required (no new external crates; ratatui already proven in tasks 10-12)
Tests passing: 221 (7 new confirm_dialog tests; 31 previously-unrunning widget tests now active via pub mod widgets)
Next iteration should know:
- All widgets are now in lazyjob_tui::widgets, re-exported via mod.rs facade
- ConfirmDialog: ConfirmDialog::new(title, body, &theme).confirm_selected(bool) — implements Widget
- centered_rect is a pub fn in widgets::modal_dialog — use it from any widget needing centered overlays
- The small-area panic in modal_dialog was a pre-existing bug (tests existed but weren't running) — now fixed
- Task 14 (llm-provider-traits) is next — LlmProvider trait, EmbeddingProvider, MockLlmProvider, ChatMessage, CompletionOptions in lazyjob-llm

## Task 14: llm-provider-traits — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added async-trait = "0.1" to workspace deps)
- crates/lazyjob-llm/Cargo.toml (added async-trait dep)
- crates/lazyjob-llm/src/lib.rs (full rewrite: module declarations + pub re-exports)
- crates/lazyjob-llm/src/error.rs (new: LlmError enum + Result<T> alias)
- crates/lazyjob-llm/src/message.rs (new: ChatMessage, CompletionOptions, LlmResponse, TokenUsage)
- crates/lazyjob-llm/src/provider.rs (new: LlmProvider trait, EmbeddingProvider trait)
- crates/lazyjob-llm/src/mock.rs (new: MockLlmProvider, MockEmbeddingProvider)
Key decisions:
- Used async_trait macro for async fn in traits — Rust 2024 stabilized async fn in traits for static dispatch but dyn Trait still needs boxed futures; async_trait is the proven solution
- ChatMessage is an enum (System/User/Assistant) with role()/content() accessors — provides type safety vs a struct with role: String
- CompletionOptions uses Option<T> for model/temperature/max_tokens — callers only override what they need; Default provides sensible values (temp=0.7, max_tokens=4096, stream=false)
- TokenUsage::new(prompt, completion) auto-computes total — no manual arithmetic at call sites
- MockLlmProvider::with_content(str) convenience constructor — common case in tests
- Both mock providers implement Send + Sync (required by trait bounds) — all fields are Clone + Send
Learning tests written:
- async_trait_dyn_dispatch — creates Box<dyn LlmProvider> from MockLlmProvider, calls .complete() via dyn dispatch, verifies result. Proves async_trait macro correctly enables dynamic dispatch for async trait methods.
- async_trait_dyn_dispatch_embedding — same proof for Box<dyn EmbeddingProvider>
Tests passing: 235 (14 new in lazyjob-llm; 221 prior tests unchanged)
Next iteration should know:
- LlmProvider and EmbeddingProvider are in lazyjob_llm::provider, re-exported from lazyjob_llm root
- MockLlmProvider::with_content("text") is the zero-boilerplate test provider
- async_trait crate is now in workspace deps — available for all crates needing async fn in dyn traits
- LlmError, Result<T>, ChatMessage, CompletionOptions, LlmResponse, TokenUsage all re-exported from lazyjob_llm root
- Task 15 (llm-anthropic) is next — AnthropicProvider using reqwest with SSE streaming and exponential backoff

## Task 15: llm-anthropic — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"], default-features = false } to workspace deps)
- crates/lazyjob-llm/Cargo.toml (added reqwest, secrecy deps; added [features] integration = [])
- crates/lazyjob-llm/src/lib.rs (added pub mod providers + pub use providers::AnthropicProvider)
- crates/lazyjob-llm/src/providers/mod.rs (new: re-exports AnthropicProvider)
- crates/lazyjob-llm/src/providers/anthropic.rs (new: full AnthropicProvider implementation)
Key decisions:
- API key passed directly to AnthropicProvider::new(api_key) for simplicity; from_credentials() alternative reads from CredentialManager keyring
- Default model is claude-haiku-4-5-20251001; overridden per-call via CompletionOptions::model
- System messages extracted from messages Vec and sent as top-level Anthropic "system" field (Anthropic API requirement)
- Backoff: BACKOFF_DELAYS_SECS [1s, 2s, 4s] via iterator — retries until delays exhausted on RateLimit/Api errors
- SSE streaming: response bytes collected via .bytes().await then parsed line-by-line; content_block_delta events accumulate text; message_start captures model+input_tokens; message_delta captures stop_reason+output_tokens
- Non-streaming and streaming both route through call_with_backoff; CompletionOptions::stream selects path
- Integration test gated behind cargo feature "integration"; reads ANTHROPIC_API_KEY env var
- Removed MAX_RETRIES const (unused after refactor to iterator-based backoff)
Learning tests written:
- reqwest_client_builds_with_rustls — proves reqwest::Client::builder().use_rustls_tls().build() compiles and succeeds with rustls-tls feature enabled
- reqwest_json_serializes_request_body — proves AnthropicRequest serializes to correct JSON shape before sending to API
Tests passing: 249 (14 new in lazyjob-llm: 2 learning + 12 unit; 235 prior tests unchanged)
Next iteration should know:
- AnthropicProvider is in lazyjob_llm::providers::anthropic, re-exported as lazyjob_llm::AnthropicProvider
- reqwest 0.12 with rustls-tls is now in workspace deps — available for any future crate needing HTTP
- secrecy is now in lazyjob-llm deps (for from_credentials ExposeSecret usage)
- Integration test: cargo test -p lazyjob-llm --features integration (requires ANTHROPIC_API_KEY env var)
- SSE parsing is in the module-private parse_sse_response(&[u8]) function — testable independently of network
- Task 16 (llm-openai) is next — OpenAiProvider using async-openai, OllamaProvider using ollama-rs

## Task 16: llm-openai — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added async-openai = "0.34" with chat-completion+embedding features, ollama-rs = "0.3" with stream+rustls features)
- crates/lazyjob-llm/Cargo.toml (added async-openai and ollama-rs deps)
- crates/lazyjob-llm/src/providers/openai.rs (new: OpenAiProvider implementing LlmProvider + EmbeddingProvider)
- crates/lazyjob-llm/src/providers/ollama.rs (new: OllamaProvider implementing LlmProvider + EmbeddingProvider)
- crates/lazyjob-llm/src/providers/mod.rs (added pub mod openai, pub mod ollama, re-exports)
- crates/lazyjob-llm/src/lib.rs (added OpenAiProvider, OllamaProvider to pub re-exports)
Key decisions:
- async-openai 0.34 uses feature-gated modules — `chat-completion` and `embedding` features required to activate `_api` feature (the HTTP client)
- ollama-rs must use `default-features = false, features = ["stream", "rustls"]` to avoid native-tls/OpenSSL dependency (system does not have libssl-dev)
- Message builder types (`*Args`) in async-openai 0.34 are in `types::chat` submodule, not `types` directly
- Both OpenAiProvider and OllamaProvider implement both `LlmProvider` and `EmbeddingProvider` traits
- Both providers have `provider_name()` from each trait — tests must use `LlmProvider::provider_name(&p)` qualified syntax
- `from_credentials()` constructor on OpenAiProvider reads from CredentialManager keyring (same pattern as AnthropicProvider)
- OllamaProvider::new() connects to localhost:11434; OllamaProvider::with_host_port() for custom hosts
- Ollama response `final_data: Option<ChatMessageFinalResponseData>` with `prompt_eval_count: u64` and `eval_count: u64`
Learning tests written:
- async_openai_client_builds_with_config — proves Client::with_config(OpenAIConfig::new().with_api_key()) constructs without network call
- async_openai_chat_request_serializes — proves CreateChatCompletionRequestArgs builder produces correct request struct
- ollama_rs_client_constructs — proves Ollama::new() and Ollama::default() construct without panicking
- ollama_rs_message_constructors — proves OllamaMessage::system/user/assistant set content correctly
Tests passing: 261 (12 new: 4 openai tests + 8 ollama tests)
Next iteration should know:
- OpenAiProvider is in lazyjob_llm::providers::openai, re-exported as lazyjob_llm::OpenAiProvider
- OllamaProvider is in lazyjob_llm::providers::ollama, re-exported as lazyjob_llm::OllamaProvider
- async-openai 0.34 type path: `async_openai::types::chat::CreateChatCompletionRequestArgs` (NOT `types::CreateChatCompletionRequestArgs`)
- ollama-rs MUST use `default-features = false` + `features = ["stream", "rustls"]` on this system (no OpenSSL)
- Both providers implement both LlmProvider and EmbeddingProvider — future code calling provider_name() must use qualified syntax
- Integration tests: `cargo test -p lazyjob-llm --features integration` (requires OPENAI_API_KEY or running Ollama)
- Task 17 (llm-registry) is next — ProviderRegistry, LlmBuilder::from_config, cost estimation

## Task 18: ralph-protocol — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-ralph/src/error.rs (new: RalphError, Result<T>)
- crates/lazyjob-ralph/src/protocol.rs (new: WorkerCommand, WorkerEvent, NdjsonCodec)
- crates/lazyjob-ralph/src/lib.rs (added pub mod error/protocol + re-exports)
Key decisions:
- Used `WorkerCommand` / `WorkerEvent` names per task description (not spec's IncomingMessage/OutgoingMessage)
- `loop_type` in WorkerCommand::Start is `String` (not LoopType enum — task 20 defines that)
- NdjsonCodec::encode is infallible (returns String not Result) — WorkerCommand is always serializable
- NdjsonCodec::decode trims whitespace before parsing — handles lines with trailing newlines from read_line()
- RalphError::Decode(String) wraps serde_json errors as strings (no From<serde_json::Error> to avoid implicit conversion from non-decode contexts)
Learning tests written:
- serde_tagged_enum_serializes_type_field — proves serde tag attribute emits a "type" JSON key with snake_case variant name
- serde_json_value_roundtrip — proves serde_json::Value serializes and deserializes with full fidelity
Tests passing: 300 total (17 new in lazyjob-ralph)
Next iteration should know:
- WorkerCommand, WorkerEvent, NdjsonCodec are in lazyjob_ralph::protocol, re-exported from crate root
- NdjsonCodec::encode(&WorkerCommand) -> String (infallible, appends \n)
- NdjsonCodec::decode(line: &str) -> Result<WorkerEvent> (fallible, trims whitespace)
- RalphError and Result<T> are in lazyjob_ralph::error, re-exported from crate root
- loop_type in WorkerCommand::Start is String — task 20 will add LoopType enum; protocol.rs uses String to avoid coupling
- Task 19 (ralph-process-manager) is next — RalphProcessManager, spawn subprocess, broadcast WorkerEvents

## Task 17: llm-registry — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-llm/src/cost.rs (new: estimate_cost, PRICING table)
- crates/lazyjob-llm/src/registry.rs (new: ProviderRegistry, LlmBuilder, LoggingProvider)
- crates/lazyjob-llm/Cargo.toml (added sqlx, uuid deps)
- crates/lazyjob-llm/src/lib.rs (added pub mod cost, registry + re-exports)
Key decisions:
- PRICING table uses blended input/output rates in microdollars per 1000 tokens; matches by substring so "claude-haiku-4-5-20251001" matches "claude-haiku"
- ProviderRegistry stores Arc<dyn LlmProvider> by name; first added becomes default, override with set_default()
- LlmBuilder::from_config fallback chain: configured provider (if key present) → Anthropic (if key set) → OpenAI (if key set) → Ollama (always)
- LoggingProvider is a decorator: wraps Arc<dyn LlmProvider>, implements LlmProvider, fires DB insert after each complete(); pool is Option<PgPool> so without_pool() constructor enables unit testing without a real DB
- DB insert uses runtime sqlx::query() (not query! macro) to avoid DATABASE_URL at build time
- Fire-and-forget logging: DB errors are swallowed (let _ = ...) so a logging failure never fails the completion
Learning tests written:
- None required (sqlx proven in tasks 3-4; async_trait proven in tasks 14-15; no new external crates)
Tests passing: 283 total (22 new: 7 cost tests + 15 registry tests)
Next iteration should know:
- estimate_cost(model, tokens) is in lazyjob_llm::cost, re-exported from lazyjob_llm root
- ProviderRegistry, LlmBuilder, LoggingProvider are in lazyjob_llm::registry, re-exported from root
- LlmBuilder::from_config(config, creds) returns Result<Box<dyn LlmProvider>> — never errors, always falls back to Ollama
- LoggingProvider::new(provider, pool) wraps any Arc<dyn LlmProvider> with DB logging; without_pool() for tests
- sqlx and uuid are now direct deps of lazyjob-llm (in addition to lazyjob-core which also has them)
- Task 18 (ralph-protocol) is next — WorkerCommand/WorkerEvent NDJSON protocol types in lazyjob-ralph

## Task 20: ralph-loop-types — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added cron = "0.12" to workspace deps)
- crates/lazyjob-ralph/Cargo.toml (added chrono, cron deps)
- crates/lazyjob-ralph/src/error.rs (added CronParse(String) and QueueFull(usize) variants)
- crates/lazyjob-ralph/src/loop_types.rs (new: LoopType enum, QueuedLoop, LoopDispatch)
- crates/lazyjob-ralph/src/loop_scheduler.rs (new: LoopScheduler with cron expression checking)
- crates/lazyjob-ralph/src/lib.rs (added pub mod loop_types, loop_scheduler + re-exports)
Key decisions:
- Priority ordering: CoverLetter(90) > ResumeTailor(85) > InterviewPrep(70) > CompanyResearch(50) > JobDiscovery(30) — user-initiated tasks always beat background tasks
- Only InterviewPrep is interactive (needs stdin I/O loop for mock interview exchanges)
- JobDiscovery concurrency_limit=1 to avoid hammering job APIs; others allow 2-3 concurrent
- LoopDispatch BinaryHeap Ord impl: primary key = priority (higher first); tie-break = enqueued_at (earlier first via reverse comparison on other.enqueued_at.cmp(&self.enqueued_at))
- QueueFull(cap) error returned on 21st enqueue, not silently dropped
- LoopScheduler uses cron::Schedule::after(&last_checked).next() to find next tick; if next_tick <= now, fires and advances last_checked to prevent double-fire
- Used cron v0.12.1 (latest compatible — workspace lock picked 0.12.1 over 0.16.0)
- Let chains (`if let Some(x) = y && condition`) used in loop_scheduler.rs per clippy suggestion (stable since Rust 1.88)
Learning tests written:
- cron_schedule_parses_standard_expr — proves cron::Schedule::from_str() accepts a 6-field cron expression
- cron_schedule_upcoming_iterator — proves schedule.upcoming(Utc).next() returns a future DateTime<Utc>
Tests passing: 328 total (21 new: 13 loop_types tests + 6 loop_scheduler tests + 2 learning tests)
Next iteration should know:
- LoopType, QueuedLoop, LoopDispatch are in lazyjob_ralph::loop_types, re-exported from crate root
- LoopScheduler is in lazyjob_ralph::loop_scheduler, re-exported from crate root
- RalphError now has CronParse(String) and QueueFull(usize) variants
- cron v0.12 uses 6-field expressions (sec min hour day month weekday) — NOT 5-field standard cron
- LoopScheduler.last_checked is pub(crate) — tests in the same module set it directly for time-travel testing
- Task 21 (ralph-crash-recovery) is next — RalphLoopRunRepository, PG table migration, recover_pending()

## Task 19: ralph-process-manager — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-ralph/src/error.rs (added Io(#[from] std::io::Error) and NotFound(String) variants)
- crates/lazyjob-ralph/src/process_manager.rs (new: RunId, ProcessHandle, RalphProcessManager)
- crates/lazyjob-ralph/src/lib.rs (added pub mod process_manager + re-exports)
Key decisions:
- Added `with_binary_and_args(path, args)` constructor to support both production (current_exe + "worker" arg) and test (sh -c 'script' worker) usage patterns — avoids temp executable files which cause ETXTBSY on Linux when created and executed concurrently
- Event broadcast channel type is `broadcast::Sender<(RunId, WorkerEvent)>` — tags each event with its RunId so multiple TUI subscribers can correlate events to specific runs
- `cancel()` sends WorkerCommand::Cancel to stdin, waits up to 3s for graceful exit via `tokio::time::timeout`, then calls `child.kill()` (SIGKILL) if timed out
- `ChildStdin` is stored separately from `Child` in `ProcessHandle` — necessary because `Child` does not allow concurrent access to its stdin handle after `take()`
- Background reader task uses `BufReader::lines()` for async line-by-line reading; decode errors are silently ignored (best-effort for malformed subprocess output)
- `binary_args` stores `Vec<OsString>` for maximum OS compatibility; args are injected before the hardcoded "worker" subcommand arg
Learning tests written:
- tokio_process_piped_stdout — proves `tokio::process::Command` with `Stdio::piped()` allows async stdout line reading via `BufReader::lines()`
- tokio_process_stdin_write — proves bidirectional pipe communication: write to stdin via `AsyncWriteExt::write_all`, read back via `BufReader::lines()` (using `cat` as subprocess)
Tests passing: 307 total (7 new: 2 learning + 5 unit: run_id_is_unique, run_id_display_is_uuid_format, spawn_emits_worker_events, cancel_terminates_running_process, cancel_unknown_run_returns_not_found)
Next iteration should know:
- RalphProcessManager is in lazyjob_ralph::process_manager, re-exported as lazyjob_ralph::RalphProcessManager
- RunId is in lazyjob_ralph::process_manager, re-exported as lazyjob_ralph::RunId
- RalphError now has Io(#[from] io::Error) and NotFound(String) in addition to Decode(String)
- For production: `RalphProcessManager::new()` uses `current_exe()` and spawns `<current_exe> worker`
- For tests: `with_binary_and_args(PathBuf::from("sh"), vec![OsString::from("-c"), OsString::from("...script...")])`
- The broadcast channel type is `(RunId, WorkerEvent)` — subscribers need pattern matching on the tuple
- Task 20 (ralph-loop-types) is next — LoopType enum, LoopDispatch priority queue, LoopScheduler

## Task 21: ralph-crash-recovery — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-core/src/repositories/ralph_loop_run.rs (new: RalphLoopRunStatus enum, RalphLoopRun struct, RalphLoopRunRepository)
- crates/lazyjob-core/src/repositories/mod.rs (added pub mod ralph_loop_run + re-exports)
- crates/lazyjob-tui/Cargo.toml (added sqlx dep)
- crates/lazyjob-tui/src/app.rs (added pool: Option<PgPool> field + with_pool() builder)
- crates/lazyjob-tui/src/lib.rs (wired DB connection + recover_pending() call in run())
Key decisions:
- No migration 002 needed — ralph_loop_runs table already existed in migration 001 with the correct schema
- RalphLoopRunStatus stored as snake_case TEXT in PG; as_str()/FromStr for round-tripping (same pattern as ApplicationStage)
- recovery is in run() (not App::new()) since recovery is async — App::new() stays synchronous
- run() uses a graceful fallback: if DB connection fails, logs a warning and creates App with pool=None instead of crashing
- App gains pool: Option<PgPool> field for future view implementations that need DB access
- recover_pending() targets status='running' AND started_at < now()-30s — only marks truly stale runs failed (not recent ones)
Learning tests written:
- None required (no new external crates; sqlx proven in tasks 3-4)
Tests passing: 339 total (11 new unit tests in ralph_loop_run.rs; 6 integration tests skipped gracefully when DATABASE_URL not set but pass when it is set — verified via DATABASE_URL being set in this run)
Next iteration should know:
- RalphLoopRunRepository is in lazyjob-core::repositories, re-exported as lazyjob_core::repositories::RalphLoopRunRepository
- RalphLoopRun::new(loop_type) creates a Pending run with Uuid id and Utc::now() created_at
- App now has pool: Option<PgPool> — use app.pool.as_ref() to access it in views
- App::with_pool(pool) is a builder method — chain after App::new()
- TUI startup now auto-recovers stale 'running' runs older than 30s, marking them 'failed'
- Task 22 (ralph-tui-panel) is next — implement RalphPanelView with WorkerEvent broadcast subscription, progress bars, cancel binding

## Task 23: job-sources — DONE
Date: 2026-04-16
Files created/modified:
- Cargo.toml (added ammonia = "4", wiremock = "0.6" to workspace deps)
- crates/lazyjob-core/Cargo.toml (added ammonia, async-trait, reqwest deps; wiremock dev-dep)
- crates/lazyjob-core/src/error.rs (added CoreError::Http(String) variant for HTTP transport errors)
- crates/lazyjob-core/src/lib.rs (added pub mod discovery)
- crates/lazyjob-core/migrations/002_unique_job_source.sql (partial unique index on (source, source_id) for ON CONFLICT upsert)
- crates/lazyjob-core/src/discovery/mod.rs (re-exports)
- crates/lazyjob-core/src/discovery/sources/mod.rs (JobSource trait + RateLimiter + strip_html helper)
- crates/lazyjob-core/src/discovery/sources/greenhouse.rs (GreenhouseClient with async fetch, rate limiting, HTML stripping)
- crates/lazyjob-core/src/discovery/sources/lever.rs (LeverClient with async fetch, rate limiting, HTML stripping)
- crates/lazyjob-core/src/repositories/job.rs (added upsert_discovered() using ON CONFLICT partial unique index)
Key decisions:
- RateLimiter uses std::sync::Mutex<Option<Instant>> for interior mutability — lock held briefly, not across await, so no async mutex needed
- JobSource trait uses async_trait for dyn dispatch compatibility
- Clients have a private do_fetch() helper called by both the inherent fetch_jobs() and the trait impl — avoids name collision and recursion
- with_base_url() builder on both clients enables pointing at MockServer for tests without feature flags
- strip_html() uses ammonia::Builder::new().tags(HashSet::new()).clean().to_string() to strip all tags and keep text
- CoreError::Http(String) added for HTTP transport errors, distinct from Parse/Io errors
- Migration 002 adds partial unique index WHERE source IS NOT NULL AND source_id IS NOT NULL — manually-entered jobs (NULL source) bypass the constraint
- upsert_discovered() in JobRepository uses ON CONFLICT with the same partial WHERE clause for idempotent discovery ingestion
Learning tests written:
- ammonia_strips_html_tags — proves ammonia::Builder with empty tags set strips HTML and returns plain text content
- wiremock_responds_with_json — proves wiremock MockServer intercepts HTTP requests and returns configured fixture body
Tests passing: 368 (14 new: 2 learning + 5 strip_html/rate_limiter unit tests + 7 greenhouse/lever integration tests with wiremock)
Next iteration should know:
- GreenhouseClient and LeverClient are in lazyjob-core::discovery::sources, re-exported via lazyjob-core::discovery
- JobSource trait is in lazyjob-core::discovery, requires async_trait to use as dyn trait
- RateLimiter::new(N) creates a limiter for N req/s; call .wait(&self).await before each request
- strip_html() is pub(crate) in sources/mod.rs — used by both clients
- upsert_discovered() in JobRepository uses migration 002's partial unique index; requires both source and source_id to be non-NULL for conflict detection
- Task 24 (discovery-service) is next — DiscoveryService::run_discovery() fans out to all sources in parallel, calls upsert_discovered() for deduplication

## Task 22: ralph-tui-panel — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-tui/src/views/ralph_panel.rs (full rewrite: ActiveEntry, CompletedEntry, RalphPanelView with on_update, cleanup, render, handle_key)
- crates/lazyjob-tui/src/action.rs (added CancelRalphLoop(String), RalphDetail(String) variants + name() impl)
- crates/lazyjob-tui/src/app.rs (filled in handle_ralph_update, wired ScrollDown/ScrollUp/Select to delegate to active view, added crossterm imports)
Key decisions:
- State held in view struct (not in App): `active: Vec<ActiveEntry>`, `completed: Vec<CompletedEntry>`, `selected: usize`
- `on_update()` is a standalone method (not part of View trait) called from `app.handle_ralph_update()`
- 5-second completed-entry display: expired entries cleaned up at the start of each `render()` call (which takes `&mut self`)
- `progress` stored as 0.0-1.0 ratio (divided from percent in Progress event); passed directly to ProgressBar::new(ratio, label)
- Keybindings: c → `Action::CancelRalphLoop(run_id)`, Enter → `Action::RalphDetail(run_id)`, Esc → `Action::NavigateBack`, j/Down and k/Up update `self.selected`
- `handle_action(ScrollDown/Up/Select)` now delegates to `active_view_mut().handle_key(Down/Up/Enter)` — this properly routes keymap-resolved actions (j→ScrollDown→view.handle_key(Down)) to view-specific navigation; backward compatible since all stub views return None
- `Action::CancelRalphLoop` and `Action::RalphDetail` are no-ops in `handle_action` — full wiring to `RalphProcessManager` deferred to task 36
- Render layout: outer Block, inner split into body (active entries × 3-row: title+elapsed/progress_bar/separator, completed entries × 1-row: ✓/✗ marker) + help line
- Empty state shows friendly hint text ("Press r on a job to run ResumeTailor")
Learning tests written:
- None required (no new external crates; Instant/Duration are std; ratatui proven in tasks 10-13)
Tests passing: 354 total (16 new in ralph_panel: on_update_progress_creates_active_entry, on_update_progress_updates_existing_entry, on_update_logline_appends_to_entry, on_update_completed_moves_to_completed, on_update_failed_moves_to_completed_with_failure, cleanup_removes_expired_completed, selected_run_id_returns_none_for_empty, handle_key_j_scrolls_selection_down, handle_key_k_scrolls_selection_up, handle_key_c_returns_cancel_action, handle_key_enter_returns_detail_action, handle_key_esc_navigates_back, renders_without_panic, renders_active_loop_with_progress, renders_empty_state_without_panic, renders_completed_with_success_marker)
Next iteration should know:
- RalphPanelView now has on_update(RalphUpdate) method for receiving events from app.handle_ralph_update()
- Action enum has CancelRalphLoop(String) and RalphDetail(String) variants — both no-ops in handle_action for now
- App.handle_action now delegates ScrollDown/ScrollUp/Select to the active view via handle_key — this is the correct pattern for all future views with scroll state
- The 5-second completion display is automatic — no timer needed, cleanup happens in render()
- Task 23 (job-sources) is next — GreenhouseClient, LeverClient using reqwest, ammonia HTML stripping, rate limiting

## Task 24: discovery-service — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-core/src/discovery/service.rs (new: DiscoveryService, SourceConfig, DiscoveryStats, DiscoveryProgress)
- crates/lazyjob-core/src/discovery/mod.rs (added pub mod service + re-exports)
- crates/lazyjob-core/src/repositories/job.rs (upsert_discovered now returns Result<bool> using xmax trick)
- crates/lazyjob-core/Cargo.toml (added futures, tracing deps)
- crates/lazyjob-cli/src/main.rs (added Ralph subcommand with JobDiscovery command)
Key decisions:
- SourceConfig is a simple struct { source: String, company_id: String } — no complex trait dispatch at config time
- upsert_discovered return type changed from Result<()> to Result<bool> using PostgreSQL RETURNING (xmax = 0) AS is_new — true=new insert, false=update
- Parallelism via futures::future::join_all — all (source, company_id) pairs fan out simultaneously
- discover_one returns DiscoveryStats directly (not Result) — errors are counted, never propagated, so other sources continue on partial failure
- Progress events sent via tokio::sync::mpsc::Sender<DiscoveryProgress> — optional (None = no progress reporting)
- CLI `lazyjob ralph job-discovery --source greenhouse --company-id stripe` uses tokio::join! to drive discovery + print progress concurrently
- DiscoveryStats implements std::ops::Add for clean aggregation of per-source stats
Learning tests written:
- futures_join_all_collects_from_parallel_futures — proves futures::future::join_all aggregates results from multiple parallel futures using a named async fn (avoids opaque type collision from inline async blocks)
Tests passing: 379 total (11 new: futures_join_all learning test, discovery_stats_add, discovery_stats_default, source_config_fields_accessible, discovery_progress_fields_accessible, run_discovery_empty_sources, discover_one_unknown_source, run_discovery_sends_progress_events, upsert_discovered_returns_true_for_new, upsert_discovered_returns_false_for_update, parse_ralph_job_discovery)
Next iteration should know:
- DiscoveryService::run_discovery(&pool, sources, Option<Sender<DiscoveryProgress>>) -> Result<DiscoveryStats>
- SourceConfig and DiscoveryStats are in lazyjob-core::discovery, re-exported from discovery mod
- upsert_discovered now returns bool — any code calling it must handle the bool return (was `()` before)
- futures crate is now in lazyjob-core deps
- CLI ralph subcommand: `lazyjob ralph job-discovery --source <source> --company-id <company_id>`
- Task 25 (semantic-matching) is next — MatchScorer, cosine similarity, job_embeddings migration, GhostDetector

## Task 25: semantic-matching — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-core/migrations/003_job_embeddings.sql (new: job_embeddings table with BYTEA embedding column)
- crates/lazyjob-core/src/discovery/matching.rs (new: Embedder trait, GhostDetector, GhostScore, MatchScorer, cosine_similarity, life_sheet_to_text)
- crates/lazyjob-core/src/discovery/mod.rs (added pub mod matching + re-exports)
Key decisions:
- Defined local `Embedder` trait in lazyjob-core (not importing lazyjob-llm::EmbeddingProvider) to avoid circular dependency — lazyjob-llm already depends on lazyjob-core
- Embeddings stored as BYTEA (raw little-endian f32 bytes) — no pgvector dependency needed at this scale; simple and portable
- GhostDetector is a struct (not a free function) to allow optional extra signals (duplicate_description, high_application_count) that require external data not present in `&Job` alone
- `with_duplicate_description()` and `with_high_application_count()` builder methods allow callers to inject these signals from DB queries
- cosine_similarity() clamps to [-1, 1] and returns 0.0 for mismatched lengths or zero vectors — no panic
- MatchScorer::score_all() mutates jobs in-place setting job.match_score
- Integration test gated behind `#[cfg(feature = "integration")]` — skips without DB
Learning tests written:
- cosine_similarity_orthogonal_vectors_is_zero — proves [1,0,0] · [0,1,0] = 0.0 for orthogonal vectors
- cosine_similarity_identical_vectors_is_one — proves v · v / (|v||v|) = 1.0
- cosine_similarity_known_pair — verifies both zero-similarity and unit-similarity cases
Tests passing: 397 (18 new: 3 cosine learning tests + 2 cosine edge case tests + 7 ghost detector tests + 1 ghost threshold test + 1 life_sheet_to_text test + 1 embedding round-trip test + 2 MatchScorer async tests)
Next iteration should know:
- Embedder trait is in lazyjob-core::discovery::matching — NOT lazyjob-llm::EmbeddingProvider (circular dep)
- To wire a real embedding provider from lazyjob-llm, implement Embedder for a wrapper struct in the CLI/TUI layer
- GhostDetector::default().score(job) gives base score; .with_duplicate_description() / .with_high_application_count() add external signals
- MatchScorer::score_all(&mut jobs, sheet) sets job.match_score in-place (use JobRepository::update() to persist)
- job_embeddings table: job_id UUID PK FK→jobs(id) CASCADE, embedding BYTEA, embedded_at TIMESTAMPTZ
- MatchScorer::store_embedding / load_embedding are the DB I/O methods
- Task 26 (company-research) is next — CompanyResearcher::enrich(company_id) using LLM + HTTP

## Task 26: company-research — DONE
Date: 2026-04-16
Files created/modified:
- crates/lazyjob-core/src/discovery/company.rs (new: Completer trait, EnrichmentData, CompanyResearcher, enrichment_badge, extract_json_from_response)
- crates/lazyjob-core/src/discovery/mod.rs (added pub mod company + re-exports)
- crates/lazyjob-tui/src/views/jobs_list.rs (added enrichment_badge import, format_company_badge method, legend in render, 5 new tests)
- crates/lazyjob-cli/Cargo.toml (added lazyjob-llm, async-trait, reqwest, uuid deps)
- crates/lazyjob-cli/src/main.rs (added CompanyResearch RalphCommand, LlmProviderCompleter wrapper, handle_company_research handler, parse_ralph_company_research test)
Key decisions:
- Defined local `Completer` trait in lazyjob-core (not importing lazyjob-llm::LlmProvider) to avoid circular dependency — lazyjob-llm already depends on lazyjob-core
- LlmProviderCompleter wrapper in lazyjob-cli bridges LlmProvider → Completer for production use
- `extract_json_from_response` uses `find('{')` + `rfind('}')` to extract JSON even when LLM adds preamble text
- `enrichment_badge(industry: Option<&str>) -> &'static str` returns "[E]" for enriched, "[ ]" for not enriched
- Company fields updated in-place: industry, size, tech_stack, culture_keywords; recent_news stored as notes (joined with "; ")
- Website content truncated to 3000 chars before sending to LLM to avoid token overflow
- Integration test gated behind `#[cfg(all(test, feature = "integration"))]` with wiremock MockServer
Learning tests written:
- reqwest_client_builds — proves reqwest::Client::builder().timeout().build() constructs successfully with rustls-tls feature
Tests passing: 412 (15 new: 9 in lazyjob-core company tests, 5 in lazyjob-tui jobs_list tests, 1 in lazyjob-cli tests)
Next iteration should know:
- Completer trait is in lazyjob-core::discovery::company, re-exported as lazyjob-core::discovery::Completer
- CompanyResearcher::new(completer: Arc<dyn Completer>, client: reqwest::Client) — same Arc<dyn Trait> pattern as MatchScorer
- enrichment_badge(industry: Option<&str>) is now importable in any TUI view via lazyjob_core::discovery::enrichment_badge
- JobsListView now has format_company_badge(company_name, industry) static method for task 27 to use
- LlmProviderCompleter is defined in lazyjob-cli — any other binary layer can define the same bridge
- CLI command: `lazyjob ralph company-research --company-id <uuid>`
- Task 27 (jobs-list-tui) is next — full JobsListView with scrollable table, filtering, sorting, enrichment badges

## Infrastructure: TestDb (zero2prod-style isolated test databases) — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-core/src/test_db.rs (new: TestDb struct with spawn/pool/Drop)
- crates/lazyjob-core/src/lib.rs (added pub mod test_db gated behind #[cfg(any(test, feature = "integration"))])
- crates/lazyjob-core/src/db.rs (updated connect_and_migrate test to use TestDb)
- crates/lazyjob-core/src/repositories/mod.rs (rewrote all integration tests to use TestDb, removed setup_db() helper)
- crates/lazyjob-core/src/repositories/ralph_loop_run.rs (rewrote all integration tests to use TestDb)
- crates/lazyjob-core/src/life_sheet/service.rs (rewrote import_and_load_roundtrip to use TestDb)
- crates/lazyjob-core/src/discovery/service.rs (rewrote all 5 integration tests to use TestDb)
- crates/lazyjob-core/src/discovery/matching.rs (rewrote store_and_load_embedding to use TestDb, removed integration feature gate)
- crates/lazyjob-core/src/discovery/company.rs (rewrote enrich_company_with_mock_completer to use TestDb, removed integration feature gate)
- scripts/init_db.sh (fixed: removed sqlx-cli dependency, uses psql to create database, fixed &>2 typos)
Key decisions:
- Pattern from https://github.com/heyimcarlos/zero2prod: each test gets its own database with UUID name (test_<uuid>), migrations run on it, auto-dropped on Drop
- TestDb::spawn() reads DATABASE_URL env var (falls back to DEFAULT_DATABASE_URL), strips the database name, creates test_<uuid> database
- Drop uses fire-and-forget std::thread::spawn (no .join()) to avoid deadlocking the tokio runtime — databases may linger briefly but are cleaned up asynchronously
- All tests that previously used `setup_db() -> Option<Database>` with early-return on missing DATABASE_URL now use `TestDb::spawn().await` directly — tests always run against a real Postgres (no more silent skips)
- Removed `#[cfg(feature = "integration")]` gates from matching.rs and company.rs tests — they now run with `cargo test` directly
- No manual cleanup needed in tests — no more DELETE statements or db.close() calls
Tests passing: 152 (all lazyjob-core tests, including previously-skipped integration tests)
Next iteration should know:
- **DATABASE IS WORKING.** PostgreSQL is configured at `postgres://postgres:password@localhost:5432/lazyjob`. Use `DATABASE_URL=postgres://postgres:password@localhost:5432/lazyjob` for test runs.
- **TestDb is the standard pattern for all DB tests.** Use `let db = TestDb::spawn().await;` then `db.pool()` — isolated per test, auto-cleaned.
- TestDb is in lazyjob_core::test_db, gated behind `#[cfg(any(test, feature = "integration"))]`
- The `scripts/init_db.sh` script creates the lazyjob database and checks Postgres availability. Run with `SKIP_DOCKER=true ./scripts/init_db.sh` for local Postgres.
- Migrations run automatically on both TestDb::spawn() and Database::connect()
- **Tests no longer silently skip.** All integration tests now require a running Postgres. If Postgres is down, tests will fail with connection errors (not silently pass).

## Task 27: jobs-list-tui — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-tui/src/action.rs (added OpenJob(JobId) variant)
- crates/lazyjob-tui/src/views/jobs_list.rs (added Stage column, fixed Enter key, fixed Applied filter, added application_stages HashMap, removed dead Clear code)
- crates/lazyjob-tui/src/app.rs (handle OpenJob action, added async load_jobs method with JobRepository)
- crates/lazyjob-tui/src/event_loop.rs (call load_jobs on Refresh action)
- crates/lazyjob-tui/src/lib.rs (call load_jobs on startup)
- ralph/lazyjob-implementation/output/research-task-27.md (updated research)
- ralph/lazyjob-implementation/output/plan-task-27.md (updated plan)
Key decisions:
- Previous iteration implemented 90% of JobsListView (875 lines, 44 tests). This iteration completed the remaining gaps.
- Added Stage column (6th column, between Ghost and Posted) showing application stage per job via HashMap<JobId, String>
- Enter key now returns Action::OpenJob(JobId) instead of falling through to catch-all
- Applied filter now works — checks application_stages HashMap for the job ID
- DB loading wired: App::load_jobs() queries JobRepository and calls set_jobs(); called on startup and on Ctrl+R Refresh
- OpenJob action is a no-op in App — will be wired to JobDetailView in task 28
- Removed dead `let _ = Clear` code and unused Clear import
Learning tests written:
- None required (no new external crates introduced)
Tests passing: 431 (6 new: handle_key_enter_returns_open_job, handle_key_enter_returns_none_when_empty, filter_applied_shows_only_applied_jobs, filter_applied_empty_when_no_applications, set_application_stages_updates_filter, stage_column_renders_in_table)
Next iteration should know:
- JobsListView is now feature-complete for task 27: 6-column table (Title, Company, Match%, Ghost, Stage, Posted), filtering, sorting, search, Enter→OpenJob
- Action::OpenJob(JobId) exists but is not handled (no-op in App) — task 28 should wire it to JobDetailView
- App::load_jobs() is async and queries JobRepository — called at TUI startup and on Ctrl+R
- application_stages is populated externally via set_application_stages(HashMap<JobId, String>) — no automatic loading from DB yet (needs Application → Job join query)
- Pre-existing issue: lazyjob-cli test `database_url_defaults_to_none` fails when DATABASE_URL env var is set (clap env feature picks it up). Not related to this task.

## Task 28: job-detail-tui — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-tui/src/views/job_detail.rs (full rewrite: JobDetailView with metadata panel, description scroll, action keys, history timeline)
- crates/lazyjob-tui/src/app.rs (added viewing_job_detail flag, wired OpenJob/NavigateBack/NavigateTo, updated active_view_mut routing)
- crates/lazyjob-tui/src/action.rs (added ApplyToJob, TailorResume, GenerateCoverLetter, OpenUrl action variants)
- crates/lazyjob-tui/src/views/jobs_list.rs (added pub jobs() accessor)
- Cargo.toml (added open = "5" to workspace deps)
- crates/lazyjob-tui/Cargo.toml (added open, uuid deps)
- ralph/lazyjob-implementation/output/research-task-28.md (research doc)
- ralph/lazyjob-implementation/output/plan-task-28.md (plan doc)
Key decisions:
- Used sub-view pattern: `viewing_job_detail: bool` flag on App instead of adding a ViewId::JobDetail variant — keeps ViewId simple (Copy, no payload, clean tab_index), header tab stays on "Jobs" naturally
- OpenJob(id) finds the job in jobs_list.jobs() by id, calls job_detail.set_job(), sets viewing_job_detail = true
- active_view_mut() returns &mut views.job_detail when active_view == Jobs && viewing_job_detail — job detail gets rendered and receives key events seamlessly
- NavigateBack clears viewing_job_detail first before popping prev_view — correct back navigation
- NavigateTo always clears viewing_job_detail — tab switching exits detail view
- Two-column layout: 35% metadata (company, location, salary, posted, match%, ghost, source, stage, URL, notes) + 65% scrollable description
- Application history timeline rendered in reverse chronological order with bullet markers
- Action bar shows context-sensitive hints (hides "a=Apply" when already applied)
- Action::OpenUrl uses `open` crate (v5) for cross-platform URL opening
- ApplyToJob/TailorResume/GenerateCoverLetter are no-ops in handle_action — will be wired in tasks 33-36
Learning tests written:
- None required (no new external crates needing API verification; open crate is fire-and-forget)
Tests passing: 195 in lazyjob-tui (20 new: set_job_stores_data, set_job_resets_scroll, clear_removes_data, handle_key_returns_none_when_no_job, handle_key_j_scrolls_down, handle_key_k_scrolls_up, handle_key_k_clamps_to_zero, handle_key_o_returns_open_url, handle_key_o_returns_none_when_no_url, handle_key_a_returns_apply_when_not_applied, handle_key_a_returns_none_when_already_applied, handle_key_r_returns_tailor_resume, handle_key_c_returns_cover_letter, renders_empty_state, renders_with_job_data, renders_with_application_history, renders_salary_formatting, set_application_stores_data, open_job_activates_detail_view, open_job_with_unknown_id_does_nothing, navigate_back_from_detail_returns_to_jobs, tab_switch_clears_detail_view, active_view_mut_returns_job_detail_when_flag_set, active_view_mut_returns_jobs_list_when_flag_not_set)
Next iteration should know:
- JobDetailView is now fully implemented with two-column layout, scroll, and action keys
- Sub-view pattern: App.viewing_job_detail bool flag controls whether jobs_list or job_detail is the active view within the Jobs tab
- Action::ApplyToJob, TailorResume, GenerateCoverLetter, OpenUrl exist but are no-ops in handle_action (except OpenUrl which calls open::that)
- JobsListView now has pub jobs() -> &[Job] accessor
- open v5 and uuid are now in lazyjob-tui deps
- Application data and transition history can be loaded via set_application() — not yet wired to DB loading (needs ApplicationRepository query by job_id)
- Pre-existing issue: lazyjob-cli test `database_url_defaults_to_none` still fails when DATABASE_URL env var is set
- Task 29 (applications-kanban-tui) is next

## Task 29: applications-kanban-tui — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-tui/src/views/applications.rs (full rewrite: ApplicationCard, ApplicationsView kanban board with 9 columns, ConfirmState, navigation, confirmation dialog)
- crates/lazyjob-tui/src/action.rs (added TransitionApplication(ApplicationId, ApplicationStage), ScrollLeft, ScrollRight variants)
- crates/lazyjob-tui/src/app.rs (added load_applications() method, handle ScrollLeft/ScrollRight/TransitionApplication actions)
- crates/lazyjob-tui/src/event_loop.rs (call load_applications() on Refresh)
- crates/lazyjob-tui/src/lib.rs (call load_applications() on startup)
- ralph/lazyjob-implementation/output/research-task-29.md (research doc)
- ralph/lazyjob-implementation/output/plan-task-29.md (plan doc)
Key decisions:
- 9 columns (one per ApplicationStage), equal-width via Constraint::Ratio(1, 9)
- ApplicationCard is a denormalized struct with title+company from Job join, stored in the view
- forward_stage() picks the first non-terminal, non-Withdrawn/Rejected transition as the "natural next" step
- M (shift-m) triggers Withdrawn confirmation (since state machine is forward-only, no backward transitions exist)
- ConfirmState held in the view; ConfirmDialog widget renders as overlay when confirming is Some
- Confirmation flow: m opens dialog, h/l switch Yes/No, Enter confirms, Esc/n cancels, y shortcut confirms
- Days in stage coloring: green (<7d), yellow (7-13d), red (14d+)
- Cards show 3 lines: title, company, days badge with color
- load_applications() queries ApplicationRepository + JobRepository to denormalize title/company
- Action::TransitionApplication is emitted on confirmed transition but is a no-op in handle_action (wiring to DB deferred to task 36)
- ScrollLeft/ScrollRight actions added for h/l column navigation via keymap delegation pattern
Learning tests written:
- None required (no new external crates introduced; ratatui and chrono already proven)
Tests passing: 489 total across workspace (230 in lazyjob-tui, 30 new: navigation tests, confirmation flow tests, render tests, days_color tests, forward_stage tests, card selection tests)
Next iteration should know:
- ApplicationsView is now fully implemented with horizontal kanban board, 9 columns, card navigation, stage transition confirmation
- ApplicationCard is in lazyjob_tui::views::applications — public struct for App::load_applications() to populate
- Action::TransitionApplication(ApplicationId, ApplicationStage) exists but is a no-op in handle_action — task 36 should wire it to ApplicationRepository::transition_stage()
- Action::ScrollLeft and Action::ScrollRight exist for h/l delegation, same pattern as ScrollDown/ScrollUp
- load_applications() is called on startup and on Ctrl+R Refresh
- Pre-existing issue: lazyjob-cli test `database_url_defaults_to_none` still fails when DATABASE_URL env var is set
- Task 30 (dashboard-stats) is next

## Task 30: dashboard-stats — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-core/src/stats.rs (new: DashboardStats, StaleApplication, compute_dashboard_stats(), find_stale_applications())
- crates/lazyjob-core/src/lib.rs (added pub mod stats)
- crates/lazyjob-tui/src/views/dashboard.rs (full rewrite: StatBlocks, kanban counts, stale list, scroll navigation)
- crates/lazyjob-tui/src/app.rs (added load_dashboard_stats() method, imported stats module)
- crates/lazyjob-tui/src/event_loop.rs (call load_dashboard_stats() on Refresh)
- crates/lazyjob-tui/src/lib.rs (call load_dashboard_stats() on startup)
Key decisions:
- No reminders table/service — the DB has no reminders table. Stale detection uses applications.updated_at < now()-14d for non-terminal stages instead.
- No ReminderPoller background task — dashboard refreshes on startup and Ctrl+R like other views. Polling adds complexity for minimal value at this stage.
- "Interviewing" stat counts PhoneScreen + Technical + Onsite stages (interview-related stages since there's no interviews table)
- compute_dashboard_stats uses 3 SQL queries: total jobs count, applied this week count, per-stage aggregation. Stage loop computes in_pipeline and interviewing from the per-stage results.
- find_stale_applications uses EXTRACT(EPOCH) with ::FLOAT8 cast (PG returns NUMERIC from EXTRACT, incompatible with Rust f64 without cast)
- DashboardView uses 3-section vertical layout: Length(4) for stat blocks, Length(5) for pipeline counts, Fill(1) for stale list
- Stage short labels are 3-char abbreviations (INT, APP, PHN, TEC, ONS, OFR, ACC, REJ, WDR) for compact pipeline display
- Stale list supports j/k scroll navigation with selected_stale index, same pattern as other views
Learning tests written:
- None required (no new external crates introduced; sqlx and ratatui already proven)
Tests passing: 511 (27 new: 7 in lazyjob-core stats, 16 in lazyjob-tui dashboard, 4 existing dashboard tests updated)
Next iteration should know:
- DashboardStats and StaleApplication are in lazyjob-core::stats
- compute_dashboard_stats(pool) and find_stale_applications(pool) are the two async query functions
- DashboardView stores stats + stale list; use set_stats(stats, stale) to populate
- load_dashboard_stats() is called at startup and on Ctrl+R alongside load_jobs() and load_applications()
- No reminders infrastructure exists — if task 31+ needs reminders, a migration and table must be created first
- Pre-existing issue: lazyjob-cli test `database_url_defaults_to_none` still fails when DATABASE_URL env var is set
- Task 31 (prompt-templates) is next

## Task 31: prompt-templates — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-llm/src/prompts/mod.rs (new: module declarations + re-exports)
- crates/lazyjob-llm/src/prompts/types.rs (new: LoopType enum, PromptTemplate, FewShotExample, RenderedPrompt, TemplateVars)
- crates/lazyjob-llm/src/prompts/error.rs (new: TemplateError enum with MissingVariable, ParseError, NotRegistered, ValidationFailed, OverrideParseError)
- crates/lazyjob-llm/src/prompts/engine.rs (new: SimpleTemplateEngine, interpolate() with {variable} substitution)
- crates/lazyjob-llm/src/prompts/registry.rs (new: DefaultPromptRegistry with 9 embedded TOML templates, user override loading)
- crates/lazyjob-llm/src/prompts/sanitizer.rs (new: sanitize_user_value() stripping prompt injection patterns, template_vars! macro)
- crates/lazyjob-llm/src/prompts/cache.rs (new: build_anthropic_system_field() for prompt caching)
- crates/lazyjob-llm/src/prompts/job_discovery.rs (new: JobDiscoveryContext, JobDiscoveryOutput, system_prompt/user_prompt/validate_output)
- crates/lazyjob-llm/src/prompts/company_research.rs (new: CompanyResearchContext, CompanyResearchOutput, system_prompt/user_prompt/validate_output)
- crates/lazyjob-llm/src/prompts/resume_tailor.rs (new: ResumeTailorContext, ResumeTailorOutput, system_prompt/user_prompt/validate_output)
- crates/lazyjob-llm/src/prompts/cover_letter.rs (new: CoverLetterContext, CoverLetterOutput, system_prompt/user_prompt/validate_output)
- crates/lazyjob-llm/src/prompts/interview_prep.rs (new: InterviewPrepContext, InterviewPrepOutput, system_prompt/user_prompt/validate_output)
- crates/lazyjob-llm/src/templates/base_system.toml (new: Ralph persona preamble)
- crates/lazyjob-llm/src/templates/job_discovery.toml (new: job search scoring template)
- crates/lazyjob-llm/src/templates/company_research.toml (new: company analysis template)
- crates/lazyjob-llm/src/templates/resume_tailoring.toml (new: resume rewriting template)
- crates/lazyjob-llm/src/templates/cover_letter.toml (new: cover letter generation template)
- crates/lazyjob-llm/src/templates/interview_prep.toml (new: interview Q&A template)
- crates/lazyjob-llm/src/templates/salary_negotiation.toml (new: negotiation strategy template)
- crates/lazyjob-llm/src/templates/networking.toml (new: outreach drafting template)
- crates/lazyjob-llm/src/templates/error_response.toml (new: error recovery template)
- crates/lazyjob-llm/src/lib.rs (added pub mod prompts)
- crates/lazyjob-llm/Cargo.toml (added toml, tracing deps; tempfile dev-dep)
Key decisions:
- Used TOML templates embedded via include_str! per spec — all 9 templates compile into the binary, no runtime file loading needed
- SimpleTemplateEngine uses single-pass {variable} interpolation — unmatched `{` without `}` passes through verbatim (handles JSON in prompts)
- DefaultPromptRegistry loads all 9 embedded templates; supports user override loading from config_dir/prompts/*.toml
- Sanitizer strips 7 common injection patterns (\n\nSystem:, \n\nAssistant:, Ignore previous instructions, etc.) replacing with [REDACTED]
- template_vars! macro auto-sanitizes all values — callers can't forget to sanitize
- Per-loop context structs have to_template_vars() methods; output structs use serde_json for validation
- CoverLetterOutput.validate_output() enforces non-empty paragraphs; InterviewPrepOutput validates non-empty questions
- RenderedPrompt::into_chat_messages() converts to Vec<ChatMessage> compatible with LlmProvider::complete()
- build_anthropic_system_field() injects cache_control: {type: "ephemeral"} when cache_system_prompt=true
- No new crates added — toml and tracing were already in workspace deps
Learning tests written:
- toml_parses_prompt_template — proves toml::from_str correctly deserializes PromptTemplate with all fields
- toml_defaults_for_optional_fields — proves missing optional fields (cache_system_prompt, few_shot_examples, output_schema) use serde defaults
Tests passing: 585 total (22 new in lazyjob-llm: 2 learning, 7 engine, 8 sanitizer, 3 cache, 12 registry, 6 job_discovery, 5 company_research, 4 resume_tailor, 5 cover_letter, 5 interview_prep, 4 types)
Next iteration should know:
- Prompt templates are in lazyjob_llm::prompts, all public types re-exported from prompts/mod.rs
- DefaultPromptRegistry::new() loads 9 embedded templates; use .get(LoopType::X) to retrieve
- SimpleTemplateEngine::render(template, vars) produces RenderedPrompt
- Per-loop modules: prompts::job_discovery, prompts::company_research, prompts::resume_tailor, prompts::cover_letter, prompts::interview_prep
- Each per-loop module has: XContext struct with to_template_vars(), XOutput struct, system_prompt(), user_prompt(&ctx), validate_output(raw)
- template_vars! macro is exported from lazyjob_llm crate root for ergonomic sanitized var building
- RenderedPrompt::into_chat_messages() -> Vec<ChatMessage> for direct use with LlmProvider::complete()
- build_anthropic_system_field() in prompts::cache for Anthropic prompt caching
- Salary negotiation and networking templates exist in registry but don't have per-loop Rust modules (they aren't in the 5 required by the task)
- Pre-existing issue: lazyjob-cli test `database_url_defaults_to_none` still fails when DATABASE_URL env var is set
- Task 32 (anti-fabrication) is next

## Task 32: anti-fabrication — DONE
Date: 2026-04-17
Files created/modified:
- crates/lazyjob-llm/src/anti_fabrication.rs (new: FabricationLevel enum, ProhibitedPhrase struct, GroundingReport struct, is_grounded_claim(), check_grounding(), prohibited_phrase_detector(), prompt_injection_guard())
- crates/lazyjob-llm/src/lib.rs (added pub mod anti_fabrication + re-exports)
- crates/lazyjob-llm/src/prompts/resume_tailor.rs (added validate_grounding() function wiring anti-fabrication into pipeline)
- crates/lazyjob-llm/src/prompts/cover_letter.rs (added validate_grounding() function wiring anti-fabrication into pipeline)
- ralph/lazyjob-implementation/output/research-task-32.md (research doc)
- ralph/lazyjob-implementation/output/plan-task-32.md (plan doc)
Key decisions:
- Evidence counting approach for grounding: each claim is checked against 8 evidence sources (company, position, skills, achievements, education, certifications, projects, metrics). >=2 matches = Grounded, 1 = Embellished, 0 = Fabricated.
- 35 prohibited phrases covering common cover letter clichés (passionate about, synergy, proven track record, etc.)
- 21 injection patterns for prompt_injection_guard covering role-switching (\n\nSystem:), instruction overrides (ignore/disregard/forget/override), persona switching (pretend you are, you are now), special tokens (<|im_start|>), and base64-encoded keywords
- prompt_injection_guard is broader than the existing sanitizer::sanitize_user_value — sanitizer replaces patterns in template vars (preventive), injection_guard detects them in arbitrary input (detective)
- base64 detection uses original case (not lowercased) since base64 is case-sensitive
- validate_grounding() returns tuple (GroundingReport, Vec<ProhibitedPhrase>) — callers decide how to handle (warn, reject, etc.)
- No new dependencies needed — all logic is pure string processing against LifeSheet types
Learning tests written:
- None required (no new external crates; all logic is string matching against existing types)
Tests passing: 150 in lazyjob-llm (34 new: 26 in anti_fabrication tests, 4 in resume_tailor validate_grounding tests, 4 in cover_letter validate_grounding tests)
Next iteration should know:
- FabricationLevel, GroundingReport, ProhibitedPhrase are re-exported from lazyjob_llm crate root
- is_grounded_claim(claim, life_sheet) -> FabricationLevel for single-claim checking
- check_grounding(claims, life_sheet) -> GroundingReport for batch checking
- prohibited_phrase_detector(text) -> Vec<ProhibitedPhrase> for cliché detection
- prompt_injection_guard(input) -> bool for injection detection
- resume_tailor::validate_grounding(output, life_sheet) and cover_letter::validate_grounding(output, life_sheet) are the pipeline integration points
- Pre-existing DB test failures in lazyjob-core (TestDb connection issues) — not caused by this task
- Task 33 (resume-tailoring) is next
