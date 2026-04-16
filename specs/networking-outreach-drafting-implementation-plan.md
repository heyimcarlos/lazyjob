# Implementation Plan: Networking Outreach Drafting

## Status
Draft

## Related Spec
[specs/networking-outreach-drafting.md](networking-outreach-drafting.md)

## Overview

The networking outreach drafting module is an LLM-powered pipeline that produces a ready-to-send outreach message for a specific contact at a target company. It assembles a `SharedContext` struct from verified LifeSheet and contact data (no LLM involvement in context computation), calls the LLM with an anti-fabrication-enforcing prompt template, then validates the draft against medium-specific length constraints and checks factual claims against the verified `SharedContext`.

The module lives in `lazyjob-core/src/networking/` and integrates with `ContactRepository`, `JobRepository`, `CompanyRepository`, `LifeSheet`, and `LlmProvider`. The pipeline has three explicit phases: pure-Rust context assembly → LLM drafting → validation. Every claim in the output must be traceable to data the user imported; invented shared context is worse than a generic message because it reads as deceptive.

LazyJob never sends messages automatically. The output is a text draft the user copies and sends via LinkedIn or email. This is a hard product constraint encoded in the type system (no `send` method exists anywhere).

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — `run_migrations`, connection pool, migration framework
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `LifeSheet`, `Experience`, `Education` domain types
- `specs/networking-connection-mapping-implementation-plan.md` — `ProfileContact`, `ContactRepository`, `SuggestedApproach`, `ConnectionTier`, `normalize_company_name()`
- `specs/job-search-company-research-implementation-plan.md` — `CompanyRecord`, `CompanyRepository`
- `specs/job-search-discovery-engine-implementation-plan.md` — `JobRecord`, `JobRepository`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` async trait, `ChatMessage`, `ChatRequest`
- `specs/17-ralph-prompt-templates-implementation-plan.md` — TOML template infrastructure, `SimpleTemplateEngine`
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI event loop, panel system

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# No new crates required — all dependencies are already in the workspace:
# uuid, chrono, serde, serde_json, sqlx, thiserror, anyhow, tokio, async-trait
# once_cell, tracing — already used in prior modules
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| Domain types (`OutreachMedium`, `OutreachTone`, `OutreachStatus`, `SharedContext`, `OutreachDraft`, `OutreachRecord`) | `lazyjob-core` | `src/networking/outreach/types.rs` |
| `SharedContext` computation (pure, no LLM) | `lazyjob-core` | `src/networking/outreach/context.rs` |
| `OutreachDraftingService` trait + `LlmOutreachDraftingService` impl | `lazyjob-core` | `src/networking/outreach/service.rs` |
| Fabrication checker | `lazyjob-core` | `src/networking/outreach/fabrication.rs` |
| Length enforcer | `lazyjob-core` | `src/networking/outreach/length.rs` |
| `OutreachRepository` trait + `SqliteOutreachRepository` impl | `lazyjob-core` | `src/networking/outreach/repo.rs` |
| SQLite migration (016) | `lazyjob-core` | `migrations/016_outreach_drafts.sql` |
| TOML prompt template | `lazyjob-llm` | `src/prompts/networking_outreach.toml` |
| TUI outreach draft view | `lazyjob-tui` | `src/views/networking/outreach_draft.rs` |
| Module re-export facade | `lazyjob-core` | `src/networking/outreach/mod.rs` |

### Core Types

```rust
// lazyjob-core/src/networking/outreach/types.rs

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::networking::ContactId;
use crate::discovery::JobId;

/// Strongly-typed outreach record ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct OutreachId(pub Uuid);
impl OutreachId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// The channel over which the user will send the message.
/// Determines hard length limits enforced before presenting the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "snake_case")]
pub enum OutreachMedium {
    /// LinkedIn connection request note — hard cap 300 chars enforced by LinkedIn.
    LinkedInConnectionNote,
    /// LinkedIn DM — up to 8000 chars; target < 400 chars for best response rate.
    LinkedInMessage,
    /// Email — target 100–300 words.
    Email,
    /// SMS / Twitter DM / Slack — hard cap 150 chars.
    ShortForm,
}

impl OutreachMedium {
    /// Returns (min_chars, max_chars, min_words, max_words) limits.
    /// `None` means no hard limit in that dimension.
    pub fn limits(&self) -> MediumLimits {
        match self {
            OutreachMedium::LinkedInConnectionNote => MediumLimits {
                max_chars: Some(300),
                min_words: None,
                max_words: None,
                hard_clip: true,
            },
            OutreachMedium::LinkedInMessage => MediumLimits {
                max_chars: Some(400), // soft target; hard limit is 8000
                min_words: None,
                max_words: None,
                hard_clip: false,
            },
            OutreachMedium::Email => MediumLimits {
                max_chars: None,
                min_words: Some(100),
                max_words: Some(300),
                hard_clip: false,
            },
            OutreachMedium::ShortForm => MediumLimits {
                max_chars: Some(150),
                min_words: None,
                max_words: None,
                hard_clip: true,
            },
        }
    }

    pub fn to_db_str(&self) -> &'static str {
        match self {
            OutreachMedium::LinkedInConnectionNote => "linkedin_connection_note",
            OutreachMedium::LinkedInMessage => "linkedin_message",
            OutreachMedium::Email => "email",
            OutreachMedium::ShortForm => "short_form",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediumLimits {
    pub max_chars: Option<usize>,
    pub min_words: Option<usize>,
    pub max_words: Option<usize>,
    /// If true, the output is hard-clipped at max_chars with a trailing ellipsis warning.
    pub hard_clip: bool,
}

/// Tone variant selected based on the `SuggestedApproach` from connection mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "snake_case")]
pub enum OutreachTone {
    /// RequestReferral: warm, direct, explicit ask for referral for a specific role.
    Warm,
    /// InformationalInterview: curious, humble, request a 20-min chat.
    Curious,
    /// ReconnectFirst: casual, no job ask — just re-establishing contact.
    Casual,
    /// ColdOutreach: professional, value-forward, introduce self before any ask.
    Professional,
}

impl OutreachTone {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            OutreachTone::Warm => "warm",
            OutreachTone::Curious => "curious",
            OutreachTone::Casual => "casual",
            OutreachTone::Professional => "professional",
        }
    }
}

/// Lifecycle state of outreach to a given contact.
/// Updated manually by the user in the TUI — LazyJob cannot detect actual sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutreachStatus {
    NotYetContacted,
    DraftGenerated,
    MessageSent { sent_at: NaiveDate },
    Responded { responded_at: NaiveDate },
    NoResponse,
    NotInterested,
}

impl OutreachStatus {
    pub fn to_db_str(&self) -> &'static str {
        match self {
            OutreachStatus::NotYetContacted => "not_yet_contacted",
            OutreachStatus::DraftGenerated => "draft_generated",
            OutreachStatus::MessageSent { .. } => "message_sent",
            OutreachStatus::Responded { .. } => "responded",
            OutreachStatus::NoResponse => "no_response",
            OutreachStatus::NotInterested => "not_interested",
        }
    }
}

/// Request to draft an outreach message.
#[derive(Debug, Clone)]
pub struct OutreachRequest {
    pub contact_id: ContactId,
    /// None for general reconnect (no specific role).
    pub job_id: Option<JobId>,
    pub medium: OutreachMedium,
    pub tone: OutreachTone,
    /// Any additional context the user wants to include (e.g. "mention the shared project").
    pub user_notes: Option<String>,
}

/// Verified overlap between user and contact — assembled without LLM involvement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedContext {
    pub shared_employers: Vec<SharedEmployer>,
    pub shared_schools: Vec<SharedSchool>,
    /// Broadest shared industry label, if both worked in the same domain.
    pub shared_industry: Option<String>,
    /// How long the contact has been in their current role (in months), if computable.
    pub contact_current_role_tenure_months: Option<u32>,
}

impl SharedContext {
    /// Returns true if there is at least one verified, specific shared fact.
    pub fn has_genuine_hook(&self) -> bool {
        !self.shared_employers.is_empty() || !self.shared_schools.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedEmployer {
    pub company_name: String,
    pub user_dates: (NaiveDate, Option<NaiveDate>),
    pub contact_dates: Option<(NaiveDate, Option<NaiveDate>)>,
    /// Some if the tenures actually overlapped; None if sequential.
    pub overlap_months: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSchool {
    pub institution_name: String,
    pub user_degree: Option<String>,
    pub contact_degree: Option<String>,
}

/// The fully assembled outreach draft returned to the caller.
#[derive(Debug, Clone)]
pub struct OutreachDraft {
    pub id: OutreachId,
    pub request: OutreachRequest,
    pub shared_context: SharedContext,
    pub draft_text: String,
    pub char_count: usize,
    pub word_count: usize,
    /// False if the draft exceeds medium limits; caller should warn the user.
    pub medium_limit_ok: bool,
    /// Non-empty if any factual claim in the draft could not be grounded in SharedContext.
    pub fabrication_warnings: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Persisted record of a generated outreach draft (stored in `outreach_drafts` table).
#[derive(Debug, Clone)]
pub struct OutreachRecord {
    pub id: OutreachId,
    pub contact_id: ContactId,
    pub job_id: Option<JobId>,
    pub medium: OutreachMedium,
    pub tone: OutreachTone,
    pub draft_text: String,
    pub shared_context_json: String, // serde_json serialized SharedContext
    pub fabrication_warnings_json: String, // serde_json serialized Vec<String>
    pub char_count: i64,
    pub word_count: i64,
    pub medium_limit_ok: bool,
    pub generated_at: DateTime<Utc>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/networking/outreach/service.rs

use async_trait::async_trait;
use std::sync::Arc;

use crate::life_sheet::LifeSheet;
use crate::networking::{ContactRepository, ProfileContact};
use crate::networking::outreach::types::*;
use crate::llm::LlmProvider;
use crate::Result;

#[async_trait]
pub trait OutreachDraftingService: Send + Sync {
    /// Assemble SharedContext, call LLM, validate output, return draft.
    /// Does NOT persist — caller must call `OutreachRepository::save_draft` after user approval.
    async fn draft(
        &self,
        req: OutreachRequest,
        life_sheet: &LifeSheet,
    ) -> Result<OutreachDraft, OutreachError>;

    /// Pure structural comparison — no LLM. Exposed for testing and TUI preview.
    async fn compute_shared_context(
        &self,
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Result<SharedContext, OutreachError>;
}

// lazyjob-core/src/networking/outreach/repo.rs

#[async_trait]
pub trait OutreachRepository: Send + Sync {
    async fn save_draft(&self, draft: &OutreachDraft) -> Result<OutreachId, OutreachError>;
    async fn get_latest_draft(&self, contact_id: &ContactId) -> Result<Option<OutreachRecord>, OutreachError>;
    async fn list_drafts_for_contact(
        &self,
        contact_id: &ContactId,
    ) -> Result<Vec<OutreachRecord>, OutreachError>;
    async fn mark_sent(
        &self,
        contact_id: &ContactId,
        sent_at: NaiveDate,
    ) -> Result<(), OutreachError>;
    async fn mark_responded(
        &self,
        contact_id: &ContactId,
        responded_at: NaiveDate,
    ) -> Result<(), OutreachError>;
    async fn mark_no_response(&self, contact_id: &ContactId) -> Result<(), OutreachError>;
    async fn update_status(
        &self,
        contact_id: &ContactId,
        status: &OutreachStatus,
    ) -> Result<(), OutreachError>;
}
```

### SQLite Schema

```sql
-- migrations/016_outreach_drafts.sql

-- Alter profile_contacts to track outreach lifecycle
ALTER TABLE profile_contacts
  ADD COLUMN outreach_status TEXT NOT NULL DEFAULT 'not_yet_contacted';
ALTER TABLE profile_contacts
  ADD COLUMN outreach_sent_at DATE;
ALTER TABLE profile_contacts
  ADD COLUMN outreach_responded_at DATE;

-- Store the most recent draft text directly on the contact row for fast display
ALTER TABLE profile_contacts
  ADD COLUMN last_draft_text TEXT;

CREATE TABLE IF NOT EXISTS outreach_drafts (
    id                       TEXT PRIMARY KEY,           -- OutreachId (UUID)
    contact_id               TEXT NOT NULL
                               REFERENCES profile_contacts(id) ON DELETE CASCADE,
    job_id                   TEXT,                       -- nullable
    medium                   TEXT NOT NULL,              -- OutreachMedium db str
    tone                     TEXT NOT NULL,              -- OutreachTone db str
    draft_text               TEXT NOT NULL,
    shared_context_json      TEXT NOT NULL,
    fabrication_warnings_json TEXT NOT NULL DEFAULT '[]',
    char_count               INTEGER NOT NULL,
    word_count               INTEGER NOT NULL,
    medium_limit_ok          INTEGER NOT NULL DEFAULT 1,  -- SQLite bool
    generated_at             TEXT NOT NULL               -- ISO 8601 UTC
);

CREATE INDEX IF NOT EXISTS idx_outreach_drafts_contact
    ON outreach_drafts (contact_id, generated_at DESC);

CREATE INDEX IF NOT EXISTS idx_outreach_drafts_job
    ON outreach_drafts (job_id)
    WHERE job_id IS NOT NULL;

-- Follow-up reminders for sent outreach (polled by existing ReminderPoller)
CREATE TABLE IF NOT EXISTS outreach_follow_up_reminders (
    id             TEXT PRIMARY KEY,
    contact_id     TEXT NOT NULL
                     REFERENCES profile_contacts(id) ON DELETE CASCADE,
    outreach_id    TEXT NOT NULL
                     REFERENCES outreach_drafts(id) ON DELETE CASCADE,
    remind_at      TEXT NOT NULL,  -- ISO 8601 UTC
    fired_at       TEXT,           -- NULL until fired
    days_after     INTEGER NOT NULL DEFAULT 7
);

CREATE INDEX IF NOT EXISTS idx_outreach_reminders_pending
    ON outreach_follow_up_reminders (remind_at)
    WHERE fired_at IS NULL;
```

### Module Structure

```
lazyjob-core/
  src/
    networking/
      mod.rs                   -- re-exports ContactId, ProfileContact, ConnectionTier, etc.
      types.rs                 -- existing domain types from connection mapping plan
      contact_repo.rs          -- existing ContactRepository trait + SqliteContactRepository
      normalize.rs             -- existing normalize_company_name()
      connection_mapper.rs     -- existing ConnectionMapper
      csv_import.rs            -- existing LinkedInCsvImporter
      contact_service.rs       -- existing ContactService
      outreach/
        mod.rs                 -- re-exports OutreachDraft, OutreachRequest, etc.
        types.rs               -- all outreach domain types (this plan)
        context.rs             -- SharedContextBuilder (pure, no LLM)
        service.rs             -- OutreachDraftingService trait + LlmOutreachDraftingService
        repo.rs                -- OutreachRepository trait + SqliteOutreachRepository
        fabrication.rs         -- OutreachFabricationChecker
        length.rs              -- MediumLengthEnforcer
  migrations/
    016_outreach_drafts.sql

lazyjob-llm/
  src/
    prompts/
      networking_outreach.toml -- TOML template with tone variant sections

lazyjob-tui/
  src/
    views/
      networking/
        outreach_draft.rs      -- OutreachDraftView widget
        outreach_list.rs       -- per-contact draft history list
```

## Implementation Phases

### Phase 1 — Core Domain and SharedContext Computation (MVP)

**Step 1.1 — Domain types in `types.rs`**

- Implement `OutreachMedium`, `OutreachTone`, `OutreachStatus`, `OutreachRequest`, `SharedContext`, `SharedEmployer`, `SharedSchool`, `OutreachDraft`, `OutreachRecord`.
- Implement `OutreachMedium::limits() -> MediumLimits` as a hardcoded `match`.
- Implement `OutreachId::new()` as `Uuid::new_v4()` wrapper.
- `OutreachError` via `thiserror` (see Error Handling section).

File: `lazyjob-core/src/networking/outreach/types.rs`

Verification: `cargo test networking::outreach::types` — test `limits()` for each variant.

**Step 1.2 — SQLite migration 016**

- Apply `migrations/016_outreach_drafts.sql` (ALTER TABLE + CREATE TABLE + indices).
- Add migration to the `run_migrations` call in `lazyjob-core/src/db.rs`.

File: `lazyjob-core/migrations/016_outreach_drafts.sql`

Verification: `cargo sqlx prepare` passes; `#[sqlx::test(migrations = "migrations")]` creates the tables.

**Step 1.3 — `SharedContextBuilder` in `context.rs`**

Pure sync struct (no LLM). Computes verified overlap by comparing LifeSheet entries against `ProfileContact` fields.

```rust
// lazyjob-core/src/networking/outreach/context.rs

use once_cell::sync::Lazy;
use strsim::jaro_winkler;

static COMPANY_NAME_NORMALIZER: Lazy<regex::Regex> = Lazy::new(|| {
    // reuses normalize_company_name() from networking::normalize
    regex::Regex::new(r"(?i)\s*(inc\.?|llc\.?|ltd\.?|corp\.?|co\.?|group|holdings?)\s*$").unwrap()
});

const JW_COMPANY_THRESHOLD: f64 = 0.92;

pub struct SharedContextBuilder;

impl SharedContextBuilder {
    /// Pure structural comparison — no async, no LLM, no I/O.
    pub fn compute(contact: &ProfileContact, life_sheet: &LifeSheet) -> SharedContext {
        let shared_employers = Self::find_shared_employers(contact, life_sheet);
        let shared_schools = Self::find_shared_schools(contact, life_sheet);
        let shared_industry = Self::infer_shared_industry(contact, life_sheet);
        let contact_current_role_tenure_months =
            Self::compute_current_tenure(contact);

        SharedContext {
            shared_employers,
            shared_schools,
            shared_industry,
            contact_current_role_tenure_months,
        }
    }

    fn find_shared_employers(
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Vec<SharedEmployer> {
        let mut result = Vec::new();
        for user_exp in &life_sheet.experience {
            let user_norm = normalize_company_name(&user_exp.company);
            // Check current company first
            if let Some(contact_company) = &contact.current_company {
                let contact_norm = normalize_company_name(contact_company);
                if names_match(&user_norm, &contact_norm) {
                    result.push(SharedEmployer {
                        company_name: contact_company.clone(),
                        user_dates: (user_exp.start_date, user_exp.end_date),
                        contact_dates: None, // contact's dates not in CSV
                        overlap_months: None,
                    });
                    continue;
                }
            }
            // Check previous companies from profile_contacts.previous_companies_json
            for prev in &contact.previous_companies {
                let prev_norm = normalize_company_name(&prev.company_name);
                if names_match(&user_norm, &prev_norm) {
                    let overlap = compute_overlap_months(
                        user_exp.start_date,
                        user_exp.end_date,
                        prev.start_year,
                        prev.end_year,
                    );
                    result.push(SharedEmployer {
                        company_name: prev.company_name.clone(),
                        user_dates: (user_exp.start_date, user_exp.end_date),
                        contact_dates: prev.start_year.map(|sy| {
                            let start = NaiveDate::from_ymd_opt(sy, 1, 1).unwrap_or_default();
                            let end = prev.end_year.and_then(|ey| NaiveDate::from_ymd_opt(ey, 12, 31).ok());
                            (start, end)
                        }),
                        overlap_months: overlap,
                    });
                }
            }
        }
        // Deduplicate by company_name (normalized)
        result.dedup_by(|a, b| {
            normalize_company_name(&a.company_name) == normalize_company_name(&b.company_name)
        });
        result
    }

    fn find_shared_schools(
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Vec<SharedSchool> {
        let mut result = Vec::new();
        for user_edu in &life_sheet.education {
            let user_norm = user_edu.institution.to_lowercase();
            for contact_school in &contact.schools {
                let contact_norm = contact_school.institution_name.to_lowercase();
                if jaro_winkler(&user_norm, &contact_norm) >= JW_COMPANY_THRESHOLD {
                    result.push(SharedSchool {
                        institution_name: user_edu.institution.clone(),
                        user_degree: user_edu.degree.clone(),
                        contact_degree: contact_school.degree.clone(),
                    });
                    break;
                }
            }
        }
        result
    }

    fn infer_shared_industry(
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Option<String> {
        // Use the most recent experience industry tag from LifeSheet,
        // compare against contact.industry if set.
        let user_industry = life_sheet.experience.first()
            .and_then(|e| e.industry.as_deref())?;
        let contact_industry = contact.industry.as_deref()?;
        if jaro_winkler(
            &user_industry.to_lowercase(),
            &contact_industry.to_lowercase(),
        ) >= 0.80 {
            Some(user_industry.to_string())
        } else {
            None
        }
    }

    fn compute_current_tenure(contact: &ProfileContact) -> Option<u32> {
        let company = contact.current_company.as_ref()?;
        let start_year = contact.current_company_start_year?;
        let now = chrono::Utc::now().naive_utc().date();
        let start = NaiveDate::from_ymd_opt(start_year, 1, 1)?;
        let months = (now.year() - start.year()) * 12
            + (now.month() as i32 - start.month() as i32);
        Some(months.max(0) as u32)
    }
}

fn names_match(a: &str, b: &str) -> bool {
    a == b || jaro_winkler(a, b) >= JW_COMPANY_THRESHOLD
}

fn normalize_company_name(name: &str) -> String {
    // Re-use the free function from networking::normalize
    crate::networking::normalize::normalize_company_name(name)
}
```

Verification: unit tests in `context.rs` test each private method with known inputs.

---

### Phase 2 — LLM Prompt Template and Length Enforcement

**Step 2.1 — Prompt template in `networking_outreach.toml`**

```toml
# lazyjob-llm/src/prompts/networking_outreach.toml
[meta]
loop_type = "NetworkingOutreach"
version = 1
cache_system_prompt = true

[system]
content = """
You are a professional networking assistant. Generate a single, personalized outreach message.

RULES (never violate):
1. Every factual claim must appear in the provided `shared_context` JSON.
2. If `shared_context.shared_employers` is empty and `shared_context.shared_schools` is empty,
   use hedged language ("I've been following your work on X") — never invent shared history.
3. Never claim mutual connections unless explicitly listed in `shared_context.mutual_connections`.
4. Never reference the contact's salary, personal details beyond their public profile,
   or internal company information.
5. For tone = "casual" (ReconnectFirst): do NOT include a job ask. The goal is relationship
   re-warming only.
6. For tone = "warm" (RequestReferral): make the referral ask specific to the role provided.

OUTPUT FORMAT: Return only the message text. No greeting meta-commentary. No subject line unless
medium = "email".
"""

[user_warm]
content = """
Draft a warm, direct outreach message. Goal: ask {contact_name} for a referral for the
{role_title} role at {company_name}.

Contact: {contact_name}, {contact_current_title} at {contact_current_company}
Shared context: {shared_context_json}
User background summary: {user_background_summary}
Target role: {role_title} at {company_name}
Additional notes from user: {user_notes}

Medium: {medium_label}
Constraints: {medium_constraints}
"""

[user_curious]
content = """
Draft a curious, humble outreach message. Goal: request a 20-minute informational conversation
with {contact_name} about their role/company.

Contact: {contact_name}, {contact_current_title} at {contact_current_company}
Shared context: {shared_context_json}
User background summary: {user_background_summary}
Additional notes from user: {user_notes}

Medium: {medium_label}
Constraints: {medium_constraints}
"""

[user_casual]
content = """
Draft a casual, low-pressure reconnect message to {contact_name}. Goal: re-establish contact.
Do NOT mention jobs or job searches.

Contact: {contact_name}, {contact_current_title} at {contact_current_company}
Shared context: {shared_context_json}
User background summary: {user_background_summary}
Additional notes from user: {user_notes}

Medium: {medium_label}
Constraints: {medium_constraints}
"""

[user_professional]
content = """
Draft a professional, value-forward cold outreach message to {contact_name}.
Goal: introduce the user and establish relevance before any ask.

Contact: {contact_name}, {contact_current_title} at {contact_current_company}
Shared context: {shared_context_json}
User background summary: {user_background_summary}
Target role (for context): {role_title} at {company_name}
Additional notes from user: {user_notes}

Medium: {medium_label}
Constraints: {medium_constraints}
"""
```

**Step 2.2 — `MediumLengthEnforcer` in `length.rs`**

```rust
// lazyjob-core/src/networking/outreach/length.rs

pub struct MediumLengthEnforcer;

impl MediumLengthEnforcer {
    /// Returns (possibly clipped) draft text and whether the limit was satisfied.
    pub fn enforce(text: &str, limits: &MediumLimits) -> (String, bool) {
        let char_count = text.chars().count();
        let word_count = text.split_whitespace().count();

        // Hard clip (only for LinkedInConnectionNote and ShortForm)
        if limits.hard_clip {
            if let Some(max_chars) = limits.max_chars {
                if char_count > max_chars {
                    let clipped: String = text.chars().take(max_chars.saturating_sub(1)).collect();
                    return (format!("{}…", clipped), false);
                }
            }
        }

        // Soft limit checks — return the original text, flag the violation
        let ok = limits.max_chars.map_or(true, |mc| char_count <= mc)
            && limits.max_words.map_or(true, |mw| word_count <= mw)
            && limits.min_words.map_or(true, |mw| word_count >= mw);

        (text.to_string(), ok)
    }

    pub fn count_chars(text: &str) -> usize { text.chars().count() }
    pub fn count_words(text: &str) -> usize { text.split_whitespace().count() }
}
```

Verification: property tests using known strings for each medium variant.

---

### Phase 3 — Fabrication Checker

**Step 3.1 — `OutreachFabricationChecker` in `fabrication.rs`**

The fabrication checker extracts factual claim patterns from the draft and verifies each against `SharedContext`. This is a best-effort pass — the primary anti-fabrication mechanism is the prompt rules. The checker catches regressions.

```rust
// lazyjob-core/src/networking/outreach/fabrication.rs

use once_cell::sync::Lazy;
use regex::Regex;
use strsim::jaro_winkler;

use crate::networking::outreach::types::SharedContext;

// Patterns that look like factual claims about a shared connection
static EMPLOYER_CLAIM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(we (both )?worked at|both (spent time|worked) at|our time at|when (we|you) were at)\s+([A-Z][A-Za-z0-9\s]+?)[\.,!]").unwrap()
});
static SCHOOL_CLAIM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(we (both )?attended|fellow (alumni|alum|graduate) (of|from)|studied at)\s+([A-Z][A-Za-z0-9\s]+?)[\.,!]").unwrap()
});

pub struct OutreachFabricationChecker;

impl OutreachFabricationChecker {
    /// Returns a list of warning strings for claims that could not be grounded.
    pub fn check(draft: &str, context: &SharedContext) -> Vec<String> {
        let mut warnings = Vec::new();

        // Check employer claims
        for cap in EMPLOYER_CLAIM.captures_iter(draft) {
            if let Some(claimed_company) = cap.get(5) {
                let claimed = claimed_company.as_str().trim();
                if !Self::company_in_context(claimed, context) {
                    warnings.push(format!(
                        "Unverified employer claim: \"{}\" not found in imported contact history.",
                        claimed
                    ));
                }
            }
        }

        // Check school claims
        for cap in SCHOOL_CLAIM.captures_iter(draft) {
            if let Some(claimed_school) = cap.get(5) {
                let claimed = claimed_school.as_str().trim();
                if !Self::school_in_context(claimed, context) {
                    warnings.push(format!(
                        "Unverified school claim: \"{}\" not found in imported contact education.",
                        claimed
                    ));
                }
            }
        }

        warnings
    }

    fn company_in_context(claimed: &str, context: &SharedContext) -> bool {
        let claimed_norm = claimed.to_lowercase();
        context.shared_employers.iter().any(|e| {
            let norm = e.company_name.to_lowercase();
            norm == claimed_norm || jaro_winkler(&norm, &claimed_norm) >= 0.88
        })
    }

    fn school_in_context(claimed: &str, context: &SharedContext) -> bool {
        let claimed_norm = claimed.to_lowercase();
        context.shared_schools.iter().any(|s| {
            let norm = s.institution_name.to_lowercase();
            norm == claimed_norm || jaro_winkler(&norm, &claimed_norm) >= 0.88
        })
    }
}
```

Verification: unit tests with crafted draft strings, some with grounded claims, some with invented ones.

---

### Phase 4 — `LlmOutreachDraftingService` and Repository

**Step 4.1 — `LlmOutreachDraftingService` in `service.rs`**

```rust
// lazyjob-core/src/networking/outreach/service.rs

use std::sync::Arc;
use async_trait::async_trait;
use tracing::{info, warn, instrument};
use uuid::Uuid;

use crate::llm::{LlmProvider, ChatMessage, ChatRole};
use crate::life_sheet::LifeSheet;
use crate::networking::{ContactRepository, ProfileContact};
use crate::networking::outreach::{
    context::SharedContextBuilder,
    fabrication::OutreachFabricationChecker,
    length::MediumLengthEnforcer,
    types::*,
    repo::OutreachRepository,
};
use crate::discovery::JobRepository;
use crate::companies::CompanyRepository;
use crate::error::OutreachError;
use crate::Result;

pub struct LlmOutreachDraftingService {
    pub llm: Arc<dyn LlmProvider>,
    pub contact_repo: Arc<dyn ContactRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub company_repo: Arc<dyn CompanyRepository>,
}

#[async_trait]
impl OutreachDraftingService for LlmOutreachDraftingService {
    #[instrument(skip(self, life_sheet), fields(contact_id = %req.contact_id.0))]
    async fn draft(
        &self,
        req: OutreachRequest,
        life_sheet: &LifeSheet,
    ) -> Result<OutreachDraft, OutreachError> {
        // Phase 1: context assembly
        let contact = self.contact_repo
            .get_contact(&req.contact_id)
            .await
            .map_err(OutreachError::ContactLoad)?
            .ok_or(OutreachError::ContactNotFound(req.contact_id.clone()))?;

        let shared_context = SharedContextBuilder::compute(&contact, life_sheet);

        let (company_name, role_title) = if let Some(job_id) = &req.job_id {
            let job = self.job_repo
                .get_job(job_id)
                .await
                .map_err(OutreachError::JobLoad)?;
            match job {
                Some(j) => (j.company_name.clone(), j.title.clone()),
                None => (
                    contact.current_company.clone().unwrap_or_default(),
                    "the role".to_string(),
                ),
            }
        } else {
            (
                contact.current_company.clone().unwrap_or_default(),
                String::new(),
            )
        };

        // Phase 2: LLM drafting
        let user_background_summary = build_user_background_summary(life_sheet);
        let medium_label = medium_label_str(&req.medium);
        let medium_constraints = medium_constraints_str(&req.medium);
        let shared_context_json = serde_json::to_string(&shared_context)
            .map_err(|e| OutreachError::Serialization(e.to_string()))?;
        let user_notes_str = req.user_notes.clone().unwrap_or_default();

        let system_prompt = load_system_prompt();
        let user_prompt = build_user_prompt(
            &req.tone,
            &contact,
            &shared_context_json,
            &user_background_summary,
            &company_name,
            &role_title,
            &user_notes_str,
            &medium_label,
            &medium_constraints,
        );

        let messages = vec![
            ChatMessage { role: ChatRole::User, content: user_prompt },
        ];
        let chat_req = crate::llm::ChatRequest {
            model: None, // use provider default
            messages,
            system: Some(system_prompt),
            temperature: Some(0.4),
            max_tokens: Some(512),
        };

        let response = self.llm
            .complete(chat_req)
            .await
            .map_err(OutreachError::LlmError)?;
        let draft_text_raw = response.content.trim().to_string();

        // Phase 3: length enforcement and fabrication check
        let limits = req.medium.limits();
        let (draft_text, medium_limit_ok) =
            MediumLengthEnforcer::enforce(&draft_text_raw, &limits);
        let fabrication_warnings =
            OutreachFabricationChecker::check(&draft_text, &shared_context);

        if !fabrication_warnings.is_empty() {
            warn!(
                contact_id = %req.contact_id.0,
                warnings = ?fabrication_warnings,
                "Outreach draft has fabrication warnings"
            );
        }

        let char_count = MediumLengthEnforcer::count_chars(&draft_text);
        let word_count = MediumLengthEnforcer::count_words(&draft_text);

        info!(
            contact_id = %req.contact_id.0,
            medium = ?req.medium,
            tone = ?req.tone,
            char_count,
            medium_limit_ok,
            fabrication_warnings_count = fabrication_warnings.len(),
            "Outreach draft generated"
        );

        Ok(OutreachDraft {
            id: OutreachId::new(),
            request: req,
            shared_context,
            draft_text,
            char_count,
            word_count,
            medium_limit_ok,
            fabrication_warnings,
            generated_at: chrono::Utc::now(),
        })
    }

    async fn compute_shared_context(
        &self,
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Result<SharedContext, OutreachError> {
        Ok(SharedContextBuilder::compute(contact, life_sheet))
    }
}

fn build_user_background_summary(life_sheet: &LifeSheet) -> String {
    // 2–3 sentence summary from the 3 most recent experience entries
    life_sheet.experience.iter().take(3).map(|e| {
        format!("{} at {}", e.position, e.company)
    }).collect::<Vec<_>>().join("; ")
}

fn medium_label_str(medium: &OutreachMedium) -> &'static str {
    match medium {
        OutreachMedium::LinkedInConnectionNote => "LinkedIn connection note",
        OutreachMedium::LinkedInMessage => "LinkedIn message",
        OutreachMedium::Email => "email",
        OutreachMedium::ShortForm => "short message (SMS/Slack/DM)",
    }
}

fn medium_constraints_str(medium: &OutreachMedium) -> String {
    let limits = medium.limits();
    let mut parts = Vec::new();
    if let Some(mc) = limits.max_chars {
        parts.push(format!("max {} characters", mc));
    }
    if let Some(mw) = limits.min_words {
        parts.push(format!("min {} words", mw));
    }
    if let Some(mw) = limits.max_words {
        parts.push(format!("max {} words", mw));
    }
    parts.join(", ")
}

fn load_system_prompt() -> String {
    // Loaded from embedded TOML template at compile time
    include_str!("../../../lazyjob-llm/src/prompts/networking_outreach.toml")
        .parse::<toml::Value>()
        .ok()
        .and_then(|v| v["system"]["content"].as_str().map(str::to_string))
        .unwrap_or_default()
}

fn build_user_prompt(
    tone: &OutreachTone,
    contact: &ProfileContact,
    shared_context_json: &str,
    user_background_summary: &str,
    company_name: &str,
    role_title: &str,
    user_notes: &str,
    medium_label: &str,
    medium_constraints: &str,
) -> String {
    let template_key = match tone {
        OutreachTone::Warm => "user_warm",
        OutreachTone::Curious => "user_curious",
        OutreachTone::Casual => "user_casual",
        OutreachTone::Professional => "user_professional",
    };
    let template_content = include_str!("../../../lazyjob-llm/src/prompts/networking_outreach.toml")
        .parse::<toml::Value>()
        .ok()
        .and_then(|v| v[template_key]["content"].as_str().map(str::to_string))
        .unwrap_or_default();

    // Simple {var} substitution using SimpleTemplateEngine from the prompt template plan
    crate::llm::prompts::SimpleTemplateEngine::render(&template_content, &[
        ("contact_name", contact.full_name.as_str()),
        ("contact_current_title", contact.current_title.as_deref().unwrap_or("professional")),
        ("contact_current_company", contact.current_company.as_deref().unwrap_or("their company")),
        ("shared_context_json", shared_context_json),
        ("user_background_summary", user_background_summary),
        ("company_name", company_name),
        ("role_title", role_title),
        ("user_notes", if user_notes.is_empty() { "None" } else { user_notes }),
        ("medium_label", medium_label),
        ("medium_constraints", medium_constraints),
    ])
}
```

**Step 4.2 — `SqliteOutreachRepository` in `repo.rs`**

```rust
// lazyjob-core/src/networking/outreach/repo.rs

use sqlx::SqlitePool;
use async_trait::async_trait;

pub struct SqliteOutreachRepository {
    pool: SqlitePool,
}

impl SqliteOutreachRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl OutreachRepository for SqliteOutreachRepository {
    async fn save_draft(&self, draft: &OutreachDraft) -> Result<OutreachId, OutreachError> {
        let id_str = draft.id.0.to_string();
        let contact_id_str = draft.request.contact_id.0.to_string();
        let job_id_str = draft.request.job_id.as_ref().map(|j| j.0.to_string());
        let medium_str = draft.request.medium.to_db_str();
        let tone_str = draft.request.tone.to_db_str();
        let shared_ctx_json = serde_json::to_string(&draft.shared_context)
            .map_err(|e| OutreachError::Serialization(e.to_string()))?;
        let warnings_json = serde_json::to_string(&draft.fabrication_warnings)
            .map_err(|e| OutreachError::Serialization(e.to_string()))?;
        let medium_limit_ok_i = if draft.medium_limit_ok { 1i64 } else { 0i64 };
        let generated_at_str = draft.generated_at.to_rfc3339();

        let mut tx = self.pool.begin().await.map_err(OutreachError::Database)?;

        sqlx::query!(
            r#"
            INSERT INTO outreach_drafts
                (id, contact_id, job_id, medium, tone, draft_text,
                 shared_context_json, fabrication_warnings_json,
                 char_count, word_count, medium_limit_ok, generated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id_str, contact_id_str, job_id_str, medium_str, tone_str,
            draft.draft_text, shared_ctx_json, warnings_json,
            draft.char_count as i64, draft.word_count as i64,
            medium_limit_ok_i, generated_at_str
        )
        .execute(&mut *tx)
        .await
        .map_err(OutreachError::Database)?;

        // Update the cached last_draft_text on the contact row
        sqlx::query!(
            "UPDATE profile_contacts SET outreach_status = 'draft_generated', last_draft_text = ? WHERE id = ?",
            draft.draft_text, contact_id_str
        )
        .execute(&mut *tx)
        .await
        .map_err(OutreachError::Database)?;

        tx.commit().await.map_err(OutreachError::Database)?;
        Ok(draft.id.clone())
    }

    async fn mark_sent(
        &self,
        contact_id: &ContactId,
        sent_at: NaiveDate,
    ) -> Result<(), OutreachError> {
        let id_str = contact_id.0.to_string();
        let sent_str = sent_at.to_string();
        sqlx::query!(
            "UPDATE profile_contacts SET outreach_status = 'message_sent', outreach_sent_at = ? WHERE id = ?",
            sent_str, id_str
        )
        .execute(&self.pool)
        .await
        .map_err(OutreachError::Database)?;
        Ok(())
    }

    async fn mark_responded(
        &self,
        contact_id: &ContactId,
        responded_at: NaiveDate,
    ) -> Result<(), OutreachError> {
        let id_str = contact_id.0.to_string();
        let resp_str = responded_at.to_string();
        sqlx::query!(
            "UPDATE profile_contacts SET outreach_status = 'responded', outreach_responded_at = ? WHERE id = ?",
            resp_str, id_str
        )
        .execute(&self.pool)
        .await
        .map_err(OutreachError::Database)?;
        Ok(())
    }

    async fn mark_no_response(&self, contact_id: &ContactId) -> Result<(), OutreachError> {
        let id_str = contact_id.0.to_string();
        sqlx::query!(
            "UPDATE profile_contacts SET outreach_status = 'no_response' WHERE id = ?",
            id_str
        )
        .execute(&self.pool)
        .await
        .map_err(OutreachError::Database)?;
        Ok(())
    }

    async fn get_latest_draft(
        &self,
        contact_id: &ContactId,
    ) -> Result<Option<OutreachRecord>, OutreachError> {
        let id_str = contact_id.0.to_string();
        let row = sqlx::query!(
            r#"
            SELECT id, contact_id, job_id, medium, tone, draft_text,
                   shared_context_json, fabrication_warnings_json,
                   char_count, word_count, medium_limit_ok, generated_at
            FROM outreach_drafts
            WHERE contact_id = ?
            ORDER BY generated_at DESC
            LIMIT 1
            "#,
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(OutreachError::Database)?;

        Ok(row.map(|r| OutreachRecord {
            id: OutreachId(uuid::Uuid::parse_str(&r.id).unwrap()),
            contact_id: ContactId(uuid::Uuid::parse_str(&r.contact_id).unwrap()),
            job_id: r.job_id.and_then(|j| uuid::Uuid::parse_str(&j).ok().map(JobId)),
            medium: OutreachMedium::from_db_str(&r.medium),
            tone: OutreachTone::from_db_str(&r.tone),
            draft_text: r.draft_text,
            shared_context_json: r.shared_context_json,
            fabrication_warnings_json: r.fabrication_warnings_json,
            char_count: r.char_count,
            word_count: r.word_count,
            medium_limit_ok: r.medium_limit_ok != 0,
            generated_at: chrono::DateTime::parse_from_rfc3339(&r.generated_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_default(),
        }))
    }

    async fn list_drafts_for_contact(
        &self,
        contact_id: &ContactId,
    ) -> Result<Vec<OutreachRecord>, OutreachError> {
        // Similar to get_latest_draft but returns all rows, no LIMIT
        todo!("same structure as get_latest_draft, returns Vec")
    }

    async fn update_status(
        &self,
        contact_id: &ContactId,
        status: &OutreachStatus,
    ) -> Result<(), OutreachError> {
        let id_str = contact_id.0.to_string();
        let status_str = status.to_db_str();
        let sent_at = if let OutreachStatus::MessageSent { sent_at } = status {
            Some(sent_at.to_string())
        } else { None };
        let responded_at = if let OutreachStatus::Responded { responded_at } = status {
            Some(responded_at.to_string())
        } else { None };

        sqlx::query!(
            r#"
            UPDATE profile_contacts
            SET outreach_status = ?,
                outreach_sent_at = COALESCE(?, outreach_sent_at),
                outreach_responded_at = COALESCE(?, outreach_responded_at)
            WHERE id = ?
            "#,
            status_str, sent_at, responded_at, id_str
        )
        .execute(&self.pool)
        .await
        .map_err(OutreachError::Database)?;
        Ok(())
    }
}
```

---

### Phase 5 — Follow-up Reminder Scheduling

**Step 5.1 — Schedule follow-up when draft is marked sent**

When `OutreachRepository::mark_sent` is called, the repository inserts a follow-up reminder row in `outreach_follow_up_reminders` at `sent_at + 7 days`. The existing `ReminderPoller` from the application workflow plan polls this table (it queries `remind_at <= now() AND fired_at IS NULL`) and emits `ReminderDueEvent::OutreachFollowUp { contact_id, outreach_id }`.

```rust
// In SqliteOutreachRepository::mark_sent — after updating profile_contacts, in the same tx:
let remind_at = (sent_at + chrono::Duration::days(7)).to_string();
let reminder_id = Uuid::new_v4().to_string();
let outreach_id_str = ""; // caller provides outreach_id; pass it to mark_sent signature
sqlx::query!(
    "INSERT INTO outreach_follow_up_reminders (id, contact_id, outreach_id, remind_at, days_after)
     VALUES (?, ?, ?, ?, 7)",
    reminder_id, id_str, outreach_id_str, remind_at
)
.execute(&mut *tx)
.await
.map_err(OutreachError::Database)?;
```

Note: This requires updating `mark_sent` to also accept the `OutreachId` that was sent. Update the trait signature in Phase 5.

**Step 5.2 — Mark reminder fired atomically**

```sql
UPDATE outreach_follow_up_reminders
SET fired_at = datetime('now')
WHERE id = ? AND fired_at IS NULL;
```

---

### Phase 6 — TUI Outreach Draft View

**Step 6.1 — `OutreachDraftView` in `lazyjob-tui`**

The TUI view is triggered from the contact detail panel (key `d` → "Draft outreach"). It renders:

1. A form to select medium and tone (if not already provided by the warm-path suggestion).
2. The generated draft in a read-only scrollable `Paragraph` widget.
3. Status bar showing char count, word count, and a colored medium-limit indicator (green = ok, yellow = over soft limit, red = hard-clipped).
4. Fabrication warnings (if any) rendered as a yellow bordered `Block` below the draft.

Keybindings in `OutreachDraftView`:
- `y` / `ctrl+c` — copy draft text to system clipboard via `arboard` crate
- `s` — mark as sent (opens a date picker defaulting to today, calls `mark_sent`)
- `r` / `Enter` — re-generate draft (re-calls `draft()`, replaces current view)
- `q` / `Esc` — close view

```rust
// lazyjob-tui/src/views/networking/outreach_draft.rs

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub struct OutreachDraftView {
    pub draft: OutreachDraft,
    pub scroll_offset: u16,
}

impl OutreachDraftView {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // header (contact name, medium, tone)
                Constraint::Min(6),     // draft text
                Constraint::Length(3),  // status bar
                Constraint::Length(if self.draft.fabrication_warnings.is_empty() { 0 } else { 4 }), // warnings
            ])
            .split(area);

        // Header
        let header_text = format!(
            "Outreach draft for {} | {} | {}",
            "contact_name",   // resolved from draft.request.contact_id at render time
            medium_display(&self.draft.request.medium),
            tone_display(&self.draft.request.tone),
        );
        let header = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(header, chunks[0]);

        // Draft text (scrollable)
        let draft_para = Paragraph::new(self.draft.draft_text.as_str())
            .block(Block::default().borders(Borders::ALL).title("Draft"))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));
        frame.render_widget(draft_para, chunks[1]);

        // Status bar
        let limit_style = if self.draft.medium_limit_ok {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        };
        let status_line = Line::from(vec![
            Span::raw(format!(" {} chars | {} words ", self.draft.char_count, self.draft.word_count)),
            Span::styled(
                if self.draft.medium_limit_ok { "✓ within limit" } else { "✗ exceeds limit" },
                limit_style,
            ),
            Span::raw("  [y] copy  [s] mark sent  [r] regenerate  [q] close"),
        ]);
        frame.render_widget(Paragraph::new(status_line), chunks[2]);

        // Fabrication warnings (if any)
        if !self.draft.fabrication_warnings.is_empty() && chunks[3].height > 0 {
            let items: Vec<ListItem> = self.draft.fabrication_warnings.iter()
                .map(|w| ListItem::new(format!("⚠ {}", w))
                    .style(Style::default().fg(Color::Yellow)))
                .collect();
            let warn_list = List::new(items)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title("Fabrication Warnings")
                    .border_style(Style::default().fg(Color::Yellow)));
            frame.render_widget(warn_list, chunks[3]);
        }
    }

    pub fn handle_scroll(&mut self, delta: i16) {
        self.scroll_offset = self.scroll_offset.saturating_add_signed(delta);
    }
}
```

**Step 6.2 — Clipboard copy**

Add `arboard` to `lazyjob-tui` dependencies:
```toml
arboard = "3"
```

In the key handler for `y`:
```rust
let mut clipboard = arboard::Clipboard::new()?;
clipboard.set_text(self.draft.draft_text.clone())?;
// Show a brief "Copied!" flash in the status bar (1s via TUI tick)
self.show_copied_flash = true;
```

---

## Key Crate APIs

- `strsim::jaro_winkler(a: &str, b: &str) -> f64` — fuzzy company/school name matching
- `once_cell::sync::Lazy<regex::Regex>` — compile regexes once at startup for claim extraction
- `serde_json::to_string(&shared_context)` — serialize `SharedContext` for LLM prompt injection
- `sqlx::query!()` macro — compile-time checked queries against SQLite
- `sqlx::SqlitePool::begin()` → `tx.commit()` — atomic transaction for draft save + contact update
- `chrono::NaiveDate::from_ymd_opt(y, m, d)` — safe date construction for tenure computation
- `arboard::Clipboard::new()` + `.set_text(s)` — clipboard write for TUI `y` keybind
- `toml::Value` + `include_str!()` — load TOML prompt template at compile time
- `tracing::instrument(skip(self, life_sheet))` — structured span for the draft pipeline
- `async_trait::async_trait` — trait impl for async trait methods

## Error Handling

```rust
// lazyjob-core/src/networking/outreach/types.rs (or error.rs)

use thiserror::Error;
use crate::networking::ContactId;

#[derive(Debug, Error)]
pub enum OutreachError {
    #[error("contact not found: {0:?}")]
    ContactNotFound(ContactId),

    #[error("failed to load contact: {0}")]
    ContactLoad(#[source] anyhow::Error),

    #[error("failed to load job: {0}")]
    JobLoad(#[source] anyhow::Error),

    #[error("LLM provider error: {0}")]
    LlmError(#[source] anyhow::Error),

    #[error("template rendering error: {0}")]
    TemplateError(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

Callers in the TUI match on `OutreachError::ContactNotFound` to show a "contact not found" dialog and on `OutreachError::LlmError` to show a retry prompt. All other variants surface as a generic error banner.

## Testing Strategy

### Unit Tests

**`context.rs`** — pure sync functions, no mocking needed:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_employers_detected_on_exact_match() {
        let contact = mock_contact_with_company("Acme Corp");
        let life_sheet = mock_life_sheet_with_experience("Acme Corp");
        let ctx = SharedContextBuilder::compute(&contact, &life_sheet);
        assert_eq!(ctx.shared_employers.len(), 1);
        assert_eq!(ctx.shared_employers[0].company_name, "Acme Corp");
    }

    #[test]
    fn shared_employers_detected_on_fuzzy_match() {
        let contact = mock_contact_with_company("Acme Corp.");   // trailing period
        let life_sheet = mock_life_sheet_with_experience("Acme Corporation");
        let ctx = SharedContextBuilder::compute(&contact, &life_sheet);
        assert_eq!(ctx.shared_employers.len(), 1);
    }

    #[test]
    fn no_false_positives_for_unrelated_companies() {
        let contact = mock_contact_with_company("Google LLC");
        let life_sheet = mock_life_sheet_with_experience("Amazon");
        let ctx = SharedContextBuilder::compute(&contact, &life_sheet);
        assert!(ctx.shared_employers.is_empty());
    }

    #[test]
    fn has_genuine_hook_false_when_empty() {
        let ctx = SharedContext {
            shared_employers: vec![],
            shared_schools: vec![],
            shared_industry: None,
            contact_current_role_tenure_months: None,
        };
        assert!(!ctx.has_genuine_hook());
    }
}
```

**`fabrication.rs`**:
```rust
#[test]
fn flags_ungrounded_employer_claim() {
    let draft = "I remember our time at Initech fondly.";
    let ctx = SharedContext { shared_employers: vec![], shared_schools: vec![], ..Default::default() };
    let warnings = OutreachFabricationChecker::check(draft, &ctx);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Initech"));
}

#[test]
fn does_not_flag_grounded_claim() {
    let draft = "We both worked at Acme Corp back in the day.";
    let ctx = SharedContext {
        shared_employers: vec![SharedEmployer { company_name: "Acme Corp".into(), .. }],
        ..Default::default()
    };
    let warnings = OutreachFabricationChecker::check(draft, &ctx);
    assert!(warnings.is_empty());
}
```

**`length.rs`**:
```rust
#[test]
fn hard_clips_linkedin_connection_note() {
    let long_text = "a".repeat(350);
    let limits = OutreachMedium::LinkedInConnectionNote.limits();
    let (clipped, ok) = MediumLengthEnforcer::enforce(&long_text, &limits);
    assert!(!ok);
    assert!(clipped.ends_with('…'));
    assert!(clipped.chars().count() <= 300);
}

#[test]
fn within_limit_returns_original() {
    let text = "Hello, this is a short note.";
    let limits = OutreachMedium::LinkedInConnectionNote.limits();
    let (result, ok) = MediumLengthEnforcer::enforce(text, &limits);
    assert!(ok);
    assert_eq!(result, text);
}
```

### Integration Tests

Use `#[sqlx::test(migrations = "migrations")]` to auto-apply migration 016:

```rust
#[sqlx::test(migrations = "migrations")]
async fn save_and_retrieve_draft(pool: SqlitePool) {
    let repo = SqliteOutreachRepository::new(pool.clone());
    // Insert a contact first (migration ensures table exists)
    insert_test_contact(&pool, "contact-123").await;

    let draft = mock_outreach_draft("contact-123");
    let id = repo.save_draft(&draft).await.unwrap();

    let retrieved = repo.get_latest_draft(&draft.request.contact_id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().draft_text, draft.draft_text);
}

#[sqlx::test(migrations = "migrations")]
async fn mark_sent_updates_contact_status(pool: SqlitePool) {
    let repo = SqliteOutreachRepository::new(pool.clone());
    insert_test_contact(&pool, "contact-456").await;
    insert_test_draft(&pool, "contact-456", "draft-789").await;

    let contact_id = ContactId(Uuid::parse_str("contact-456").unwrap());
    repo.mark_sent(&contact_id, NaiveDate::from_ymd_opt(2026, 4, 16).unwrap())
        .await
        .unwrap();

    let row = sqlx::query!(
        "SELECT outreach_status, outreach_sent_at FROM profile_contacts WHERE id = 'contact-456'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.outreach_status, "message_sent");
    assert_eq!(row.outreach_sent_at, Some("2026-04-16".to_string()));
}
```

### LLM Integration Tests (wiremock)

```rust
#[tokio::test]
async fn draft_calls_llm_and_returns_valid_draft() {
    let mock_server = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_anthropic_response("Hi there!")))
        .mount(&mock_server)
        .await;

    let llm = AnthropicProvider::with_base_url(mock_server.uri());
    let service = LlmOutreachDraftingService { llm: Arc::new(llm), /* ... */ };
    let req = OutreachRequest {
        contact_id: ContactId::new(),
        job_id: None,
        medium: OutreachMedium::LinkedInMessage,
        tone: OutreachTone::Professional,
        user_notes: None,
    };
    let result = service.draft(req, &mock_life_sheet()).await;
    assert!(result.is_ok());
    assert!(!result.unwrap().draft_text.is_empty());
}
```

### TUI Tests

`OutreachDraftView::render` is a pure function — test it using `ratatui::backend::TestBackend`:

```rust
#[test]
fn renders_fabrication_warning_when_present() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let view = OutreachDraftView {
        draft: mock_draft_with_fabrication_warning("⚠ Unverified employer claim: Initech"),
        scroll_offset: 0,
    };
    terminal.draw(|f| view.render(f, f.area())).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Assert that "Fabrication Warnings" title appears
    let rendered = format!("{:?}", buffer);
    assert!(rendered.contains("Fabrication"));
}
```

## Open Questions

1. **Email subject line generation**: The spec mentions `email` medium but doesn't specify whether a subject line should be generated separately. Recommendation: for `Email` medium, the prompt template should output a `Subject: ...` line as the first line of the draft; the TUI strips it for display and char counting but shows it separately.

2. **Multi-message sequences**: The spec defers this to post-MVP (Phase 1 = single message only). The data model can support sequences by adding a `sequence_position INTEGER DEFAULT 0` column to `outreach_drafts`, but this should NOT be implemented until explicitly specced.

3. **Contact data freshness warning**: The spec recommends using "I see you're at [company]" vs. "You work at [company]". This is enforced via prompt instruction in the template. No code change required — but the prompt should be validated with a few test drafts to confirm the LLM obeys the hedged phrasing.

4. **Hunter.io enrichment (Phase 2)**: When the user wants a work email for a contact, add `OutreachService::lookup_work_email(contact_id, hunter_api_key)` that calls `https://api.hunter.io/v2/email-finder` via `reqwest`. Store API key in OS keyring under `lazyjob/hunter_api_key`. Return `Option<String>` — never block draft generation if the lookup fails.

5. **Clipboard on Linux headless**: `arboard` requires either X11 or Wayland. In headless SSH sessions (no display), `arboard::Clipboard::new()` will return an error. The TUI should catch this and fall back to printing the draft text to a temporary file path the user can `cat`.

## Related Specs

- [specs/networking-connection-mapping.md](networking-connection-mapping.md) — `ProfileContact`, `SuggestedApproach`, `ConnectionTier`, `normalize_company_name()`
- [specs/profile-life-sheet-data-model.md](profile-life-sheet-data-model.md) — `LifeSheet`, `Experience`, `Education`
- [specs/agentic-llm-provider-abstraction.md](agentic-llm-provider-abstraction.md) — `LlmProvider` async trait, `ChatRequest`, `ChatMessage`
- [specs/17-ralph-prompt-templates.md](17-ralph-prompt-templates.md) — `SimpleTemplateEngine`, TOML template infrastructure
- [specs/09-tui-design-keybindings.md](09-tui-design-keybindings.md) — TUI event loop, panel system, keybinding dispatch
- [specs/networking-referral-management.md](networking-referral-management.md) — referral request lifecycle that follows an outreach draft
