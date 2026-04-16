# Spec: Interview Prep Session Resumability

## Context

Users practice mock interviews with Ralph's MockInterviewLoop. Sessions can be long and users may need to quit mid-session. Without resumability, all progress is lost. This spec addresses partial session saving and resumption.

## Motivation

- **User reality**: Users will quit mid-session (interrupted, tired, time-limited)
- **High-stakes preservation**: Losing 30 minutes of interview prep is costly
- **Flexible practice**: "One question at a time" across multiple sessions

## Design

### Session State Persistence

```rust
pub struct MockInterviewSession {
    pub id: SessionId,
    pub application_id: Option<ApplicationId>,
    pub loop_type: LoopType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub questions: Vec<RecordedQuestion>,
    pub current_question_index: usize,
    pub current_turn: InterviewTurn,
    pub score_so_far: Option<SessionScore>,
    pub user_profile: ResumeContext,
    pub company_context: CompanyContext,
    pub conversation_history: Vec<ConversationTurn>,
}

pub enum SessionStatus {
    InProgress,
    Completed,
    Partial,      // User quit early but session was saved
    Abandoned,    // User quit without saving
}

pub struct RecordedQuestion {
    pub index: usize,
    pub question_text: String,
    pub question_type: InterviewType,
    pub user_answer: Option<String>,
    pub feedback: Option<QuestionFeedback>,
    pub answered_at: Option<DateTime<Utc>>,
}

pub struct InterviewTurn {
    pub role: TurnRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

pub enum TurnRole {
    RalphAsInterviewer,
    UserAsCandidate,
}
```

### Saving Partial Sessions

```rust
impl MockInterviewService {
    /// Auto-save every N minutes or after each question
    pub async fn auto_save(&self, session_id: SessionId) -> Result<()> {
        let session = self.get_session(session_id).await?;

        let save = PartialSessionSave {
            session_id,
            saved_at: Utc::now(),
            questions_completed: session.current_question_index,
            conversation_summary: self.summarize_conversation(&session.conversation_history)?,
            current_state: CurrentState::from_session(&session),
            auto_save_number: session.auto_save_count + 1,
        };

        self.db.save_partial(&save).await?;
        self.notify_tui(UIEvent::SessionAutoSaved {
            session_id,
            questions_completed: save.questions_completed,
        }).await?;

        Ok(())
    }
}

pub struct CurrentState {
    pub question_text: String,
    pub user_has_answered: bool,
    pub awaiting_ralph_question: bool,
    pub time_in_current_turn_secs: u64,
}
```

### Resuming Sessions

```rust
pub struct ResumeRequest {
    pub session_id: SessionId,
    pub estimated_token_cost: usize,  // Warn user before resuming
}

impl MockInterviewService {
    pub async fn can_resume(&self, session_id: SessionId) -> Result<ResumeCheck> {
        let session = self.get_session(session_id).await?;

        // Check staleness
        let hours_old = (Utc::now() - session.updated_at).num_hours();
        if hours_old > 24 {
            return Ok(ResumeCheck::TooStale {
                hours: hours_old,
                suggestion: "Start a new session instead".to_string(),
            });
        }

        // Estimate token cost for resuming
        let token_cost = self.estimate_resume_tokens(&session).await?;

        Ok(ResumeCheck::Resumeable {
            session_id,
            questions_completed: session.current_question_index,
            total_questions: session.questions.len(),
            estimated_token_cost,
        })
    }

    pub async fn resume_session(&self, session_id: SessionId) -> Result<ResumeResult> {
        let check = self.can_resume(session_id).await?;

        match check {
            ResumeCheck::TooStale { .. } => Err(ResumeError::SessionTooStale),
            ResumeCheck::Resumeable { token_cost, .. } => {
                // Load session, reconstruct context for LLM
                let session = self.load_full_session(session_id).await?;

                // Warn about token cost if high
                if token_cost > 5000 {
                    // Notify user, let them confirm
                }

                Ok(ResumeResult {
                    session,
                    resume_from_question: session.current_question_index,
                })
            }
        }
    }
}
```

### Resume UX

When user opens LazyJob after a partial session:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  ⚠️  You have a partial mock interview session                       │   │
│  │                                                                     │   │
│  │  Stripe - Senior SWE Interview                                      │   │
│  │  Questions completed: 4 of 8                                        │   │
│  │  Last activity: 2 hours ago                                        │   │
│  │                                                                     │   │
│  │  Estimated cost to resume: ~3,500 tokens ($0.02)                    │   │
│  │                                                                     │   │
│  │  [Resume Session]  [Save & Close]  [Discard Session]                │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Resume Token Cost Transparency

```rust
impl MockInterviewService {
    async fn estimate_resume_tokens(&self, session: &MockInterviewSession) -> Result<usize> {
        // Count conversation history turns
        let history_tokens = session.conversation_history
            .iter()
            .map(|t| self.count_tokens(&t.content))
            .sum::<usize>();

        // Count all question/answer pairs so far
        let qa_tokens = session.questions
            .iter()
            .filter_map(|q| q.user_answer.as_ref())
            .map(|a| self.count_tokens(a))
            .sum::<usize>();

        // Context summary tokens (always sent)
        let context_tokens = self.count_tokens(&self.generate_context_summary(session)?);

        // The LLM needs all of this for continuity
        Ok(history_tokens + qa_tokens + context_tokens)
    }
}
```

### Session Timeout Handling

```rust
pub struct SessionTimeoutPolicy {
    pub inactivity_threshold_mins: u32 = 30,
    pub auto_save_before_timeout: bool = true,
    pub prompt_before_timeout_secs: u32 = 60,
}

impl MockInterviewService {
    pub async fn handle_inactivity_check(&self, session_id: SessionId) -> Result<TimeoutAction> {
        let session = self.get_session(session_id).await?;
        let inactive_secs = (Utc::now() - session.updated_at).num_seconds() as u32;

        let threshold_secs = self.policy.inactivity_threshold_mins * 60;

        if inactive_secs >= threshold_secs {
            if self.policy.auto_save_before_timeout {
                self.auto_save(session_id).await?;
            }

            return Ok(TimeoutAction::SessionPaused {
                session_id,
                resume_available_until: Utc::now() + Duration::hours(24),
            });
        }

        Ok(TimeoutAction::Continue)
    }
}
```

Sessions auto-close (can't resume) after 24 hours of inactivity.

## Implementation Notes

- Auto-save triggers: every 5 minutes, after each question answer
- Partial sessions stored in `mock_interview_sessions` table with `status = Partial`
- On app startup, check for partial sessions and surface notification
- Resumption sends full conversation history to LLM for context

## Open Questions

1. **Multiple partial sessions**: Can user have several partial sessions?
2. **Resume vs restart**: Should we prompt to resume or just restart fresh?
3. **Session merging**: Can two partial sessions be merged?

## Related Specs

- `interview-prep-mock-loop.md` - Mock interview loop
- `XX-llm-context-management.md` - LLM context handling
- `XX-llm-cost-budget-management.md` - Token cost awareness