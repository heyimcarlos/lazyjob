# LazyJob Architecture Overview — Implementation Plan

## Spec Reference
- **Spec file**: `specs/01-architecture.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
This spec defines the foundational crate architecture for LazyJob — a three-tier structure separating core domain logic, LLM abstraction, and Ralph agent integration from the TUI presentation layer. The implementation sets up the Cargo workspace skeleton, core data models, error handling patterns, and basic module stubs that all other specs will build upon.

## Problem Statement
LazyJob needs a scalable architecture supporting: (1) rich TUI with multiple views, (2) Ralph autonomous agent loops for AI tasks, (3) local-first SQLite persistence, (4) multi-provider LLM abstraction, and (5) path to SaaS without rewrite.

## Implementation Phases

### Phase 1: Workspace Foundation
1. Create `Cargo.toml` workspace definition with members:
   - `lazyjob-core`
   - `lazyjob-llm`
   - `lazyjob-ralph`
   - `lazyjob-tui`
   - `lazyjob-cli`
2. Set `resolver = "2"` for complex workspace support
3. Create stub `lib.rs` and `main.rs` for each crate with proper visibility
4. Add all required external dependencies to workspace root `Cargo.toml` as shared

### Phase 2: Core Domain Models (`lazyjob-core`)
1. Define `lazyjob-core/src/lib.rs` with module declarations:
   - `pub mod models;`
   - `pub mod state;`
   - `pub mod persistence;`
   - `pub mod discovery;`
   - `pub mod error;`
2. Implement error types using `thiserror`:
   - `pub type Result<T> = std::result::Result<T, Error>;`
   - `pub enum Error { ... }` variants for Db, Io, Parse, Validation, NotFound
3. Implement core domain structs in `models/`:
   - `Job`, `JobStatus`, `Company`, `CompanySize`
   - `Application`, `ApplicationStatus`
   - `Contact`, `ContactRelationship`
   - `LifeSheet`, `PersonalInfo`, `Experience`, `Education`, `Skill`, `JobPreferences`
   - `SalaryRange`, `FollowUp`
4. Define `Repository` trait in `persistence/`:
   ```rust
   pub trait Repository: Send + Sync {
       fn jobs(&self) -> Result<Vec<Job>>;
       fn job(&self, id: Uuid) -> Result<Option<Job>>;
       fn save_job(&mut self, job: Job) -> Result<()>;
       fn delete_job(&mut self, id: Uuid) -> Result<()>;
       fn applications(&self) -> Result<Vec<Application>>;
       fn save_application(&mut self, app: Application) -> Result<()>;
       fn contacts(&self) -> Result<Vec<Contact>>;
       fn save_contact(&mut self, contact: Contact) -> Result<()>;
       fn life_sheet(&self) -> Result<LifeSheet>;
       fn save_life_sheet(&mut self, sheet: LifeSheet) -> Result<()>;
   }
   ```
5. Define `AppState` and `StateMachine` trait in `state/`
6. Define `FilterSet`, `View` enum in `state/`

### Phase 3: LLM Abstraction (`lazyjob-llm`)
1. Define `lazyjob-llm/src/lib.rs`:
   - `pub mod provider;`
   - `pub mod anthropic;`
   - `pub mod openai;`
   - `pub mod ollama;`
   - `pub mod prompts;`
2. Define `LLMProvider` trait with async methods:
   ```rust
   pub trait LLMProvider: Send + Sync {
       async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse>;
       async fn complete(&self, prompt: &str) -> Result<String>;
       async fn embed(&self, text: &str) -> Result<Vec<f32>>;
   }
   ```
3. Define `ChatMessage` enum and `ChatResponse` struct
4. Implement `AnthropicProvider`, `OpenAIProvider`, `OllamaProvider` as concrete types
5. Create `prompts/` module with placeholder prompt templates for each Ralph loop type

### Phase 4: Ralph Integration (`lazyjob-ralph`)
1. Define `lazyjob-ralph/src/lib.rs`:
   - `pub mod ipc;`
   - `pub mod process;`
   - `pub mod state_sync;`
2. Define `RalphMessage` enum and `RalphLoop` enum
3. Define `RalphIPC` trait for subprocess communication
4. Define `ProcessManager` for spawning/managing Ralph subprocesses
5. Define `StateSync` for syncing state between TUI and Ralph loops

### Phase 5: TUI Foundation (`lazyjob-tui`)
1. Define `lazyjob-tui/src/lib.rs`:
   - `pub mod app;`
   - `pub mod views;`
   - `pub mod widgets;`
   - `pub mod keymap;`
   - `pub mod theme;`
2. Define `app.rs` with main `App` struct and event loop stub
3. Define basic view stubs in `views/`:
   - `dashboard.rs`, `jobs.rs`, `detail.rs`, `search.rs`, `help.rs`
4. Define `keymap.rs` with vim-inspired keybindings
5. Define `theme.rs` with color scheme
6. Set up ratatui `Terminal` initialization

### Phase 6: CLI Entry Point (`lazyjob-cli`)
1. Create `lazyjob-cli/src/main.rs`:
   - Parse CLI arguments using `clap`
   - Initialize logging with `tracing`
   - Run TUI or headless mode based on flags

## Data Model

### SQLite Schema (Phase 2 - defined, not yet implemented)
Tables to be created in `04-sqlite-persistence.md`:
- `jobs` — id, title, company_id, location, url, description, salary_min, salary_max, status, discovered_at, applied_at, notes
- `companies` — id, name, website, industry, size, notes
- `applications` — id, job_id, submitted_at, status, resume_version, cover_letter_version
- `contacts` — id, name, role, email, linkedin_url, company_id, relationship, notes
- `life_sheet` — YAML blob stored as JSON in SQLite

### New Structs/Types
| Struct | Crate | Purpose |
|--------|-------|---------|
| `Job`, `Company`, `Application`, `Contact`, `LifeSheet` | lazyjob-core | Domain entities |
| `Error` enum | lazyjob-core | Error handling |
| `Repository` trait | lazyjob-core | Persistence abstraction |
| `AppState` | lazyjob-core | Application state container |
| `LLMProvider` trait | lazyjob-llm | LLM abstraction |
| `ChatMessage`, `ChatResponse` | lazyjob-llm | LLM types |
| `RalphMessage`, `RalphLoop` | lazyjob-ralph | Ralph IPC types |
| `App` | lazyjob-tui | TUI application |

## API Surface

### lazyjob-core
```rust
pub use crate::models::{Job, Company, Application, Contact, LifeSheet, ...};
pub use crate::error::{Error, Result};
pub use crate::persistence::Repository;
pub use crate::state::{AppState, StateMachine, View, FilterSet};
```

### lazyjob-llm
```rust
pub use crate::provider::{LLMProvider, ChatMessage, ChatResponse};
pub use crate::anthropic::AnthropicProvider;
pub use crate::openai::OpenAIProvider;
pub use crate::ollama::OllamaProvider;
```

### lazyjob-ralph
```rust
pub use crate::process::{ProcessManager, RalphMessage, RalphLoop};
pub use crate::ipc::RalphIPC;
```

### lazyjob-tui
```rust
pub use crate::app::App;
pub use crate::views::*;
pub use crate::keymap::KeyMap;
pub use crate::theme::Theme;
```

## Key Technical Decisions

1. **Three-tier over simpler two-tier**: Rationale per spec — distinct UI/AI/data layers, path to SaaS, LLM isolation beneficial for testing
2. **Trait-based LLM abstraction**: Enables runtime provider switching without recompilation
3. **Ralph as subprocess**: JSON protocol over stdio, language-agnostic, crash-resilient
4. **Repository trait in core**: Allows SQLite implementation now, PostgreSQL later for SaaS
5. **No macros crate yet**: `lazyjob-macros` deferred until concrete boilerplate emerges

### Alternatives Rejected
- **Monolithic single crate**: Poor scalability past ~20k lines
- **Loom-style 30+ crates**: Excessive complexity for MVP phase
- **Embedded LLM**: Ollama support included but not primary (too slow for production use cases)

## File Structure
```
lazyjob/
├── Cargo.toml                    # Workspace root (create)
├── lazyjob-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── error.rs
│       └── models/
│           ├── mod.rs
│           ├── job.rs
│           ├── company.rs
│           ├── application.rs
│           ├── contact.rs
│           └── life_sheet.rs
│       └── state/
│           ├── mod.rs
│           └── app_state.rs
│       └── persistence/
│           ├── mod.rs
│           └── repository.rs
│       └── discovery/
│           └── mod.rs
├── lazyjob-llm/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── provider.rs
│       ├── anthropic.rs
│       ├── openai.rs
│       ├── ollama.rs
│       └── prompts/
│           └── mod.rs
├── lazyjob-ralph/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── ipc.rs
│       ├── process.rs
│       └── state_sync.rs
├── lazyjob-tui/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── app.rs
│       ├── views/
│       │   ├── mod.rs
│       │   ├── dashboard.rs
│       │   ├── jobs.rs
│       │   ├── detail.rs
│       │   ├── search.rs
│       │   └── help.rs
│       ├── widgets/
│       │   └── mod.rs
│       ├── keymap.rs
│       └── theme.rs
└── lazyjob-cli/
    ├── Cargo.toml
    └── src/
        └── main.rs
```

## Dependencies

### Workspace Root Dependencies
```toml
[workspace.dependencies]
uuid = { version = "1.8", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1.0"
anyhow = "1.0"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
tracing = "0.1"
tracing-subscriber = "0.3"
```

### lazyjob-core
- `uuid`, `chrono`, `thiserror`, `anyhow`, `serde`, `tokio`

### lazyjob-llm
- `async-trait`, `reqwest` (for HTTP), `serde`, `tokio`

### lazyjob-ralph
- `tokio`, `serde`, `uuid`

### lazyjob-tui
- `ratatui`, `ratatui-core`, `ratatui-crossterm`, `crossterm`, `lazyjob-core`, `lazyjob-llm`, `lazyjob-ralph`

### lazyjob-cli
- `clap`, `lazyjob-core`, `lazyjob-tui`

## Testing Strategy

### Unit Tests
- Core models: serialization/deserialization round-trips
- State machine: valid/invalid transitions
- Repository trait: mock implementation for unit tests

### Integration Tests
- Workspace compiles with `cargo build --all`
- All crates pass `cargo clippy -- -D warnings`
- All crates format correctly with `cargo fmt --all`

### Module Stub Verification
Each crate should compile independently:
```bash
cargo check -p lazyjob-core
cargo check -p lazyjob-llm
cargo check -p lazyjob-ralph
cargo check -p lazyjob-tui
cargo check -p lazyjob-cli
```

## Open Questions

1. **Ralph Loop Lifecycle**: IPC protocol details deferred to `agentic-ralph-subprocess-protocol.md`
2. **State Persistence**: Full SQLite implementation deferred to `04-sqlite-persistence.md`
3. **Undo/Redo**: Not in scope for MVP; revisit later
4. **Plugin System**: Out of scope for MVP
5. **Multi-user/SaaS**: Data model supports it structurally but auth not designed yet

## Dependencies on Other Specs

This spec must be implemented FIRST as all other specs depend on:
- Crate structure defined here
- Domain models in `lazyjob-core`
- Error types and Result alias
- Repository trait signature

Implementation order after this:
1. `02-llm-provider-abstraction.md` — fills in LLM provider details
2. `03-life-sheet-data-model.md` — fills in LifeSheet details
3. `04-sqlite-persistence.md` — implements Repository trait
4. `06-ralph-loop-integration.md` — fills in Ralph details
5. `09-tui-design-keybindings.md` — fills in TUI details

## Effort Estimate
**Rough: 2-3 days**

Reasoning: This is foundational skeleton work. Creating empty crates, module stubs, and domain types is straightforward but requires careful attention to dependency graph ordering. The domain models are simple structs (no complex logic). Main complexity is ensuring the workspace compiles cleanly with all inter-crate dependencies properly declared.
