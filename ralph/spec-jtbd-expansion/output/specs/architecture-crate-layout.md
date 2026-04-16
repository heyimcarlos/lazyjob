# Spec: Architecture — Crate Layout

**JTBD**: A fast, reliable tool that works offline
**Topic**: Define the workspace organization, crate boundaries, and dependency graph for the LazyJob Rust project
**Domain**: architecture

---

## What

The LazyJob project is organized as a Cargo workspace with 5 primary crates: `lazyjob-core` (domain models + persistence), `lazyjob-llm` (LLM abstraction + prompts), `lazyjob-ralph` (Ralph subprocess IPC), `lazyjob-tui` (terminal UI), and `lazyjob-cli` (binary entry point). This spec defines the boundaries, public API surface, and dependency rules between crates.

## Why

A clean crate organization enables:
- Parallel compilation of independent crates
- Clear boundaries that prevent spaghetti dependencies
- Independent testing of each layer (test LLM without TUI, test persistence without AI)
- A path to publishing internal crates or swapping UIs (TUI → headless → web)
- A clean foundation for the SaaS migration (extract interfaces once, implement multiple backends)

The dependency graph flows downward: CLI depends on TUI, TUI depends on Ralph, Ralph depends on LLM and Core, LLM depends on Core. No upward dependencies.

## How

### Workspace Structure

```toml
# /home/lab-admin/repos/lazyjob/Cargo.toml
[workspace]
members = [
    "lazyjob-core",
    "lazyjob-llm",
    "lazyjob-ralph",
    "lazyjob-tui",
    "lazyjob-cli",
]
resolver = "2"
```

### Crate Boundaries

```
lazyjob-core/           # lazyjob-core/src/lib.rs — public re-exports
├── src/
│   ├── lib.rs              # Re-exports: models, persistence, discovery, config
│   ├── error.rs            # crate::Error (thiserror), Result<T>
│   ├── models/             # Domain entities: Job, Application, Contact, LifeSheet, Company
│   │   ├── mod.rs
│   │   ├── job.rs
│   │   ├── application.rs
│   │   ├── contact.rs
│   │   ├── company.rs
│   │   └── life_sheet.rs
│   ├── persistence/        # Repository traits + sqlx SQLite implementations
│   │   ├── mod.rs
│   │   ├── database.rs     # Database struct (SqlitePool wrapper)
│   │   ├── jobs.rs
│   │   ├── applications.rs
│   │   ├── contacts.rs
│   │   ├── companies.rs
│   │   └── migrations/     # sqlx migration files
│   ├── discovery/          # Job discovery: sources, deduplication, scoring
│   │   ├── mod.rs
│   │   ├── aggregation.rs
│   │   ├── deduplication.rs
│   │   └── normalizers.rs
│   ├── platforms/          # Platform integrations: ATS APIs, scraping, aggregation
│   │   ├── mod.rs
│   │   ├── traits.rs      # PlatformClient trait, DiscoveredJob, RawJob
│   │   ├── greenhouse.rs
│   │   ├── lever.rs
│   │   ├── adzuna.rs
│   │   ├── workday.rs
│   │   ├── manual.rs      # LinkedIn URL bookmark import
│   │   └── rate_limiter.rs
│   ├── config/            # lazyjob.toml parsing
│   │   ├── mod.rs
│   │   └── schema.rs
│   └── lexicon/           # Shared text-processing utilities
│       ├── mod.rs
│       ├── tech_terms.rs   # Technical term regex lexicon (shared by ghost detection + skills)
│       └── jurisdictions.rs # Pay transparency jurisdiction list

lazyjob-llm/            # lazyjob-llm/src/lib.rs
├── src/
│   ├── lib.rs              # Re-exports: LlmProvider, LlmBuilder, prompts
│   ├── error.rs
│   ├── provider.rs        # LlmProvider trait, LlmClient enum
│   ├── anthropic.rs       # AnthropicProvider
│   ├── openai.rs          # OpenAIProvider
│   ├── ollama.rs          # OllamaProvider (chat + embeddings)
│   ├── embeddings.rs      # EmbeddingProvider trait
│   ├── cost.rs            # Microdollar cost estimation per model
│   └── prompts/           # Prompt templates (referenced by ralph loops)
│       ├── mod.rs
│       ├── resume_tailoring.rs
│       ├── cover_letter.rs
│       ├── networking_outreach.rs
│       ├── interview_prep.rs
│       └── salary_negotiation.rs

lazyjob-ralph/          # lazyjob-ralph/src/lib.rs
├── src/
│   ├── lib.rs              # Re-exports: LoopType, RalphHandle, RalphEvent
│   ├── error.rs
│   ├── process.rs         # RalphProcessManager: spawn, kill, restart
│   ├── protocol.rs       # stdio JSON framing, WorkerCommand/WorkerEvent
│   ├── dispatch.rs       # LoopDispatch: PostTransitionSuggestion → LoopType mapping
│   └── loops/            # Loop-specific logic
│       ├── mod.rs
│       ├── job_discovery.rs
│       ├── resume_tailoring.rs
│       └── ...           # One module per LoopType

lazyjob-tui/            # lazyjob-tui/src/lib.rs
├── src/
│   ├── lib.rs              # App struct, run(), event loop
│   ├── app.rs            # App struct + crossterm event handling
│   ├── views/           # View implementations
│   │   ├── mod.rs
│   │   ├── dashboard.rs
│   │   ├── jobs.rs
│   │   ├── job_detail.rs
│   │   ├── applications.rs
│   │   ├── contacts.rs
│   │   ├── ralph.rs
│   │   ├── settings.rs
│   │   └── help.rs
│   ├── widgets/         # Custom ratatui widgets
│   │   ├── mod.rs
│   │   ├── job_card.rs
│   │   ├── application_card.rs
│   │   ├── contact_card.rs
│   │   └── ...
│   ├── keymap.rs        # Keybinding definitions per view
│   ├── theme.rs         # Color scheme (dark/light)
│   └── state.rs         # AppState: selected view, job filter, etc.

lazyjob-cli/            # lazyjob-cli/src/main.rs
├── src/
│   └── main.rs          # Entry point: run(), lazyjob_tui::run()
```

### Dependency Graph

```
lazyjob-cli
└── lazyjob-tui
    ├── lazyjob-ralph
    │   ├── lazyjob-llm
    │   │   └── lazyjob-core
    │   └── lazyjob-core
    └── lazyjob-core
```

**Rule**: No upward dependencies. `lazyjob-core` has zero dependencies on other internal crates. `lazyjob-llm` depends only on `lazyjob-core`. `lazyjob-ralph` depends on both `lazyjob-llm` and `lazyjob-core`. `lazyjob-tui` depends on all of the above.

### Dependency Enforcement

At the crate boundary, use `pub(crate)` visibility to control what is accessible across crates. Internal modules use default private visibility. Only explicitly re-exported items in `lib.rs` are public:

```rust
// lazyjob-core/src/lib.rs
pub mod models;
pub mod persistence;
pub mod discovery;
pub mod platforms;
pub mod config;
pub mod lexicon;
pub mod error;

pub use error::{Error, Result};
```

### Shared Utilities Location

Three utilities are shared across multiple domains and must live in `lazyjob-core/src/lexicon/`:
1. **`tech_terms.rs`**: Technical term regex lexicon — used by ghost detection and skills gap analysis
2. **`jurisdictions.rs`**: Pay transparency jurisdiction list — used by ghost detection and salary negotiation
3. **`SkillNormalizer`**: ESCO skill alias table + normalization — used by resume tailoring, skills gap analysis, and job search

These are the only exceptions to the "no shared mutable state" rule — they are pure static data.

### Cargo Feature Flags

```toml
# lazyjob-core/Cargo.toml
[features]
default = []
unsafe-sqlx = []  # For sqlx offline mode (prepare_all without DB connection)
```

```toml
# lazyjob-llm/Cargo.toml
[features]
default = []
```

### Key Files Reference

| File | Purpose | Visibility |
|------|---------|------------|
| `lazyjob-core/src/lib.rs` | Public re-exports | `pub` |
| `lazyjob-core/src/models/*.rs` | Domain structs | `pub(crate)` |
| `lazyjob-core/src/persistence/*.rs` | Repository impls | `pub(crate)` |
| `lazyjob-core/src/error.rs` | Error types | `pub` (re-exported) |
| `lazyjob-llm/src/provider.rs` | LlmProvider trait | `pub` |
| `lazyjob-ralph/src/protocol.rs` | IPC types | `pub(crate)` |
| `lazyjob-tui/src/app.rs` | App entry | private |

## Open Questions

- **`unsafe-sqlx` feature**: The spec-inventory notes `sqlx.toml` for offline query preparation. We need `CARGO_BUILD_SQLX` or a `sqlx.toml` for offline mode. Is `unsafe-sqlx` the right approach or should we use `sqlx.toml` approach?
- **`lazyjob-macros` crate**: Not included in the workspace yet. Should a derive macros crate be added for e.g., `#[derive(Repository)]` on domain structs? Defer to Phase 2.
- **`embedding` sub-crate**: The spec-inventory suggests a separate embedding crate for offline semantic search. OllamaProvider serves both chat and embeddings in the current spec. Splitting is an optimization for later.

## Implementation Tasks

- [ ] Create `Cargo.toml` workspace at `/home/lab-admin/repos/lazyjob/` with members list for all 5 crates
- [ ] Scaffold `lazyjob-core/src/` with `lib.rs`, `error.rs`, and all subdirectory `mod.rs` files with empty modules
- [ ] Scaffold `lazyjob-llm/src/`, `lazyjob-ralph/src/`, `lazyjob-tui/src/`, `lazyjob-cli/src/main.rs` with empty modules
- [ ] Verify dependency graph: `cargo build` from workspace root compiles all crates in correct order
- [ ] Verify no circular dependencies with `cargo check --all` after initial scaffold
- [ ] Add `lazyjob-core/src/lexicon/tech_terms.rs` and `jurisdictions.rs` with static data (populated from ghost detection and salary specs)
