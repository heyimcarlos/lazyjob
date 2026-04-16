# Spec: Architecture — TUI Skeleton

**JTBD**: A fast, reliable tool that works offline
**Topic**: Define the ratatui-based terminal UI architecture: app structure, view hierarchy, event loop, widget system, keybindings, and theme
**Domain**: architecture

---

## What

The LazyJob TUI is a ratatui application with a lazygit-inspired design: an app struct manages view state, the main event loop handles crossterm events and dispatches to view handlers, and each view (dashboard, jobs, applications, contacts, ralph, settings, help) is a self-contained module. The TUI is the human control plane for the Ralph autonomous agent system — ralph runs as a subprocess and streams status/events back to the TUI via the RalphEvent broadcast channel.

## Why

The TUI is the primary interface users interact with. Its design must be:
- **Discoverable**: `?` shows contextual help — no need to memorize keybindings
- **Efficient**: Common actions are single-key (`a` add, `e` edit, `d` delete)
- **Consistent**: Same navigation patterns across all views
- **Informative**: Status bar shows Ralph state, job count, filter state at all times
- **Responsive**: All async operations (LLM calls, platform API fetches) show loading state

The lazygit-inspiration is deliberate: the terminal tool for developers who hate mouse-driven UIs. LazyJob extends this philosophy to job search.

## How

### App Structure

```rust
// lazyjob-tui/src/app.rs

pub struct App {
    db: Database,
    llm: LlmBuilder,
    ralph: RalphProcessManager,
    state: AppState,
    theme: Theme,
}

pub enum AppView {
    Dashboard,
    Jobs,
    JobDetail(Uuid),
    Applications,
    ApplicationDetail(Uuid),
    Contacts,
    ContactDetail(Uuid),
    Ralph,
    RalphLoopDetail(Uuid),
    Settings,
    Help,
}

pub struct AppState {
    pub current_view: AppView,
    pub jobs_filter: JobFilter,
    pub selected_job_ids: Vec<Uuid>,
    pub ralph_events: broadcast::Receiver<RalphEvent>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let db = Database::with_auto_backup(&data_dir()).await?;
        let llm = LlmBuilder::from_config()?;
        let ralph = RalphProcessManager::new()?;
        let (tx, rx) = broadcast::channel(100);
        ralph.set_event_sender(tx);

        Ok(Self {
            db,
            llm,
            ralph,
            state: AppState {
                current_view: AppView::Dashboard,
                jobs_filter: JobFilter::default(),
                selected_job_ids: Vec::new(),
                ralph_events: rx,
            },
            theme: Theme::dark(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = Terminal::new()?;
        loop {
            terminal.draw(|frame| self.render(frame))?;
            if let Some(event) = self.handle_events()? {
                match event {
                    AppEvent::Quit => break Ok(()),
                    AppEvent::SwitchView(v) => self.state.current_view = v,
                    AppEvent::RalphEvent(e) => self.handle_ralph_event(e),
                }
            }
        }
    }
}
```

### Main Event Loop

```rust
// lazyjob-tui/src/app.rs

use crossterm::event::{self, Event, KeyEvent};

impl App {
    fn handle_events(&mut self) -> Result<Option<AppEvent>> {
        // Poll Ralph events concurrently with terminal events
        while let Ok(ctx) = self.state.ralph_events.try_recv() {
            self.handle_ralph_event(ctx);
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(KeyEvent { code: KeyCode::Char('q'), modifiers: KeyModifiers::NONE, .. }) => {
                    return Ok(Some(AppEvent::Quit));
                }
                Event::Key(KeyEvent { code: KeyCode::Char('?'), .. }) => {
                    self.state.current_view = AppView::Help;
                }
                Event::Key(KeyEvent { code: KeyCode::Char('g'), .. }) => {
                    self.state.pending_g = true;
                }
                Event::Key(KeyEvent { code: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, .. })
                    if self.state.pending_g => {
                    self.state.current_view = AppView::Dashboard;
                    self.state.pending_g = false;
                }
                Event::Key(KeyEvent { code: KeyCode::Char('1'), .. }) => self.state.current_view = AppView::Dashboard,
                Event::Key(KeyEvent { code: KeyCode::Char('2'), .. }) => self.state.current_view = AppView::Jobs,
                Event::Key(KeyEvent { code: KeyCode::Char('3'), .. }) => self.state.current_view = AppView::Applications,
                Event::Key(KeyEvent { code: KeyCode::Char('4'), .. }) => self.state.current_view = AppView::Contacts,
                Event::Key(KeyEvent { code: KeyCode::Char('5'), .. }) => self.state.current_view = AppView::Ralph,
                Event::Key(KeyEvent { code: KeyCode::Char('6'), .. }) => self.state.current_view = AppView::Settings,
                Event::Key(KeyEvent { code: KeyCode::Esc, .. }) => {
                    self.state.current_view = self.state.previous_view.take().unwrap_or(AppView::Dashboard);
                }
                event => {
                    // Delegate to view-specific handler
                    self.state.current_view.handle_key(event, &mut self.state)?;
                }
            }
        }
        Ok(None)
    }
}
```

### View Trait

```rust
// lazyjob-tui/src/views/mod.rs

pub trait View {
    fn title(&self) -> &str;
    fn render(&mut self, frame: &mut Frame);
    fn handle_key(&mut self, key: Event, state: &mut AppState) -> Result<bool>;
}

impl AppView {
    pub fn handle_key(&mut self, key: Event, state: &mut AppState) -> Result<bool> {
        match self {
            AppView::Jobs => self.jobs_view.handle_key(key, state),
            AppView::Applications => self.applications_view.handle_key(key, state),
            // ...
        }
    }
}
```

### Layout

```rust
// Main layout constraints
const HEADER_HEIGHT: u16 = 3;
const SIDEBAR_WIDTH: u16 = 30;
const STATUS_BAR_HEIGHT: u16 = 1;

fn main_layout(area: Rect) -> Vec<Rect> {
    Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Fill(1),
        Constraint::Length(STATUS_BAR_HEIGHT),
    ])
    .areas(area)
}

fn content_area(area: Rect) -> (Rect, Rect) {
    Layout::horizontal([
        Constraint::Length(SIDEBAR_WIDTH.min(area.width / 3)),
        Constraint::Fill(1),
    ])
    .areas(area)
}
```

### Status Bar

The status bar is always visible and shows:
```
[Job: 42] [Filter: Engineering] [Matched: 12] [Ralph: ●] [12:34]
```
- `Job: N` — total jobs in database
- `Filter: X` — current filter name (or "All")
- `Matched: N` — jobs matching current filter
- `Ralph: ●` — Ralph loop status (`●` running, `○` idle, `✗` error)
- `12:34` — current time

### Header Navigation

```
LazyJob  [Dashboard]  [Jobs]  [Applications]  [Contacts]  [Ralph]  [Settings]
```

Clickable via tab navigation (`←/→` or `tab`), or number keys `1-6`.

### Widget Structure

```
lazyjob-tui/src/widgets/
├── mod.rs
├── job_card.rs       # Job list item (status dot, title, company, salary, age)
├── application_card.rs # Kanban card (company, applied date, last contact)
├── contact_card.rs   # Contact list item (name, role, company, quality stars)
├── stat_block.rs     # Dashboard stat (icon, label, value)
├── progress_bar.rs  # Ralph loop progress indicator
├── filter_panel.rs   # Sidebar filter controls
├── modal.rs          # Base modal (confirm, input, selection)
├── confirm_dialog.rs
├── input_dialog.rs
└── help_overlay.rs   # Full-screen keybinding reference
```

### Theme

```rust
// lazyjob-tui/src/theme.rs

#[derive(Clone, Copy)]
pub struct Theme {
    pub primary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub border: Color,
    pub border_focused: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            primary: Color::LightBlue,
            success: Color::LightGreen,
            warning: Color::Yellow,
            error: Color::LightRed,
            text_primary: Color::White,
            text_secondary: Color::Gray,
            bg_primary: Color::Black,
            bg_secondary: Color::DarkGray,
            border: Color::DarkGray,
            border_focused: Color::LightBlue,
        }
    }
}
```

### WorkflowEvent Subscription

The TUI subscribes to a single `WorkflowEvent` broadcast channel (defined in `agentic-ralph-orchestration.md`). Multiple pollers emit different event variants:

```rust
// TUI main event loop
let mut workflow_events = WorkflowEventReceiver::new();

// The ReminderPoller (application-workflow-actions.md) emits:
//   WorkflowEvent::ReminderDue(ApplicationId)
// The NetworkingReminderPoller emits:
//   WorkflowEvent::NetworkingReminderDue(ContactId)
// The DigestService emits:
//   WorkflowEvent::DigestReady(Vec<Job>)
```

### Error States

```rust
// All error states shown as modal overlay
pub fn render_error(frame: &mut Frame, area: Rect, error: &Error) {
    let block = Block::bordered()
        .title(" Error ")
        .style(Style::new().fg(theme.error));
    let para = Paragraph::new(error.to_string()).block(block);
    frame.render_widget(para, area);
}
```

## Open Questions

- **Mouse support**: The TUI spec doesn't explicitly include mouse support. Should clicking on job cards, drag-and-drop for kanban, and sidebar clicks be added? Defer to Phase 2.
- **Native OS notifications**: Should the TUI emit native OS notifications for interview reminders and digest delivery? The spec-inventory notes this as a "morning digest" feature that depends on the notification system. Phase 2.
- **Copy/paste**: How should copy/paste work? `y` to yank URL to clipboard (via clipboard crate), `p` to paste in input dialogs. Implement with `copypasta` or `arboard` crate. Phase 2.

## Implementation Tasks

- [ ] Implement `App::new()` in `lazyjob-tui/src/app.rs` — load database, initialize LLM, spawn RalphProcessManager
- [ ] Implement `App::run()` event loop in `lazyjob-tui/src/app.rs` — crossterm event polling, RalphEvent receiver, view dispatch
- [ ] Implement all views: Dashboard, Jobs, JobDetail, Applications, Contacts, Ralph, Settings, Help — each in `lazyjob-tui/src/views/`
- [ ] Implement view keybinding dispatch: `AppView::handle_key()` routes to active view's key handler
- [ ] Implement `theme.rs` with dark/light themes and all color constants
- [ ] Implement custom widgets: `job_card.rs`, `application_card.rs`, `contact_card.rs`, `stat_block.rs`, `progress_bar.rs`, `modal.rs`
- [ ] Implement header navigation bar with view tabs and number key shortcuts `1-6`
- [ ] Implement status bar with job count, filter state, Ralph status indicator, and current time
- [ ] Wire `WorkflowEvent` broadcast channel subscription in TUI event loop — consume and handle ReminderDue, NetworkingReminderDue, DigestReady events
- [ ] Implement onboarding prompt for `LifeSheet.goals.short_term` on first run (if goals empty, show banner prompting user to fill in life-sheet.yaml)
