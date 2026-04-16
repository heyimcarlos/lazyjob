# Implementation Plan: TUI Accessibility (Screen Readers, Color Blind Mode, High Contrast)

## Status
Draft

## Related Spec
[specs/XX-tui-accessibility.md](XX-tui-accessibility.md)

## Overview

This plan details how LazyJob's terminal UI achieves accessibility for users with visual impairments, color blindness, and motion sensitivities. Terminal UIs cannot implement WCAG 2.1 verbatim — they run inside terminal emulators that intermediate between the application and OS accessibility APIs — but they can implement a structured set of accommodations that meaningfully improve usability across the full spectrum of visual ability.

The core design principle is **palette indirection**: every color used in the TUI flows through a `ThemePalette` struct resolved at startup from an `AccessibilityConfig`. No widget hardcodes a `Color::*` value; all rendering paths call `palette.color_for(SemanticColor)`. Color-blind modes, high contrast modes, and custom palettes are therefore free: only the palette resolver changes. Separately, the **shape-augmented status system** ensures that state information (Applied, Interview, Rejected, etc.) is conveyed by both color _and_ a distinctive Unicode symbol, so the information is fully accessible when color perception is limited or absent.

Screen reader support in terminal applications is structurally different from GUI accessibility. Most screen readers read the terminal character buffer directly via the terminal emulator (tmux + Orca on Linux, iTerm2 + VoiceOver on macOS). LazyJob cannot push AT events to screen readers the way a GUI framework pushes ATK/NSAccessibility events. Instead, the plan implements a **live region output channel**: a one-line `AccessibilityBar` rendered at the bottom of the screen that is updated with plain-text announcements whenever meaningful state changes occur. Screen readers that track the terminal cursor or have tmux-awareness will pick up these strings naturally.

## Prerequisites

### Specs/plans that must be implemented first
- `specs/09-tui-design-keybindings-implementation-plan.md` — `App`, `EventLoop`, `View`, ratatui widget hierarchy, `ColorPalette` stub
- `specs/08-gaps-salary-tui-implementation-plan.md` — `AccessibilityConfig`, `ColorMode`, `FocusIndicatorStyle` initial definitions (GAP-78 work); this plan extends and completes those stubs

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# Already expected (confirm present):
ratatui     = "0.29"
crossterm   = { version = "0.28", features = ["event-stream"] }
serde       = { version = "1", features = ["derive"] }
toml        = "0.8"
once_cell   = "1.19"

# New additions for this plan:
termenv     = "0.15"     # Terminal capability detection (color support, TERM, etc.)
```

`termenv` is already a transitive dependency of ratatui via `ratatui-crossterm`; confirm the version and expose it explicitly only if direct calls are needed.

---

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `AccessibilityConfig`, `ColorBlindMode`, `FocusIndicatorStyle`, `AccessibilityProfile` | `lazyjob-tui` | `src/accessibility/config.rs` |
| `ThemePalette`, `SemanticColor`, `StatusSymbol` | `lazyjob-tui` | `src/accessibility/palette.rs` |
| `AccessibilityBar` (live region widget) | `lazyjob-tui` | `src/accessibility/live_region.rs` |
| `AccessibilitySettingsView` | `lazyjob-tui` | `src/views/accessibility_settings.rs` |
| `FocusRing` + `FocusOrder` | `lazyjob-tui` | `src/accessibility/focus.rs` |
| `AnimationConfig` | `lazyjob-tui` | `src/accessibility/motion.rs` |

### Module Structure

```
lazyjob-tui/
  src/
    accessibility/
      mod.rs              # pub use re-exports
      config.rs           # AccessibilityConfig, AccessibilityProfile, ColorBlindMode, FocusIndicatorStyle
      palette.rs          # ThemePalette, SemanticColor, StatusSymbol, PaletteBuilder
      live_region.rs      # LiveRegion, AccessibilityBar widget
      focus.rs            # FocusRing, FocusOrder, FocusTarget trait
      motion.rs           # AnimationConfig, reduced_motion detection
    views/
      accessibility_settings.rs  # AccessibilitySettingsView, SettingsField enum
    app.rs                # App struct — gains `accessibility: AccessibilityProfile`
```

### Core Types

```rust
// src/accessibility/config.rs

/// Persisted in lazyjob.toml under [accessibility]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessibilityConfig {
    pub screen_reader_enabled: bool,
    pub high_contrast: bool,
    pub color_blind_mode: ColorBlindMode,
    /// Multiplier for terminal-legible spacing units.
    /// 1.0 = default; 1.5 = more line spacing; 2.0 = maximum.
    /// NOTE: terminal emulators control font size; this scales LazyJob's
    /// internal spacing decisions (padding, truncation thresholds) only.
    pub spacing_scale: f32,
    pub reduced_motion: bool,
    pub focus_indicator: FocusIndicatorStyle,
    /// If true, show an accessibility live-region bar at the bottom of the screen.
    pub show_live_region: bool,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        Self {
            screen_reader_enabled: false,
            high_contrast: false,
            color_blind_mode: ColorBlindMode::None,
            spacing_scale: 1.0,
            reduced_motion: false,
            focus_indicator: FocusIndicatorStyle::Border,
            show_live_region: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorBlindMode {
    #[default]
    None,
    /// Red-green deficiency (most common, ~6% of men)
    Deuteranopia,
    /// Red-green deficiency (less common, ~1% of men)
    Protanopia,
    /// Blue-yellow deficiency (rare)
    Tritanopia,
}

impl ColorBlindMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Deuteranopia => "Deuteranopia (red-green)",
            Self::Protanopia => "Protanopia (red-green)",
            Self::Tritanopia => "Tritanopia (blue-yellow)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FocusIndicatorStyle {
    /// No visual indicator (rely on color only — least accessible)
    None,
    Underline,
    Bold,
    #[default]
    /// Render a thick Unicode box border around the focused component
    Border,
    /// Invert foreground and background colors (ReverseVideo)
    Reverse,
}

/// Runtime-resolved profile: combines config + system detections.
/// Rebuilt on config change via App::reload_accessibility().
#[derive(Debug, Clone)]
pub struct AccessibilityProfile {
    pub config: AccessibilityConfig,
    /// True when terminal reports 256 or truecolor support
    pub terminal_has_truecolor: bool,
    /// True when REDUCE_MOTION env var is set (system-level preference)
    pub system_reduce_motion: bool,
    pub palette: ThemePalette,
}

impl AccessibilityProfile {
    pub fn build(config: AccessibilityConfig) -> Self {
        let terminal_has_truecolor = termenv::supports_color() >= termenv::ColorLevel::TrueColor;
        let system_reduce_motion = std::env::var("REDUCE_MOTION").is_ok();
        let palette = PaletteBuilder::new(&config, terminal_has_truecolor).build();
        Self { config, terminal_has_truecolor, system_reduce_motion, palette }
    }

    pub fn reduce_motion(&self) -> bool {
        self.config.reduced_motion || self.system_reduce_motion
    }
}
```

```rust
// src/accessibility/palette.rs

use ratatui::style::Color;

/// All semantically distinct colors used in the TUI.
/// Widgets never call Color::* directly; they call palette.fg(SemanticColor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticColor {
    // Base
    Background,
    Surface,
    Text,
    TextDim,
    Border,
    BorderFocused,
    // Status: application stages
    StageDiscovered,
    StageApplied,
    StagePhoneScreen,
    StageTechnical,
    StageOnsite,
    StageOffer,
    StageAccepted,
    StageRejected,
    StageWithdrawn,
    // Scores / emphasis
    MatchHigh,
    MatchMedium,
    MatchLow,
    GhostWarning,
    // UI chrome
    Accent,
    Success,
    Warning,
    Error,
    // Salary / financial
    SalaryAboveMarket,
    SalaryAtMarket,
    SalaryBelowMarket,
}

/// A fully resolved color palette.
#[derive(Debug, Clone)]
pub struct ThemePalette {
    pub bg: Color,
    pub surface: Color,
    pub text: Color,
    pub text_dim: Color,
    pub border: Color,
    pub border_focused: Color,
    pub stage_discovered: Color,
    pub stage_applied: Color,
    pub stage_phone_screen: Color,
    pub stage_technical: Color,
    pub stage_onsite: Color,
    pub stage_offer: Color,
    pub stage_accepted: Color,
    pub stage_rejected: Color,
    pub stage_withdrawn: Color,
    pub match_high: Color,
    pub match_medium: Color,
    pub match_low: Color,
    pub ghost_warning: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub salary_above: Color,
    pub salary_at: Color,
    pub salary_below: Color,
}

impl ThemePalette {
    pub fn color_for(&self, sc: SemanticColor) -> Color {
        match sc {
            SemanticColor::Background      => self.bg,
            SemanticColor::Surface         => self.surface,
            SemanticColor::Text            => self.text,
            SemanticColor::TextDim         => self.text_dim,
            SemanticColor::Border          => self.border,
            SemanticColor::BorderFocused   => self.border_focused,
            SemanticColor::StageDiscovered => self.stage_discovered,
            SemanticColor::StageApplied    => self.stage_applied,
            SemanticColor::StagePhoneScreen => self.stage_phone_screen,
            SemanticColor::StageTechnical  => self.stage_technical,
            SemanticColor::StageOnsite     => self.stage_onsite,
            SemanticColor::StageOffer      => self.stage_offer,
            SemanticColor::StageAccepted   => self.stage_accepted,
            SemanticColor::StageRejected   => self.stage_rejected,
            SemanticColor::StageWithdrawn  => self.stage_withdrawn,
            SemanticColor::MatchHigh       => self.match_high,
            SemanticColor::MatchMedium     => self.match_medium,
            SemanticColor::MatchLow        => self.match_low,
            SemanticColor::GhostWarning    => self.ghost_warning,
            SemanticColor::Accent          => self.accent,
            SemanticColor::Success         => self.success,
            SemanticColor::Warning         => self.warning,
            SemanticColor::Error           => self.error,
            SemanticColor::SalaryAboveMarket => self.salary_above,
            SemanticColor::SalaryAtMarket    => self.salary_at,
            SemanticColor::SalaryBelowMarket => self.salary_below,
        }
    }
}

/// Builder that selects the right palette given config + terminal capabilities.
pub struct PaletteBuilder<'a> {
    config: &'a AccessibilityConfig,
    truecolor: bool,
}

impl<'a> PaletteBuilder<'a> {
    pub fn new(config: &'a AccessibilityConfig, truecolor: bool) -> Self {
        Self { config, truecolor }
    }

    pub fn build(&self) -> ThemePalette {
        let base = if self.config.high_contrast {
            Self::high_contrast_dark()
        } else {
            Self::default_dark()
        };
        self.apply_color_blind_overrides(base)
    }

    fn default_dark() -> ThemePalette {
        ThemePalette {
            bg:              Color::Reset,
            surface:         Color::Rgb(30, 30, 40),
            text:            Color::White,
            text_dim:        Color::Gray,
            border:          Color::DarkGray,
            border_focused:  Color::Cyan,
            stage_discovered: Color::Gray,
            stage_applied:    Color::Cyan,
            stage_phone_screen: Color::Blue,
            stage_technical:  Color::Magenta,
            stage_onsite:     Color::Yellow,
            stage_offer:      Color::Green,
            stage_accepted:   Color::LightGreen,
            stage_rejected:   Color::Red,
            stage_withdrawn:  Color::DarkGray,
            match_high:       Color::LightGreen,
            match_medium:     Color::Yellow,
            match_low:        Color::Gray,
            ghost_warning:    Color::LightRed,
            accent:           Color::Cyan,
            success:          Color::Green,
            warning:          Color::Yellow,
            error:            Color::Red,
            salary_above:     Color::Green,
            salary_at:        Color::Yellow,
            salary_below:     Color::Red,
        }
    }

    fn high_contrast_dark() -> ThemePalette {
        ThemePalette {
            bg:              Color::Black,
            surface:         Color::Black,
            text:            Color::White,
            text_dim:        Color::Gray,
            border:          Color::White,
            border_focused:  Color::LightYellow,
            stage_discovered: Color::Gray,
            stage_applied:    Color::LightCyan,
            stage_phone_screen: Color::LightBlue,
            stage_technical:  Color::LightMagenta,
            stage_onsite:     Color::LightYellow,
            stage_offer:      Color::LightGreen,
            stage_accepted:   Color::LightGreen,
            stage_rejected:   Color::LightRed,
            stage_withdrawn:  Color::DarkGray,
            match_high:       Color::LightGreen,
            match_medium:     Color::LightYellow,
            match_low:        Color::Gray,
            ghost_warning:    Color::LightRed,
            accent:           Color::LightYellow,
            success:          Color::LightGreen,
            warning:          Color::LightYellow,
            error:            Color::LightRed,
            salary_above:     Color::LightGreen,
            salary_at:        Color::LightYellow,
            salary_below:     Color::LightRed,
        }
    }

    fn apply_color_blind_overrides(&self, mut p: ThemePalette) -> ThemePalette {
        match self.config.color_blind_mode {
            ColorBlindMode::None => {}

            // Deuteranopia + Protanopia: both are red-green deficiencies.
            // Replace all red/green semantic uses with blue/purple/cyan variants
            // that remain distinguishable for dichromats.
            ColorBlindMode::Deuteranopia | ColorBlindMode::Protanopia => {
                p.stage_offer    = Color::LightBlue;     // was Green
                p.stage_accepted = Color::LightBlue;     // was LightGreen
                p.stage_rejected = Color::LightMagenta;  // was Red
                p.match_high     = Color::LightBlue;     // was LightGreen
                p.match_low      = Color::DarkGray;      // was Gray (safe)
                p.success        = Color::LightBlue;
                p.error          = Color::LightMagenta;
                p.salary_above   = Color::LightBlue;
                p.salary_below   = Color::LightMagenta;
            }

            // Tritanopia: blue-yellow deficiency.
            // Avoid blue and yellow as primary signal colors.
            ColorBlindMode::Tritanopia => {
                p.stage_phone_screen = Color::LightGreen; // was Blue
                p.stage_onsite       = Color::LightMagenta; // was Yellow
                p.match_medium       = Color::LightMagenta; // was Yellow
                p.warning            = Color::LightMagenta;
                p.salary_at          = Color::LightMagenta;
            }
        }
        p
    }
}
```

```rust
// src/accessibility/palette.rs — StatusSymbol

/// Unicode symbols that augment color for status display.
/// Each symbol must be a single terminal cell wide (verified via unicode-width).
#[derive(Debug, Clone, Copy)]
pub struct StatusSymbol;

impl StatusSymbol {
    pub fn for_stage(stage: ApplicationStage) -> &'static str {
        match stage {
            ApplicationStage::Discovered   => "○",  // empty circle
            ApplicationStage::Applied      => "◎",  // bullseye
            ApplicationStage::PhoneScreen  => "◐",  // half circle left
            ApplicationStage::Technical    => "◑",  // half circle right
            ApplicationStage::Onsite       => "◕",  // large circle
            ApplicationStage::Offer        => "▲",  // triangle = milestone
            ApplicationStage::Accepted     => "●",  // filled circle = success
            ApplicationStage::Rejected     => "■",  // square = terminal negative
            ApplicationStage::Withdrawn    => "◻",  // empty square
        }
    }

    pub fn for_match_score(score: f32) -> &'static str {
        if score >= 0.80 { "★" }
        else if score >= 0.50 { "☆" }
        else { "·" }
    }

    pub fn ghost_warning() -> &'static str { "⚠" }
}
```

```rust
// src/accessibility/live_region.rs

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use ratatui::{Frame, layout::Rect, widgets::Paragraph, style::Style};

/// Thread-safe queue of plain-text announcements for screen readers.
/// The most recent message is displayed in the AccessibilityBar at the
/// bottom of the terminal. Screen readers that track the terminal cursor
/// (Orca in focus-tracking mode, tmux-accessibility, VoiceOver iTerm2)
/// will read this string on update.
#[derive(Debug, Clone)]
pub struct LiveRegion {
    inner: Arc<Mutex<LiveRegionInner>>,
}

#[derive(Debug)]
struct LiveRegionInner {
    history: VecDeque<String>,
    current: Option<String>,
}

impl LiveRegion {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveRegionInner {
                history: VecDeque::with_capacity(32),
                current: None,
            })),
        }
    }

    /// Push a new announcement. Replaces the current displayed message.
    /// Call on meaningful state transitions: page load, search complete,
    /// dialog open, operation success/failure.
    pub fn announce(&self, msg: impl Into<String>) {
        let msg = msg.into();
        let mut g = self.inner.lock().unwrap();
        g.history.push_back(msg.clone());
        if g.history.len() > 32 {
            g.history.pop_front();
        }
        g.current = Some(msg);
    }

    pub fn current(&self) -> Option<String> {
        self.inner.lock().unwrap().current.clone()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().current = None;
    }

    pub fn history(&self) -> Vec<String> {
        self.inner.lock().unwrap().history.iter().cloned().collect()
    }
}

/// ratatui widget that renders the live region bar.
pub struct AccessibilityBar<'a> {
    pub region: &'a LiveRegion,
    pub palette: &'a ThemePalette,
}

impl<'a> ratatui::widgets::Widget for AccessibilityBar<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::style::{Color, Modifier};
        let text = self.region.current()
            .unwrap_or_default();
        // Render with reversed video so it's visually distinct from status bar
        let style = Style::default()
            .fg(self.palette.color_for(SemanticColor::Background))
            .bg(self.palette.color_for(SemanticColor::TextDim));
        let p = Paragraph::new(format!("[ {} ]", text))
            .style(style);
        p.render(area, buf);
    }
}
```

```rust
// src/accessibility/focus.rs

/// Ordered list of focusable targets within a view.
/// Tab/Shift-Tab cycle through them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FocusTarget {
    JobList,
    JobDetailPane,
    SearchInput,
    FilterPanel,
    ApplicationForm,
    SettingsField(usize),  // index into the settings field list
    Button(String),
    // ... extend per view
}

/// A ring buffer of focus targets for a single view.
pub struct FocusRing {
    targets: Vec<FocusTarget>,
    current: usize,
}

impl FocusRing {
    pub fn new(targets: Vec<FocusTarget>) -> Self {
        assert!(!targets.is_empty(), "FocusRing requires at least one target");
        Self { targets, current: 0 }
    }

    pub fn current(&self) -> &FocusTarget {
        &self.targets[self.current]
    }

    pub fn next(&mut self) -> &FocusTarget {
        self.current = (self.current + 1) % self.targets.len();
        self.current()
    }

    pub fn prev(&mut self) -> &FocusTarget {
        self.current = self.current.checked_sub(1).unwrap_or(self.targets.len() - 1);
        self.current()
    }

    pub fn set(&mut self, target: &FocusTarget) -> bool {
        if let Some(idx) = self.targets.iter().position(|t| t == target) {
            self.current = idx;
            true
        } else {
            false
        }
    }
}
```

```rust
// src/accessibility/motion.rs

/// Controls whether TUI animations play.
/// All animation sites must call `config.should_animate()` before scheduling.
pub struct AnimationConfig {
    pub user_disabled: bool,
}

impl AnimationConfig {
    pub fn new(config: &AccessibilityConfig) -> Self {
        Self { user_disabled: config.reduced_motion }
    }

    pub fn should_animate(&self) -> bool {
        if self.user_disabled {
            return false;
        }
        // Respect OS-level env signal
        if std::env::var("REDUCE_MOTION").is_ok() {
            return false;
        }
        true
    }

    pub fn tick_rate_ms(&self) -> u64 {
        if self.should_animate() { 16 } else { 200 }
    }
}
```

### Trait Definitions

```rust
// src/accessibility/mod.rs

/// Any widget that participates in accessibility must implement this.
pub trait Accessible {
    /// Plain-text name announced on focus (e.g., "Job list: 42 jobs").
    fn accessible_name(&self) -> String;

    /// Plain-text description of current state (e.g., "Selected: Senior SWE at Stripe, 82% match").
    fn accessible_description(&self) -> String;

    /// Hint text read after name+description (e.g., "Press Enter to view details").
    fn accessible_hint(&self) -> Option<String> { None }
}
```

### SQLite Schema

No new database tables are required for this feature. The `AccessibilityConfig` is stored in the TOML config file, not in SQLite.

### Settings Persistence in TOML

```toml
# ~/.config/lazyjob/lazyjob.toml

[accessibility]
screen_reader_enabled = false
high_contrast = false
color_blind_mode = "none"   # "none" | "deuteranopia" | "protanopia" | "tritanopia"
spacing_scale = 1.0
reduced_motion = false
focus_indicator = "border"  # "none" | "underline" | "bold" | "border" | "reverse"
show_live_region = false
```

---

## Implementation Phases

### Phase 1 — Palette Indirection and Semantic Colors (MVP)

**Goal**: Eliminate all hardcoded `Color::*` from widgets; route everything through `ThemePalette`.

**Step 1.1 — Define the accessibility module**

Create `lazyjob-tui/src/accessibility/mod.rs`:
```rust
pub mod config;
pub mod palette;
pub mod live_region;
pub mod focus;
pub mod motion;

pub use config::{AccessibilityConfig, AccessibilityProfile, ColorBlindMode, FocusIndicatorStyle};
pub use palette::{ThemePalette, SemanticColor, StatusSymbol, PaletteBuilder};
pub use live_region::{LiveRegion, AccessibilityBar};
pub use focus::{FocusRing, FocusTarget};
pub use motion::AnimationConfig;
```

Create all five submodules with the types defined above.

**Step 1.2 — Add `AccessibilityProfile` to `App`**

In `lazyjob-tui/src/app.rs`:
```rust
pub struct App {
    // ... existing fields ...
    pub accessibility: AccessibilityProfile,
    pub live_region: LiveRegion,
    pub animation: AnimationConfig,
}
```

`App::new()` calls `AccessibilityProfile::build(config.accessibility.clone())`.

**Step 1.3 — Migrate all widget rendering to `SemanticColor`**

Audit every `Color::*` literal in `lazyjob-tui/src/`. Replace with `app.accessibility.palette.color_for(SemanticColor::*)`.

Key sites to migrate:
- `src/views/jobs.rs` — job list row colors (stage, match score, ghost badge)
- `src/views/applications.rs` — kanban stage colors
- `src/views/dashboard.rs` — metric bar colors
- `src/widgets/status_bar.rs` — mode indicator colors
- `src/widgets/header.rs` — accent colors

**Step 1.4 — Add `AccessibilityConfig` to `Config` struct**

In `lazyjob-core/src/config.rs` (or equivalent):
```rust
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Config {
    // ... existing ...
    #[serde(default)]
    pub accessibility: AccessibilityConfig,
}
```

**Step 1.5 — Config loading**

`Config::load()` deserializes from `~/.config/lazyjob/lazyjob.toml`. The `[accessibility]` section uses `#[serde(default)]` so missing fields use `AccessibilityConfig::default()`.

**Verification**: Run `LAZYJOB_ACCESSIBILITY_COLOR_BLIND_MODE=deuteranopia cargo run` and confirm job stage colors shift from green/red to blue/magenta.

---

### Phase 2 — Shape-Augmented Status Indicators

**Goal**: Every status display shows symbol + color, never color alone.

**Step 2.1 — `StatusSymbol::for_stage()` integration**

In `src/views/jobs.rs`, `JobRow::render()`:
```rust
let stage_sym = StatusSymbol::for_stage(job.stage);
let stage_color = palette.color_for(SemanticColor::for_stage(job.stage));
let cell = Span::styled(
    format!("{} {}", stage_sym, job.stage.label()),
    Style::default().fg(stage_color),
);
```

`SemanticColor::for_stage(stage)` is a mapping function in `palette.rs`.

**Step 2.2 — Match score symbols**

Replace percentage-only match score with `★ 82%` / `☆ 55%` / `· 23%` format. The symbol conveys the tier even when color is absent.

**Step 2.3 — Ghost warning symbol**

Ghost badge: `⚠ ghost` — the warning symbol is always present regardless of color mode.

**Step 2.4 — Salary position symbols**

In the offer comparison view:
- Above market: `↑ $185k` (up arrow + green)
- At market: `→ $165k` (right arrow + yellow)
- Below market: `↓ $140k` (down arrow + red)

**Verification**: Switch terminal to monochrome (e.g., `TERM=xterm-mono`) and confirm all status information is still readable.

---

### Phase 3 — High Contrast Modes

**Goal**: `AccessibilityConfig { high_contrast: true }` produces a palette with maximum contrast ratios.

**Step 3.1 — High contrast dark palette** (already defined in `PaletteBuilder::high_contrast_dark()` above)

All foreground colors use bright ANSI variants. Border uses `Color::White`. No `Color::Rgb()` calls — only named ANSI colors for maximum terminal compatibility.

**Step 3.2 — High contrast light palette** (Phase 3, not MVP)

```rust
fn high_contrast_light() -> ThemePalette {
    ThemePalette {
        bg:   Color::White,
        text: Color::Black,
        // ... etc
    }
}
```

Add `ColorMode::HighContrastLight` variant and wire into `PaletteBuilder::build()`.

**Step 3.3 — System preference detection**

Detect if the user is on macOS with "Increase Contrast" enabled:
```rust
fn system_high_contrast() -> bool {
    // macOS: check via `defaults read com.apple.universalaccess increaseContrast`
    // Linux: check GNOME accessibility bus via D-Bus (too heavy for MVP)
    // Conservative: check env var FORCE_HIGH_CONTRAST
    std::env::var("FORCE_HIGH_CONTRAST").is_ok()
}
```

**Verification**: Run with `high_contrast = true` in config. All text must be readable in a screenshot-based accessibility checker.

---

### Phase 4 — Color Blind Modes

**Goal**: `ColorBlindMode::Deuteranopia` / `Protanopia` / `Tritanopia` produce correct palette overrides.

**Step 4.1 — Apply palette overrides** (already in `PaletteBuilder::apply_color_blind_overrides()`)

**Step 4.2 — Verify with color simulation**

Use `convert` (ImageMagick) + a color blindness simulation filter on a terminal screenshot to verify distinguishability. Document the test procedure in `tests/accessibility/README.md`.

**Step 4.3 — No purely red/green coding**

Audit all `Color::Green` / `Color::Red` usages in the codebase. Each must also have a shape or text label accompanying it (already enforced by Phase 2). File a TODO for any remaining purely color-only distinctions.

**Verification**: Run with each `color_blind_mode` variant and confirm the kanban board stage columns remain distinguishable without relying on hue.

---

### Phase 5 — Focus Management and Indicators

**Goal**: Tab/Shift-Tab cycles through all interactive elements without traps.

**Step 5.1 — `FocusRing` integration per view**

In `JobsState`, `ApplicationsState`, `SettingsState` (and each new view as added):
```rust
pub struct JobsState {
    pub focus: FocusRing,
    // ... rest of state
}

impl Default for JobsState {
    fn default() -> Self {
        Self {
            focus: FocusRing::new(vec![
                FocusTarget::SearchInput,
                FocusTarget::FilterPanel,
                FocusTarget::JobList,
            ]),
            // ...
        }
    }
}
```

**Step 5.2 — Tab key dispatch**

In `EventLoop::handle_event()`:
```rust
KeyCode::Tab => {
    app.current_view_focus_mut().next();
    announce_focus_change(&app);
}
KeyCode::BackTab => {
    app.current_view_focus_mut().prev();
    announce_focus_change(&app);
}
```

`announce_focus_change` calls `app.live_region.announce(target.accessible_description())`.

**Step 5.3 — Render focus indicator**

In each widget's `render()`, check if it is the active focus target, then apply `FocusIndicatorStyle`:
```rust
fn apply_focus_style(style: Style, is_focused: bool, indicator: FocusIndicatorStyle) -> Style {
    if !is_focused {
        return style;
    }
    match indicator {
        FocusIndicatorStyle::None    => style,
        FocusIndicatorStyle::Bold    => style.add_modifier(Modifier::BOLD),
        FocusIndicatorStyle::Underline => style.add_modifier(Modifier::UNDERLINED),
        FocusIndicatorStyle::Reverse => style.add_modifier(Modifier::REVERSED),
        FocusIndicatorStyle::Border  => {
            // The widget's block border changes from `palette.border` to `palette.border_focused`
            style // border color set separately via Block::border_style
        }
    }
}
```

**Step 5.4 — No focus traps**

Modals use `ratatui::widgets::Clear` to render overlay. The `FocusRing` is swapped to the modal's ring for the modal's lifetime. `Escape` restores the previous ring. No path should leave focus stuck inside a modal without an Escape path.

**Verification**: Use the Tab key alone to navigate through every element of the Jobs view and the Settings view. Confirm every interactive element is reachable and the ring cycles correctly.

---

### Phase 6 — Live Region and Screen Reader Support

**Goal**: Meaningful state changes produce plain-text announcements visible to screen readers.

**Step 6.1 — `LiveRegion` integration**

Add `app.live_region: LiveRegion` to `App`. The `AccessibilityBar` widget is rendered in the last row of the layout when `accessibility.config.show_live_region == true`.

**Step 6.2 — Layout change for live region bar**

In `EventLoop::render()`, when `show_live_region` is true:
```rust
let chunks = Layout::vertical([
    Constraint::Length(1),        // header
    Constraint::Min(0),           // main content
    Constraint::Length(1),        // status bar
    Constraint::Length(1),        // accessibility bar (conditional)
]).split(frame.area());
let main_area = chunks[1];
let status_bar_area = chunks[2];
let a11y_bar_area = chunks[3];

if app.accessibility.config.show_live_region {
    AccessibilityBar { region: &app.live_region, palette: &app.accessibility.palette }
        .render(a11y_bar_area, frame.buffer_mut());
}
```

**Step 6.3 — Announcement sites**

Announce at every meaningful state transition in `EventLoop::handle_action()`:

| Action | Announcement |
|--------|-------------|
| View switches to `Jobs` | `"Jobs list: N jobs loaded"` |
| Discovery completes | `"Job discovery complete: N new jobs"` |
| Application stage advanced | `"Application moved to {stage}: {company} — {title}"` |
| Search results filtered | `"Filter applied: N jobs match"` |
| Error occurs | `"Error: {message}"` |
| Ralph loop starts | `"Ralph starting: {loop_type}"` |
| Ralph loop completes | `"Ralph done: {loop_type}"` |
| Settings saved | `"Settings saved"` |
| Modal opens | `"Dialog: {title}. Press Escape to close."` |

All calls follow:
```rust
app.live_region.announce(format!("Jobs list: {} jobs loaded", jobs.len()));
```

**Step 6.4 — Screen reader compatibility notes**

Document in `docs/accessibility.md`:
- **Linux + Orca**: Works best with `tmux-accessibility` plugin or Orca in "screen review" mode. Set `show_live_region = true` in config.
- **macOS + VoiceOver**: iTerm2 has VoiceOver integration. Navigate to the accessibility bar row after each action.
- **Windows + NVDA**: Windows Terminal + NVDA reads terminal output. The live region approach is compatible.
- **Braille displays**: PuTTY or BRLTTY with a compatible terminal emulator; live region approach is compatible.

**Verification**: With `show_live_region = true`, perform 5 actions and confirm each produces a plain-text announcement in the accessibility bar row.

---

### Phase 7 — Spacing Scale and Motion Control

**Goal**: Users with visual processing difficulties can increase spacing; reduced motion users see no animations.

**Step 7.1 — Spacing scale**

Terminal emulators control font rendering. `spacing_scale` in LazyJob affects:
- **Padding in panels**: when `scale >= 1.5`, add an extra blank row above list items
- **Truncation thresholds**: increase the minimum characters shown before truncation by `scale * base_threshold`
- **Line height in lists**: render list items with an intervening blank `Span` when `scale >= 2.0`

```rust
pub fn list_row_height(scale: f32) -> u16 {
    if scale >= 2.0 { 2 } else { 1 }
}

pub fn panel_padding(scale: f32) -> u16 {
    if scale >= 1.5 { 1 } else { 0 }
}
```

**Step 7.2 — Reduced motion: disable spinner**

The Ralph activity spinner (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`) cycles at the `tick_rate_ms()` from `AnimationConfig`. When `reduce_motion()` returns true, replace the spinner with a static `[running]` text:
```rust
fn ralph_status_span(state: &RalphLoopState, anim: &AnimationConfig) -> Span {
    if anim.should_animate() {
        let frame = SPINNER_FRAMES[state.tick % SPINNER_FRAMES.len()];
        Span::raw(format!("{} Running", frame))
    } else {
        Span::raw("[running]")
    }
}
```

**Step 7.3 — No blinking cursors**

Confirm that `crossterm::cursor::SetCursorStyle::SteadyBlock` is set on startup (already planned in the TUI keybindings implementation plan). Blinking cursor is never used.

**Step 7.4 — Instant state transitions**

When `reduce_motion()` is true, any `sleep`-based animation frames in the event loop are skipped. (If no animations exist yet, this is a no-op guard for future code.)

**Verification**: Set `reduced_motion = true` and `spacing_scale = 2.0`. Verify list rows have extra spacing and spinner shows static `[running]`.

---

### Phase 8 — Accessibility Settings View

**Goal**: A dedicated TUI view lets users change all accessibility settings with live preview.

**Step 8.1 — `AccessibilitySettingsView` struct**

```rust
// src/views/accessibility_settings.rs

pub struct AccessibilitySettingsView {
    pub config: AccessibilityConfig,      // in-progress (not yet saved)
    pub focus: FocusRing,
    pub preview_palette: ThemePalette,   // rebuilt on every change for live preview
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsField {
    ScreenReaderEnabled,
    HighContrast,
    ColorBlindMode,
    SpacingScale,
    ReducedMotion,
    FocusIndicator,
    ShowLiveRegion,
    ButtonPreview,
    ButtonSave,
    ButtonCancel,
}
```

**Step 8.2 — Render layout**

The settings view renders as a `Block`-bordered panel with labeled fields. Each field row shows:
- Field name (left, 40% width)
- Current value (right, 60% width)
- Focused fields are highlighted with the active `FocusIndicatorStyle`

Toggle fields (`bool`) use `[ON]` / `[OFF]` labels.
Enum fields use `[← Prev]  Current Value  [Next →]` layout.
Slider fields (`spacing_scale`) use a `Gauge` widget from ratatui.

**Step 8.3 — Key handling**

| Key | Action |
|-----|--------|
| Tab / Shift-Tab | Move focus between fields |
| Space / Enter | Toggle bool field / activate button |
| Left / Right | Cycle enum values |
| +/- | Adjust spacing_scale by 0.1 (clamped to 0.8–2.0) |
| s | Save to `lazyjob.toml`, rebuild palette, return to previous view |
| Escape | Discard changes, return to previous view |
| p | Preview — rebuild `preview_palette` from current in-progress config |

**Step 8.4 — Live preview panel**

Below the form, render a small preview panel showing:
- A mock job row with stage symbol + color
- A match score badge
- A ghost warning badge

This panel uses `preview_palette` so changes are visible immediately (no need to save first).

**Step 8.5 — Save path**

On `s` press:
1. Serialize `config` to TOML fragment
2. Call `Config::save()` which atomically writes `lazyjob.toml` (write to temp file, rename)
3. Call `App::reload_accessibility()` which rebuilds `AccessibilityProfile` from the saved config
4. Announce `"Settings saved"` to live region
5. Return to previous view

**Verification**: Open the settings view, change color blind mode, press `p` — confirm the preview row reflects the new palette without saving. Press `s`, navigate to jobs view — confirm the job list uses the new palette.

---

## Key Crate APIs

- `ratatui::style::Color` — `Color::Reset`, `Color::Black`, `Color::White`, named ANSI colors, `Color::Rgb(r,g,b)` for truecolor palettes
- `ratatui::style::Modifier` — `Modifier::BOLD`, `Modifier::UNDERLINED`, `Modifier::REVERSED`
- `ratatui::widgets::Paragraph::new(text).style(style)` — for live region bar
- `ratatui::widgets::Block::borders(Borders::ALL).border_style(style)` — for focus border indicator
- `ratatui::widgets::Gauge::default().percent(n)` — for spacing scale slider
- `crossterm::cursor::SetCursorStyle::SteadyBlock` — non-blinking cursor
- `termenv::supports_color()` — returns `ColorLevel::TrueColor`, `ColorLevel::Ansi256`, or `ColorLevel::Ansi`
- `std::env::var("REDUCE_MOTION")` — OS-level motion preference signal
- `toml::to_string_pretty(&config)` + `std::fs::write(path, content)` — config persistence
- `unicode_width::UnicodeWidthChar::width(c)` — verify symbols are single-cell wide

---

## Error Handling

```rust
// src/accessibility/mod.rs

#[derive(Debug, thiserror::Error)]
pub enum AccessibilityError {
    #[error("failed to read accessibility config: {0}")]
    ConfigReadFailed(#[from] std::io::Error),

    #[error("failed to parse accessibility config: {0}")]
    ConfigParseFailed(#[from] toml::de::Error),

    #[error("failed to write accessibility config: {0}")]
    ConfigWriteFailed(String),

    #[error("invalid spacing_scale {value}: must be between 0.8 and 2.0")]
    InvalidSpacingScale { value: f32 },
}

pub type AccessibilityResult<T> = Result<T, AccessibilityError>;
```

`AccessibilityError::ConfigReadFailed` is non-fatal at startup: if the `[accessibility]` section is missing from `lazyjob.toml`, `AccessibilityConfig::default()` is used instead.

`AccessibilityError::ConfigWriteFailed` is shown as a dismissable TUI error dialog. The in-memory config still updates (the save failure is surfaced but does not block use).

---

## Testing Strategy

### Unit Tests

**`palette.rs` tests** — `#[test]` functions, no terminal needed:
```rust
#[test]
fn deuteranopia_has_no_pure_green_status_colors() {
    let config = AccessibilityConfig { color_blind_mode: ColorBlindMode::Deuteranopia, ..Default::default() };
    let palette = PaletteBuilder::new(&config, false).build();
    // stage_accepted must not be Color::Green or Color::LightGreen
    assert_ne!(palette.stage_accepted, Color::Green);
    assert_ne!(palette.stage_accepted, Color::LightGreen);
}

#[test]
fn high_contrast_text_is_always_white_on_black() {
    let config = AccessibilityConfig { high_contrast: true, ..Default::default() };
    let palette = PaletteBuilder::new(&config, false).build();
    assert_eq!(palette.text, Color::White);
    assert_eq!(palette.bg, Color::Black);
}

#[test]
fn default_palette_differs_from_high_contrast() {
    let default = PaletteBuilder::new(&AccessibilityConfig::default(), true).build();
    let hc = PaletteBuilder::new(&AccessibilityConfig { high_contrast: true, ..Default::default() }, true).build();
    assert_ne!(default.bg, hc.bg);
}
```

**`focus.rs` tests**:
```rust
#[test]
fn focus_ring_cycles_forward_and_back() {
    let mut ring = FocusRing::new(vec![FocusTarget::SearchInput, FocusTarget::JobList, FocusTarget::FilterPanel]);
    assert_eq!(ring.current(), &FocusTarget::SearchInput);
    ring.next();
    assert_eq!(ring.current(), &FocusTarget::JobList);
    ring.prev();
    assert_eq!(ring.current(), &FocusTarget::SearchInput);
    // wrap around backward
    ring.prev();
    assert_eq!(ring.current(), &FocusTarget::FilterPanel);
}
```

**`live_region.rs` tests**:
```rust
#[test]
fn announce_updates_current() {
    let lr = LiveRegion::new();
    assert!(lr.current().is_none());
    lr.announce("Job discovery complete: 5 new jobs");
    assert_eq!(lr.current(), Some("Job discovery complete: 5 new jobs".to_string()));
    lr.clear();
    assert!(lr.current().is_none());
}
```

**`motion.rs` tests**:
```rust
#[test]
fn reduced_motion_disables_animation() {
    let config = AccessibilityConfig { reduced_motion: true, ..Default::default() };
    let anim = AnimationConfig::new(&config);
    assert!(!anim.should_animate());
}

#[test]
fn default_config_allows_animation() {
    let config = AccessibilityConfig::default();
    let anim = AnimationConfig::new(&config);
    // Only true when REDUCE_MOTION env is not set
    // Use temp env:
    let _ = std::env::remove_var("REDUCE_MOTION");
    assert!(anim.should_animate());
}
```

**`StatusSymbol` tests**:
```rust
#[test]
fn all_stage_symbols_are_single_cell_wide() {
    use unicode_width::UnicodeWidthStr;
    for stage in ApplicationStage::all_variants() {
        let sym = StatusSymbol::for_stage(stage);
        assert_eq!(UnicodeWidthStr::width(sym), 1, "Symbol {:?} for {:?} is not single-cell", sym, stage);
    }
}
```

### Integration Tests

**Config round-trip test**:
```rust
#[test]
fn accessibility_config_serializes_and_deserializes() {
    let config = AccessibilityConfig {
        high_contrast: true,
        color_blind_mode: ColorBlindMode::Tritanopia,
        spacing_scale: 1.5,
        ..Default::default()
    };
    let s = toml::to_string_pretty(&config).unwrap();
    let back: AccessibilityConfig = toml::from_str(&s).unwrap();
    assert_eq!(back.color_blind_mode, ColorBlindMode::Tritanopia);
    assert!((back.spacing_scale - 1.5).abs() < 0.001);
}
```

### TUI Tests

ratatui's `TestBackend` allows rendering widgets to a buffer and asserting on character/style values:
```rust
#[test]
fn accessibility_bar_renders_message() {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    let backend = TestBackend::new(40, 1);
    let mut term = Terminal::new(backend).unwrap();
    let lr = LiveRegion::new();
    lr.announce("5 new jobs found");
    let palette = PaletteBuilder::new(&AccessibilityConfig::default(), false).build();
    term.draw(|frame| {
        AccessibilityBar { region: &lr, palette: &palette }
            .render(Rect::new(0, 0, 40, 1), frame.buffer_mut());
    }).unwrap();
    let buf = term.backend().buffer().clone();
    let rendered: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(rendered.contains("5 new jobs found"), "got: {}", rendered);
}
```

### Manual Accessibility Testing Checklist

Document in `docs/accessibility-testing.md`:

1. **Color blindness simulation**: Screenshot the kanban board. Pass through [Coblis](https://www.color-blindness.com/coblis-color-blindness-simulator/) for each of the 3 CVD types. Confirm stage columns remain visually distinct.
2. **Monochrome terminal**: `TERM=xterm` (16 colors). Confirm all stage symbols remain readable.
3. **High contrast**: Set `high_contrast = true`. Confirm text contrast in a terminal screenshot.
4. **Tab navigation**: Using only Tab/Shift-Tab and Space/Enter, complete the workflow: navigate to Jobs → select a job → open Application form → submit. No mouse required.
5. **Screen reader on Linux**: Install Orca. Open LazyJob in GNOME Terminal. Enable Orca. Navigate the jobs view. Set `show_live_region = true`. Confirm Orca reads the live region bar on state transitions.
6. **Reduced motion**: Set `reduced_motion = true`. Confirm Ralph spinner shows static `[running]` and no visual animations play.
7. **Spacing scale**: Set `spacing_scale = 2.0`. Confirm list items have increased vertical spacing.

---

## Open Questions

1. **Orca + tmux compatibility**: tmux intercepts terminal output in ways that can confuse screen readers. The live region approach is the safest path, but the exact tmux configuration needed for Orca to track it needs empirical testing. Consider documenting a recommended tmux config in `docs/accessibility.md`.

2. **System high-contrast detection on Linux**: macOS has `defaults read com.apple.universalaccess increaseContrast`. Linux accessibility preferences live on the AT-SPI D-Bus, which is heavy to integrate. For MVP, use only the `FORCE_HIGH_CONTRAST` environment variable. Post-MVP: integrate with `at-spi2-core` D-Bus queries.

3. **`spacing_scale` in fully rendered multi-column layouts**: The kanban board uses exact `Rect` arithmetic. When `spacing_scale = 2.0` adds blank rows, some columns may become too short to render correctly. Need to define a minimum row height and column height constraint that blocks the scale from taking effect in layouts where it would break rendering.

4. **Font size scaling beyond spacing**: Some terminal emulators (Kitty, WezTerm) support per-pane font size via escape sequences. A future `font_size_delta` setting could emit these sequences for supported terminals while degrading gracefully to `spacing_scale` for others. Not in scope for this plan.

5. **Braille display testing**: Braille displays work with BRLTTY on Linux. Testing requires physical hardware or a VM with BRLTTY emulation. Document as a known-untested configuration.

6. **International keyboard layouts**: Tab, Shift-Tab, and Escape are universal. Vim motions (`h`, `j`, `k`, `l`) are ASCII and work across layouts. The primary concern is non-QWERTY users who have remapped Escape. The existing keybinding configurability system (from the TUI keybindings plan) should be sufficient.

---

## Related Specs

- `specs/09-tui-design-keybindings.md` — base TUI architecture, `App`, `EventLoop`, `ColorPalette` stub
- `specs/08-gaps-salary-tui.md` — GAP-78 initial `AccessibilityConfig` definition (this plan supersedes and expands it)
- `specs/XX-tui-vim-mode.md` — vim mode's `VimState::process_key` must pass focus events through `FocusRing` correctly
- `specs/16-privacy-security.md` — `lazyjob.toml` encryption: accessibility settings are non-sensitive and should remain in plaintext even when DB is encrypted
