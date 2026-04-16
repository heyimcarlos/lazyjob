# Spec: Mock Interview Loop

**JTBD**: A-5 — Prepare for interviews systematically
**Topic**: Run an interactive mock interview session where the AI asks questions and gives scored feedback per response, including STAR method evaluation for behavioral questions.
**Domain**: interview-prep

---

## What

`MockInterviewLoop` is a ralph subprocess that runs an interactive, turn-based mock interview session. The user types responses to AI-generated questions; the LLM evaluates each response and provides structured feedback (STAR adherence, communication clarity, content gaps). After all questions are answered, the loop generates a session summary with per-category scores and specific improvement recommendations. Sessions are persisted to SQLite so the candidate can track progress over multiple practice rounds.

## Why

Existing mock interview tools are either peer-to-peer (Pramp — quality varies), professional-coached (Interviewing.io — limited availability and expensive), or narrowly focused on coding execution (LeetCode mock). None provide: (1) text-based STAR behavioral evaluation with ground truth checking against a candidate's actual story bank, (2) a session that covers behavioral + technical + culture fit in one flow, (3) persistent progress tracking across multiple practice sessions for the same role. The result is that candidates practice haphazardly and can't measure improvement. The mock loop solves the feedback and measurement gap for the terminal-native user who prefers async, text-based interaction.

## How

**Architecture: ralph subprocess (async, text-based)**

The mock loop runs as a ralph subprocess (`lazyjob-ralph` crate), communicating with the TUI via the established newline-delimited JSON IPC protocol. It is NOT a real-time streaming session — each exchange is one async round-trip: the subprocess emits a `RalphEvent::Question`, the TUI displays it and waits for user input, the user submits a response, the TUI sends it back via stdin, the subprocess evaluates and emits `RalphEvent::Feedback`.

This architecture was chosen over real-time voice because: (1) text-based responses are easier to evaluate rigorously, (2) no audio processing latency or privacy issues, (3) users can take their time composing a response (which is actually good interview prep discipline — composition forces clarity). The tradeoff is lower fidelity than a real spoken interview; this is explicitly acknowledged in the session UI.

**Session flow:**
```
1. Load InterviewPrepSession questions (from question_gen.md — pre-generated)
2. For each question:
   a. Emit RalphEvent::MockQuestion { question, category, tips }
   b. Wait for user response via stdin
   c. Evaluate response with LLM → QuestionFeedback
   d. Emit RalphEvent::MockFeedback { feedback }
   e. Store (question, response, feedback) to mock_interview_responses table
3. Compute SessionScore from all QuestionFeedback items
4. Emit RalphEvent::MockSessionSummary { session_score, improvements }
5. Persist MockInterviewSession to SQLite
```

**Evaluation rubrics by category:**

*Behavioral (STAR):*
- Situation (0-2): Did they set a clear, specific context?
- Task (0-2): Did they define what THEY needed to accomplish?
- Action (0-3): Did they describe specific, first-person actions (not "we did")? Did they avoid over-crediting the team?
- Result (0-3): Did they quantify impact? Did they reflect on what they learned?
- Total: 0–10 per question

*Technical:*
- Accuracy (0-4): Is the answer technically correct?
- Depth (0-3): Did they go beyond surface-level? Did they discuss tradeoffs?
- Communication (0-3): Did they explain their reasoning, not just state conclusions?
- Total: 0–10 per question

*Culture/Situational:*
- Authenticity signal (0-3): Does the response feel specific and genuine (not generic)?
- Values alignment (0-4): Does it match the company's stated culture signals?
- Completeness (0-3): Did they fully answer the scenario?
- Total: 0–10 per question

**Behavioral fabrication guard:** For behavioral questions where a `candidate_story_ref` is present (linked LifeSheet experience from question generation), the evaluator prompt includes the linked story's verified facts. If the candidate's typed response introduces claims not in the story (new numbers, different company, invented outcome), the evaluator flags it in `QuestionFeedback.fabrication_warning`. This is an advisory warning — not a hard block — but it must be visible in the UI.

**Anti-overconfidence note in UI:** The session summary must display: "AI feedback is an approximation. Practice with real humans for system design and live coding." This is not optional copy — it is a product commitment from the research findings (AI-graded responses can produce false confidence).

**Progress tracking:** `MockInterviewSession.per_category_score` is stored per session. The TUI can query multiple sessions for the same application and show a trend line (e.g., behavioral score improved from 6.2 → 8.1 over 3 sessions). This is a passive database query — no LLM needed for the trend view.

**Crate placement:** Loop logic lives in `lazyjob-ralph/src/loops/mock_interview.rs`. Evaluation types (`QuestionFeedback`, `SessionScore`, `MockInterviewSession`) live in `lazyjob-core/src/interview/mock_session.rs`. The LLM prompt templates for evaluation live in `lazyjob-llm/src/prompts/interview_eval.rs`.

## Interface

```rust
// lazyjob-core/src/interview/mock_session.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionFeedback {
    pub question_id: Uuid,
    pub score: u8,                  // 0-10
    pub score_breakdown: ScoreBreakdown,
    pub strengths: Vec<String>,     // specific observed strengths
    pub improvements: Vec<String>,  // specific, actionable improvements
    pub fabrication_warning: Option<String>, // set if response introduces unverified claims
    pub example_stronger_answer: Option<String>, // LLM-generated example, clearly labeled
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub situation: Option<u8>,  // STAR: only for Behavioral
    pub task: Option<u8>,
    pub action: Option<u8>,
    pub result: Option<u8>,
    pub accuracy: Option<u8>,   // Technical
    pub depth: Option<u8>,
    pub communication: u8,      // all categories
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockInterviewSession {
    pub id: Uuid,
    pub prep_session_id: Uuid,      // references InterviewPrepSession
    pub application_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub responses: Vec<MockResponse>,
    pub session_score: SessionScore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    pub question_id: Uuid,
    pub response_text: String,
    pub feedback: QuestionFeedback,
    pub answered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionScore {
    pub overall: f32,              // 0-10 weighted average
    pub behavioral_avg: Option<f32>,
    pub technical_avg: Option<f32>,
    pub culture_avg: Option<f32>,
    pub top_strength: String,
    pub top_improvement: String,
}

// lazyjob-ralph/src/loops/mock_interview.rs
pub struct MockInterviewLoop {
    pub prep_session_id: Uuid,
    pub application_id: Uuid,
}

// IPC events emitted during the mock loop
// (extends the RalphEvent enum in agentic-ralph-subprocess-protocol.md)
// RalphEvent::MockQuestion { question: InterviewQuestion }
// RalphEvent::MockFeedback { feedback: QuestionFeedback }
// RalphEvent::MockSessionSummary { session: MockInterviewSession }
```

**SQLite tables:**
```sql
CREATE TABLE mock_interview_sessions (
    id              TEXT PRIMARY KEY,
    prep_session_id TEXT NOT NULL REFERENCES interview_prep_sessions(id),
    application_id  TEXT NOT NULL REFERENCES applications(id),
    started_at      TEXT NOT NULL,
    completed_at    TEXT,
    session_score_json TEXT,
    overall_score   REAL   -- denormalized for trend queries without JSON parsing
);

CREATE TABLE mock_interview_responses (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES mock_interview_sessions(id),
    question_id     TEXT NOT NULL,
    response_text   TEXT NOT NULL,
    feedback_json   TEXT NOT NULL,
    answered_at     TEXT NOT NULL
);

CREATE INDEX idx_mock_sessions_application
    ON mock_interview_sessions(application_id, started_at);
```

## Open Questions

- **Partial sessions**: If the user quits mid-session (`:q` or process kill), should the session be saved as `completed_at = NULL` and resumable, or discarded? Resuming mid-session requires re-reading previous Q&A pairs back to the LLM for context — adds token cost and complexity.
- **Estimated question count displayed upfront**: Should the TUI show "6 questions, ~20 min" before starting the session? Users have abandoned session-based tools when they couldn't gauge time commitment.
- **System design evaluation**: The rubrics above handle behavioral and technical, but system design requires the candidate to draw or describe an architecture. In a text-based interface, the candidate types their design narrative. Is the current text-evaluation rubric sufficient, or do we need a separate system design structured template (e.g., "define requirements", "estimate scale", "name components")?
- **Example stronger answers**: `QuestionFeedback.example_stronger_answer` is LLM-generated. This is a potential fabrication surface — the "example" might invent metrics or company-specific details. Should we restrict example answers to structural coaching only (e.g., "Add a quantified result — e.g., 'reduced latency by X%'"), never generating a full invented example?

## Implementation Tasks

- [ ] Define `QuestionFeedback`, `MockResponse`, `MockInterviewSession`, `SessionScore` types in `lazyjob-core/src/interview/mock_session.rs`
- [ ] Implement `MockInterviewLoop` in `lazyjob-ralph/src/loops/mock_interview.rs` using the IPC protocol (emit `MockQuestion`, receive stdin response, emit `MockFeedback`) — refs: `agentic-ralph-subprocess-protocol.md`
- [ ] Implement evaluation prompt templates in `lazyjob-llm/src/prompts/interview_eval.rs` with per-category rubrics (STAR breakdown for behavioral, accuracy/depth/communication for technical) and fabrication detection when `candidate_story_ref` is set — refs: `agentic-prompt-templates.md`
- [ ] Create `mock_interview_sessions` and `mock_interview_responses` migration in `lazyjob-core/src/db/migrations/`
- [ ] Implement `MockInterviewRepository` with `save_session`, `get_sessions_for_application`, `get_score_trend` (returns `Vec<(DateTime, f32)>` for trend display) methods
- [ ] Add TUI mock interview view: sequential Q→A→feedback panels using the ralph event stream; include the anti-overconfidence disclaimer in the session header — refs: `architecture-tui-skeleton.md`
- [ ] Add progress trend panel in application detail view: query `MockInterviewRepository::get_score_trend` and display per-category score history across sessions
