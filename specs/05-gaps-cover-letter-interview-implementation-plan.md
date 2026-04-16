# Implementation Plan: Cover Letter & Interview Gap Closures (GAP-49 through GAP-58)

## Status
Draft

## Related Spec
[specs/05-gaps-cover-letter-interview.md](./05-gaps-cover-letter-interview.md)

## Overview

This plan closes ten identified gaps (GAP-49 through GAP-58) and three cross-spec gaps (K, L, M) that span the cover letter generation, interview preparation, salary data, and networking outreach subsystems. The gaps were identified by reviewing the full set of cover letter and interview prep specs against real user workflows.

The plan is organized into four phases. Phase 1 closes the two critical gaps (cover letter sent-state tracking and interview session resumability) because they represent data-loss and UX-regression risks in the MVP. Phase 2 closes four important gaps (real-time question aggregation, fatigue management, async video prep, system design evaluation). Phase 3 closes four moderate gaps (cover letter anti-ghosting, feedback aggregation, salary data freshness, warm personalization at scale). Phase 4 resolves the three cross-spec concerns (shared fabrication module, CompanyRecord dependency hardening, session↔application lifecycle).

All new code lives in `lazyjob-core` unless stated otherwise. SQLite migrations are numbered sequentially after the highest existing migration in the relevant area (migrations 020–026 are allocated here). All monetary values stored as `i64` cents, all timestamps as `TEXT` ISO-8601 UTC.

## Prerequisites

### Specs/Plans that must be implemented first
- `specs/profile-cover-letter-generation-implementation-plan.md` — `CoverLetterVersion`, `CoverLetterVersionRepository`, `FabricationLevel`
- `specs/interview-prep-mock-loop-implementation-plan.md` — `MockInterviewSession`, `MockInterviewLoop`, `QuestionFeedback`, `SessionScore`
- `specs/interview-prep-question-generation-implementation-plan.md` — `InterviewQuestion`, `PrepContext`, `InterviewPrepService`
- `specs/application-state-machine-implementation-plan.md` — `ApplicationStage`, `ApplicationRepository`
- `specs/job-search-company-research-implementation-plan.md` — `CompanyRecord`, `CompanyRepository`
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `LifeSheet`, `is_grounded_claim()`
- `specs/salary-market-intelligence-implementation-plan.md` — `MarketData`, `MarketDataRepository`
- `specs/networking-outreach-drafting-implementation-plan.md` — `OutreachDraft`, `SharedContext`, `ProfileContact`

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml (additions)
[dependencies]
# Cross-session pattern detection
itertools            = "0.13"          # window() and group_by() for trend analysis

# Cover letter genericness detection
edit-distance        = "2"             # Levenshtein for pairwise phrase comparison

# Salary freshness: already have chrono, no new deps

# System design structured evaluation: reuse existing LLM provider

# All others: regex, once_cell, strsim, sha2, similar — already required by prior plans
```

---

## Architecture

### Crate Placement

| Crate | New Responsibility |
|-------|-------------------|
| `lazyjob-core` | All new domain logic (sent-state, resumability, fatigue, anti-ghosting, feedback aggregation, salary freshness, fabrication oracle) |
| `lazyjob-core/src/fabrication.rs` | Canonical shared `FabricationLevel` enum + detection logic (Cross-Spec K) |
| `lazyjob-tui` | Updated CL version browser, session resume picker, fatigue banner, system design eval panel |
| `lazyjob-ralph` | `LoopType::SystemDesignEval` subprocess (Phase 2) |

---

## Phase 1 — Critical Gap Closures

### GAP-49: Cover Letter Sent-State and Version Management

#### 1.1 Extend CoverLetterVersion with sent-state

```rust
// lazyjob-core/src/cover_letter/types.rs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum CoverLetterSentStatus {
    Draft,
    Sent,
    Archived,
}

/// Channel via which the letter was submitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionChannel {
    Email,
    LinkedInEasyApply,
    CompanyPortal,
    InPerson,
    Other(String),
}

/// A single version of a cover letter tied to one job application.
pub struct CoverLetterVersion {
    pub id: CoverLetterVersionId,
    pub job_id: JobId,
    pub application_id: Option<ApplicationId>,
    pub version_number: u32,
    pub label: Option<String>,                   // user-defined: "Engineering Manager v2"
    pub body_text: String,
    pub word_count: u32,
    pub sent_status: CoverLetterSentStatus,
    pub sent_at: Option<DateTime<Utc>>,
    pub channel: Option<SubmissionChannel>,      // serialized as JSON in SQLite
    pub paired_resume_version_id: Option<ResumeVersionId>,
    pub diff_from_prev: Option<String>,          // unified diff TEXT
    pub content_hash: String,                    // SHA-256 of body_text
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

#### 1.2 SQLite migration 020 — sent-state columns

```sql
-- migrations/020_cover_letter_sent_state.sql
ALTER TABLE cover_letter_versions ADD COLUMN label TEXT;
ALTER TABLE cover_letter_versions ADD COLUMN sent_status TEXT NOT NULL DEFAULT 'draft'
    CHECK (sent_status IN ('draft', 'sent', 'archived'));
ALTER TABLE cover_letter_versions ADD COLUMN sent_at TEXT;  -- ISO-8601 UTC or NULL
ALTER TABLE cover_letter_versions ADD COLUMN channel TEXT;  -- JSON or NULL
ALTER TABLE cover_letter_versions ADD COLUMN paired_resume_version_id TEXT
    REFERENCES resume_versions(id) ON DELETE SET NULL;

-- Index for "which letters are sent?" query
CREATE INDEX IF NOT EXISTS idx_clv_sent_status
    ON cover_letter_versions(job_id, sent_status);
```

#### 1.3 Repository operations

```rust
// lazyjob-core/src/cover_letter/repository.rs (additions)

#[async_trait]
pub trait CoverLetterVersionRepository: Send + Sync {
    // ... existing methods ...

    /// Mark version as sent, recording channel and linked resume.
    async fn mark_sent(
        &self,
        id: CoverLetterVersionId,
        channel: SubmissionChannel,
        paired_resume_version_id: Option<ResumeVersionId>,
    ) -> Result<(), CoverLetterError>;

    /// Set a human-readable label for a version.
    async fn set_label(
        &self,
        id: CoverLetterVersionId,
        label: String,
    ) -> Result<(), CoverLetterError>;

    /// Archive all non-sent versions for a job when one is marked sent.
    async fn archive_unsent_versions_for_job(
        &self,
        job_id: JobId,
        except: CoverLetterVersionId,
    ) -> Result<u32, CoverLetterError>;  // returns archived count

    /// List all versions for a job sorted by version_number desc.
    async fn list_by_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<CoverLetterVersion>, CoverLetterError>;

    /// Compute unified diff between two consecutive versions.
    fn compute_diff(prev: &str, next: &str) -> String {
        use similar::{ChangeTag, TextDiff};
        let diff = TextDiff::from_lines(prev, next);
        diff.unified_diff()
            .header("v_prev", "v_next")
            .to_string()
    }
}
```

`mark_sent` runs a single SQLite transaction: updates the row's `sent_status`, `sent_at`, `channel`, and `paired_resume_version_id`, then calls `archive_unsent_versions_for_job` within the same transaction.

#### 1.4 TUI: Version History Browser

```
// lazyjob-tui/src/cover_letter/version_browser.rs

// Layout: 30% left pane (version list) / 70% right pane (diff or full text)

struct VersionBrowserState {
    versions: Vec<CoverLetterVersion>,
    selected_idx: usize,
    view_mode: VersionViewMode,
}

enum VersionViewMode {
    FullText,
    DiffFromPrev,
}

// Keybinds (Normal mode):
//   j/k    — navigate version list
//   d      — toggle diff view
//   s      — mark selected as sent (opens channel picker dialog)
//   l      — set label (opens inline text input)
//   Enter  — open in editor
//   q      — close
```

The sent-state column in the list renders as:
- `[SENT]` in green bold if `sent_status == Sent`
- `[draft]` in dim if `Draft`
- `-` in dark gray if `Archived`

**Verification:** `cargo test cover_letter::version_management` — tests include: `mark_sent_archives_other_versions`, `diff_computed_correctly`, `version_list_sorted_desc`.

---

### GAP-50: Interview Session Resumability

#### 2.1 Checkpoint table and types

```rust
// lazyjob-core/src/interview/session_checkpoint.rs

pub struct SessionCheckpoint {
    pub id: Uuid,
    pub session_id: MockInterviewSessionId,
    pub checkpoint_idx: u32,       // 0-based question index at save point
    pub qa_history_json: String,   // JSON array of CompletedTurn
    pub token_cost_estimate: u32,  // tokens needed to resume (for UX display)
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>, // created_at + 48 hours
}

/// A completed Q&A turn serialized into the checkpoint.
#[derive(Serialize, Deserialize)]
pub struct CompletedTurn {
    pub question: InterviewQuestion,
    pub user_response: String,
    pub feedback: QuestionFeedback,
}
```

```sql
-- migrations/021_interview_session_checkpoints.sql
CREATE TABLE IF NOT EXISTS interview_session_checkpoints (
    id                  TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL REFERENCES mock_interview_sessions(id) ON DELETE CASCADE,
    checkpoint_idx      INTEGER NOT NULL,
    qa_history_json     TEXT NOT NULL,
    token_cost_estimate INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_isc_session
    ON interview_session_checkpoints(session_id, created_at DESC);

-- Update mock_interview_sessions to track resumption state
ALTER TABLE mock_interview_sessions ADD COLUMN is_partial INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mock_interview_sessions ADD COLUMN resumed_from_checkpoint_id TEXT
    REFERENCES interview_session_checkpoints(id) ON DELETE SET NULL;
```

#### 2.2 Checkpoint persistence in MockInterviewLoop

The existing `MockInterviewLoop` (`lazyjob-ralph` subprocess) must call `checkpoint_after_question()` after each question's feedback is finalized:

```rust
// lazyjob-core/src/interview/mock_loop_checkpointing.rs

pub struct SessionCheckpointer {
    pool: SqlitePool,
}

impl SessionCheckpointer {
    /// Called after each question completes. Overwrites previous checkpoint
    /// for this session (only the latest is kept).
    pub async fn save(&self, session_id: MockInterviewSessionId, turns: &[CompletedTurn])
        -> Result<SessionCheckpoint, InterviewError>
    {
        let history_json = serde_json::to_string(turns)?;
        let token_estimate = turns.iter()
            .map(|t| estimate_tokens_for_turn(t))
            .sum::<u32>();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(48);

        // Upsert: ON CONFLICT(session_id) — only one checkpoint per session
        sqlx::query!(
            r#"
            INSERT INTO interview_session_checkpoints
                (id, session_id, checkpoint_idx, qa_history_json, token_cost_estimate,
                 created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(session_id) DO UPDATE SET
                checkpoint_idx      = excluded.checkpoint_idx,
                qa_history_json     = excluded.qa_history_json,
                token_cost_estimate = excluded.token_cost_estimate,
                created_at          = excluded.created_at,
                expires_at          = excluded.expires_at
            "#,
            Uuid::new_v4().to_string(), session_id.0.to_string(),
            turns.len() as i64, history_json, token_estimate as i64,
            now.to_rfc3339(), expires_at.to_rfc3339(),
        ).execute(&self.pool).await?;
        // ...
    }

    /// Estimate tokens for a completed turn (question + response + feedback JSON).
    fn estimate_tokens_for_turn(turn: &CompletedTurn) -> u32 {
        // Rough: 1 token ≈ 4 chars
        let chars = turn.question.text.len()
            + turn.user_response.len()
            + serde_json::to_string(&turn.feedback).unwrap_or_default().len();
        (chars / 4) as u32
    }

    /// Load latest checkpoint for a session if not expired.
    pub async fn load(&self, session_id: MockInterviewSessionId)
        -> Result<Option<SessionCheckpoint>, InterviewError>
    {
        // Selects only if expires_at > now()
    }

    /// Delete expired checkpoints (called at TUI startup).
    pub async fn prune_expired(&self) -> Result<u64, InterviewError>;
}
```

#### 2.3 Session Resume UX

```rust
// lazyjob-tui/src/interview/session_resume_picker.rs

// Displayed when user opens mock interview and a valid checkpoint exists:
//
//  ┌─ Resume Session? ────────────────────────────────────────────────────┐
//  │  You have a partial session from 2h ago:                             │
//  │    Company: Stripe  •  Role: Backend Engineer  •  5 of 8 questions   │
//  │    Est. resume cost: ~520 tokens                                     │
//  │                                                                      │
//  │  [r] Resume   [n] New session   [d] Discard checkpoint               │
//  └──────────────────────────────────────────────────────────────────────┘

enum ResumeDialogAction {
    Resume,
    NewSession,
    DiscardAndNew,
}
```

When user chooses Resume:
1. `SessionCheckpointer::load()` fetches the `CompletedTurn` history
2. `MockInterviewLoop` is started with `WorkerParams::Resume { qa_history: Vec<CompletedTurn>, next_question_idx: usize }`
3. The loop injects prior Q&A as system-context messages before the next question prompt
4. `mock_interview_sessions.is_partial = 0` is set when the session completes

**Verification:** `cargo test interview::session_resumability` — tests: `checkpoint_saved_after_each_question`, `expired_checkpoint_not_loaded`, `resumed_session_continues_from_correct_idx`.

---

## Phase 2 — Important Gap Closures

### GAP-51: Real-Time Company Interview Question Aggregation

#### 3.1 Design decision

Direct scraping of Blind/Glassdoor is ToS-violating. Instead:

1. **User-paste flow**: User pastes raw text from Glassdoor/Blind/LeetCode Discuss into a TUI input box. LazyJob parses and stores it as `UserSourcedInterviewSignal` — this is explicitly user-owned data.
2. **Freshness scoring**: Signals have a `reported_year: Option<u16>` field and a `staleness_score()` method.
3. **Aggregation**: `InterviewSignalAggregator::aggregate()` merges programmatic signals (from `CompanyRecord.interview_signals`) with user-sourced signals, deduplicating by question similarity.

```rust
// lazyjob-core/src/interview/signals.rs

pub struct UserSourcedInterviewSignal {
    pub id: Uuid,
    pub company_id: CompanyId,
    pub raw_text: String,
    pub questions_extracted: Vec<ExtractedQuestion>,
    pub source_label: String,       // "Glassdoor", "Blind", "LeetCode", "friend"
    pub reported_year: Option<u16>,
    pub imported_at: DateTime<Utc>,
}

pub struct ExtractedQuestion {
    pub text: String,
    pub category: Option<QuestionCategory>,
    pub reported_year: Option<u16>,
}

impl ExtractedQuestion {
    /// 0.0 (fresh) to 1.0 (very stale).
    pub fn staleness_score(&self) -> f32 {
        let current_year = chrono::Utc::now().year() as u16;
        match self.reported_year {
            None => 0.5,  // unknown age → neutral
            Some(y) => ((current_year.saturating_sub(y)) as f32 / 3.0).min(1.0),
        }
    }
}
```

```sql
-- migrations/022_user_sourced_interview_signals.sql
CREATE TABLE IF NOT EXISTS user_sourced_interview_signals (
    id              TEXT PRIMARY KEY,
    company_id      TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    raw_text        TEXT NOT NULL,
    questions_json  TEXT NOT NULL,  -- JSON array of ExtractedQuestion
    source_label    TEXT NOT NULL,
    reported_year   INTEGER,
    imported_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_usis_company
    ON user_sourced_interview_signals(company_id, imported_at DESC);
```

LLM extraction prompt (Phase 2 of question-generation plan can call this):

```
System: You are an interview question extractor. Return JSON only.
User: Extract interview questions from the following pasted text.
For each question return: {"text": "...", "category": "behavioral|technical|culture|system_design|unknown", "reported_year": <int or null>}
Text: {raw_text}
```

#### 3.2 TUI: Paste-import flow

In the company detail panel, pressing `I` opens a multi-line text paste box. On submit, `InterviewSignalImporter::import()` calls the LLM extractor and saves to SQLite.

---

### GAP-52: Interview Fatigue Management

#### 4.1 Types and scoring

```rust
// lazyjob-core/src/interview/fatigue.rs

pub struct FatigueReport {
    pub sessions_last_7_days: u32,
    pub sessions_today: u32,
    pub avg_score_last_3: f32,
    pub avg_score_prev_3: f32,
    pub trend: ScoreTrend,
    pub recommendation: FatigueRecommendation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreTrend {
    Improving,
    Stable,
    Declining,
    Insufficient,   // fewer than 3 sessions to compare
}

#[derive(Debug, Clone)]
pub enum FatigueRecommendation {
    Rest { reason: &'static str },
    Continue,
    LightSession { max_questions: u8 },
}

pub struct FatigueAnalyzer;

impl FatigueAnalyzer {
    /// All logic is pure: takes completed sessions as input, no I/O.
    pub fn analyze(sessions: &[SessionSummary]) -> FatigueReport {
        let today = chrono::Utc::now().date_naive();
        let week_ago = today - chrono::Duration::days(7);

        let sessions_today = sessions.iter()
            .filter(|s| s.completed_at.date_naive() == today)
            .count() as u32;

        let sessions_last_7 = sessions.iter()
            .filter(|s| s.completed_at.date_naive() >= week_ago)
            .count() as u32;

        let last_3: Vec<f32> = sessions.iter()
            .rev()
            .take(3)
            .map(|s| s.overall_score)
            .collect();
        let prev_3: Vec<f32> = sessions.iter()
            .rev()
            .skip(3)
            .take(3)
            .map(|s| s.overall_score)
            .collect();

        let avg_last_3 = if last_3.len() >= 3 {
            last_3.iter().sum::<f32>() / last_3.len() as f32
        } else { 0.0 };
        let avg_prev_3 = if prev_3.len() >= 3 {
            prev_3.iter().sum::<f32>() / prev_3.len() as f32
        } else { 0.0 };

        let trend = if last_3.len() < 3 || prev_3.len() < 3 {
            ScoreTrend::Insufficient
        } else if avg_last_3 > avg_prev_3 + 0.5 {
            ScoreTrend::Improving
        } else if avg_last_3 < avg_prev_3 - 0.5 {
            ScoreTrend::Declining
        } else {
            ScoreTrend::Stable
        };

        let recommendation = if sessions_today >= 3 {
            FatigueRecommendation::Rest { reason: "3+ sessions today — diminishing returns" }
        } else if matches!(trend, ScoreTrend::Declining) && sessions_last_7 >= 5 {
            FatigueRecommendation::Rest { reason: "Score declining with high volume — rest helps" }
        } else if sessions_today == 2 {
            FatigueRecommendation::LightSession { max_questions: 3 }
        } else {
            FatigueRecommendation::Continue
        };

        FatigueReport {
            sessions_last_7_days: sessions_last_7,
            sessions_today,
            avg_score_last_3: avg_last_3,
            avg_score_prev_3: avg_prev_3,
            trend,
            recommendation,
        }
    }
}
```

#### 4.2 TUI integration

The `FatigueReport` is computed before opening the mock interview flow. When `recommendation` is `Rest` or `LightSession`, a dismissable banner is shown at the top of the mock interview launcher:

```
 ⚠  You've had 3 sessions today. Consider a rest — performance typically drops after this.
    [c] Continue anyway   [q] Return to main
```

Recommendation is informational only — the user can always proceed. `max_questions` for `LightSession` is suggested (shown in the UI) but not enforced.

---

### GAP-53: Async Video Interview Preparation

#### 5.1 Scope decision

Full video analysis (HireVue-style) is out of scope for the CLI/TUI binary. This implementation delivers:
1. **Platform format guide**: structured data for common async platforms (HireVue, Spark Hire, Pillar)
2. **Text-response practice**: user types a response to a timed prompt; LazyJob evaluates it with the existing `MockInterviewLoop` evaluation rubric
3. **Timing coach**: countdown timer displayed during practice

```rust
// lazyjob-core/src/interview/async_format.rs

#[derive(Debug, Clone)]
pub struct AsyncPlatformFormat {
    pub platform: AsyncPlatform,
    pub think_time_secs: u32,       // seconds to think before recording
    pub response_time_secs: u32,    // max response time
    pub retakes_allowed: u32,
    pub typical_question_count: u32,
    pub prep_tips: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncPlatform {
    HireVue,
    SparkHire,
    Pillar,
    Generic,
}

impl AsyncPlatformFormat {
    pub fn for_platform(p: &AsyncPlatform) -> Self {
        match p {
            AsyncPlatform::HireVue => Self {
                platform: p.clone(),
                think_time_secs: 30,
                response_time_secs: 180,
                retakes_allowed: 0,
                typical_question_count: 5,
                prep_tips: &[
                    "Look directly at the camera, not the preview",
                    "Use STAR structure even for short responses",
                    "Keep answers to 90–120 seconds",
                ],
            },
            // ... SparkHire, Pillar, Generic
        }
    }
}
```

The TUI adds an "Async Video Prep" sub-mode in the interview prep panel that shows the platform format and runs a timed text practice round using the existing evaluation engine.

---

### GAP-54: Whiteboard System Design Evaluation

#### 6.1 Evaluation rubric

```rust
// lazyjob-core/src/interview/system_design.rs

pub struct SystemDesignRubric {
    pub requirements_elicitation: RubricScore,   // 0-10: did candidate clarify scope?
    pub capacity_estimation: RubricScore,        // 0-10: reasonable scale estimates?
    pub high_level_design: RubricScore,          // 0-10: named components, responsibilities?
    pub deep_dive: RubricScore,                  // 0-10: at least one component explored deeply?
    pub trade_off_awareness: RubricScore,        // 0-10: CAP theorem, SQL vs NoSQL, etc.?
    pub seniority_appropriateness: RubricScore,  // 0-10: right scope for stated level?
    pub overall: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct RubricScore {
    pub score: u8,   // 0-10
    pub rationale: &'static str,  // placeholder; LLM fills this at runtime
}

/// Seniority scaling constants used in the evaluation prompt.
pub const SENIOR_SCOPE_GUIDANCE: &str =
    "Expect a complete system design with 3+ components, \
     at least one deep dive, and awareness of scalability trade-offs.";
pub const STAFF_SCOPE_GUIDANCE: &str =
    "Expect org-level thinking, multi-system interactions, \
     platform constraints, and engineering culture implications.";
```

#### 6.2 SystemDesignEvalLoop (Ralph subprocess)

`LoopType::SystemDesignEval` is a non-interactive single-shot loop:

1. User types their design response in a TUI editor (multi-line `TextArea` widget)
2. On submit, spawns `LoopType::SystemDesignEval` with the text
3. Prompt includes: the original question, the user's response, the `seniority_level`, and the rubric criteria
4. LLM returns structured JSON matching `SystemDesignRubric`
5. TUI renders a scored rubric table with collapsible rationale lines

**Follow-up simulation** (Phase 2): After initial evaluation, a follow-up question is generated based on the deepest-scored component (`deep_dive` score) and the user can respond to it as a second turn.

```rust
// lazyjob-ralph/src/loops/system_design.rs

pub struct SystemDesignEvalParams {
    pub question: String,
    pub user_response: String,
    pub seniority: SeniorityLevel,
    pub role_context: String,
}

pub enum SeniorityLevel {
    Junior,
    Mid,
    Senior,
    Staff,
    Principal,
}

impl SeniorityLevel {
    pub fn scope_guidance(&self) -> &'static str {
        match self {
            Self::Senior => SENIOR_SCOPE_GUIDANCE,
            Self::Staff | Self::Principal => STAFF_SCOPE_GUIDANCE,
            _ => "Expect a basic design with clear component separation.",
        }
    }
}
```

---

## Phase 3 — Moderate Gap Closures

### GAP-55: Cover Letter Anti-Ghosting Detection

#### 7.1 Genericness detector

```rust
// lazyjob-core/src/cover_letter/anti_ghosting.rs

use once_cell::sync::Lazy;
use regex::Regex;

/// Phrases that mark AI-generated letter genericness.
static GENERIC_PHRASES: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    [
        ("passion_opener",      r"(?i)\bI(?:'m| am) (passionate|excited|thrilled) about"),
        ("company_fit_cliche",  r"(?i)\byour (company|organization|team)'s (mission|vision|values)"),
        ("humble_opener",       r"(?i)\bI am writing to express"),
        ("contribute_cliche",   r"(?i)\bcontribute (my|to) (skills|experience|expertise)"),
        ("opportunity_cliche",  r"(?i)\bthis (role|position|opportunity) aligns with"),
    ]
    .into_iter()
    .map(|(name, pat)| (name, Regex::new(pat).unwrap()))
    .collect()
});

pub struct GenericnessReport {
    pub triggered_phrases: Vec<&'static str>,
    pub genericness_score: f32,          // 0.0 clean to 1.0 very generic
    pub has_company_specific_ref: bool,  // mentions at least one fact from CompanyRecord
    pub has_personal_story_ref: bool,    // references at least one LifeSheet achievement
    pub recommendation: String,
}

pub struct AntiGhostingDetector;

impl AntiGhostingDetector {
    pub fn analyze(body: &str, company: &CompanyRecord, life_sheet: &LifeSheet) -> GenericnessReport {
        let triggered: Vec<&'static str> = GENERIC_PHRASES
            .iter()
            .filter(|(_, re)| re.is_match(body))
            .map(|(name, _)| *name)
            .collect();

        let genericness_score = triggered.len() as f32 / GENERIC_PHRASES.len() as f32;

        // Check company-specific reference: does body mention company.name or any product_name?
        let has_company_ref = {
            let name_lower = company.name.to_lowercase();
            let body_lower = body.to_lowercase();
            body_lower.contains(&name_lower)
                || company.products.iter().any(|p| body_lower.contains(&p.to_lowercase()))
        };

        // Check personal story: does body mention any achievement.metric_value or key phrase?
        let has_story_ref = life_sheet.experiences.iter()
            .flat_map(|e| &e.achievements)
            .any(|a| {
                a.metric_value.as_deref()
                    .map(|m| body.contains(m))
                    .unwrap_or(false)
                    || a.headline.split_whitespace()
                        .filter(|w| w.len() > 5)
                        .any(|w| body.to_lowercase().contains(&w.to_lowercase()))
            });

        let recommendation = if genericness_score > 0.4 {
            format!(
                "Cover letter contains {} generic phrases. Consider replacing: {}",
                triggered.len(),
                triggered.join(", ")
            )
        } else {
            "Letter appears authentic.".to_string()
        };

        GenericnessReport {
            triggered_phrases: triggered,
            genericness_score,
            has_company_specific_ref: has_company_ref,
            has_personal_story_ref: has_story_ref,
            recommendation,
        }
    }
}
```

`AntiGhostingDetector::analyze()` is called by `CoverLetterService::generate()` after fabrication check. `GenericnessReport` is included in the draft preview sidebar alongside `FabricationLevel`.

---

### GAP-56: Interview Feedback Aggregation and Pattern Detection

#### 8.1 Cross-session analytics

```rust
// lazyjob-core/src/interview/analytics.rs

pub struct FeedbackPattern {
    pub category: QuestionCategory,
    pub avg_score: f32,
    pub session_count: u32,
    pub trend: ScoreTrend,
    pub lowest_sub_score: SubScoreType,  // e.g., "result" in STAR
}

pub struct PrepAnalyticsReport {
    pub total_sessions: u32,
    pub date_range: (DateTime<Utc>, DateTime<Utc>),
    pub patterns_by_category: Vec<FeedbackPattern>,
    pub weakest_category: Option<QuestionCategory>,
    pub improvement_velocity: f32,  // points per session, negative = declining
    pub spaced_repetition_suggestions: Vec<QuestionCategory>,
    pub readiness_score: u8,  // 0-100, aggregated from recent session scores
}

pub struct PrepAnalyticsService {
    pool: SqlitePool,
}

impl PrepAnalyticsService {
    /// Compute full analytics report from stored session data.
    /// All DB queries then pure Rust computation — no LLM.
    pub async fn compute_report(
        &self,
        application_id: Option<ApplicationId>,
    ) -> Result<PrepAnalyticsReport, InterviewError>
    {
        let sessions = self.load_sessions(application_id).await?;
        let all_feedback = self.load_all_feedback(&sessions).await?;
        Ok(Self::compute_pure(&sessions, &all_feedback))
    }

    fn compute_pure(
        sessions: &[SessionSummary],
        feedback: &[QuestionFeedback],
    ) -> PrepAnalyticsReport {
        use itertools::Itertools;

        let patterns: Vec<FeedbackPattern> = feedback
            .iter()
            .into_group_map_by(|f| f.category)
            .into_iter()
            .map(|(cat, items)| {
                let scores: Vec<f32> = items.iter().map(|i| i.score as f32).collect();
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                let trend = FatigueAnalyzer::compute_trend(&scores); // reuse
                FeedbackPattern {
                    category: cat,
                    avg_score: avg,
                    session_count: items.len() as u32,
                    trend,
                    lowest_sub_score: Self::find_lowest_sub_score(&items),
                }
            })
            .collect();

        let weakest = patterns.iter()
            .min_by(|a, b| a.avg_score.partial_cmp(&b.avg_score).unwrap())
            .map(|p| p.category);

        // Spaced repetition: categories with avg < 6.0 and trend Stable/Declining
        let suggestions: Vec<QuestionCategory> = patterns.iter()
            .filter(|p| p.avg_score < 6.0 && !matches!(p.trend, ScoreTrend::Improving))
            .map(|p| p.category)
            .collect();

        // Readiness: weighted average of last 2 sessions' overall scores
        let readiness = if sessions.len() >= 2 {
            let last2_avg = sessions.iter().rev().take(2)
                .map(|s| s.overall_score)
                .sum::<f32>() / 2.0;
            (last2_avg * 10.0).min(100.0) as u8
        } else {
            0
        };

        PrepAnalyticsReport {
            total_sessions: sessions.len() as u32,
            date_range: (
                sessions.first().map(|s| s.completed_at).unwrap_or_else(Utc::now),
                sessions.last().map(|s| s.completed_at).unwrap_or_else(Utc::now),
            ),
            patterns_by_category: patterns,
            weakest_category: weakest,
            improvement_velocity: Self::compute_velocity(sessions),
            spaced_repetition_suggestions: suggestions,
            readiness_score: readiness,
        }
    }
}
```

TUI: A `PrepAnalyticsDashboard` panel shows category scores as a ratatui `BarChart`, trend arrows, weakest-link highlight in red, and a `Readiness: 72/100` gauge at the top.

---

### GAP-57: Salary Data Freshness and Staleness

#### 9.1 Extend MarketData with freshness metadata

```rust
// lazyjob-core/src/salary/market_data.rs (additions)

pub struct MarketData {
    // ... existing fields ...
    pub source: SalaryDataSource,
    pub collected_at: DateTime<Utc>,     // when LazyJob fetched/imported this
    pub data_as_of: Option<NaiveDate>,   // reported date from the source
    pub sample_count: u32,
    pub confidence: DataConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DataConfidence {
    Low,     // sample_count < 5
    Medium,  // 5-19 samples
    High,    // 20+ samples
}

impl MarketData {
    /// Age in days since collection.
    pub fn age_days(&self) -> i64 {
        (Utc::now() - self.collected_at).num_days()
    }

    /// Whether data is fresh enough for a given use case.
    pub fn is_fresh_for(&self, purpose: SalaryDataPurpose) -> bool {
        let max_age_days: i64 = match purpose {
            SalaryDataPurpose::NegotiationSupport => 30,
            SalaryDataPurpose::RoleResearch      => 90,
            SalaryDataPurpose::MarketExploration => 180,
        };
        self.age_days() <= max_age_days
    }

    /// Human-readable staleness label for TUI display.
    pub fn freshness_label(&self) -> &'static str {
        match self.age_days() {
            0..=7   => "Fresh",
            8..=30  => "Recent",
            31..=90 => "Aging",
            _       => "Stale",
        }
    }
}

pub enum SalaryDataPurpose {
    NegotiationSupport,
    RoleResearch,
    MarketExploration,
}
```

```sql
-- migrations/023_salary_data_freshness.sql
ALTER TABLE market_data ADD COLUMN data_as_of TEXT;          -- ISO date or NULL
ALTER TABLE market_data ADD COLUMN collected_at TEXT NOT NULL DEFAULT (datetime('now'));
ALTER TABLE market_data ADD COLUMN sample_count INTEGER NOT NULL DEFAULT 0;
```

TUI salary comparison view: each market data row shows a colored `freshness_label()` badge (`Fresh`=green, `Recent`=yellow, `Aging`=orange, `Stale`=red) and a `DataConfidence` indicator (`●●●` / `●●○` / `●○○`). When `is_fresh_for(NegotiationSupport)` is false, a warning banner appears: `⚠ Market data is 45 days old — verify before using in negotiation`.

---

### GAP-58: Networking Outreach Warm Personalization at Scale

#### 10.1 Personalization depth scoring

```rust
// lazyjob-core/src/networking/personalization.rs

pub struct PersonalizationScore {
    pub mutual_connection_mentioned: bool,
    pub shared_employer_mentioned: bool,
    pub shared_school_mentioned: bool,
    pub recent_work_referenced: bool,    // any CompanyRecord or contact blog post
    pub depth: PersonalizationDepth,
    pub score: u8,   // 0-100
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PersonalizationDepth {
    Cold,       // score 0-24: only name substitution
    Warm,       // score 25-59: shared context (employer/school)
    Hot,        // score 60-84: mutual connection + context
    Deep,       // score 85-100: recent work + specific project mention
}

pub struct PersonalizationScorer;

impl PersonalizationScorer {
    /// Pure sync function — no I/O.
    pub fn score(
        draft: &str,
        contact: &ProfileContact,
        shared_ctx: &SharedContext,
        company: &CompanyRecord,
    ) -> PersonalizationScore {
        let mut points: u8 = 0;

        let mutual = shared_ctx.mutual_connections.iter()
            .any(|m| draft.to_lowercase().contains(&m.to_lowercase()));
        if mutual { points = points.saturating_add(25); }

        let shared_employer = shared_ctx.shared_employers.iter()
            .any(|e| draft.to_lowercase().contains(&e.to_lowercase()));
        if shared_employer { points = points.saturating_add(20); }

        let shared_school = shared_ctx.shared_schools.iter()
            .any(|s| draft.to_lowercase().contains(&s.to_lowercase()));
        if shared_school { points = points.saturating_add(15); }

        // Recent work: any reference to a company product or recent news headline
        let recent_work = company.products.iter()
            .chain(&company.recent_news_headlines)
            .any(|item| draft.to_lowercase().contains(&item.to_lowercase()));
        if recent_work { points = points.saturating_add(30); }

        // Base: name mentioned → always true for well-formed message
        if draft.contains(&contact.name) { points = points.saturating_add(10); }

        let depth = match points {
            0..=24  => PersonalizationDepth::Cold,
            25..=59 => PersonalizationDepth::Warm,
            60..=84 => PersonalizationDepth::Hot,
            _       => PersonalizationDepth::Deep,
        };

        PersonalizationScore {
            mutual_connection_mentioned: mutual,
            shared_employer_mentioned: shared_employer,
            shared_school_mentioned: shared_school,
            recent_work_referenced: recent_work,
            depth,
            score: points,
        }
    }
}
```

#### 10.2 Scale limiting and quality gate

```rust
// lazyjob-core/src/networking/scale_guard.rs

pub const DAILY_OUTREACH_LIMIT: u32 = 10;
pub const COLD_OUTREACH_DAILY_LIMIT: u32 = 3;  // Cold messages need more care

pub struct ScaleGuardReport {
    pub sent_today: u32,
    pub cold_sent_today: u32,
    pub gate_result: ScaleGateResult,
}

pub enum ScaleGateResult {
    Allow,
    Warn { message: &'static str },
    SoftBlock { reason: &'static str },  // user can override
}

impl ScaleGuardReport {
    pub fn evaluate(sent_today: u32, cold_today: u32, depth: &PersonalizationDepth) -> Self {
        let gate = if matches!(depth, PersonalizationDepth::Cold) && cold_today >= COLD_OUTREACH_DAILY_LIMIT {
            ScaleGateResult::SoftBlock { reason: "3 cold messages today — quality typically drops beyond this" }
        } else if sent_today >= DAILY_OUTREACH_LIMIT {
            ScaleGateResult::SoftBlock { reason: "10 outreach messages today — consider pacing for quality" }
        } else if sent_today >= 7 {
            ScaleGateResult::Warn { message: "7+ messages today — ensure each is genuinely personalized" }
        } else {
            ScaleGateResult::Allow
        };
        ScaleGuardReport { sent_today, cold_sent_today: cold_today, gate_result: gate }
    }
}
```

`PersonalizationScore` and `ScaleGuardReport` are displayed in the outreach draft sidebar before the user marks a draft as sent. `SoftBlock` renders a yellow warning with an `[o] Override` keybind.

---

## Phase 4 — Cross-Spec Concern Resolutions

### Cross-Spec K: Shared Fabrication Detection Module

All three specs that define `FabricationLevel` (cover letter, resume tailoring, mock interview) must be unified under a single module:

```rust
// lazyjob-core/src/fabrication.rs  ← NEW canonical location

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FabricationLevel {
    Safe,        // claim verified against LifeSheet
    Acceptable,  // claim plausible but unverifiable
    Risky,       // claim unverifiable and metric-specific
    Forbidden,   // competing offer fabrication, numbers pulled from thin air
}

pub struct FabricationFinding {
    pub claim_text: String,
    pub level: FabricationLevel,
    pub reason: &'static str,
}

pub struct FabricationReport {
    pub findings: Vec<FabricationFinding>,
    pub max_level: FabricationLevel,
    pub blocked: bool,   // true if max_level >= Forbidden
}

pub struct FabricationOracle<'a> {
    pub life_sheet: &'a LifeSheet,
}

impl<'a> FabricationOracle<'a> {
    /// Run all tiers of claim verification on generated text.
    pub fn analyze(&self, text: &str) -> FabricationReport {
        let mut findings = Vec::new();

        // Tier 1: Numeric claims extracted via regex, verified against achievements
        findings.extend(self.check_numeric_claims(text));

        // Tier 2: Employer/institution claims, verified against experience/education
        findings.extend(self.check_entity_claims(text));

        // Tier 3: Competing offer references — always Forbidden
        findings.extend(self.check_competing_offer_claims(text));

        let max_level = findings.iter()
            .map(|f| f.level)
            .max()
            .unwrap_or(FabricationLevel::Safe);

        FabricationReport {
            blocked: max_level >= FabricationLevel::Forbidden,
            max_level,
            findings,
        }
    }

    fn check_numeric_claims(&self, text: &str) -> Vec<FabricationFinding> {
        static NUMERIC_CLAIM: Lazy<Regex> = Lazy::new(||
            Regex::new(r"\b(\d[\d,]*(?:\.\d+)?)\s*(?:%|x|×|times|percent|million|billion|k\b)").unwrap()
        );
        // For each match, verify against life_sheet.experiences[*].achievements[*].metric_value
        // via exact substring or jaro_winkler >= 0.88
        // ...
        todo!()
    }
}
```

**Migration**: After implementing `lazyjob-core/src/fabrication.rs`:
1. `cover_letter/fabrication.rs` → delete, import from `crate::fabrication`
2. `resume/fabrication.rs` → delete, import from `crate::fabrication`
3. `interview/mock_loop.rs` → update `fabrication_warning` to use `FabricationFinding`

**Verification**: `cargo test fabrication` — tests cover all three tier types, `Forbidden` blocking behavior, and empty-life-sheet graceful degradation.

---

### Cross-Spec L: CompanyRecord Dependency Hardening

`InterviewPrepService::build_context()` must gracefully degrade when `CompanyRecord` is missing or stale:

```rust
// lazyjob-core/src/interview/prep_context.rs (addition)

pub enum CompanySignalQuality {
    Rich,    // interview_signals.len() >= 3 and last_updated < 30 days
    Sparse,  // < 3 signals or signals from user-sourced only
    Missing, // no CompanyRecord at all
}

impl PrepContextBuilder {
    fn assess_company_signal_quality(company: Option<&CompanyRecord>) -> CompanySignalQuality {
        match company {
            None => CompanySignalQuality::Missing,
            Some(c) if c.interview_signals.is_empty() => CompanySignalQuality::Missing,
            Some(c) if c.interview_signals.len() < 3 => CompanySignalQuality::Sparse,
            Some(c) => {
                let stale = c.last_enriched_at
                    .map(|t| (Utc::now() - t).num_days() > 30)
                    .unwrap_or(true);
                if stale { CompanySignalQuality::Sparse } else { CompanySignalQuality::Rich }
            }
        }
    }
}
```

`PrepContext.company_signal_quality` is included in the prep session prompt so the LLM knows to use generic question patterns when signals are `Missing`.

TUI banner when quality is `Missing`: `⚠ No company interview data available — questions are role-generic. Run [R]esearch to improve.`

---

### Cross-Spec M: Session ↔ Application Lifecycle

When an application transitions to `Rejected` or `Withdrawn`, mock interview sessions should be preserved (historical record) but explicitly marked as terminal:

```sql
-- migrations/024_session_application_lifecycle.sql
ALTER TABLE mock_interview_sessions ADD COLUMN application_terminal_at TEXT;
-- Filled by trigger or by ApplicationRepository.update_stage()
```

```rust
// lazyjob-core/src/interview/repository.rs (addition)

impl SqliteMockInterviewSessionRepository {
    /// Called by ApplicationRepository when stage transitions to Rejected/Withdrawn.
    /// Does NOT delete sessions — marks them as belonging to a closed application.
    pub async fn mark_application_terminal(
        &self,
        application_id: ApplicationId,
    ) -> Result<u32, InterviewError>  // returns sessions affected
    {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query!(
            "UPDATE mock_interview_sessions
             SET application_terminal_at = ?1
             WHERE application_id = ?2 AND application_terminal_at IS NULL",
            now, application_id.0.to_string()
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(rows as u32)
    }
}
```

Sessions with `application_terminal_at IS NOT NULL` still appear in the session history browser but are shown in a subdued `[closed]` style and excluded from the readiness score calculation.

---

## Module Structure

```
lazyjob-core/
  src/
    fabrication.rs               ← NEW canonical shared module (Cross-Spec K)
    cover_letter/
      mod.rs
      types.rs                   ← extended with CoverLetterSentStatus, SubmissionChannel
      repository.rs              ← mark_sent, set_label, archive_unsent_versions_for_job
      anti_ghosting.rs           ← NEW (GAP-55): AntiGhostingDetector, GenericnessReport
    interview/
      mod.rs
      session_checkpoint.rs      ← NEW (GAP-50): SessionCheckpoint, SessionCheckpointer
      signals.rs                 ← NEW (GAP-51): UserSourcedInterviewSignal, ExtractedQuestion
      fatigue.rs                 ← NEW (GAP-52): FatigueReport, FatigueAnalyzer
      async_format.rs            ← NEW (GAP-53): AsyncPlatformFormat, AsyncPlatform
      system_design.rs           ← NEW (GAP-54): SystemDesignRubric, SeniorityLevel
      analytics.rs               ← NEW (GAP-56): PrepAnalyticsReport, PrepAnalyticsService
      prep_context.rs            ← extended (Cross-Spec L): CompanySignalQuality
      repository.rs              ← extended (Cross-Spec M): mark_application_terminal
    salary/
      market_data.rs             ← extended (GAP-57): DataConfidence, freshness_label, is_fresh_for
    networking/
      personalization.rs         ← NEW (GAP-58): PersonalizationScore, PersonalizationScorer
      scale_guard.rs             ← NEW (GAP-58): ScaleGuardReport, ScaleGateResult

lazyjob-ralph/
  src/
    loops/
      system_design.rs           ← NEW (GAP-54): SystemDesignEvalParams, evaluation loop

lazyjob-tui/
  src/
    cover_letter/
      version_browser.rs         ← updated (GAP-49): sent-state, label, diff view
    interview/
      session_resume_picker.rs   ← NEW (GAP-50): resume/discard dialog
      fatigue_banner.rs          ← NEW (GAP-52): dismissable banner
      system_design_panel.rs     ← NEW (GAP-54): rubric table, timed response editor
      analytics_dashboard.rs     ← NEW (GAP-56): BarChart, trend arrows, readiness gauge
    salary/
      market_freshness_badge.rs  ← NEW (GAP-57): colored freshness indicator
    networking/
      personalization_sidebar.rs ← NEW (GAP-58): depth score, scale guard
```

---

## Key Crate APIs

- `similar::TextDiff::from_lines(prev, next).unified_diff().to_string()` — cover letter version diffs (GAP-49)
- `sqlx::query!("... ON CONFLICT(session_id) DO UPDATE SET ...")` — single-row checkpoint upsert (GAP-50)
- `chrono::Duration::hours(48)` — checkpoint expiry window (GAP-50)
- `once_cell::sync::Lazy<Vec<(&str, Regex)>>` — compiled generic phrase patterns (GAP-55)
- `itertools::Itertools::into_group_map_by()` — category-grouped feedback aggregation (GAP-56)
- `chrono::Utc::now() - self.collected_at).num_days()` — salary data age (GAP-57)
- `strsim::jaro_winkler(a, b) >= 0.92` — company/school fuzzy matching in personalization (GAP-58)
- `tokio::process::Command::new` (existing) — spawning `SystemDesignEvalLoop` (GAP-54)

---

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum GapClosureError {
    #[error("cover letter version not found: {0}")]
    VersionNotFound(CoverLetterVersionId),

    #[error("cannot mark as sent: version is already archived")]
    VersionArchived,

    #[error("session checkpoint expired (created: {0})")]
    CheckpointExpired(DateTime<Utc>),

    #[error("no checkpoint found for session: {0}")]
    NoCheckpoint(MockInterviewSessionId),

    #[error("system design eval loop failed: {0}")]
    SystemDesignEvalFailed(String),

    #[error("salary data freshness check failed")]
    FreshnessCheckFailed,

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}
```

---

## SQLite Migrations Summary

| Migration | Purpose |
|-----------|---------|
| `020_cover_letter_sent_state.sql` | Add `label`, `sent_status`, `sent_at`, `channel`, `paired_resume_version_id` to `cover_letter_versions` |
| `021_interview_session_checkpoints.sql` | New `interview_session_checkpoints` table; `is_partial`, `resumed_from_checkpoint_id` on sessions |
| `022_user_sourced_interview_signals.sql` | New `user_sourced_interview_signals` table |
| `023_salary_data_freshness.sql` | Add `data_as_of`, `collected_at`, `sample_count` to `market_data` |
| `024_session_application_lifecycle.sql` | Add `application_terminal_at` to `mock_interview_sessions` |

---

## Testing Strategy

### Unit Tests

- `fabrication::tests::tier1_numeric_claim_detected` — numeric claim in generated text not in LifeSheet → `Risky`
- `fabrication::tests::tier3_competing_offer_blocked` — "another offer of $X" → `Forbidden`, `blocked = true`
- `cover_letter::anti_ghosting::tests::generic_phrase_detected` — "I am writing to express" → score > 0.0
- `cover_letter::anti_ghosting::tests::company_ref_found` — body contains company.name → `has_company_specific_ref = true`
- `interview::fatigue::tests::rest_after_three_sessions` — `sessions_today = 3` → `FatigueRecommendation::Rest`
- `interview::fatigue::tests::light_session_after_two` — `sessions_today = 2` → `LightSession { max_questions: 3 }`
- `interview::analytics::tests::weakest_category_identified` — behavioral avg 4.5 < technical avg 7.2 → `weakest = Behavioral`
- `salary::market_data::tests::stale_for_negotiation` — `age_days = 45` → `is_fresh_for(NegotiationSupport) = false`
- `networking::personalization::tests::cold_score` — no shared context, no mutual → `Cold` depth, score < 25
- `networking::scale_guard::tests::soft_block_at_limit` — `cold_today = 3`, depth Cold → `SoftBlock`

### Integration Tests

- `#[sqlx::test(migrations = "migrations")]` for all repository operations
- `cover_letter::repository::tests::mark_sent_archives_others` — mark version 2 sent, assert version 1 = archived
- `interview::session_checkpoint::tests::checkpoint_upsert_idempotent` — calling `save()` twice, only one row in DB
- `interview::session_checkpoint::tests::expired_checkpoint_returns_none` — manually set `expires_at` to past, `load()` returns `None`

### TUI Tests

- `version_browser`: render with one Draft + one Sent version; assert Sent row shows `[SENT]` in `Style::fg(Color::Green)`
- `session_resume_picker`: render with valid checkpoint; assert `[r] Resume` action visible
- `fatigue_banner`: render with `Rest` recommendation; assert banner spans full width in `Color::Yellow`

---

## Open Questions

1. **GAP-53 video storage**: If future versions add actual video recording, should storage be local-only (filesystem path in SQLite) or cloud-optional? This plan defers to a separate `XX-async-video-storage.md` spec.

2. **Cross-Spec K migration timing**: `fabrication.rs` unification requires updating all three existing plans. Which iteration ships this unification? Recommendation: ship with the first plan that is implemented (likely resume tailoring, since it's Phase 1 of the MVP), then other plans import from it.

3. **GAP-51 LLM extraction cost**: At ~200 tokens per extraction, importing 5 user-pasted interview signal texts costs ~1000 tokens. Should this be shown to the user before extracting? Add a token-cost preview to the import UX.

4. **GAP-52 readiness scoring**: The `readiness_score` formula is based only on mock session scores. Real interview pass rate data would improve it. Phase 3 could add a correlation analysis when users record real interview outcomes.

5. **Cross-Spec M hard-delete**: If a user deletes an application entirely (not just withdraws), should mock sessions be deleted? Current plan preserves all sessions for historical prep analytics. Add `ON DELETE SET NULL` for `application_id` FK to allow orphaned sessions.

---

## Related Specs

- [specs/profile-cover-letter-generation.md](./profile-cover-letter-generation.md)
- [specs/XX-cover-letter-version-management.md](./XX-cover-letter-version-management.md)
- [specs/interview-prep-mock-loop.md](./interview-prep-mock-loop.md)
- [specs/XX-interview-session-resumability.md](./XX-interview-session-resumability.md)
- [specs/interview-prep-question-generation.md](./interview-prep-question-generation.md)
- [specs/application-state-machine.md](./application-state-machine.md)
- [specs/salary-market-intelligence.md](./salary-market-intelligence.md)
- [specs/networking-outreach-drafting.md](./networking-outreach-drafting.md)
- [specs/job-search-company-research.md](./job-search-company-research.md)
- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md)
