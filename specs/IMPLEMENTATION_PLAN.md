# LazyJob — Implementation Plan

Synthesized from 33 spec files in `ralph/spec-jtbd-expansion/output/specs/`. This is the direct input for future ralph build loops.

**Ordering principles**: (1) foundational infrastructure first, (2) core user-facing features second, (3) agentic/AI features third, (4) premium/SaaS last.

---

## Phase 1: Crate Scaffold & Data Foundation

### Crate Layout & Workspace

- [ ] Create `Cargo.toml` workspace at `/home/lab-admin/repos/lazyjob/` with members list for all 5 crates — refs: `architecture-crate-layout.md`
- [ ] Scaffold `lazyjob-core/src/` with `lib.rs`, `error.rs`, and all subdirectory `mod.rs` files with empty modules — refs: `architecture-crate-layout.md`
- [ ] Scaffold `lazyjob-llm/src/`, `lazyjob-ralph/src/`, `lazyjob-tui/src/`, `lazyjob-cli/src/main.rs` with empty modules — refs: `architecture-crate-layout.md`
- [ ] Verify dependency graph: `cargo build` from workspace root compiles all crates in correct order — refs: `architecture-crate-layout.md`
- [ ] Verify no circular dependencies with `cargo check --all` after initial scaffold — refs: `architecture-crate-layout.md`
- [ ] Add `lazyjob-core/src/lexicon/tech_terms.rs` and `jurisdictions.rs` with static data (populated from ghost detection and salary specs) — refs: `architecture-crate-layout.md`, `job-search-ghost-job-detection.md`

### SQLite Persistence & Schema

- [ ] Create `lazyjob-core/src/persistence/migrations/001_initial_schema.sql` with all DDL from spec — refs: `architecture-sqlite-persistence.md`, `04-sqlite-persistence.md`
- [ ] Implement `Database::open()` in `lazyjob-core/src/persistence/database.rs` with SqlitePool, WAL mode, foreign keys, migrations — refs: `architecture-sqlite-persistence.md`
- [ ] Implement all repository traits: `JobRepository`, `ApplicationRepository`, `CompanyRepository`, `ProfileContactRepository`, `InterviewRepository`, `OfferRepository`, `LifeSheetRepository` — refs: `architecture-sqlite-persistence.md`
- [ ] Add `source_quality TEXT DEFAULT 'api'` to `jobs` table in migration `004_jobs_source_quality.sql` — refs: `architecture-sqlite-persistence.md`, `platform-ats-open-apis.md`
- [ ] Add `token_usage_log` table in migration `007_token_usage_log.sql` — refs: `architecture-sqlite-persistence.md`
- [ ] Add `duplicate_log` table in migration `008_duplicate_log.sql` — refs: `architecture-sqlite-persistence.md`
- [ ] Add `offer_details` table (excluded from SaaS sync) in migration `009_offer_details.sql` — refs: `architecture-sqlite-persistence.md`
- [ ] Implement `Database::with_auto_backup()` with WAL dirty-shutdown detection and auto-backup — refs: `architecture-sqlite-persistence.md`
- [ ] Implement `Database::export_all()` for JSON data portability — refs: `architecture-sqlite-persistence.md`
- [ ] Write `cargo sqlx prepare --all` to `sqlx.toml` for offline query compilation — refs: `architecture-sqlite-persistence.md`
- [ ] Implement Ralph subprocess database helper (`open Ralph_database()`) with `busy_timeout=2000` — refs: `architecture-sqlite-persistence.md`, `agentic-ralph-subprocess-protocol.md`

### Life Sheet Data Model

- [ ] Define `LifeSheetYaml` serde structs in `lazyjob-core/src/life_sheet/yaml.rs` matching the YAML schema — refs: `profile-life-sheet-data-model.md`, `03-life-sheet-data-model.md`
- [ ] Write SQLite DDL migration `lazyjob-core/migrations/001_life_sheet.sql` with all life sheet tables and indexes — refs: `profile-life-sheet-data-model.md`
- [ ] Implement `SqliteLifeSheetRepository` in `lazyjob-core/src/life_sheet/sqlite.rs` with `get`, `import`, `export_json_resume`, `get_skills_flat`, `get_experience_for_tailoring` — refs: `profile-life-sheet-data-model.md`
- [ ] Implement `import_life_sheet` in `lazyjob-core/src/life_sheet/import.rs` — parse YAML, truncate tables, re-insert all entities with deterministic IDs — refs: `profile-life-sheet-data-model.md`
- [ ] Implement `export_json_resume` in `lazyjob-core/src/life_sheet/export.rs` — map SQLite rows to JSON Resume schema — refs: `profile-life-sheet-data-model.md`
- [ ] Add `is_grounded_claim` predicate in `lazyjob-core/src/life_sheet/fabrication.rs` used as the anti-fabrication check in resume tailoring — refs: `profile-life-sheet-data-model.md`, `agentic-prompt-templates.md`
- [ ] Wire `lazyjob-cli` `profile import` and `profile export` subcommands to the repository trait — refs: `profile-life-sheet-data-model.md`

---

## Phase 2: TUI Skeleton & Config

### TUI Architecture

- [ ] Implement `App::new()` in `lazyjob-tui/src/app.rs` — load database, initialize LLM, spawn RalphProcessManager — refs: `architecture-tui-skeleton.md`
- [ ] Implement `App::run()` event loop in `lazyjob-tui/src/app.rs` — crossterm event polling, RalphEvent receiver, view dispatch — refs: `architecture-tui-skeleton.md`
- [ ] Implement all views: Dashboard, Jobs, JobDetail, Applications, Contacts, Ralph, Settings, Help — each in `lazyjob-tui/src/views/` — refs: `architecture-tui-skeleton.md`
- [ ] Implement view keybinding dispatch: `AppView::handle_key()` routes to active view's key handler — refs: `architecture-tui-skeleton.md`, `09-tui-design-keybindings.md`
- [ ] Implement `theme.rs` with dark/light themes and all color constants — refs: `architecture-tui-skeleton.md`
- [ ] Implement custom widgets: `job_card.rs`, `application_card.rs`, `contact_card.rs`, `stat_block.rs`, `progress_bar.rs`, `modal.rs` — refs: `architecture-tui-skeleton.md`
- [ ] Implement header navigation bar with view tabs and number key shortcuts `1-6` — refs: `architecture-tui-skeleton.md`, `09-tui-design-keybindings.md`
- [ ] Implement status bar with job count, filter state, Ralph status indicator, and current time — refs: `architecture-tui-skeleton.md`
- [ ] Wire `WorkflowEvent` broadcast channel subscription in TUI event loop — consume and handle ReminderDue, NetworkingReminderDue, DigestReady events — refs: `architecture-tui-skeleton.md`, `application-workflow-actions.md`
- [ ] Implement onboarding prompt for `LifeSheet.goals.short_term` on first run (if goals empty, show banner prompting user to fill in life-sheet.yaml) — refs: `architecture-tui-skeleton.md`, `profile-life-sheet-data-model.md`

### Config Management

- [ ] Define `Config` struct with all sections in `lazyjob-core/src/config/mod.rs` using `toml_edit` for serialization — refs: `architecture-config-management.md`
- [ ] Implement `Config::load()`, `Config::ensure_exists()`, `Config::default()`, `Config::config_path()` for TOML file management — refs: `architecture-config-management.md`
- [ ] Implement API key resolution via `CredentialManager` for `[llm.anthropic]`, `[llm.openai]` references in TOML — refs: `architecture-config-management.md`, `16-privacy-security.md`
- [ ] Add `toml_edit` to `lazyjob-core` dependencies (for config read/write) — refs: `architecture-config-management.md`
- [ ] Add `dirs` crate for `~/.lazyjob` path resolution — refs: `architecture-config-management.md`
- [ ] Implement `[networking]` section: `max_weekly_new_contacts`, `max_follow_up_reminders_per_contact`, `referral_ghost_score_threshold` — wire these values into the networking and referral specs — refs: `architecture-config-management.md`, `networking-referral-management.md`
- [ ] Add `config_version` field to `[general]` for schema migration — refs: `architecture-config-management.md`
- [ ] Implement `Config::migrate_if_needed()` with version-based migration — refs: `architecture-config-management.md`
- [ ] Write TUI settings view (`lazyjob-tui/src/views/settings.rs`) with form fields for all config sections, Save button writes back to TOML, API key Change button updates keyring — refs: `architecture-config-management.md`, `09-tui-design-keybindings.md`
- [ ] Write CLI subcommand `lazyjob config get/set/list` for command-line config management — refs: `architecture-config-management.md`

### Privacy & Security

- [ ] Implement `CredentialManager` in `lazyjob-core/src/security/credentials.rs` with keyring integration and encrypted-file fallback — refs: `architecture-privacy-security.md`, `16-privacy-security.md`
- [ ] Add `SECRET_SERVICE` feature flag to `keyring` crate for Linux `libsecret` support — refs: `architecture-privacy-security.md`
- [ ] Implement `FileEncryption` in `lazyjob-core/src/security/database.rs` with `encrypt_file`/`decrypt_file` for age encryption — refs: `architecture-privacy-security.md`
- [ ] Add `[privacy]` section to `lazyjob.toml`: `mode = "full" | "minimal" | "stealth"`, `encrypt_database = false` — refs: `architecture-privacy-security.md`
- [ ] Implement `PrivacySettings::from_config()` to gate LLM calls, platform API calls, and Ralph loops based on privacy mode — refs: `architecture-privacy-security.md`
- [ ] Implement `export_all()` and `import_all()` in `lazyjob-core/src/security/export.rs` — refs: `architecture-privacy-security.md`
- [ ] Add `NEVER_SYNC_TABLES` constant, integrate with SaaS migration spec — refs: `architecture-privacy-security.md`, `saas-migration-path.md`
- [ ] Write integration test: export → delete → import → verify counts match (roundtrip test) — refs: `architecture-privacy-security.md`

---

## Phase 3: LLM Provider Abstraction

- [ ] Define `LlmProvider` and `EmbeddingProvider` traits in `lazyjob-llm/src/provider.rs` with `async_trait` + `Send + Sync` bounds — refs: `agentic-llm-provider-abstraction.md`, `02-llm-provider-abstraction.md`
- [ ] Define `ChatMessage`, `ChatResponse`, `ChatStreamChunk`, `TokenUsage` in `lazyjob-llm/src/types.rs` with serde derives — refs: `agentic-llm-provider-abstraction.md`
- [ ] Implement `AnthropicProvider` in `lazyjob-llm/src/providers/anthropic.rs` using reqwest with manual SSE parsing, 3-retry backoff, 200K context — refs: `agentic-llm-provider-abstraction.md`
- [ ] Implement `OpenAIProvider` in `lazyjob-llm/src/providers/openai.rs` using `async-openai`, including `EmbeddingProvider` with `text-embedding-3-small` — refs: `agentic-llm-provider-abstraction.md`
- [ ] Implement `OllamaProvider` in `lazyjob-llm/src/providers/ollama.rs` using `ollama-rs`, including `EmbeddingProvider` with `nomic-embed-text` (768 dims) — refs: `agentic-llm-provider-abstraction.md`
- [ ] Implement `ProviderRegistry` and `LlmBuilder` with `from_config()` constructor and Ollama fallback chain — refs: `agentic-llm-provider-abstraction.md`
- [ ] Create `token_usage_log` SQLite table DDL and `cost.rs` microdollar cost estimator — refs: `agentic-llm-provider-abstraction.md`, `architecture-sqlite-persistence.md`
- [ ] Write mockall-based `MockLlmProvider` in `[dev-dependencies]` for use across all worker integration tests — refs: `agentic-llm-provider-abstraction.md`

---

## Phase 4: Ralph Subprocess Protocol & Orchestration

- [ ] Define `WorkerCommand` and `WorkerEvent` enums in `lazyjob-ralph/src/protocol.rs` with serde `tag="type"` derivations — refs: `agentic-ralph-subprocess-protocol.md`, `06-ralph-loop-integration.md`
- [ ] Implement `RalphProcessManager::spawn()` with tokio::process stdin/stdout pipe, stdout reader task that parses `WorkerEvent` and broadcasts, and stdin writer task consuming `mpsc::Sender<WorkerCommand>` — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Implement `RalphProcessManager::cancel()` with 3-second kill fallback using `tokio::time::timeout` — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Implement `RalphProcessManager::send_user_input()` for interactive `MockInterviewLoop` mode — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Create `ralph_loop_runs` SQLite table DDL (migration) and implement `recover_pending()` to detect TUI-crash-orphaned runs on startup — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Implement `reap_dead_workers()` via `child.try_wait()` health check — called from a 5-second periodic task in the TUI event loop — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Add stderr-to-log-file redirection in process spawner (`~/.lazyjob/logs/ralph-<loop_id>.log`, 7-day retention) — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Write unit tests for `WorkerCommand`/`WorkerEvent` round-trip JSON serialization and the interactive state machine transitions — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Define `LoopType` enum in `lazyjob-ralph/src/loop_types.rs` with `concurrency_limit()`, `priority()`, `is_interactive()`, `cli_subcommand()` methods — refs: `agentic-ralph-orchestration.md`
- [ ] Implement `LoopDispatch` in `lazyjob-ralph/src/dispatch.rs` with `enqueue()` (immediate spawn or queue), `dispatch_suggestion()` mapping `PostTransitionSuggestion` variants to `LoopType`+params, and `drain_queue()` — refs: `agentic-ralph-orchestration.md`
- [ ] Implement bounded priority queue (`BinaryHeap<QueuedLoop>`, cap 20, `priority` field) in `lazyjob-ralph/src/queue.rs` — refs: `agentic-ralph-orchestration.md`
- [ ] Implement `LoopScheduler` in `lazyjob-ralph/src/scheduler.rs` with cron expression parsing and daily `JobDiscovery` dispatch — refs: `agentic-ralph-orchestration.md`
- [ ] Add `ralph_loop_runs` status update calls in `LoopDispatch` at each lifecycle event — refs: `agentic-ralph-orchestration.md`
- [ ] Write `[ralph.scheduler]` config section documentation in `lazyjob.toml` example file — refs: `agentic-ralph-orchestration.md`
- [ ] Integration test: spawn `JobDiscovery` to concurrency limit, verify 3rd enqueue goes to queue, verify drain after first completes — refs: `agentic-ralph-orchestration.md`

---

## Phase 5: Prompt Templates & Anti-Fabrication

- [ ] Create `lazyjob-llm/src/prompts/` module hierarchy with one file per loop type; each file exposes `system_prompt()`, `user_prompt(ctx)`, `validate_output(raw, ctx)` — refs: `agentic-prompt-templates.md`, `17-ralph-prompt-templates.md`
- [ ] Define all seven context structs (grounding inputs) in `lazyjob-llm/src/prompts/context.rs`, drawing from established types in `lazyjob-core` — refs: `agentic-prompt-templates.md`
- [ ] Implement `is_grounded_claim()` and `check_negotiation_fabrication()` in `lazyjob-core/src/life_sheet/fabrication.rs` with deterministic regex+lookup approach — refs: `agentic-prompt-templates.md`
- [ ] Add the `FabricationLevel` enum and `FabricationFinding` struct with `field_name`, `claimed_value`, `grounded_value` fields — refs: `agentic-prompt-templates.md`
- [ ] Implement prohibited-phrase detection for cover letter template (compile regex patterns once at startup using `once_cell::sync::Lazy`) — refs: `agentic-prompt-templates.md`
- [ ] Write prompt injection defense block as a module-level constant in `lazyjob-llm/src/prompts/mod.rs` shared across all system prompts — refs: `agentic-prompt-templates.md`
- [ ] Write unit tests for each `validate_output()` with golden JSON examples (clean pass, fabrication case, missing field case) — refs: `agentic-prompt-templates.md`
- [ ] Document all JSON output schemas in `lazyjob-llm/src/prompts/schemas/` as `serde_json::Value` schema constants for runtime validation — refs: `agentic-prompt-templates.md`

---

## Phase 6: Job Search & Discovery

- [ ] Define `JobSource` trait and implement `GreenhouseSource` and `LeverSource` in `lazyjob-core/src/discovery/sources/` — refs: `job-search-discovery-engine.md`, `05-job-discovery-layer.md`, `11-platform-api-integrations.md`
- [ ] Implement `EnrichmentPipeline` with HTML sanitization (ammonia), salary extraction (regex), and remote classification — refs: `job-search-discovery-engine.md`
- [ ] Implement `CompanyRegistry` that reads from `config.toml` and dispatches to registered sources — refs: `job-search-discovery-engine.md`
- [ ] Implement `DiscoveryService::run_discovery()` with parallel fan-out via `tokio::spawn` + `RateLimiter` per source — refs: `job-search-discovery-engine.md`
- [ ] Add cross-source deduplication pass after initial ingestion using title + company_id normalized match — refs: `job-search-discovery-engine.md`, `platform-aggregation-deduplication.md`
- [ ] Implement `AdzunaSource` as optional Phase 2 source gated behind config flag — refs: `job-search-discovery-engine.md`
- [ ] Wire `DiscoveryService` into `lazyjob-ralph` subprocess so discovery results write directly to SQLite — refs: `job-search-discovery-engine.md`, `agentic-ralph-subprocess-protocol.md`

### Semantic Matching

- [ ] Define `Embedder` trait and implement `OllamaEmbedder` using the `/api/embeddings` endpoint in `lazyjob-core/src/matching/` — refs: `job-search-semantic-matching.md`, `05-job-discovery-layer.md`
- [ ] Add `embedding BLOB` and `match_score REAL` columns to the `jobs` table migration and implement `JobRepository::update_embedding` and `update_match_score` — refs: `job-search-semantic-matching.md`
- [ ] Implement `MatchScorer::embed_life_sheet()` that converts `LifeSheet` struct to normalized text and generates embedding — refs: `job-search-semantic-matching.md`
- [ ] Implement `MatchScorer::score_all_jobs()` as a batch cosine similarity pass over all unscored/stale job embeddings — refs: `job-search-semantic-matching.md`
- [ ] Implement `FeedRanker::compute_feed_score()` combining match_score, ghost_score, recency decay, and feedback multiplier — refs: `job-search-semantic-matching.md`, `agentic-job-matching.md`
- [ ] Implement `SkillInferenceEngine::infer_skills()` with LLM prompt and caching by experience text hash — refs: `job-search-semantic-matching.md`, `agentic-prompt-templates.md`
- [ ] Wire embedding generation + scoring into the ralph discovery loop (post-ingestion step) — refs: `job-search-semantic-matching.md`, `agentic-ralph-subprocess-protocol.md`

### Ghost Job Detection

- [ ] Create `job_postings_history` table tracking `(company_name_normalized, title_normalized, location_normalized, first_seen_at, last_seen_at, repost_count)` — refs: `job-search-ghost-job-detection.md`, `04-sqlite-persistence.md`
- [ ] Implement `GhostDetector::score()` with the 7-signal weighted heuristic and `GhostSignals::explain()` for TUI tooltip — refs: `job-search-ghost-job-detection.md`, `agentic-job-matching.md`, `job-platforms-comparison.md`
- [ ] Embed `pay_transparency_jurisdictions` as a static `HashSet<&str>` in `lazyjob-core/src/discovery/ghost_detection.rs` — refs: `job-search-ghost-job-detection.md`, `job-platforms-comparison.md`
- [ ] Add `ghost_score REAL` and `ghost_overridden BOOLEAN` columns to `jobs` table migration — refs: `job-search-ghost-job-detection.md`
- [ ] Implement `description_vagueness` scorer using a regex-based technical term lexicon — refs: `job-search-ghost-job-detection.md`, `agentic-job-matching.md`
- [ ] Integrate `GhostDetector::score_batch()` into the ralph discovery loop (runs after enrichment, before final SQLite write) — refs: `job-search-ghost-job-detection.md`, `agentic-ralph-subprocess-protocol.md`
- [ ] Add `GhostBadge` display to the job feed TUI widget with tooltip showing `explain()` reasons — refs: `job-search-ghost-job-detection.md`, `09-tui-design-keybindings.md`

### Company Research

- [ ] Define `CompanyRecord` struct and create `companies` table DDL and `CompanyRepository` trait in `lazyjob-core/src/companies/` — refs: `job-search-company-research.md`, `04-sqlite-persistence.md`, `company-pages.md`
- [ ] Implement `CompanyResearcher::infer_tech_stack_from_jobs()` as an offline regex-based extractor using the technical term lexicon — refs: `job-search-company-research.md`, `agentic-job-matching.md`
- [ ] Implement `CompanyResearcher::extract_from_html()` with an LLM extraction prompt targeting `CompanyExtractionResult` — refs: `job-search-company-research.md`, `08-cover-letter-generation.md`, `agentic-prompt-templates.md`
- [ ] Implement `CompanyResearcher::enrich()` that fetches the company website, runs extraction, merges with job-description inference, and upserts to `CompanyRepository` — refs: `job-search-company-research.md`, `05-job-discovery-layer.md`
- [ ] Wire `CompanyResearcher::enrich()` into the ralph discovery loop (triggered async after job ingestion for new companies) — refs: `job-search-company-research.md`, `agentic-ralph-subprocess-protocol.md`
- [ ] Add a `CompanyView` panel to the TUI showing `CompanyRecord` fields with staleness indicators and a manual refresh keybind — refs: `job-search-company-research.md`, `09-tui-design-keybindings.md`
- [ ] Add `list_stale()` to the ralph daily refresh loop to re-enrich companies last updated > 7 days ago — refs: `job-search-company-research.md`, `agentic-ralph-subprocess-protocol.md`

---

## Phase 7: Platform Integrations

### ATS Open APIs

- [ ] Define `PlatformClient` trait and `DiscoveredJob` struct in `lazyjob-core/src/platforms/traits.rs` — include `RawJob`, `ApplicationSubmission`, `ApplicationResponse` types — refs: `platform-ats-open-apis.md`, `11-platform-api-integrations.md`
- [ ] Implement `GreenhouseClient` in `lazyjob-core/src/platforms/greenhouse.rs` — fetch_jobs, fetch_job, submit_application, job normalization with HTML stripping — refs: `platform-ats-open-apis.md`
- [ ] Implement `LeverClient` in `lazyjob-core/src/platforms/lever.rs` — fetch_jobs, fetch_job, submit_application, normalization — refs: `platform-ats-open-apis.md`
- [ ] Build `PlatformRegistry` in `lazyjob-core/src/platforms/mod.rs` with `register`, `client`, `names` methods — refs: `platform-ats-open-apis.md`
- [ ] Add `RateLimiter` with `LazyLock` singleton in `lazyjob-core/src/platforms/rate_limiter.rs`, apply to all client fetch calls — refs: `platform-ats-open-apis.md`
- [ ] Add `[platforms.greenhouse]` and `[platforms.lever]` TOML config sections, parse in `lazyjob-core/src/config.rs` — refs: `platform-ats-open-apis.md`, `architecture-config-management.md`
- [ ] Write integration tests with mock HTTP responses for both Greenhouse and Lever — verify normalization, rate limiting, error handling — refs: `platform-ats-open-apis.md`

### Closed Platforms

- [ ] Implement `AdzunaClient` in `lazyjob-core/src/platforms/adzuna.rs` with `search`, `job_detail` methods and `AdzunaJob` → `DiscoveredJob` normalization — refs: `platform-closed-platforms.md`, `job-platforms-comparison.md`
- [ ] Add `[platforms.adzuna]` and `[platforms.apify]` TOML config sections for Adzuna app_id/app_key and Apify API key — refs: `platform-closed-platforms.md`, `architecture-config-management.md`
- [ ] Build `WorkdayIntegration::fetch_career_page()` in `lazyjob-core/src/platforms/workday.rs` calling Apify Actor API with user-provided career page URL — refs: `platform-closed-platforms.md`
- [ ] Add `Job.source_quality` field (`"api" | "scraped" | "aggregated"`) to the jobs table to flag scraped/aggregated sources for ghost detection weighting — refs: `platform-closed-platforms.md`, `job-search-ghost-job-detection.md`
- [ ] Implement `JobRepository::import_linkedin_url()` in `lazyjob-core/src/platforms/manual.rs` — stores LinkedIn URL as a manual bookmark, no fetch attempted — refs: `platform-closed-platforms.md`
- [ ] Write TUI flow for Apify Workday scrape: user pastes Workday URL → Apify fetch → TUI preview table → user selects which jobs to save → save to repository — refs: `platform-closed-platforms.md`, `architecture-tui-skeleton.md`
- [ ] Add `source_quality` to ghost detection scoring formula: scraped jobs get +0.1 additional ghost_score weight since quality signals are weaker — refs: `platform-closed-platforms.md`, `job-search-ghost-job-detection.md`

### Aggregation & Deduplication

- [ ] Implement `DedupEngine::deduplicate()` in `lazyjob-core/src/discovery/deduplication.rs` with two-tier dedup (exact on source+source_id, fuzzy on normalized key) — refs: `platform-aggregation-deduplication.md`
- [ ] Implement `CompanyResolver::normalize()` in `lazyjob-core/src/discovery/normalizers.rs` with alias map and common suffix stripping — refs: `platform-aggregation-deduplication.md`
- [ ] Implement `TitleNormalizer::normalize()` with stop-word list, and `normalize_location()` for fuzzy location matching — refs: `platform-aggregation-deduplication.md`
- [ ] Build `DiscoveryService::run_discovery()` in `lazyjob-core/src/discovery/aggregation.rs` fetching from all enabled platforms concurrently, normalizing, deduplicating, and enriching — refs: `platform-aggregation-deduplication.md`, `job-search-discovery-engine.md`
- [ ] Add `JobRepository::insert_batch()` method for efficient bulk inserts during Ralph discovery loops — refs: `platform-aggregation-deduplication.md`, `architecture-sqlite-persistence.md`
- [ ] Create `duplicate_log` table DDL, track dedup events during `DiscoveryService::run_discovery()`, add `lazyjob stats --dupes` CLI command — refs: `platform-aggregation-deduplication.md`

---

## Phase 8: Application Tracking

- [ ] Define `ApplicationStage` enum with `can_transition_to` in `lazyjob-core/src/application/stage.rs` — refs: `application-state-machine.md`, `10-application-workflow.md`
- [ ] Define `Application`, `StageTransition`, `Interview`, `Offer` structs in `lazyjob-core/src/application/model.rs` — refs: `application-state-machine.md`
- [ ] Write `002_applications.sql` migration with `applications`, `application_transitions`, `application_contacts`, `interviews`, `offers` tables — refs: `application-state-machine.md`
- [ ] Implement `SqliteApplicationRepository` in `lazyjob-core/src/application/sqlite.rs` with transition validation enforced at `update_stage` — refs: `application-state-machine.md`
- [ ] Add `ApplicationRepository` trait to `lazyjob-core/src/application/repository.rs` — refs: `application-state-machine.md`
- [ ] Add `ApplicationFilter` struct supporting stage/job_id/since/active_only filters — refs: `application-state-machine.md`
- [ ] Expose `Application` module from `lazyjob-core/src/lib.rs` — refs: `application-state-machine.md`

### Workflow Actions

- [ ] Implement `ApplyWorkflow::execute` and `check_duplicate` in `lazyjob-core/src/application/workflows.rs` — refs: `application-workflow-actions.md`, `10-application-workflow.md`
- [ ] Implement `MoveStageWorkflow::execute` with pre/post side-effect hooks in `lazyjob-core/src/application/workflows.rs` — refs: `application-workflow-actions.md`
- [ ] Implement `ScheduleInterviewWorkflow::execute` in `lazyjob-core/src/application/workflows.rs` — refs: `application-workflow-actions.md`
- [ ] Implement `LogContactWorkflow::execute` in `lazyjob-core/src/application/workflows.rs` — refs: `application-workflow-actions.md`
- [ ] Implement `ReminderService` and `SqliteReminderRepository` in `lazyjob-core/src/application/reminders.rs` — refs: `application-workflow-actions.md`
- [ ] Add `WorkflowEvent` enum and tokio broadcast channel wiring in `lazyjob-core/src/application/events.rs` — refs: `application-workflow-actions.md`
- [ ] Add `reminders` table to `lazyjob-core/migrations/002_applications.sql` — refs: `application-workflow-actions.md`
- [ ] Add ghost score check call in `ApplyWorkflow` (delegates to `GhostDetector` from job-search domain) — refs: `application-workflow-actions.md`, `job-search-ghost-job-detection.md`
- [ ] Build TUI apply confirmation dialog in `lazyjob-tui/src/views/apply_confirm.rs` — shows job title, company, resume version, cover letter status, ghost score warning if applicable — refs: `application-workflow-actions.md`, `architecture-tui-skeleton.md`
- [ ] Build TUI stage transition dialog in `lazyjob-tui/src/views/stage_transition.rs` — shows current → next stage, optional reason field, confirm/cancel — refs: `application-workflow-actions.md`, `architecture-tui-skeleton.md`

### Pipeline Metrics & Digest

- [ ] Implement `PipelineMetrics` struct and `MetricsService::compute` in `lazyjob-core/src/application/metrics.rs` — refs: `application-pipeline-metrics.md`, `10-application-workflow.md`
- [ ] Implement `MetricsService::list_stale` with configurable threshold in `lazyjob-core/src/application/metrics.rs` — refs: `application-pipeline-metrics.md`
- [ ] Implement `MetricsService::list_action_required` returning typed `ActionItem` variants in `lazyjob-core/src/application/metrics.rs` — refs: `application-pipeline-metrics.md`
- [ ] Implement `DigestService::generate_daily_digest` and `should_show_today` in `lazyjob-core/src/application/digest.rs` — refs: `application-pipeline-metrics.md`
- [ ] Implement `ReminderPoller` tokio background task in `lazyjob-core/src/application/reminders.rs` — refs: `application-pipeline-metrics.md`

---

## Phase 9: Profile & Resume

### Resume Tailoring

- [ ] Implement `JobDescriptionAnalysis::parse` in `lazyjob-core/src/resume/jd_parser.rs` — LLM extracts structured requirements from raw JD text, falls back to TF-IDF on LLM error — refs: `profile-resume-tailoring.md`, `07-resume-tailoring-pipeline.md`, `resume-optimization.md`
- [ ] Implement `GapAnalysis::compute` in `lazyjob-core/src/resume/gap_analysis.rs` — pure Rust comparison of LifeSheet skills against JD requirements, producing `FabricationLevel` per missing item — refs: `profile-resume-tailoring.md`, `skills-endorsements.md`
- [ ] Implement `FabricationLevel` enum and `FabricationReport::generate` in `lazyjob-core/src/resume/fabrication.rs` — uses `is_grounded_claim` from LifeSheet module as the ground truth check — refs: `profile-resume-tailoring.md`, `agentic-prompt-templates.md`
- [ ] Implement `ResumeContent::draft` in `lazyjob-core/src/resume/drafter.rs` — LLM rewrites bullets with JD keywords, generates targeted summary using style examples extracted from LifeSheet — refs: `profile-resume-tailoring.md`
- [ ] Implement `generate_resume_docx` in `lazyjob-core/src/resume/docx_generator.rs` using `docx-rs` — single-column ATS-safe format with all sections — refs: `profile-resume-tailoring.md`, `07-resume-tailoring-pipeline.md`
- [ ] Implement `SqliteResumeVersionRepository` in `lazyjob-core/src/resume/sqlite.rs` with `save`, `get`, `list_for_job` methods — refs: `profile-resume-tailoring.md`
- [ ] Wire `ResumeTailor::tailor` in `lazyjob-core/src/resume/mod.rs` composing all 6 pipeline stages, returning `TailoredResume` with fabrication report for TUI review step — refs: `profile-resume-tailoring.md`
- [ ] Add `resume_versions` table to `lazyjob-core/migrations/002_applications.sql` with FK to `jobs.id` and `applications.id` — refs: `profile-resume-tailoring.md`

### Cover Letter Generation

- [ ] Implement `CoverLetterService::generate` in `lazyjob-core/src/cover_letter/mod.rs` — orchestrates CompanyRepository lookup, LifeSheet experience selection, template selection, and LLM generation — refs: `profile-cover-letter-generation.md`, `08-cover-letter-generation.md`
- [ ] Implement three prompt templates (StandardProfessional, ProblemSolution, CareerChanger) in `lazyjob-core/src/cover_letter/templates.rs` with placeholder substitution from company research and LifeSheet data — refs: `profile-cover-letter-generation.md`
- [ ] Implement tone and template auto-selection heuristics in `lazyjob-core/src/cover_letter/selector.rs` using CompanyRecord.culture_signals and LifeSheet.goals — refs: `profile-cover-letter-generation.md`
- [ ] Implement cover letter fabrication checker in `lazyjob-core/src/cover_letter/fabrication.rs` — extract quantified claims from generated text, verify each against LifeSheet achievement metrics — refs: `profile-cover-letter-generation.md`, `agentic-prompt-templates.md`
- [ ] Implement `SqliteCoverLetterVersionRepository` in `lazyjob-core/src/cover_letter/sqlite.rs` with `save`, `get`, `list_for_job` — refs: `profile-cover-letter-generation.md`
- [ ] Implement DOCX export for cover letters via `docx-rs` in `lazyjob-core/src/cover_letter/docx.rs` — single-page letter format with name/date header — refs: `profile-cover-letter-generation.md`
- [ ] Build TUI cover letter review view in `lazyjob-tui/src/views/cover_letter_review.rs` — left pane editable Markdown draft, right pane metadata panel, confirm/regenerate/export actions — refs: `profile-cover-letter-generation.md`, `architecture-tui-skeleton.md`

### Skills Gap Analysis

- [ ] Create `SkillNormalizer` in `lazyjob-core/src/lexicon/skill_normalizer.rs` — lowercase/strip punctuation + alias lookup from embedded alias table seeded from ESCO aliases — refs: `profile-skills-gap-analysis.md`, `skills-endorsements.md`
- [ ] Implement `UserSkillExtractor` in `lazyjob-core/src/gap_analysis/extractor.rs` — pulls explicit skills from SQLite + runs regex lexicon over experience/achievement text — refs: `profile-skills-gap-analysis.md`
- [ ] Implement `GapMatrix::compute` in `lazyjob-core/src/gap_analysis/matrix.rs` — computes frequency weights per JD skill, applies severity thresholds, produces sorted `Vec<SkillGap>` — refs: `profile-skills-gap-analysis.md`
- [ ] Implement `GapAnalysisService::find_transferable_skills` in `lazyjob-core/src/gap_analysis/transfer.rs` — LLM prompt that maps source experience text to target skill vocabulary, returns `Vec<TransferableSkill>` for user review — refs: `profile-skills-gap-analysis.md`, `agentic-prompt-templates.md`
- [ ] Add `gap_analysis_cache` table to SQLite migrations and implement cache read/write in `GapAnalysisService` — refs: `profile-skills-gap-analysis.md`
- [ ] Build TUI skills heat map widget in `lazyjob-tui/src/views/gap_analysis.rs` — skill × frequency matrix with color coding, interactive drill-down to job IDs — refs: `profile-skills-gap-analysis.md`, `architecture-tui-skeleton.md`
- [ ] Add static learning resources YAML at `lazyjob-core/assets/learning_resources.yaml` and integrate lookup into `GapReport.learning_resource` field — refs: `profile-skills-gap-analysis.md`

---

## Phase 10: Networking & Referrals

### Connection Mapping

- [ ] Add `previous_companies_json`, `schools_json`, `relationship_notes`, `last_contacted_at`, `contact_source` columns to `profile_contacts` DDL in `lazyjob-core/src/db/schema.sql` — refs: `networking-connection-mapping.md`, `03-life-sheet-data-model.md`
- [ ] Implement `ConnectionMapper::import_linkedin_csv` with column-name-based CSV parsing and upsert-by-email logic in `lazyjob-core/src/networking/csv_import.rs` — refs: `networking-connection-mapping.md`, `networking-referrals-agentic.md`
- [ ] Implement `ConnectionMapper::warm_paths_for_job` with company name normalization matching against `CompanyRepository` — refs: `networking-connection-mapping.md`
- [ ] Implement `ConnectionTier` computation and `SuggestedApproach` classification rules — refs: `networking-connection-mapping.md`
- [ ] Add `ContactRepository` trait and `SqliteContactRepository` impl in `lazyjob-core/src/networking/` — refs: `networking-connection-mapping.md`
- [ ] Add Warm Paths panel to job detail TUI view (`lazyjob-tui/src/views/job_detail/warm_paths.rs`) with tier-badged contact list and `n` keybinding — refs: `networking-connection-mapping.md`, `architecture-tui-skeleton.md`
- [ ] Write unit tests for company name normalization edge cases and `ConnectionTier` scoring logic — refs: `networking-connection-mapping.md`

### Outreach Drafting

- [ ] Implement `SharedContext` computation in `lazyjob-core/src/networking/context.rs` using pure LifeSheet ↔ ProfileContact structural comparison (no LLM) — refs: `networking-outreach-drafting.md`, `networking-referrals-agentic.md`
- [ ] Write outreach prompt template at `lazyjob-llm/src/prompts/networking_outreach.md` with anti-fabrication rules, tone variants, and medium-specific length instructions — refs: `networking-outreach-drafting.md`, `agentic-prompt-templates.md`
- [ ] Implement `LlmOutreachDraftingService::draft` in `lazyjob-core/src/networking/outreach.rs` with three-phase pipeline (context assembly → LLM → length/fabrication validation) — refs: `networking-outreach-drafting.md`
- [ ] Implement medium-specific char/word count enforcement and `medium_limit_ok` flag; for `LinkedInConnectionNote`, hard-clip at 300 chars with ellipsis warning — refs: `networking-outreach-drafting.md`
- [ ] Add `outreach_status`, `outreach_sent_at`, `outreach_responded_at`, `last_draft_text` columns to `profile_contacts` DDL — refs: `networking-outreach-drafting.md`
- [ ] Add TUI outreach draft view (`lazyjob-tui/src/views/networking/outreach_draft.rs`): show draft text in editable textarea, char count, copy-to-clipboard action (`y`), mark-sent action (`s`) — refs: `networking-outreach-drafting.md`, `architecture-tui-skeleton.md`
- [ ] Add `OutreachStatus` transitions to `ContactRepository`: `mark_sent(contact_id)`, `mark_responded(contact_id)`, `mark_no_response(contact_id)` — refs: `networking-outreach-drafting.md`

### Referral Management

- [ ] Add `relationship_stage`, `interaction_count`, `follow_up_exhausted`, `reminder_count_this_month` columns to `profile_contacts` DDL — refs: `networking-referral-management.md`, `03-life-sheet-data-model.md`
- [ ] Create `referral_asks` table DDL with `(contact_id, job_id)` unique constraint — refs: `networking-referral-management.md`
- [ ] Implement `ReferralReadinessChecker` in `lazyjob-core/src/networking/referral_readiness.rs` with all 5 readiness criteria and `GhostDetector` integration — refs: `networking-referral-management.md`, `job-search-ghost-job-detection.md`
- [ ] Implement `NetworkingReminderPoller` as a tokio background task in `lazyjob-ralph/src/networking_poller.rs` with configurable interval and 2-reminder anti-spam cap — refs: `networking-referral-management.md`
- [ ] Wire `WorkflowEvent::NetworkingReminderDue` into the TUI's broadcast channel subscriber (same channel as `ReminderPoller` from `application-workflow-actions.md`) — refs: `networking-referral-management.md`, `architecture-tui-skeleton.md`
- [ ] Wire `PostTransitionSuggestion::UpdateReferralOutcome` dispatch in `lazyjob-ralph/src/dispatch.rs` when application stage transitions to `Offered` or `Rejected` — refs: `networking-referral-management.md`, `agentic-ralph-orchestration.md`
- [ ] Add networking dashboard TUI view (`lazyjob-tui/src/views/networking/dashboard.rs`): contacts grouped by company with stage badge, reminder count, days-since-contact indicator — refs: `networking-referral-management.md`, `architecture-tui-skeleton.md`
- [ ] Add interaction logging action to TUI contact detail view: `l` to log interaction (date + note), updates `interaction_count`, `last_contacted_at`, advances stage — refs: `networking-referral-management.md`, `architecture-tui-skeleton.md`

---

## Phase 11: Interview Prep & Salary Negotiation

### Interview Prep

- [ ] Define `InterviewType`, `QuestionCategory`, `InterviewQuestion`, `PrepContext`, `InterviewPrepSession` types in `lazyjob-core/src/interview/mod.rs` — refs: `interview-prep-question-generation.md`, `interview-prep-agentic.md`
- [ ] Implement `PrepContextBuilder` in `lazyjob-core/src/interview/context.rs` that assembles verified context from `JobListing`, `LifeSheet`, and `CompanyRecord` without LLM — refs: `interview-prep-question-generation.md`, `job-search-company-research.md`, `profile-life-sheet-data-model.md`
- [ ] Implement `InterviewPrepService::generate_prep_session` using `LlmProvider::complete` with a structured JSON schema prompt; include mix ratios per `InterviewType` — refs: `interview-prep-question-generation.md`, `agentic-prompt-templates.md`
- [ ] Implement `map_stories_to_behavioral_questions` to link generated behavioral questions to matching `LifeSheet.work_experience` entries by keyword overlap — refs: `interview-prep-question-generation.md`, `profile-life-sheet-data-model.md`
- [ ] Create `interview_prep_sessions` table migration in `lazyjob-core/src/db/migrations/` — refs: `interview-prep-question-generation.md`
- [ ] Implement `InterviewPrepRepository` trait with `save_session`, `get_sessions_for_application`, and `get_latest_session_for_application` methods — refs: `interview-prep-question-generation.md`
- [ ] Wire `PostTransitionSuggestion::GenerateInterviewPrep` from `application-workflow-actions.md` to dispatch a ralph loop — refs: `interview-prep-question-generation.md`, `agentic-ralph-orchestration.md`
- [ ] Add TUI view: "Interview Prep" panel accessible from the application detail view, showing the latest `InterviewPrepSession` with questions organized by category — refs: `interview-prep-question-generation.md`, `architecture-tui-skeleton.md`

### Mock Interview Loop

- [ ] Define `QuestionFeedback`, `MockResponse`, `MockInterviewSession`, `SessionScore` types in `lazyjob-core/src/interview/mock_session.rs` — refs: `interview-prep-mock-loop.md`, `interview-prep-agentic.md`
- [ ] Implement `MockInterviewLoop` in `lazyjob-ralph/src/loops/mock_interview.rs` using the IPC protocol (emit `MockQuestion`, receive stdin response, emit `MockFeedback`) — refs: `interview-prep-mock-loop.md`, `agentic-ralph-subprocess-protocol.md`
- [ ] Implement evaluation prompt templates in `lazyjob-llm/src/prompts/interview_eval.rs` with per-category rubrics and fabrication detection — refs: `interview-prep-mock-loop.md`, `agentic-prompt-templates.md`
- [ ] Create `mock_interview_sessions` and `mock_interview_responses` migration in `lazyjob-core/src/db/migrations/` — refs: `interview-prep-mock-loop.md`
- [ ] Implement `MockInterviewRepository` with `save_session`, `get_sessions_for_application`, `get_score_trend` methods — refs: `interview-prep-mock-loop.md`
- [ ] Add TUI mock interview view: sequential Q→A→feedback panels using the ralph event stream; include the anti-overconfidence disclaimer in the session header — refs: `interview-prep-mock-loop.md`, `architecture-tui-skeleton.md`
- [ ] Add progress trend panel in application detail view: query `MockInterviewRepository::get_score_trend` and display per-category score history across sessions — refs: `interview-prep-mock-loop.md`, `architecture-tui-skeleton.md`

### Salary Market Intelligence

- [ ] Define `OfferDetails`, `EquityGrant`, `EquityType`, `CompanyStage`, `TotalCompBreakdown`, `OfferEvaluation`, `MarketDataPoint` types in `lazyjob-core/src/salary/model.rs` — refs: `salary-market-intelligence.md`, `salary-negotiation-offers.md`
- [ ] Implement `SalaryIntelligenceService::compute_total_comp` with equity risk-adjustment table and RSU-vs-options distinction — refs: `salary-market-intelligence.md`
- [ ] Implement `is_pay_transparent_jurisdiction` using `PAY_TRANSPARENT_JURISDICTIONS` static set in `lazyjob-core/src/salary/jurisdictions.rs`; ensure this module is shared with `job-search-ghost-job-detection.md`'s `salary_absent_in_transparency_state` signal — refs: `salary-market-intelligence.md`, `job-search-ghost-job-detection.md`
- [ ] Implement `LevelsFyiParser::parse_paste` for parsing user-pasted salary table text into `MarketDataPoint` records — refs: `salary-market-intelligence.md`
- [ ] Create SQLite schema migration for `offer_details`, `market_data_references`, `salary_references` tables; extend `OfferRepository` — refs: `salary-market-intelligence.md`, `application-state-machine.md`
- [ ] Implement H1B LCA data importer (`lazyjob-core/src/salary/h1b_importer.rs`): download annual DOL LCA CSV, parse, upsert into `market_data_references` table; run as a one-time setup step — refs: `salary-market-intelligence.md`
- [ ] Add TUI offer evaluation view: form for entering offer details, auto-computed `TotalCompBreakdown` displayed inline, competing offers side-by-side comparison panel — refs: `salary-market-intelligence.md`, `architecture-tui-skeleton.md`
- [ ] Wire `PostTransitionSuggestion::RunSalaryComparison` from `application-workflow-actions.md` to open the offer entry form in the TUI when an application transitions to `Offer` stage — refs: `salary-market-intelligence.md`, `application-workflow-actions.md`

### Counter-Offer Drafting

- [ ] Define `CounterOfferRequest`, `CounterOfferDraft`, `NegotiationHistory`, `NegotiationOutcome`, `NegotiationRound` types in `lazyjob-core/src/salary/counter_offer.rs` and `lazyjob-core/src/salary/outcome.rs` — refs: `salary-counter-offer-drafting.md`, `salary-negotiation-offers.md`
- [ ] Implement `CounterOfferDraftService::generate_draft` using `LlmProvider::complete` with a strict grounding prompt: include verified comp figures, never generate competing offer references unless `competing_offer_annualized` is present — refs: `salary-counter-offer-drafting.md`, `agentic-prompt-templates.md`, `agentic-llm-provider-abstraction.md`
- [ ] Implement negotiation prompt template in `lazyjob-llm/src/prompts/salary_negotiation.rs` with tone variants, priority ordering, and per-company-stage negotiable components list — refs: `salary-counter-offer-drafting.md`, `agentic-prompt-templates.md`
- [ ] Create `counter_offer_drafts` and `negotiation_history` schema migration in `lazyjob-core/src/db/migrations/` — refs: `salary-counter-offer-drafting.md`
- [ ] Implement `CounterOfferDraftService::record_outcome` to close the negotiation loop: save `NegotiationHistory`, compute `comp_delta`, update `application_contacts` with final hiring-manager contact if provided — refs: `salary-counter-offer-drafting.md`
- [ ] Add TUI counter-offer view: `CounterOfferRequest` form (tone selector, priorities, target comp), draft display panel with `[DRAFT - NOT SENT]` header and copy-to-clipboard action, talking points accordion — refs: `salary-counter-offer-drafting.md`, `architecture-tui-skeleton.md`
- [ ] Add negotiation outcome recording UI: after draft is viewed, prompt user to record outcome when they return to the application detail view; wire outcome to `PostTransitionSuggestion::RunSalaryComparison` completing the loop — refs: `salary-counter-offer-drafting.md`, `application-workflow-actions.md`

---

## Phase 12: SaaS & Monetization

### Migration Path

- [ ] Extract all repository traits in `lazyjob-core/src/persistence/` to a `Repository` trait hierarchy (JobRepository, ApplicationRepository, etc.) — no implementation changes, just trait extraction — refs: `saas-migration-path.md`, `18-saas-migration-path.md`
- [ ] Add `SqliteJobRepository`, `SqliteApplicationRepository`, etc. implementations in the same modules — refs: `saas-migration-path.md`
- [ ] Add `TenantRepository`, `UserRepository` in `lazyjob-core/src/persistence/` for SaaS multi-tenancy (Phase 3) — refs: `saas-migration-path.md`
- [ ] Create `lazyjob-sync/` crate scaffold with `SyncProtocol`, `SyncState`, `SyncOperation` types — refs: `saas-migration-path.md`
- [ ] Add Supabase client crate (`lazyjob-sync`) to workspace with `Repository` implementations for PostgreSQL — refs: `saas-migration-path.md`
- [ ] Add `tenant_id` column to all core tables in a new migration (`010_add_tenant_id.sql`) — refs: `saas-migration-path.md`
- [ ] Implement sync deduplication in `SyncProtocol`: don't re-upload unchanged rows (compare `updated_at`) — refs: `saas-migration-path.md`
- [ ] Implement `offer_details` and `token_usage_log` to `NEVER_SYNC_TABLES` list in `lazyjob-sync/src/never_sync.rs` — refs: `saas-migration-path.md`, `salary-market-intelligence.md`
- [ ] Write Supabase migration files in `lazyjob-sync/migrations/` for PostgreSQL schema creation — refs: `saas-migration-path.md`

### LLM Proxy

- [ ] Create `lazyjob-proxy/` crate scaffold: `lazyjob-proxy/src/lib.rs`, `Cargo.toml`, endpoint handler — refs: `saas-llm-proxy.md`, `02-llm-provider-abstraction.md`
- [ ] Implement `LoomProxyProvider` in `lazyjob-llm/src/loom_proxy.rs` that implements `LlmProvider` and routes HTTP requests to the proxy server — refs: `saas-llm-proxy.md`
- [ ] Add `proxy_url` field to `[llm]` config section in `lazyjob.toml` — refs: `saas-llm-proxy.md`, `architecture-config-management.md`
- [ ] Implement `LlmBuilder::build()` with `LoomProxyProvider` variant when `proxy_url` is set — refs: `saas-llm-proxy.md`, `agentic-llm-provider-abstraction.md`
- [ ] Implement `TierGating` in `lazyjob-proxy/src/tier.rs` with `SubscriptionTier`, `max_tokens_per_month()`, `allowed_models()` — refs: `saas-llm-proxy.md`
- [ ] Implement `UsageTracker` in `lazyjob-proxy/src/usage.rs` with `record()`, `get_monthly_usage()`, `calculate_cost()` — refs: `saas-llm-proxy.md`
- [ ] Implement `LlmRouter::cheapest_capable()` for automatic model selection based on message complexity — refs: `saas-llm-proxy.md`
- [ ] Add SSE streaming response type to `LoomProxyProvider` for real-time token streaming to TUI — refs: `saas-llm-proxy.md`, `agentic-llm-provider-abstraction.md`
- [ ] Implement `token_usage_log` billing query endpoint (`GET /api/usage/:user_id`) for user-facing usage dashboard — refs: `saas-llm-proxy.md`
- [ ] Add `cost_microdollars` column to `token_usage_log` and wire `calculate_cost()` from both SaaS and local LLM calls — refs: `saas-llm-proxy.md`, `agentic-llm-provider-abstraction.md`

### Pricing Strategy

- [ ] Define `SubscriptionTier` enum and `FeatureGate` struct in `lazyjob-core/src/billing/tier.rs` — refs: `saas-pricing-strategy.md`, `premium-monetization.md`, `19-competitor-analysis.md`
- [ ] Implement `MonthlyCounters` with reset logic (check `created_at` vs current month, reset if new month) — refs: `saas-pricing-strategy.md`
- [ ] Implement `FeatureGate::check()` and `FeatureGate::record_usage()` for all gated features — refs: `saas-pricing-strategy.md`
- [ ] Add `[billing]` section to `lazyjob.toml`: `tier = "free" | "pro" | "team"`, subscription key for SaaS mode — refs: `saas-pricing-strategy.md`, `architecture-config-management.md`
- [ ] Implement `FeatureLimitExceeded` error variant in `lazyjob-core/src/error.rs` with upgrade URL — refs: `saas-pricing-strategy.md`
- [ ] Wire `FeatureGate::check()` calls in all ralph loop entry points (discovery, tailoring, cover letter, interview prep) — refs: `saas-pricing-strategy.md`
- [ ] Add in-app upgrade prompt component in TUI — appears once when user hits 80% of free tier limit — refs: `saas-pricing-strategy.md`, `architecture-tui-skeleton.md`
- [ ] Implement `lazyjob billing usage` CLI command showing current month usage and limits — refs: `saas-pricing-strategy.md`
- [ ] Add `credit_back` events to usage tracking for per-feature refund tracking — refs: `saas-pricing-strategy.md`

---

## Spec Count Summary

| Phase | Domain | Spec Files | Implementation Tasks |
|-------|--------|------------|----------------------|
| 1 | Crate/SQLite/LifeSheet | 3 specs | ~30 tasks |
| 2 | TUI/Config/Privacy | 3 specs | ~30 tasks |
| 3 | LLM Provider | 1 spec | ~8 tasks |
| 4 | Ralph Protocol/Orchestration | 2 specs | ~14 tasks |
| 5 | Prompt Templates | 1 spec | ~8 tasks |
| 6 | Job Search/Discovery | 4 specs | ~28 tasks |
| 7 | Platform Integrations | 3 specs | ~24 tasks |
| 8 | Application Tracking | 3 specs | ~21 tasks |
| 9 | Profile/Resume | 3 specs | ~26 tasks |
| 10 | Networking/Referrals | 3 specs | ~26 tasks |
| 11 | Interview/Salary | 4 specs | ~30 tasks |
| 12 | SaaS/Monetization | 3 specs | ~30 tasks |
| **Total** | | **33 specs** | **~275 tasks** |
