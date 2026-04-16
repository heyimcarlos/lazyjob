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
