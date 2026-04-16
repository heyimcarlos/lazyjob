# Gap Analysis: Salary / TUI (salary-*, 09-tui-design-keybindings specs)

## Specs Reviewed
- `salary-market-intelligence.md` - Salary intelligence service, total comp calculation
- `salary-negotiation-offers.md` - Research on negotiation and equity valuation
- `salary-counter-offer-drafting.md` - Counter-offer email drafting service
- `09-tui-design-keybindings.md` - TUI design, layout, keybindings, component patterns

---

## What's Well-Covered

### salary-market-intelligence.md
- SalaryIntelligenceService with full OfferEvaluation structure
- H1B LCA data as primary market source (Phase 1, no scraping)
- Levels.fyi clipboard paste import (Phase 2, user action required)
- Total comp calculation: base + bonus + equity_annual + signing_amortized
- Equity risk factors by company stage (Public=1.0, LatePrivate=0.7, MidPrivate=0.4, EarlyPrivate=0.15)
- RSU vs Options distinction with 409A FMV handling
- Pay transparency jurisdictions (CA, CO, NY, WA, IL, NJ, MA, MD, RI, HI, NV, DC)
- Competing offer comparison
- Privacy-first: offer_details excluded from SaaS sync

### salary-negotiation-offers.md
- Comprehensive research on negotiation gap (40-50% negotiate, 5-15% increase)
- Total comp blind spot (20-40% of comp is equity/bonus)
- Level 1-4 agentic opportunity framework
- Detailed equity valuation complexity (RSU, options, liquidation preferences)
- Levels.fyi, Glassdoor, Blind, Payscale analysis
- Research on gender dynamics in negotiation

### salary-counter-offer-drafting.md
- CounterOfferDraftService with strict grounding requirement
- Competing offer handling: never fabricates, only uses if user entered
- Three tone variants (Professional, Enthusiastic, Assertive)
- Per-company-stage negotiable components
- NegotiationOutcome tracking (Accepted, Rejected, OfferRevised, Deferred)
- NegotiationHistory with rounds and comp_delta
- Human-in-the-loop boundary: [DRAFT - NOT SENT] header, copy only

### 09-tui-design-keybindings.md
- Complete TUI layout (header, sidebar, main content, status bar)
- View hierarchy (Dashboard, Jobs, Applications, Contacts, Ralph, Settings)
- Panel dimensions with constraints
- Vim-inspired keybinding philosophy (hjkl, primary actions on obvious keys)
- Complete keybinding reference
- Component patterns (job_card, application_card, contact_card, stat_block, etc.)
- Modal dialogs (confirm, input)
- State management (JobsListState, ApplicationState, ContactListState)
- Color scheme (dark theme with primary, secondary, status colors)
- Animation/transitions guidance (instant, smooth scroll, fade)

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-78: TUI Accessibility (Screen Readers, Color Blind Mode) (CRITICAL)

**Location**: `09-tui-design-keybindings.md` - Open Question #5: "Screen reader support? High contrast mode?" - no resolution

**What's missing**:
1. **Screen reader (a11y) support**: TUI apps can use terminal a11y APIs but this requires explicit design. How does screen reading work with ratatui?
2. **High contrast mode**: For visually impaired users, the dark theme isn't sufficient. Need a high-contrast variant.
3. **Color blind modes**: Deuteranopia (red-green), Protanopia (red-green), Tritanopia (blue-yellow). Need palette alternatives.
4. **Focus indicators**: For keyboard navigation, clear visible focus states for all interactive elements
5. **ARIA labels**: For complex widgets, how to provide accessible labels in terminal context?
6. **Reduced motion**: Can animations be disabled for users who need this?
7. **Font size scaling**: Some users need larger text. Can TUI text scale?

**Why critical**: A significant portion of users have accessibility needs. Ignoring this excludes users and may have legal implications (ADA).

**What could go wrong**:
- Blind user can't use LazyJob at all
- Color blind user can't distinguish interview stages (green vs red badges)
- Low vision user can't read small text in dense TUI layout

---

### GAP-79: Vim Mode Deep Implementation (CRITICAL)

**Location**: `09-tui-design-keybindings.md` - "Vim-inspired navigation" philosophy but no actual mode system

**What's missing**:
1. **Mode system**: Normal, Insert, Visual modes. What's the actual mode system?
2. **Mode indicators**: How does user know which mode they're in? (Cursor shape? Status indicator?)
3. **Insert mode triggers**: What keys switch to insert mode? (`i`, `a`, `o`, etc.)
4. **Normal mode commands**: Full vim command repertoire or subset? (`dd`, `yy`, `p`, `cw`, etc.)
5. **Leader key**: Is there a leader key for LazyJob-specific commands?
6. **Text objects**: For navigating structured text (job descriptions, etc.)
7. **Macros**: Can users record and playback key sequences?
8. **Registers**: For copying/pasting multiple things
9. **Motion commands**: `w`, `b`, `f`, `t`, `}`, `{`, `G`, `gg`, etc.

**Why critical**: Vim users (common among developers) expect a real vim implementation, not just vim-like keys.

**What could go wrong**:
- Vim user expects `ci"` (change inside quotes) but it doesn't work
- User presses `i` thinking they're in vim but they're not
- Mode confusion leads to accidental actions

---

### GAP-80: TUI Copy/Paste and Clipboard Integration (CRITICAL)

**Location**: `09-tui-design-keybindings.md` - Open Question #2: "How should copy/paste work in TUI context?" - no resolution

**What's missing**:
1. **System clipboard access**: Can LazyJob read/write the system clipboard? (Rust crates like `arboard`)
2. **Visual selection mode**: How does user select text? (v for visual, motion to select)
3. **Copy command**: `y` to yank selected text to clipboard
4. **Paste command**: `p` to paste from clipboard
5. **Selection persistence**: If user selects text in visual mode, then switches views, is selection preserved?
6. **Smart selection**: Auto-select current word, current line, etc.
7. **Copy to LazyJob clipboard**: Can user copy from LazyJob content (job description, cover letter draft) to system clipboard?
8. **Paste into forms**: Can user paste into input fields?

**Why critical**: Copy/paste is fundamental. Without it, users can't copy job descriptions or outreach drafts.

**What could go wrong**:
- User can't copy job URL from LazyJob
- User can't paste their resume content into LazyJob form
- Copying from TUI conflicts with terminal selection mode

---

### GAP-81: Startup Equity Valuation (Black-Scholes) (IMPORTANT)

**Location**: `salary-negotiation-offers.md` - research mentions Black-Scholes for options; `salary-market-intelligence.md` - simple multiplier table only

**What's missing**:
1. **Black-Scholes implementation**: For options at early-stage startups, simple multiplier (0.15) is crude. Should we implement proper option valuation?
2. **Required inputs**: Strike price, 409A FMV, expected volatility, time to expiration, risk-free rate
3. **Liquidation preference modeling**: Preferred vs common equity changes option value
4. **Dilution projection**: How future funding rounds affect option value
5. **User-facing complexity**: Should users enter all these parameters, or is there a simplified mode?
6. **Accuracy vs simplicity**: What's the right balance for a job search tool?

**Why important**: For startup offers, the simple multiplier table could be wildly off. A $1M option grant could be worth $10K or $500K depending on company trajectory.

**What could go wrong**:
- User accepts startup offer thinking equity is worth $X, it's actually worth $X/10
- Too many parameters confuse users, they give up on equity comparison
- Option valuation is inherently speculative for early-stage startups

---

### GAP-82: Offer Letter Parsing (IMPORTANT)

**Location**: `salary-counter-offer-drafting.md` - Open Question #2: offer letter parsing, no resolution

**What's missing**:
1. **PDF extraction**: How to extract text from offer letter PDF? (pdf-extract? tesseract?)
2. **LLM parsing**: Use LLM to parse structured fields from unstructured offer text
3. **Field extraction**: Base salary, equity grant, strike price, vest schedule, signing bonus, start date
4. **Confidence scoring**: How confident is the parser? Which fields are uncertain?
5. **User confirmation**: All extracted fields must be confirmed by user before saving
6. **Error handling**: What if parsing fails completely?
7. **Supported formats**: PDF only? Or also DOCX, HTML email?

**Why important**: Manual entry of offer details is tedious and error-prone. Parsing would dramatically reduce friction.

**What could go wrong**:
- Parser extracts wrong salary (off by factor of 10), user doesn't notice, makes bad decision
- Parsing fails, user has to enter everything manually anyway
- Privacy: offer letter uploaded to LLM for parsing - where does data go?

---

### GAP-83: Benefits Valuation Comparison (MODERATE)

**Location**: `salary-market-intelligence.md` - TotalCompBreakdown includes base/bonus/equity/signing but NOT benefits

**What's missing**:
1. **Benefits components**: Health insurance (family vs single), 401k match %, PTO days, sick leave, parental leave
2. **Dollar value calculation**: Health insurance cost difference between offers, 401k match dollar value
3. **Comparable total**: "Total compensation including benefits" = cash + equity + benefits_value
4. **Benefits survey**: Can user enter benefits from offer letter and get a comparison?
5. **Hidden benefits value**: Some companies have excellent benefits worth significant money
6. **Non-monetary factors**: Remote policy, WLB, company culture - harder to quantify but important

**Why important**: Benefits can add 20-40% to effective compensation. Two offers with same cash comp could differ by $30K+ in benefits.

**What could go wrong**:
- User picks offer with lower cash but worse benefits, unaware of total value difference
- Benefits entry is too complex, user skips it
- Benefits are hard to quantify (career growth, team quality)

---

### GAP-84: Salary Data Internationalization (MODERATE)

**Location**: All salary specs - all data appears to be USD-only

**What's missing**:
1. **Multi-currency support**: User might be comparing offers in USD, EUR, GBP
2. **Currency conversion**: Live exchange rates? Cached daily rates?
3. **International salary data**: H1B LCA is US-only. How to handle international job searches?
4. **Location adjustment**: Same role pays differently in SF vs Austin vs remote
5. **International cost-of-living**: How to compare offers in different locations?
6. **Tax awareness**: Salaries are pre-tax. After-tax comparison may differ significantly

**Why important**: Users may search globally, not just in US.

**What could go wrong**:
- User compares EUR offer to USD offer without converting, thinks one is lower
- International job search users get no salary data support
- Tax differences make raw salary comparison misleading

---

### GAP-85: TUI Notification System (MODERATE)

**Location**: `09-tui-design-keybindings.md` - Open Question #3: "Should there be terminal notifications (native OS notifications)?"

**What's missing**:
1. **Native OS notifications**: Can we send real OS notifications via Rust crates (notify-rust)?
2. **In-TUI notifications**: Toast notifications within the TUI itself
3. **Notification triggers**: Interview reminder, offer expiring, job match found, Ralph loop complete
4. **Notification preferences**: Per-category on/off, quiet hours
5. **Notification persistence**: If TUI is closed, do notifications still fire?
6. **Email fallback**: For critical items, email notification if TUI is offline?

**Why important**: Users can't stare at TUI all day. They need alerts for important events.

**What could go wrong**:
- User misses interview because they weren't watching TUI
- Notification spam: every new job match triggers notification
- TUI closed, notifications don't fire, user misses critical deadline

---

### GAP-86: Mouse Support in TUI (MODERATE)

**Location**: `09-tui-design-keybindings.md` - Open Question #1: "Mouse support?" - no resolution

**What's missing**:
1. **Click to focus**: Clicking on a view focuses it
2. **Click to select**: Clicking on list item selects it
3. **Scroll support**: Mouse scroll in lists and long content
4. **Drag support**: For kanban card movement in applications view
5. **Resize handles**: For panels that could be resized
6. **Right-click context menu**: Right-click on items for context actions
7. **Hover states**: Visual indication when hovering over clickable elements

**Why important**: Many users expect mouse support in TUI apps. Vim purists may disagree but accessibility requires options.

**What could go wrong**:
- User tries to click on kanban card but it doesn't work
- Scroll wheel doesn't work in long job description
- Inconsistent behavior between mouse and keyboard

---

### GAP-87: Negotiation Round Warning System (MODERATE)

**Location**: `salary-counter-offer-drafting.md` - Open Question #1 mentions 3+ rounds warning but no spec

**What's missing**:
1. **Round counter**: Track which negotiation round user is on for each application
2. **Round threshold**: At which round to show warning? (2? 3?)
3. **Warning message**: What specifically to say at each threshold
4. **Warning UI**: How to surface the warning? Modal? Status bar alert?
5. **Override option**: Can user dismiss warning and continue negotiating?
6. **Round logging**: Track all rounds in NegotiationHistory

**Why important**: Research shows 3+ counter-offer rounds damages relationships. Users should be warned before they hurt their candidacy.

**What could go wrong**:
- User goes to round 4, relationship damaged, doesn't know why
- Warning too aggressive/paternalistic, user ignores or annoyed
- No tracking of rounds, user loses sense of negotiation state

---

## Cross-Spec Gaps

### Cross-Spec R: TUI State ↔ Application State

The TUI state management (ApplicationState, JobsListState, ContactListState) interacts with but isn't formally connected to the application workflow state machine. There's no spec for:
- What happens to TUI state when application transitions?
- How to invalidate cached TUI state after database writes
- How TUI state is persisted/restored across sessions

**Affected specs**: `09-tui-design-keybindings.md`, `application-state-machine.md`

### Cross-Spec S: Salary Privacy ↔ SaaS Sync

`salary-market-intelligence.md` explicitly excludes offer_details from SaaS sync. But what about NegotiationHistory (which contains offer details)? And market_data_references?

**Affected specs**: `salary-market-intelligence.md`, `salary-counter-offer-drafting.md`, (saas-migration-path.md)

---

## Specs to Create

### Critical Priority

1. **XX-tui-accessibility.md** - Screen reader support, high contrast, color blind modes, focus indicators, scaling
2. **XX-tui-vim-mode.md** - Full vim mode implementation (normal/insert/visual), mode indicators, commands, motion
3. **XX-tui-clipboard-integration.md** - System clipboard access, visual selection, copy/paste commands

### Important Priority

4. **XX-startup-equity-valuation.md** - Black-Scholes implementation, liquidation preferences, dilution projection
5. **XX-offer-letter-parsing.md** - PDF extraction, LLM parsing, confidence scoring, user confirmation

### Moderate Priority

6. **XX-benefits-valuation-comparison.md** - Health, 401k, PTO dollar values, comparable total
7. **XX-salary-internationalization.md** - Multi-currency, exchange rates, international data, tax awareness
8. **XX-tui-notification-system.md** - OS notifications, in-TUI toasts, preferences, email fallback
9. **XX-tui-mouse-support.md** - Click focus, scroll, drag, context menus, hover states
10. **XX-negotiation-round-warning.md** - Round counter, threshold warnings, UI surfacing, override option

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-78: TUI Accessibility | Critical | High | User inclusion |
| GAP-79: Vim Mode Deep | Critical | High | Developer UX |
| GAP-80: Clipboard Integration | Critical | Medium | Core functionality |
| GAP-81: Startup Equity Valuation | Important | High | Offer accuracy |
| GAP-82: Offer Letter Parsing | Important | Medium | UX friction reduction |
| GAP-83: Benefits Valuation | Moderate | Medium | Complete comparison |
| GAP-84: Salary Internationalization | Moderate | Medium | Global support |
| GAP-85: TUI Notifications | Moderate | Medium | User awareness |
| GAP-86: Mouse Support | Moderate | Low | Accessibility |
| GAP-87: Negotiation Round Warning | Moderate | Low | Relationship protection |
