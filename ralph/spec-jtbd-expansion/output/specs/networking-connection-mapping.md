# Spec: Networking Connection Mapping

**JTBD**: A-4 — Get warm introductions that beat cold applications
**Topic**: Map the user's imported professional contacts to target companies to surface warm paths to specific open roles
**Domain**: networking

---

## What

A contact-company mapping service that cross-references the user's manually-imported professional network against their active job feed. For any job in the feed, it ranks every relevant contact by warmth tier — first-degree current employees first, then alumni, then second-degree paths — and surfaces a suggested approach for each (referral ask, informational interview, reconnect first, or cold outreach).

## Why

Referrals account for 30–50% of all hires and create 7–18x higher hire probability than inbound applications. Yet most job seekers never systematically map their network to their target company list — the mapping effort is too large to do manually across dozens of companies. Without tooling, people either skip networking entirely (leaving the most effective channel untouched) or reach out randomly without understanding relationship warmth, which produces the 3–5% cold response rates that make networking feel futile.

## How

### Data Sources and Import

LazyJob has **no LinkedIn API access** and does not perform any LinkedIn scraping. All contact data is user-imported:

1. **LinkedIn CSV export** (`My Network → Connections → Export → connections.csv`) — includes first name, last name, email, company, position, connected on date.
2. **Manual entry** via TUI form.
3. **Application contacts** promoted to profile_contacts when a recruiter or hiring manager from an application becomes a general network contact (user action).

The `profile_contacts` table (established in `profile-life-sheet-data-model.md`) is the single authoritative contacts store for JTBD A-4. It is **distinct** from `application_contacts` (which tracks per-hire-process recruiters and interviewers).

### Contact-Company Matching

Matching is performed in `lazyjob-core/src/networking/connection_mapper.rs`:

1. **Direct current-company match**: `contact.current_company_name` (normalized) matches `company.name` in `CompanyRepository`. Company name normalization uses the same `normalize_company_name()` function as the deduplication pipeline (lowercase, strip LLC/Inc/Corp suffixes, strip punctuation).
2. **Former-company alumni match**: `contact.previous_companies: Vec<PreviousCompany>` parsed from the `profile_contacts.previous_companies_json` column. Alumni matches are lower-tier than current employees.
3. **Second-degree approximation**: if contact A is at company X and contact B lists the same company as a previous employer in the same overlapping date range, B might know A. This is heuristic-only — no algorithmic graph traversal since LazyJob doesn't have LinkedIn's graph. Flagged as `ConnectionTier::SecondDegreeHeuristic` with low confidence.

### Warmth Tiers

```
ConnectionTier::FirstDegreeCurrentEmployee  → score 100
ConnectionTier::FirstDegreeRecentAlumni     → score 70   (left < 2 years ago)
ConnectionTier::FirstDegreeAlumni           → score 40   (left 2–5 years ago)
ConnectionTier::FirstDegreeDistantAlumni    → score 20   (left > 5 years ago)
ConnectionTier::SecondDegreeHeuristic       → score 10   (same former employer overlap)
ConnectionTier::SharedAlumni { school }     → score 15   (same school)
ConnectionTier::Cold                        → score 0
```

Tier is computed at query time, not stored — because contacts change jobs and stale data would make stored tiers misleading.

### Suggested Approach

`SuggestedApproach` is computed from `ConnectionTier` + `last_contacted_at`:

| Tier | Last contact | Suggested approach |
|------|-------------|-------------------|
| FirstDegreeCurrentEmployee | < 90 days | `RequestReferral` |
| FirstDegreeCurrentEmployee | 90–365 days | `ReconnectFirst` |
| FirstDegreeCurrentEmployee | > 365 days / never | `ReconnectFirst` |
| FirstDegreeRecentAlumni | any | `InformationalInterview` |
| FirstDegreeAlumni | any | `InformationalInterview` |
| SharedAlumni | any | `InformationalInterview` |
| SecondDegreeHeuristic / Cold | any | `ColdOutreach` |

### TUI Integration

The per-job detail view (TUI) includes a "Warm Paths" panel listing ranked contacts. The panel is rendered by `lazyjob-tui/src/views/job_detail/warm_paths.rs`. Keybinding: `n` from the job detail view opens the Warm Paths panel; `Enter` on a contact opens the outreach drafting view (see `networking-outreach-drafting.md`).

## Interface

```rust
// lazyjob-core/src/networking/connection_mapper.rs

pub enum ConnectionTier {
    FirstDegreeCurrentEmployee,
    FirstDegreeRecentAlumni { months_since_departure: u32 },
    FirstDegreeAlumni { months_since_departure: u32 },
    FirstDegreeDistantAlumni { months_since_departure: u32 },
    SharedAlumni { institution: String },
    SecondDegreeHeuristic { via_shared_employer: String },
    Cold,
}

pub enum SuggestedApproach {
    RequestReferral,
    InformationalInterview,
    ReconnectFirst,
    ColdOutreach,
}

pub struct WarmPath {
    pub contact_id: Uuid,
    pub contact_name: String,
    pub contact_current_title: Option<String>,
    pub company_id: Uuid,
    pub tier: ConnectionTier,
    pub tier_score: u8,
    pub last_contacted_at: Option<NaiveDate>,
    pub relationship_notes: Option<String>,
    pub suggested_approach: SuggestedApproach,
}

pub struct ImportResult {
    pub inserted: usize,
    pub updated: usize,
    pub skipped_duplicates: usize,
    pub parse_errors: Vec<String>,
}

#[async_trait]
pub trait ContactRepository: Send + Sync {
    async fn list_contacts(&self) -> Result<Vec<ProfileContact>>;
    async fn upsert_contact(&self, contact: ProfileContact) -> Result<Uuid>;
    async fn contacts_at_company(&self, company_id: Uuid) -> Result<Vec<ProfileContact>>;
    async fn contacts_with_company_name(&self, name: &str) -> Result<Vec<ProfileContact>>;
    async fn find_by_email(&self, email: &str) -> Result<Option<ProfileContact>>;
}

pub struct ConnectionMapper {
    contact_repo: Arc<dyn ContactRepository>,
    company_repo: Arc<dyn CompanyRepository>,
    job_repo: Arc<dyn JobRepository>,
}

impl ConnectionMapper {
    /// Returns ranked warm paths for a specific job, sorted by tier_score DESC
    pub async fn warm_paths_for_job(&self, job_id: Uuid) -> Result<Vec<WarmPath>>;

    /// Returns all contacts associated with a company (by company_id or name match)
    pub async fn contacts_at_company(&self, company_id: Uuid) -> Result<Vec<WarmPath>>;

    /// Parses LinkedIn connections CSV and upserts into profile_contacts
    pub async fn import_linkedin_csv(&self, csv_path: &Path) -> Result<ImportResult>;
}

// DDL additions to profile_contacts table (lazyjob-core/src/db/schema.sql):
// previous_companies_json TEXT,           -- JSON array of {company, title, start_date, end_date}
// schools_json            TEXT,           -- JSON array of {institution, degree, graduation_year}
// relationship_notes      TEXT,
// last_contacted_at       DATE,
// contact_source          TEXT NOT NULL DEFAULT 'manual',  -- 'manual' | 'linkedin_csv' | 'application_promoted'
```

## Open Questions

- **Name normalization collision rate**: `normalize_company_name()` may match "Apple" (the fruit company) to "Apple Inc" (the tech company) if a contact works at an unrelated Apple. Should name matching require additional signals (location, industry)? Phase 1: accept false positives and let the user dismiss; Phase 2: use `company_id` FK if contact data includes a company URL.
- **LinkedIn CSV schema stability**: LinkedIn periodically changes its CSV export format. Should the parser be defensive (column-name-based, not position-based) or should we version the parser? Recommendation: column-name-based parsing with explicit unknown-column logging.
- **Second-degree heuristic**: The shared-former-employer heuristic for 2nd-degree approximation will produce false matches. Should it be off by default and opt-in? Or shown with a clear confidence indicator?
- **Contact import permissions**: Importing a LinkedIn CSV involves importing data about third parties (your contacts). This should be local-only with explicit in-app notice.

## Implementation Tasks

- [ ] Add `previous_companies_json`, `schools_json`, `relationship_notes`, `last_contacted_at`, `contact_source` columns to `profile_contacts` DDL in `lazyjob-core/src/db/schema.sql`
- [ ] Implement `ConnectionMapper::import_linkedin_csv` with column-name-based CSV parsing and upsert-by-email logic in `lazyjob-core/src/networking/csv_import.rs`
- [ ] Implement `ConnectionMapper::warm_paths_for_job` with company name normalization matching against `CompanyRepository` in `lazyjob-core/src/networking/connection_mapper.rs`
- [ ] Implement `ConnectionTier` computation and `SuggestedApproach` classification rules
- [ ] Add `ContactRepository` trait and `SqliteContactRepository` impl in `lazyjob-core/src/networking/`
- [ ] Add Warm Paths panel to job detail TUI view (`lazyjob-tui/src/views/job_detail/warm_paths.rs`) with tier-badged contact list and `n` keybinding
- [ ] Write unit tests for company name normalization edge cases (suffixes, punctuation, case) and `ConnectionTier` scoring logic
