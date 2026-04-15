# OpenAPI MVP: Build Plan

## Status
Draft

## Purpose

This spec synthesizes all research into an actionable MVP build plan. It defines what to build, in what order, and how to prioritize.

---

## MVP Definition

**Core Value Proposition**: A terminal-based job search command center that automates discovery, tailors applications, and keeps you organized — powered by AI agents.

**MVP Scope**:
- TUI application with jobs, applications, and contacts views
- Local SQLite persistence
- Greenhouse/Lever API integration for job discovery
- Resume tailoring via docx-rs
- Ralph loops for background automation
- Basic privacy (API keys in keyring)

**Out of MVP Scope**:
- Cloud sync
- Cover letter generation
- Interview prep AI
- Salary negotiation
- Networking automation
- Mobile app

---

## Crate Architecture (Implemented)

```
lazyjob/
├── lazyjob-core/           # Domain models, SQLite persistence
├── lazyjob-llm/           # LLM provider abstraction
├── lazyjob-tui/           # Terminal UI
├── lazyjob-cli/           # Binary entry point
├── lazyjob-ralph/         # Ralph loop integration (MVP: basic)
└── specs/                 # Architecture specs (this repo)
```

---

## Implementation Phases

### Phase 1: Foundation (2-3 weeks)

**Goals**: Core data model, SQLite persistence, basic CLI

#### Week 1: Project Setup
- [x] Initialize Cargo workspace
- [x] Set up logging (tracing)
- [x] Define core domain models (Job, Application, Contact, LifeSheet)
- [x] Set up SQLite with rusqlite
- [x] Basic repository pattern
- [x] Migration system

**Deliverables**:
- `lazyjob-core` crate with domain models
- SQLite database schema
- Migration runner

#### Week 2: CLI + TUI Skeleton
- [x] Build basic CLI with clap
- [x] Set up ratatui with crossterm backend
- [x] Create main layout (header, sidebar, content, status bar)
- [x] Implement Jobs List view (static data first)
- [x] Implement basic keybindings

**Deliverables**:
- Working TUI application
- Jobs list view with navigation
- Quit, help keybindings work

#### Week 3: Full CRUD + LLM Provider
- [x] Implement Jobs CRUD (create, read, update, delete)
- [x] Set up LLM provider trait with OpenAI implementation
- [x] Implement company research (simple version)
- [x] Connect discovery service to TUI

**Deliverables**:
- Full job management in TUI
- LLM integration working
- Company research from TUI

### Phase 2: Application Tracking (2-3 weeks)

**Goals**: Application workflow, pipeline view, contacts

#### Week 4: Application Pipeline
- [x] Application state machine (Discovered → Applied → etc.)
- [x] Pipeline kanban view in TUI
- [x] Application CRUD
- [x] Stage transitions with history

**Deliverables**:
- Kanban board in TUI
- Drag cards between stages
- Application detail view

#### Week 5: Contacts + Activity
- [x] Contacts management
- [x] Activity log for applications
- [x] Follow-up reminders
- [x] Dashboard view with metrics

**Deliverables**:
- Contacts list and detail views
- Activity timeline
- Dashboard statistics

### Phase 3: Job Discovery (1-2 weeks)

**Goals**: Real job discovery from Greenhouse/Lever

#### Week 6: Discovery Service
- [x] Implement Greenhouse API client
- [x] Implement Lever API client
- [x] Job source registry
- [x] Deduplication logic
- [x] Refresh/sync from TUI

**Deliverables**:
- Can add companies by board token
- Jobs fetched and stored in SQLite
- Manual refresh works

### Phase 4: Resume Tailoring (2 weeks)

**Goals**: Tailored resume generation

#### Week 7: Resume Service
- [x] Life sheet parsing (YAML)
- [x] Job description analysis
- [x] Gap analysis
- [x] docx-rs document generation
- [x] TUI integration for tailoring

**Deliverables**:
- `tailor` command in TUI
- Generates tailored resume DOCX
- Opens in default app

### Phase 5: Ralph Integration (2-3 weeks)

**Goals**: Background automation

#### Week 8-9: Ralph Process Manager
- [x] Ralph CLI interface
- [x] Subprocess spawning with tokio
- [x] JSON protocol over stdio
- [x] Event handling in TUI
- [x] Ralph panel in TUI

**Deliverables**:
- Can start Ralph loop from TUI
- Progress shown in panel
- Results saved to database

#### Week 10: Ralph Loops (MVP)
- [x] Basic job discovery loop
- [x] Basic company research loop
- [x] Ralph status tracking

**Deliverables**:
- Discovery loop finds jobs
- Status updates stream to TUI
- Loop completion handled

### Phase 6: Polish (1 week)

**Goals**: Quality, performance, UX**

#### Week 11-12
- [x] Error handling throughout
- [x] Loading states and spinners
- [x] Confirmation dialogs for destructive actions
- [x] Help overlays
- [x] Settings view (basic)
- [x] Data export (JSON)

**Deliverables**:
- Production-quality TUI
- Comprehensive help system
- Data portability

---

## Technical Decisions (Final)

### Stack
- **Language**: Rust
- **UI**: ratatui + crossterm
- **Database**: SQLite (rusqlite)
- **Async**: tokio
- **Serialization**: serde + serde_json
- **LLM**: async-openai + custom trait
- **Document**: docx-rs
- **Keyring**: keyring-rs

### Data Model

```rust
// Core entities (see Spec 3 for full schema)
struct Job { id, title, company_name, description, status, ... }
struct Application { id, job_id, stage, resume_version, ... }
struct Contact { id, name, role, company, ... }
struct LifeSheet { personal, experience[], education[], skills[], ... }
```

### Key Files

```
lazyjob/
├── lazyjob-core/src/
│   ├── models/          # Domain models
│   ├── persistence/      # SQLite repositories
│   ├── discovery/       # Job source clients
│   └── lib.rs
├── lazyjob-llm/src/
│   ├── provider.rs      # Trait + implementations
│   ├── prompts/         # Prompt templates
│   └── lib.rs
├── lazyjob-tui/src/
│   ├── app.rs           # Main app + event loop
│   ├── views/           # View implementations
│   ├── widgets/         # Custom widgets
│   └── lib.rs
├── lazyjob-ralph/src/
│   ├── process.rs       # Ralph subprocess manager
│   ├── protocol.rs      # JSON message handling
│   └── lib.rs
└── lazyjob-cli/src/
    └── main.rs
```

---

## OpenAPI MVP Scope

### What's Included

1. **TUI with Views**:
   - Dashboard (stats, recent activity)
   - Jobs List (filter, sort, CRUD)
   - Job Detail
   - Applications Pipeline (kanban)
   - Contacts
   - Settings (API keys, companies)
   - Help overlay

2. **Data Persistence**:
   - SQLite with migrations
   - All CRUD operations
   - Activity logging

3. **Job Discovery**:
   - Greenhouse API integration
   - Lever API integration
   - Manual job entry
   - Basic deduplication

4. **LLM Integration**:
   - OpenAI provider (required)
   - Ollama provider (optional)
   - Company research via LLM
   - Job description analysis

5. **Resume Tailoring**:
   - Life sheet (YAML) import
   - Job requirements analysis
   - Keyword extraction
   - DOCX generation

6. **Ralph Integration**:
   - Subprocess manager
   - JSON protocol
   - Event handling
   - Basic discovery loop

7. **Privacy**:
   - API keys in system keyring
   - Local-only by default
   - Data export

### What's NOT Included (Post-MVP)

- Cover letter generation
- Interview prep AI
- Salary negotiation
- Networking automation
- Cloud sync
- Team collaboration
- Web UI

---

## Success Metrics

### Week 1 Milestone
- [ ] Can build and run `lazyjob-cli`
- [ ] TUI displays with jobs list
- [ ] Can add/edit/delete jobs
- [ ] SQLite persists data

### Week 6 Milestone
- [ ] Full CRUD working
- [ ] Pipeline kanban works
- [ ] Contacts manageable
- [ ] Dashboard shows stats

### Week 12 Milestone (MVP Complete)
- [ ] TUI polished and usable
- [ ] Job discovery from Greenhouse/Lever
- [ ] Resume tailoring generates DOCX
- [ ] Ralph loops run discovery
- [ ] Data exportable
- [ ] Help system complete

---

## Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-------------|--------|-------------|
| ratatui complexity | Medium | High | Use existing examples, start simple |
| LLM API costs | High | Medium | Ollama fallback, caching |
| Ralph protocol design | Medium | High | Start simple, iterate |
| DOCX generation edge cases | Medium | Low | Test with real JDs first |
| SQLite concurrency | Low | Medium | WAL mode, proper timeouts |

---

## Dependencies Summary

```toml
[workspace]
members = ["lazyjob-core", "lazyjob-llm", "lazyjob-tui", "lazyjob-cli"]

[workspace.dependencies]
ratatui = "0.29"
crossterm = "0.28"
rusqlite = "0.32"
sqlx = "0.8"
tokio = "1"
clap = "4"
serde = "1"
serde_yaml = "0.9"
docx-rs = "0.4"
keyring = "3"
age = "0.9"
tracing = "0.1"
thiserror = "2"
anyhow = "1"
```

---

## Sources

All specs in this directory:
- `01-architecture.md`
- `02-llm-provider-abstraction.md`
- `03-life-sheet-data-model.md`
- `04-sqlite-persistence.md`
- `05-job-discovery-layer.md`
- `06-ralph-loop-integration.md`
- `07-resume-tailoring-pipeline.md`
- `08-cover-letter-generation.md`
- `09-tui-design-keybindings.md`
- `10-application-workflow.md`
- `11-platform-api-integrations.md`
- `12-15-interview-salary-networking-notifications.md`
- `16-privacy-security.md`
- `17-ralph-prompt-templates.md`
- `18-saas-migration-path.md`
- `19-competitor-analysis.md`
