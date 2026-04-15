# Spec: Networking Outreach Drafting

**JTBD**: A-4 — Get warm introductions that beat cold applications
**Topic**: Generate personalized, research-grounded outreach message drafts for specific contacts at target companies
**Domain**: networking

---

## What

An LLM-powered drafting pipeline that produces a ready-to-send outreach message for a specific contact at a target company, calibrated to the user's relationship warmth, the contact's background, any shared context, and the target role. The user reviews, edits, copies, and sends the message manually through LinkedIn or email — LazyJob never sends messages automatically.

## Why

Personalized cold outreach achieves 30–45% acceptance rates vs. 3–5% for generic templates. The bottleneck is research time: reading the contact's profile, finding the genuine shared hook (same school, same former employer, same interest), and writing something that doesn't read as a template. LLM can compress this from 15 minutes per message to 60 seconds of review. The quality lever is the research grounding — fabricated shared context is worse than a generic message because it reads as deceptive.

**Anti-fabrication is non-negotiable**: 91% of recruiters have spotted candidate deception in applications (Greenhouse 2025). A message that invents a shared connection or misattributes a quote is toxic to the sender's reputation. Every claim in the generated message must be grounded in verifiable data.

## How

### Pipeline Architecture

`OutreachDraftingService` in `lazyjob-core/src/networking/outreach.rs` orchestrates three phases:

**Phase 1 — Context assembly**
- Pull `ProfileContact` from `ContactRepository`
- Pull `JobRecord` and `CompanyRecord` from `JobRepository` / `CompanyRepository`
- Pull relevant sections of the user's `LifeSheet` (work experience companies, school, skills)
- Compute `SharedContext` struct: overlapping employers, overlapping schools, mutual skills/industries

**Phase 2 — LLM drafting**
- Invoke `LlmProvider::complete` with the outreach prompt template
- Template lives at `lazyjob-llm/src/prompts/networking_outreach.md`
- Prompt includes: user background summary, contact background, shared context facts, target role, desired tone, medium constraints

**Phase 3 — Length enforcement and validation**
- Medium-specific length clipping:
  - `LinkedInConnectionNote`: hard cap 300 chars (LinkedIn enforces this)
  - `LinkedInMessage`: soft target 150–400 chars (research shows <400 chars performs 22% better)
  - `Email`: 100–300 words
  - `ShortForm`: ≤150 chars (SMS / Twitter DM / Slack)
- Fabrication check: extract all factual claims from draft → verify each against `SharedContext`. Any claim not grounded in imported data is flagged and removed or replaced with a hedge ("I believe we may have crossed paths at...").

### Shared Context Computation

```
SharedContext {
    shared_employers: Vec<SharedEmployer>,    // overlapping companies in contact + user history
    shared_schools: Vec<SharedSchool>,        // overlapping institutions
    shared_industry: Option<String>,          // if both worked in same industry
    contact_current_role_tenure_months: u32,  // how long they've been at this company
    role_relevance_note: Option<String>,      // why user's background is relevant to their domain
}
```

`SharedContext` is computed in `lazyjob-core/src/networking/context.rs` without LLM involvement — it is a pure structural comparison of the user's LifeSheet against the contact's profile data. This ensures the LLM is generating language around verified facts, not inventing facts.

### Tone Calibration

Four tone variants mapped to `SuggestedApproach` (from `networking-connection-mapping.md`):

| Suggested approach | Tone | Goal of message |
|---|---|---|
| `RequestReferral` | Warm, direct | Ask for a referral for a specific role |
| `InformationalInterview` | Curious, humble | Request a 20-minute conversation to learn about their role/company |
| `ReconnectFirst` | Casual, no ask | Re-establish contact with no immediate job ask |
| `ColdOutreach` | Professional, value-forward | Introduce self and establish relevance before any ask |

The LLM is instructed to **never include the actual job-ask in `ReconnectFirst` tone** — the goal is relationship re-warming, not immediate extraction.

### Anti-Fabrication Prompt Rules

The outreach prompt template enforces:
1. All factual claims must appear in the provided `SharedContext` JSON.
2. If no genuine shared context exists, use hedged language ("I've been following your work on X") rather than invented facts.
3. Never claim mutual connections unless explicitly listed in `SharedContext.mutual_connections` (populated only if user explicitly imports that data).
4. Never reference the contact's specific salary, personal details beyond their public profile, or internal company information.

### Output Status Tracking

After drafting, `ProfileContact.outreach_status` is updated to `DraftGenerated`. The user then manually marks it `MessageSent` via TUI action after they've sent it. LazyJob does not detect actual LinkedIn sends.

### No Automation Guarantee

LazyJob does not:
- Store or use LinkedIn credentials
- Open LinkedIn in a browser or WebView
- Send messages, connection requests, or emails on the user's behalf
- Bypass any platform's rate limits

The output is a text draft the user copies. This is a hard product constraint, not a future roadmap item.

## Interface

```rust
// lazyjob-core/src/networking/outreach.rs

pub enum OutreachMedium {
    LinkedInConnectionNote,  // ≤ 300 chars
    LinkedInMessage,         // ≤ 8000 chars, target < 400
    Email,                   // 100–300 words
    ShortForm,               // ≤ 150 chars
}

pub enum OutreachTone {
    Warm,          // for RequestReferral
    Curious,       // for InformationalInterview
    Casual,        // for ReconnectFirst
    Professional,  // for ColdOutreach
}

pub enum OutreachStatus {
    NotYetContacted,
    DraftGenerated,
    MessageSent { sent_at: NaiveDate },
    Responded { responded_at: NaiveDate },
    NoResponse,
    NotInterested,
}

pub struct OutreachRequest {
    pub contact_id: Uuid,
    pub job_id: Option<Uuid>,       // None for general reconnect (no specific role)
    pub medium: OutreachMedium,
    pub tone: OutreachTone,
    pub user_notes: Option<String>, // any additional context the user wants included
}

pub struct SharedContext {
    pub shared_employers: Vec<SharedEmployer>,
    pub shared_schools: Vec<SharedSchool>,
    pub shared_industry: Option<String>,
    pub contact_current_role_tenure_months: Option<u32>,
}

pub struct SharedEmployer {
    pub company_name: String,
    pub user_dates: (NaiveDate, Option<NaiveDate>),
    pub contact_dates: Option<(NaiveDate, Option<NaiveDate>)>,
    pub overlap_months: Option<u32>,  // None if no overlap (sequential tenures)
}

pub struct OutreachDraft {
    pub request: OutreachRequest,
    pub shared_context: SharedContext,
    pub draft_text: String,
    pub char_count: usize,
    pub word_count: usize,
    pub medium_limit_ok: bool,
    pub fabrication_warnings: Vec<String>,  // flags if any claim seems ungrounded
}

#[async_trait]
pub trait OutreachDraftingService: Send + Sync {
    async fn draft(&self, req: OutreachRequest, life_sheet: &LifeSheet) -> Result<OutreachDraft>;
    async fn compute_shared_context(
        &self,
        contact: &ProfileContact,
        life_sheet: &LifeSheet,
    ) -> Result<SharedContext>;
}

pub struct LlmOutreachDraftingService {
    llm: Arc<dyn LlmProvider>,
    contact_repo: Arc<dyn ContactRepository>,
    job_repo: Arc<dyn JobRepository>,
    company_repo: Arc<dyn CompanyRepository>,
}
```

```sql
-- Migration: add outreach_status to profile_contacts
ALTER TABLE profile_contacts
  ADD COLUMN outreach_status TEXT NOT NULL DEFAULT 'not_yet_contacted',
  ADD COLUMN outreach_sent_at DATE,
  ADD COLUMN outreach_responded_at DATE,
  ADD COLUMN last_draft_text TEXT;
```

## Open Questions

- **Should LazyJob store draft history?** If the user regenerates a draft, should old versions be kept? Recommendation: store only the most recent draft text (`last_draft_text` column) — draft versioning adds complexity with low user value.
- **Email address sourcing**: The contact's LinkedIn-exported email is often a personal address (GMail) not their work email. Work email is more professional for outreach but requires external lookup (Hunter.io). Phase 1: use whatever email is in the CSV. Phase 2: offer Hunter.io lookup as an optional enrichment (user provides API key, stored in OS keychain).
- **Multi-message sequences**: Some networking advice recommends a 2–3 message sequence (intro → follow-up → final). Should LazyJob generate a sequence or just a single message? Phase 1: single message only. Multi-message sequencing risks crossing into spam territory and adds state management complexity.
- **Contact data freshness**: Contact's current company from a 2-year-old LinkedIn CSV export may be wrong. LazyJob has no way to verify. Should the draft include a hedge? Recommendation: prompt template should use "I see you're at [company]" (present observation) rather than "You work at [company]" (assertion).

## Implementation Tasks

- [ ] Implement `SharedContext` computation in `lazyjob-core/src/networking/context.rs` using pure LifeSheet ↔ ProfileContact structural comparison (no LLM)
- [ ] Write outreach prompt template at `lazyjob-llm/src/prompts/networking_outreach.md` with anti-fabrication rules, tone variants, and medium-specific length instructions
- [ ] Implement `LlmOutreachDraftingService::draft` in `lazyjob-core/src/networking/outreach.rs` with three-phase pipeline (context assembly → LLM → length/fabrication validation)
- [ ] Implement medium-specific char/word count enforcement and `medium_limit_ok` flag; for `LinkedInConnectionNote`, hard-clip at 300 chars with ellipsis warning
- [ ] Add `outreach_status`, `outreach_sent_at`, `outreach_responded_at`, `last_draft_text` columns to `profile_contacts` DDL
- [ ] Add TUI outreach draft view (`lazyjob-tui/src/views/networking/outreach_draft.rs`): show draft text in editable textarea, char count, copy-to-clipboard action (`y`), mark-sent action (`s`)
- [ ] Add `OutreachStatus` transitions to `ContactRepository`: `mark_sent(contact_id)`, `mark_responded(contact_id)`, `mark_no_response(contact_id)`
