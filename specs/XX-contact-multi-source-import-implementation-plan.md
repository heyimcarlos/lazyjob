# Implementation Plan: Multi-Source Contact Import

## Status
Draft

## Related Spec
[specs/XX-contact-multi-source-import.md](XX-contact-multi-source-import.md)

## Overview

The multi-source contact import module extends LazyJob's networking layer to ingest contacts from five distinct sources: LinkedIn CSV export, vCard files (.vcf), Gmail Contacts (via Google People API), Apple Contacts (macOS AddressBook framework via FFI), and business card photos (image → OCR text → LLM structured extraction). All import sources converge into a single `ImportedContact` canonical type that flows through a shared deduplication and upsert pipeline into the `profile_contacts` SQLite table.

The architecture is parser-per-source with a shared `ContactImportService` orchestrator. Each parser is an async trait object, making it independently testable and swappable. Deduplication uses a three-tier strategy: exact email match (auto-merge, high confidence), fuzzy full-name + current-company match (above 0.92 threshold: auto-merge; 0.82–0.92: queue for TUI review), and everything else treated as a new contact. This strategy is consistent with the contact deduplication design in `07-gaps-networking-outreach-implementation-plan.md`.

The module is strictly local-first. Gmail OAuth tokens are stored exclusively in the OS keychain (never SQLite). Apple Contacts access is fully `#[cfg(target_os = "macos")]`-gated with no-op stubs on other platforms. Business card OCR is performed by the LLM provider (vision capability) rather than a native OCR crate, to avoid a heavy native dependency and allow offline graceful degradation. Batch imports run on a background tokio task and stream progress events via `broadcast::Sender<ImportProgress>` to the TUI without blocking the main event loop.

## Prerequisites

### Must be implemented first
- `specs/04-sqlite-persistence-implementation-plan.md` — connection pool, `run_migrations`, migration framework
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `profile_contacts` table, `ProfileContact` domain type, `ContactId`
- `specs/networking-connection-mapping-implementation-plan.md` — `ContactSource` enum (existing LinkedIn CSV variant is extended here), `SqliteContactRepository`, `ContactRepository` trait
- `specs/07-gaps-networking-outreach-implementation-plan.md` — `FuzzyContactDeduplicator` struct (this plan reuses it)
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait (for business card vision extraction)
- `specs/16-privacy-security-implementation-plan.md` — `keyring::Entry` credential storage pattern

### Crates to add to Cargo.toml
```toml
[workspace.dependencies]
# vCard parsing
vcard4 = "0.5"        # Pure Rust vCard 4.0 / 3.0 parser; no unsafe; covers RFC 6350

# Gmail API
oauth2 = "4.4"        # OAuth2 PKCE flow; already in SaaS plan — ensure version consistency
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"] }
                      # Already in workspace; confirm features include stream for image upload

# Image encoding for LLM vision
base64 = "0.22"       # For encoding business card image bytes to LLM vision payload

# CSV (LinkedIn) — already present from networking-connection-mapping plan
csv = "1.3"

# Apple Contacts FFI — macOS only
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.5"                     # Safe Objective-C 2.0 bindings; replaces deprecated objc crate
objc2-contacts = "0.2"            # Contacts.framework bindings for objc2
```

> Note: `objc2-contacts` may not yet be stable enough for production. Phase 4 documents an alternative using a small Swift shim binary spawned as a subprocess if the FFI bindings prove insufficient.

## Architecture

### Crate Placement

| Component | Crate | Module |
|-----------|-------|--------|
| `ImportedContact`, `ContactName`, `Email`, `PhoneNumber`, `ImportSource`, `ImportStats`, `ImportProgress` | `lazyjob-core` | `src/contact_import/types.rs` |
| `ContactParser` async trait | `lazyjob-core` | `src/contact_import/parser.rs` |
| `LinkedInCsvParser` | `lazyjob-core` | `src/contact_import/linkedin.rs` |
| `VCardParser` | `lazyjob-core` | `src/contact_import/vcard.rs` |
| `GmailContactsParser` | `lazyjob-core` | `src/contact_import/gmail.rs` |
| `GmailOAuthFlow` credential manager | `lazyjob-core` | `src/contact_import/gmail_oauth.rs` |
| `AppleContactsParser` (macOS) | `lazyjob-core` | `src/contact_import/apple.rs` |
| `BusinessCardScanner` | `lazyjob-core` | `src/contact_import/business_card.rs` |
| `ImportDedupService` | `lazyjob-core` | `src/contact_import/dedup.rs` |
| `ContactImportService` orchestrator | `lazyjob-core` | `src/contact_import/service.rs` |
| `IncrementalImportTracker` | `lazyjob-core` | `src/contact_import/incremental.rs` |
| `ContactNormalizer` (field cleaning) | `lazyjob-core` | `src/contact_import/normalize.rs` |
| SQLite migration 022 | `lazyjob-core` | `migrations/022_contact_import.sql` |
| TUI Import Wizard | `lazyjob-tui` | `src/views/contacts/import_wizard.rs` |
| TUI Duplicate Review View | `lazyjob-tui` | `src/views/contacts/dedup_review.rs` |
| TUI Import Progress Panel | `lazyjob-tui` | `src/views/contacts/import_progress.rs` |

### Core Types

```rust
// lazyjob-core/src/contact_import/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Strongly-typed import session ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImportSessionId(pub Uuid);
impl ImportSessionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Which source produced this contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportSource {
    LinkedInCsv { file_path: PathBuf, imported_at: DateTime<Utc> },
    VCardFile   { file_path: PathBuf, imported_at: DateTime<Utc> },
    Gmail       { account_email: String, imported_at: DateTime<Utc> },
    AppleContacts { imported_at: DateTime<Utc> },
    BusinessCardPhoto { photo_path: PathBuf, imported_at: DateTime<Utc> },
    ManualEntry,
}

impl ImportSource {
    /// Stable string key used in SQLite and keyring entries.
    pub fn source_key(&self) -> &'static str {
        match self {
            Self::LinkedInCsv { .. }      => "linkedin_csv",
            Self::VCardFile { .. }        => "vcard",
            Self::Gmail { .. }            => "gmail",
            Self::AppleContacts { .. }    => "apple_contacts",
            Self::BusinessCardPhoto { .. }=> "business_card",
            Self::ManualEntry             => "manual",
        }
    }
}

/// A name as imported from the source; may be split or unsplit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactName {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// Always populated: derived from given+family or the source's full name field.
    pub full_name: String,
}

impl ContactName {
    /// Constructs from optional parts, using full_name fallback.
    pub fn from_parts(given: Option<String>, family: Option<String>, full: Option<String>) -> Self {
        let full_name = full.unwrap_or_else(|| {
            [given.as_deref(), family.as_deref()]
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        });
        Self { given_name: given, family_name: family, full_name }
    }
}

/// Validated email address newtype. Parse-don't-validate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Email(String);

impl Email {
    /// Returns None if the string has no '@'.
    pub fn parse(s: impl Into<String>) -> Option<Self> {
        let s = s.into();
        if s.contains('@') { Some(Self(s)) } else { None }
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// Phone number (normalized to E.164 where possible, raw otherwise).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneNumber {
    pub raw: String,
    /// E.164 format (+12125551234) if normalization succeeded.
    pub e164: Option<String>,
}

/// A contact as returned by any parser — source-agnostic canonical form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedContact {
    /// Ephemeral ID assigned during parsing; NOT the final `ContactId`.
    pub import_id: Uuid,
    pub source: ImportSource,
    pub names: Vec<ContactName>,
    pub emails: Vec<Email>,
    pub phone_numbers: Vec<PhoneNumber>,
    pub current_company: Option<String>,
    pub current_title: Option<String>,
    pub previous_companies: Vec<ImportedPreviousCompany>,
    pub notes: Option<String>,
    /// LinkedIn public profile URL if available.
    pub linkedin_url: Option<String>,
    /// Opaque source-specific timestamp for incremental filtering.
    pub source_modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedPreviousCompany {
    pub name: String,
    pub title: Option<String>,
    pub start_year: Option<i32>,
    pub end_year: Option<i32>,
}

/// Outcome of deduplication for a single imported contact.
#[derive(Debug)]
pub enum ImportMergeDecision {
    /// Auto-merged (confidence >= 0.92 or exact email match). Existing contact updated.
    AutoMerged { existing_contact_id: Uuid, confidence: f32 },
    /// Pending human review (0.82 <= confidence < 0.92).
    PendingReview { existing_contact_id: Uuid, confidence: f32 },
    /// New contact — no match found above threshold.
    NewContact,
}

/// Progress event broadcast during import.
#[derive(Debug, Clone)]
pub enum ImportProgress {
    Started { session_id: ImportSessionId, total_contacts: usize },
    Parsed { parsed: usize, total: usize },
    Deduplicating,
    Inserted  { count: usize },
    AutoMerged { count: usize },
    PendingReview { count: usize },
    Completed(ImportStats),
    Error(String),
}

/// Final import session statistics persisted to SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStats {
    pub session_id: ImportSessionId,
    pub source_key: String,
    pub parsed_count: usize,
    pub inserted_count: usize,
    pub auto_merged_count: usize,
    pub pending_review_count: usize,
    pub skipped_count: usize,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/contact_import/parser.rs

use super::types::{ImportedContact, ImportSource};
use crate::errors::ContactImportError;

/// Every import source implements this trait.
/// Each implementation handles its own I/O (file read, HTTP, FFI) and
/// returns a flat Vec of ImportedContact ready for deduplication.
#[async_trait::async_trait]
pub trait ContactParser: Send + Sync {
    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError>;

    /// Human-readable source name for UI and logging.
    fn source_name(&self) -> &'static str;
}
```

```rust
// lazyjob-core/src/contact_import/service.rs

/// High-level orchestrator that sequences: parse → normalize → dedup → upsert → broadcast.
pub struct ContactImportService {
    linkedin_parser: Arc<LinkedInCsvParser>,
    vcard_parser:    Arc<VCardParser>,
    gmail_parser:    Arc<GmailContactsParser>,
    #[cfg(target_os = "macos")]
    apple_parser:    Arc<AppleContactsParser>,
    biz_card_scanner: Arc<BusinessCardScanner>,
    dedup:           Arc<ImportDedupService>,
    normalizer:      Arc<ContactNormalizer>,
    contact_repo:    Arc<dyn ContactRepository>,
    incremental:     Arc<IncrementalImportTracker>,
    db:              Arc<sqlx::SqlitePool>,
    progress_tx:     broadcast::Sender<ImportProgress>,
}

impl ContactImportService {
    /// Begins an import for a single source. Runs entirely on a background tokio task.
    /// Returns the session ID immediately; caller subscribes to progress_tx for updates.
    pub async fn start_import(
        self: Arc<Self>,
        source: ImportSource,
    ) -> Result<ImportSessionId, ContactImportError>;

    /// Returns contacts with ImportMergeDecision::PendingReview awaiting human decision.
    pub async fn list_pending_reviews(
        &self,
    ) -> Result<Vec<PendingReviewItem>, ContactImportError>;

    /// User confirms a merge: applies the update to profile_contacts.
    pub async fn confirm_merge(
        &self,
        existing_id: Uuid,
        imported: &ImportedContact,
    ) -> Result<(), ContactImportError>;

    /// User rejects a merge: marks it distinct, inserts as new contact.
    pub async fn reject_merge(
        &self,
        existing_id: Uuid,
        imported: &ImportedContact,
    ) -> Result<Uuid, ContactImportError>;

    pub fn subscribe_progress(&self) -> broadcast::Receiver<ImportProgress>;
}
```

### SQLite Schema

```sql
-- migrations/022_contact_import.sql

-- Import session log: one row per import run.
CREATE TABLE IF NOT EXISTS contact_import_sessions (
    id                 TEXT PRIMARY KEY,           -- ImportSessionId (UUID)
    source_key         TEXT NOT NULL,              -- 'linkedin_csv' | 'vcard' | 'gmail' | 'apple_contacts' | 'business_card'
    source_detail_json TEXT NOT NULL DEFAULT '{}', -- e.g. {"file_path": "...", "account_email": "..."}
    parsed_count       INTEGER NOT NULL DEFAULT 0,
    inserted_count     INTEGER NOT NULL DEFAULT 0,
    auto_merged_count  INTEGER NOT NULL DEFAULT 0,
    pending_review_count INTEGER NOT NULL DEFAULT 0,
    skipped_count      INTEGER NOT NULL DEFAULT 0,
    status             TEXT NOT NULL DEFAULT 'running', -- 'running' | 'completed' | 'failed'
    error_message      TEXT,
    started_at         TEXT NOT NULL,              -- ISO 8601
    completed_at       TEXT                        -- NULL while running
);

-- Contacts awaiting user dedup decision.
CREATE TABLE IF NOT EXISTS contact_import_pending_reviews (
    id                  TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    session_id          TEXT NOT NULL REFERENCES contact_import_sessions(id) ON DELETE CASCADE,
    existing_contact_id TEXT NOT NULL REFERENCES profile_contacts(id) ON DELETE CASCADE,
    imported_json       TEXT NOT NULL,             -- serialized ImportedContact
    confidence          REAL NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at         TEXT,                      -- NULL until user decides
    resolution          TEXT                       -- 'merged' | 'distinct'
);

CREATE INDEX idx_pending_reviews_session
    ON contact_import_pending_reviews(session_id);

CREATE INDEX idx_pending_reviews_unresolved
    ON contact_import_pending_reviews(resolved_at)
    WHERE resolved_at IS NULL;

-- Incremental import watermark per (source_key, source_detail_key).
-- e.g. source_key='gmail', source_detail_key='user@gmail.com'
CREATE TABLE IF NOT EXISTS contact_import_watermarks (
    source_key         TEXT NOT NULL,
    source_detail_key  TEXT NOT NULL,              -- file path hash, gmail account, etc.
    last_imported_at   TEXT NOT NULL,              -- ISO 8601
    PRIMARY KEY (source_key, source_detail_key)
);

-- Gmail OAuth tokens stored here as encrypted blobs.
-- Actual access_token and refresh_token go to OS keyring; this stores metadata only.
CREATE TABLE IF NOT EXISTS gmail_oauth_sessions (
    account_email      TEXT PRIMARY KEY,
    authorized_at      TEXT NOT NULL,
    scopes             TEXT NOT NULL DEFAULT 'https://www.googleapis.com/auth/contacts.readonly',
    token_expiry_hint  TEXT                        -- helps determine if re-auth is needed
);
```

**Extensions to `profile_contacts` table** (new columns added by this migration):

```sql
-- Add import provenance columns to existing profile_contacts table.
ALTER TABLE profile_contacts ADD COLUMN linkedin_url TEXT;
ALTER TABLE profile_contacts ADD COLUMN import_source_key TEXT DEFAULT 'manual';
ALTER TABLE profile_contacts ADD COLUMN import_session_id TEXT;
-- phone_numbers_json already expected from networking plan; add if missing:
-- ALTER TABLE profile_contacts ADD COLUMN phone_numbers_json TEXT DEFAULT '[]';
```

### Module Structure

```
lazyjob-core/
  src/
    contact_import/
      mod.rs          -- re-exports: ContactImportService, ContactParser, ImportedContact, ImportSource, ImportProgress
      types.rs        -- ImportedContact, ContactName, Email, PhoneNumber, ImportStats, ImportProgress, ImportMergeDecision
      parser.rs       -- ContactParser trait
      linkedin.rs     -- LinkedInCsvParser
      vcard.rs        -- VCardParser
      gmail.rs        -- GmailContactsParser
      gmail_oauth.rs  -- GmailOAuthFlow (PKCE flow, keyring storage)
      apple.rs        -- AppleContactsParser (#[cfg(target_os = "macos")])
      business_card.rs-- BusinessCardScanner
      dedup.rs        -- ImportDedupService
      normalize.rs    -- ContactNormalizer (phone E.164, company name)
      incremental.rs  -- IncrementalImportTracker
      service.rs      -- ContactImportService orchestrator
  migrations/
    022_contact_import.sql

lazyjob-tui/
  src/
    views/
      contacts/
        import_wizard.rs   -- source selection, per-source configuration, progress display
        dedup_review.rs    -- side-by-side merge review UI
        import_progress.rs -- real-time progress panel (subscribes to broadcast)
```

---

## Implementation Phases

### Phase 1 — LinkedIn CSV and vCard Parsers (MVP Core)

**Goal:** Ship the two non-authenticated, file-based import sources. These require no OAuth, no FFI, and are usable immediately. Deduplication, upsert, and TUI import wizard are included.

#### Step 1.1 — Domain types (`types.rs`)

File: `lazyjob-core/src/contact_import/types.rs`

Implement all types as defined above: `ImportSource`, `ContactName`, `Email::parse()`, `PhoneNumber`, `ImportedContact`, `ImportMergeDecision`, `ImportProgress`, `ImportStats`, `ImportSessionId`.

Derive `Serialize`/`Deserialize` on all types. Implement `ImportSource::source_key()`. No external crates needed beyond `chrono`, `uuid`, `serde`, `serde_json`.

**Verification:** `cargo test -p lazyjob-core contact_import::types` — test `Email::parse()` with valid and invalid inputs, `ContactName::from_parts()` edge cases.

#### Step 1.2 — `ContactNormalizer` (`normalize.rs`)

File: `lazyjob-core/src/contact_import/normalize.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static LEGAL_SUFFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\s*(inc\.?|llc\.?|ltd\.?|corp\.?|co\.?|gmbh|s\.a\.|plc)\s*$").unwrap()
});
static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

pub struct ContactNormalizer;

impl ContactNormalizer {
    /// Strip legal suffixes, collapse whitespace, lowercase.
    pub fn normalize_company(name: &str) -> String {
        let stripped = LEGAL_SUFFIX.replace_all(name.trim(), "");
        WHITESPACE.replace_all(stripped.trim(), " ").to_lowercase()
    }

    /// Normalize full name: trim, collapse whitespace, title-case.
    pub fn normalize_name(name: &str) -> String {
        WHITESPACE
            .replace_all(name.trim(), " ")
            .split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Best-effort E.164 normalization (US numbers only for MVP).
    /// Returns raw as-is if cannot normalize.
    pub fn normalize_phone(raw: &str) -> PhoneNumber {
        let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        let e164 = match digits.len() {
            10 => Some(format!("+1{digits}")),
            11 if digits.starts_with('1') => Some(format!("+{digits}")),
            _ => None,
        };
        PhoneNumber { raw: raw.to_owned(), e164 }
    }
}
```

**Verification:** Unit tests for `normalize_company("Acme, Inc.")` → `"acme"`, `normalize_phone("(212) 555-1234")` → `e164 = Some("+12125551234")`.

#### Step 1.3 — LinkedIn CSV Parser (`linkedin.rs`)

File: `lazyjob-core/src/contact_import/linkedin.rs`

LinkedIn exports a CSV with headers: `First Name`, `Last Name`, `Email Address`, `Company`, `Position`, `Connected On`.

```rust
use csv::ReaderBuilder;
use std::io::Cursor;
use tokio::task;

pub struct LinkedInCsvParser;

#[async_trait::async_trait]
impl ContactParser for LinkedInCsvParser {
    fn source_name(&self) -> &'static str { "LinkedIn CSV" }

    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        let path = match source {
            ImportSource::LinkedInCsv { file_path, .. } => file_path.clone(),
            _ => return Err(ContactImportError::WrongSource),
        };

        // CSV parsing is sync + potentially large file; run on blocking thread.
        task::spawn_blocking(move || Self::parse_file(&path))
            .await
            .map_err(|e| ContactImportError::Internal(e.to_string()))?
    }
}

impl LinkedInCsvParser {
    fn parse_file(path: &std::path::Path) -> Result<Vec<ImportedContact>, ContactImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ContactImportError::IoError(e.to_string()))?;

        // LinkedIn CSV has a 3-line preamble before actual headers; skip it.
        let header_start = content
            .lines()
            .enumerate()
            .find(|(_, line)| line.starts_with("First Name"))
            .map(|(i, _)| i)
            .ok_or(ContactImportError::ParseError("No header row found".into()))?;

        let csv_content: String = content
            .lines()
            .skip(header_start)
            .collect::<Vec<_>>()
            .join("\n");

        let mut rdr = ReaderBuilder::new()
            .flexible(true)
            .from_reader(Cursor::new(csv_content.as_bytes()));

        let headers = rdr.headers()
            .map_err(|e| ContactImportError::ParseError(e.to_string()))?
            .clone();

        // Build column-name → index map so column reordering is tolerated.
        let col_idx = |name: &str| -> Option<usize> {
            headers.iter().position(|h| h.trim().eq_ignore_ascii_case(name))
        };

        let first_name_col  = col_idx("First Name");
        let last_name_col   = col_idx("Last Name");
        let email_col       = col_idx("Email Address");
        let company_col     = col_idx("Company");
        let position_col    = col_idx("Position");
        let connected_col   = col_idx("Connected On");

        let mut contacts = vec![];
        for result in rdr.records() {
            let record = result.map_err(|e| ContactImportError::ParseError(e.to_string()))?;
            let get = |idx: Option<usize>| -> Option<String> {
                idx.and_then(|i| record.get(i)).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
            };

            let given  = get(first_name_col);
            let family = get(last_name_col);
            if given.is_none() && family.is_none() { continue; } // skip header repeat / blank rows

            let emails: Vec<Email> = get(email_col)
                .and_then(Email::parse)
                .into_iter()
                .collect();

            let connected_at: Option<DateTime<Utc>> = get(connected_col)
                .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%d %b %Y").ok())
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc());

            contacts.push(ImportedContact {
                import_id: Uuid::new_v4(),
                source: source.clone(),
                names: vec![ContactName::from_parts(given, family, None)],
                emails,
                phone_numbers: vec![],
                current_company: get(company_col).map(|s| ContactNormalizer::normalize_company(&s)),
                current_title: get(position_col),
                previous_companies: vec![],
                notes: None,
                linkedin_url: None,
                source_modified_at: connected_at,
            });
        }
        Ok(contacts)
    }
}
```

**Verification:** Unit test with a fixture `tests/fixtures/linkedin_sample.csv` containing 5 rows including one with a blank email and one with a reordered column header. Assert that 4 contacts are returned (blank-name row skipped), emails are parsed correctly, `current_company` is normalized.

#### Step 1.4 — vCard Parser (`vcard.rs`)

File: `lazyjob-core/src/contact_import/vcard.rs`

Use `vcard4` crate for RFC 6350 vCard 3.0/4.0 parsing. Single file may contain multiple VCARD blocks.

```rust
use vcard4::VCard;
use tokio::task;

pub struct VCardParser;

#[async_trait::async_trait]
impl ContactParser for VCardParser {
    fn source_name(&self) -> &'static str { "vCard File" }

    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        let path = match source {
            ImportSource::VCardFile { file_path, .. } => file_path.clone(),
            _ => return Err(ContactImportError::WrongSource),
        };

        task::spawn_blocking(move || Self::parse_file(&path, source))
            .await
            .map_err(|e| ContactImportError::Internal(e.to_string()))?
    }
}

impl VCardParser {
    fn parse_file(path: &std::path::Path, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ContactImportError::IoError(e.to_string()))?;

        let vcards: Vec<VCard> = vcard4::parse_str(&content)
            .map_err(|e| ContactImportError::ParseError(e.to_string()))?;

        let mut contacts = vec![];
        for vcard in vcards {
            let full_name = vcard.fn_value().map(|s| s.to_owned());
            let (given, family) = vcard.n_value()
                .map(|n| (n.given_name().map(str::to_owned), n.family_name().map(str::to_owned)))
                .unwrap_or((None, None));

            let name = ContactName::from_parts(given, family, full_name);
            if name.full_name.is_empty() { continue; }

            let emails: Vec<Email> = vcard.email_values()
                .filter_map(|e| Email::parse(e.value()))
                .collect();

            let phone_numbers: Vec<PhoneNumber> = vcard.tel_values()
                .map(|t| ContactNormalizer::normalize_phone(t.value()))
                .collect();

            // ORG property gives employer.
            let current_company = vcard.org_values()
                .next()
                .and_then(|o| o.values().first().map(|s| s.as_str()))
                .map(|s| ContactNormalizer::normalize_company(s));

            let current_title = vcard.title_values()
                .next()
                .map(|t| t.value().to_owned());

            let linkedin_url = vcard.url_values()
                .map(|u| u.value().to_owned())
                .find(|u| u.contains("linkedin.com"));

            let notes = vcard.note_values()
                .next()
                .map(|n| n.value().to_owned());

            contacts.push(ImportedContact {
                import_id: Uuid::new_v4(),
                source: source.clone(),
                names: vec![name],
                emails,
                phone_numbers,
                current_company,
                current_title,
                previous_companies: vec![],
                notes,
                linkedin_url,
                source_modified_at: None,
            });
        }
        Ok(contacts)
    }
}
```

**Verification:** Unit test with a multi-VCARD fixture containing a vCard with no FN (skipped), a vCard with ORG + TITLE, and a vCard with a LinkedIn URL. Assert 2 contacts, company normalized, URL extracted.

#### Step 1.5 — `ImportDedupService` (`dedup.rs`)

File: `lazyjob-core/src/contact_import/dedup.rs`

This reuses the `FuzzyContactDeduplicator` from `07-gaps-networking-outreach-implementation-plan.md` and wraps it in import-specific decisions.

```rust
use strsim::jaro_winkler;

pub struct ImportDedupService {
    /// Threshold for auto-merge (high confidence).
    pub auto_merge_threshold: f32,   // default 0.92
    /// Threshold for pending review (medium confidence).
    pub review_threshold: f32,       // default 0.82
}

impl Default for ImportDedupService {
    fn default() -> Self {
        Self { auto_merge_threshold: 0.92, review_threshold: 0.82 }
    }
}

impl ImportDedupService {
    /// For each ImportedContact, determine the merge decision against existing contacts.
    /// Pure function — no I/O. All data comes in as parameters.
    pub fn classify(
        &self,
        imported: &ImportedContact,
        existing: &[ProfileContact],
    ) -> (ImportMergeDecision, Option<&ProfileContact>) {
        for existing_contact in existing {
            let confidence = self.compute_similarity(imported, existing_contact);
            if confidence >= self.auto_merge_threshold {
                return (
                    ImportMergeDecision::AutoMerged {
                        existing_contact_id: existing_contact.id.0,
                        confidence,
                    },
                    Some(existing_contact),
                );
            }
            if confidence >= self.review_threshold {
                return (
                    ImportMergeDecision::PendingReview {
                        existing_contact_id: existing_contact.id.0,
                        confidence,
                    },
                    Some(existing_contact),
                );
            }
        }
        (ImportMergeDecision::NewContact, None)
    }

    fn compute_similarity(&self, imported: &ImportedContact, existing: &ProfileContact) -> f32 {
        // Tier 1: Exact email match → auto-merge (1.0).
        for ie in &imported.emails {
            if existing.emails.iter().any(|ee| ee.eq_ignore_ascii_case(ie.as_str())) {
                return 1.0;
            }
        }

        // Tier 2: Name similarity (0.4 weight) + company similarity (0.1 weight).
        let name_sim = imported.names.iter()
            .map(|n| {
                let existing_name = &existing.full_name;
                jaro_winkler(&n.full_name.to_lowercase(), &existing_name.to_lowercase()) as f32
            })
            .fold(0.0_f32, f32::max);

        let company_sim = match (&imported.current_company, &existing.current_company) {
            (Some(i), Some(e)) => jaro_winkler(i, e) as f32,
            _ => 0.5, // unknown company — neutral
        };

        // Weighted combination: name 0.75, company 0.25
        (name_sim * 0.75 + company_sim * 0.25).min(0.99) // cap below 1.0 (reserved for exact email)
    }
}
```

**Verification:** Unit tests — exact email match returns 1.0, same name + same company > 0.92, different company shifts score down, completely different contact returns < 0.82.

#### Step 1.6 — SQLite Migration 022

File: `lazyjob-core/migrations/022_contact_import.sql`

Apply the DDL from the schema section above. Use `sqlx::migrate!` via `run_migrations()`.

**Verification:** `cargo test -p lazyjob-core -- --test-threads=1` runs the full migration stack on in-memory SQLite without errors.

#### Step 1.7 — `ContactImportService::start_import()` — Phase 1 Core

File: `lazyjob-core/src/contact_import/service.rs`

The orchestrator runs the import pipeline as a background tokio task:

```rust
pub async fn start_import(
    self: Arc<Self>,
    source: ImportSource,
) -> Result<ImportSessionId, ContactImportError> {
    let session_id = ImportSessionId::new();
    let svc = Arc::clone(&self);
    let sid = session_id.clone();

    tokio::spawn(async move {
        if let Err(e) = svc.run_import_pipeline(sid, source).await {
            let _ = svc.progress_tx.send(ImportProgress::Error(e.to_string()));
        }
    });

    Ok(session_id)
}

async fn run_import_pipeline(
    &self,
    session_id: ImportSessionId,
    source: ImportSource,
) -> Result<(), ContactImportError> {
    // 1. Persist session start
    self.persist_session_started(&session_id, &source).await?;

    // 2. Select parser
    let parser: &dyn ContactParser = match &source {
        ImportSource::LinkedInCsv { .. }     => &*self.linkedin_parser,
        ImportSource::VCardFile { .. }       => &*self.vcard_parser,
        ImportSource::Gmail { .. }           => &*self.gmail_parser,
        #[cfg(target_os = "macos")]
        ImportSource::AppleContacts { .. }   => &*self.apple_parser,
        ImportSource::BusinessCardPhoto { .. }=> &*self.biz_card_scanner,
        ImportSource::ManualEntry            => return Err(ContactImportError::ManualEntryNotBatchImport),
    };

    // 3. Parse
    let raw = parser.parse(&source).await?;
    let total = raw.len();
    let _ = self.progress_tx.send(ImportProgress::Started {
        session_id: session_id.clone(),
        total_contacts: total,
    });

    // 4. Normalize
    let normalized: Vec<ImportedContact> = raw.into_iter()
        .map(|mut c| {
            c.names.iter_mut().for_each(|n| n.full_name = ContactNormalizer::normalize_name(&n.full_name));
            c.current_company = c.current_company.map(|s| ContactNormalizer::normalize_company(&s));
            c
        })
        .collect();

    // 5. Load existing contacts for dedup (full scan; acceptable for <10k contacts)
    let existing = self.contact_repo.list_all_contacts().await?;

    // 6. Dedup
    let _ = self.progress_tx.send(ImportProgress::Deduplicating);
    let mut inserted = 0usize;
    let mut auto_merged = 0usize;
    let mut pending_review = 0usize;
    let mut skipped = 0usize;

    let mut tx = self.db.begin().await?;

    for (i, contact) in normalized.iter().enumerate() {
        let (decision, _existing_match) = self.dedup.classify(contact, &existing);

        match decision {
            ImportMergeDecision::NewContact => {
                self.contact_repo.insert_from_import_tx(&mut tx, contact).await?;
                inserted += 1;
            }
            ImportMergeDecision::AutoMerged { existing_contact_id, .. } => {
                self.contact_repo.merge_into_tx(&mut tx, existing_contact_id, contact).await?;
                auto_merged += 1;
            }
            ImportMergeDecision::PendingReview { existing_contact_id, confidence } => {
                sqlx::query!(
                    "INSERT INTO contact_import_pending_reviews
                     (session_id, existing_contact_id, imported_json, confidence, created_at)
                     VALUES (?, ?, ?, ?, datetime('now'))",
                    session_id.0.to_string(),
                    existing_contact_id.to_string(),
                    serde_json::to_string(contact).unwrap(),
                    confidence,
                )
                .execute(&mut *tx)
                .await?;
                pending_review += 1;
            }
        }

        if i % 50 == 0 {
            let _ = self.progress_tx.send(ImportProgress::Parsed { parsed: i + 1, total });
        }
    }

    tx.commit().await?;

    // 7. Update incremental watermark
    self.incremental.update_watermark(&source).await?;

    // 8. Finalize session
    let stats = ImportStats {
        session_id: session_id.clone(),
        source_key: source.source_key().to_owned(),
        parsed_count: total,
        inserted_count: inserted,
        auto_merged_count: auto_merged,
        pending_review_count: pending_review,
        skipped_count: skipped,
        started_at: Utc::now(), // re-read from DB in real impl
        completed_at: Utc::now(),
    };
    self.persist_session_completed(&session_id, &stats).await?;

    let _ = self.progress_tx.send(ImportProgress::Completed(stats));
    Ok(())
}
```

**Verification:** Integration test imports a 20-row LinkedIn CSV fixture and asserts correct `inserted_count`, `auto_merged_count` (for a duplicate row), and `pending_review_count`.

#### Step 1.8 — TUI Import Wizard

File: `lazyjob-tui/src/views/contacts/import_wizard.rs`

The import wizard is a 3-step modal overlay:

1. **Source selection**: Grid of source buttons (LinkedIn CSV, vCard, Gmail, Apple Contacts, Business Card, Manual Entry). Use `ratatui::widgets::Block` + `Paragraph` per cell, highlight focused cell with `Style::default().reversed()`.

2. **Source configuration**: Per-source config step:
   - LinkedIn CSV / vCard: file path input field + `[Browse]` hint (path is typed; no native file picker in terminal).
   - Gmail: shows "OAuth authorization required" → `[Authorize]` key launches browser.
   - Apple Contacts: shows "Permission required" → `[Grant Access]` triggers macOS dialog.
   - Business Card: file path input for the image.

3. **Progress / results**: Subscribes to `broadcast::Receiver<ImportProgress>` and renders a live summary:
   ```
   Importing LinkedIn CSV: contacts.csv
   ─────────────────────────────────────
   Parsed:        234 / 234
   New contacts:   198
   Auto-merged:     32
   Pending review:   4
   ─────────────────────────────────────
   [r] Review duplicates   [Enter] Dismiss
   ```

Key bindings: `[Tab]` move focus, `[Enter]` activate, `[Esc]` dismiss wizard.

**Verification:** Manual test: launch TUI, press `[i]` (import shortcut), navigate to LinkedIn CSV, enter fixture path, confirm import runs and progress is shown.

---

### Phase 2 — Gmail Contacts Import

**Goal:** Integrate Google Contacts API using OAuth2 PKCE flow. No client secret required for installed apps.

#### Step 2.1 — `GmailOAuthFlow` (`gmail_oauth.rs`)

OAuth2 PKCE device-flow or loopback redirect:

```rust
use oauth2::{
    AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
    AuthUrl, TokenUrl, ClientId, RedirectUrl, Scope,
    basic::BasicClient,
};
use keyring::Entry;

const GMAIL_CLIENT_ID: &str = env!("LAZYJOB_GMAIL_CLIENT_ID"); // set via build.rs env

pub struct GmailOAuthFlow {
    client: BasicClient,
}

impl GmailOAuthFlow {
    pub fn new() -> Self {
        let client = BasicClient::new(
            ClientId::new(GMAIL_CLIENT_ID.to_owned()),
            None, // PKCE public client; no secret
            AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".into()).unwrap(),
            Some(TokenUrl::new("https://oauth2.googleapis.com/token".into()).unwrap()),
        )
        .set_redirect_uri(RedirectUrl::new("http://localhost:8585/oauth/callback".into()).unwrap());

        Self { client }
    }

    /// Returns the URL to open in the browser + a verifier to complete the flow.
    pub fn begin_authorization(&self) -> (String, PkceCodeVerifier, CsrfToken) {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, csrf_token) = self.client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("https://www.googleapis.com/auth/contacts.readonly".into()))
            .set_pkce_challenge(pkce_challenge)
            .url();
        (url.to_string(), pkce_verifier, csrf_token)
    }

    /// Exchanges the auth code for tokens and stores them in the OS keyring.
    pub async fn complete_authorization(
        &self,
        code: AuthorizationCode,
        verifier: PkceCodeVerifier,
        account_email: &str,
    ) -> Result<(), ContactImportError> {
        let token = self.client
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| ContactImportError::OAuthError(e.to_string()))?;

        let access_token = token.access_token().secret().to_owned();
        let refresh_token = token.refresh_token()
            .map(|t| t.secret().to_owned())
            .unwrap_or_default();

        // Store in OS keyring — never SQLite.
        Entry::new("lazyjob", &format!("gmail_access_token::{account_email}"))
            .map_err(|e| ContactImportError::KeyringError(e.to_string()))?
            .set_password(&access_token)
            .map_err(|e| ContactImportError::KeyringError(e.to_string()))?;

        if !refresh_token.is_empty() {
            Entry::new("lazyjob", &format!("gmail_refresh_token::{account_email}"))
                .map_err(|e| ContactImportError::KeyringError(e.to_string()))?
                .set_password(&refresh_token)
                .map_err(|e| ContactImportError::KeyringError(e.to_string()))?;
        }

        Ok(())
    }

    pub fn load_access_token(&self, account_email: &str) -> Result<secrecy::Secret<String>, ContactImportError> {
        let token = Entry::new("lazyjob", &format!("gmail_access_token::{account_email}"))
            .map_err(|e| ContactImportError::KeyringError(e.to_string()))?
            .get_password()
            .map_err(|_| ContactImportError::NotAuthorized { source: "gmail".into() })?;
        Ok(secrecy::Secret::new(token))
    }
}
```

The TUI OAuth callback uses a local HTTP server spawned via `tokio::net::TcpListener` on port 8585, listens for exactly one GET request with `code=` and `state=` query params, extracts them, and shuts down. The user is instructed to open the authorization URL in their browser.

#### Step 2.2 — `GmailContactsParser` (`gmail.rs`)

```rust
use reqwest::Client;
use secrecy::ExposeSecret;

pub struct GmailContactsParser {
    http: Client,
    oauth: Arc<GmailOAuthFlow>,
    base_url: String, // "https://people.googleapis.com" — overridable for tests
}

impl GmailContactsParser {
    pub fn new(oauth: Arc<GmailOAuthFlow>) -> Self {
        Self {
            http: Client::new(),
            oauth,
            base_url: "https://people.googleapis.com".into(),
        }
    }

    /// Constructor that allows overriding the base URL for wiremock tests.
    pub fn with_base_url(oauth: Arc<GmailOAuthFlow>, base_url: impl Into<String>) -> Self {
        Self { http: Client::new(), oauth, base_url: base_url.into() }
    }
}

#[async_trait::async_trait]
impl ContactParser for GmailContactsParser {
    fn source_name(&self) -> &'static str { "Gmail Contacts" }

    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        let account_email = match source {
            ImportSource::Gmail { account_email, .. } => account_email.clone(),
            _ => return Err(ContactImportError::WrongSource),
        };

        let token = self.oauth.load_access_token(&account_email)?;

        // Paginate through all connections (page size 1000 max per Google docs).
        let mut contacts = vec![];
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!(
                "{}/v1/people/me/connections?personFields=names,emailAddresses,organizations,phoneNumbers&pageSize=1000",
                self.base_url
            );
            if let Some(pt) = &page_token {
                url.push_str(&format!("&pageToken={pt}"));
            }

            let resp: serde_json::Value = self.http
                .get(&url)
                .bearer_auth(token.expose_secret())
                .send()
                .await
                .map_err(|e| ContactImportError::HttpError(e.to_string()))?
                .json()
                .await
                .map_err(|e| ContactImportError::ParseError(e.to_string()))?;

            if let Some(connections) = resp["connections"].as_array() {
                for person in connections {
                    if let Some(c) = Self::parse_person(person, source) {
                        contacts.push(c);
                    }
                }
            }

            page_token = resp["nextPageToken"].as_str().map(str::to_owned);
            if page_token.is_none() { break; }
        }

        Ok(contacts)
    }
}

impl GmailContactsParser {
    fn parse_person(person: &serde_json::Value, source: &ImportSource) -> Option<ImportedContact> {
        let names: Vec<ContactName> = person["names"].as_array()
            .map(|arr| arr.iter().filter_map(|n| {
                let full = n["displayName"].as_str().map(str::to_owned);
                let given = n["givenName"].as_str().map(str::to_owned);
                let family = n["familyName"].as_str().map(str::to_owned);
                if full.is_none() && given.is_none() && family.is_none() { return None; }
                Some(ContactName::from_parts(given, family, full))
            }).collect())
            .unwrap_or_default();

        if names.is_empty() { return None; }

        let emails: Vec<Email> = person["emailAddresses"].as_array()
            .map(|arr| arr.iter()
                .filter_map(|e| e["value"].as_str().and_then(Email::parse))
                .collect())
            .unwrap_or_default();

        let phone_numbers: Vec<PhoneNumber> = person["phoneNumbers"].as_array()
            .map(|arr| arr.iter()
                .filter_map(|p| p["value"].as_str())
                .map(ContactNormalizer::normalize_phone)
                .collect())
            .unwrap_or_default();

        let current_company = person["organizations"].as_array()
            .and_then(|arr| arr.first())
            .and_then(|o| o["name"].as_str())
            .map(|s| ContactNormalizer::normalize_company(s));

        let current_title = person["organizations"].as_array()
            .and_then(|arr| arr.first())
            .and_then(|o| o["title"].as_str())
            .map(str::to_owned);

        Some(ImportedContact {
            import_id: Uuid::new_v4(),
            source: source.clone(),
            names,
            emails,
            phone_numbers,
            current_company,
            current_title,
            previous_companies: vec![],
            notes: None,
            linkedin_url: None,
            source_modified_at: None,
        })
    }
}
```

**Verification:** `wiremock` integration test mocking the People API with a 2-page fixture (1000 + 234 contacts). Assert total 1234 contacts, pagination terminates correctly.

---

### Phase 3 — Business Card Scanning

**Goal:** LLM-assisted extraction from a photo of a business card.

#### Step 3.1 — `BusinessCardScanner` (`business_card.rs`)

```rust
use base64::{Engine as _, engine::general_purpose};
use std::path::Path;

pub struct BusinessCardScanner {
    llm: Arc<dyn LlmProvider>,
}

impl BusinessCardScanner {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self { Self { llm } }
}

#[async_trait::async_trait]
impl ContactParser for BusinessCardScanner {
    fn source_name(&self) -> &'static str { "Business Card" }

    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        let photo_path = match source {
            ImportSource::BusinessCardPhoto { photo_path, .. } => photo_path.clone(),
            _ => return Err(ContactImportError::WrongSource),
        };

        let contact = self.scan_photo(&photo_path, source).await?;
        Ok(vec![contact])
    }
}

impl BusinessCardScanner {
    async fn scan_photo(&self, path: &Path, source: &ImportSource) -> Result<ImportedContact, ContactImportError> {
        // Read and base64-encode the image.
        let bytes = tokio::fs::read(path).await
            .map_err(|e| ContactImportError::IoError(e.to_string()))?;

        let b64 = general_purpose::STANDARD.encode(&bytes);
        let mime = Self::detect_mime(path);

        // Build a vision prompt requesting structured JSON output.
        let prompt = format!(
            r#"Extract the contact information from this business card image and return ONLY valid JSON.

JSON format:
{{
  "full_name": "...",
  "given_name": "...",
  "family_name": "...",
  "email": "...",
  "phone": "...",
  "company": "...",
  "title": "..."
}}

Return null for any field not visible on the card. Return ONLY the JSON object, no other text."#
        );

        let messages = vec![ChatMessage {
            role: ChatRole::User,
            content: ChatContent::ImageAndText {
                image_base64: b64,
                image_mime: mime.to_owned(),
                text: prompt,
            },
        }];

        let response = self.llm
            .chat_completion(ChatRequest {
                messages,
                temperature: 0.0,
                max_tokens: 256,
                ..Default::default()
            })
            .await
            .map_err(|e| ContactImportError::LlmError(e.to_string()))?;

        let text = response.content_text()
            .ok_or_else(|| ContactImportError::ParseError("LLM returned no text".into()))?;

        // Strip markdown code fences if present.
        let json_text = text.trim().trim_start_matches("```json").trim_end_matches("```").trim();

        #[derive(serde::Deserialize)]
        struct CardExtraction {
            full_name: Option<String>,
            given_name: Option<String>,
            family_name: Option<String>,
            email: Option<String>,
            phone: Option<String>,
            company: Option<String>,
            title: Option<String>,
        }

        let extracted: CardExtraction = serde_json::from_str(json_text)
            .map_err(|e| ContactImportError::ParseError(format!("LLM JSON malformed: {e}")))?;

        let name = ContactName::from_parts(
            extracted.given_name,
            extracted.family_name,
            extracted.full_name,
        );

        if name.full_name.is_empty() {
            return Err(ContactImportError::ParseError("Could not extract name from business card".into()));
        }

        let emails: Vec<Email> = extracted.email
            .and_then(Email::parse)
            .into_iter()
            .collect();

        let phone_numbers: Vec<PhoneNumber> = extracted.phone
            .map(|p| ContactNormalizer::normalize_phone(&p))
            .into_iter()
            .collect();

        Ok(ImportedContact {
            import_id: Uuid::new_v4(),
            source: source.clone(),
            names: vec![name],
            emails,
            phone_numbers,
            current_company: extracted.company.map(|s| ContactNormalizer::normalize_company(&s)),
            current_title: extracted.title,
            previous_companies: vec![],
            notes: None,
            linkedin_url: None,
            source_modified_at: None,
        })
    }

    fn detect_mime(path: &Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/jpeg", // default fallback
        }
    }
}
```

**Verification:** Unit test with a `MockLlmProvider` that returns a fixed JSON string for any vision request. Assert the returned `ImportedContact` has the expected name, email, and company. Test with a response that includes markdown code fences. Test with malformed JSON returns `ParseError`.

---

### Phase 4 — Apple Contacts (macOS)

**Goal:** Read from the macOS Contacts framework. If `objc2-contacts` proves insufficient, use a Swift shim.

#### Step 4.1 — FFI-based approach (preferred)

File: `lazyjob-core/src/contact_import/apple.rs`

```rust
#[cfg(target_os = "macos")]
pub struct AppleContactsParser;

#[cfg(target_os = "macos")]
#[async_trait::async_trait]
impl ContactParser for AppleContactsParser {
    fn source_name(&self) -> &'static str { "Apple Contacts" }

    async fn parse(&self, source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        // Contacts.framework is synchronous; run on blocking thread.
        tokio::task::spawn_blocking(move || Self::fetch_via_ffi(source))
            .await
            .map_err(|e| ContactImportError::Internal(e.to_string()))?
    }
}

#[cfg(target_os = "macos")]
impl AppleContactsParser {
    fn fetch_via_ffi(source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        use objc2_contacts::{CNContactStore, CNEntityType, CNContact};
        use objc2::rc::autoreleasepool;

        let mut contacts = vec![];

        autoreleasepool(|pool| {
            let store = CNContactStore::new();

            // Request access; blocks until user grants/denies.
            let (granted, error) = store.request_access_for_entity_type_completion_handler(
                CNEntityType::Contacts
            );

            if !granted {
                return Err(ContactImportError::PermissionDenied {
                    source: "Apple Contacts".into(),
                    detail: error.map(|e| e.description().to_string()),
                });
            }

            let keys = vec![
                CNContactGivenNameKey,
                CNContactFamilyNameKey,
                CNContactEmailAddressesKey,
                CNContactPhoneNumbersKey,
                CNContactOrganizationNameKey,
                CNContactJobTitleKey,
                CNContactUrlAddressesKey,
                CNContactNoteKey,
            ];

            let fetch_request = CNContactFetchRequest::new_with_keys_to_fetch(&keys);
            store.enumerate_contacts_with_fetch_request_error_using_block(
                &fetch_request,
                |cn_contact, _stop| {
                    let name = ContactName::from_parts(
                        Some(cn_contact.given_name().to_string()).filter(|s| !s.is_empty()),
                        Some(cn_contact.family_name().to_string()).filter(|s| !s.is_empty()),
                        None,
                    );
                    if name.full_name.is_empty() { return; }

                    let emails: Vec<Email> = cn_contact.email_addresses()
                        .iter()
                        .filter_map(|labeled| Email::parse(labeled.value().to_string()))
                        .collect();

                    let phones: Vec<PhoneNumber> = cn_contact.phone_numbers()
                        .iter()
                        .map(|labeled| ContactNormalizer::normalize_phone(
                            &labeled.value().string_value().to_string()
                        ))
                        .collect();

                    let current_company = Some(cn_contact.organization_name().to_string())
                        .filter(|s| !s.is_empty())
                        .map(|s| ContactNormalizer::normalize_company(&s));

                    contacts.push(ImportedContact {
                        import_id: Uuid::new_v4(),
                        source: source.clone(),
                        names: vec![name],
                        emails,
                        phone_numbers: phones,
                        current_company,
                        current_title: Some(cn_contact.job_title().to_string()).filter(|s| !s.is_empty()),
                        previous_companies: vec![],
                        notes: Some(cn_contact.note().to_string()).filter(|s| !s.is_empty()),
                        linkedin_url: cn_contact.url_addresses().iter()
                            .map(|u| u.value().to_string())
                            .find(|u| u.contains("linkedin.com")),
                        source_modified_at: None,
                    });
                },
            )?;
            Ok(contacts)
        })
    }
}

/// No-op stubs for non-macOS targets.
#[cfg(not(target_os = "macos"))]
pub struct AppleContactsParser;

#[cfg(not(target_os = "macos"))]
#[async_trait::async_trait]
impl ContactParser for AppleContactsParser {
    fn source_name(&self) -> &'static str { "Apple Contacts" }
    async fn parse(&self, _source: &ImportSource) -> Result<Vec<ImportedContact>, ContactImportError> {
        Err(ContactImportError::UnsupportedPlatform { feature: "Apple Contacts", platform: std::env::consts::OS })
    }
}
```

#### Step 4.2 — Swift shim fallback (if `objc2-contacts` proves insufficient)

If the FFI bindings are not stable enough, a minimal Swift binary (`lazyjob-contacts-helper`) is compiled as part of the macOS build and bundled alongside the main binary. It outputs NDJSON to stdout with one contact per line. `AppleContactsParser` spawns it via `tokio::process::Command` and parses the output:

```rust
// Fallback: spawn helper binary
let output = tokio::process::Command::new("lazyjob-contacts-helper")
    .output()
    .await
    .map_err(|e| ContactImportError::HelperBinaryNotFound(e.to_string()))?;

let contacts: Vec<ImportedContact> = String::from_utf8_lossy(&output.stdout)
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .collect();
```

This approach is documented as an Open Question (see below).

---

### Phase 5 — Incremental Import and Duplicate Review TUI

#### Step 5.1 — `IncrementalImportTracker` (`incremental.rs`)

```rust
pub struct IncrementalImportTracker {
    db: Arc<SqlitePool>,
}

impl IncrementalImportTracker {
    /// Returns the last import timestamp for a source, if any.
    pub async fn last_import_time(
        &self,
        source_key: &str,
        source_detail_key: &str,
    ) -> Result<Option<DateTime<Utc>>, ContactImportError> {
        let row = sqlx::query!(
            "SELECT last_imported_at FROM contact_import_watermarks
             WHERE source_key = ? AND source_detail_key = ?",
            source_key, source_detail_key
        )
        .fetch_optional(&*self.db)
        .await?;

        Ok(row.map(|r| DateTime::parse_from_rfc3339(&r.last_imported_at).ok())
               .flatten()
               .map(|dt| dt.with_timezone(&Utc)))
    }

    /// Records the current time as the last import timestamp.
    pub async fn update_watermark(
        &self,
        source: &ImportSource,
    ) -> Result<(), ContactImportError> {
        let key = source.source_key();
        let detail_key = Self::detail_key(source);
        let now = Utc::now().to_rfc3339();

        sqlx::query!(
            "INSERT INTO contact_import_watermarks (source_key, source_detail_key, last_imported_at)
             VALUES (?, ?, ?)
             ON CONFLICT(source_key, source_detail_key) DO UPDATE SET last_imported_at = excluded.last_imported_at",
            key, detail_key, now
        )
        .execute(&*self.db)
        .await?;

        Ok(())
    }

    fn detail_key(source: &ImportSource) -> String {
        match source {
            ImportSource::LinkedInCsv { file_path, .. } => {
                format!("{:x}", md5::compute(file_path.to_string_lossy().as_bytes()))
            }
            ImportSource::VCardFile { file_path, .. } => {
                format!("{:x}", md5::compute(file_path.to_string_lossy().as_bytes()))
            }
            ImportSource::Gmail { account_email, .. } => account_email.clone(),
            ImportSource::AppleContacts { .. } => "default".into(),
            ImportSource::BusinessCardPhoto { photo_path, .. } => {
                format!("{:x}", md5::compute(photo_path.to_string_lossy().as_bytes()))
            }
            ImportSource::ManualEntry => "manual".into(),
        }
    }
}
```

> Note: MD5 is used here as a non-cryptographic path key (not security-sensitive). No new crate needed — `md5 = "0.10"` added to dev-dependencies or replaced with a simple hash via `std::collections::hash_map::DefaultHasher`.

**Incremental filtering in parsers:** After parsing, `ContactImportService::run_import_pipeline()` filters `imported.source_modified_at >= last_import_time` before running deduplication. Contacts without `source_modified_at` are always included (no watermark-based skip possible).

#### Step 5.2 — TUI Duplicate Review View

File: `lazyjob-tui/src/views/contacts/dedup_review.rs`

Layout: 50/50 horizontal split.

```
┌─ Pending Duplicates (4) ─────────┬─ Review ─────────────────────────────────┐
│                                  │ Existing:   John Smith @ Acme Corp       │
│ > John Smith  (confidence: 0.87) │ Imported:   John Smith @ Acme Corp       │
│   Jane Doe    (confidence: 0.83) │ Source:     LinkedIn CSV                 │
│   Bob Jones   (confidence: 0.84) │ Email match: ✗  Name match: ●  Co: ●    │
│                                  │                                           │
│                                  │ Existing emails: john@old.com            │
│                                  │ Imported emails: jsmith@acme.com         │
│                                  │                                           │
│                                  │ Existing company: Acme Corp (2019–now)   │
│                                  │ Imported company: Acme Corp              │
│                                  │                                           │
│                                  │ [m] Merge  [d] Keep Distinct  [?] Help   │
└──────────────────────────────────┴───────────────────────────────────────────┘
```

Key bindings: `[j/k]` or `[↑/↓]` navigate list, `[m]` confirm merge, `[d]` keep distinct, `[Esc]` dismiss.

On `[m]` or `[d]`: calls `ContactImportService::confirm_merge()` or `::reject_merge()`, removes item from list, advances to next.

**Verification:** Unit test `ImportDedupService::classify()` with a known-similar pair — assert `PendingReview` with confidence in [0.82, 0.92).

---

## Key Crate APIs

- `csv::ReaderBuilder::new().flexible(true).from_reader(Cursor::new(&bytes))` — LinkedIn CSV parsing with reordered-column tolerance
- `vcard4::parse_str(&content) -> Result<Vec<VCard>, _>` — multi-VCARD block parsing
- `vcard4::VCard::fn_value() -> Option<&str>` — formatted name
- `vcard4::VCard::n_value() -> Option<&Name>` — structured name components
- `vcard4::VCard::email_values() -> impl Iterator<Item = &Email>` — email list
- `vcard4::VCard::tel_values() -> impl Iterator<Item = &Tel>` — phone list
- `oauth2::BasicClient::authorize_url()` — begin PKCE authorization
- `oauth2::BasicClient::exchange_code()` — exchange auth code for tokens
- `keyring::Entry::new(service, username).set_password(token)` — token storage
- `keyring::Entry::new(service, username).get_password()` — token retrieval
- `base64::engine::general_purpose::STANDARD.encode(&bytes)` — image encoding for LLM vision
- `strsim::jaro_winkler(a, b) -> f64` — fuzzy name matching in deduplication
- `tokio::task::spawn_blocking(|| csv_io())` — blocking I/O offloading
- `tokio::sync::broadcast::channel::<ImportProgress>(64)` — progress event bus
- `sqlx::Pool::begin() -> Transaction` — atomic batch upsert
- `sqlx::query!(...).execute(&mut *tx)` — transactional insert

---

## Error Handling

```rust
// lazyjob-core/src/contact_import/error.rs

#[derive(thiserror::Error, Debug)]
pub enum ContactImportError {
    #[error("Wrong import source type for this parser")]
    WrongSource,

    #[error("Manual entry cannot be batch-imported")]
    ManualEntryNotBatchImport,

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("OAuth error: {0}")]
    OAuthError(String),

    #[error("Keyring error: {0}")]
    KeyringError(String),

    #[error("Not authorized for source '{source}'")]
    NotAuthorized { source: String },

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("LLM error: {0}")]
    LlmError(String),

    #[error("Permission denied for '{source}': {detail:?}")]
    PermissionDenied { source: String, detail: Option<String> },

    #[error("Unsupported platform: '{feature}' is not available on {platform}")]
    UnsupportedPlatform { feature: &'static str, platform: &'static str },

    #[error("Apple Contacts helper binary not found: {0}")]
    HelperBinaryNotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
```

---

## Testing Strategy

### Unit Tests

| Test | File | What it tests |
|------|------|---------------|
| `test_email_parse_valid` | `types.rs` | `Email::parse("a@b.com")` returns `Some` |
| `test_email_parse_invalid` | `types.rs` | `Email::parse("not-an-email")` returns `None` |
| `test_normalize_company_legal_suffix` | `normalize.rs` | "Acme, Inc." → "acme" |
| `test_normalize_company_gmbh` | `normalize.rs` | "Firma GmbH" → "firma" |
| `test_normalize_phone_us_10_digit` | `normalize.rs` | "(212) 555-1234" → `e164: Some("+12125551234")` |
| `test_normalize_phone_international` | `normalize.rs` | "+44 20 7946 0958" → `e164: None` (non-US, raw preserved) |
| `test_linkedin_parser_happy_path` | `linkedin.rs` | 5-row fixture → 4 contacts (blank-name row skipped) |
| `test_linkedin_parser_no_email_column` | `linkedin.rs` | CSV without email column → contacts with empty email list |
| `test_vcard_parser_multi_block` | `vcard.rs` | 3-VCARD file → 2 contacts (1 blank FN skipped) |
| `test_vcard_parser_linkedin_url` | `vcard.rs` | vCard with URL property → `linkedin_url` extracted |
| `test_dedup_exact_email_match` | `dedup.rs` | Same email → `AutoMerged { confidence: 1.0 }` |
| `test_dedup_name_company_match` | `dedup.rs` | Same name + same company → `AutoMerged` (>= 0.92) |
| `test_dedup_name_only_match` | `dedup.rs` | Same name, no company data → `PendingReview` (~0.82) |
| `test_dedup_different_person` | `dedup.rs` | Different name + company → `NewContact` |
| `test_biz_card_scanner_happy` | `business_card.rs` | MockLlm returns valid JSON → populated `ImportedContact` |
| `test_biz_card_scanner_code_fence` | `business_card.rs` | LLM wraps JSON in ```json fences → still parsed correctly |
| `test_biz_card_scanner_malformed_json` | `business_card.rs` | LLM returns prose → `ParseError` |

### Integration Tests

| Test | What it tests |
|------|---------------|
| `test_import_linkedin_csv_end_to_end` | Full pipeline: parse fixture, normalize, dedup against empty DB, upsert, assert `ImportStats.inserted_count == 4` |
| `test_import_dedup_auto_merge` | Import same CSV twice: second run auto-merges all contacts |
| `test_import_pending_review_created` | Import contact with name+company match (no email) against existing contact: `pending_review_count == 1`, row appears in `contact_import_pending_reviews` |
| `test_gmail_parser_pagination` | `wiremock` mock: first page returns `nextPageToken`, second returns none; assert total count |
| `test_gmail_parser_token_not_found` | No keyring entry → `NotAuthorized` error |
| `test_incremental_watermark_updated` | After import, `contact_import_watermarks` row exists; second import (same source) has correct `last_imported_at` |

### TUI Tests

- Manual integration test: run TUI, open import wizard, import `tests/fixtures/linkedin_sample.csv`, verify progress panel updates and contacts list refreshes.
- Manual duplicate review: inject a pre-populated `contact_import_pending_reviews` row, open dedup review, press `[m]`, verify contact merged (columns updated in `profile_contacts`).

---

## Open Questions

1. **`objc2-contacts` stability**: The `objc2-contacts` crate may not have stable 0.2 bindings for all required `CNContact` keys. If Phase 4 hits blocking FFI issues, the Swift shim binary approach should be used. The shim compiles with `swift build` and outputs NDJSON. This requires documenting `swift` as a build-time macOS dependency.

2. **Gmail client ID distribution**: `LAZYJOB_GMAIL_CLIENT_ID` is read via `env!()` at compile time. This requires users who build from source to supply their own Google Cloud project client ID. Binary releases from CI can bundle a shared client ID (subject to Google's OAuth verification requirements for installed apps). An alternative is BYOK (bring your own client ID) stored in `config.toml`.

3. **LinkedIn URL in CSV**: The standard LinkedIn CSV export does not include the LinkedIn profile URL. The `linkedin_url` field will remain `None` for LinkedIn CSV imports. It is only populated from vCard URLs. Future work: TUI "link LinkedIn profile" flow where user pastes a URL manually.

4. **Incremental filtering for LinkedIn CSV**: LinkedIn CSV does not expose a `modified_at` per connection — only `Connected On`. The watermark filter therefore only excludes contacts connected before the last import timestamp. New connections added since the last import will be ingested; re-imports within the same day may produce false-positive new contacts if the connection date is today. Acceptable for MVP.

5. **Apple Contacts permission prompt timing**: macOS shows the Contacts permission dialog the first time `CNContactStore.requestAccess` is called. This must happen on the main thread. The `spawn_blocking` approach moves work to a blocking thread pool, which may or may not satisfy macOS's main-thread requirement for some system APIs. If this causes a crash, the alternative is to use a dedicated main-thread callback pattern via `dispatch_async_main` (available via `core-foundation` crate).

6. **Google People API rate limits**: 60 read requests/minute per user per project. For large contact lists (10,000+), the 1000-per-page pagination may approach this limit. A `tokio::time::sleep(Duration::from_secs(1))` between pages is sufficient mitigation for MVP.

7. **Merge strategy for `confirm_merge()`**: When merging an imported contact into an existing one, the current plan uses `COALESCE(excluded.field, existing.field)` — imported data fills gaps but does not overwrite non-null existing data. This is conservative. A future enhancement could show field-by-field merge conflict UI (like git mergetool).

---

## Related Specs

- `specs/networking-connection-mapping.md` — `ProfileContact` type, `ContactId`, `profile_contacts` table owner
- `specs/networking-connection-mapping-implementation-plan.md` — `ContactRepository` trait, `LinkedInCsvParser` (Phase 1 of this plan supersedes the simpler version there)
- `specs/07-gaps-networking-outreach-implementation-plan.md` — `FuzzyContactDeduplicator` (reused in `ImportDedupService`)
- `specs/16-privacy-security-implementation-plan.md` — keyring entry naming convention (`lazyjob::<namespace>::<key>`)
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait and `ChatMessage`/`ChatContent` types used by `BusinessCardScanner`
- `specs/XX-contact-identity-resolution.md` — post-import identity resolution (separate spec, not yet planned)
