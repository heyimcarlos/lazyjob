# Spec: TUI Vim Mode Deep Implementation

## Context

LazyJob targets developers who expect real vim behavior, not just vim-like keybindings. This spec defines a complete vim mode system for the TUI.

## Motivation

- **User expectation**: Developers familiar with vim expect full vim semantics
- **Efficiency**: vim mode enables rapid text navigation and manipulation
- **Differentiation**: Real vim mode is rare in TUIs, a competitive advantage

## Design

### Mode System

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,   // Navigate and execute commands
    Insert,   // Insert text
    Visual,   // Select text
    VisualLine,  // Select lines
    Command,  // Execute :commands
    Search,   // / or ? search
}

pub struct VimState {
    mode: VimMode,
    register: Register,
    macro_recording: Option<char>,  // 'q' followed by register name
    last_search_pattern: Option<String>,
    last_yank_id: Option<YankId>,
    motion_repeat: u8,  // e.g., "3w" = repeat motion 3x
}

pub struct Register {
    a: Option<String>,
    b: Option<String>,  // ... z, plus 0-9
}
```

### Mode Indicators

```rust
impl VimMode {
    fn status_indicator(&self) -> &'static str {
        match self {
            VimMode::Normal => "",
            VimMode::Insert => "-- INSERT --",
            VimMode::Visual => "-- VISUAL --",
            VimMode::VisualLine => "-- VISUAL LINE --",
            VimMode::Command => "",
            VimMode::Search => "",
        }
    }
}
```

In TUI status bar:
```
[NORMAL]  Job: Senior Software Engineer at Stripe  │  [Jobs]  │  $3.42 budget
```

### Insert Mode Triggers

| Key | Action |
|-----|--------|
| `i` | Enter insert mode before cursor |
| `a` | Enter insert mode after cursor |
| `A` | Enter insert mode at end of line |
| `o` | Open new line below, enter insert |
| `O` | Open new line above, enter insert |
| `s` | Substitute character (delete char, insert) |
| `S` | Substitute line (delete line, insert) |
| `c` + motion | Change (delete, enter insert) |
| `cc` | Change entire line |
| `I` | Go to first non-whitespace, insert |
| `gi` | Go to last insert point, insert |

### Normal Mode Commands

**Motion**:
```
h/j/k/l       Left/down/up/right
w/W           Word forward (punctuation as word/boundary)
b/B           Word backward
e/E           End of word
ge/gE         End of word backward
0             Beginning of line
^             First non-whitespace
$             End of line
gg            First line
G             Last line
{ / }         Paragraph up/down
( / )         Sentence up/down
f/F + char    Find forward/backward to char
t/T + char    Till (before) char
/ + pattern   Search forward
? + pattern   Search backward
n/N           Repeat search forward/backward
* / #         Search word under cursor forward/backward
%             Match bracket
```

**Actions**:
```
d + motion    Delete (cut)
dd            Delete line
dw            Delete word
d$            Delete to end of line
y + motion   Yank (copy)
yy            Yank line
yw            Yank word
p/P           Paste after/before cursor
x             Delete character
r             Replace character
~             Toggle case
u             Undo
Ctrl+R        Redo
.             Repeat last change
c + motion    Change (delete, insert mode)
J             Join lines
<< / >>       Dedent/indent
==            Auto-indent
```

**Text Objects** (in visual or after operator):
```
iw / aw       Inner/around word
i" / a"       Inner/around double quotes
i' / a'       Inner/around single quotes
i( / a(       Inner/around parens
i[ / a[       Inner/around brackets
i{ / a{       Inner/around braces
ip / ap       Inner/around paragraph
```

### Operator + Motion Syntax

vim's power comes from operators combined with motions:

```rust
pub enum Operator {
    Delete,
    Yank,
    Paste,
    Change,
    Indent,
    // ...
}

pub fn execute_operator(state: &mut VimState, op: Operator, count: u8, motion: Motion) {
    for _ in 0..count {
        match op {
            Operator::Delete => apply_delete(state, motion),
            Operator::Yank => apply_yank(state, motion),
            // ...
        }
    }
    // Return to normal mode after operator
    state.mode = VimMode::Normal;
}
```

### Visual Mode

```
v             Enter visual (character)
V             Enter visual line
Esc           Return to normal
d             Delete selection
y             Yank selection
c             Change selection
p             Paste over selection
o             Swap cursor to other end
O             Swap cursor to other end (line)
```

### Command Mode (:commands)

```
:w            Write (save)
:q            Quit
:q!           Force quit
:wq           Write and quit
:e            Edit file (reload)
:u            Undo
:red          Redo
:noh          Clear search highlight
:set option   Set option
/             Enter search mode
?             Enter search mode (backward)
```

**Extended commands**:
```
:buffer jobs       Switch to jobs buffer
:buffer apps       Switch to applications buffer
: Ralph start      Start ralph loop
: Ralph cancel     Cancel ralph loop
```

### Leader Key

`<Leader>` defaults to `\` (backslash):

```
<Leader>r   Start ralph
<Leader>c   Copy job URL
<Leader>d   Toggle ghost job filter
<Leader>j   Jump to job
```

### Macros

```rust
pub fn start_macro(state: &mut VimState, register: char) {
    state.macro_recording = Some(register);
    // Begin recording keystrokes
}

pub fn stop_macro(state: &mut VimState) {
    // Save recording to register
    state.macro_recording = None;
}

pub fn play_macro(state: &mut VimState, register: char) {
    // Replay keystrokes from register
}
```

### Registers

```
"ayw   Yank word to register a
"ap    Paste from register a
"bdd   Delete line to register b
"bd    Delete to register b
:reg a Display register a contents
```

### Configuration

```toml
[vim]
enabled = true
_leader = "\\"
wrap_motions = false
hlsearch = true
incsearch = true
timeoutlen = 1000  # ms to wait for motion keys
```

## Implementation Notes

- Vim state stored per-view (not global)
- Key sequences captured and interpreted
- No terminal vim emulation (different approach)
- Text input widgets bypass vim mode (enter insert directly)

## Open Questions

1. **Macros stored**: Text only or key sequences?
2. **Marks (m + char)**: Support marks for quick navigation?
3. **Text objects in TUI**: What are "inner paragraph" equivalents?

## Related Specs

- `09-tui-design-keybindings.md` - Existing keybinding design
- `XX-tui-clipboard-integration.md` - System clipboard integration