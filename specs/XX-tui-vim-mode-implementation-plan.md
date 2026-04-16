# Implementation Plan: TUI Vim Mode

## Status
Draft

## Related Spec
`specs/XX-tui-vim-mode.md`

## Overview

This plan specifies how to build a complete vim modal editing system for the LazyJob TUI. LazyJob targets developers who expect real vim semantics — not just vim-like keybindings — so the implementation must faithfully emulate Normal, Insert, Visual, VisualLine, Command, and Search modes with operator-motion composition, text objects, registers, and macro recording.

The vim layer is a pure Rust state machine that receives `crossterm::event::KeyEvent` values and produces `VimAction` outputs consumed by the TUI event loop. It has no I/O and no async dependencies, making it trivially unit-testable. All editor widgets (text input, detail panels, note editors) integrate by embedding a `VimState` and delegating key events to the vim engine before applying the resulting action.

The implementation is phased: Phase 1 delivers the core mode machine, normal-mode motions, and insert mode for text fields. Phase 2 adds operator-motion composition, visual mode, and text objects. Phase 3 adds command mode (ex commands), search, and leader-key sequences. Phase 4 adds registers, macro recording/playback, and per-user configuration.

## Prerequisites

### Specs/plans that must be implemented first
- `specs/09-tui-design-keybindings-implementation-plan.md` — the `App`, `View`, `InputMode`, and `EventLoop` types from this plan are extended by the vim layer. The existing `InputMode::Normal`/`Insert` enum is replaced by `VimMode`.
- `specs/04-sqlite-persistence-implementation-plan.md` — vim state is not persisted to SQLite, but the TUI event loop structure must exist.

### Crates to add
```toml
# lazyjob-tui/Cargo.toml
[dependencies]
# Already present from spec 09:
ratatui   = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
unicode-width = "0.2"   # grapheme cluster width for cursor placement

# New for vim mode:
unicode-segmentation = "1.12"   # grapheme cluster iteration (move-by-grapheme, not byte)
```

No async crates are needed — the vim engine is entirely synchronous.

---

## Architecture

### Crate Placement

All vim mode code lives in `lazyjob-tui/src/vim/`. The engine is a pure library sub-module with no dependencies on `ratatui` or `crossterm` types (it works with our own `Key` wrapper). Only the TUI integration layer (`lazyjob-tui/src/vim/integration.rs`) imports `crossterm::event::KeyEvent`.

Keeping the engine free of `crossterm` means it can be tested with simple string-driven tests and reused across any future UI layer.

### Module Structure

```
lazyjob-tui/
  src/
    vim/
      mod.rs            # pub re-exports: VimState, VimAction, VimEngine
      mode.rs           # VimMode enum, mode transitions
      key.rs            # Key newtype wrapping crossterm KeyCode+Modifiers
      motion.rs         # Motion enum + resolve_motion()
      operator.rs       # Operator enum + execute_operator()
      text_object.rs    # TextObject enum + resolve_text_object()
      register.rs       # RegisterBank struct (a-z, 0-9, +, *, ", -)
      macro_.rs         # MacroRecorder struct
      command.rs        # ExCommand enum + ExCommandParser
      search.rs         # SearchState, SearchDirection, incremental highlight
      visual.rs         # VisualAnchor, selection range computation
      action.rs         # VimAction enum (output of the engine)
      config.rs         # VimConfig loaded from config.toml [vim] section
      engine.rs         # VimEngine — top-level event handler
      integration.rs    # crossterm::KeyEvent → Key conversion
    widgets/
      vim_text_area.rs  # Editable text widget embedding VimState
      search_bar.rs     # Search-mode input bar (bottom of screen)
      command_line.rs   # Command-mode input bar (`:` prefix)
    app.rs              # App struct — embed VimState per view
```

### Core Types

#### Key abstraction

```rust
// lazyjob-tui/src/vim/key.rs

/// Platform-independent key representation.
/// Wraps crossterm's KeyCode but removes terminal-specific noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    F(u8),
    Esc,
    Enter,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Null,
}

impl Key {
    pub fn is_printable(&self) -> bool {
        matches!(self, Key::Char(_))
    }
}
```

#### Mode

```rust
// lazyjob-tui/src/vim/mode.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
    Visual,       // character-wise selection
    VisualLine,   // line-wise selection
    Command,      // ex command input: ":"
    Search,       // forward "/" or backward "?"
}

impl VimMode {
    /// Text shown in the TUI status bar.
    pub fn status_label(&self) -> &'static str {
        match self {
            VimMode::Normal     => "NORMAL",
            VimMode::Insert     => "INSERT",
            VimMode::Visual     => "VISUAL",
            VimMode::VisualLine => "V-LINE",
            VimMode::Command    => "COMMAND",
            VimMode::Search     => "SEARCH",
        }
    }

    /// Cursor style sent to crossterm when entering this mode.
    pub fn cursor_style(&self) -> crossterm::cursor::SetCursorStyle {
        use crossterm::cursor::SetCursorStyle;
        match self {
            VimMode::Insert => SetCursorStyle::BlinkingBar,
            _               => SetCursorStyle::SteadyBlock,
        }
    }

    pub fn is_editing(&self) -> bool {
        matches!(self, VimMode::Insert | VimMode::Command | VimMode::Search)
    }
}
```

#### Motion

```rust
// lazyjob-tui/src/vim/motion.rs

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,           // w
    WordForwardWide,       // W
    WordBackward,          // b
    WordBackwardWide,      // B
    WordEnd,               // e
    WordEndWide,           // E
    WordEndBackward,       // ge
    LineStart,             // 0
    LineFirstNonWhite,     // ^
    LineEnd,               // $
    FileStart,             // gg
    FileEnd,               // G
    ParagraphForward,      // }
    ParagraphBackward,     // {
    SentenceForward,       // )
    SentenceBackward,      // (
    FindCharForward(char), // f<c>
    FindCharBackward(char),// F<c>
    TillCharForward(char), // t<c>
    TillCharBackward(char),// T<c>
    RepeatFind,            // ;
    RepeatFindReverse,     // ,
    MatchBracket,          // %
    SearchNext,            // n
    SearchPrev,            // N
    WordUnderCursor,       // *
    WordUnderCursorBack,   // #
    TextObject(TextObject),
}

/// A Motion can specify a count: "3w" = WordForward with count 3.
#[derive(Clone, Debug)]
pub struct CountedMotion {
    pub count: usize,  // 1 = no repeat
    pub motion: Motion,
}
```

#### Operator

```rust
// lazyjob-tui/src/vim/operator.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,    // d
    Yank,      // y
    Change,    // c  (delete + enter Insert)
    Indent,    // >
    Dedent,    // <
    AutoIndent,// =
    SwapCase,  // g~ (toggle case)
    Upper,     // gU
    Lower,     // gu
}
```

#### Text Object

```rust
// lazyjob-tui/src/vim/text_object.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Inner, // i
    Around,// a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextObject {
    Word(Boundary),          // iw / aw
    WideWord(Boundary),      // iW / aW
    DoubleQuote(Boundary),   // i" / a"
    SingleQuote(Boundary),   // i' / a'
    Backtick(Boundary),      // i` / a`
    Paren(Boundary),         // i( / a(
    Bracket(Boundary),       // i[ / a[
    Brace(Boundary),         // i{ / a{
    AngleBracket(Boundary),  // i< / a<
    Paragraph(Boundary),     // ip / ap
    Sentence(Boundary),      // is / as
}
```

#### Register Bank

```rust
// lazyjob-tui/src/vim/register.rs

use std::collections::HashMap;

/// Named registers: "a–"z, "0–"9, "+ (system), "* (primary), "" (unnamed), "- (small delete)
#[derive(Debug, Default)]
pub struct RegisterBank {
    named: HashMap<char, RegisterValue>,   // 'a'..'z'
    numbered: [Option<RegisterValue>; 10], // 0..=9
    unnamed: Option<RegisterValue>,        // ""
    small_delete: Option<RegisterValue>,   // "-
    system: Option<RegisterValue>,         // "+ — backed by arboard if available
    primary: Option<RegisterValue>,        // "* — X11 PRIMARY selection
}

#[derive(Clone, Debug)]
pub struct RegisterValue {
    pub text: String,
    pub kind: RegisterKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterKind {
    Characterwise,
    Linewise,
    Blockwise,
}

impl RegisterBank {
    pub fn yank(&mut self, reg: char, text: String, kind: RegisterKind) {
        let value = RegisterValue { text, kind };
        match reg {
            '"' => { self.unnamed = Some(value.clone()); }
            '-' => { self.small_delete = Some(value.clone()); }
            '+' => { self.system = Some(value.clone()); }
            '*' => { self.primary = Some(value.clone()); }
            'a'..='z' => {
                self.named.insert(reg, value.clone());
                self.unnamed = Some(value);
            }
            '0'..='9' => {
                let idx = (reg as u8 - b'0') as usize;
                self.numbered[idx] = Some(value.clone());
                self.unnamed = Some(value);
            }
            _ => {}
        }
    }

    pub fn get(&self, reg: char) -> Option<&RegisterValue> {
        match reg {
            '"' => self.unnamed.as_ref(),
            '-' => self.small_delete.as_ref(),
            '+' => self.system.as_ref(),
            '*' => self.primary.as_ref(),
            'a'..='z' => self.named.get(&reg),
            '0'..='9' => {
                let idx = (reg as u8 - b'0') as usize;
                self.numbered[idx].as_ref()
            }
            _ => None,
        }
    }
}
```

#### Visual Selection

```rust
// lazyjob-tui/src/vim/visual.rs

/// Tracks the anchor point when visual mode is entered.
/// The selection spans from `anchor` to `cursor` (inclusive), ordered in the buffer.
#[derive(Clone, Debug)]
pub struct VisualAnchor {
    /// Byte offset in the buffer where visual mode was entered.
    pub byte_offset: usize,
}

/// Resolved selection — always start <= end in buffer byte offsets.
#[derive(Clone, Debug)]
pub struct Selection {
    pub start: usize,
    pub end: usize,   // inclusive
    pub kind: SelectionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Characterwise,
    Linewise,
}
```

#### VimAction (output)

```rust
// lazyjob-tui/src/vim/action.rs

/// The output of the vim engine for a single key event.
/// The TUI event loop (or widget) applies these to its own state.
#[derive(Debug, Clone)]
pub enum VimAction {
    // ── Text mutations ──────────────────────────────────────────────
    Insert(String),           // insert text at cursor
    DeleteRange(usize, usize),// delete bytes start..=end (inclusive) in buffer
    ReplaceRange { start: usize, end: usize, replacement: String },

    // ── Cursor movement ─────────────────────────────────────────────
    MoveCursor(CursorMove),

    // ── Mode transitions ─────────────────────────────────────────────
    EnterMode(VimMode),

    // ── Clipboard / register ─────────────────────────────────────────
    YankToRegister { reg: char, text: String, kind: RegisterKind },
    PasteFromRegister { reg: char, before_cursor: bool },

    // ── Undo/Redo ────────────────────────────────────────────────────
    Undo,
    Redo,

    // ── Application-level commands ───────────────────────────────────
    ExCommand(ExCommand),
    LeaderAction(LeaderAction),

    // ── Search ───────────────────────────────────────────────────────
    SearchSubmit { pattern: String, direction: SearchDirection },
    ClearHighlight,

    // ── No-op ────────────────────────────────────────────────────────
    None,
}

#[derive(Debug, Clone, Copy)]
pub enum CursorMove {
    Left(usize),
    Right(usize),
    Up(usize),
    Down(usize),
    ToByteOffset(usize),
    LineStart,
    LineFirstNonWhite,
    LineEnd,
    FileStart,
    FileEnd,
}

#[derive(Debug, Clone, Copy)]
pub enum SearchDirection {
    Forward,
    Backward,
}
```

#### Ex Commands

```rust
// lazyjob-tui/src/vim/command.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExCommand {
    Write,                        // :w — save current entity
    Quit,                         // :q
    QuitForce,                    // :q!
    WriteQuit,                    // :wq
    Reload,                       // :e
    Undo,                         // :u
    Redo,                         // :red
    NoHighlight,                  // :noh
    SetOption { key: String, value: String },  // :set key=value
    Buffer(String),               // :buffer <name> — switch view
    RalphStart,                   // :ralph start
    RalphCancel,                  // :ralph cancel
    Unknown(String),              // pass-through for unrecognised commands
}

pub struct ExCommandParser;

impl ExCommandParser {
    /// Parse a command string (without the leading `:`).
    pub fn parse(input: &str) -> ExCommand {
        let trimmed = input.trim();
        match trimmed {
            "w"             => ExCommand::Write,
            "q"             => ExCommand::Quit,
            "q!"            => ExCommand::QuitForce,
            "wq" | "x"     => ExCommand::WriteQuit,
            "e"             => ExCommand::Reload,
            "u"             => ExCommand::Undo,
            "red" | "redo"  => ExCommand::Redo,
            "noh" | "nohl"  => ExCommand::NoHighlight,
            s if s.starts_with("buffer ") => {
                ExCommand::Buffer(s["buffer ".len()..].trim().to_string())
            }
            s if s.starts_with("ralph ") => match s["ralph ".len()..].trim() {
                "start"  => ExCommand::RalphStart,
                "cancel" => ExCommand::RalphCancel,
                _        => ExCommand::Unknown(trimmed.to_string()),
            },
            s if s.starts_with("set ") => {
                let rest = s["set ".len()..].trim();
                if let Some((k, v)) = rest.split_once('=') {
                    ExCommand::SetOption {
                        key: k.trim().to_string(),
                        value: v.trim().to_string(),
                    }
                } else {
                    ExCommand::SetOption { key: rest.to_string(), value: "true".to_string() }
                }
            }
            _ => ExCommand::Unknown(trimmed.to_string()),
        }
    }
}
```

#### Leader Actions

```rust
// lazyjob-tui/src/vim/action.rs (continued)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderAction {
    StartRalph,          // <leader>r
    CopyJobUrl,          // <leader>c
    ToggleGhostFilter,   // <leader>d
    JumpToJob,           // <leader>j
    Custom(String),      // user-configured leader binding
}
```

#### VimConfig

```rust
// lazyjob-tui/src/vim/config.rs

use std::collections::HashMap;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct VimConfig {
    pub enabled: bool,
    pub leader: char,
    pub wrap_motions: bool,
    pub hlsearch: bool,
    pub incsearch: bool,
    pub timeoutlen_ms: u64,
    /// User-defined leader bindings: {"<leader>r": "start_ralph", ...}
    pub leader_bindings: HashMap<char, String>,
}

impl Default for VimConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            leader: '\\',
            wrap_motions: false,
            hlsearch: true,
            incsearch: true,
            timeoutlen_ms: 1000,
            leader_bindings: HashMap::new(),
        }
    }
}
```

#### VimState (per-view state)

```rust
// lazyjob-tui/src/vim/engine.rs

use std::time::Instant;

pub struct VimState {
    pub mode: VimMode,
    pub registers: RegisterBank,
    pub macro_recorder: MacroRecorder,

    // Pending key sequence for multi-key commands (e.g., "gg", "dw", "3j")
    pub pending_count: Option<usize>,       // numeric prefix accumulator
    pub pending_operator: Option<Operator>, // operator waiting for motion
    pub pending_register: Option<char>,     // " prefix for register selection
    pub pending_g: bool,                    // "g" prefix pending (ge, gE, gg, gU, gu, g~)
    pub pending_leader: bool,               // <leader> prefix pending
    pub find_char_direction: Option<bool>,  // true=forward, for ; / ,

    // Visual mode anchor
    pub visual_anchor: Option<VisualAnchor>,

    // Search
    pub last_search: Option<String>,
    pub search_direction: SearchDirection,
    pub search_input_buffer: String,       // accumulates chars in Search mode

    // Command mode
    pub command_buffer: String,            // accumulates chars in Command mode

    // Last insert text for `gi`, `.` repeat
    pub last_insert_text: Option<String>,
    pub last_change: Option<LastChange>,

    // Timeout tracking for multi-key sequences
    pub last_key_time: Option<Instant>,

    pub config: VimConfig,
}

/// Stores enough info to replay `.` (repeat last change).
#[derive(Clone, Debug)]
pub struct LastChange {
    pub operator: Operator,
    pub count: usize,
    pub motion: CountedMotion,
}
```

#### MacroRecorder

```rust
// lazyjob-tui/src/vim/macro_.rs

use super::key::Key;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct MacroRecorder {
    /// Which register is being recorded into, if any.
    pub recording_register: Option<char>,
    /// Keys recorded so far.
    pub current_recording: Vec<Key>,
    /// Stored macros keyed by register char.
    pub macros: HashMap<char, Vec<Key>>,
}

impl MacroRecorder {
    pub fn start(&mut self, register: char) {
        self.recording_register = Some(register);
        self.current_recording.clear();
    }

    /// Record a key during active recording. Ignores 'q' (stop key).
    pub fn record_key(&mut self, key: Key) {
        if self.recording_register.is_some() {
            self.current_recording.push(key);
        }
    }

    /// Stop recording, save to register. Returns the saved key sequence.
    pub fn stop(&mut self) -> Option<(char, Vec<Key>)> {
        let reg = self.recording_register.take()?;
        let keys = std::mem::take(&mut self.current_recording);
        self.macros.insert(reg, keys.clone());
        Some((reg, keys))
    }

    pub fn get(&self, register: char) -> Option<&[Key]> {
        self.macros.get(&register).map(|v| v.as_slice())
    }

    pub fn is_recording(&self) -> bool {
        self.recording_register.is_some()
    }
}
```

### VimEngine — top-level dispatcher

```rust
// lazyjob-tui/src/vim/engine.rs

impl VimState {
    pub fn new(config: VimConfig) -> Self {
        Self {
            mode: VimMode::Normal,
            registers: RegisterBank::default(),
            macro_recorder: MacroRecorder::default(),
            pending_count: None,
            pending_operator: None,
            pending_register: None,
            pending_g: false,
            pending_leader: false,
            find_char_direction: None,
            visual_anchor: None,
            last_search: None,
            search_direction: SearchDirection::Forward,
            search_input_buffer: String::new(),
            command_buffer: String::new(),
            last_insert_text: None,
            last_change: None,
            last_key_time: None,
            config,
        }
    }

    /// Process one key event. Returns the action(s) the TUI must apply.
    /// `buffer` and `cursor` are the current text content and byte-offset cursor
    /// position, needed for motion resolution and text object expansion.
    pub fn process_key(
        &mut self,
        key: Key,
        buffer: &str,
        cursor: usize,
    ) -> Vec<VimAction> {
        // Record key if macro recording is active (before dispatch)
        self.macro_recorder.record_key(key);

        match self.mode {
            VimMode::Normal     => self.handle_normal(key, buffer, cursor),
            VimMode::Insert     => self.handle_insert(key),
            VimMode::Visual
            | VimMode::VisualLine => self.handle_visual(key, buffer, cursor),
            VimMode::Command    => self.handle_command(key),
            VimMode::Search     => self.handle_search(key),
        }
    }
}
```

The `handle_*` methods are detailed in the Implementation Phases below.

---

## SQLite Schema

Vim mode has no SQLite tables. All state is in-memory, per-view. The undo/redo stack for editable text fields is stored in `VimTextAreaState` (in-memory ring buffer, max 100 entries).

---

## Implementation Phases

### Phase 1 — Core Mode Machine + Insert Mode (MVP)

**Goal**: Replace the existing `InputMode::Normal/Insert` enum in `App` with `VimMode`. Normal mode allows navigation motions (h/j/k/l, w/b, 0/$, gg/G). Insert mode accepts text and `Esc` returns to Normal.

#### Step 1.1 — Add `unicode-segmentation` to Cargo.toml

File: `lazyjob-tui/Cargo.toml`

Add:
```toml
unicode-segmentation = "1.12"
```

**Verification**: `cargo build -p lazyjob-tui` succeeds.

#### Step 1.2 — Implement Key, VimMode, VimConfig, RegisterBank, MacroRecorder

Create all type files in `lazyjob-tui/src/vim/` as defined in the Core Types section.

Create `lazyjob-tui/src/vim/mod.rs`:
```rust
pub mod action;
pub mod command;
pub mod config;
pub mod engine;
pub mod integration;
pub mod key;
pub mod macro_;
pub mod mode;
pub mod motion;
pub mod operator;
pub mod register;
pub mod search;
pub mod text_object;
pub mod visual;

pub use action::VimAction;
pub use config::VimConfig;
pub use engine::VimState;
pub use key::Key;
pub use mode::VimMode;
```

**Verification**: `cargo build -p lazyjob-tui` with no unused import warnings.

#### Step 1.3 — Integration layer: crossterm → Key

File: `lazyjob-tui/src/vim/integration.rs`

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use super::key::Key;

pub fn crossterm_key_to_vim(event: KeyEvent) -> Key {
    let mods = event.modifiers;
    match event.code {
        KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => Key::Ctrl(c),
        KeyCode::Char(c) if mods.contains(KeyModifiers::ALT)     => Key::Alt(c),
        KeyCode::Char(c)     => Key::Char(c),
        KeyCode::Esc         => Key::Esc,
        KeyCode::Enter       => Key::Enter,
        KeyCode::Backspace   => Key::Backspace,
        KeyCode::Delete      => Key::Delete,
        KeyCode::Up          => Key::Up,
        KeyCode::Down        => Key::Down,
        KeyCode::Left        => Key::Left,
        KeyCode::Right       => Key::Right,
        KeyCode::Home        => Key::Home,
        KeyCode::End         => Key::End,
        KeyCode::PageUp      => Key::PageUp,
        KeyCode::PageDown    => Key::PageDown,
        KeyCode::Tab         => Key::Tab,
        KeyCode::BackTab     => Key::BackTab,
        KeyCode::F(n)        => Key::F(n),
        _                    => Key::Null,
    }
}
```

**Verification**: Unit test: `crossterm_key_to_vim(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))` → `Key::Char('d')`.

#### Step 1.4 — Normal mode: handle_normal (motions only, Phase 1 subset)

File: `lazyjob-tui/src/vim/engine.rs`

Implement `VimState::handle_normal` handling:
- Count prefix accumulation: `Key::Char('0'..='9')` when `!pending_g && !pending_operator` → accumulate `pending_count`.
- `h/j/k/l` → `VimAction::MoveCursor(Left/Right/Down/Up(count))`.
- `w/b/e/W/B/E` → word motions via `motion::resolve_word_forward/backward()`.
- `0/^/$` → `CursorMove::LineStart/LineFirstNonWhite/LineEnd`.
- `g` → set `pending_g = true`, timeout after `timeoutlen_ms`.
- `gg` (pending_g + `g`) → `CursorMove::FileStart`.
- `G` → `CursorMove::FileEnd`.

```rust
fn handle_normal(&mut self, key: Key, buffer: &str, cursor: usize) -> Vec<VimAction> {
    let count = self.take_count();

    // Leader key
    if self.pending_leader {
        self.pending_leader = false;
        return self.handle_leader(key);
    }

    // "g" prefix
    if self.pending_g {
        self.pending_g = false;
        return self.handle_g_prefix(key, count, buffer, cursor);
    }

    // Register prefix: "a-z, 0-9, +, *, ", -
    if self.pending_register.is_some() {
        return self.handle_register_prefix(key, count, buffer, cursor);
    }

    // Operator pending (waiting for motion/text-object)
    if let Some(op) = self.pending_operator {
        self.pending_operator = None;
        return self.handle_operator_motion(op, count, key, buffer, cursor);
    }

    match key {
        // Count accumulation
        Key::Char(c @ '1'..='9') => {
            self.pending_count = Some(
                self.pending_count.unwrap_or(0) * 10 + (c as usize - '0' as usize)
            );
            return vec![VimAction::None];
        }
        Key::Char('0') if self.pending_count.is_some() => {
            self.pending_count = Some(self.pending_count.unwrap() * 10);
            return vec![VimAction::None];
        }

        // Mode transitions
        Key::Char('i') => return vec![VimAction::EnterMode(VimMode::Insert)],
        Key::Char('I') => return vec![
            VimAction::MoveCursor(CursorMove::LineFirstNonWhite),
            VimAction::EnterMode(VimMode::Insert),
        ],
        Key::Char('a') => return vec![
            VimAction::MoveCursor(CursorMove::Right(1)),
            VimAction::EnterMode(VimMode::Insert),
        ],
        Key::Char('A') => return vec![
            VimAction::MoveCursor(CursorMove::LineEnd),
            VimAction::EnterMode(VimMode::Insert),
        ],
        Key::Char('o') => return vec![
            VimAction::ExCommand(ExCommand::InsertLineBelow),
            VimAction::EnterMode(VimMode::Insert),
        ],
        Key::Char('O') => return vec![
            VimAction::ExCommand(ExCommand::InsertLineAbove),
            VimAction::EnterMode(VimMode::Insert),
        ],
        Key::Char('v') => {
            self.visual_anchor = Some(VisualAnchor { byte_offset: cursor });
            return vec![VimAction::EnterMode(VimMode::Visual)];
        }
        Key::Char('V') => {
            self.visual_anchor = Some(VisualAnchor { byte_offset: cursor });
            return vec![VimAction::EnterMode(VimMode::VisualLine)];
        }
        Key::Char(':') => return vec![VimAction::EnterMode(VimMode::Command)],
        Key::Char('/') => {
            self.search_direction = SearchDirection::Forward;
            return vec![VimAction::EnterMode(VimMode::Search)];
        }
        Key::Char('?') => {
            self.search_direction = SearchDirection::Backward;
            return vec![VimAction::EnterMode(VimMode::Search)];
        }

        // Motions
        Key::Char('h') | Key::Left  => return vec![VimAction::MoveCursor(CursorMove::Left(count))],
        Key::Char('l') | Key::Right => return vec![VimAction::MoveCursor(CursorMove::Right(count))],
        Key::Char('k') | Key::Up    => return vec![VimAction::MoveCursor(CursorMove::Up(count))],
        Key::Char('j') | Key::Down  => return vec![VimAction::MoveCursor(CursorMove::Down(count))],
        Key::Char('w') => return vec![self.move_word_forward(buffer, cursor, count, false)],
        Key::Char('W') => return vec![self.move_word_forward(buffer, cursor, count, true)],
        Key::Char('b') => return vec![self.move_word_backward(buffer, cursor, count, false)],
        Key::Char('B') => return vec![self.move_word_backward(buffer, cursor, count, true)],
        Key::Char('e') => return vec![self.move_word_end(buffer, cursor, count, false)],
        Key::Char('E') => return vec![self.move_word_end(buffer, cursor, count, true)],
        Key::Char('0') => return vec![VimAction::MoveCursor(CursorMove::LineStart)],
        Key::Char('^') => return vec![VimAction::MoveCursor(CursorMove::LineFirstNonWhite)],
        Key::Char('$') => return vec![VimAction::MoveCursor(CursorMove::LineEnd)],
        Key::Char('G') => return vec![VimAction::MoveCursor(CursorMove::FileEnd)],
        Key::Char('g') => { self.pending_g = true; return vec![VimAction::None]; }

        // Operators
        Key::Char('d') => { self.pending_operator = Some(Operator::Delete); return vec![VimAction::None]; }
        Key::Char('y') => { self.pending_operator = Some(Operator::Yank); return vec![VimAction::None]; }
        Key::Char('c') => { self.pending_operator = Some(Operator::Change); return vec![VimAction::None]; }
        Key::Char('>') => { self.pending_operator = Some(Operator::Indent); return vec![VimAction::None]; }
        Key::Char('<') => { self.pending_operator = Some(Operator::Dedent); return vec![VimAction::None]; }

        // Single-key actions
        Key::Char('x') => return self.delete_char_under_cursor(buffer, cursor),
        Key::Char('r') => { /* next key replaces char */ return vec![VimAction::None]; }
        Key::Char('p') => return vec![VimAction::PasteFromRegister { reg: self.active_register(), before_cursor: false }],
        Key::Char('P') => return vec![VimAction::PasteFromRegister { reg: self.active_register(), before_cursor: true }],
        Key::Char('u') => return vec![VimAction::Undo],
        Key::Ctrl('r') => return vec![VimAction::Redo],
        Key::Char('"') => { self.pending_register = Some('"'); return vec![VimAction::None]; }

        // Leader
        c if c == Key::Char(self.config.leader) => {
            self.pending_leader = true;
            return vec![VimAction::None];
        }

        // Macro recording
        Key::Char('q') if !self.macro_recorder.is_recording() => {
            // Next key is the register; handled via pending state
            // (reuse pending_register slot)
            self.pending_register = Some('q'); // special sentinel
            return vec![VimAction::None];
        }
        Key::Char('q') if self.macro_recorder.is_recording() => {
            self.macro_recorder.stop();
            return vec![VimAction::None];
        }
        Key::Char('@') => {
            // Next key is macro register to play
            self.pending_register = Some('@'); // special sentinel
            return vec![VimAction::None];
        }

        Key::Char('n') => return vec![VimAction::MoveCursor(self.search_next(buffer, cursor, false))],
        Key::Char('N') => return vec![VimAction::MoveCursor(self.search_next(buffer, cursor, true))],
        Key::Char('*') => return self.search_word_under_cursor(buffer, cursor, SearchDirection::Forward),
        Key::Char('#') => return self.search_word_under_cursor(buffer, cursor, SearchDirection::Backward),

        _ => return vec![VimAction::None],
    }
}

fn take_count(&mut self) -> usize {
    self.pending_count.take().unwrap_or(1).max(1)
}

fn active_register(&mut self) -> char {
    self.pending_register.take().unwrap_or('"')
}
```

**Verification**: Unit tests:
- `process_key(Key::Char('j'), ...) → [MoveCursor(Down(1))]`
- `process_key(Key::Char('3'), ...) then process_key(Key::Char('j'), ...) → [MoveCursor(Down(3))]`
- `process_key(Key::Char('i'), ...) → [EnterMode(Insert)]`

#### Step 1.5 — handle_insert

```rust
fn handle_insert(&mut self, key: Key) -> Vec<VimAction> {
    match key {
        Key::Esc => {
            // Save last insert for gi / .
            self.mode = VimMode::Normal;
            vec![
                VimAction::MoveCursor(CursorMove::Left(1)), // cursor moves left on Esc
                VimAction::EnterMode(VimMode::Normal),
            ]
        }
        Key::Backspace => vec![VimAction::ExCommand(ExCommand::DeleteCharBefore)],
        Key::Delete    => vec![VimAction::ExCommand(ExCommand::DeleteCharAfter)],
        Key::Enter     => vec![VimAction::Insert("\n".to_string())],
        Key::Char(c)   => vec![VimAction::Insert(c.to_string())],
        Key::Ctrl('w') => vec![VimAction::ExCommand(ExCommand::DeleteWordBefore)],
        Key::Ctrl('u') => vec![VimAction::ExCommand(ExCommand::DeleteToLineStart)],
        _              => vec![VimAction::None],
    }
}
```

Extend `ExCommand` with the new variants used above:
```rust
// Additional ExCommand variants for insert mode:
DeleteCharBefore,
DeleteCharAfter,
DeleteWordBefore,   // Ctrl-w
DeleteToLineStart,  // Ctrl-u
InsertLineBelow,    // 'o'
InsertLineAbove,    // 'O'
```

**Verification**: Typing `i`, then `hello`, then `Esc` produces the Insert actions followed by `EnterMode(Normal)` and `MoveCursor(Left(1))`.

#### Step 1.6 — VimTextArea widget (Phase 1 subset)

File: `lazyjob-tui/src/widgets/vim_text_area.rs`

A single-line text area embedded in e.g. the job notes panel:

```rust
pub struct VimTextAreaState {
    pub vim: VimState,
    pub buffer: String,
    pub cursor: usize,             // byte offset
    pub undo_stack: VecDeque<(String, usize)>, // (buffer, cursor) snapshots
    pub redo_stack: VecDeque<(String, usize)>,
}

impl VimTextAreaState {
    pub fn handle_key(&mut self, key: Key) {
        let actions = self.vim.process_key(key, &self.buffer, self.cursor);
        for action in actions {
            self.apply_action(action);
        }
    }

    fn apply_action(&mut self, action: VimAction) {
        match action {
            VimAction::Insert(s) => {
                self.snapshot();
                self.buffer.insert_str(self.cursor, &s);
                self.cursor += s.len();
            }
            VimAction::DeleteRange(start, end) => {
                self.snapshot();
                let end = end.min(self.buffer.len().saturating_sub(1));
                self.buffer.drain(start..=end);
                self.cursor = start;
            }
            VimAction::MoveCursor(m) => self.apply_cursor_move(m),
            VimAction::Undo => self.undo(),
            VimAction::Redo => self.redo(),
            VimAction::EnterMode(m) => {
                self.vim.mode = m;
                // Update terminal cursor style
            }
            _ => {}
        }
    }

    fn snapshot(&mut self) {
        self.undo_stack.push_back((self.buffer.clone(), self.cursor));
        if self.undo_stack.len() > 100 {
            self.undo_stack.pop_front();
        }
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some((buf, cur)) = self.undo_stack.pop_back() {
            self.redo_stack.push_back((self.buffer.clone(), self.cursor));
            self.buffer = buf;
            self.cursor = cur;
        }
    }

    fn redo(&mut self) {
        if let Some((buf, cur)) = self.redo_stack.pop_back() {
            self.undo_stack.push_back((self.buffer.clone(), self.cursor));
            self.buffer = buf;
            self.cursor = cur;
        }
    }
}
```

`ratatui` rendering: render `buffer` as a `Paragraph`, overlay the cursor using a styled `Span` at the cursor grapheme position. Use `unicode-segmentation::UnicodeSegmentation::grapheme_indices` to map byte offset to column offset:

```rust
pub fn render(state: &VimTextAreaState, area: Rect, buf: &mut Buffer) {
    use unicode_segmentation::UnicodeSegmentation;
    let graphemes: Vec<&str> = state.buffer.graphemes(true).collect();
    // find cursor grapheme index from byte offset
    let cursor_grapheme = state.buffer
        .grapheme_indices(true)
        .position(|(i, _)| i == state.cursor)
        .unwrap_or(graphemes.len());

    let spans: Vec<Span> = graphemes.iter().enumerate().map(|(i, g)| {
        if i == cursor_grapheme && state.vim.mode != VimMode::Insert {
            Span::styled(*g, Style::default().bg(Color::White).fg(Color::Black))
        } else {
            Span::raw(*g)
        }
    }).collect();

    let paragraph = Paragraph::new(Line::from(spans));
    paragraph.render(area, buf);
}
```

**Verification**: Manual TUI test: open a text field, press `i`, type "hello", press `Esc`, press `dw` — word is deleted.

---

### Phase 2 — Operator + Motion Composition, Visual Mode, Text Objects

**Goal**: Full `d{motion}`, `y{motion}`, `c{motion}`, visual mode selection and operators, and all text objects (iw/aw, i"/a", ip/ap, etc.).

#### Step 2.1 — Motion resolution engine

File: `lazyjob-tui/src/vim/motion.rs`

```rust
/// Resolve a Motion to a (start, end) byte-range in `buffer` from `cursor`.
/// Returns `None` if the motion is not applicable (e.g., `w` at end of file).
pub fn resolve_motion(
    motion: &Motion,
    count: usize,
    buffer: &str,
    cursor: usize,
) -> Option<(usize, usize)> {
    // Delegate to per-motion helpers using unicode-segmentation graphemes.
    // All helpers work on grapheme boundaries, not raw bytes.
    match motion {
        Motion::WordForward      => word_forward(buffer, cursor, count, false),
        Motion::WordForwardWide  => word_forward(buffer, cursor, count, true),
        Motion::WordBackward     => word_backward(buffer, cursor, count, false),
        Motion::WordBackwardWide => word_backward(buffer, cursor, count, true),
        Motion::WordEnd          => word_end(buffer, cursor, count, false),
        Motion::LineStart        => Some((cursor, line_start_offset(buffer, cursor))),
        Motion::LineEnd          => Some((cursor, line_end_offset(buffer, cursor))),
        Motion::FileStart        => Some((cursor, 0)),
        Motion::FileEnd          => Some((cursor, buffer.len())),
        Motion::FindCharForward(c)  => find_char(buffer, cursor, *c, count, true),
        Motion::FindCharBackward(c) => find_char(buffer, cursor, *c, count, false),
        Motion::TillCharForward(c)  => till_char(buffer, cursor, *c, count, true),
        Motion::TillCharBackward(c) => till_char(buffer, cursor, *c, count, false),
        Motion::MatchBracket        => match_bracket(buffer, cursor),
        Motion::TextObject(to)      => resolve_text_object(to, buffer, cursor),
        Motion::ParagraphForward    => paragraph_forward(buffer, cursor, count),
        Motion::ParagraphBackward   => paragraph_backward(buffer, cursor, count),
        _ => None,
    }
}
```

Key implementation notes:
- Use `unicode-segmentation::UnicodeSegmentation::grapheme_indices(true)` throughout to handle multi-byte UTF-8 correctly.
- `word_forward` scans grapheme-by-grapheme, treating `[a-zA-Z0-9_]` as word chars for `w` and any non-whitespace for `W`.
- `match_bracket` searches forward/backward for the matching `({[<>]})` pair, handling nesting depth.

#### Step 2.2 — Text object resolution

File: `lazyjob-tui/src/vim/text_object.rs`

```rust
pub fn resolve_text_object(obj: &TextObject, buffer: &str, cursor: usize) -> Option<(usize, usize)> {
    match obj {
        TextObject::Word(b)        => text_object_word(buffer, cursor, *b, false),
        TextObject::WideWord(b)    => text_object_word(buffer, cursor, *b, true),
        TextObject::DoubleQuote(b) => text_object_quoted(buffer, cursor, '"', *b),
        TextObject::SingleQuote(b) => text_object_quoted(buffer, cursor, '\'', *b),
        TextObject::Backtick(b)    => text_object_quoted(buffer, cursor, '`', *b),
        TextObject::Paren(b)       => text_object_paired(buffer, cursor, '(', ')', *b),
        TextObject::Bracket(b)     => text_object_paired(buffer, cursor, '[', ']', *b),
        TextObject::Brace(b)       => text_object_paired(buffer, cursor, '{', '}', *b),
        TextObject::AngleBracket(b)=> text_object_paired(buffer, cursor, '<', '>', *b),
        TextObject::Paragraph(b)   => text_object_paragraph(buffer, cursor, *b),
        TextObject::Sentence(b)    => text_object_sentence(buffer, cursor, *b),
    }
}
```

`text_object_quoted`: scan left from cursor for opening delimiter, scan right for closing delimiter. `Inner` excludes delimiters; `Around` includes them and trailing whitespace.

`text_object_paired`: same but handles nesting depth.

**Parsing operator+motion text objects** in `handle_operator_motion`:
After an operator key (e.g., `d`), the next key is either:
- A motion key (`w`, `b`, `e`, `j`, `k`, `$`, etc.) → `resolve_motion`
- `i` or `a` → next key is the text object delimiter → `resolve_text_object`
- The same operator letter (e.g., `dd`, `yy`, `cc`) → operate on whole line

```rust
fn handle_operator_motion(
    &mut self,
    op: Operator,
    count: usize,
    key: Key,
    buffer: &str,
    cursor: usize,
) -> Vec<VimAction> {
    // Double-operator: dd, yy, cc → operate on line(s)
    if key == Key::Char(op.char()) {
        let (start, end) = select_lines(buffer, cursor, count);
        return self.apply_operator(op, start, end, RegisterKind::Linewise, buffer);
    }

    // Text object: i or a
    if matches!(key, Key::Char('i') | Key::Char('a')) {
        // Stash boundary, wait for next key
        self.pending_operator = Some(op);
        self.pending_text_object_boundary = Some(if key == Key::Char('i') { Boundary::Inner } else { Boundary::Around });
        return vec![VimAction::None];
    }

    // Regular motion
    let motion = key_to_motion(key, self);
    let Some(motion) = motion else { return vec![VimAction::None]; };
    let Some((start, end)) = resolve_motion(&motion, count, buffer, cursor) else {
        return vec![VimAction::None];
    };
    self.apply_operator(op, start, end, RegisterKind::Characterwise, buffer)
}

fn apply_operator(&mut self, op: Operator, start: usize, end: usize, kind: RegisterKind, buffer: &str) -> Vec<VimAction> {
    let (lo, hi) = (start.min(end), start.max(end));
    match op {
        Operator::Delete => {
            let text = buffer[lo..=hi.min(buffer.len()-1)].to_string();
            self.registers.yank('"', text.clone(), kind);
            vec![VimAction::DeleteRange(lo, hi)]
        }
        Operator::Yank => {
            let text = buffer[lo..=hi.min(buffer.len()-1)].to_string();
            self.registers.yank('"', text, kind);
            vec![VimAction::None]
        }
        Operator::Change => {
            let text = buffer[lo..=hi.min(buffer.len()-1)].to_string();
            self.registers.yank('"', text, kind);
            vec![
                VimAction::DeleteRange(lo, hi),
                VimAction::EnterMode(VimMode::Insert),
            ]
        }
        _ => vec![VimAction::None],
    }
}
```

#### Step 2.3 — Visual mode key handler

```rust
fn handle_visual(&mut self, key: Key, buffer: &str, cursor: usize) -> Vec<VimAction> {
    match key {
        Key::Esc => {
            self.visual_anchor = None;
            return vec![VimAction::EnterMode(VimMode::Normal)];
        }
        Key::Char('o') => {
            // Swap cursor and anchor
            if let Some(ref mut anchor) = self.visual_anchor {
                let old = anchor.byte_offset;
                anchor.byte_offset = cursor;
                return vec![VimAction::MoveCursor(CursorMove::ToByteOffset(old))];
            }
            return vec![VimAction::None];
        }
        Key::Char('d') | Key::Char('x') => {
            let sel = self.compute_selection(cursor, buffer);
            self.visual_anchor = None;
            let kind = if self.mode == VimMode::VisualLine { RegisterKind::Linewise } else { RegisterKind::Characterwise };
            let text = buffer[sel.start..=sel.end.min(buffer.len()-1)].to_string();
            self.registers.yank('"', text, kind);
            return vec![
                VimAction::DeleteRange(sel.start, sel.end),
                VimAction::EnterMode(VimMode::Normal),
            ];
        }
        Key::Char('y') => {
            let sel = self.compute_selection(cursor, buffer);
            self.visual_anchor = None;
            let kind = if self.mode == VimMode::VisualLine { RegisterKind::Linewise } else { RegisterKind::Characterwise };
            let text = buffer[sel.start..=sel.end.min(buffer.len()-1)].to_string();
            self.registers.yank('"', text, kind);
            return vec![VimAction::EnterMode(VimMode::Normal)];
        }
        Key::Char('c') => {
            let sel = self.compute_selection(cursor, buffer);
            self.visual_anchor = None;
            return vec![
                VimAction::DeleteRange(sel.start, sel.end),
                VimAction::EnterMode(VimMode::Insert),
            ];
        }
        // Motion keys extend the selection
        motion_key => {
            return self.handle_normal(motion_key, buffer, cursor);
        }
    }
}

fn compute_selection(&self, cursor: usize, buffer: &str) -> Selection {
    let anchor = self.visual_anchor.as_ref().map(|a| a.byte_offset).unwrap_or(cursor);
    let (start, end) = if anchor <= cursor { (anchor, cursor) } else { (cursor, anchor) };
    if self.mode == VimMode::VisualLine {
        let line_start = buffer[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line_end = buffer[end..].find('\n').map(|p| end + p).unwrap_or(buffer.len().saturating_sub(1));
        Selection { start: line_start, end: line_end, kind: SelectionKind::Linewise }
    } else {
        Selection { start, end, kind: SelectionKind::Characterwise }
    }
}
```

**ratatui rendering of visual selection**: In the widget render function, if `vim.mode == VimMode::Visual` and `visual_anchor.is_some()`, compute the selection range and apply `Style::default().bg(Color::Blue)` to the selected graphemes.

**Verification**:
- `V` → select line, `d` → line deleted from buffer.
- `viw` → select word, `y` → unnamed register populated.

---

### Phase 3 — Command Mode, Search, Leader Key

**Goal**: `:` ex commands, `/` and `?` search with incremental highlighting, and leader-key sequences.

#### Step 3.1 — handle_command

```rust
fn handle_command(&mut self, key: Key) -> Vec<VimAction> {
    match key {
        Key::Esc => {
            self.command_buffer.clear();
            vec![VimAction::EnterMode(VimMode::Normal)]
        }
        Key::Enter => {
            let cmd_text = std::mem::take(&mut self.command_buffer);
            let cmd = ExCommandParser::parse(&cmd_text);
            vec![
                VimAction::EnterMode(VimMode::Normal),
                VimAction::ExCommand(cmd),
            ]
        }
        Key::Backspace => {
            if self.command_buffer.is_empty() {
                // Cancel command mode if buffer empty
                vec![VimAction::EnterMode(VimMode::Normal)]
            } else {
                self.command_buffer.pop();
                vec![VimAction::None]
            }
        }
        Key::Char(c) => {
            self.command_buffer.push(c);
            vec![VimAction::None]
        }
        _ => vec![VimAction::None],
    }
}
```

The `CommandLineWidget` in `lazyjob-tui/src/widgets/command_line.rs` renders the command buffer as a `Paragraph` at the bottom of the screen (replacing the status bar) with a `:` prefix.

#### Step 3.2 — handle_search

```rust
fn handle_search(&mut self, key: Key) -> Vec<VimAction> {
    match key {
        Key::Esc => {
            self.search_input_buffer.clear();
            vec![VimAction::EnterMode(VimMode::Normal)]
        }
        Key::Enter => {
            let pattern = std::mem::take(&mut self.search_input_buffer);
            self.last_search = Some(pattern.clone());
            vec![
                VimAction::EnterMode(VimMode::Normal),
                VimAction::SearchSubmit { pattern, direction: self.search_direction },
            ]
        }
        Key::Backspace => {
            self.search_input_buffer.pop();
            // For incsearch: emit SearchSubmit with current buffer content
            if self.config.incsearch {
                let pattern = self.search_input_buffer.clone();
                return vec![VimAction::SearchSubmit { pattern, direction: self.search_direction }];
            }
            vec![VimAction::None]
        }
        Key::Char(c) => {
            self.search_input_buffer.push(c);
            if self.config.incsearch {
                let pattern = self.search_input_buffer.clone();
                return vec![VimAction::SearchSubmit { pattern, direction: self.search_direction }];
            }
            vec![VimAction::None]
        }
        _ => vec![VimAction::None],
    }
}
```

The `SearchBarWidget` renders at the bottom of the screen with `/` or `?` prefix. The widget state tracks match positions for `hlsearch` rendering — all graphemes matching the pattern are highlighted in the `Paragraph`.

Search navigation (`n`/`N`): `search_next()` method finds the next/prev occurrence of `last_search` in `buffer` relative to `cursor`, wrapping if `wrap_motions = true`.

#### Step 3.3 — Leader key handler

```rust
fn handle_leader(&mut self, key: Key) -> Vec<VimAction> {
    match key {
        Key::Char('r') => vec![VimAction::LeaderAction(LeaderAction::StartRalph)],
        Key::Char('c') => vec![VimAction::LeaderAction(LeaderAction::CopyJobUrl)],
        Key::Char('d') => vec![VimAction::LeaderAction(LeaderAction::ToggleGhostFilter)],
        Key::Char('j') => vec![VimAction::LeaderAction(LeaderAction::JumpToJob)],
        Key::Char(c) => {
            // Check user-configured leader bindings
            if let Some(action_name) = self.config.leader_bindings.get(&c) {
                vec![VimAction::LeaderAction(LeaderAction::Custom(action_name.clone()))]
            } else {
                vec![VimAction::None]
            }
        }
        _ => vec![VimAction::None],
    }
}
```

The TUI event loop (`App::handle_action`) pattern-matches on `LeaderAction` and dispatches to the appropriate service call or view switch.

**Verification**:
- Type `:wq`, press Enter → `ExCommand::WriteQuit` emitted.
- Type `/hello`, press Enter → `SearchSubmit { pattern: "hello", direction: Forward }` emitted.
- Type `\r` → `LeaderAction::StartRalph` emitted.

---

### Phase 4 — Registers, Macros, `.` Repeat, Configuration

**Goal**: Named registers (`"a`–`"z`, `"+`, `"*`), macro recording (`qa`…`q`, `@a`), `.` repeat, and TOML-configurable vim settings.

#### Step 4.1 — Register selection in normal mode

Register selection (`"a`, `"+`, etc.) flows through `pending_register`:
- `Key::Char('"')` → set `pending_register = Some('"')`  (sentinel: next char is register name)
- Next `Key::Char(c)` when `pending_register == Some('"')` → set `pending_register = Some(c)`, clear sentinel
- Operator then uses `self.active_register()` to pick the target

Append-to-register: uppercase register name (`"Ayw` appends to `a`):

```rust
impl RegisterBank {
    pub fn yank_or_append(&mut self, reg: char, text: String, kind: RegisterKind) {
        if reg.is_uppercase() {
            let lower = reg.to_ascii_lowercase();
            if let Some(existing) = self.named.get_mut(&lower) {
                existing.text.push_str(&text);
                return;
            }
        }
        self.yank(reg.to_ascii_lowercase(), text, kind);
    }
}
```

#### Step 4.2 — Macro recording

In `handle_normal`, when `pending_register == Some('q')`:
- Next key `c` → `macro_recorder.start(c)`.
- While `macro_recorder.is_recording()`, all subsequent keys are passed to `record_key` before dispatch.
- Next `q` → `macro_recorder.stop()`.

Macro playback (`@a`):
- The engine emits `VimAction::ExCommand(ExCommand::PlayMacro(register))`.
- The TUI event loop retrieves the key sequence from `VimState.macro_recorder.get(register)` and re-feeds the keys through `VimState.process_key` in a loop.
- Guard: max replay depth = 10 to prevent infinite loops.

```rust
ExCommand::PlayMacro(register) => {
    if let Some(keys) = app.vim_state.macro_recorder.get(register).map(|s| s.to_vec()) {
        for key in keys {
            let actions = app.vim_state.process_key(key, &app.buffer, app.cursor);
            app.apply_vim_actions(actions);
        }
    }
}
```

#### Step 4.3 — `.` repeat last change

`LastChange` is recorded whenever an operator+motion or insert sequence completes:

```rust
// After apply_operator:
self.last_change = Some(LastChange { operator: op, count, motion: counted_motion });

// In handle_normal, Key::Char('.'):
if let Some(ref change) = self.last_change.clone() {
    return self.handle_operator_motion(change.operator, change.count, key_for_motion(&change.motion), buffer, cursor);
}
```

#### Step 4.4 — Configuration loading

In `Config::load()` (from the architecture spec), the `[vim]` section is deserialized into `VimConfig` using `serde::Deserialize`. The `VimConfig::default()` is the fallback. `VimState::new(config.vim.clone())` is called once per editable widget.

```toml
# ~/.config/lazyjob/config.toml
[vim]
enabled = true
leader = "\\"
wrap_motions = false
hlsearch = true
incsearch = true
timeoutlen_ms = 1000

[vim.leader_bindings]
"p" = "toggle_privacy_mode"
"s" = "search_jobs"
```

**Verification**:
- `"ayw` yanks word to register `a`; `"ap` pastes it.
- `qa dw q @a` records "delete word" and replays it.
- `.` after `dw` deletes the next word.

---

## Key Crate APIs

| API | Purpose |
|-----|---------|
| `unicode_segmentation::UnicodeSegmentation::grapheme_indices(s, true)` | Iterate grapheme clusters with byte offsets for motion resolution |
| `unicode_segmentation::UnicodeSegmentation::graphemes(s, true)` | Collect grapheme slice vec for rendering |
| `unicode_width::UnicodeWidthStr::width(s)` | Compute column width (CJK double-width chars) |
| `crossterm::cursor::SetCursorStyle::SteadyBlock` / `BlinkingBar` | Switch cursor shape on mode change |
| `crossterm::execute!(stdout, SetCursorStyle::BlinkingBar)` | Actually update terminal cursor |
| `ratatui::widgets::Paragraph::new(text).render(area, buf)` | Render text with styled spans |
| `ratatui::style::Style::default().bg(Color::Blue)` | Visual selection highlight |
| `ratatui::text::{Line, Span}` | Inline styled text for character-level rendering |

---

## Error Handling

```rust
// lazyjob-tui/src/vim/error.rs

#[derive(thiserror::Error, Debug)]
pub enum VimError {
    #[error("motion {0:?} not applicable at cursor position {1}")]
    MotionOutOfRange(Motion, usize),

    #[error("unmatched bracket at position {0}")]
    UnmatchedBracket(usize),

    #[error("register '{0}' is empty")]
    EmptyRegister(char),

    #[error("macro replay depth exceeded limit")]
    MacroDepthExceeded,
}
```

All `VimError` variants are non-fatal: the engine returns `vec![VimAction::None]` on error and optionally emits a TUI status-bar message. The TUI never panics due to vim input.

---

## Testing Strategy

### Unit tests (pure engine, no TUI)

File: `lazyjob-tui/src/vim/engine.rs` (inline `#[cfg(test)]` module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::key::Key;
    use crate::vim::config::VimConfig;

    fn engine() -> VimState {
        VimState::new(VimConfig::default())
    }

    fn keys(state: &mut VimState, keys: &[Key], buffer: &str, cursor: usize) -> Vec<VimAction> {
        let mut all = vec![];
        let mut cur = cursor;
        for &key in keys {
            all.extend(state.process_key(key, buffer, cur));
        }
        all
    }

    #[test]
    fn test_insert_mode_entry_and_exit() {
        let mut s = engine();
        let actions = keys(&mut s, &[Key::Char('i')], "hello", 0);
        assert!(actions.iter().any(|a| matches!(a, VimAction::EnterMode(VimMode::Insert))));
        assert_eq!(s.mode, VimMode::Insert);
        let actions2 = keys(&mut s, &[Key::Esc], "hello", 0);
        assert!(actions2.iter().any(|a| matches!(a, VimAction::EnterMode(VimMode::Normal))));
    }

    #[test]
    fn test_count_prefix_motion() {
        let mut s = engine();
        let actions = keys(&mut s, &[Key::Char('3'), Key::Char('j')], "a\nb\nc\nd", 0);
        assert!(actions.iter().any(|a| matches!(a, VimAction::MoveCursor(CursorMove::Down(3)))));
    }

    #[test]
    fn test_dd_deletes_line() {
        let mut s = engine();
        let buf = "hello\nworld\n";
        let actions = keys(&mut s, &[Key::Char('d'), Key::Char('d')], buf, 0);
        assert!(actions.iter().any(|a| matches!(a, VimAction::DeleteRange(0, _))));
    }

    #[test]
    fn test_visual_yank() {
        let mut s = engine();
        let buf = "hello world";
        keys(&mut s, &[Key::Char('v')], buf, 0);
        keys(&mut s, &[Key::Char('4'), Key::Char('l')], buf, 0); // extend selection
        let actions = keys(&mut s, &[Key::Char('y')], buf, 4);
        assert!(actions.iter().any(|a| matches!(a, VimAction::EnterMode(VimMode::Normal))));
        assert!(s.registers.get('"').is_some());
    }

    #[test]
    fn test_ex_command_parse_write_quit() {
        let cmd = ExCommandParser::parse("wq");
        assert_eq!(cmd, ExCommand::WriteQuit);
    }

    #[test]
    fn test_leader_r_start_ralph() {
        let mut s = engine();
        let buf = "";
        keys(&mut s, &[Key::Char('\\')], buf, 0);
        let actions = keys(&mut s, &[Key::Char('r')], buf, 0);
        assert!(actions.iter().any(|a| matches!(a, VimAction::LeaderAction(LeaderAction::StartRalph))));
    }

    #[test]
    fn test_register_yank_and_paste() {
        let mut s = engine();
        let buf = "hello world";
        // "ayw — yank word to register 'a'
        keys(&mut s, &[Key::Char('"'), Key::Char('a'), Key::Char('y'), Key::Char('w')], buf, 0);
        assert!(s.registers.get('a').is_some());
        assert_eq!(s.registers.get('a').unwrap().text, "hello");
    }
}
```

### Integration tests (widget-level)

File: `lazyjob-tui/tests/vim_text_area.rs`

Drive `VimTextAreaState` through a sequence of keystrokes and assert buffer contents:

```rust
#[test]
fn test_type_and_delete_word() {
    let mut state = VimTextAreaState::new(VimConfig::default());
    // Type "hello world"
    for c in "hello world".chars() {
        state.handle_key(Key::Char(c));
    }
    // Wait — we're in Insert mode from start? No: vim_text_area starts in Normal.
    // Press 'i' first:
    // Actually: start in Normal, press 'i', type, press Esc.
    // Reset and redo:
    let mut state = VimTextAreaState::new(VimConfig::default());
    state.handle_key(Key::Char('i'));
    for c in "hello world".chars() {
        state.handle_key(Key::Char(c));
    }
    state.handle_key(Key::Esc);
    assert_eq!(state.buffer, "hello world");
    // Move to start, delete word
    state.handle_key(Key::Char('g')); state.handle_key(Key::Char('g'));
    state.handle_key(Key::Char('d')); state.handle_key(Key::Char('w'));
    assert_eq!(state.buffer, "world");
}
```

### TUI smoke test

After the full Phase 1 integration: manually run `cargo run --bin lazyjob`, navigate to a text field (e.g., job notes), press `i`, type a word, press `Esc`, confirm the cursor turns from bar to block, press `dd`, confirm the line is gone.

---

## Open Questions

1. **Multi-line `VimTextArea`**: This plan covers a `VimTextAreaState` generic enough for multi-line buffers (line count, line-wise motions), but the `apply_cursor_move(Down/Up)` implementation requires knowing line lengths. The implementation must track a logical `(row, col)` cursor alongside the byte offset, or maintain a line index cache. This should be resolved before Phase 2 ships the multi-line note editor.

2. **Clipboard system register (`"+`)**: The `arboard` crate works on macOS/Linux/Windows but requires a display server on Linux (headless CI will fail). Use `Option<arboard::Clipboard>` with graceful degradation (log warning on `Clipboard::new()` failure). This is shared with the `XX-tui-clipboard-integration.md` spec — coordinate to avoid duplicate `arboard` initialization.

3. **Macro semantics: text vs. key sequences**: The spec asks whether macros store text or key sequences. This plan stores key sequences (`Vec<Key>`), which is faithful to vim behaviour (playback re-interprets context). Storing text would be simpler but breaks mode-dependent key semantics. Recommendation: store key sequences.

4. **Marks (`m` + char)**: Not in Phase 1–4 scope. Can be added in Phase 5 by extending `VimState` with a `HashMap<char, usize>` of named byte-offset marks. Backtick and `'` navigation would use `resolve_motion(Mark(c))`.

5. **Timeout for multi-key sequences**: `pending_g`, `pending_leader`, and two-key motions (`f<c>`, `t<c>`) must be cancelled after `timeoutlen_ms` elapses without a follow-up key. The engine currently uses `last_key_time: Option<Instant>` but the TUI event loop must check the timeout on each tick and emit a synthetic `Key::Null` to flush the pending state. This should be wired in `EventLoop::run()`.

6. **`r` (replace character)**: Requires a special `pending_replace: bool` state in `handle_normal` to capture the next character as the replacement. Not complex but omitted from the Phase 1 code block above for brevity.

7. **`~` (toggle case), `gU`/`gu` (upper/lower case)**: These are operators in Phase 2's operator system but require grapheme-level case mapping. Use `char::to_uppercase()` / `char::to_lowercase()` applied per-grapheme, collecting into a new `String`.

## Related Specs

- `specs/09-tui-design-keybindings.md` / `specs/09-tui-design-keybindings-implementation-plan.md` — The existing `InputMode` enum is superseded by `VimMode`. The `App::handle_key` dispatch in spec 09 is updated to call `VimState::process_key` and apply `VimAction`s.
- `specs/XX-tui-clipboard-integration.md` — The `"+` and `"*` registers delegate to the `arboard` clipboard; coordinate initialization.
- `specs/XX-tui-accessibility.md` — Vim mode must not break keyboard-only navigation. Non-vim mode (for accessibility) should fall back to `InputMode::Insert` always.
