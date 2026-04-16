# Implementation Plan: Networking & Outreach Gap Closure

## Status
Draft

## Related Spec
[specs/07-gaps-networking-outreach.md](07-gaps-networking-outreach.md)

## Overview

This plan closes all 9 identified gaps (GAP-69 through GAP-77) and 2 cross-spec concerns
(Cross-Spec P: Contact ↔ LifeSheet, Cross-Spec Q: Outreach ↔ Application) in the networking
and outreach subsystem. It also includes the design for the two highest-priority new specs
(`XX-contact-multi-source-import.md` and `XX-relationship-decay-modeling.md`) identified in the
gap analysis.

The existing networking foundation is strong: `ConnectionMapper`, `RelationshipStage` state
machine, `OutreachFabricationChecker`, and `NetworkingReminderPoller` are all well-specified.
These gaps address the surrounding ecosystem — contact data completeness, identity resolution,
interaction tracking, analytics attribution, and outreach quality feedback — that transforms the
networking layer from a referral-request tool into a full relationship-management system.

Implementation is organized into four phases: Phase 1 addresses the two Critical gaps (multi-source
import, identity resolution), Phase 2 closes the Important gaps (relationship decay, warm path
expansion, LinkedIn policy guardrails), Phase 3 closes Moderate gaps (analytics, interaction
logging, cadence recommendations, outreach quality scoring), and Phase 4 resolves the cross-spec
data model conflicts.

## Prerequisites

### Must be implemented first
- `specs/networking-connection-mapping-implementation-plan.md` — `ProfileContact`, `ContactRepository`, `ConnectionTier`, `profile_contacts` table
- `specs/networking-referral-management-implementation-plan.md` — `RelationshipStage`, `NetworkingReminderPoller`, `referral_asks` table
- `specs/networking-outreach-drafting-implementation-plan.md` — `OutreachDraft`, `OutreachRepository`, `SharedContextBuilder`
- `specs/networking-referrals-agentic-implementation-plan.md` — `WarmPathFinder`, `RelationshipHealthScorer`
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `LifeSheetId`, `LifeSheetRepository`, `life_sheet_experience`
- `specs/04-sqlite-persistence-implementation-plan.md` — `run_migrations`, `DbPool`
- `specs/09-tui-design-keybindings-implementation-plan.md` — TUI panel/overlay system

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# Multi-source import:
csv              = "1.3"        # LinkedIn CSV + generic CSV parsing (likely already present)
vcard4           = "0.6"        # vCard 4.0 (.vcf) parsing
# Identity resolution:
strsim           = "0.11"       # jaro_winkler for name/company fuzzy match (likely present)
# Analytics:
chrono           = { version = "0.4", features = ["serde"] }   # already present
# Outreach quality:
once_cell        = "1.19"       # Lazy<Regex> (already present)
regex            = "1.10"       # AI-phrase detection patterns (already present)
# All other deps (uuid, serde, serde_json, sqlx, thiserror, anyhow, tokio,
# async-trait, tracing) already declared from prior modules.
```

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `ContactImporter` trait + implementations | `lazyjob-core` | `src/networking/import/` |
| `ContactDeduplicator` (identity resolution) | `lazyjob-core` | `src/networking/dedup/` |
| `RelationshipDecayModel` | `lazyjob-core` | `src/networking/decay/` |
| `WarmPathExpander` (community/event paths) | `lazyjob-core` | `src/networking/expansion/` |
| `InteractionLogger` (non-outreach events) | `lazyjob-core` | `src/networking/interactions/` |
| `NetworkingAnalyticsService` | `lazyjob-core` | `src/networking/analytics/` |
| `TouchpointCadenceAdvisor` | `lazyjob-core` | `src/networking/cadence/` |
| `OutreachQualityScorer` | `lazyjob-core` | `src/networking/quality/` |
| `ContactLifeSheetLinker` (Cross-Spec P) | `lazyjob-core` | `src/networking/life_sheet_link.rs` |
| `ReferralApplicationLinker` (Cross-Spec Q) | `lazyjob-core` | `src/networking/referral_application.rs` |
| SQLite migrations (019–021) | `lazyjob-core` | `migrations/019_*`, `020_*`, `021_*` |
| TUI import wizard | `lazyjob-tui` | `src/views/networking/import_wizard.rs` |
| TUI dedup review panel | `lazyjob-tui` | `src/views/networking/dedup_review.rs` |
| TUI interaction logger overlay | `lazyjob-tui` | `src/views/networking/log_interaction.rs` |
| TUI analytics dashboard | `lazyjob-tui` | `src/views/networking/analytics.rs` |
| TUI decay heat map | `lazyjob-tui` | `src/views/networking/decay_view.rs` |

### Core Types

```rust
// lazyjob-core/src/networking/import/types.rs

/// Canonical import source label stored in profile_contacts.contact_source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactSource {
    LinkedInCsv,
    VCard,
    GenericCsv,
    ManualEntry,
    LifeSheetColleague,   // Promoted from LifeSheet past experience
}

/// Parsed contact before SQLite write. All fields optional to accommodate
/// incomplete import formats (e.g., vCard missing company/title).
#[derive(Debug, Clone)]
pub struct RawContact {
    pub full_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub linkedin_url: Option<String>,
    pub notes: Option<String>,
    pub source: ContactSource,
}

/// Result of one import run.
#[derive(Debug)]
pub struct ImportReport {
    pub inserted: usize,
    pub merged: usize,
    pub skipped_duplicates: usize,
    pub errors: Vec<ImportError>,
}

// lazyjob-core/src/networking/dedup/types.rs

/// A candidate duplicate pair surface for user review.
#[derive(Debug)]
pub struct DuplicatePair {
    pub existing: ProfileContact,
    pub candidate: RawContact,
    pub confidence: f32,           // 0.0..=1.0
    pub match_reason: MatchReason,
}

#[derive(Debug, Clone)]
pub enum MatchReason {
    ExactEmail,
    ExactName,
    FuzzyNamePlusCompany { name_score: f32, company_score: f32 },
    FuzzyNamePlusLinkedinUrl,
}

/// User decision for a dedup pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupDecision {
    Merge,
    KeepBoth,
    SkipImport,
}

// lazyjob-core/src/networking/decay/types.rs

/// Computed relationship health score (not persisted; computed at query time
/// from interaction log + elapsed time). Stored in relationship_health via
/// RelationshipHealthLoop for the TUI heat-map.
#[derive(Debug, Clone)]
pub struct RelationshipHealth {
    pub contact_id: uuid::Uuid,
    pub score: u8,                 // 0..=100
    pub decay_status: DecayStatus,
    pub days_since_interaction: i64,
    pub computed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DecayStatus {
    Healthy,      // score >= 70
    Warning,      // 40..70
    Stale,        // 15..40
    Dormant,      // < 15
}

// lazyjob-core/src/networking/interactions/types.rs

/// Every recorded touch point with a contact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InteractionRecord {
    pub id: uuid::Uuid,
    pub contact_id: uuid::Uuid,
    pub interaction_type: InteractionType,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub quality: InteractionQuality,
    pub notes: Option<String>,
    pub logged_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionType {
    PhoneCall,
    VideoChat,
    CoffeeInPerson,
    ConferenceMeeting,
    EmailReply,
    LinkedInReply,
    LinkedInCommentExchange,
    OutreachSent,   // Populated automatically when mark_sent() is called
    ReferralAsked,  // Populated automatically when referral_asks updated
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum InteractionQuality {
    Substantive,  // Call, meeting, coffee, in-depth exchange
    Light,        // Quick reply, comment, DM
    Automated,    // System-generated (OutreachSent, etc.)
}

// lazyjob-core/src/networking/analytics/types.rs

/// Aggregated outreach funnel for the analytics dashboard.
#[derive(Debug, Clone)]
pub struct OutreachFunnel {
    pub total_drafted: usize,
    pub total_sent: usize,         // outreach_status = 'sent'
    pub total_replied: usize,      // contacts with RelationshipStage >= Replied
    pub total_warmed: usize,       // contacts with RelationshipStage >= Warmed
    pub total_referrals_asked: usize,
    pub total_referrals_succeeded: usize,
    pub response_rate: f32,        // replied / sent, display-only f32
    pub warm_rate: f32,            // warmed / replied
    pub referral_success_rate: f32,
}

/// Per-channel (LinkedIn, Email, InMail, etc.) breakdown.
#[derive(Debug, Clone)]
pub struct ChannelMetrics {
    pub medium: OutreachMedium,    // from OutreachDraft.medium (existing type)
    pub sent: usize,
    pub replied: usize,
    pub response_rate: f32,
    pub median_response_days: Option<f32>,
}

/// Networking attribution: did this contact lead to a hire?
#[derive(Debug, Clone)]
pub struct ReferralAttributionRecord {
    pub referral_ask_id: uuid::Uuid,
    pub application_id: uuid::Uuid,
    pub outcome: AttributionOutcome,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AttributionOutcome {
    Hired,
    OfferedNotAccepted,
    Interviewed,
    Rejected,
    Unknown,
}

// lazyjob-core/src/networking/quality/types.rs

/// Outreach quality score. Not persisted — recomputed on demand.
#[derive(Debug, Clone)]
pub struct OutreachQualityReport {
    pub draft_id: uuid::Uuid,
    pub personalization_score: u8,  // 0..=100
    pub length_ok: bool,
    pub ai_signature_risk: AiSignatureRisk,
    pub flags: Vec<QualityFlag>,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AiSignatureRisk {
    Low,      // No detected AI-signature phrases
    Medium,   // 1-2 detected phrases
    High,     // 3+ detected phrases
}

#[derive(Debug, Clone)]
pub enum QualityFlag {
    TooLong { limit: usize, actual: usize },
    TooShort { minimum: usize, actual: usize },
    NoPersonalization,          // SharedContext has items but none referenced
    ContainsAiPhrase(String),   // e.g., "I hope this message finds you well"
    RepetitiveSentenceOpeners,
}

// lazyjob-core/src/networking/decay/model.rs

/// Pure function: compute health score from tier, interaction history, and tenure.
pub struct DecayModel;

impl DecayModel {
    /// Compute relationship health (0..=100) from:
    /// - `days_since_interaction`: days elapsed since last recorded interaction
    /// - `tier`: ConnectionTier (higher tier = slower decay)
    /// - `interaction_quality_recent`: quality of the most recent interaction
    ///
    /// Formula:
    ///   base_score = 100 * e^(-days / half_life(tier))
    ///   quality_boost = if substantive { +10.min(base_score) } else { 0 }
    ///   clamped to [0, 100]
    pub fn compute(
        days_since_interaction: i64,
        tier: &ConnectionTier,
        last_quality: InteractionQuality,
    ) -> RelationshipHealth;

    /// Half-life in days by tier.
    fn half_life(tier: &ConnectionTier) -> f64 {
        match tier {
            ConnectionTier::FirstDegreeCurrentEmployee => 90.0,
            ConnectionTier::FirstDegreeFormerColleague => 60.0,
            ConnectionTier::FirstDegreeAlumni => 45.0,
            ConnectionTier::SecondDegree => 30.0,
            ConnectionTier::Cold => 20.0,
        }
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/networking/import/mod.rs

#[async_trait::async_trait]
pub trait ContactImporter: Send + Sync {
    /// Parse raw bytes (file content) into a list of RawContacts.
    /// Does NOT write to SQLite — returns data for caller to dedup and insert.
    async fn parse(&self, bytes: &[u8]) -> Result<Vec<RawContact>, ImportError>;

    /// User-visible format name, e.g. "LinkedIn CSV", "vCard (.vcf)".
    fn format_name(&self) -> &'static str;
}

// lazyjob-core/src/networking/dedup/mod.rs

pub trait ContactDeduplicator: Send + Sync {
    /// Find duplicate pairs between `candidates` and existing `contacts`.
    /// Pure function — does NOT read SQLite.
    fn find_duplicates(
        &self,
        candidates: &[RawContact],
        existing: &[ProfileContact],
    ) -> Vec<DuplicatePair>;

    /// Merge a RawContact into an existing ProfileContact.
    /// Source wins only when existing field is None (COALESCE semantics).
    fn merge(&self, existing: &ProfileContact, candidate: &RawContact) -> ProfileContact;
}

// lazyjob-core/src/networking/quality/mod.rs

pub trait OutreachQualityScorer: Send + Sync {
    /// Score a draft without calling the LLM. Pure sync.
    fn score(&self, draft: &OutreachDraft, shared_context: &SharedContext) -> OutreachQualityReport;
}
```

### SQLite Schema

```sql
-- migrations/019_networking_gaps_phase1.sql

-- Extended contact columns for multi-source import (GAP-69, GAP-74)
ALTER TABLE profile_contacts ADD COLUMN phone TEXT;
ALTER TABLE profile_contacts ADD COLUMN linkedin_url TEXT;
ALTER TABLE profile_contacts ADD COLUMN vcard_uid TEXT;  -- stable vCard UID for dedup
ALTER TABLE profile_contacts ADD COLUMN life_sheet_link_id TEXT;  -- FK to life_sheet_experience

-- Dedup decisions table: records user-confirmed merge decisions to prevent re-prompting
CREATE TABLE IF NOT EXISTS contact_dedup_decisions (
    id            TEXT PRIMARY KEY,  -- uuid
    existing_id   TEXT NOT NULL REFERENCES profile_contacts(id),
    candidate_key TEXT NOT NULL,     -- SHA-256(name + email + company) of the rejected RawContact
    decision      TEXT NOT NULL,     -- 'merge' | 'keep_both' | 'skip_import'
    decided_at    TEXT NOT NULL      -- ISO8601
);
CREATE INDEX IF NOT EXISTS idx_dedup_decisions_candidate ON contact_dedup_decisions(candidate_key);

-- Non-outreach interaction log (GAP-75)
CREATE TABLE IF NOT EXISTS contact_interactions (
    id               TEXT PRIMARY KEY,  -- uuid
    contact_id       TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    interaction_type TEXT NOT NULL,     -- see InteractionType variants
    quality          TEXT NOT NULL,     -- 'substantive' | 'light' | 'automated'
    occurred_at      TEXT NOT NULL,     -- ISO8601
    notes            TEXT,
    logged_at        TEXT NOT NULL      -- ISO8601
);
CREATE INDEX IF NOT EXISTS idx_interactions_contact ON contact_interactions(contact_id, occurred_at DESC);

-- migrations/020_networking_gaps_phase2.sql

-- Outreach analytics attribution (Cross-Spec Q, GAP-73)
CREATE TABLE IF NOT EXISTS referral_attribution (
    id               TEXT PRIMARY KEY,  -- uuid
    referral_ask_id  TEXT NOT NULL REFERENCES referral_asks(id),
    application_id   TEXT NOT NULL REFERENCES applications(id),
    outcome          TEXT NOT NULL,     -- AttributionOutcome
    recorded_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_referral_attribution_ask
    ON referral_attribution(referral_ask_id);

-- Outreach quality scores (GAP-77) cached to avoid re-scoring on each render
CREATE TABLE IF NOT EXISTS outreach_quality_cache (
    draft_id              TEXT PRIMARY KEY REFERENCES outreach_drafts(id) ON DELETE CASCADE,
    personalization_score INTEGER NOT NULL,
    ai_signature_risk     TEXT NOT NULL,   -- 'low' | 'medium' | 'high'
    flags_json            TEXT NOT NULL,   -- JSON array of QualityFlag
    suggestions_json      TEXT NOT NULL,   -- JSON array of String
    scored_at             TEXT NOT NULL
);

-- migrations/021_networking_gaps_phase3.sql

-- Touchpoint cadence overrides (GAP-76): user can override default tier cadence per contact
CREATE TABLE IF NOT EXISTS contact_cadence_overrides (
    contact_id      TEXT PRIMARY KEY REFERENCES profile_contacts(id) ON DELETE CASCADE,
    days_interval   INTEGER NOT NULL,  -- e.g., 30 = reach out every 30 days
    quiet_until     TEXT,              -- ISO8601 snooze date
    updated_at      TEXT NOT NULL
);

-- Warm path expansion hints (GAP-72): LLM-generated suggestions stored for TUI browsing
CREATE TABLE IF NOT EXISTS warm_path_expansion_hints (
    id              TEXT PRIMARY KEY,  -- uuid
    job_id          TEXT NOT NULL REFERENCES jobs(id),
    hint_type       TEXT NOT NULL,     -- 'event' | 'community' | 'alumni' | 'content_engage'
    title           TEXT NOT NULL,
    description     TEXT NOT NULL,
    url             TEXT,
    confidence      REAL NOT NULL,
    generated_at    TEXT NOT NULL,
    dismissed_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_expansion_hints_job ON warm_path_expansion_hints(job_id, dismissed_at);
```

### Module Structure

```
lazyjob-core/
  src/
    networking/
      mod.rs
      import/
        mod.rs
        types.rs
        linkedin_csv.rs          # LinkedInCsvImporter
        vcard.rs                 # VCardImporter
        generic_csv.rs           # GenericCsvImporter
        service.rs               # ContactImportService (orchestrates parse + dedup + write)
      dedup/
        mod.rs
        types.rs
        fuzzy.rs                 # FuzzyContactDeduplicator
        merge.rs                 # field-level merge logic
      decay/
        mod.rs
        types.rs
        model.rs                 # DecayModel (pure fn)
        repository.rs            # reads contact_interactions, writes relationship_health
      interactions/
        mod.rs
        types.rs
        repository.rs            # SqliteInteractionRepository
        service.rs               # InteractionLogger (+ automatic system interactions)
      analytics/
        mod.rs
        types.rs
        queries.rs               # raw SQL aggregation queries
        service.rs               # NetworkingAnalyticsService
      cadence/
        mod.rs
        types.rs
        advisor.rs               # TouchpointCadenceAdvisor
      quality/
        mod.rs
        types.rs
        scorer.rs                # HeuristicOutreachQualityScorer
        patterns.rs              # AI_SIGNATURE_PHRASES: Lazy<Vec<Regex>>
      expansion/
        mod.rs
        types.rs
        service.rs               # WarmPathExpander (wraps LLM loop results)
      life_sheet_link.rs         # ContactLifeSheetLinker
      referral_application.rs    # ReferralApplicationLinker

lazyjob-tui/
  src/
    views/
      networking/
        import_wizard.rs
        dedup_review.rs
        log_interaction.rs
        analytics.rs
        decay_view.rs
        expansion_hints.rs
```

---

## Implementation Phases

### Phase 1 — Critical Gaps: Multi-Source Import + Identity Resolution (GAP-69, GAP-74)

**Step 1.1 — `LinkedInCsvImporter`** (`src/networking/import/linkedin_csv.rs`)

```rust
pub struct LinkedInCsvImporter;

#[async_trait::async_trait]
impl ContactImporter for LinkedInCsvImporter {
    async fn parse(&self, bytes: &[u8]) -> Result<Vec<RawContact>, ImportError> {
        tokio::task::spawn_blocking({
            let bytes = bytes.to_vec();
            move || {
                let mut rdr = csv::ReaderBuilder::new()
                    .has_headers(true)
                    .flexible(true)
                    .from_reader(bytes.as_slice());
                let headers = rdr.headers()
                    .map_err(ImportError::CsvParse)?
                    .clone();

                // Column lookup by name — tolerant of column reordering
                let col_idx = |name: &str| {
                    headers.iter().position(|h| h.trim().eq_ignore_ascii_case(name))
                };
                let first_name_idx   = col_idx("First Name");
                let last_name_idx    = col_idx("Last Name");
                let email_idx        = col_idx("Email Address");
                let company_idx      = col_idx("Company");
                let position_idx     = col_idx("Position");
                let connected_on_idx = col_idx("Connected On");

                let mut contacts = Vec::new();
                for result in rdr.records() {
                    let record = result.map_err(ImportError::CsvParse)?;
                    let get = |idx: Option<usize>| -> Option<String> {
                        idx.and_then(|i| record.get(i))
                           .map(str::trim)
                           .filter(|s| !s.is_empty())
                           .map(str::to_owned)
                    };
                    let first = get(first_name_idx).unwrap_or_default();
                    let last = get(last_name_idx).unwrap_or_default();
                    let full_name = format!("{first} {last}").trim().to_owned();
                    if full_name.is_empty() { continue; }
                    contacts.push(RawContact {
                        full_name,
                        email: get(email_idx),
                        phone: None,
                        company: get(company_idx),
                        title: get(position_idx),
                        linkedin_url: None,
                        notes: get(connected_on_idx)
                            .map(|d| format!("LinkedIn connected: {d}")),
                        source: ContactSource::LinkedInCsv,
                    });
                }
                Ok(contacts)
            }
        })
        .await
        .map_err(|_| ImportError::SpawnBlocking)?
    }

    fn format_name(&self) -> &'static str { "LinkedIn CSV" }
}
```

**Step 1.2 — `VCardImporter`** (`src/networking/import/vcard.rs`)

Use `vcard4` crate. Parse `.vcf` bytes, extract `FN`, `EMAIL`, `TEL`, `ORG`, `TITLE`, `URL` (for LinkedIn), `UID` properties. The `UID` is stored as `vcard_uid` in `profile_contacts` for stable dedup on re-import.

```rust
pub struct VCardImporter;

#[async_trait::async_trait]
impl ContactImporter for VCardImporter {
    async fn parse(&self, bytes: &[u8]) -> Result<Vec<RawContact>, ImportError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ImportError::Encoding)?;
        tokio::task::spawn_blocking({
            let text = text.to_owned();
            move || {
                let vcards = vcard4::parse(&text)
                    .map_err(|e| ImportError::VCardParse(e.to_string()))?;
                let mut contacts = Vec::new();
                for vcard in vcards {
                    let full_name = vcard.fn_property
                        .map(|p| p.value)
                        .unwrap_or_default();
                    if full_name.is_empty() { continue; }
                    let email = vcard.email.into_iter().next()
                        .map(|p| p.value);
                    let phone = vcard.telephone.into_iter().next()
                        .map(|p| p.value.to_string());
                    let company = vcard.organization.into_iter().next()
                        .and_then(|p| p.value.into_iter().next());
                    let title = vcard.title.into_iter().next()
                        .map(|p| p.value);
                    let linkedin_url = vcard.url.into_iter()
                        .find(|p| p.value.as_str().contains("linkedin.com"))
                        .map(|p| p.value.to_string());
                    contacts.push(RawContact {
                        full_name,
                        email,
                        phone,
                        company,
                        title,
                        linkedin_url,
                        notes: None,
                        source: ContactSource::VCard,
                    });
                }
                Ok(contacts)
            }
        })
        .await
        .map_err(|_| ImportError::SpawnBlocking)?
    }

    fn format_name(&self) -> &'static str { "vCard (.vcf)" }
}
```

**Step 1.3 — `FuzzyContactDeduplicator`** (`src/networking/dedup/fuzzy.rs`)

```rust
use once_cell::sync::Lazy;
use strsim::jaro_winkler;

static AUTO_MERGE_THRESHOLD: f32 = 0.97;  // email exact OR very high name+company score
static REVIEW_THRESHOLD: f32 = 0.82;      // surface for user review

pub struct FuzzyContactDeduplicator;

impl ContactDeduplicator for FuzzyContactDeduplicator {
    fn find_duplicates(
        &self,
        candidates: &[RawContact],
        existing: &[ProfileContact],
    ) -> Vec<DuplicatePair> {
        let mut pairs = Vec::new();
        for candidate in candidates {
            for contact in existing {
                // Tier 1: exact email match
                if let (Some(c_email), Some(e_email)) = (&candidate.email, &contact.email) {
                    if c_email.to_lowercase() == e_email.to_lowercase() {
                        pairs.push(DuplicatePair {
                            existing: contact.clone(),
                            candidate: candidate.clone(),
                            confidence: 1.0,
                            match_reason: MatchReason::ExactEmail,
                        });
                        continue;
                    }
                }
                // Tier 2: exact name (case-insensitive)
                if candidate.full_name.to_lowercase() == contact.full_name.to_lowercase() {
                    pairs.push(DuplicatePair {
                        existing: contact.clone(),
                        candidate: candidate.clone(),
                        confidence: 0.90,
                        match_reason: MatchReason::ExactName,
                    });
                    continue;
                }
                // Tier 3: fuzzy name + company
                let name_score = jaro_winkler(
                    &candidate.full_name.to_lowercase(),
                    &contact.full_name.to_lowercase(),
                ) as f32;
                let company_score = match (&candidate.company, &contact.company) {
                    (Some(c), Some(e)) => jaro_winkler(
                        &normalize_company_name(c),
                        &normalize_company_name(e),
                    ) as f32,
                    _ => 0.0,
                };
                let combined = name_score * 0.7 + company_score * 0.3;
                if combined >= REVIEW_THRESHOLD {
                    pairs.push(DuplicatePair {
                        existing: contact.clone(),
                        candidate: candidate.clone(),
                        confidence: combined,
                        match_reason: MatchReason::FuzzyNamePlusCompany {
                            name_score,
                            company_score,
                        },
                    });
                }
            }
        }
        // Sort by confidence descending
        pairs.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        pairs
    }

    fn merge(&self, existing: &ProfileContact, candidate: &RawContact) -> ProfileContact {
        ProfileContact {
            email:        existing.email.clone().or_else(|| candidate.email.clone()),
            phone:        existing.phone.clone().or_else(|| candidate.phone.clone()),
            company:      existing.company.clone().or_else(|| candidate.company.clone()),
            title:        existing.title.clone().or_else(|| candidate.title.clone()),
            linkedin_url: existing.linkedin_url.clone()
                             .or_else(|| candidate.linkedin_url.clone()),
            ..existing.clone()
        }
    }
}
```

**Step 1.4 — `ContactImportService`** (`src/networking/import/service.rs`)

High-level orchestrator:

```rust
pub struct ContactImportService {
    pool: Arc<DbPool>,
    deduplicator: Arc<dyn ContactDeduplicator>,
}

impl ContactImportService {
    /// Full import pipeline:
    ///   1. parse bytes with the given importer
    ///   2. load existing contacts from SQLite
    ///   3. find duplicates
    ///   4. auto-merge high-confidence pairs (confidence >= 0.97)
    ///   5. return remaining candidates + pairs needing user review
    pub async fn prepare_import(
        &self,
        importer: &dyn ContactImporter,
        bytes: &[u8],
    ) -> Result<PreparedImport, ImportError>;

    /// Called after user resolves all DuplicatePair decisions in TUI.
    /// Applies decisions: merge, insert, or skip.
    pub async fn apply_decisions(
        &self,
        prepared: PreparedImport,
        decisions: Vec<(uuid::Uuid, DedupDecision)>,
    ) -> Result<ImportReport, ImportError>;
}

pub struct PreparedImport {
    pub auto_merged:    Vec<(ProfileContact, RawContact)>,  // already resolved
    pub to_insert:      Vec<RawContact>,                     // unique, no match found
    pub needs_review:   Vec<DuplicatePair>,                  // user must decide
}
```

**Verification:**
- `cargo test -p lazyjob-core networking::import` with a real LinkedIn CSV fixture
- Confirmed: duplicate email maps to `ExactEmail`, same name different company maps to `FuzzyNamePlusCompany`, confidence < 0.82 is not surfaced

---

### Phase 2 — Important Gaps: Relationship Decay, Warm Path Expansion, LinkedIn Policy (GAP-70, GAP-71, GAP-72)

**Step 2.1 — `DecayModel` and `RelationshipHealthRepository`** (`src/networking/decay/`)

`DecayModel::compute()` is a pure sync function — no I/O. The exponential half-life formula:

```rust
impl DecayModel {
    pub fn compute(
        days_since_interaction: i64,
        tier: &ConnectionTier,
        last_quality: InteractionQuality,
    ) -> RelationshipHealth {
        let half_life = Self::half_life(tier);
        let base = 100.0 * (-0.693 * days_since_interaction as f64 / half_life).exp();
        let boost: f64 = match last_quality {
            InteractionQuality::Substantive => 10.0,
            InteractionQuality::Light       =>  3.0,
            InteractionQuality::Automated   =>  0.0,
        };
        let score = (base + boost).clamp(0.0, 100.0) as u8;
        let decay_status = match score {
            70..=100 => DecayStatus::Healthy,
            40..=69  => DecayStatus::Warning,
            15..=39  => DecayStatus::Stale,
            _        => DecayStatus::Dormant,
        };
        RelationshipHealth {
            contact_id: uuid::Uuid::nil(), // populated by repository
            score,
            decay_status,
            days_since_interaction,
            computed_at: chrono::Utc::now(),
        }
    }
}
```

`RelationshipHealthRepository` reads the most recent `contact_interactions` entry per contact and batches `DecayModel::compute()` calls, writing results to the existing `relationship_health` table. Called by `RelationshipHealthLoop` (daily).

**Step 2.2 — Interaction Logger** (`src/networking/interactions/service.rs`)

```rust
pub struct InteractionLogger {
    pool: Arc<DbPool>,
    stage_tx: tokio::sync::broadcast::Sender<RelationshipStageEvent>,
}

impl InteractionLogger {
    /// Log any interaction. If the interaction is Substantive and the contact
    /// is in Contacted stage, automatically advances to Replied.
    pub async fn log(
        &self,
        contact_id: uuid::Uuid,
        interaction_type: InteractionType,
        quality: InteractionQuality,
        occurred_at: chrono::DateTime<chrono::Utc>,
        notes: Option<String>,
    ) -> Result<InteractionRecord, InteractionError>;

    /// Called automatically by OutreachRepository::mark_sent().
    /// Inserts InteractionType::OutreachSent with quality=Automated.
    pub async fn log_outreach_sent(
        &self,
        contact_id: uuid::Uuid,
        draft_id: uuid::Uuid,
    ) -> Result<(), InteractionError>;
}
```

The `SqliteInteractionRepository::most_recent_interaction()` query:
```sql
SELECT interaction_type, quality, occurred_at
FROM contact_interactions
WHERE contact_id = ?1
ORDER BY occurred_at DESC
LIMIT 1
```

**Step 2.3 — LinkedIn Policy Guardrails (GAP-71)**

LazyJob does NOT integrate with the LinkedIn API (no official OAuth "Apply with LinkedIn" for desktop apps). The spec-side decision is documented in `src/networking/quality/patterns.rs` as a crate-level policy constant and enforced in the TUI.

```rust
/// Policy constant: LazyJob does not automate LinkedIn actions.
/// Users manually copy drafts from LazyJob into LinkedIn.
/// This is enforced at the TUI layer by:
///  - "Copy to clipboard" button only (no "Send via LinkedIn" button)
///  - A one-time advisory banner shown the first time a LinkedIn draft is generated
pub const LINKEDIN_AUTOMATION_POLICY: &str =
    "LazyJob generates drafts — you send them. \
     LinkedIn's ToS prohibits automated sending. \
     Use Ctrl+C to copy, then paste into LinkedIn.";

/// LazyJob will NOT implement these due to ToS/legal risk:
///  - LinkedIn OAuth (no official desktop OAuth app type exists for LazyJob's use case)
///  - LinkedIn API access (requires LinkedIn Partnership approval)
///  - Browser automation for sending (violates Section 8.2 of LinkedIn User Agreement)
///  - InMail API (deprecated for non-Partners)
```

The `NetworkingLinkedinPolicyView` (TUI) renders this advisory once per session on first LinkedIn draft access, with a "Got it" dismiss button. The advisory state is stored in `~/.config/lazyjob/ux_acknowledgements.toml`.

**Step 2.4 — Warm Path Expansion (GAP-72)**

`WarmPathExpander` generates expansion hints via a Ralph loop when the user has zero or weak warm paths to a target company.

```rust
// lazyjob-core/src/networking/expansion/service.rs

pub struct WarmPathExpander {
    pool: Arc<DbPool>,
    loop_manager: Arc<LoopManager>,
}

impl WarmPathExpander {
    /// Returns cached hints for a job if available (<24h old).
    /// Otherwise schedules WarmPathExpansionLoop ralph loop.
    /// Returns BriefStatus::Ready (cached) or BriefStatus::Generating (loop scheduled).
    pub async fn get_or_schedule_hints(
        &self,
        job_id: uuid::Uuid,
    ) -> Result<ExpansionStatus, ExpansionError>;

    /// Called by TUI dismiss action.
    pub async fn dismiss_hint(
        &self,
        hint_id: uuid::Uuid,
    ) -> Result<(), ExpansionError>;
}

pub enum ExpansionStatus {
    Ready(Vec<WarmPathExpansionHint>),
    Generating,
    NotNeeded,  // job already has warm paths via ConnectionMapper
}
```

The `WarmPathExpansionLoop` (in `lazyjob-ralph`) is a single-shot loop triggered when `ConnectionMapper::map_contacts_to_company()` returns only `Cold` or `SecondDegree` paths. It generates hints across 4 categories:
1. `Event`: industry events (from Meetup/Eventbrite RSS via reqwest) where company employees may attend
2. `Community`: GitHub orgs, Discord, Twitter — offline pattern matching from company domain
3. `Alumni`: cross-references the user's LifeSheet schools against company's job posts for degree requirements
4. `ContentEngage`: recent posts by company employees on LinkedIn topics the user cares about (Phase 3)

---

### Phase 3 — Moderate Gaps: Analytics, Cadence, Outreach Quality Scoring (GAP-73, GAP-75, GAP-76, GAP-77)

**Step 3.1 — `NetworkingAnalyticsService`** (`src/networking/analytics/service.rs`)

All queries run against existing + new tables. No materialized views; computed on demand.

```rust
pub struct NetworkingAnalyticsService {
    pool: Arc<DbPool>,
}

impl NetworkingAnalyticsService {
    pub async fn outreach_funnel(&self) -> Result<OutreachFunnel, AnalyticsError>;
    pub async fn channel_metrics(&self) -> Result<Vec<ChannelMetrics>, AnalyticsError>;
    pub async fn time_to_response_by_channel(&self) -> Result<Vec<(OutreachMedium, Option<f64>)>, AnalyticsError>;
}
```

Key query for `OutreachFunnel`:
```sql
-- Funnel step 1: total sent
SELECT COUNT(*) FROM outreach_drafts WHERE outreach_status = 'sent';

-- Funnel step 2: replied (contact stage advanced to Replied or beyond)
SELECT COUNT(DISTINCT pc.id)
FROM profile_contacts pc
JOIN outreach_drafts od ON od.contact_id = pc.id
WHERE od.outreach_status = 'sent'
  AND pc.relationship_stage IN ('replied','warmed','referral_asked','referral_resolved');

-- Channel breakdown
SELECT od.medium, COUNT(*) as sent,
       SUM(CASE WHEN pc.relationship_stage NOT IN ('identified','contacted') THEN 1 ELSE 0 END) as replied
FROM outreach_drafts od
JOIN profile_contacts pc ON pc.id = od.contact_id
WHERE od.outreach_status = 'sent'
GROUP BY od.medium;
```

**Step 3.2 — Referral Attribution (`ReferralApplicationLinker`, Cross-Spec Q)**

```rust
pub struct ReferralApplicationLinker {
    pool: Arc<DbPool>,
}

impl ReferralApplicationLinker {
    /// Called when an application transitions to a terminal state.
    /// Finds any referral_asks for the same (contact, job) and records attribution.
    pub async fn record_attribution(
        &self,
        application_id: uuid::Uuid,
        outcome: AttributionOutcome,
    ) -> Result<Option<ReferralAttributionRecord>, LinkError>;

    /// Returns the referral ask that contributed to this application, if any.
    pub async fn get_attribution(
        &self,
        application_id: uuid::Uuid,
    ) -> Result<Option<ReferralAttributionRecord>, LinkError>;
}
```

Schema: `referral_attribution` table (see migration 020 above). The `StageTransitionEvent` handler in `ApplicationWorkflow` calls `record_attribution()` when `ApplicationStage::is_terminal()`.

**Step 3.3 — `TouchpointCadenceAdvisor`** (`src/networking/cadence/advisor.rs`)

```rust
pub struct TouchpointCadenceAdvisor;

impl TouchpointCadenceAdvisor {
    /// Return recommended cadence interval (in days) for a contact.
    /// Checks for user override first, then falls back to tier default.
    pub fn recommend_interval(
        tier: &ConnectionTier,
        override_days: Option<i64>,
    ) -> i64 {
        if let Some(days) = override_days {
            return days;
        }
        match tier {
            ConnectionTier::FirstDegreeCurrentEmployee => 90,
            ConnectionTier::FirstDegreeFormerColleague => 60,
            ConnectionTier::FirstDegreeAlumni          => 45,
            ConnectionTier::SecondDegree               => 30,
            ConnectionTier::Cold                       => 21,
        }
    }

    /// Check if a date falls within a holiday quiet period.
    /// Current rules: Dec 20–Jan 5, Jul 4 (US), Christmas week.
    pub fn is_quiet_period(date: chrono::NaiveDate) -> bool {
        let (month, day) = (date.month(), date.day());
        matches!((month, day),
            (12, 20..=31) | (1, 1..=5) | (7, 4)
        )
    }

    /// Suggest next outreach date respecting cadence interval + quiet periods.
    pub fn next_outreach_date(
        last_interaction: chrono::NaiveDate,
        interval_days: i64,
    ) -> chrono::NaiveDate {
        let mut candidate = last_interaction + chrono::Duration::days(interval_days);
        while Self::is_quiet_period(candidate) {
            candidate += chrono::Duration::days(1);
        }
        candidate
    }
}
```

The `NetworkingReminderPoller` calls `TouchpointCadenceAdvisor::next_outreach_date()` for contacts with a cadence override, replacing the fixed 7/14/21-day threshold.

**Step 3.4 — `HeuristicOutreachQualityScorer`** (`src/networking/quality/scorer.rs`)

```rust
use once_cell::sync::Lazy;
use regex::Regex;

// AI-signature phrases detected via literal pattern matching (no regex needed for most)
static AI_SIGNATURE_PHRASES: Lazy<Vec<&'static str>> = Lazy::new(|| vec![
    "I hope this message finds you well",
    "I hope you are doing well",
    "I wanted to reach out",
    "I am reaching out",
    "Please let me know if you have any questions",
    "Thank you for your time and consideration",
    "I look forward to hearing from you",
    "I am excited to learn more",
    "touching base",
    "circle back",
    "synergy",
    "leverage my skills",
    "passionate about",
]);

static REPETITIVE_OPENER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(I |My |As a )").unwrap()
});

pub struct HeuristicOutreachQualityScorer;

impl OutreachQualityScorer for HeuristicOutreachQualityScorer {
    fn score(&self, draft: &OutreachDraft, shared_context: &SharedContext) -> OutreachQualityReport {
        let body = &draft.body_text;
        let word_count = body.split_whitespace().count();

        let mut flags = Vec::new();
        let mut ai_phrase_count = 0usize;

        // Length check
        let (min_words, max_words) = draft.medium.word_range();
        if word_count < min_words { flags.push(QualityFlag::TooShort { minimum: min_words, actual: word_count }); }
        if word_count > max_words { flags.push(QualityFlag::TooLong { limit: max_words, actual: word_count }); }

        // AI phrase detection
        for phrase in AI_SIGNATURE_PHRASES.iter() {
            if body.to_lowercase().contains(*phrase) {
                flags.push(QualityFlag::ContainsAiPhrase(phrase.to_string()));
                ai_phrase_count += 1;
            }
        }

        // Repetitive sentence openers
        let opener_count = REPETITIVE_OPENER.find_iter(body).count();
        if opener_count >= 3 {
            flags.push(QualityFlag::RepetitiveSentenceOpeners);
        }

        // Personalization: check if any SharedContext item is mentioned
        let personalization_found = shared_context.shared_employers.iter()
            .any(|emp| body.to_lowercase().contains(&emp.to_lowercase()))
            || shared_context.shared_schools.iter()
                .any(|s| body.to_lowercase().contains(&s.to_lowercase()));

        if !personalization_found && !shared_context.is_empty() {
            flags.push(QualityFlag::NoPersonalization);
        }

        let personalization_score = if personalization_found { 80u8 }
            else if shared_context.is_empty() { 50u8 }
            else { 20u8 };

        let ai_signature_risk = match ai_phrase_count {
            0     => AiSignatureRisk::Low,
            1..=2 => AiSignatureRisk::Medium,
            _     => AiSignatureRisk::High,
        };

        let suggestions = self.build_suggestions(&flags);

        OutreachQualityReport {
            draft_id: draft.id,
            personalization_score,
            length_ok: !flags.iter().any(|f| matches!(f, QualityFlag::TooLong {..} | QualityFlag::TooShort {..})),
            ai_signature_risk,
            flags,
            suggestions,
        }
    }
}
```

The scorer is called synchronously during draft generation and results cached in `outreach_quality_cache`. The TUI shows a color-coded badge (green/yellow/red) in the draft list.

---

### Phase 4 — Cross-Spec Data Model Fixes (Cross-Spec P, Cross-Spec Q)

**Cross-Spec P: Contact ↔ LifeSheet Overlap**

`ContactLifeSheetLinker` runs at the end of each LifeSheet import (in `LifeSheetService::import()`):

```rust
pub struct ContactLifeSheetLinker {
    pool: Arc<DbPool>,
}

impl ContactLifeSheetLinker {
    /// For each experience entry in the imported LifeSheet, find profile_contacts
    /// with the same company using normalize_company_name() + jaro_winkler >= 0.92.
    /// For matches, writes life_sheet_link_id = experience_id into profile_contacts.
    /// Does NOT create new contacts — only links existing ones.
    pub async fn link_from_life_sheet(
        &self,
        experience_ids: &[(uuid::Uuid, String)], // (experience_id, company_name)
    ) -> Result<usize, LinkError>; // returns count of linked contacts

    /// For each unlinked profile_contact, check if the user worked at the same company.
    /// Surface as a suggestion in the TUI import wizard: "Did you meet [Contact] at [Company]?"
    pub async fn find_unlinked_colleagues(
        &self,
    ) -> Result<Vec<UnlinkedColleagueSuggestion>, LinkError>;
}

pub struct UnlinkedColleagueSuggestion {
    pub contact: ProfileContact,
    pub company_name: String,
    pub life_sheet_experience_id: uuid::Uuid,
}
```

The `life_sheet_link_id` column enables:
1. SharedContextBuilder to automatically include shared-employer context for contacts with a link
2. The agentic warm path finder to know which contacts are former colleagues (highest-trust warm path)

**Cross-Spec Q: Outreach ↔ Application State (Referral Attribution)**

The `ApplicationWorkflowService::move_stage()` method is extended to call `ReferralApplicationLinker::record_attribution()` when transitioning to a terminal stage (Rejected, Accepted, Withdrawn):

```rust
// In lazyjob-core/src/application/workflow/move_stage.rs
// After the transition is committed to SQLite:
if next_stage.is_terminal() {
    if let Err(e) = self.referral_linker
        .record_attribution(application_id, AttributionOutcome::from(&next_stage))
        .await
    {
        tracing::warn!("referral attribution failed: {e}");
        // Non-fatal: attribution failure never blocks the workflow
    }
}
```

The attribution lookup query:
```sql
SELECT ra.id, ra.contact_id, pc.full_name
FROM referral_asks ra
JOIN profile_contacts pc ON pc.id = ra.contact_id
WHERE ra.job_id = (SELECT job_id FROM applications WHERE id = ?1)
  AND ra.outcome = 'succeeded'
LIMIT 1
```

---

## Key Crate APIs

- `csv::ReaderBuilder::new().has_headers(true).flexible(true).from_reader(bytes)` — tolerant CSV parsing
- `vcard4::parse(text)` — parse vCard 4.0 format from string
- `tokio::task::spawn_blocking(closure)` — offload sync CSV/vCard parsing from async executor
- `strsim::jaro_winkler(a, b) -> f64` — name/company fuzzy matching
- `once_cell::sync::Lazy<Vec<&'static str>>` — AI phrase table compiled at first use
- `regex::Regex::find_iter(text)` — repetitive opener detection
- `chrono::Utc::now()` — timestamps throughout
- `chrono::Duration::days(n)` — cadence interval arithmetic
- `sqlx::query!("INSERT OR IGNORE INTO ...", ...)` — idempotent upsert for interaction log
- `tokio::sync::broadcast::Sender<RelationshipStageEvent>` — stage change notifications to TUI

## Error Handling

```rust
// lazyjob-core/src/networking/import/error.rs
#[derive(thiserror::Error, Debug)]
pub enum ImportError {
    #[error("CSV parse error: {0}")]
    CsvParse(#[from] csv::Error),

    #[error("vCard parse error: {0}")]
    VCardParse(String),

    #[error("UTF-8 encoding error in import file")]
    Encoding,

    #[error("spawn_blocking panicked")]
    SpawnBlocking,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// lazyjob-core/src/networking/interactions/error.rs
#[derive(thiserror::Error, Debug)]
pub enum InteractionError {
    #[error("contact {0} not found")]
    ContactNotFound(uuid::Uuid),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// lazyjob-core/src/networking/analytics/error.rs
#[derive(thiserror::Error, Debug)]
pub enum AnalyticsError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// lazyjob-core/src/networking/expansion/error.rs
#[derive(thiserror::Error, Debug)]
pub enum ExpansionError {
    #[error("loop scheduling failed: {0}")]
    LoopScheduling(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

## Testing Strategy

### Unit tests

**Import:**
- `LinkedInCsvImporter::parse()` with a fixture CSV file — verify field extraction, tolerance of missing columns, empty-row skip
- `VCardImporter::parse()` with a multi-contact `.vcf` fixture — verify UID extraction, LinkedIn URL detection
- `FuzzyContactDeduplicator::find_duplicates()` — exact email match, fuzzy name+company above and below threshold, no false positives for common names with different companies

**Decay model:**
- `DecayModel::compute()` with 0, 30, 60, 90, 120 days for each tier — verify monotonic decrease
- `DecayStatus` transitions: score=72 → Healthy, score=50 → Warning, etc.

**Outreach quality:**
- `HeuristicOutreachQualityScorer::score()` with a draft containing all AI phrases → AiSignatureRisk::High
- Score with a genuinely personalized draft → Low risk, high personalization score
- Length check: under minimum, over maximum, in range

**Cadence:**
- `TouchpointCadenceAdvisor::is_quiet_period()` — Dec 25, Jan 3, Jul 4 are quiet; Feb 15 is not
- `next_outreach_date()` — if candidate falls in quiet period, advances past it

### Integration tests

- Full import round-trip: parse LinkedIn CSV → dedup against existing contacts → insert new → confirm import report counts
- `InteractionLogger::log()` with quality=Substantive + contact in Contacted stage → verify `RelationshipStage` advances to Replied
- `ReferralApplicationLinker::record_attribution()` — transition application to Hired → verify attribution row written, `referral_asks.outcome` updated to Succeeded
- Analytics funnel: seed known outreach + reply state → verify OutreachFunnel.response_rate matches expected

### TUI tests

- Import wizard: shows "X contacts to insert, Y pairs need review" counts after prepare_import
- Dedup review panel: renders DuplicatePair cards with merge/keep_both/skip buttons; Merge writes merged contact, keeps only one row in profile_contacts
- Interaction logger overlay: quick-log form opens with `i` keybind in contact detail view; submits with Enter
- Analytics dashboard: OutreachFunnel renders as a 5-step bar chart; channel metrics render as a table

## Open Questions

1. **vCard library maturity**: `vcard4` crate (0.6.x) may lack edge-case coverage for complex multi-value fields. Consider falling back to line-by-line regex parsing for Phase 1 if `vcard4` proves fragile. The `GenericCsvImporter` can handle most real-world contact exports as a simpler fallback.

2. **Decay model half-lives**: The half-life constants (90/60/45/30/20 days) are engineering estimates. A post-MVP calibration loop (GAP-70 mentions learning from which cadences produced responses) should refine these based on `referral_attribution` outcomes.

3. **Outreach quality A/B testing**: GAP-77 mentions A/B variant generation. This is not implemented in Phase 3 (Moderate priority). A future spec should define what "comparison" means — does the user rate both? Does response rate difference require statistical significance before surfacing?

4. **Alumni network (GAP-72, Step 2.4)**: LinkedIn school inference from job posts requires parsing "Bachelor's degree" / "from Stanford preferred" language in job descriptions. This is currently a WarmPathExpansionLoop LLM task. Consider a pure offline lexicon approach (like TechTermLexicon) for common schools before sending to LLM.

5. **Privacy mode and analytics**: `NetworkingAnalyticsService` queries by company name. When `PrivacyMode::Stealth` is active, should the analytics dashboard replace company names with `[redacted]`? This was handled in `DigestService` for job pipeline metrics — the same pattern should apply here.

6. **Contact phone import**: The `RawContact.phone` field is populated but not displayed in any existing TUI view. A future iteration should add phone to the `ContactDetailView` and document that LazyJob never dials or SMS-sends.

## Related Specs

- [specs/networking-connection-mapping-implementation-plan.md](networking-connection-mapping-implementation-plan.md)
- [specs/networking-outreach-drafting-implementation-plan.md](networking-outreach-drafting-implementation-plan.md)
- [specs/networking-referral-management-implementation-plan.md](networking-referral-management-implementation-plan.md)
- [specs/networking-referrals-agentic-implementation-plan.md](networking-referrals-agentic-implementation-plan.md)
- [specs/profile-life-sheet-data-model-implementation-plan.md](profile-life-sheet-data-model-implementation-plan.md)
- [specs/application-workflow-actions-implementation-plan.md](application-workflow-actions-implementation-plan.md)
- [specs/16-privacy-security-implementation-plan.md](16-privacy-security-implementation-plan.md)
- [specs/XX-contact-multi-source-import.md](XX-contact-multi-source-import.md) *(spec to be written)*
