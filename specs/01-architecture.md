# LazyJob Architecture Overview

## Status
Researching

## Problem Statement

LazyJob is a lazygit-style terminal UI for job search management, built in Rust. It needs a scalable architecture that supports:
1. A rich, responsive TUI with multiple panels and views
2. Integration with ralph autonomous agent loops for AI-powered job search tasks
3. Local-first data persistence with SQLite
4. Multi-provider LLM abstraction for generative features
5. A path to SaaS without a complete rewrite

This spec defines the crate organization, component hierarchy, and foundational patterns.

---

## Research Findings

### Ratatui Architecture

Ratatui is the dominant Rust TUI library (forked from tui-rs, now under ratatui-org). Key architectural insights:

**Crate Organization (v0.30.0+)**
- `ratatui` - Main crate, re-exports everything for app developers
- `ratatui-core` - Foundational types, Widget/StatefulWidget traits, text rendering, buffer, layout, styles. Minimal stability surface for widget library authors.
- `ratatui-widgets` - Built-in widgets (Block, Paragraph, List, Table, Chart, Gauge, etc.)
- `ratatui-crossterm` - Cross-platform terminal backend (most common)
- `ratatui-termion` - Unix-specific backend
- `ratatui-termwiz` - Advanced terminal features backend
- `ratatui-macros` - Declarative macros for boilerplate

**Core Traits**
```rust
// Stateless widget
pub trait Widget {
    fn render(self, area: Rect, buf: &mut Buffer);
}

// Stateful widget with external state
pub trait StatefulWidget {
    type State: ?Sized;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}
```

**Layout System**
```rust
use ratatui_core::layout::{Constraint, Direction, Layout, Rect};

let [header, content, footer] = Layout::vertical([
    Constraint::Length(3),
    Constraint::Fill(1),
    Constraint::Length(1),
])
.areas(area);

let [sidebar, main] = Layout::horizontal([
    Constraint::Length(20),
    Constraint::Fill(1),
])
.areas(area);
```

**Constraint Types**
- `Constraint::Length(n)` - Fixed character count
- `Constraint::Percentage(n)` - Percentage of area
- `Constraint::Ratio(n, d)` - Ratio division
- `Constraint::Fill(n)` - Fill remaining space
- `Constraint::Min(n)` / `Constraint::Max(n)` - Min/max bounds

**Application Pattern**
```rust
use ratatui::Terminal;

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| {
        let mut app = App::new();
        loop {
            terminal.draw(|frame| app.render(frame))?;
            if app.handle_events()? {
                break Ok(());
            }
        }
    })
}
```

**Important**: Ratatui does NOT include input handling. Apps use `crossterm::event` directly for keyboard/mouse/terminal resize events.

**Styling System**
- Uses a builder pattern with method chaining: `Style::new().fg(Color::Blue).bg(Color::Black).bold()`
- Text modifiers: `bold()`, `italic()`, `underlined()`, `dim()`, `reversed()`, etc.
- Colors: 256-color palette + RGB support
- Block widget for borders, titles, padding

**Built-in Stateful Widgets**
| Widget | Associated State |
|--------|------------------|
| List | ListState |
| Table | TableState |
| Scrollbar | ScrollbarState |
| Tabs | TabState |

### Lazygit Architecture Patterns (Go)

Lazygit is the primary inspiration for LazyJob. While written in Go using gocui, its patterns are transferable:

**Repository Structure**
```
lazygit/
├── main.go              # Entry point
├── pkg/
│   ├── gui/             # Main GUI logic, view orchestration
│   ├── config/          # Configuration system
│   ├── commands/        # Git commands wrapping
│   ├── i18n/            # Internationalization
│   ├── system/          # OS-level operations
│   └── utils/           # Shared utilities
├── docs/                # User documentation
└── test/                # Integration tests
```

**Key Architectural Patterns**
1. **View/Window abstraction**: Lazygit manages multiple "views" (panels), each with its own keybinding context
2. **Layered rendering**: Views are rendered in layers (background commits, foreground worktrees, etc.)
3. **Selective keybinding**: Keybindings are context-aware (only active when certain panels are focused)
4. **Command pattern**: User actions are executed as "commands" with undo support
5. **State management**: Central app state struct holding all domain data, passed through command chain

**Keybinding Philosophy**
- `space` - Primary action (stage, toggle, select)
- `enter` - Open/detail view
- `q` / `esc` - Quit/back
- `v` - Range selection
- `?` - Help/overlay
- Navigation via vim-style `hjkl` or arrows

### Cargo Workspace Patterns

Based on loom's multi-crate architecture approach (researched via general patterns):

**Workspace Benefits**
1. Independent versioning of internal crates
2. Parallel compilation
3. Clear boundaries between subsystems
4. Can publish internal crates separately if needed
5. Enables `#[path = ...]` for large monolithic files

**Common Pattern**
```toml
[workspace]
members = [
    "lazyjob-core",
    "lazyjob-tui",
    "lazyjob-llm",
    "lazyjob-cli",
    "lazyjob-macros",
]
resolver = "2"  # Required for complex workspaces
```

---

## Design Options

### Option A: Monolithic Single Crate

**Description**: All code in a single `lazyjob` crate with modules.

**Pros**:
- Simplest to build and test
- No inter-crate dependency management
- Easy refactoring during initial development
- Single `Cargo.lock`, faster initial builds

**Cons**:
- Poor scalability past ~20k lines
- Can't publish internal subsystems independently
- Compilation times degrade as codebase grows
- Harder to enforce boundaries

**Best for**: MVP phase, small team, proving concept

### Option B: Two-Crate Split (Core + TUI)

**Description**:
- `lazyjob-core` - Domain models, business logic, persistence, LLM abstraction
- `lazyjob-tui` - Terminal UI, user interaction, presentation

```toml
[workspace]
members = ["lazyjob-core", "lazyjob-tui"]
```

**Pros**:
- Clear separation between logic and presentation
- Core can be unit tested without terminal
- Enables future headless operation (core as library)
- Better for parallel development
- Core can be published independently later

**Cons**:
- Still relatively simple structure
- Some circular dependency risk if not careful
- Requires disciplined module boundaries

**Best for**: Most projects, good balance of simplicity and scalability

### Option C: Three-Tier Crate Architecture

**Description**:
- `lazyjob-core` - Domain models, state machine, persistence interfaces
- `lazyjob-llm` - LLM provider abstraction, prompt templates, ralph integration
- `lazyjob-tui` - Terminal UI layer
- `lazyjob-cli` - Binary crate for CLI entry point
- `lazyjob-macros` - Procedural macros for boilerplate

**Pros**:
- Clean separation of concerns
- LLM layer can be tested independently
- Enables multiple UIs (TUI, headless, web) backed by core
- Best parallelism for large team
- Clear dependency graph

**Cons**:
- More complex build configuration
- Inter-crate API boundaries require discipline
- Potential for excessive abstraction
- Slower initial setup

**Best for**: Production-grade, multi-developer teams, long-term maintenance

### Option D: Loom-Style Multi-Crate (30+ Crates)

**Description**: Granular crates per component (following loom's pattern).

**Pros**:
- Maximum flexibility and reusability
- Each crate can evolve independently
- Fine-grained dependency control
- Enables open-sourcing individual components

**Cons**:
- Extremely complex workspace management
- Overhead for small features
- Dependency hell risk
- Requires sophisticated CI/CD
- Context-switching overhead for developers

**Best for**: Large organizations, proven stable components, potential open-source ecosystem

---

## Recommended Approach

**Option C: Three-Tier Crate Architecture** is recommended for LazyJob.

Rationale:
1. LazyJob is a non-trivial application with distinct layers (UI, AI, data)
2. The team is likely small to medium (startup/solo developer)
3. Need path to SaaS without rewrite
4. Need to integrate ralph loops (autonomous agents) - this is novel and should be isolated
5. Avoids the complexity of 30+ crates while still having clear boundaries

The `lazyjob-core` crate is the anchor - it should be usable headless. The TUI is the primary UI but could theoretically be swapped.

---

## LazyJob Crate Layout

```
lazyjob/
├── Cargo.toml              # Workspace definition
├── lazyjob-core/           # Core domain layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── models/         # Domain models (Job, Company, Application, Contact)
│       ├── state/          # Application state machine
│       ├── persistence/    # SQLite repository trait & implementation
│       ├── discovery/      # Job search and matching
│       └── error.rs        # Error types
├── lazyjob-llm/            # LLM abstraction layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── provider.rs     # Trait: LLMProvider
│       ├── anthropic.rs    # Anthropic implementation
│       ├── openai.rs       # OpenAI implementation
│       ├── ollama.rs       # Ollama (local) implementation
│       └── prompts/        # Prompt templates for ralph loops
├── lazyjob-ralph/          # Ralph loop integration
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── ipc.rs          # Unix socket / pipe communication
│       ├── process.rs      # Ralph subprocess management
│       └── state_sync.rs   # State synchronization
├── lazyjob-tui/            # Terminal UI layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── app.rs          # Main app struct, event loop
│       ├── views/          # View components
│       │   ├── mod.rs
│       │   ├── jobs.rs     # Job list view
│       │   ├── detail.rs   # Job detail view
│       │   ├── search.rs   # Search/filter view
│       │   ├── dashboard.rs # Overview dashboard
│       │   └── help.rs     # Help overlay
│       ├── widgets/        # Custom ratatui widgets
│       ├── keymap.rs       # Keybinding definitions
│       └── theme.rs        # Color scheme
├── lazyjob-cli/            # CLI binary
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── lazyjob-macros/         # Procedural macros
    ├── Cargo.toml
    └── src/
        └── lib.rs
```

### Dependency Graph

```
lazyjob-cli
    └── lazyjob-tui
            ├── lazyjob-ralph
            │       ├── lazyjob-llm
            │       │       └── lazyjob-core
            │       └── lazyjob-core
            └── lazyjob-core
```

---

## Data Model (High-Level)

### Core Domain Entities

```rust
// lazyjob-core/src/models/

pub struct Job {
    pub id: Uuid,
    pub title: String,
    pub company: Company,
    pub location: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub salary_range: Option<SalaryRange>,
    pub status: JobStatus,
    pub applied_at: Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub notes: String,
}

pub enum JobStatus {
    Discovered,
    Interested,
    Applied,
    PhoneScreen,
    Technical,
    Onsite,
    Offer,
    Rejected,
    Withdrawn,
}

pub struct Company {
    pub id: Uuid,
    pub name: String,
    pub website: Option<String>,
    pub industry: Option<String>,
    pub size: Option<CompanySize>,
    pub notes: String,
}

pub struct Application {
    pub id: Uuid,
    pub job_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub status: ApplicationStatus,
    pub resume_version: String,
    pub cover_letter_version: Option<String>,
    pub contacts: Vec<Contact>,
    pub follow_ups: Vec<FollowUp>,
}

pub struct LifeSheet {
    pub id: Uuid,
    pub personal: PersonalInfo,
    pub experience: Vec<Experience>,
    pub education: Vec<Education>,
    pub skills: Vec<Skill>,
    pub preferences: JobPreferences,
}

pub struct Contact {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub email: Option<String>,
    pub linkedin_url: Option<String>,
    pub company_id: Uuid,
    pub relationship: ContactRelationship,
    pub notes: String,
}
```

---

## API Surface

### lazyjob-core

```rust
// Persistence trait
pub trait Repository: Send + Sync {
    fn jobs(&self) -> Result<Vec<Job>>;
    fn job(&self, id: Uuid) -> Result<Option<Job>>;
    fn save_job(&mut self, job: Job) -> Result<()>;
    fn delete_job(&mut self, id: Uuid) -> Result<()>;
    fn applications(&self) -> Result<Vec<Application>>;
    fn save_application(&mut self, app: Application) -> Result<()>;
    // ... etc
}

// State management
pub struct AppState {
    pub jobs: HashMap<Uuid, Job>,
    pub applications: HashMap<Uuid, Application>,
    pub life_sheet: LifeSheet,
    pub current_view: View,
    pub filters: FilterSet,
}

pub trait StateMachine {
    fn transition(&mut self, event: AppEvent) -> Result<()>;
}
```

### lazyjob-llm

```rust
// LLM Provider trait
pub trait LLMProvider: Send + Sync {
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse>;
    async fn complete(&self, prompt: &str) -> Result<String>;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub enum ChatMessage {
    System { content: String },
    User { content: String },
    Assistant { content: String },
}

pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
}
```

### lazyjob-ralph

```rust
// Ralph subprocess communication
pub trait RalphIPC: Send + Sync {
    async fn send(&self, msg: RalphMessage) -> Result<()>;
    async fn recv(&self) -> Result<RalphMessage>;
    fn child_mut(&mut self) -> &mut Child;
}

pub enum RalphLoop {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrep,
    SalaryNegotiation,
}
```

### lazyjob-tui

```rust
// Main app interface
pub trait TUIApp {
    fn draw(&mut self, frame: &mut Frame);
    fn handle_events(&mut self, event: Event) -> Result<bool>;
    fn state(&self) -> &AppState;
    fn state_mut(&mut self) -> &mut AppState;
}
```

---

## TUI View Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│ Header: [LazyJob]  [Dashboard]  [Jobs]  [Search]  [Settings]   │
├─────────────┬───────────────────────────────────────────────────┤
│             │                                                   │
│  Sidebar    │              Main Content Area                   │
│  (context-  │                                                   │
│  dependent) │  ┌─────────────────────────────────────────────┐ │
│             │  │                                             │ │
│  - Job list │  │         Active View Content                 │ │
│  - Filters  │  │                                             │ │
│  - Contacts │  │                                             │ │
│             │  │                                             │ │
│             │  └─────────────────────────────────────────────┘ │
│             │                                                   │
├─────────────┴───────────────────────────────────────────────────┤
│ Status Bar: [Job count] [Current filter] [LLM status] [Time] │
└─────────────────────────────────────────────────────────────────┘
```

### View States

1. **Dashboard**: Overview statistics, recent activity, upcoming follow-ups
2. **Jobs List**: Filterable list of all jobs with status indicators
3. **Job Detail**: Full job info, company research, application status
4. **Search**: Advanced search with embedding-based similarity
5. **Applications**: Kanban-style application pipeline view
6. **Contacts**: Networking contacts and referral tracking
7. **Settings**: Configuration, LLM provider setup, data export
8. **Help Overlay**: Full keybinding reference (lazygit-style `?`)

### Keybinding Philosophy

Inspired by lazygit (vim-style):
- `hjkl` or arrows: Navigation
- `space`: Toggle/select primary action
- `enter`: Open detail view
- `e`: Edit
- `d`: Delete
- `a`: Add new
- `/`: Search/filter
- `?`: Help overlay
- `q` / `esc`: Back/quit
- `:`: Command palette
- `ctrl+r`: Refresh/sync
- `tab` / `shift+tab`: Switch panels

---

## Failure Modes

1. **LLM Provider Failure**: If Anthropic/OpenAI API is down, fall back to local Ollama or show cached results with staleness indicator
2. **Ralph Subprocess Crash**: Auto-restart ralph loop, preserve state in SQLite, show non-intrusive error in status bar
3. **SQLite Corruption**: WAL mode provides durability; backup on startup; export to JSON periodically
4. **Terminal Resize**: Re-calculate Layout constraints on resize events; minimum size enforced
5. **Unicode/Encoding Issues**: Use `unicode-width` crate; validate text before rendering
6. **Network Failures**: Offline-first; queue API calls for retry; show network status indicator

---

## Open Questions

1. **Ralph Loop Lifecycle**: How does the TUI start/stop/manage ralph subprocesses? Need to define IPC protocol.
2. **State Persistence Granularity**: Full state in SQLite on every change, or periodic snapshots + WAL?
3. **Undo/Redo**: Does LazyJob need command-level undo like lazygit? Initially probably not.
4. **Multi-user / Sharing**: Future SaaS - does local data model support multi-user?
5. **Plugin/Extension System**: Lazygit has custom commands. Should LazyJob have a plugin system?

---

## Dependencies

- **ratatui** + **ratatui-crossterm**: UI framework
- **tokio**: Async runtime for LLM calls and ralph IPC
- **rusqlite**: SQLite persistence (or sqlx with SQLite feature)
- **uuid**: ID generation
- **chrono**: Date/time handling
- **serde** + **serde_yaml**: Serialization for life sheet config
- **thiserror**: Error handling
- **tracing**: Logging

---

## Sources

- [Ratatui Documentation](https://ratatui.rs/)
- [Ratatui Widget Examples](https://github.com/ratatui-org/ratatui/tree/main/ratatui-widgets/examples)
- [Ratatui App Examples](https://github.com/ratatui-org/ratatui/tree/main/examples)
- [Lazygit Repository](https://github.com/jesseduffield/lazygit)
- [Lazygit Keybindings Documentation](https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings)
- [Crossterm Event Handling](https://docs.rs/crossterm/latest/crossterm/event/)
- [Ratatui Architecture (YouTube - EuroRust 2024)](https://www.youtube.com/watch?v=hWG51Mc1DlM)
