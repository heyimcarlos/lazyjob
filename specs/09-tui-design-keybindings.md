# TUI Design & Keybindings

## Status
Researching

## Problem Statement

The LazyJob TUI is the primary user interface. Inspired by lazygit, it must be:
1. **Discoverable**: Users can figure out how to do things without memorizing keybindings
2. **Efficient**: Common actions accessible via few keystrokes
3. **Consistent**: Same patterns across all views
4. **Informative**: Always clear what's happening and what state you're in
5. **Responsive**: Updates in real-time, handles async operations gracefully

This spec defines the complete TUI design: layout, views, keybindings, and interaction patterns.

---

## Layout Structure

### Main Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ Header: [LazyJob Logo]  [Dashboard|Jobs|Apps|Contacts|Settings]│
├─────────────┬─────────────────────────────────────────────────┤
│             │                                                   │
│  Sidebar    │              Main Content Area                    │
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
│ Status Bar: [Job: 42] [Filter: Engineering] [Ralph: ●] [12:34]│
└─────────────────────────────────────────────────────────────────┘
```

### View Hierarchy

```
LazyJob
├── Dashboard           # Overview statistics, recent activity
├── Jobs                # Job search and list
│   ├── Jobs List       # Filterable list of all jobs
│   ├── Job Detail      # Full job info + actions
│   └── Job Edit        # Edit job details
├── Applications        # Kanban pipeline view
│   ├── Pipeline        # Kanban board (Discovered → Offer)
│   └── Application     # Single application detail
├── Contacts            # Networking contacts
│   ├── Contacts List   # All contacts
│   ├── Contact Detail  # Single contact
│   └── Add Contact     # New contact form
├── Ralph               # Active loops panel
│   └── Loop Detail     # Single loop status + output
├── Settings            # Configuration
│   ├── General         # Basic settings
│   ├── LLM Providers   # API key configuration
│   ├── Companies       # Company discovery config
│   └── Data            # Import/export
└── Help Overlay       # Full keybinding reference
```

### Panel Dimensions

```rust
// Default layout constraints
const HEADER_HEIGHT: u16 = 3;
const SIDEBAR_WIDTH: u16 = 30;
const STATUS_BAR_HEIGHT: u16 = 1;
const MIN_CONTENT_WIDTH: u16 = 50;

impl Layout {
    fn main_layout(width: u16, height: u16) -> Vec<Rect> {
        Layout::vertical([
            Constraint::Length(HEADER_HEIGHT),           // Header
            Constraint::Fill(1),                         // Main area
            Constraint::Length(STATUS_BAR_HEIGHT),       // Status bar
        ])
        .areas(Rect::new(0, 0, width, height))
    }

    fn content_area(area: Rect) -> (Rect, Rect) {
        Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH.min(area.width / 3)),
            Constraint::Fill(1),
        ])
        .areas(area)
    }
}
```

---

## View Specifications

### 1. Dashboard View

**Purpose**: High-level overview of job search status.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│                          DASHBOARD                               │
├───────────────────────┬─────────────────────────────────────────┤
│  📊 Statistics        │  📰 Recent Activity                     │
│                       │                                          │
│  Jobs Discovered: 42  │  • Applied to Stripe (2h ago)           │
│  Applications: 12     │  • New job: SRE at GitHub (5h ago)     │
│  Interviews: 3        │  • Interview scheduled: Meta (1d ago)  │
│  Offers: 1           │  • Offer received: Datadog (3d ago)    │
│                       │                                          │
├───────────────────────┼─────────────────────────────────────────┤
│  ⏰ Upcoming          │  🎯 Recommended Jobs                      │
│                       │                                          │
│  • Follow up: Stripe  │  • Senior SRE @ Google (92% match)      │
│    (Tomorrow)         │  • Platform Eng @ Figma (89% match)    │
│  • Interview: Meta    │  • Staff Eng @ Linear (87% match)       │
│    (Thursday)         │                                          │
└───────────────────────┴─────────────────────────────────────────┘
```

**Keybindings**:
- `j/k` or `↓/↑`: Navigate items
- `enter`: Open selected item
- `a`: New application
- `r`: Refresh/sync
- `?`: Help

### 2. Jobs List View

**Purpose**: Browse and filter all discovered jobs.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ 🔍 Jobs                    [🔍 Search] [⚡ Filters] [↻ Refresh]│
├─────────────┬───────────────────────────────────────────────────┤
│ FILTERS     │ JOBS (42)                                        │
│             │                                                   │
│ Status      │ ┌─────────────────────────────────────────────┐  │
│ □ All (42)  │ │ ● Senior SRE  |  Stripe  |  SF / Remote   │  │
│ ○ Active(8) │ │   Engineering • $180-220k • 3d ago        │  │
│ ○ Applied(12)│ └─────────────────────────────────────────────┘  │
│ ○ Interview(3)│ ┌─────────────────────────────────────────────┐  │
│             │ │ ○ Platform Engineer  |  Figma  |  SF        │  │
│ Skills      │ │   Backend • $160-200k • 5d ago               │  │
│ ☑ Rust     │ └─────────────────────────────────────────────┘  │
│ ☑ Python   │                                                   │
│ □ Go       │                                                   │
│             │                                                   │
│ Salary      │                                                   │
│ [$100k-$300k]│                                                  │
│             │                                                   │
│ Remote      │                                                   │
│ ☑ Remote OK │                                                   │
└─────────────┴───────────────────────────────────────────────────┘
```

**Job List Item States**:
- Default: `○` unselected
- Selected: `●` with highlight background
- Applied: `●` with green tint
- Interview: `●` with yellow tint
- Offer: `●` with green bold
- Rejected: Strikethrough, dimmed

**Keybindings**:
- `j/k` or `↓/↑`: Navigate jobs
- `l/→` or `enter`: Open job detail
- `space`: Toggle select for bulk action
- `a`: Apply to selected job(s)
- `/`: Focus search
- `f`: Toggle filter panel
- `r`: Refresh job listings
- `n`: Add new job manually
- `d`: Delete selected job
- `?`: Help

### 3. Job Detail View

**Purpose**: View full job details and take actions.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ ← Back    Senior Site Reliability Engineer    [❤️ Interested ▼]│
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  🏢 Stripe                                          [🌐 Apply]  │
│  📍 San Francisco, CA  |  Remote OK                            │
│  💰 $180,000 - $220,000 (USD)                                   │
│  📅 Posted 3 days ago                                            │
│  🔗 https://stripe.com/jobs/sre-123                             │
│                                                                   │
├─────────────────────────────────────────────────────────────────┤
│ DESCRIPTION                                                      │
│                                                                   │
│ We're looking for a Senior Site Reliability Engineer to join      │
│ our Infrastructure team. You'll work on..."                       │
│                                                                   │
├─────────────────────────────────────────────────────────────────┤
│ MATCHING                                                        │
│                                                                   │
│ Your Profile Match: 87%                                          │
│ ✓ 7/8 required skills                                           │
│ ✓ Experience in distributed systems                              │
│ ✓ SRE background at scale                                       │
│ ✗ Missing: Kubernetes certification                             │
│                                                                   │
├─────────────────────────────────────────────────────────────────┤
│ COMPANY INFO                                                     │
│                                                                   │
│ About Stripe...                                                  │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

**Keybindings**:
- `←/h/escape`: Back to list
- `e`: Edit job details
- `a`: Apply to this job
- `r`: Tailor resume for this job
- `c`: Write cover letter
- `i`: Add to interview prep
- `d`: Delete job
- `y`: Copy URL to clipboard

### 4. Applications Pipeline View

**Purpose**: Track application status like a kanban board.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ Applications Pipeline                        [↻ Refresh] [?]   │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  DISCOVERED    INTERESTED      APPLIED     PHONE    TECHNICAL  │
│  ───────────   ────────────   ─────────   ─────    ─────────  │
│                                                                   │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐ ┌─────────┐        │
│  │Job Card │   │Job Card │   │Job Card │ │Job Card │        │
│  │         │   │         │   │         │ │         │        │
│  └─────────┘   └─────────┘   └─────────┘ └─────────┘        │
│                                                                   │
│                                                                   │
│  ON-SITE        OFFER          REJECTED     WITHDRAWN             │
│  ─────────      ─────         ─────────     ──────────          │
│                                                                   │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐ ┌─────────┐          │
│  │Job Card │   │Job Card │   │Job Card │ │Job Card │          │
│  │         │   │         │   │         │ │         │        │
│  └─────────┘   └─────────┘   └─────────┘ └─────────┘          │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

**Pipeline Stages**:
1. Discovered - Jobs found but not yet reviewed
2. Interested - Marked as interesting, considering
3. Applied - Submitted application
4. Phone Screen - Initial recruiter call
5. Technical - Technical assessment/interview
6. On-site - On-site/virtual final rounds
7. Offer - Received offer
8. Rejected - Not moving forward
9. Withdrawn - Chose to withdraw

**Application Card**:
```
┌─────────────────────┐
│ ● Stripe SRE        │
│ Applied: 5d ago     │
│ Last contact: 2d    │
│ [📧 Email] [📅 Cal] │
└─────────────────────┘
```

**Keybindings**:
- `j/k/←/→`: Navigate between cards and columns
- `m`: Move card to next stage (with confirmation)
- `shift+m`: Move card to previous stage
- `enter`: Open application detail
- `space`: Select card for bulk move
- `d`: Delete/archive card
- `?`: Help

### 5. Contacts View

**Purpose**: Manage networking contacts and referral sources.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ Contacts                          [+ Add] [🔍 Search] [↻ Sync] │
├─────────────┬───────────────────────────────────────────────────┤
│ FILTERS     │ CONTACTS (156)                                    │
│             │                                                   │
│ Company     │ ┌─────────────────────────────────────────────┐  │
│ □ Stripe (12)│ │ 👤 Sarah Johnson                               │  │
│ □ Google (8) │ │   Former Manager @ Stripe                    │  │
│ □ Meta (5)   │ │   ★★★★★ | Referred: 2                        │  │
│             │ │   📧 sarah@example.com                         │  │
│ Relationship│ │   🔗 linkedin.com/in/sarahjohnson             │  │
│ □ Recruiter │ └─────────────────────────────────────────────┘  │
│ □ Hiring Mgr│ ┌─────────────────────────────────────────────┐  │
│ □ Referrer  │ │ 👤 Michael Chen                               │  │
│ □ Colleague │ │   Recruiter @ Google                        │  │
│             │ │   ★★★★☆ | Referred: 0                        │  │
│ Quality     │ │   📧 michael@google.com                       │  │
│ ★★★★☆ (8)   │ └─────────────────────────────────────────────┘  │
│ ★★★★★ (3)   │                                                   │
│             │                                                   │
└─────────────┴───────────────────────────────────────────────────┘
```

**Keybindings**:
- `j/k`: Navigate contacts
- `enter`: Open contact detail
- `a`: Add new contact
- `e`: Edit contact
- `r`: Send outreach/follow-up
- `d`: Delete contact
- `?`: Help

### 6. Ralph Panel

**Purpose**: Monitor active Ralph loops and their progress.

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ Ralph Loops                              [🔍 New Loop] [⏹ Stop]│
├─────────────┬───────────────────────────────────────────────────┤
│ ACTIVE (2)   │ LOOP DETAIL                                      │
│             │                                                   │
│ 🔍 Discovery│ ┌─────────────────────────────────────────────┐  │
│   Phase:    │ │ Job Discovery Loop                           │  │
│   Fetching  │ │                                             │  │
│   Progress: │ │ Status: Running                              │  │
│   ████░░ 60%│ │ Started: 5 minutes ago                       │  │
│             │ │                                             │  │
│ 📄 Resume    │ │ ████████████░░░░░░░░░░░░░ 63%               │  │
│   Phase:    │ │                                             │  │
│   Drafting  │ │ Phase: Searching companies...                │  │
│   ████████  │ │                                             │  │
│             │ │ Found 23 new jobs                            │  │
│             │ │ Currently processing: Stripe                  │  │
│             │ │                                             │  │
│             │ │ [View Output] [Cancel] [Background]           │  │
│             │ └─────────────────────────────────────────────┘  │
├─────────────┴───────────────────────────────────────────────────┤
│ COMPLETED                                                        │
│ 🔍 Discovery #12 - Completed 2h ago - Found 47 jobs            │
│ 📄 Resume #8 - Completed 1d ago - Stripe SRE                   │
└─────────────────────────────────────────────────────────────────┘
```

**Keybindings**:
- `j/k`: Navigate loops
- `enter`: Open loop detail
- `space`: Toggle select
- `s`: Stop selected loop
- `n`: Start new loop
- `l`: View loop output/log
- `?`: Help

### 7. Settings View

**Layout**:
```
┌─────────────────────────────────────────────────────────────────┐
│ Settings                                                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ GENERAL                                                    │   │
│  │                                                             │   │
│  │  Database: ~/.lazyjob/lazyjob.db          [Change]       │   │
│  │  Resume Output: ~/.lazyjob/resumes/        [Change]      │   │
│  │  Theme: Dark                                    [▼]       │   │
│  │  Polling Interval: 60 minutes                 [▼]        │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ LLM PROVIDERS                                            │   │
│  │                                                             │   │
│  │  Primary: Anthropic                           [▼]        │   │
│  │  API Key: ●●●●●●●●●●●●●●●                    [Change]   │   │
│  │  Model: claude-3-5-sonnet-20241022                        │   │
│  │                                                             │   │
│  │  Fallback: OpenAI                             [▼]         │   │
│  │  API Key: ●●●●●●●●●●●●●●●                    [Change]   │   │
│  │  Model: gpt-4o                                           │   │
│  │                                                             │   │
│  │  Local: Ollama                                [▼]         │   │
│  │  Endpoint: http://localhost:11434                         │   │
│  │  Model: llama3.2                                          │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ DATA                                                     │   │
│  │                                                             │   │
│  │  [📤 Export All]  [📥 Import]  [🗑 Clear Database]       │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

**Keybindings**:
- `↑/↓` or `j/k`: Navigate options
- `←/→` or `h/l`: Change values
- `enter`: Activate selected
- `e`: Edit value
- `?`: Help

### 8. Help Overlay

**Purpose**: Full keybinding reference (lazygit-style `?`).

```
┌─────────────────────────────────────────────────────────────────┐
│                         KEYBINDINGS                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  NAVIGATION                    ACTIONS                          │
│  ───────────                   ───────                          │
│  j/k or ↓/↑    Move down/up    a       Add new                  │
│  h/l or ←/→    Move left/right e       Edit                     │
│  gg           Jump to top      d       Delete                   │
│  G            Jump to bottom   space   Select/Toggle           │
│  /            Search           enter   Open/Confirm             │
│  n            Next search      escape  Cancel/Back               │
│  N            Prev search      r       Refresh/Reload            │
│                                                                   │
│  GLOBAL                          Ralph LOOP                      │
│  ───────                         ───────────                      │
│  ?           Show this help     s       Stop loop                │
│  g           Go to dashboard   n       New loop                  │
│  1-8        Switch view         l       View loop log           │
│  ctrl+r     Refresh current     space   Toggle select            │
│  q           Quit                                                 │
│                                                                   │
│  For detailed help on any view, press ? while in that view.     │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Keybinding Philosophy

### Guiding Principles

1. **Vim-inspired navigation**: `hjkl` + arrows for movement
2. **Primary actions on obvious keys**: `a` for add, `e` for edit, `d` for delete
3. **Consistent across views**: Same keys mean same things
4. **Discoverable**: `?` always shows help
5. **Reversible actions**: Destructive actions require confirmation
6. **No single-key combos**: No ctrl/meta combos except `ctrl+c` (cancel) and `ctrl+r` (refresh)

### Global Keybindings

| Key | Action |
|-----|--------|
| `?` | Show help overlay |
| `g` + `d` | Go to dashboard |
| `g` + `j` | Go to jobs |
| `g` + `a` | Go to applications |
| `g` + `c` | Go to contacts |
| `g` + `s` | Go to settings |
| `1` | Dashboard |
| `2` | Jobs |
| `3` | Applications |
| `4` | Contacts |
| `5` | Ralph |
| `6` | Settings |
| `q` | Quit |
| `ctrl+r` | Refresh |
| `ctrl+c` | Cancel current operation |
| `escape` | Back / Cancel |
| `space` | Select / Toggle |
| `enter` | Open / Confirm |
| `/` | Search |

### Context-Specific Keybindings

Each view has context-specific bindings that only apply when that view is focused.

---

## Component Patterns

### Stateful Widgets

```rust
// Jobs list state
pub struct JobsListState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub filter: JobFilter,
    pub sort_by: JobSortField,
    pub sort_direction: SortDirection,
}

// Application state
pub struct ApplicationState {
    pub selected_card_index: usize,
    pub selected_column: PipelineStage,
}

// Contact list state
pub struct ContactListState {
    pub selected_index: usize,
    pub filter: ContactFilter,
}
```

### Reusable Widgets

```rust
// lazyjob-tui/src/widgets/

pub mod job_card;
pub mod application_card;
pub mod contact_card;
pub mod stat_block;
pub mod progress_bar;
pub mod filter_panel;
pub mod modal;
pub mod confirm_dialog;
pub mod input_dialog;
```

### Modal Dialogs

**Confirm Dialog**:
```
┌─────────────────────────────────────┐
│                                     │
│  Delete this job?                    │
│                                     │
│  This action cannot be undone.       │
│                                     │
│        [Cancel]  [Delete]           │
│                                     │
└─────────────────────────────────────┘
```

**Input Dialog**:
```
┌─────────────────────────────────────┐
│                                     │
│  Add new job                        │
│                                     │
│  Company: [___________________]     │
│  Title:   [___________________]     │
│  URL:     [___________________]     │
│                                     │
│        [Cancel]  [Save]             │
│                                     │
└─────────────────────────────────────┘
```

---

## Error Handling & States

### Loading States

```
┌─────────────────────────────────────┐
│                                     │
│  Loading jobs...                     │
│                                     │
│  ████████████░░░░░░░░ 60%          │
│                                     │
└─────────────────────────────────────┘
```

### Error States

```
┌─────────────────────────────────────┐
│  ⚠ Error loading jobs               │
│                                     │
│  Could not connect to database.      │
│                                     │
│  [Retry]  [View Details]            │
│                                     │
└─────────────────────────────────────┘
```

### Empty States

```
┌─────────────────────────────────────┐
│                                     │
│  📭 No jobs yet                     │
│                                     │
│  Start by adding companies to track  │
│  or searching for new opportunities │
│                                     │
│     [Add Company]  [Search Jobs]    │
│                                     │
└─────────────────────────────────────┘
```

### Offline Indicator

```
┌─────────────────────────────────────┐
│  ⚠ Offline - Changes will sync      │
│     when connection is restored      │
└─────────────────────────────────────┘
```

---

## Status Bar

The status bar provides persistent context:

```
[Job: 42] [Filter: Engineering] [Matched: 12] [Ralph: ●] [12:34]
│       │              │              │          │       │
│       │              │              │          │       └─ Current time
│       │              │              │          └─ Ralph status (● running, ○ idle, ✗ error)
│       │              │              └─ Matching jobs count
│       │              └─ Active filter
│       └─ Total job count
└─ Current context
```

### Ralph Status Indicators

| Symbol | Meaning |
|--------|---------|
| `●` | Loop running |
| `○` | No active loops |
| `✗` | Loop error |
| `⏸` | Loop paused |

---

## Animation & Transitions

1. **View transitions**: Instant (no fancy animations)
2. **List scrolling**: Smooth scroll with 150ms duration
3. **Modal appearance**: Fade in 100ms
4. **Progress bars**: Smooth width transition
5. **Hover states**: Immediate (no delay)

---

## Color Scheme

### Dark Theme (Default)

```rust
const THEME: Theme = Theme {
    // Primary colors
    primary: Color::LightBlue,
    secondary: Color::DarkGray,

    // Status colors
    success: Color::LightGreen,
    warning: Color::LightYellow,
    error: Color::LightRed,

    // Text
    text_primary: Color::White,
    text_secondary: Color::Gray,
    text_muted: Color::DarkGray,

    // Background
    bg_primary: Color::Black,
    bg_secondary: Color::DarkGray,
    bg_elevated: Color::Gray,

    // Borders
    border: Color::DarkGray,
    border_focused: Color::LightBlue,
};
```

---

## Open Questions

1. **Mouse support**: Should we support mouse clicks for navigation?
2. **Copy/paste**: How should copy/paste work in TUI context?
3. **Notifications**: Should there be terminal notifications (native OS notifications)?
4. **Sound**: Any audio feedback for important events?
5. **Accessibility**: Screen reader support? High contrast mode?

---

## Sources

- [lazygit Keybindings Documentation](https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings)
- [Ratatui Widgets Documentation](https://ratatui.rs/)
- [vim Hardmode Philosophy](https://hardmode.xyz/)
- [Luke's XML Schema for TUI](http://luke.d 汉)
