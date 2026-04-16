# Implementation Plan: TUI Design & Keybindings

## Status
Draft

## Related Spec
`specs/09-tui-design-keybindings.md`

## Overview

This plan specifies how to build the LazyJob terminal UI using `ratatui` and `crossterm`. The TUI is the only user-facing interface: a lazygit-inspired, vim-modal, local-first terminal dashboard that renders job search state and dispatches commands to the persistence and agent layers.

The design follows a top-level `App` state machine that owns all view states and dispatches input events. Views are composed of stateless `Widget` impls that borrow slices of `App`. A central `EventLoop` drives rendering at ~60 fps and processes `crossterm` events. A separate `TuiMessage` channel receives updates from async Ralph subprocesses without blocking the render thread.

The implementation is phased: Phase 1 delivers a runnable skeleton with header, navigation, status bar, and the Jobs List view. Phase 2 adds the remaining views (Dashboard, Applications pipeline, Contacts, Ralph panel, Settings). Phase 3 covers modal dialogs, help overlay, configurable keymaps, and advanced vim motions.

## Prerequisites

### Specs/plans that must be implemented first
- `specs/01-architecture-implementation-plan.md` — workspace layout must exist
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, `JobRepository`, `ApplicationRepository`, `ContactRepository` must be callable

### Crates to add to `lazyjob-tui/Cargo.toml`
```toml
[dependencies]
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
futures = "0.3"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
unicode-width = "0.2"         # accurate terminal column widths for CJK/emoji
crossbeam-channel = "0.5"     # sync channel from async tasks to render loop

lazyjob-core = { path = "../lazyjob-core" }
lazyjob-llm  = { path = "../lazyjob-llm" }
lazyjob-ralph = { path = "../lazyjob-ralph" }
```

---

## Architecture

### Crate Placement

All TUI code lives in `lazyjob-tui`. The binary entry point (`lazyjob-cli/src/main.rs`) calls `lazyjob_tui::run(config, db)`. Nothing in `lazyjob-tui` should contain domain logic; it reads from repositories and sends commands via service traits.

### Core Types

```rust
// lazyjob-tui/src/app.rs

/// Top-level application state. Owns all view-specific states.
pub struct App {
    pub active_view: View,
    pub prev_view: Option<View>,          // for `escape` nav
    pub modal: Option<Modal>,
    pub help_open: bool,
    pub status_bar: StatusBarState,
    pub input_mode: InputMode,

    // Per-view states
    pub dashboard: DashboardState,
    pub jobs: JobsState,
    pub applications: ApplicationsState,
    pub contacts: ContactsState,
    pub ralph: RalphState,
    pub settings: SettingsState,

    // Cross-cutting
    pub db: Arc<Database>,
    pub config: Arc<Config>,
    pub ralph_rx: Receiver<RalphUpdate>,  // crossbeam Receiver
    pub should_quit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Jobs,
    JobDetail(JobId),
    Applications,
    ApplicationDetail(ApplicationId),
    Contacts,
    ContactDetail(ContactId),
    Ralph,
    RalphDetail(LoopId),
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,          // text field focused
    Command,         // `:` ex-command bar
    Search,          // `/` search active
}

// lazyjob-tui/src/modal.rs
#[derive(Debug, Clone)]
pub enum Modal {
    Confirm {
        title: String,
        body: String,
        on_confirm: Action,
    },
    InputDialog {
        title: String,
        fields: Vec<InputField>,
        on_submit: Action,
    },
    Alert {
        title: String,
        body: String,
    },
}

#[derive(Debug, Clone)]
pub struct InputField {
    pub label: &'static str,
    pub value: String,
    pub cursor: usize,
    pub secure: bool,
}

// lazyjob-tui/src/action.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    NavigateTo(View),
    NavigateBack,
    ToggleHelp,
    OpenModal(Box<ModalSpec>),
    CloseModal,
    Refresh,
    // Job actions
    DeleteJob(JobId),
    ApplyToJob(JobId),
    TailorResume(JobId),
    // Application actions
    AdvanceApplication(ApplicationId),
    RegressApplication(ApplicationId),
    DeleteApplication(ApplicationId),
    // Contact actions
    DeleteContact(ContactId),
    // Ralph actions
    StopLoop(LoopId),
    StartLoop(LoopKind),
    // Settings actions
    SaveSettings,
}

// lazyjob-tui/src/types.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApplicationId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContactId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopId(pub String);
```

### View State Types

```rust
// lazyjob-tui/src/views/jobs.rs
pub struct JobsState {
    pub items: Vec<JobSummary>,
    pub list_state: ListState,         // ratatui::widgets::ListState
    pub filter: JobFilter,
    pub filter_panel_open: bool,
    pub search_query: String,
    pub loading: bool,
    pub error: Option<String>,
    pub selected_for_bulk: HashSet<JobId>,
}

// lazyjob-tui/src/views/applications.rs
pub struct ApplicationsState {
    pub columns: Vec<PipelineColumn>,
    pub focused_column: usize,
    pub focused_card: usize,
    pub selected_for_bulk: HashSet<ApplicationId>,
    pub loading: bool,
}

pub struct PipelineColumn {
    pub stage: ApplicationStage,
    pub cards: Vec<ApplicationCard>,
}

pub struct ApplicationCard {
    pub id: ApplicationId,
    pub company: String,
    pub title: String,
    pub applied_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_contact_at: Option<chrono::DateTime<chrono::Utc>>,
}

// lazyjob-tui/src/views/ralph.rs
pub struct RalphState {
    pub active_loops: Vec<LoopEntry>,
    pub completed_loops: Vec<LoopEntry>,
    pub list_state: ListState,
    pub selected_loop: Option<LoopId>,
    pub log_scroll: usize,
}

pub struct LoopEntry {
    pub id: LoopId,
    pub kind: LoopKind,
    pub phase: String,
    pub progress: f64,            // 0.0..=1.0
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub log_lines: Vec<String>,
    pub status: LoopStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStatus {
    Running,
    Paused,
    Completed,
    Failed(String),
}

// lazyjob-tui/src/views/dashboard.rs
pub struct DashboardState {
    pub stats: Option<DashboardStats>,
    pub recent_activity: Vec<ActivityEntry>,
    pub upcoming_reminders: Vec<ReminderEntry>,
    pub recommended_jobs: Vec<JobSummary>,
    pub loading: bool,
}

pub struct DashboardStats {
    pub jobs_discovered: u32,
    pub applications: u32,
    pub interviews: u32,
    pub offers: u32,
}
```

### Theme and Color Scheme

```rust
// lazyjob-tui/src/theme.rs
use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub primary:        Color,
    pub secondary:      Color,
    pub success:        Color,
    pub warning:        Color,
    pub error:          Color,
    pub text_primary:   Color,
    pub text_secondary: Color,
    pub text_muted:     Color,
    pub bg_primary:     Color,
    pub bg_secondary:   Color,
    pub bg_elevated:    Color,
    pub border:         Color,
    pub border_focused: Color,
}

impl Theme {
    pub const DARK: Self = Self {
        primary:        Color::LightBlue,
        secondary:      Color::DarkGray,
        success:        Color::LightGreen,
        warning:        Color::LightYellow,
        error:          Color::LightRed,
        text_primary:   Color::White,
        text_secondary: Color::Gray,
        text_muted:     Color::DarkGray,
        bg_primary:     Color::Black,
        bg_secondary:   Color::Rgb(30, 30, 30),
        bg_elevated:    Color::DarkGray,
        border:         Color::DarkGray,
        border_focused: Color::LightBlue,
    };

    pub fn status_bar_style(&self) -> Style {
        Style::default().fg(self.text_secondary).bg(self.bg_secondary)
    }

    pub fn selected_style(&self) -> Style {
        Style::default()
            .fg(self.text_primary)
            .bg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn focused_border_style(&self) -> Style {
        Style::default().fg(self.border_focused)
    }

    pub fn unfocused_border_style(&self) -> Style {
        Style::default().fg(self.border)
    }
}
```

### Keybinding System

```rust
// lazyjob-tui/src/keybindings.rs
use crossterm::event::{KeyCode, KeyModifiers};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyCombo {
    pub fn plain(code: KeyCode) -> Self {
        Self { code, mods: KeyModifiers::NONE }
    }
    pub fn ctrl(code: KeyCode) -> Self {
        Self { code, mods: KeyModifiers::CONTROL }
    }
    pub fn shift(code: KeyCode) -> Self {
        Self { code, mods: KeyModifiers::SHIFT }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyContext {
    Global,
    Jobs,
    JobDetail,
    Applications,
    Contacts,
    Ralph,
    Settings,
    Modal,
    Search,
}

/// A flat table of (context, combo) → Action.
pub struct KeyMap(HashMap<(KeyContext, KeyCombo), Action>);

impl KeyMap {
    pub fn default_keymap() -> Self {
        let mut m = HashMap::new();

        // Global
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('?'))), Action::ToggleHelp);
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('q'))), Action::Quit);
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Esc)), Action::NavigateBack);
        m.insert((KeyContext::Global, KeyCombo::ctrl(KeyCode::Char('r'))), Action::Refresh);
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('1'))), Action::NavigateTo(View::Dashboard));
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('2'))), Action::NavigateTo(View::Jobs));
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('3'))), Action::NavigateTo(View::Applications));
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('4'))), Action::NavigateTo(View::Contacts));
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('5'))), Action::NavigateTo(View::Ralph));
        m.insert((KeyContext::Global, KeyCombo::plain(KeyCode::Char('6'))), Action::NavigateTo(View::Settings));

        // Jobs context
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('j'))), Action::JobsDown);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Down)), Action::JobsDown);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('k'))), Action::JobsUp);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Up)), Action::JobsUp);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Enter)), Action::JobsOpen);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('/'))), Action::SearchFocus);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('f'))), Action::ToggleFilterPanel);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('n'))), Action::AddJob);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('d'))), Action::DeleteJobPrompt);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char(' '))), Action::JobToggleSelect);
        m.insert((KeyContext::Jobs, KeyCombo::plain(KeyCode::Char('r'))), Action::Refresh);

        Self(m)
    }

    /// Resolve an action for a context and key press. Falls back to Global.
    pub fn resolve(&self, ctx: &KeyContext, combo: &KeyCombo) -> Option<&Action> {
        self.0.get(&(ctx.clone(), combo.clone()))
            .or_else(|| self.0.get(&(KeyContext::Global, combo.clone())))
    }
}

/// Configurable keymap loaded from TOML — merges on top of defaults.
#[derive(Debug, serde::Deserialize, Default)]
pub struct KeyMapOverrides {
    pub global: HashMap<String, String>,
    pub jobs: HashMap<String, String>,
    pub applications: HashMap<String, String>,
    pub contacts: HashMap<String, String>,
    pub ralph: HashMap<String, String>,
    pub settings: HashMap<String, String>,
}
```

### Layout System

```rust
// lazyjob-tui/src/layout.rs
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub const HEADER_HEIGHT: u16 = 3;
pub const STATUS_BAR_HEIGHT: u16 = 1;
pub const SIDEBAR_MIN_WIDTH: u16 = 28;
pub const SIDEBAR_MAX_RATIO: f32 = 0.3;

pub struct AppLayout {
    pub header: Rect,
    pub body: Rect,
    pub status_bar: Rect,
}

pub struct BodyLayout {
    pub sidebar: Rect,
    pub content: Rect,
}

impl AppLayout {
    pub fn compute(frame_size: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Fill(1),
                Constraint::Length(STATUS_BAR_HEIGHT),
            ])
            .split(frame_size);
        Self {
            header:     chunks[0],
            body:       chunks[1],
            status_bar: chunks[2],
        }
    }
}

impl BodyLayout {
    pub fn compute(body: Rect, sidebar_visible: bool) -> Self {
        if !sidebar_visible || body.width < SIDEBAR_MIN_WIDTH * 2 {
            return Self { sidebar: Rect::default(), content: body };
        }
        let sidebar_w = ((body.width as f32 * SIDEBAR_MAX_RATIO) as u16)
            .max(SIDEBAR_MIN_WIDTH)
            .min(40);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(sidebar_w),
                Constraint::Fill(1),
            ])
            .split(body);
        Self { sidebar: chunks[0], content: chunks[1] }
    }
}
```

### Module Structure

```
lazyjob-tui/
  src/
    lib.rs                  # pub fn run(config, db) -> Result<()>
    app.rs                  # App, View, InputMode
    action.rs               # Action enum
    types.rs                # JobId, ApplicationId, ContactId, LoopId
    theme.rs                # Theme struct, DARK constant
    keybindings.rs          # KeyCombo, KeyContext, KeyMap, KeyMapOverrides
    layout.rs               # AppLayout, BodyLayout, constants
    event_loop.rs           # EventLoop, TuiMessage, tick/render/event cycle
    modal.rs                # Modal, InputField, ModalSpec
    router.rs               # input_to_action(), handle_action()
    status_bar.rs           # StatusBarState, render_status_bar()
    header.rs               # render_header(), tab navigation
    help.rs                 # HelpOverlay widget
    views/
      mod.rs
      dashboard.rs          # DashboardState, render_dashboard()
      jobs.rs               # JobsState, render_jobs(), render_job_detail()
      applications.rs       # ApplicationsState, PipelineColumn, render_applications()
      contacts.rs           # ContactsState, render_contacts(), render_contact_detail()
      ralph.rs              # RalphState, LoopEntry, render_ralph()
      settings.rs           # SettingsState, render_settings()
    widgets/
      mod.rs
      job_card.rs           # JobCard widget
      application_card.rs   # ApplicationCard widget
      contact_card.rs       # ContactCard widget
      stat_block.rs         # StatBlock (4-up stats grid)
      progress_bar.rs       # ProgressBar with label
      filter_panel.rs       # FilterPanel (sidebar)
      confirm_dialog.rs     # ConfirmDialog modal
      input_dialog.rs       # InputDialog modal (multi-field)
      scrollable_text.rs    # ScrollableText for long descriptions
      pipeline_board.rs     # Kanban board renderer
```

---

## Implementation Phases

### Phase 1 — Skeleton + Jobs List (MVP)

**Goal**: A runnable terminal app with header, navigation tabs, Jobs List view (populated from SQLite), status bar, and basic `j/k`/`enter` keybindings.

#### Step 1.1 — Create crate scaffold

**File**: `lazyjob-tui/Cargo.toml`

Add the dependencies listed above. Declare the crate as `lib` + `bin` split:
```toml
[lib]
name = "lazyjob_tui"
path = "src/lib.rs"
```

**Verification**: `cargo check -p lazyjob-tui` passes.

#### Step 1.2 — `run()` entry point

**File**: `lazyjob-tui/src/lib.rs`

```rust
pub mod app;
pub mod action;
pub mod event_loop;
pub mod keybindings;
pub mod layout;
pub mod modal;
pub mod router;
pub mod status_bar;
pub mod theme;
pub mod types;
pub mod views;
pub mod widgets;
mod header;
mod help;

pub use app::App;

pub async fn run(config: Arc<lazyjob_core::config::Config>, db: Arc<lazyjob_core::persistence::Database>) -> anyhow::Result<()> {
    let app = App::new(config, db).await?;
    event_loop::EventLoop::new(app).run().await
}
```

**Key API**: `crossterm::terminal::enable_raw_mode()`, `ratatui::Terminal::new(CrosstermBackend::new(stdout()))`.

#### Step 1.3 — EventLoop

**File**: `lazyjob-tui/src/event_loop.rs`

```rust
use crossterm::event::{Event, EventStream, KeyCode};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::Stdout;
use tokio::time::{self, Duration};

const TICK_RATE: Duration = Duration::from_millis(16); // ~60 fps

pub struct EventLoop {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    app: App,
}

impl EventLoop {
    pub fn new(app: App) -> anyhow::Result<Self> { ... }

    pub async fn run(mut self) -> anyhow::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        // enter alternate screen
        crossterm::execute!(std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        )?;

        let mut event_reader = EventStream::new();
        let mut tick = time::interval(TICK_RATE);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    self.terminal.draw(|f| render(f, &mut self.app))?;
                }
                Some(Ok(event)) = event_reader.next() => {
                    if let Event::Key(key) = event {
                        let action = router::key_to_action(&self.app, key);
                        if let Some(action) = action {
                            handle_action(&mut self.app, action).await?;
                        }
                    }
                    if let Event::Resize(_, _) = event {
                        // ratatui handles this automatically on next draw
                    }
                }
                // Ralph updates from async channel
                update = self.app.ralph_rx.recv_async() => {
                    if let Ok(update) = update {
                        self.app.apply_ralph_update(update);
                    }
                }
            }

            if self.app.should_quit {
                break;
            }
        }

        // restore terminal
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        )?;
        Ok(())
    }
}
```

**Key APIs**:
- `crossterm::event::EventStream` — async event stream from `crossterm` with `event-stream` feature
- `futures::StreamExt::next()` — poll next event as `Future`
- `ratatui::Terminal::draw()` — renders a single frame
- `tokio::select!` — multiplex tick + event + ralph channel

**Verification**: Application launches, shows blank screen, exits on `q`.

#### Step 1.4 — Top-level render function

**File**: `lazyjob-tui/src/event_loop.rs` (or `render.rs`)

```rust
pub fn render(frame: &mut ratatui::Frame, app: &mut App) {
    let layout = AppLayout::compute(frame.area());

    header::render_header(frame, layout.header, app);
    status_bar::render_status_bar(frame, layout.status_bar, app);

    match &app.active_view.clone() {
        View::Dashboard => views::dashboard::render(frame, layout.body, app),
        View::Jobs => views::jobs::render(frame, layout.body, app),
        View::JobDetail(id) => views::jobs::render_detail(frame, layout.body, app, id),
        View::Applications => views::applications::render(frame, layout.body, app),
        View::Contacts => views::contacts::render(frame, layout.body, app),
        View::Ralph => views::ralph::render(frame, layout.body, app),
        View::Settings => views::settings::render(frame, layout.body, app),
        _ => {}
    }

    if let Some(modal) = &app.modal.clone() {
        modal::render_modal(frame, frame.area(), modal, app);
    }

    if app.help_open {
        help::render_help_overlay(frame, frame.area(), &app.active_view, &app.keymap);
    }
}
```

**Key API**: `ratatui::Frame::area()` returns the full terminal `Rect`; `ratatui::widgets::Clear` + centered `Rect` for overlays.

#### Step 1.5 — Header widget

**File**: `lazyjob-tui/src/header.rs`

Renders a `Block` with the LazyJob title and tab labels. Highlights the active tab. Uses `ratatui::widgets::Tabs`.

```rust
pub fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let tab_titles = vec!["Dashboard", "Jobs", "Applications", "Contacts", "Ralph", "Settings"];
    let selected = match &app.active_view {
        View::Dashboard => 0,
        View::Jobs | View::JobDetail(_) => 1,
        View::Applications | View::ApplicationDetail(_) => 2,
        View::Contacts | View::ContactDetail(_) => 3,
        View::Ralph | View::RalphDetail(_) => 4,
        View::Settings => 5,
    };
    let tabs = Tabs::new(tab_titles)
        .select(selected)
        .highlight_style(app.theme.selected_style())
        .divider("|");
    frame.render_widget(tabs, area);
}
```

**Key APIs**:
- `ratatui::widgets::Tabs::new()`, `.select()`, `.highlight_style()`
- `ratatui::style::Style`, `ratatui::style::Modifier`

#### Step 1.6 — Status bar

**File**: `lazyjob-tui/src/status_bar.rs`

```rust
pub struct StatusBarState {
    pub job_count: u32,
    pub active_filter: Option<String>,
    pub matched_jobs: u32,
    pub ralph_status: RalphStatus,
}

#[derive(Clone, Copy)]
pub enum RalphStatus { Idle, Running, Error }

pub fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let sb = &app.status_bar;
    let ralph_symbol = match sb.ralph_status {
        RalphStatus::Idle    => "○",
        RalphStatus::Running => "●",
        RalphStatus::Error   => "✗",
    };
    let now = chrono::Local::now().format("%H:%M");
    let text = format!(
        " Jobs: {}  Matched: {}  Ralph: {}  {} ",
        sb.job_count, sb.matched_jobs, ralph_symbol, now,
    );
    let p = Paragraph::new(text).style(app.theme.status_bar_style());
    frame.render_widget(p, area);
}
```

#### Step 1.7 — Jobs List view

**File**: `lazyjob-tui/src/views/jobs.rs`

Renders a two-column split: filter panel (sidebar) + scrollable list (content).

```rust
pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let body = BodyLayout::compute(area, app.jobs.filter_panel_open);
    if app.jobs.filter_panel_open {
        widgets::filter_panel::render(frame, body.sidebar, app);
    }
    render_jobs_list(frame, body.content, app);
}

fn render_jobs_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = app.jobs.items.iter().map(|j| {
        let line = Line::from(vec![
            Span::styled(format!("{:<30}", j.title), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(format!("{:<20}", j.company), Style::default().fg(Color::LightBlue)),
            Span::raw("  "),
            Span::styled(j.location.as_deref().unwrap_or(""), Style::default().fg(Color::Gray)),
        ]);
        ListItem::new(line)
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Jobs"))
        .highlight_style(app.theme.selected_style())
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.jobs.list_state);
}
```

**Key APIs**:
- `ratatui::widgets::List::new()`, `ListItem`, `ListState` — stateful widget for selection
- `ratatui::widgets::Block`, `ratatui::widgets::Borders`
- `ratatui::text::{Line, Span}`, `ratatui::style::Style`
- `frame.render_stateful_widget(widget, area, state)` — renders widget with mutable state

#### Step 1.8 — Router: key → action

**File**: `lazyjob-tui/src/router.rs`

```rust
pub fn key_to_action(app: &App, key: crossterm::event::KeyEvent) -> Option<Action> {
    let ctx = active_context(app);
    let combo = KeyCombo { code: key.code, mods: key.modifiers };

    // Modal intercepts all keys when open
    if app.modal.is_some() {
        return modal_key_handler(app, key);
    }
    // Search mode intercepts printable chars
    if app.input_mode == InputMode::Search {
        return search_key_handler(app, key);
    }

    app.keymap.resolve(&ctx, &combo).cloned()
}

fn active_context(app: &App) -> KeyContext {
    match &app.active_view {
        View::Jobs | View::JobDetail(_) => KeyContext::Jobs,
        View::Applications | View::ApplicationDetail(_) => KeyContext::Applications,
        View::Contacts | View::ContactDetail(_) => KeyContext::Contacts,
        View::Ralph | View::RalphDetail(_) => KeyContext::Ralph,
        View::Settings => KeyContext::Settings,
        _ => KeyContext::Global,
    }
}
```

#### Step 1.9 — Action handler

**File**: `lazyjob-tui/src/router.rs`

```rust
pub async fn handle_action(app: &mut App, action: Action) -> anyhow::Result<()> {
    match action {
        Action::Quit => app.should_quit = true,
        Action::NavigateTo(view) => {
            app.prev_view = Some(app.active_view.clone());
            app.active_view = view;
            app.refresh_current_view().await?;
        }
        Action::NavigateBack => {
            if let Some(prev) = app.prev_view.take() {
                app.active_view = prev;
            } else {
                app.active_view = View::Dashboard;
            }
        }
        Action::ToggleHelp => app.help_open = !app.help_open,
        Action::Refresh => app.refresh_current_view().await?,
        Action::JobsDown => app.jobs.list_state.select_next(),
        Action::JobsUp   => app.jobs.list_state.select_previous(),
        Action::JobsOpen => {
            if let Some(idx) = app.jobs.list_state.selected() {
                if let Some(job) = app.jobs.items.get(idx) {
                    let id = job.id.clone();
                    app.prev_view = Some(app.active_view.clone());
                    app.active_view = View::JobDetail(id);
                }
            }
        }
        Action::DeleteJobPrompt => {
            // open confirm modal, on_confirm → Action::DeleteJob(id)
        }
        // ... all other variants
        _ => {}
    }
    Ok(())
}
```

**Key APIs**:
- `ratatui::widgets::ListState::select_next()`, `select_previous()` — built into ratatui 0.29

#### Step 1.10 — Data loading

**File**: `lazyjob-tui/src/app.rs`

```rust
impl App {
    pub async fn new(config: Arc<Config>, db: Arc<Database>) -> anyhow::Result<Self> {
        let (ralph_tx, ralph_rx) = crossbeam_channel::unbounded();
        let mut app = Self {
            active_view: View::Dashboard,
            db, config,
            keymap: KeyMap::default_keymap(),
            theme: Theme::DARK,
            ralph_rx,
            // ... zero-init all states
        };
        app.load_jobs().await?;
        Ok(app)
    }

    pub async fn load_jobs(&mut self) -> anyhow::Result<()> {
        self.jobs.loading = true;
        let repo = JobRepository::new(self.db.pool());
        let items = repo.list(&self.jobs.filter).await
            .map_err(|e| anyhow::anyhow!("load jobs: {e}"))?;
        self.jobs.items = items.into_iter().map(JobSummary::from).collect();
        self.jobs.loading = false;
        Ok(())
    }

    pub async fn refresh_current_view(&mut self) -> anyhow::Result<()> {
        match &self.active_view.clone() {
            View::Dashboard => self.load_dashboard().await?,
            View::Jobs => self.load_jobs().await?,
            View::Applications => self.load_applications().await?,
            View::Contacts => self.load_contacts().await?,
            View::Ralph => {} // Ralph state is pushed via channel
            _ => {}
        }
        Ok(())
    }
}
```

**Verification**: `cargo run` shows Jobs List populated from in-memory SQLite; `j/k` scrolls; `enter` navigates to JobDetail placeholder; `q` quits cleanly.

---

### Phase 2 — All Views

#### Step 2.1 — Dashboard View

**File**: `lazyjob-tui/src/views/dashboard.rs`

Four-quadrant layout using `Layout::horizontal` + `Layout::vertical`. Statistics are loaded async in `App::load_dashboard()` via SQL aggregate queries:

```sql
SELECT
  COUNT(*) FILTER (WHERE 1=1) AS total_jobs,
  COUNT(*) FILTER (WHERE status='applied') AS applied,
  COUNT(*) FILTER (WHERE status='interview') AS interviews
FROM applications;
```

Rendered as a `ratatui::widgets::Table` or custom `StatBlock` widget.

**Key APIs**: `ratatui::widgets::Table`, `ratatui::widgets::Row`, `ratatui::widgets::Cell`.

#### Step 2.2 — Job Detail View

**File**: `lazyjob-tui/src/views/jobs.rs` (`render_detail` function)

Full-page scrollable view. Long description rendered via `ScrollableText` widget (wraps `ratatui::widgets::Paragraph` with scroll offset). Match percentage rendered as a custom `ProgressBar`.

```rust
pub fn render_detail(frame: &mut Frame, area: Rect, app: &mut App, id: &JobId) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),  // header meta (company, salary, etc.)
            Constraint::Length(4),  // action bar
            Constraint::Fill(1),    // description
            Constraint::Length(8),  // match score panel
        ])
        .split(area);
    // render each section...
}
```

#### Step 2.3 — Applications Pipeline View

**File**: `lazyjob-tui/src/views/applications.rs`

Horizontal kanban board. Each column is rendered using `ratatui::widgets::Block` + inner `List`. Column width is computed as `area.width / num_visible_stages`.

Navigation: `h/l` moves between columns; `j/k` selects within column. `m` advances the selected card to the next stage (calls `ApplicationRepository::advance_stage()` with a confirmation modal).

```rust
pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let visible_stages = [
        ApplicationStage::Discovered,
        ApplicationStage::Interested,
        ApplicationStage::Applied,
        ApplicationStage::PhoneScreen,
        ApplicationStage::Technical,
        ApplicationStage::OnSite,
        ApplicationStage::Offer,
    ];
    let col_width = area.width / visible_stages.len() as u16;
    let chunks: Vec<Rect> = (0..visible_stages.len())
        .map(|i| Rect::new(area.x + (i as u16 * col_width), area.y, col_width, area.height))
        .collect();

    for (i, (stage, chunk)) in visible_stages.iter().zip(chunks.iter()).enumerate() {
        let is_focused = i == app.applications.focused_column;
        let column = app.applications.columns.iter().find(|c| &c.stage == stage);
        widgets::pipeline_board::render_column(frame, *chunk, column, is_focused, &app.theme);
    }
}
```

**Key APIs**: Custom column layout (manual `Rect` arithmetic since ratatui doesn't support N equal columns via `Layout::constraints`).

#### Step 2.4 — Ralph Panel

**File**: `lazyjob-tui/src/views/ralph.rs`

Split view: active loop list on left, loop detail (progress bar + log) on right. Log lines rendered via `ScrollableText` with autoscroll when at bottom.

`RalphUpdate` messages arrive via `crossbeam_channel::Receiver<RalphUpdate>`. In `EventLoop::run()`, the select arm calls `App::apply_ralph_update()` which mutates `app.ralph`.

```rust
pub enum RalphUpdate {
    Progress { id: LoopId, phase: String, percent: f64 },
    LogLine  { id: LoopId, line: String },
    Completed { id: LoopId },
    Failed { id: LoopId, reason: String },
    Spawned { id: LoopId, kind: LoopKind },
}
```

**Key API**: `ratatui::widgets::Gauge` for progress bar; `ratatui::widgets::List` for log lines with autoscroll via `ListState::select(Some(last_index))`.

#### Step 2.5 — Settings View

**File**: `lazyjob-tui/src/views/settings.rs`

Form-style view. Each setting is a `SettingRow` rendered as a table row. When `enter` is pressed on a row, the appropriate input modal opens. Settings changes call `Config::save()` on submit.

Sensitive values (API keys) display as `●●●●●●●●●●●●●●●` using masked display in `InputField { secure: true }`.

#### Step 2.6 — Contacts View

Similar structure to Jobs List with a contact card widget. Filter panel shows company/relationship/quality filters.

#### Step 2.7 — Confirm Dialog Modal

**File**: `lazyjob-tui/src/widgets/confirm_dialog.rs`

```rust
pub fn render_confirm(frame: &mut Frame, area: Rect, modal: &Modal, app: &App) {
    // Center a fixed-size box
    let popup = centered_rect(40, 10, area);
    frame.render_widget(Clear, popup);  // clear background
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.focused_border_style())
        .title(modal.title());
    // render body text + [Cancel] [Confirm] buttons
}

/// Compute a centered Rect of given width% and height% within `r`.
fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(r);
    let w = r.width * percent_x / 100;
    Rect::new(r.x + (r.width - w) / 2, popup_layout[1].y, w, height)
}
```

**Key API**: `ratatui::widgets::Clear` — erases background before drawing modal so it doesn't bleed through.

---

### Phase 3 — Polish, Vim Motions, Configurable Keymaps

#### Step 3.1 — Vim `gg`/`G` motions

`gg` requires detecting a two-key sequence. Implement a `pending_key: Option<KeyCode>` field in `App`. In the router:

```rust
if app.pending_key == Some(KeyCode::Char('g')) && key.code == KeyCode::Char('g') {
    app.pending_key = None;
    return Some(Action::JumpToTop);
}
if key.code == KeyCode::Char('g') {
    app.pending_key = Some(KeyCode::Char('g'));
    return None; // wait for second key
}
```

`G` maps directly to `Action::JumpToBottom`.

#### Step 3.2 — Search (`/`)

When `/` is pressed, `app.input_mode` becomes `InputMode::Search`. Printable characters append to `app.jobs.search_query`. `Escape` exits search. On each character, call `App::filter_jobs_by_query()` which does a case-insensitive substring match against title+company in-memory.

The search query is rendered in the status bar when active.

#### Step 3.3 — Configurable keymaps from TOML

**File**: `lazyjob-tui/src/keybindings.rs`

```rust
impl KeyMap {
    /// Load overrides from `~/.lazyjob/keybindings.toml` and merge.
    pub fn load_with_overrides(defaults: Self, path: &Path) -> anyhow::Result<Self> {
        if !path.exists() { return Ok(defaults); }
        let text = std::fs::read_to_string(path)?;
        let overrides: KeyMapOverrides = toml::from_str(&text)?;
        // parse each string key ("ctrl+r", "j", etc.) into KeyCombo
        // insert into defaults.0 map
        Ok(defaults)
    }
}
```

**Key API**: `toml` crate for parsing; string-to-KeyCombo parser using a `nom` or hand-rolled lexer.

#### Step 3.4 — Help Overlay

**File**: `lazyjob-tui/src/help.rs`

Full-screen centered popup with scrollable key reference. Groups keys by context. The help content is generated programmatically from `KeyMap` so it always reflects the user's actual bindings:

```rust
pub fn render_help_overlay(frame: &mut Frame, area: Rect, view: &View, keymap: &KeyMap) {
    let ctx = view_to_context(view);
    let entries: Vec<(String, String)> = keymap.entries_for_context(&ctx);
    // render as two-column table
}
```

#### Step 3.5 — Mouse support (optional)

Enable `crossterm::event::EnableMouseCapture`. Map `MouseEvent::Down` to the equivalent keyboard action based on click position. Use `frame.area()` coordinate math to determine which widget was clicked. This is optional per the spec's open question.

#### Step 3.6 — Color theme switching

Add `ThemeName` enum (`Dark`, `Light`, `HighContrast`). `App.theme` is set at startup from `Config.theme`. Add a `[Appearance]` settings section.

---

## Key Crate APIs

| Crate | API | Purpose |
|-------|-----|---------|
| `ratatui` | `Terminal::draw(frame: &mut Frame)` | Frame render entry point |
| `ratatui` | `Frame::render_widget(widget, area)` | Stateless widget |
| `ratatui` | `Frame::render_stateful_widget(widget, area, state)` | Widget with external state (List, Table) |
| `ratatui` | `Frame::area() -> Rect` | Full terminal area |
| `ratatui` | `widgets::Clear` | Erase background for overlays |
| `ratatui` | `widgets::List`, `ListItem`, `ListState` | Scrollable selectable list |
| `ratatui` | `widgets::Table`, `Row`, `Cell` | Grid data |
| `ratatui` | `widgets::Tabs` | Header navigation tabs |
| `ratatui` | `widgets::Gauge` | Progress bar |
| `ratatui` | `widgets::Paragraph` | Text with wrapping/scroll |
| `ratatui` | `widgets::Block`, `Borders` | Bordered containers |
| `ratatui` | `text::{Line, Span, Text}` | Rich text spans |
| `ratatui` | `layout::{Layout, Constraint, Direction, Rect}` | Layout computation |
| `crossterm` | `event::EventStream` | Async event stream (requires `event-stream` feature) |
| `crossterm` | `event::{KeyCode, KeyModifiers, MouseEvent}` | Input event types |
| `crossterm` | `terminal::{enable_raw_mode, disable_raw_mode}` | Raw mode control |
| `crossterm` | `execute!(EnterAlternateScreen)` | Alternate screen |
| `crossterm` | `style::Color` | 256-color/RGB support |
| `crossterm` | `cursor::Hide` | Hide cursor during rendering |
| `tokio` | `select!` | Multiplex async sources |
| `tokio::time` | `interval(Duration)` | Tick timer |
| `crossbeam_channel` | `unbounded::<T>()` | Sync channel for thread→async bridge |
| `unicode-width` | `UnicodeWidthStr::width(&str)` | Accurate column counts |

---

## Error Handling

```rust
// lazyjob-tui/src/error.rs

#[derive(thiserror::Error, Debug)]
pub enum TuiError {
    #[error("terminal setup failed: {0}")]
    Terminal(#[from] std::io::Error),

    #[error("crossterm error: {0}")]
    Crossterm(#[from] crossterm::ErrorKind),

    #[error("database error: {0}")]
    Database(#[from] lazyjob_core::persistence::DbError),

    #[error("render error: {0}")]
    Render(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, TuiError>;
```

All internal view/widget functions use `anyhow::Result<()>` for simplicity. Only the public `run()` boundary uses `TuiError`.

On panic (e.g., during rendering), the `EventLoop::run()` must restore the terminal before propagating. Use a `scopeguard::defer!` or `Drop` impl on `EventLoop` to guarantee cleanup:

```rust
impl Drop for EventLoop {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}
```

---

## Testing Strategy

### Unit Tests

**Theme**: Test that `Theme::DARK` produces correct `Style` values.
```rust
#[test]
fn dark_theme_selected_style_has_bg() {
    assert_eq!(Theme::DARK.selected_style().bg, Some(Color::LightBlue));
}
```

**Layout**: Test `AppLayout::compute` and `BodyLayout::compute` for correct `Rect` dimensions on various terminal sizes.
```rust
#[test]
fn app_layout_splits_correctly() {
    let area = Rect::new(0, 0, 120, 40);
    let layout = AppLayout::compute(area);
    assert_eq!(layout.header.height, HEADER_HEIGHT);
    assert_eq!(layout.status_bar.height, STATUS_BAR_HEIGHT);
    assert_eq!(layout.body.height, 40 - HEADER_HEIGHT - STATUS_BAR_HEIGHT);
}
```

**KeyMap**: Test that `KeyMap::default_keymap().resolve()` returns expected actions.
```rust
#[test]
fn quit_resolves_globally() {
    let km = KeyMap::default_keymap();
    let combo = KeyCombo::plain(KeyCode::Char('q'));
    assert_eq!(km.resolve(&KeyContext::Global, &combo), Some(&Action::Quit));
}
```

**Router**: Test `key_to_action` with mocked `App` for each view context.

**Modal**: Test that `centered_rect(50, 10, Rect::new(0,0,120,40))` produces a centered `Rect`.

### Integration Tests

**Full render test** using `ratatui::backend::TestBackend`:
```rust
#[tokio::test]
async fn jobs_list_renders_without_panic() {
    let db = Database::in_memory().await.unwrap();
    let config = Config::default();
    let app = App::new(Arc::new(config), Arc::new(db)).await.unwrap();
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, &mut app)).unwrap();
}
```

**Key event round-trip**: Inject `KeyEvent` into the router and assert state changes:
```rust
#[tokio::test]
async fn q_key_sets_should_quit() {
    let mut app = test_app().await;
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    let action = router::key_to_action(&app, key).unwrap();
    handle_action(&mut app, action).await.unwrap();
    assert!(app.should_quit);
}
```

### TUI Visual Tests

`ratatui::backend::TestBackend` captures rendered output as a buffer of `Cell`s. Use `.assert_buffer_eq()` to snapshot-test widget output:
```rust
terminal.draw(|f| render_status_bar(f, f.area(), &app)).unwrap();
terminal.backend().assert_buffer_eq(/* expected buffer */);
```

For complex views, prefer `insta` snapshots of the buffer's `to_string()` representation.

---

## Open Questions

1. **Mouse support**: The spec asks whether to support mouse clicks. Recommendation: enable `EnableMouseCapture` in Phase 3 but treat it as opt-in via `config.mouse_enabled = false` default to avoid conflicts with terminal emulator selection.

2. **Copy to clipboard**: The Job Detail view maps `y` to "copy URL". Needs `arboard` or `copypasta` crate (platform-dependent). Defer to Phase 3; show a status-bar notification on copy.

3. **Animation timing**: The spec lists 150ms scroll and 100ms modal fade. `ratatui` has no built-in animation. For modal fade, use an `opacity: u8` counter decremented per tick. For smooth scroll, track a `scroll_animation: Option<ScrollAnim>` with a target and interpolated offset.

4. **Offline indicator**: Spec shows an "Offline" banner. The TUI doesn't manage network state directly — this must come from `lazyjob-ralph` reporting connectivity to `ralph_rx`. Design needed.

5. **Accessibility / screen reader**: `crossterm` outputs ANSI escape sequences; screen readers that support terminal emulators (e.g., `nvda` + `ConEmu`) should work. Explicit accessibility support (e.g., `accessible_output` crate) is deferred.

6. **Resize handling**: `crossterm::event::Event::Resize(w, h)` fires on terminal resize. `ratatui` automatically uses the new size on the next `draw()` call. No explicit handling needed. Verify on very small terminals (width < 60).

7. **Configurable keymap file format**: The spec mentions storing keybinds in config. Recommend TOML at `~/.lazyjob/keybindings.toml` mirroring the `KeyMapOverrides` serde struct. Document the key string format (`"ctrl+r"`, `"shift+m"`, `"j"`, `"enter"`, `"f1"`) in the README.

---

## Related Specs

- `specs/agentic-ralph-orchestration.md` — Ralph loop status pushed to TUI via channel
- `specs/agentic-ralph-subprocess-protocol.md` — `RalphUpdate` message format
- `specs/10-application-workflow.md` — kanban state machine used in Pipeline View
- `specs/16-privacy-security.md` — API key masking in Settings View
- `specs/04-sqlite-persistence-implementation-plan.md` — data loading from repositories
- `specs/XX-tui-vim-mode.md` — deeper vim modal editing (Phase 3 extension)
- `specs/XX-tui-accessibility.md` — high-contrast mode and screen reader support
