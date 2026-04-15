# LazyJob Architecture Overview

## Status
Researching

## Problem Statement

LazyJob is a lazygit-style terminal user interface for job search management, built in Rust. It needs to handle three major concerns simultaneously: a rich interactive TUI (terminal user interface), autonomous background agents (ralph loops) that perform job search tasks, and a structured data layer for tracking jobs, applications, and the user's life sheet. The architecture must support local-first operation with a clear path to SaaS.

## Research Findings

### Lazygit Architecture (Go/gocui)

Lazygit is the primary inspiration. Its architecture is organized around:

**Project Structure:**
- `main.go` — minimal entry point, delegates to `App`
- `pkg/app/` — application bootstrapping
- `pkg/gui/` — all TUI logic (contexts, controllers, keybindings, views, presentation)

**Core Patterns:**
1. **Context System**: The UI is organized around "contexts" — independent panel states implementing `Context` interface with `HandleFocus()`, `HandleRender()`, `HandleRenderToMain()`. Contexts have kinds: SIDE_CONTEXT, MAIN_CONTEXT, POPUP, EXTRAS, GLOBAL. A context stack manages navigation (`Push`, `Pop`, `Replace`, `Activate`).

2. **Controller Pattern**: Business logic lives in "controllers" attached to contexts. Controllers implement `IController` with `GetKeybindings()`, lifecycle methods. Each context can have multiple controllers attached. This separates WHAT is shown (presentation) from HOW it responds to input (controller).

3. **Dimensional Layout**: Views positioned via X0, Y0, X1, Y1 coordinates. Lazygit calculates dimensions per-context via `getWindowDimensions()`. Views are layered — base views, then overlaid views, then popups.

4. **Keybinding System**: Binding struct contains `ViewName`, `Key`, `Handler`, `Description`, `GetDisabledReason()` guard function. Keybindings are registered per-context and checked against current context to filter available actions.

5. **Presentation Layer**: `pkg/gui/presentation/` handles all text formatting — commit messages, file trees, list items. Separates content transformation from rendering.

**Source:** https://github.com/jesseduffield/lazygit

### Ratatui Architecture (Rust)

Ratatui is the Rust TUI library, a community fork of `tui-rs`.

**Core Abstractions:**

1. **Component Trait** — Object-oriented, trait-based UI components:
```rust
pub trait Component {
    fn init(&mut self) -> Result<()>;
    fn handle_events(&mut self, event: Option<Event>) -> Action;
    fn handle_key_events(&mut self, key: KeyEvent) -> Action;
    fn handle_mouse_events(&mut self, mouse: MouseEvent) -> Action;
    fn update(&mut self, action: Action) -> Action;
    fn render(&mut self, f: &mut Frame, rect: Rect);
}
```

2. **Layout System** — Flexbox-like constraint-based layout:
```rust
Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(50), Constraint::Length(10)])
    .split(frame.area())
```
Constraint types: `Length`, `Percentage`, `Ratio`, `Min`, `Max`, `Fill`. `Flex` enum controls alignment: `Legacy`, `Start`, `Center`, `End`, `SpaceBetween`, `SpaceAround`.

3. **Widget System** — Stateless `Widget` trait and stateful `StatefulWidget`:
```rust
pub trait Widget {
    fn render(self, area: Rect, buf: &mut Buffer);
}
pub trait StatefulWidget {
    type State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}
```

4. **Backend Abstraction** — Supports Crossterm (default), Termion, Termwiz. `Terminal<Backend>` wraps the backend.

5. **Event Handling** — Ratatui delegates to `crossterm::event` for input. Three patterns: centralized handler, centralized catching with message passing, distributed loops. Ratatui itself is synchronous — async integration requires a separate thread for TUI operations.

6. **Immediate Mode Rendering** — UI is recreated every frame. State changes directly affect next render. No persistent widget objects — just call `frame.render_widget()` based on current application state.

**Source:** https://ratatui.rs/ | https://github.com/ratatui-org/ratatui

### Rust Workspace Organization (General Patterns)

Researching Tokio, rust-analyzer, and diesel provides these workspace patterns:

**Tokio:**
```
tokio/
├── Cargo.toml (workspace)
├── tokio/           (core runtime)
├── tokio-macros/    (proc macros)
├── tokio-stream/    (async stream utilities)
├── tokio-util/      (utility functions)
└── tokio-test/      (testing utilities)
```

**Rust-analyzer:**
```
rust-analyzer/
├── Cargo.toml (workspace)
├── crates/
│   ├── parser/      (hand-written recursive descent)
│   ├── syntax/      (per-file syntax tree via rowan)
│   ├── hir-expand/  (macro expansion)
│   ├── hir-def/    (type/resolution definitions)
│   ├── hir-ty/     (type inference)
│   ├── ide/        (IDE features)
│   ├── ide-db/     (salsa-based database)
│   └── rust-analyzer/ (LSP server)
```

**Key Principles:**
- Single `Cargo.lock` at root ensures dependency compatibility across crates
- Shared `target/` reduces rebuild time
- Crates separated when: independent publishing needed, proc macros (must be separate), platform-specific code, heavy optional dependencies
- Inter-crate communication via public API re-exports, typed interfaces, shared traits in a "core" crate

**Source:** https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html | https://rust-analyzer.github.io/book/contributing/architecture.html

### Ralph Loop Pattern

The ralph loop pattern (from `ralph.sh` in this project) spawns a fresh Claude CLI instance per iteration. Each iteration:
1. Reads `progress.md`, `tasks.json`, and context from previous iterations (via files on disk)
2. Works on a single task
3. Writes results to disk (spec files, progress.md, updated tasks.json)
4. Outputs `<promise>COMPLETE</promise>` to signal completion

Key architectural implications:
- **State flows through files, not memory** — between iterations, state persists on disk
- **Subprocess communication** — the TUI spawns `ralph.sh` as a subprocess; communication is via files, not in-memory channels
- **Process isolation** — each ralph iteration is a fresh process with no in-memory state from previous iterations
- **Long-running vs short-lived** — the TUI is a long-running process; ralph loops are short-lived processes spawned on demand

---

## Design Options

### Option A: Monolithic Crate

Single `lazyjob/` crate with `src/main.rs` and `src/lib.rs`. Modules: `tui/`, `domain/`, `agents/`, `db/`.

**Pros:**
- Simplest to build and navigate
- No inter-crate dependency management
- Shared type system without trait objects
- Fast iteration for a v1

**Cons:**
- No independent testability of layers
- Cannot publish components to crates.io
- Harder to have platform-specific implementations
- Module boundaries get murky as complexity grows

### Option B: Two-Crate Split (lazyjob-core + lazyjob-cli)

`lazyjob-core/` contains: domain logic, TUI components, database, LLM providers. `lazyjob/` depends on core and adds CLI entry point.

**Pros:**
- Core can be tested and used as a library
- Clear separation between library and CLI
- Natural API boundary for future SaaS server

**Cons:**
- Still couples TUI to domain — can't use core in a non-TUI context (e.g., a web interface)
- Two-crate complexity for what's probably a v1

### Option C: Layered Three-Crate (lazyjob-core + lazyjob-tui + lazyjob-cli)

- `lazyjob-core/` — Pure domain: job, application, life_sheet entities; no I/O, no TUI
- `lazyjob-tui/` — TUI components, widgets, keybindings; depends on core
- `lazyjob-cli/` — Binary, orchestration, argument parsing; depends on core + tui

**Pros:**
- Clean separation: domain knows nothing about presentation
- Core can be used headless (background agents, web frontend future)
- TUI can be tested with mock core
- Clear API boundaries for future SaaS (server mode)

**Cons:**
- Three crates where one might suffice for v1
- More boilerplate for inter-crate types

### Option D: Full Workspace (6+ crates)

Based on loom's pattern with many focused crates:
- `lazyjob-core/` — Domain entities, traits, no I/O
- `lazyjob-db/` — SQLite persistence layer
- `lazyjob-llm/` — LLM provider abstraction
- `lazyjob-agents/` — Ralph loop invocation and result handling
- `lazyjob-tui/` — TUI components
- `lazyjob-cli/` — Binary
- Potentially: `lazyjob-providers-greenhouse/`, `lazyjob-providers- lever/` for platform-specific code

**Pros:**
- Maximum modularity and testability
- Platform integrations can be developed independently
- Clear ownership boundaries for team

**Cons:**
- 6+ crates is heavyweight for v1
- Inter-crate dependency management overhead
- Cognitive load of navigating workspace
- loom's 30+ crates emerged from years of iteration — premature optimization

---

## Recommended Approach

**Start with Option C (Three-Crate) but design for Option D modularity.**

The three-crate architecture provides enough separation for testability and future SaaS while avoiding premature complexity. The key insight is that `lazyjob-core/` should be designed as if it will become a library, but we don't over-engineer the crate boundaries yet.

**Architectural layers:**

```
lazyjob-cli (binary)
    └── lazyjob-tui (tui entry point)
            └── lazyjob-core (domain)
                    ├── entities (Job, Application, LifeSheet, Company)
                    ├── traits (LlmProvider, JobSource, Persister)
                    └── services (matching, tailoring, company research)
```

**Ralph loop integration:**
- TUI spawns `ralph.sh` as a subprocess via `Command`
- Communication via files: TUI writes task context to `ralph_context.json`, ralph reads and writes results
- Ralph output streamed to TUI via file polling or `WaitForCondition` pattern
- Results stored in SQLite via the `ralph_outputs` table

**Data flow:**
1. User action in TUI → command to domain layer
2. Domain layer emits events (e.g., `JobDiscovered`, `ResumeTailored`)
3. TUI components subscribe to events and update their state
4. Background ralph loops write results to SQLite; TUI polls for updates

---

## Crate Map

### lazyjob-core/

**Purpose:** Pure domain logic, no I/O dependencies.

**Modules:**
- `entities/` — `Job`, `Application`, `LifeSheet`, `Company`, `Interview`, `Note`
- `traits/` — `LlmProvider`, `JobSource`, `Persister`, `Notifier`
- `services/` — `JobMatcher`, `ResumeTailorer`, `CoverLetterGenerator`, `CompanyResearcher`
- `errors.rs` — `LazyJobError` enum

**Dependencies:** `serde`, `chrono`, `uuid`, `thiserror`. No async, no database, no TUI.

### lazyjob-tui/

**Purpose:** All TUI presentation and interaction logic.

**Modules:**
- `app.rs` — `LazyJobApp` struct, main loop, event handling
- `components/` — `JobsList`, `JobDetail`, `ApplicationForm`, `LifeSheetEditor`, `RalphMonitor`, `Settings`
- `layouts/` — Panel layout calculations
- `keybindings/` — Keybinding definitions and dispatch
- `presentation/` — Text formatting, table rendering, form components
- `state.rs` — Application state management (context stack)

**Dependencies:** `ratatui`, `crossterm`, `lazyjob-core`. Async runtime internally (tokio for ralph subprocess communication).

### lazyjob-cli/

**Purpose:** Binary entry point, argument parsing, mode selection.

**Modules:**
- `main.rs` — Entry point
- `args.rs` — CLI argument definitions ( clap)
- `modes.rs` — Interactive vs daemon vs one-shot modes

**Dependencies:** `lazyjob-tui`, `lazyjob-core`, `clap`.

### Workspace Root

**Cargo.toml:**
```toml
[workspace]
resolver = "2"
members = ["lazyjob-core", "lazyjob-tui", "lazyjob-cli"]

[workspace.dependencies]
ratatui = "0.28"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

---

## Module Responsibilities

### Entities (lazyjob-core)

**Job:**
```rust
pub struct Job {
    pub id: Uuid,
    pub source: JobSource,
    pub source_id: String,        // ID in the source system
    pub title: String,
    pub company: String,
    pub company_id: Option<Uuid>, // Link to known company
    pub location: Option<String>,
    pub remote: RemoteOption,
    pub url: String,
    pub description: String,      // Full JD text
    pub embedding: Option<Vec<f32>>, // Semantic embedding
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    pub salary_currency: Option<String>,
    pub posted_at: Option<DateTime>,
    pub discovered_at: DateTime,
    pub status: JobStatus,
}

pub enum RemoteOption {
    Remote,
    Hybrid,
    OnSite,
    Unknown,
}

pub enum JobStatus {
    Discovered,
    Saved,
    Applied,
    Interviewing,
    Offer,
    Rejected,
    Dismissed,
}
```

**Application:**
```rust
pub struct Application {
    pub id: Uuid,
    pub job_id: Uuid,
    pub life_sheet_id: Uuid,
    pub status: ApplicationStatus,
    pub resume_path: Option<PathBuf>,
    pub cover_letter_path: Option<PathBuf>,
    pub submitted_at: Option<DateTime>,
    pub last_activity_at: DateTime,
    pub notes: Vec<Note>,
    pub interviews: Vec<Interview>,
    pub ralph_tasks: Vec<RalphTask>,
}

pub enum ApplicationStatus {
    Draft,
    Submitted,
    UnderReview,
    InterviewScheduled,
    OfferReceived,
    Rejected,
    Withdrawn,
}
```

**LifeSheet:**
```rust
pub struct LifeSheet {
    pub id: Uuid,
    pub person: Person,
    pub experience: Vec<Experience>,
    pub education: Vec<Education>,
    pub skills: Vec<Skill>,
    pub preferences: JobPreferences,
    pub compensation: CompensationExpectation,
    pub search_params: SearchParams,
}

pub struct Person {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub location: Option<String>,
    pub linkedin_url: Option<String>,
    pub github_url: Option<String>,
    pub portfolio_url: Option<String>,
}

pub struct Experience {
    pub id: Uuid,
    pub company: String,
    pub title: String,
    pub description: String,
    pub bullets: Vec<String>,      // Tailored resume bullets
    pub start_date: Option<Date>,
    pub end_date: Option<Date>,
    pub current: bool,
    pub skills: Vec<String>,
}

pub struct Skill {
    pub name: String,
    pub category: SkillCategory,
    pub level: SkillLevel,         // Expert, Proficient, Familiar
}

pub enum SkillCategory {
    Language,
    Framework,
    Tool,
    Domain,
    Soft,
}
```

### Traits (lazyjob-core)

**LlmProvider:**
```rust
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, LlmError>;
    async fn complete(&self, prompt: &str) -> Result<String, LlmError>;
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError>;
    fn provider_name(&self) -> &'static str;
    fn config(&self) -> &ProviderConfig;
}
```

**JobSource:**
```rust
pub trait JobSource: Send + Sync {
    async fn search(&self, params: &SearchParams) -> Result<Vec<Job>, SourceError>;
    async fn get_job(&self, source_id: &str) -> Result<Job, SourceError>;
    fn source_name(&self) -> &'static str;
}
```

**Persister:**
```rust
pub trait Persister: Send + Sync {
    async fn save_job(&self, job: &Job) -> Result<(), PersistError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<Job>, PersistError>;
    async fn list_jobs(&self, filter: &JobFilter) -> Result<Vec<Job>, PersistError>;
    async fn save_application(&self, app: &Application) -> Result<(), PersistError>;
    async fn get_life_sheet(&self) -> Result<Option<LifeSheet>, PersistError>;
    // ... full CRUD for all entities
}
```

### Services (lazyjob-core)

**JobMatcher:**
```rust
pub struct JobMatcher {
    llm: Arc<dyn LlmProvider>,
    embedder: Arc<dyn LlmProvider>, // Separate embedder model
}

impl JobMatcher {
    pub fn score_job(&self, job: &Job, life_sheet: &LifeSheet) -> f32 {
        // Semantic similarity: job description embedding vs life sheet skills
        // Keyword overlap: title, skills, experience
        // Preference matching: location, remote, salary range
        // Combined weighted score
    }

    pub fn generate_match_explanation(&self, job: &Job, life_sheet: &LifeSheet) -> String {
        // LLM-generated explanation of why this job matches
    }
}
```

**ResumeTailorer:**
```rust
pub struct ResumeTailorer {
    llm: Arc<dyn LlmProvider>,
}

impl ResumeTailorer {
    pub async fn tailor(
        &self,
        master_resume: &MasterResume,
        job: &Job,
        life_sheet: &LifeSheet,
    ) -> Result<TailoredResume, TailorError> {
        // 1. Extract key requirements from job description
        // 2. Select relevant experience from life_sheet
        // 3. Rewrite bullets to emphasize relevant skills
        // 4. Ensure ATS-friendly formatting
    }
}
```

### TUI Components (lazyjob-tui)

**LazyJobApp:**
```rust
pub struct LazyJobApp {
    pub state: AppState,
    pub db: Arc<DbPool>,          // SQLite connection pool
    pub llm_provider: Arc<dyn LlmProvider>,
    pub ralph_runner: RalphRunner, // Subprocess management
    pub event_tx: EventTx,         // TUI event emission
}

pub enum AppState {
    JobsList,
    JobDetail(Uuid),
    ApplicationForm(Uuid),  // Job ID being applied to
    LifeSheetEditor,
    RalphMonitor,
    Settings,
}
```

**Component implementations:**
- `JobsListComponent` — List of discovered/saved jobs with filtering and search
- `JobDetailComponent` — Full job info, apply button, match score
- `ApplicationFormComponent` — Multi-step form: resume selection, cover letter review, submit
- `LifeSheetEditorComponent` — CRUD for life sheet entities
- `RalphMonitorComponent` — Live view of running ralph loops, output streaming
- `SettingsComponent` — Provider config, keybindings, notification preferences

### Ralph Integration (lazyjob-tui)

**RalphRunner:**
```rust
pub struct RalphRunner {
    work_dir: PathBuf,
    task_tx: mpsc::Sender<RalphTask>,
    output_rx: mpsc::Receiver<RalphOutput>,
}

impl RalphRunner {
    pub fn spawn_loop(&self, task: RalphTask) -> Uuid {
        // Write task context to work_dir/task-{id}/context.json
        // Spawn ralph.sh subprocess
        // Track subprocess PID
        // Return task ID for monitoring
    }

    pub fn stream_output(&self, task_id: Uuid) -> impl Stream<Item: String> {
        // Poll output file, yield new lines since last check
    }

    pub fn cancel(&self, task_id: Uuid) -> Result<()> {
        // Send SIGTERM to subprocess
        // Clean up work_dir/task-{id}/
    }
}
```

---

## Data Flow

### Job Discovery Flow

```
1. User triggers discovery → RalphRunner.spawn_loop("job-discovery")
2. Ralph loop reads life_sheet from SQLite, queries job sources
3. New jobs written to jobs table via Persister trait
4. TUI polls job list every N seconds, re-renders
5. New jobs highlighted with notification badge
```

### Application Flow

```
1. User selects job, presses 'A' to apply
2. TUI shows ApplicationFormComponent (draft)
3. User reviews/edits resume and cover letter (generated by ralph)
4. User presses submit
5. TUI calls platform API (Greenhouse/Lever) via PlatformIntegrations
6. Application created in DB with status=Submitted
7. Ralph loop monitors application status (checks email, ATS)
8. Status changes surface in TUI notifications
```

### Life Sheet Editing Flow

```
1. User navigates to LifeSheetEditorComponent
2. TUI loads LifeSheet from SQLite via Persister
3. User edits through form components
4. On save: Persister.update_life_sheet() called
5. User can trigger "distill from resume" — calls LLM to parse PDF resume → LifeSheet
```

---

## Key Architectural Decisions

### 1. Immediate Mode TUI with Ratatui Component

Each TUI component implements `ratatui::Component`. The main loop:
```rust
loop {
    terminal.draw(|f| {
        app.render(f);
    })?;

    if let Event::Key(key) = event::read()? {
        app.handle_key(key);
    }
}
```

`render()` delegates to active component's `render()`. State flows top-down via `App` struct holding all state.

### 2. Ralph Subprocess with File-Based IPC

Ralph loops are OS subprocesses, not in-process async tasks. Rationale:
- Ralph loop is a fresh Claude CLI instance — cannot share memory with TUI anyway
- Process isolation means ralph crashes don't corrupt TUI state
- File-based communication matches ralph loop's design (progress.md, tasks.json on disk)
- Easy to interrupt with SIGTERM

### 3. SQLite with rusqlite in Main Thread, Background Writes

SQLite accessed from TUI main thread for reads. Writes go through a tokio task:
```rust
let db = Arc::new(Database::open("lazyjob.db")?);
// Reads happen synchronously in main thread
// Writes:
let db_clone = db.clone();
tokio::spawn(async move {
    db_clone.write(&application).await?;
});
```

SQLite WAL mode enabled for concurrent read/write from multiple processes (TUI + Ralph subprocesses access same DB via file system).

### 4. LLM Provider as Trait, Config in YAML

```yaml
# ~/.config/lazyjob/providers.yaml
providers:
  - name: anthropic
    type: anthropic
    api_key_env: ANTHROPIC_API_KEY
    model: claude-3-5-sonnet
  - name: ollama-local
    type: ollama
    base_url: http://localhost:11434
    model: llama3.2
```

Providers loaded at startup, first available used as default.

### 5. Event-Driven UI Updates

TUI doesn't actively poll Ralph subprocesses. Instead:
- Ralph writes outputs to SQLite `ralph_outputs` table
- TUI has a background tokio task that polls `ralph_outputs` every 2 seconds
- New output triggers an event to update the RalphMonitorComponent
- User sees streamed output in near-real-time

---

## Open Questions

1. **Ralph output streaming granularity**: Should Ralph emit output line-by-line or only on `<promise>COMPLETE</promise>`? Line-by-line is better UX but requires more file polling. Current ralph spec suggests complete output only.

2. **LazyJob daemon mode**: Should there be a background daemon (lazyjobd) that runs ralph loops even when TUI is closed? This would enable continuous job monitoring and morning briefs. Adds complexity — defer to v2.

3. **Platform API integration timing**: When does LazyJob call Greenhouse/Lever API to actually submit? The spec says user presses submit in TUI, but what about auto-apply? Need to handle authentication flows (OAuth tokens, session cookies).

4. **Life sheet distill accuracy**: The LLM-based resume-to-LifeSheet distillation will have errors. How does the user correct them? Inline editing vs. re-upload vs. structured form?

---

## Dependencies

- **ratatui** — TUI framework
- **crossterm** — Terminal input handling
- **tokio** — Async runtime, subprocess spawning
- **rusqlite** — SQLite (bundled mode)
- **serde** — Serialization
- **uuid** — Entity IDs
- **chrono** — Date/time handling
- **clap** — CLI argument parsing
- **rustyline** — Readline-style line editing for forms

---

## Sources

- https://github.com/jesseduffield/lazygit — Lazygit source code
- https://ratatui.rs/ — Ratatui documentation
- https://github.com/ratatui-org/ratatui — Ratatui source code
- https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html — Cargo workspace docs
- https://rust-analyzer.github.io/book/contributing/architecture.html — Rust-analyzer architecture
- https://deterministic.space/elegant-apis-in-rust.html — Elegant Rust API design