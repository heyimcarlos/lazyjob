# Spec: TUI Accessibility (Screen Readers, Color Blind Mode)

## Context

LazyJob TUI must be accessible to users with visual impairments, color blindness, and other accessibility needs. This spec addresses screen reader support, high contrast modes, and color blind accommodations.

## Motivation

- **User inclusion**: A significant portion of users have accessibility needs
- **Legal compliance**: ADA requires reasonable accessibility accommodations
- **Vim mode integration**: Screen reader + vim navigation must work together

## Design

### Accessibility Profile

```rust
pub struct AccessibilityProfile {
    pub screen_reader_enabled: bool,
    pub high_contrast_mode: bool,
    pub color_blind_mode: ColorBlindMode,
    pub font_size_scale: f32,      // 1.0 = default, 1.5 = 50% larger
    pub reduced_motion: bool,
    pub focus_indicators: FocusIndicator,
}

pub enum ColorBlindMode {
    None,
    Deuteranopia,    // Red-green (most common)
    Protanopia,      // Red-green
    Tritanopia,      // Blue-yellow
}

pub enum FocusIndicator {
    None,
    Underline,
    Bold,
    Box,
}
```

### Screen Reader Support

Ratuit has experimental accessibility support. Key patterns:

```rust
impl TUIComponent {
    /// Get accessible name for screen reader
    fn accessible_name(&self) -> String {
        // E.g., "Job card: Senior Software Engineer at Stripe, San Francisco"
    }
    
    /// Get accessible description with state
    fn accessible_description(&self) -> String {
        // E.g., "Selected, 80% match score, ghost job warning"
    }
}
```

**Strategies**:
1. **ARIA-like labels**: All interactive elements have descriptive labels
2. **Live regions**: Status changes announced (e.g., "Job discovery complete, 15 new jobs")
3. **Focus management**: Logical focus order, no focus traps

### High Contrast Mode

```rust
const HIGH_CONTRAST_PALETTE: ColorPalette = ColorPalette {
    primary: Color::White,
    secondary: Color::BrightBlack,
    background: Color::Black,
    text: Color::White,
    accent: Color::BrightYellow,
    status_interview: Color::BrightGreen,
    status_rejected: Color::BrightRed,
    status_pending: Color::BrightCyan,
    border: Color::White,
};
```

All text on dark background with maximum contrast ratios.

### Color Blind Modes

```rust
pub fn adjusted_palette(base: &ColorPalette, mode: ColorBlindMode) -> ColorPalette {
    match mode {
        ColorBlindMode::Deuteranopia | ColorBlindMode::Protanopia => {
            // Replace red/green with patterns + blue/purple
            let mut p = base.clone();
            p.status_interview = Color::BrightBlue;      // Blue = positive
            p.status_rejected = Color::BrightMagenta;    // Magenta = negative
            p.status_pending = Color::BrightCyan;        // Cyan = neutral
            p
        }
        ColorBlindMode::Tritanopia => {
            // Blue-yellow color blind - use shapes
            let mut p = base.clone();
            p.status_interview = Color::BrightGreen;
            p.status_rejected = Color::BrightRed;
            p.status_pending = Color::BrightYellow;
            p
        }
        ColorBlindMode::None => base.clone(),
    }
}
```

**Status indicators** include shape + color, not color alone:

```
[●] Interview scheduled  (circle = positive)
[■] Application rejected   (square = negative)
[▲] Offer received        (triangle = milestone)
```

### Font Size Scaling

```rust
impl TUIView {
    fn render_with_scale(&self, scale: f32) {
        // Scale all text sizes by scale factor
        // Minimum: 0.8x (below this, too small)
        // Maximum: 2.0x (2x magnification)
    }
}
```

Configured in settings:

```toml
[accessibility]
font_size_scale = 1.5  # 50% larger text
screen_reader = true
high_contrast = false
color_blind_mode = "deuteranopia"
```

### Reduced Motion

```rust
impl AnimationConfig {
    pub fn should_animate(&self, profile: &AccessibilityProfile) -> bool {
        if profile.reduced_motion {
            return false;
        }
        // Check system preference via termenv
        std::env::var("REDUCE_MOTION").is_ok()
    }
}
```

- Disable scroll animations
- Instant state transitions (no fade)
- No blinking cursors (use solid instead)

### Focus Indicators

```rust
fn render_focused(widget: &Widget, indicator: FocusIndicator) {
    match indicator {
        FocusIndicator::Underline => {
            // Render underlined text
        }
        FocusIndicator::Bold => {
            // Render bold text
        }
        FocusIndicator::Box => {
            // Render with border
        }
        FocusIndicator::None => {
            // Rely on color only
        }
    }
}
```

### Settings UI

```
┌─────────────────────────────────────────────────────────────┐
│  Accessibility Settings                                      │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Screen Reader Mode        [ ON  / OFF ]                    │
│                                                             │
│  Display                                                              │
│    High Contrast Mode      [ ON  / OFF ]                    │
│    Color Blind Mode         [ None ▼ ]                      │
│      → Deuteranopia (red-green)                             │
│      → Protanopia (red-green)                               │
│      → Tritanopia (blue-yellow)                             │
│                                                             │
│  Text                                                              │
│    Font Size              [────●────] 1.5x                  │
│                                                             │
│  Motion                                                             │
│    Reduced Motion         [ ON  / OFF ]                     │
│                                                             │
│  Focus Indicators         [ Box ▼ ]                         │
│      → None                                               │
│      → Underline                                         │
│      → Bold                                            │
│      → Box                                            │
│                                                             │
│  [Preview Changes]                                          │
└─────────────────────────────────────────────────────────────┘
```

## Implementation Notes

- Accessibility profile stored in `lazyjob.toml`
- System accessibility prefs detected via `termenv`
- Preview shows changes before applying
- All color usages go through `palette.adjusted()` function

## Open Questions

1. **Screen reader testing**: What tools to use for testing?
2. **Linux screen reader support**: Orca + tmux compatibility?
3. **Mobile companion accessibility**: Beyond TUI scope?

## Related Specs

- `09-tui-design-keybindings.md` - TUI design
- `16-privacy-security.md` - User preferences storage