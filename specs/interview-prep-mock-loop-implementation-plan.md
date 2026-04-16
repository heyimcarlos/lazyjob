# Implementation Plan: Mock Interview Loop

## Status
Draft

## Related Spec
[specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md)

## Overview

The `MockInterviewLoop` is an interactive ralph subprocess that drives a turn-based mock interview session. The candidate reads each AI-generated question in the TUI, types a response, and immediately receives structured per-question feedback — STAR breakdown for behavioral questions, accuracy/depth/communication for technical, authenticity/values/completeness for culture/situational. After all questions are answered the loop computes a `SessionScore` (weighted average across categories), emits a `MockSessionSummary` event, and persists the full session to SQLite.

Unlike all other Ralph loops, which are fire-and-forget workers that write output to SQLite and exit, `MockInterviewLoop` is *bidirectional and interactive*. After emitting each `MockQuestion` event it blocks on stdin waiting for a `WorkerCommand::UserInput`. The `RalphProcessManager` must exempt interactive workers from inactivity kill timeouts; only the user's own timeout setting in the TUI applies. The TUI renders a chat-style Q→A→feedback cadence within the Ralph panel.

Session data is fully persisted. The `mock_interview_sessions` and `mock_interview_responses` tables are append-only; a partially completed session (where `completed_at IS NULL`) can be queried and displayed as a historical artifact, and a future extension can resume from the last answered question at the cost of re-supplying prior Q&A context to the LLM.

## Prerequisites

### Specs/Plans that must precede this
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `WorkExperience`, `LifeSheetRepository`
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, migration runner, `sqlx::Pool<Sqlite>`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>`, `ChatMessage`, `CompletionRequest`
- `specs/agentic-ralph-subprocess-protocol-implementation-plan.md` — defines `WorkerCommand`, `WorkerEvent`, NDJSON codec, `RalphProcessManager`, interactive-worker flag, `CancelToken`
- `specs/agentic-ralph-orchestration-implementation-plan.md` — provides `LoopType::MockInterview`, `LoopQueue`, `LoopDispatch`
- `specs/interview-prep-question-generation-implementation-plan.md` — provides `InterviewQuestion`, `QuestionCategory`, `InterviewPrepSession`, `InterviewPrepRepository`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel system, `KeyContext`
- `specs/agentic-prompt-templates-implementation-plan.md` — provides `RenderedPrompt`, `TemplateEngine`, template TOML format

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml — new additions
serde_json = "1"          # already present — QuestionFeedback JSON blob storage
regex      = "1"          # fabrication warning pattern matching
once_cell  = "1"          # Lazy<Regex> patterns
strsim     = "0.11"       # Jaro-Winkler for keyword overlap (story ref matching)

# lazyjob-ralph/Cargo.toml — new additions (all workspace deps)
lazyjob-core  = { path = "../lazyjob-core" }
lazyjob-llm   = { path = "../lazyjob-llm" }
tokio         = { workspace = true, features = ["full"] }
serde         = { workspace = true }
serde_json    = { workspace = true }
uuid          = { workspace = true }
anyhow        = { workspace = true }
thiserror     = { workspace = true }
tracing       = { workspace = true }

# lazyjob-tui/Cargo.toml — new additions
ratatui    = { workspace = true }
crossterm  = { workspace = true }
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|---------------|
| `lazyjob-core` | `QuestionFeedback`, `ScoreBreakdown`, `MockResponse`, `MockInterviewSession`, `SessionScore`, `MockInterviewRepository`, `MockInterviewService`, SQLite DDL, migrations 014-015 |
| `lazyjob-llm` | Evaluation prompt template (`LoopType::MockInterviewEval`) embedded as `interview_eval.toml`; `InterviewEvalContext`, `FabricationCheckContext`, prompt builders |
| `lazyjob-ralph` | `MockInterviewLoop` struct, session driver, interactive IPC flow, cancel handling, `WorkerEvent::MockQuestion` / `MockFeedback` / `MockSessionSummary` variants |
| `lazyjob-tui` | `MockInterviewView`, `QuestionPanel`, `AnswerInputWidget`, `FeedbackPanel`, `SessionSummaryPanel`, `ScoreTrendWidget` |
| `lazyjob-cli` | `lazyjob interview mock <application-id> [--prep-session <uuid>]` subcommand |

`lazyjob-core` has no dependency on `lazyjob-ralph` or `lazyjob-tui`. All types flow upward.

### Core Types

```rust
// lazyjob-core/src/interview/mock_session.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-question score breakdown; fields are None when the category doesn't apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    // Behavioral (STAR) — only set when QuestionCategory::Behavioral
    pub situation:     Option<u8>,   // 0-2
    pub task:          Option<u8>,   // 0-2
    pub action:        Option<u8>,   // 0-3
    pub result:        Option<u8>,   // 0-3
    // Technical — only set when QuestionCategory::Technical or SystemDesign
    pub accuracy:      Option<u8>,   // 0-4
    pub depth:         Option<u8>,   // 0-3
    // Universal — all categories
    pub communication: u8,           // 0-3
}

impl ScoreBreakdown {
    /// Sum all present sub-scores. Maximum is 10 regardless of category.
    pub fn total(&self) -> u8 {
        [
            self.situation, self.task, self.action, self.result,
            self.accuracy, self.depth, Some(self.communication),
        ]
        .into_iter()
        .flatten()
        .sum()
    }
}

/// LLM-generated evaluation for a single question-response pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionFeedback {
    pub question_id:             Uuid,
    /// 0–10 (mirrors ScoreBreakdown::total())
    pub score:                   u8,
    pub score_breakdown:         ScoreBreakdown,
    /// 1-3 specific observed strengths.
    pub strengths:               Vec<String>,
    /// 1-3 specific, actionable improvement suggestions.
    pub improvements:            Vec<String>,
    /// Set if the response introduces numeric claims or facts not in the candidate_story_ref.
    pub fabrication_warning:     Option<String>,
    /// Structural coaching hint (never a full fabricated example). See Open Questions.
    pub example_stronger_answer: Option<String>,
}

/// One question→answer→feedback triple recorded during a mock session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    pub id:            Uuid,
    pub question_id:   Uuid,
    pub response_text: String,
    pub feedback:      QuestionFeedback,
    pub answered_at:   DateTime<Utc>,
}

/// Aggregate score across all questions in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionScore {
    /// Weighted average across all answered questions (0.0–10.0).
    pub overall:           f64,
    pub behavioral_avg:    Option<f64>,
    pub technical_avg:     Option<f64>,
    pub culture_avg:       Option<f64>,
    /// Most frequently cited positive signal across all feedback.
    pub top_strength:      String,
    /// Most frequently cited improvement area across all feedback.
    pub top_improvement:   String,
    pub questions_answered: u32,
    pub questions_total:   u32,
}

impl SessionScore {
    /// Compute from a slice of MockResponses + the full question count for the session.
    pub fn compute(responses: &[MockResponse], total_questions: u32) -> Self {
        // Partition by category, compute per-category averages, then weighted overall.
        // top_strength/top_improvement: collect all strengths/improvements into a
        // frequency map (String → usize), return the key with max value.
        todo!()
    }
}

/// Persistent record of a complete (or partial) mock interview session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockInterviewSession {
    pub id:              Uuid,
    pub prep_session_id: Uuid,     // references interview_prep_sessions.id
    pub application_id:  Uuid,     // references applications.id
    pub started_at:      DateTime<Utc>,
    pub completed_at:    Option<DateTime<Utc>>,
    /// All responses in answer order.
    pub responses:       Vec<MockResponse>,
    /// Populated when completed_at is Some.
    pub session_score:   Option<SessionScore>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/interview/mock_session_repository.rs

use async_trait::async_trait;
use uuid::Uuid;
use chrono::DateTime;
use chrono::Utc;

#[async_trait]
pub trait MockInterviewRepository: Send + Sync {
    /// Insert a new session row (completed_at=NULL, no score yet).
    async fn create_session(&self, session: &MockInterviewSession) -> Result<(), MockSessionError>;

    /// Append a MockResponse to an in-progress session.
    async fn save_response(&self, session_id: Uuid, response: &MockResponse) -> Result<(), MockSessionError>;

    /// Set completed_at and session_score_json on a session row.
    async fn complete_session(&self, session_id: Uuid, score: &SessionScore) -> Result<(), MockSessionError>;

    /// Return all sessions for an application, newest first.
    async fn get_sessions_for_application(&self, application_id: Uuid) -> Result<Vec<MockInterviewSession>, MockSessionError>;

    /// Return (started_at, overall_score) pairs for trend display, newest-first.
    async fn get_score_trend(&self, application_id: Uuid) -> Result<Vec<(DateTime<Utc>, f64)>, MockSessionError>;

    /// Load a full session with all responses for display/resume.
    async fn get_session(&self, session_id: Uuid) -> Result<Option<MockInterviewSession>, MockSessionError>;
}
```

```rust
// lazyjob-ralph/src/loops/mock_interview.rs — loop driver trait

use async_trait::async_trait;

/// Implemented by MockInterviewLoop; injectable for testing.
#[async_trait]
pub trait InteractiveMockLoop: Send {
    async fn run(&mut self) -> anyhow::Result<()>;
}
```

### SQLite Schema

```sql
-- migration 014: mock interview sessions
CREATE TABLE mock_interview_sessions (
    id               TEXT PRIMARY KEY,
    prep_session_id  TEXT NOT NULL REFERENCES interview_prep_sessions(id) ON DELETE CASCADE,
    application_id   TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    started_at       TEXT NOT NULL,
    completed_at     TEXT,                 -- NULL means session is in-progress or was abandoned
    session_score_json TEXT,               -- serialized SessionScore JSON blob
    overall_score    REAL,                 -- denormalized for trend queries (no JSON parsing)
    questions_answered INTEGER NOT NULL DEFAULT 0,
    questions_total    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_mock_sessions_application
    ON mock_interview_sessions(application_id, started_at DESC);

-- migration 015: mock interview responses
CREATE TABLE mock_interview_responses (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES mock_interview_sessions(id) ON DELETE CASCADE,
    question_id   TEXT NOT NULL,
    response_text TEXT NOT NULL,
    feedback_json TEXT NOT NULL,           -- serialized QuestionFeedback JSON blob
    score         INTEGER NOT NULL,        -- denormalized 0-10
    answered_at   TEXT NOT NULL
);

CREATE INDEX idx_mock_responses_session
    ON mock_interview_responses(session_id, answered_at ASC);
```

### Module Structure

```
lazyjob-core/
  src/
    interview/
      mod.rs               -- pub use types::*, mock_session::*, mock_session_repository::*
      types.rs             -- InterviewType, QuestionCategory, InterviewQuestion (from question-gen plan)
      mock_session.rs      -- QuestionFeedback, ScoreBreakdown, MockResponse, MockInterviewSession, SessionScore
      mock_session_repository.rs  -- MockInterviewRepository trait + SqliteMockInterviewRepository impl
    db/
      migrations/
        014_mock_interview_sessions.sql
        015_mock_interview_responses.sql

lazyjob-llm/
  src/
    prompts/
      interview_eval.rs    -- InterviewEvalContext, FabricationCheckContext, build_eval_prompt()
      templates/
        interview_eval.toml  -- embedded TOML template for behavioral / technical / culture rubrics

lazyjob-ralph/
  src/
    loops/
      mock_interview.rs    -- MockInterviewLoop::run(), IPC event loop, evaluation dispatch
    lib.rs                 -- pub mod loops;

lazyjob-tui/
  src/
    views/
      mock_interview/
        mod.rs             -- MockInterviewView
        question_panel.rs  -- QuestionPanel widget
        answer_input.rs    -- AnswerInputWidget (modal textarea)
        feedback_panel.rs  -- FeedbackPanel widget with score breakdown bars
        session_summary.rs -- SessionSummaryPanel + anti-overconfidence disclaimer
        score_trend.rs     -- ScoreTrendWidget (per-category sparklines)
```

---

## Implementation Phases

### Phase 1 — Core Domain Types + SQLite Repository (MVP)

**Step 1.1 — Define core types**

File: `lazyjob-core/src/interview/mock_session.rs`

Implement:
- `ScoreBreakdown` with `total()` helper
- `QuestionFeedback` with all fields from spec
- `MockResponse` struct
- `SessionScore` with `compute()` — partition `responses` into behavioral/technical/culture slices, compute `f64` averages using `sum::<u8>() as f64 / count as f64`, collect all `strengths` and `improvements` strings into a `HashMap<&str, usize>` frequency map, return the `max_by_value` key for `top_strength` and `top_improvement`
- `MockInterviewSession` struct

Verification: `cargo test -p lazyjob-core -- interview::mock_session` with a unit test that calls `SessionScore::compute()` on a hand-built slice of `MockResponse` values and asserts category averages are within 0.01.

**Step 1.2 — Write migrations**

Files:
- `lazyjob-core/src/db/migrations/014_mock_interview_sessions.sql`
- `lazyjob-core/src/db/migrations/015_mock_interview_responses.sql`

Use the DDL from the SQLite schema section exactly. Run `sqlx migrate run` against a test DB and verify `PRAGMA table_info(mock_interview_sessions)` returns the expected columns.

**Step 1.3 — Implement `SqliteMockInterviewRepository`**

File: `lazyjob-core/src/interview/mock_session_repository.rs`

Key sqlx calls:
- `create_session`: `sqlx::query!("INSERT INTO mock_interview_sessions (...) VALUES (?,...)", ...)` — serializes nothing to JSON yet (session_score_json = NULL)
- `save_response`: INSERT into `mock_interview_responses`, also increments `mock_interview_sessions.questions_answered` via `UPDATE mock_interview_sessions SET questions_answered = questions_answered + 1 WHERE id = ?`
- `complete_session`: `UPDATE mock_interview_sessions SET completed_at = ?, session_score_json = ?, overall_score = ? WHERE id = ?`
- `get_sessions_for_application`: `SELECT * FROM mock_interview_sessions WHERE application_id = ? ORDER BY started_at DESC` — then for each session load responses via a second `SELECT * FROM mock_interview_responses WHERE session_id = ? ORDER BY answered_at ASC` and deserialize `feedback_json` via `serde_json::from_str::<QuestionFeedback>(...)`
- `get_score_trend`: `SELECT started_at, overall_score FROM mock_interview_sessions WHERE application_id = ? AND overall_score IS NOT NULL ORDER BY started_at DESC LIMIT 20` — map to `Vec<(DateTime<Utc>, f64)>`
- `get_session`: same as sessions_for_application but scoped to one ID

Use `#[sqlx::test(migrations = "src/db/migrations")]` for all repository tests. Provide a `MockInterviewRepository` in-memory variant backed by `Arc<Mutex<Vec<MockInterviewSession>>>` for use in non-DB unit tests.

Verification: All `#[sqlx::test]` tests pass with `cargo test -p lazyjob-core`.

---

### Phase 2 — LLM Evaluation Prompt Templates

**Step 2.1 — Define evaluation context structs**

File: `lazyjob-llm/src/prompts/interview_eval.rs`

```rust
use serde::Serialize;
use lazyjob_core::interview::types::{InterviewQuestion, QuestionCategory};
use lazyjob_core::life_sheet::WorkExperience;

/// Context injected into the behavioral STAR evaluation prompt.
#[derive(Debug, Serialize)]
pub struct BehavioralEvalContext<'a> {
    pub question_text:       &'a str,
    pub candidate_response:  &'a str,
    /// Serialized JSON of the linked WorkExperience if candidate_story_ref is Some.
    pub story_ref:           Option<&'a WorkExperience>,
}

/// Context injected into the technical evaluation prompt.
#[derive(Debug, Serialize)]
pub struct TechnicalEvalContext<'a> {
    pub question_text:      &'a str,
    pub candidate_response: &'a str,
    pub technical_domain:   &'a str,  // e.g. "Rust async", "Kubernetes"
}

/// Context for culture/situational evaluation.
#[derive(Debug, Serialize)]
pub struct CultureEvalContext<'a> {
    pub question_text:      &'a str,
    pub candidate_response: &'a str,
    /// Company culture signals from CompanyRecord (e.g. ["customer obsession", "bias for action"])
    pub culture_signals:    &'a [String],
}
```

**Step 2.2 — Write evaluation TOML templates**

File: `lazyjob-llm/src/prompts/templates/interview_eval.toml`

```toml
[behavioral]
system = """
You are a rigorous interview coach evaluating a candidate's behavioral interview response.
Your output must be a JSON object conforming exactly to the QuestionFeedback schema.

Evaluation rubric (STAR framework):
- situation (0-2): Was the context clear and specific?
- task (0-2): Did the candidate clearly state their personal objective?
- action (0-3): Were the actions first-person and specific? Penalize "we did" without a personal contribution.
- result (0-3): Was the impact quantified? Did they reflect on what they learned?
- communication (0-3): Was the response clear, concise, and structured?

{story_ref_section}

Respond ONLY with valid JSON. No prose before or after.
"""

story_ref_section_present = """
GROUNDING CHECK — The following verified facts about the candidate's experience are provided.
If the candidate's response introduces numeric claims (percentages, dollar amounts, team sizes)
or specific outcomes NOT present in the verified facts below, set fabrication_warning to a
specific warning string. Do not invent a warning if nothing new was introduced.

Verified story facts:
{story_ref_json}
"""

story_ref_section_absent = """
No verified story reference provided. Do not check for fabrication in this evaluation.
"""

[technical]
system = """
You are a senior engineer evaluating a technical interview response.
Your output must be a JSON object conforming exactly to the QuestionFeedback schema.

Evaluation rubric:
- accuracy (0-4): Is the answer technically correct? Penalize factual errors.
- depth (0-3): Did the candidate explain tradeoffs, edge cases, or system implications?
- communication (0-3): Was the reasoning explained (not just the conclusion)?

Respond ONLY with valid JSON. No prose before or after.
"""

[culture]
system = """
You are evaluating a candidate's response to a culture-fit or situational interview question.
Your output must be a JSON object conforming exactly to the QuestionFeedback schema.

Evaluation rubric:
- authenticity_signal (0-3): Does the response feel specific and genuine (not generic buzzwords)?
- values_alignment (0-4): Does it align with the company values: {culture_signals_list}?
- completeness (0-3): Did the candidate fully address the scenario?
- communication (0-3): Was the response clear and well-structured?

Respond ONLY with valid JSON. No prose before or after.
"""
```

**Step 2.3 — Implement `build_eval_prompt()`**

File: `lazyjob-llm/src/prompts/interview_eval.rs`

```rust
use crate::prompts::{RenderedPrompt, TemplateEngine};
use lazyjob_core::interview::types::QuestionCategory;

pub fn build_eval_prompt(
    engine:   &TemplateEngine,
    category: &QuestionCategory,
    question: &str,
    response: &str,
    ctx:      EvalPromptExtra<'_>,
) -> anyhow::Result<RenderedPrompt> {
    // Select template section based on category.
    // Inject story_ref_section_present or story_ref_section_absent.
    // Return RenderedPrompt with cache_system_prompt = true (system is stable per category).
    todo!()
}

pub enum EvalPromptExtra<'a> {
    Behavioral { story_ref: Option<&'a WorkExperience> },
    Technical  { domain: &'a str },
    Culture    { culture_signals: &'a [String] },
}
```

The `RenderedPrompt` produced by `build_eval_prompt()` sets `cache_system_prompt = true` for all three categories — the system prompt is the same for every evaluation in a session, so Anthropic prompt caching amortizes the cost across all questions.

Verification: Unit test that calls `build_eval_prompt()` with a mocked `TemplateEngine` and asserts the rendered system prompt contains "STAR framework" for behavioral and "technically correct" for technical.

---

### Phase 3 — Ralph Subprocess Loop Driver

**Step 3.1 — Add `MockInterview` variant to `LoopType` and `WorkerEvent`**

File: `lazyjob-core/src/ralph/types.rs`

```rust
// Add to LoopType enum
MockInterview,

impl LoopType {
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::MockInterview)
    }
    pub fn concurrency_limit(&self) -> usize {
        match self {
            Self::MockInterview => 1,  // never run two mock sessions simultaneously
            // ... other variants
        }
    }
}
```

File: `lazyjob-ralph/src/protocol.rs` (or wherever `WorkerEvent` is defined)

```rust
// Add to WorkerEvent enum
MockQuestion {
    question: InterviewQuestion,
    question_number: u32,
    total_questions: u32,
},
MockFeedback {
    feedback: QuestionFeedback,
    response_id: Uuid,
},
MockSessionSummary {
    session: MockInterviewSession,
},
```

File: `lazyjob-ralph/src/manager.rs` — in `spawn()`, add a guard:

```rust
if loop_type.is_interactive() {
    // Set no inactivity_kill_timeout for this worker.
    config.inactivity_kill_timeout = None;
}
```

**Step 3.2 — Implement `MockInterviewLoop::run()`**

File: `lazyjob-ralph/src/loops/mock_interview.rs`

```rust
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;
use uuid::Uuid;

pub struct MockInterviewLoop {
    pub prep_session_id: Uuid,
    pub application_id:  Uuid,
    db_pool:             sqlx::SqlitePool,
    llm:                 Arc<dyn LlmProvider>,
    cancel:              watch::Receiver<bool>,
}

impl MockInterviewLoop {
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // 1. Load InterviewPrepSession (questions ordered by category/number).
        // 2. Create mock_interview_sessions row via repo.create_session().
        // 3. For each InterviewQuestion:
        //    a. Emit WorkerEvent::MockQuestion (serde_json serialized, newline-terminated to stdout).
        //    b. Call self.read_user_response().await — blocks on stdin.
        //    c. If CancelToken fires, break out of loop (session stays open/partial).
        //    d. Call self.evaluate_response(question, response).await.
        //    e. Emit WorkerEvent::MockFeedback.
        //    f. Call repo.save_response(session_id, &mock_response).await.
        // 4. Compute SessionScore::compute(&responses, total).
        // 5. Call repo.complete_session(session_id, &score).await.
        // 6. Emit WorkerEvent::MockSessionSummary.
        Ok(())
    }

    async fn read_user_response(&mut self) -> anyhow::Result<Option<String>> {
        // Read lines from stdin until WorkerCommand::UserInput or Cancel arrives.
        // WorkerCommand::UserInput { text } → return Some(text)
        // WorkerCommand::Cancel → return None (triggers partial-session cleanup)
        // EOF on stdin → return None
        todo!()
    }

    async fn evaluate_response(
        &self,
        question: &InterviewQuestion,
        response: &str,
        story_ref: Option<&WorkExperience>,
    ) -> anyhow::Result<QuestionFeedback> {
        // Build RenderedPrompt via build_eval_prompt().
        // Call self.llm.complete(CompletionRequest { ..., temperature: 0.2 }).await.
        // Parse response as QuestionFeedback via serde_json::from_str().
        // If parsing fails: return a QuestionFeedback with score=0, all fields set to
        // "evaluation unavailable — JSON parse error" and log a tracing::warn!.
        // Do NOT propagate the parse error — the session must continue.
        todo!()
    }
}
```

The `temperature: 0.2` for evaluation calls is deliberately low: the rubric is well-defined and we want consistent, predictable scoring. Creative variation here works against the user (two sessions for the same answer should give similar scores).

**Step 3.3 — IPC stdin reader**

File: `lazyjob-ralph/src/loops/mock_interview.rs`

```rust
async fn read_user_response(&mut self) -> anyhow::Result<Option<String>> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    loop {
        tokio::select! {
            biased;
            _ = self.cancel.changed() => {
                if *self.cancel.borrow() { return Ok(None); }
            }
            n = reader.read_line(&mut line) => {
                if n? == 0 { return Ok(None); }  // EOF
                let cmd: WorkerCommand = serde_json::from_str(line.trim())?;
                line.clear();
                match cmd {
                    WorkerCommand::UserInput { text } => return Ok(Some(text)),
                    WorkerCommand::Cancel => return Ok(None),
                    _ => continue,
                }
            }
        }
    }
}
```

**Step 3.4 — Emit NDJSON events**

All events are written to stdout using the NDJSON codec established in the subprocess protocol plan:

```rust
fn emit_event(event: &WorkerEvent) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    std::io::stdout().write_all(line.as_bytes())?;
    std::io::stdout().flush()?;
    Ok(())
}
```

Interactive workers must never buffer stdout. The `flush()` after every event is mandatory.

Verification: Write an integration test that spawns `MockInterviewLoop::run()` in a `tokio::spawn` task, sends mock `WorkerCommand::UserInput` events on the stdin channel, and asserts that `WorkerEvent::MockFeedback` events are received (using a mock `LlmProvider` that returns a hardcoded `QuestionFeedback`).

---

### Phase 4 — TUI Mock Interview View

**Step 4.1 — `MockInterviewView` layout**

File: `lazyjob-tui/src/views/mock_interview/mod.rs`

The view consists of three vertical sections:

```
┌─────────────────────────────────────────────────────┐
│  [Session Header] Application: Acme Corp — Phone    │
│  Mock Interview  q3/6  [disclaimer strip]           │
├──────────────────────────┬──────────────────────────┤
│  QuestionPanel           │  FeedbackPanel           │
│  (left 55%)              │  (right 45%)             │
│                          │                          │
│  Q: Tell me about a time │  Score: 7/10             │
│  you led a project under │  ✓ Clear situation        │
│  ambiguity.              │  ✓ First-person actions   │
│                          │  △ Quantify the result    │
│  [Your Answer:]          │                          │
│  ┌────────────────────┐  │                          │
│  │ type here...       │  │                          │
│  └────────────────────┘  │                          │
├──────────────────────────┴──────────────────────────┤
│  [q]uit  [Enter] submit  [Tab] toggle focus         │
└─────────────────────────────────────────────────────┘
```

`ratatui::layout::Layout::default().direction(Direction::Vertical)` splits into header (3 lines), body (main area), footer (2 lines). The body uses `Direction::Horizontal` with `Constraint::Percentage(55)` and `Constraint::Percentage(45)`.

**Step 4.2 — `AnswerInputWidget`**

File: `lazyjob-tui/src/views/mock_interview/answer_input.rs`

The answer input uses a `tui-textarea` widget (crate: `tui-textarea 0.7`) for multi-line editing with vim-mode support. This is consistent with the keybindings plan (vim-style modal navigation).

```toml
# lazyjob-tui/Cargo.toml
tui-textarea = { version = "0.7", features = ["crossterm"] }
```

```rust
use tui_textarea::{Input, TextArea};

pub struct AnswerInputWidget<'a> {
    textarea: TextArea<'a>,
}

impl<'a> AnswerInputWidget<'a> {
    pub fn new() -> Self {
        let mut ta = TextArea::default();
        ta.set_block(Block::default().borders(Borders::ALL).title("Your Answer"));
        ta.set_placeholder_text("Type your answer and press Enter to submit…");
        Self { textarea: ta }
    }

    pub fn handle_input(&mut self, input: Input) {
        self.textarea.input(input);
    }

    pub fn submit(&mut self) -> String {
        let text = self.textarea.lines().join("\n");
        self.textarea = TextArea::default();  // clear for next question
        text
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(&self.textarea, area);
    }
}
```

Submission is triggered by `Ctrl+Enter` (not bare `Enter` to avoid accidental early submit). This is configured in the keybinding map under `KeyContext::MockInterviewInput`.

**Step 4.3 — `FeedbackPanel` with score bars**

File: `lazyjob-tui/src/views/mock_interview/feedback_panel.rs`

When no feedback is available yet (waiting for LLM response), the panel shows a spinner using the `indicatif`-style Unicode spinner rendered manually on each tick. Once `WorkerEvent::MockFeedback` arrives, the panel renders:

```rust
// STAR sub-scores as colored progress bars using ratatui::widgets::Gauge
// score/max * 100 as the ratio; color based on score:
// >= 80%: Color::Green
// >= 50%: Color::Yellow
// < 50%:  Color::Red

let gauge = Gauge::default()
    .block(Block::default().title("Action"))
    .gauge_style(Style::default().fg(score_color))
    .ratio(action_score as f64 / 3.0);
frame.render_widget(gauge, sub_area);
```

Strengths are shown as `Span::styled("✓ ...", Style::default().fg(Color::Green))`.
Improvements are shown as `Span::styled("△ ...", Style::default().fg(Color::Yellow))`.
`fabrication_warning` is shown as a bold red banner if `Some`.

**Step 4.4 — `SessionSummaryPanel` with anti-overconfidence disclaimer**

File: `lazyjob-tui/src/views/mock_interview/session_summary.rs`

```rust
const ANTI_OVERCONFIDENCE_DISCLAIMER: &str =
    "AI feedback is an approximation. Practice with real humans for system design and live coding.";
```

This string is a constant — NOT configurable. It must appear in the session summary panel above the score table. Render it as `Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC)`.

The summary renders:
1. Disclaimer (yellow italic)
2. Overall score as a large `Gauge` widget
3. Per-category scores as three horizontal `Gauge` widgets
4. Top strength (green) and top improvement (yellow) as styled paragraphs
5. List of all `MockResponse` question titles + per-question scores

**Step 4.5 — `ScoreTrendWidget`**

File: `lazyjob-tui/src/views/mock_interview/score_trend.rs`

Displayed in the application detail view (not within the active session). Queries `MockInterviewRepository::get_score_trend()` and renders per-category score history using `ratatui::widgets::Chart` with `Dataset` per category:

```rust
let behavioral_data: Vec<(f64, f64)> = trend
    .iter()
    .enumerate()
    .map(|(i, (_, score))| (i as f64, *score))
    .collect();

let dataset = Dataset::default()
    .name("Behavioral")
    .marker(symbols::Marker::Dot)
    .graph_type(GraphType::Line)
    .style(Style::default().fg(Color::Cyan))
    .data(&behavioral_data);
```

If fewer than 2 sessions exist, show a placeholder: "Complete 2+ sessions to see trend."

Verification: Unit test `ScoreTrendWidget::render()` by calling it with a `TestBackend` and asserting the rendered buffer contains "Behavioral".

---

### Phase 5 — Session Resumability (Extension)

This phase is deferred to post-MVP. It addresses the "partial sessions" open question from the spec.

**Design decision (record for future implementation):** A session where `completed_at IS NULL` is treated as "abandoned" in Phase 1-4. In Phase 5, the TUI will offer: "Resume previous session? (Y/n)" on entering the mock loop if an in-progress session exists for the prep_session. Resuming requires:

1. Loading all previous `MockResponse` rows from SQLite.
2. Reconstructing the conversation history as `Vec<ChatMessage>` alternating user responses and assistant feedback.
3. Prepending this history to the LLM context before evaluating the next question.

Token cost: ~500 tokens per prior Q&A pair. At 6 questions per session, the worst case (resuming on question 6) adds ~2500 tokens to the context — acceptable for Anthropic Claude 3.5 Sonnet.

**Step 5.1 — Detect and offer resume in TUI**

In `MockInterviewView::new()`, before creating a new session:

```rust
if let Some(partial) = repo.get_partial_session(prep_session_id).await? {
    // Emit TUI event: MockInterviewAction::ConfirmResume { partial_session_id }
    // Show confirmation dialog
}
```

**Step 5.2 — Resume path in `MockInterviewLoop`**

Add `resume_from: Option<Uuid>` to `MockInterviewLoop`. When `Some`, load prior responses, skip already-answered questions, inject prior Q&A into LLM context as conversation history.

---

## Key Crate APIs

- `sqlx::query_as!(MockInterviewSessionRow, "SELECT ...", ...)` — typed query macro with compile-time SQL checking
- `sqlx::query!("INSERT INTO mock_interview_responses ...", ...)` — for `save_response`
- `serde_json::from_str::<QuestionFeedback>(row.feedback_json.as_str())` — deserialize stored JSON blob
- `serde_json::to_string(&WorkerEvent::MockQuestion { ... })` — NDJSON serialization for IPC
- `tokio::io::BufReader::new(tokio::io::stdin())` — async stdin reader in subprocess
- `tokio::io::AsyncBufReadExt::read_line()` — line-by-line stdin reading without blocking the async runtime
- `tokio::select! { biased; _ = cancel.changed() => ..., n = reader.read_line(&mut line) => ... }` — cancel-aware stdin loop
- `tui_textarea::TextArea::input(crossterm_event)` — multi-line answer input
- `ratatui::widgets::Gauge::default().ratio(score as f64 / max as f64)` — score bar rendering
- `ratatui::widgets::Chart::new(vec![dataset])` — score trend sparkline
- `ratatui::layout::Layout::default().constraints([...]).split(area)` — panel layout
- `ratatui::widgets::Clear` — used to erase behind the fabrication warning modal overlay

---

## Error Handling

```rust
// lazyjob-core/src/interview/mock_session.rs

#[derive(thiserror::Error, Debug)]
pub enum MockSessionError {
    #[error("session not found: {0}")]
    NotFound(Uuid),

    #[error("session already completed: {0}")]
    AlreadyCompleted(Uuid),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// lazyjob-ralph/src/loops/mock_interview.rs

#[derive(thiserror::Error, Debug)]
pub enum MockLoopError {
    #[error("prep session not found: {0}")]
    PrepSessionNotFound(Uuid),

    #[error("no questions in prep session")]
    NoQuestions,

    #[error("LLM evaluation failed: {0}")]
    EvaluationFailed(#[from] anyhow::Error),

    #[error("IPC stdin closed unexpectedly")]
    StdinClosed,

    #[error("cancelled by user")]
    Cancelled,
}
```

`MockLoopError::EvaluationFailed` is caught per-question (not per-session). On LLM failure for a single question, the loop emits a `WorkerEvent::MockFeedback` with a fallback `QuestionFeedback` (`score = 0`, `improvements = vec!["Evaluation unavailable — LLM error".to_string()]`) and continues to the next question. Sessions must not abort because of a single failed evaluation.

`MockLoopError::Cancelled` causes the loop to flush a partial `SessionScore` (over the questions answered so far) and persist the session with `completed_at = None`, then exit cleanly. This is not an error condition for the process manager.

---

## Testing Strategy

### Unit tests

**`lazyjob-core`**

- `SessionScore::compute()` with 3 behavioral responses: assert `behavioral_avg` is mean of scores, `overall` matches weighted formula, `top_strength` is the most-common strength string.
- `ScoreBreakdown::total()` for a behavioral breakdown (sum of STAR + communication) and a technical breakdown (accuracy + depth + communication).
- `SqliteMockInterviewRepository::create_session` + `save_response` + `complete_session` round-trip using `#[sqlx::test(migrations = "src/db/migrations")]`.
- `SqliteMockInterviewRepository::get_score_trend` returns results sorted newest-first for an application with 3 sessions.

**`lazyjob-llm`**

- `build_eval_prompt()` with `EvalPromptExtra::Behavioral { story_ref: Some(...) }` — assert the returned system prompt contains "GROUNDING CHECK" and the serialized story facts.
- `build_eval_prompt()` with `story_ref: None` — assert the returned system prompt contains "No verified story reference" instead.
- `build_eval_prompt()` with `EvalPromptExtra::Technical` — assert "accuracy" appears in the system prompt.

**`lazyjob-ralph`**

- `MockInterviewLoop::read_user_response()` with a `WorkerCommand::UserInput { text }` sent on the mock stdin channel — assert `Some(text)` is returned.
- `MockInterviewLoop::read_user_response()` with `WorkerCommand::Cancel` — assert `None` is returned.
- `MockInterviewLoop::evaluate_response()` using a `MockLlmProvider` that returns a JSON `QuestionFeedback` — assert the returned feedback matches.
- `MockInterviewLoop::evaluate_response()` with a malformed JSON response from the mock LLM — assert a fallback `QuestionFeedback` is returned (no panic, no error propagation).

**`lazyjob-tui`**

- `SessionSummaryPanel::render()` using `ratatui::backend::TestBackend` — assert the rendered buffer contains `ANTI_OVERCONFIDENCE_DISCLAIMER` text.
- `ScoreTrendWidget::render()` with 3 sessions — assert the buffer contains "Behavioral" label.
- `ScoreTrendWidget::render()` with 1 session — assert the buffer contains "Complete 2+ sessions".
- `AnswerInputWidget::submit()` — assert returned text matches lines typed into the textarea.

### Integration tests

**End-to-end subprocess test** (in `lazyjob-ralph/tests/mock_interview_subprocess.rs`):

```rust
#[tokio::test]
async fn mock_interview_loop_completes_3_questions() {
    // Spawn MockInterviewLoop as a task with a pipe-backed stdin/stdout.
    // Use MockLlmProvider returning valid QuestionFeedback JSON.
    // Use a temporary sqlx::SqlitePool with migrations applied.
    // Send 3 WorkerCommand::UserInput events.
    // Assert 3 WorkerEvent::MockFeedback events received.
    // Assert 1 WorkerEvent::MockSessionSummary received.
    // Query mock_interview_sessions: assert completed_at IS NOT NULL.
    // Query mock_interview_responses: assert 3 rows with valid feedback_json.
}

#[tokio::test]
async fn mock_interview_loop_cancel_saves_partial_session() {
    // After sending 1 UserInput, send WorkerCommand::Cancel.
    // Assert session row exists with completed_at IS NULL and questions_answered = 1.
}
```

**TUI interaction test** (in `lazyjob-tui/tests/mock_interview_view.rs`):

```rust
#[test]
fn pressing_ctrl_enter_submits_answer_and_clears_input() {
    // Build AnswerInputWidget, type "test answer" via handle_input(),
    // trigger submission via key event, assert textarea is cleared.
}
```

---

## Open Questions

1. **Partial session resumability**: Phase 1 treats abandoned sessions as non-resumable artifacts. Phase 5 adds resume. Before implementing Phase 5, decide: should partial sessions be visible in the session list (as "Incomplete — 3/6 questions") or hidden until resumed? Lean toward visible to encourage users to finish, with a "(Resume)" action available.

2. **`example_stronger_answer` fabrication risk**: The spec flags this. Decision for Phase 1: `example_stronger_answer` is always structural coaching, never a full generated example. The evaluation prompt for behavioral questions must include the instruction: "In example_stronger_answer, provide only structural coaching (e.g., 'Add a quantified result for the impact — e.g., reduced X by Y%'). Do not write a full answer or invent specific metrics." If the LLM violates this, the fabrication_warning field provides a secondary check.

3. **System design structured template**: The current rubrics cover text-based system design responses under `QuestionCategory::SystemDesign` using the technical rubric (accuracy/depth/communication). For MVP this is acceptable. Post-MVP: add a `SystemDesignRubric` with sub-scores for requirements definition, scale estimation, component naming, and data model — but this requires a separate TOML template section and LLM prompt path.

4. **Session count display upfront**: Phase 1 should display "6 questions, ~20 min estimated" in the session header before the first question. Compute estimated time as `question_count * 3 minutes` (empirical constant). This is a TUI-only change with no LLM impact.

5. **Anti-overconfidence disclaimer UX**: The disclaimer appears in the session summary. Consider also showing a dimmed one-line version in the session header during the session itself. This is a one-liner TUI change — include it in Phase 4.

---

## Related Specs

- [specs/interview-prep-question-generation.md](./interview-prep-question-generation.md) — provides `InterviewQuestion`, `InterviewPrepSession`, `candidate_story_ref`
- [specs/agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) — NDJSON IPC, `WorkerCommand`, `WorkerEvent`, interactive worker handling
- [specs/agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) — `LoopType`, `LoopQueue`, `LoopDispatch`
- [specs/agentic-prompt-templates.md](./agentic-prompt-templates.md) — `RenderedPrompt`, `TemplateEngine`, Anthropic prompt caching
- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) — `LifeSheet`, `WorkExperience`, `SkillEntry`
- [specs/09-tui-design-keybindings.md](./09-tui-design-keybindings.md) — `App`, `EventLoop`, `KeyContext`, panel system
- [specs/16-privacy-security.md](./16-privacy-security.md) — `PrivacyMode::Stealth` suppresses session summary screenshots and LLM calls
