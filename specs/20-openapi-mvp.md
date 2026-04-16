# OpenAPI MVP: Build Plan

## Status

Stable

## Purpose

This spec synthesizes all 19 research specs into a single, actionable MVP build plan. It defines what to build, in what order, how to prioritize, and what to defer. It is the canonical reference for the first production release of LazyJob.

---

## Executive Summary

**LazyJob MVP**: A lazygit-style terminal UI for job search management, powered by autonomous AI agent loops (Ralph) underneath. The core value proposition is **automation that saves time** on the repetitive parts of job searching, combined with **organization** that gives visibility into the overall pipeline.

**Target User**: A tech professional actively searching for a new role. Comfortable with terminal tools. Applying to 10-100+ positions simultaneously. Time-poor and emotionally stressed.

**MVP Delivery**: 12-week single-engineer build, structured in 6 phases.

---

## Core Architecture

### Crate Layout

```
lazyjob/
├── lazyjob-core/           # Domain models, persistence, discovery
├── lazyjob-llm/           # LLM provider abstraction + prompts
├── lazyjob-ralph/         # Ralph subprocess IPC
├── lazyjob-tui/           # Terminal UI (ratatui)
├── lazyjob-cli/           # Binary entry point
└── lazyjob-macros/        # Procedural macros (future)
```

**Dependency Graph**:
```
lazyjob-cli
    └── lazyjob-tui
            ├── lazyjob-ralph
            │       ├── lazyjob-llm
            │       │       └── lazyjob-core
            │       └── lazyjob-core
            └── lazyjob-core
```

### Technology Stack

| Component | Technology | Rationale |
|-----------|------------|-----------|
| Language | Rust | Performance, safety, laziness |
| UI | ratatui + crossterm | Mature TUI ecosystem, inspired by lazygit |
| Database | SQLite (rusqlite) | Local-first, zero-config, WAL mode |
| Async | tokio | Async runtime for LLM calls, subprocesses |
| Serialization | serde + serde_yaml | Life sheet YAML, JSON for IPC |
| LLM | async-openai + custom trait | OpenAI primary, Ollama fallback |
| CLI | clap | Ergonomic argument parsing |
| Secrets | keyring-rs | OS-native credential storage |
| Tracing | tracing + tracing-subscriber | Structured logging |

### Rust Patterns (from `rust-patterns.md`)

1. **lib.rs + thin main.rs**: All logic in library crates; `main.rs` is orchestration only
2. **Newtype wrappers**: Parse, don't validate — `ApiKey`, `JobId`, `ApplicationId`
3. **secrecy::Secret**: API keys wrapped so they never leak in logs
4. **thiserror + anyhow**: `thiserror` for public errors callers match on; `anyhow` for internal context chaining
5. **Feature-gated deps**: `reqwest` with `rustls-tls` not OpenSSL
6. **Facade modules**: Parent `mod.rs` re-exports only the public surface

---

## The Ralph Loop Architecture

Ralph loops are the defining innovation of LazyJob. They are autonomous agent subprocesses that run background tasks — job discovery, company research, resume tailoring — and communicate with the TUI via newline-delimited JSON over stdio.

### Ralph Protocol

**TUI -> Ralph (stdin)**:
```json
{"type": "start", "loop": "job_discovery", "params": {"companies": ["stripe", "airbnb"]}}
{"type": "cancel"}
{"type": "pause"}
{"type": "resume"}
```

**Ralph -> TUI (stdout)**:
```json
{"type": "status", "phase": "fetching", "progress": 0.3, "message": "Fetching from Stripe..."}
{"type": "results", "loop": "job_discovery", "data": {"jobs": [...], "new_count": 5}}
{"type": "done", "success": true}
{"type": "error", "code": "rate_limited", "message": "Greenhouse rate limited"}
```

### Ralph Loop Types (MVP Priority)

| Loop | Priority | Description |
|------|----------|-------------|
| JobDiscovery | P0 | Fetch jobs from Greenhouse/Lever for configured companies |
| CompanyResearch | P1 | Deep research on a specific company from public sources |
| ResumeTailor | P1 | Generate tailored resume DOCX for a specific job |
| CoverLetter | P2 | Generate cover letter (deferred to post-MVP) |
| InterviewPrep | P2 | Generate practice questions (deferred to post-MVP) |
| SalaryNegotiation | P2 | Market data + strategy (deferred to post-MVP) |
| Networking | P2 | Contact finding + outreach drafting (deferred to post-MVP) |

### Ralph Process Manager

The TUI spawns Ralph as a `tokio::process::Command` subprocess. Each loop type is a CLI subcommand. The `RalphProcessManager`:
- Maintains a `HashMap<LoopId, ChildHandle>` of running processes
- Uses `BufReader::lines()` to read stdout as an async stream
- Sends cancellation via `child.kill().await`
- Broadcasts events to TUI views via `tokio::sync::broadcast`
- Persists loop results directly to the shared SQLite (WAL mode)

State survives TUI restart: on startup, the TUI queries SQLite for "in_progress" loops and offers to resume or cancel them.

---

## MVP Scope: What We Build

### P0 — Essential (MVP Core)

These features are the minimum viable product. Without them, the app has no value.

#### TUI Views

1. **Dashboard**: Statistics (total jobs, applications, response rate), recent activity, upcoming reminders
2. **Jobs List**: Filterable, sortable list of all discovered jobs. Status indicators (discovered, interested, applied). CRUD operations.
3. **Job Detail**: Full job info, match score, company info summary, action buttons
4. **Applications Pipeline**: Kanban board with columns (Discovered -> Interested -> Applied -> Phone Screen -> Technical -> On-site -> Offer). Application cards show company, title, last contact.
5. **Contacts**: Contact list with name, role, company, relationship type, quality rating
6. **Settings**: LLM provider configuration (API keys stored in keyring), company discovery config (add/remove by board token), data export (JSON)
7. **Help Overlay**: Full keybinding reference (lazygit-style `?`)

#### Data Layer

8. **SQLite Persistence**: All entities persisted. WAL mode for concurrent TUI + Ralph subprocess access. Migration system.
9. **Life Sheet**: YAML file at `~/.lazyjob/life-sheet.yaml`. User-editable. Contains personal info, experience, education, skills, certifications, preferences, goals.

#### Integrations

10. **Greenhouse API**: Fetch jobs from company job boards via public API
11. **Lever API**: Fetch jobs via public API
12. **LLM Provider Trait**: OpenAI implementation (GPT-4o). Ollama implementation (local fallback). Streaming responses.

#### Automation

13. **Ralph JobDiscovery Loop**: Background process fetches jobs from all configured companies, stores in SQLite, computes match scores
14. **Ralph CompanyResearch Loop**: Fetches company info from public sources, synthesizes with LLM

### P1 — High Value (MVP+)

These are high-impact features that should be included if schedule allows.

15. **Resume Tailoring**: LLM-powered job description analysis, gap analysis, docx-rs DOCX generation, fabrication guardrails
16. **Ralph ResumeTailor Loop**: Background resume tailoring for a specific job
17. **Confirmation Dialogs**: For destructive actions (delete, withdraw), stage transitions
18. **Activity Log**: Tracks all state changes with timestamps

### P2 — Nice to Have (Deferred to Post-MVP)

These are valuable but not MVP-critical.

- Cover letter generation
- Interview prep AI
- Salary negotiation
- Networking automation
- Morning brief notifications
- Cloud sync
- Team collaboration
- Web UI
- Mobile app

---

## What's NOT MVP

Based on competitive analysis and JTBD research, these features are explicitly deferred:

| Feature | Reason to Defer |
|---------|----------------|
| Cover letters | Useful but not core value prop; requires company research loop first |
| Interview prep AI | Valuable but complex; depends on company research being solid |
| Salary negotiation | Requires reliable market data integration; lower immediate impact |
| Networking automation | LinkedIn ToS issues; high complexity for marginal gain |
| Morning brief | Nice to have but not a core JTBD driver |
| Cloud sync | Local-first MVP is the point; sync is a post-MVP SaaS concern |
| Team collaboration | Single-user local tool is the MVP; multi-user adds enormous complexity |
| Web UI | TUI is the differentiator; web is a post-MVP migration decision |

---

## Implementation Phases

### Phase 1: Foundation (Weeks 1-3)

**Goal**: Core data model, SQLite persistence, working TUI shell with static data.

#### Week 1: Project Bootstrap

**Tasks**:
- [ ] Initialize Cargo workspace with members: `lazyjob-core`, `lazyjob-tui`, `lazyjob-cli`
- [ ] Set up `tracing` with `tracing-subscriber` (JSON fmt, RUST_LOG env var)
- [ ] Define core domain models in `lazyjob-core`: `Job`, `Company`, `Application`, `Contact`, `LifeSheet`, `Interview`, `Offer`
- [ ] Implement `ApplicationStage` enum with `can_transition_to()` validation
- [ ] Set up rusqlite with `rusqlite::Connection::open()`, WAL mode pragma
- [ ] Write first migration: `001_initial_schema.sql` (jobs, companies, applications, contacts, interviews, offers, activity_log tables)
- [ ] Implement `JobRepository` with `list()`, `get()`, `insert()`, `update()`, `delete()`
- [ ] Implement basic CLI with clap: `lazyjob-cli` with `jobs list`, `jobs add`, `jobs delete` subcommands

**Deliverables**:
- `cargo build` succeeds
- `lazyjob-cli jobs list` returns empty list (no jobs yet)
- SQLite file created at `~/.lazyjob/lazyjob.db`

#### Week 2: TUI Shell

**Tasks**:
- [ ] Set up ratatui with crossterm backend
- [ ] Implement main layout: Header (3 lines), Content (fill), Status Bar (1 line)
- [ ] Create `App` struct with `new()`, `draw()`, `handle_events()` loop
- [ ] Implement Jobs List view with `List` widget and `ListState`
- [ ] Implement basic keybindings: `j/k` navigation, `q` quit, `?` help
- [ ] Implement Help Overlay (full keybinding table)
- [ ] Wire up CLI to launch TUI: `lazyjob-cli tui` starts the full app

**Deliverables**:
- TUI renders with header, jobs list panel, status bar
- `j/k` moves selection
- `?` shows help overlay
- `q` exits cleanly
- Application state persists across restarts (SQLite)

#### Week 3: Full CRUD + Company/Discovery

**Tasks**:
- [ ] Implement Job CRUD in TUI: add job form, edit job, delete job with confirmation
- [ ] Implement Company CRUD: add company with Greenhouse/Lever board token
- [ ] Implement Job Detail view: shows all job fields, match score placeholder
- [ ] Implement company config file: `~/.lazyjob/config.yaml` with companies list
- [ ] Implement Greenhouse API client (no auth, public board API)
- [ ] Implement Lever API client (no auth, public postings API)
- [ ] Implement `CompanyRegistry` with `discover_company_jobs()`
- [ ] Connect discovery to TUI: "Refresh" button fetches from all companies

**Deliverables**:
- Can add a company by name + board token
- "Refresh" fetches real jobs from Greenhouse/Lever
- Jobs appear in list with title, company, location, posted date
- Job Detail shows full description

### Phase 2: Application Tracking (Weeks 4-5)

**Goal**: Kanban pipeline, application lifecycle, contacts, dashboard metrics.

#### Week 4: Pipeline + Applications

**Tasks**:
- [ ] Implement `Application` entity with stage, history, notes
- [ ] Implement Application state machine transitions with validation
- [ ] Implement Applications Pipeline view (kanban with 9 columns)
- [ ] Implement Application cards showing company, title, last contact, stage age
- [ ] Implement `m` keybinding: move card to next stage (with confirmation dialog)
- [ ] Implement `shift+m`: move to previous stage
- [ ] Implement Application Detail view: shows full app info, interview history, notes
- [ ] Implement Stage History timeline in Application Detail
- [ ] Implement Activity Log: all state changes logged to `activity_log` table

**Deliverables**:
- Kanban board renders with all 9 columns
- Can drag (via keybindings) application cards between columns
- Stage transitions validated by `can_transition_to()`
- Confirmation dialog before destructive stage moves
- Activity log shows history of all changes

#### Week 5: Contacts + Dashboard

**Tasks**:
- [ ] Implement `Contact` entity: name, role, email, linkedin, company, relationship, quality
- [ ] Implement Contacts List view with filtering by company, relationship type
- [ ] Implement Add/Edit Contact modal
- [ ] Implement Dashboard view: statistics panels, recent activity feed, upcoming reminders
- [ ] Implement pipeline metrics: response rate, interview rate, offer rate, stale count
- [ ] Implement Follow-up reminder creation in Application Detail
- [ ] Implement "Actions Required" queue in Dashboard: overdue follow-ups, upcoming interviews, offer deadlines

**Deliverables**:
- Dashboard shows live statistics from SQLite
- Contacts CRUD fully functional
- Recent activity feed shows last 10 events
- "Actions Required" section surfaces urgent items

### Phase 3: LLM Integration (Weeks 6-7)

**Goal**: LLM provider abstraction, company research, embedding-based job matching.

#### Week 6: LLM Provider + Ralph Foundation

**Tasks**:
- [ ] Implement `LLMProvider` trait with `chat()`, `chat_stream()`, `complete()`, `embed()`
- [ ] Implement `OpenAIProvider` using `async-openai`
- [ ] Implement `OllamaProvider` using `ollama-rs`
- [ ] Implement `ProviderRegistry` with `HashMap<String, Arc<dyn LLMProvider>>`
- [ ] Implement `LLMBuilder` for ergonomic setup from config
- [ ] Implement `CredentialManager` using `keyring-rs` for API key storage
- [ ] Implement Ralph process manager: `RalphProcessManager::new()`, `start_loop()`, `cancel_loop()`
- [ ] Implement stdio JSON protocol: `send_status()`, `send_results()`, `send_error()`
- [ ] Implement Ralph event broadcast to TUI views
- [ ] Implement Ralph Panel in TUI: shows active loops, progress bars, history

**Deliverables**:
- Can configure OpenAI API key in Settings (stored in system keyring)
- `cargo test` passes with mock LLM provider
- Ralph Panel shows active loops with progress
- Can cancel running Ralph loops

#### Week 7: Job Matching + Company Research

**Tasks**:
- [ ] Implement text embedding: OpenAI `text-embedding-ada-002` or Ollama `nomic-embed-text`
- [ ] Implement `JobMatcher` with cosine similarity
- [ ] Implement life sheet embedding: combine skills + experience text
- [ ] Implement `find_matching_jobs()` returning top-k matches with scores
- [ ] Implement Match Score display in Job Detail view
- [ ] Implement Ralph CompanyResearch loop: fetch website, LinkedIn, news, synthesize with LLM
- [ ] Store company research in `companies` table (enriched columns)
- [ ] Connect Company Research to Job Detail: "Research Company" button

**Deliverables**:
- Job Detail shows match score (0-100%)
- "Research Company" button starts Ralph loop
- Company info (mission, size, culture signals) visible in Job Detail
- Matching jobs surfaced in Dashboard "Recommended" section

### Phase 4: Resume Tailoring (Weeks 8-9)

**Goal**: AI-powered resume tailoring with fabrication guardrails.

#### Week 8: Resume Pipeline

**Tasks**:
- [ ] Implement `LifeSheetRepository` for YAML loading + SQLite sync
- [ ] Implement `JobDescriptionAnalysis`: LLM parses JD into required skills, nice-to-have, keywords, responsibilities
- [ ] Implement `GapAnalysis`: match user skills to JD requirements, identify missing skills
- [ ] Implement `FabricationGuardrails`: assess fabrication risk level (Safe/Acceptable/Risky/Forbidden)
- [ ] Implement resume content drafting: targeted summary, rewritten bullets with keywords, ordered skills
- [ ] Implement DOCX generation with `docx-rs`: name, contact, summary, experience, skills, education sections
- [ ] Implement Ralph ResumeTailor loop: orchestrates the full pipeline in background
- [ ] Implement "Tailor Resume" button in Job Detail view

**Deliverables**:
- Can generate tailored resume DOCX for any job in the database
- Fabrication warnings shown before export
- TUI shows Ralph loop progress during tailoring
- Tailored resume opens in default DOCX viewer

#### Week 9: Polish + Export

**Tasks**:
- [ ] Implement resume version tracking: store each tailored version against the application
- [ ] Implement Application version selector: which resume version was submitted
- [ ] Implement application submission workflow: confirm resume + cover letter versions before "submitting"
- [ ] Implement data export: full JSON export of all data
- [ ] Implement JSON import: restore from export
- [ ] Implement confirmation for "Applied" stage transition (confirm submission date)
- [ ] Error handling pass: loading states, empty states, error messages throughout

**Deliverables**:
- Resume versions tracked per application
- Data export/import works as a backup mechanism
- All destructive actions have confirmation dialogs
- Application "submit" workflow captures submission date

### Phase 5: Polish + Launch (Weeks 10-12)

**Goal**: Production quality, edge case handling, documentation.

#### Week 10: Edge Cases + Ralph Discovery Loop

**Tasks**:
- [ ] Implement Ralph JobDiscovery loop: for each configured company, fetch from Greenhouse/Lever, deduplicate, store, embed
- [ ] Implement deduplication: compare (title, company, location) hash to detect duplicates
- [ ] Implement polling interval config: how often to refresh
- [ ] Implement new job notification: Ralph loop reports "N new jobs found"
- [ ] Implement empty state views: "No jobs yet", "No applications", "No contacts"
- [ ] Implement loading states: spinners and progress during async operations
- [ ] Implement offline handling: graceful degradation when APIs unreachable

**Deliverables**:
- Ralph JobDiscovery loop works end-to-end: configured companies -> new jobs in SQLite
- New jobs appear in TUI after background refresh
- Empty states guide user to first action
- Offline mode shows cached data with staleness indicator

#### Week 11: Testing + Bug Fixes

**Tasks**:
- [ ] Write integration tests: repository CRUD, state machine transitions, LLM provider mock
- [ ] Write unit tests: gap analysis, cosine similarity, fabrication guardrails
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --all` applied
- [ ] Manual testing: full user journey from "no data" to "applied to 5 jobs"
- [ ] Bug fixes from manual testing
- [ ] Performance: startup time < 2 seconds, TUI frame rate smooth

**Deliverables**:
- All tests pass
- No clippy warnings
- Full user journey works without crashes

#### Week 12: Documentation + Release Prep

**Tasks**:
- [ ] Write `README.md`: installation, configuration, keybindings, troubleshooting
- [ ] Write `CONTRIBUTING.md`: development setup, testing, code style
- [ ] Set up GitHub Actions CI: build + test + clippy on PRs
- [ ] Create initial GitHub release with binary artifacts
- [ ] Verify install script or `cargo install lazyjob-cli` works
- [ ] Test on clean environment (no existing config)

**Deliverables**:
- `README.md` with clear getting-started guide
- CI pipeline green
- First GitHub release published
- Binary installs and runs on clean system

---

## Data Model Summary

### Core Entities

```
Job
  - id: UUID
  - title, company_name, location, remote, url, description
  - salary_min, salary_max, salary_currency
  - status: discovered | interested | applied | ...
  - interest_level: 1-5
  - source: greenhouse | lever | manual
  - discovered_at, applied_at, created_at, updated_at

Application
  - id: UUID
  - job_id: FK -> Job
  - stage: discovered -> interested -> applied -> phone_screen -> technical -> onsite -> offer -> accepted/rejected/withdrawn
  - stage_history: [(from, to, timestamp, reason)]
  - resume_version, cover_letter_version
  - last_contact_at, next_follow_up, notes

Contact
  - id: UUID
  - name, role, email, linkedin_url, company_id (FK)
  - relationship: recruiter | hiring_manager | interviewer | referral | network
  - quality: 1-5

Interview
  - id: UUID
  - application_id: FK
  - type: phone_screen | technical | behavioral | onsite | final
  - scheduled_at, duration_minutes, location, meeting_url
  - status: scheduled | completed | cancelled | no_show
  - feedback, rating

LifeSheet (YAML file, not SQLite)
  - basics: name, email, phone, location, summary, profiles[]
  - experience[]: company, position, dates, summary, context{team_size, org_size, industry}, achievements[]{description, metrics{}}
  - education[]: institution, degree, field, dates, courses[]
  - skills[]: name, level, keywords[]{name, years, proficiency}
  - certifications[], languages[], projects[]
  - preferences: job_types[], locations[], industries[], salary{min,max}
  - goals: short_term, long_term, timeline
```

### SQLite Schema Conventions

- UUID primary keys: `lower(hex(randomblob(16)))`
- Timestamps: ISO 8601 `datetime('now')`
- JSON columns for arrays: `serde_json` serialization
- Indexes on foreign keys and status columns
- WAL mode: `PRAGMA journal_mode=WAL`
- Foreign keys: `PRAGMA foreign_keys=on`

---

## Key Files

```
lazyjob/
├── lazyjob-core/src/
│   ├── lib.rs                       # Module exports
│   ├── error.rs                     # thiserror Error enums
│   ├── models/
│   │   ├── mod.rs
│   │   ├── job.rs
│   │   ├── application.rs
│   │   ├── contact.rs
│   │   ├── interview.rs
│   │   └── company.rs
│   ├── persistence/
│   │   ├── mod.rs                   # Database struct, SqlitePool
│   │   ├── migrations.rs
│   │   ├── job_repo.rs
│   │   ├── application_repo.rs
│   │   └── contact_repo.rs
│   ├── discovery/
│   │   ├── mod.rs                   # DiscoveryService
│   │   ├── sources/
│   │   │   ├── mod.rs
│   │   │   ├── greenhouse.rs
│   │   │   └── lever.rs
│   │   └── matcher.rs               # Embedding + cosine similarity
│   ├── resume/
│   │   ├── mod.rs                   # ResumeTailor
│   │   ├── jd_parser.rs
│   │   ├── gap_analysis.rs
│   │   ├── drafting.rs
│   │   ├── fabrication_guardrails.rs
│   │   └── docx_generator.rs
│   └── life_sheet/
│       └── mod.rs                   # YAML <-> SQLite conversion
│
├── lazyjob-llm/src/
│   ├── lib.rs
│   ├── error.rs
│   ├── provider.rs                  # LLMProvider trait
│   ├── message.rs                  # ChatMessage, ChatResponse types
│   ├── registry.rs                 # ProviderRegistry
│   ├── builder.rs                  # LLMBuilder
│   └── providers/
│       ├── mod.rs
│       ├── openai.rs
│       ├── anthropic.rs
│       └── ollama.rs
│
├── lazyjob-ralph/src/
│   ├── lib.rs
│   ├── process.rs                   # RalphProcessManager
│   ├── protocol.rs                 # JSON message types
│   └── loops/
│       ├── mod.rs
│       ├── job_discovery.rs
│       ├── company_research.rs
│       └── resume_tailor.rs
│
├── lazyjob-tui/src/
│   ├── lib.rs
│   ├── app.rs                      # Main app + event loop
│   ├── keymap.rs                   # Keybinding definitions
│   ├── theme.rs                    # Color scheme
│   ├── views/
│   │   ├── mod.rs
│   │   ├── dashboard.rs
│   │   ├── jobs_list.rs
│   │   ├── job_detail.rs
│   │   ├── applications.rs         # Pipeline kanban
│   │   ├── contacts.rs
│   │   ├── ralph_panel.rs
│   │   ├── settings.rs
│   │   └── help.rs                 # Help overlay
│   └── widgets/
│       ├── mod.rs
│       ├── job_card.rs
│       ├── application_card.rs
│       ├── stat_block.rs
│       ├── modal.rs
│       ├── confirm_dialog.rs
│       └── input_dialog.rs
│
└── lazyjob-cli/src/
    └── main.rs                      # Thin entry point
```

---

## Dependencies

```toml
[workspace]
members = ["lazyjob-core", "lazyjob-llm", "lazyjob-ralph", "lazyjob-tui", "lazyjob-cli"]
resolver = "2"

[workspace.dependencies]
# Core
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
futures = "0.3"
secrecy = "0.8"

# TUI
ratatui = "0.29"
crossterm = "0.28"

# Database
rusqlite = { version = "0.32", features = ["bundled"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }

# LLM
async-openai = "0.34"
ollama-rs = "0.3"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Document
docx-rs = "0.4"

# Security
keyring = "3"

# Utils
regex = "1"
ammonia = "4"
indicatif = "0.17"
```

---

## Open Questions Resolved

| Question | Decision |
|----------|----------|
| SQLite vs sqlx | rusqlite (synchronous, simpler for single-user) |
| Ralph as crate or binary | Separate binary, communicates via stdio JSON |
| Embeddings provider | OpenAI ada-002 (1536d) for MVP; Ollama nomic-embed-text as fallback |
| Vector DB | None needed; in-memory cosine similarity sufficient for 100s-1000s of jobs |
| Cover letters in MVP | Deferred; depends on company research being solid |
| Multiple resume templates | Single template for MVP; templating is post-MVP |
| Cloud sync | Deferred; local-first is the MVP point |
| Ralph loop persistence | SQLite `ralph_loops` table with status, survives TUI restart |

---

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| ratatui complexity underestimated | Medium | High | Start with existing examples; defer custom widgets |
| LLM API costs spiral | High | Medium | Ollama fallback; caching; budget alerts |
| Ralph protocol changes break TUI | Medium | High | Version the protocol; both sides version-check |
| DOCX edge cases (formatting) | Medium | Low | Test with 5-10 real JDs; fallback to plain text |
| SQLite WAL corruption on crash | Low | High | WAL mode; backup on startup if WAL file exists |
| Greenhouse/Lever API changes | Medium | Medium | Version pin; graceful degradation with error message |

---

## Success Criteria

### Week 3: Foundation Complete
- [ ] `cargo build` succeeds with no warnings
- [ ] TUI launches with working keybindings
- [ ] Can add company by name + board token
- [ ] "Refresh" fetches real jobs and displays in list

### Week 5: Tracking Complete
- [ ] Kanban pipeline fully functional with stage transitions
- [ ] Contacts CRUD works
- [ ] Dashboard shows live metrics from SQLite

### Week 7: LLM Integrated
- [ ] API key stored in system keyring (not plaintext)
- [ ] Ralph panel shows loop progress
- [ ] Job match scores displayed in Job Detail

### Week 9: Resume Tailoring Complete
- [ ] Can generate tailored DOCX resume for any job
- [ ] Fabrication warnings prevent false claims
- [ ] Ralph ResumeTailor loop works end-to-end

### Week 12: MVP Shipped
- [ ] `lazyjob-cli` publishes to GitHub Releases
- [ ] `cargo install lazyjob-cli` works
- [ ] README with clear getting-started
- [ ] CI pipeline green
- [ ] Full user journey tested end-to-end

---

## Spec Cross-References

This MVP plan synthesizes all 19 prior specs:

| Spec | Topic | Key Contribution to MVP |
|------|-------|----------------------|
| 01-architecture.md | Crate layout, ratatui patterns | Crate structure + view hierarchy |
| 02-llm-provider-abstraction.md | LLM provider trait | `LLMProvider` trait + OpenAI/Ollama |
| 03-life-sheet-data-model.md | Life sheet schema | YAML schema + SQLite tables |
| 04-sqlite-persistence.md | SQLite patterns | Repository pattern, WAL mode, migrations |
| 05-job-discovery-layer.md | Job sources, matching | Greenhouse/Lever clients, cosine similarity |
| 06-ralph-loop-integration.md | Ralph IPC | stdio JSON protocol, ProcessManager |
| 07-resume-tailoring-pipeline.md | Resume AI | JD parsing, gap analysis, docx-rs |
| 08-cover-letter-generation.md | Cover letters | Deferred to post-MVP |
| 09-tui-design-keybindings.md | TUI layout | Full view specs + keybindings |
| 10-application-workflow.md | State machine | ApplicationStage enum, transition rules |
| 11-platform-api-integrations.md | Greenhouse/Lever | API clients + rate limiting |
| 12-15-*.md | Interview/salary/networking | Deferred to post-MVP |
| 16-privacy-security.md | Keyring, encryption | API keys in keyring, no telemetry |
| 17-ralph-prompt-templates.md | Ralph prompts | Prompt templates for each loop type |
| 18-saas-migration-path.md | Future scaling | Repository trait for future PostgreSQL |
| 19-competitor-analysis.md | Market positioning | Differentiation: local-first + AI + TUI |
| AUDIENCE_JTBD.md | User needs | Anchor for all prioritization decisions |
| rust-patterns.md | Code patterns | Newtype wrappers, secrecy, thiserror/anyhow |
