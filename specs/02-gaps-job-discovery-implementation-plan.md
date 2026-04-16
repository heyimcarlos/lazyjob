# Gap Analysis: Job Discovery — Implementation Plan

## Spec Reference
- **Spec file**: `specs/02-gaps-job-discovery.md`
- **Status**: Researching (gap analysis document)
- **Last updated**: 2026-04-15

## Executive Summary
This document is a gap analysis reviewing 5 job discovery specs and identifying 11 critical-to-moderate gaps plus 3 cross-spec gaps. Unlike a feature spec, this document synthesizes findings and proposes new specs to create. This implementation plan covers how to act on the gap analysis: creating the proposed new specs, addressing cross-spec gaps, and integrating solutions into existing specs.

## Problem Statement
The job discovery system (05-job-discovery-layer, job-search-discovery-engine, job-search-semantic-matching, job-search-ghost-job-detection, job-search-company-research) has significant gaps in real-time alerting, authentication, data normalization, failure recovery, and filtering that limit LazyJob's effectiveness.

## Implementation Phases

### Phase 1: Create Critical New Specs (GAP-16, GAP-17)

#### 1.1 XX-job-alert-webhooks.md
Create a new spec for real-time job alert webhook receivers:
- Webhook endpoint architecture (nGROK tunnel for dev, LazyJob cloud relay for prod)
- HMAC signature authentication for webhook security
- Retry queue with exponential backoff for failed processing
- Email-based alert fallback for companies without webhooks
- Deduplication strategy to prevent webhook+polling duplicates

#### 1.2 XX-authenticated-job-sources.md
Create a new spec for authenticated job source integration:
- LinkedIn session cookie authentication flow
- Indeed and Glassdoor authentication
- Session rotation and re-authentication when cookies expire
- Captcha handling strategy
- 2FA account handling
- Secure storage of session tokens

### Phase 2: Address Important Gaps (GAP-18, GAP-19, GAP-20, GAP-21)

#### 2.1 XX-company-name-resolution.md
Create unified company name resolution spec:
- Normalization algorithm (strip Inc/LLC/Corp, lowercase, trim)
- Fuzzy matching with configurable threshold
- Acronym and abbreviation handling
- Parent/subsidiary company resolution
- Cross-source name reconciliation (Greenhouse "Stripe" + Lever "Stripe Inc." → same company)
- Case-insensitive matching

#### 2.2 XX-pay-transparency-jurisdictions-dynamic.md
Create dynamic jurisdiction updates spec:
- JSON-based jurisdiction list (not hardcoded in binary)
- Git-hosted jurisdiction updates with binary release cadence
- User-configurable custom jurisdictions
- State-level granularity for US (not just country)
- Historical enforcement exemption for pre-effective-date postings

#### 2.3 XX-company-staleness-per-field.md
Create per-field staleness tracking spec:
- Per-field `last_refreshed_at` timestamps
- Staleness thresholds by use case (ghost detection: monthly; interview prep: weekly; mission: quarterly)
- Refresh prioritization queue
- Partial update strategy for cost efficiency
- Optional history tracking of previous field values

#### 2.4 XX-job-notification-system.md
Create notification system spec:
- Notification channels (TUI notification, email, push)
- Notification filtering (match_score threshold)
- Quiet hours configuration
- Notification batching strategy
- Per-company notification preferences

### Phase 3: Address Moderate Gaps (GAP-22, GAP-23, GAP-24, GAP-25, GAP-26)

#### 3.1 XX-semantic-query-expansion.md
- User text query expansion (backend engineer → backend, fullstack, SRE, software engineer)
- Synonym handling (React/React.js/ReactJS)
- Acronym expansion (SRE → Site Reliability Engineer)
- Localization (US vs UK job titles)
- Career level expansion (Senior → Staff, Principal, Lead)

#### 3.2 XX-discovery-failure-recovery.md
- Partial failure handling (3 of 10 sources fail, 7 succeed)
- Rate limit exponential backoff strategy
- Source health tracking with auto-disable
- Manual retry capability
- Failure notification to user
- Circuit breaker pattern implementation

#### 3.3 XX-cross-source-priority.md
- Source priority ordering (LinkedIn > Greenhouse > Lever)
- Data quality by source matrix
- Trust scoring per source
- User-configurable source preferences
- Conflict resolution when sources have conflicting data

#### 3.4 XX-job-type-filtering.md
- Job type filtering (full-time, part-time, contract, internship)
- Experience level classification (entry, mid, senior, staff)
- Security clearance filtering
- Travel requirement filtering
- Job type extraction from description keywords

#### 3.5 XX-rate-limit-deep-design.md
- Per-source rate limit values documented
- Rate limit header parsing from APIs
- Global rate limiting across all sources
- Burst handling strategy
- Rate limit monitoring dashboard

### Phase 4: Cross-Spec Gap Resolution

#### 4.1 Cross-Gap D: Company Name Resolution Fragmentation
- Define unified `CompanyNameNormalizer` trait in lazyjob-core
- Update `CompanyConfig` (discovery) and `CompanyRecord` (research) to use same normalizer
- Update ghost job detection to use normalized company names
- Add migration for existing data with inconsistent company names

#### 4.2 Cross-Gap E: Embedding Model Migration
- Design embedding version field in database
- Create migration path for switching Ollama ↔ OpenAI embeddings
- Define embedding invalidation strategy when model changes
- Timeline: run migration during quiet period, background re-embed

#### 4.3 Cross-Gap F: Real-Time vs Batch Discovery Tension
- Define consistency model: webhook updates immediately visible in TUI
- Deduplication happens at ingestion (webhook + polling both go through same pipeline)
- Discovery run lock prevents webhook + polling from running simultaneously
- Webhook processing prioritized over scheduled polling

## Data Model

### New Database Tables/Fields

```sql
-- Per-field company staleness
ALTER TABLE companies ADD COLUMN tech_stack_refreshed_at TIMESTAMP;
ALTER TABLE companies ADD COLUMN mission_refreshed_at TIMESTAMP;
ALTER TABLE companies ADD COLUMN culture_refreshed_at TIMESTAMP;
ALTER TABLE companies ADD COLUMN salary_range_refreshed_at TIMESTAMP;

-- Source health tracking
CREATE TABLE source_health (
    source_id TEXT PRIMARY KEY,
    last_success_at TIMESTAMP,
    last_failure_at TIMESTAMP,
    consecutive_failures INTEGER DEFAULT 0,
    is_disabled BOOLEAN DEFAULT FALSE
);

-- Notification preferences per company
CREATE TABLE company_notification_prefs (
    company_id TEXT PRIMARY KEY,
    notify_mode TEXT DEFAULT 'instant', -- instant, daily, never
    match_threshold REAL DEFAULT 0.7
);

-- Webhook retry queue
CREATE TABLE webhook_retry_queue (
    id INTEGER PRIMARY KEY,
    payload TEXT,
    source TEXT,
    retry_count INTEGER DEFAULT 0,
    next_retry_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### New Structs/Types

```rust
// lazyjob-core/src/company.rs
pub struct CompanyNameNormalizer;
impl CompanyNameNormalizer {
    pub fn normalize(name: &str) -> String;
    pub fn fuzzy_match(a: &str, b: &str) -> f64; // 0.0-1.0
}

// lazyjob-core/src/discovery.rs
pub struct SourceHealth {
    pub source_id: String,
    pub is_healthy: bool,
    pub consecutive_failures: u32,
}

pub struct JobAlertWebhook {
    pub source: String,
    pub job_id: String,
    pub company_name: String,
    pub title: String,
    pub url: String,
    pub posted_at: DateTime<Utc>,
}

// lazyjob-core/src/notification.rs
pub struct NotificationPrefs {
    pub channel: NotificationChannel,
    pub quiet_hours_start: Option<NaiveTime>,
    pub quiet_hours_end: Option<NaiveTime>,
    pub batch_threshold: u32,
}
```

## API Surface

### New Public Modules/Functions

```
lazyjob-core/
├── src/
│   ├── company/
│   │   ├── normalize(name: &str) -> String
│   │   ├── fuzzy_match(a: &str, b: &str) -> f64
│   │   └── resolve_cross_source(names: Vec<&str>) -> String
│   ├── discovery/
│   │   ├── SourceHealth::is_healthy()
│   │   ├── discover_with_fallback(sources: &[Source]) -> Result<Vec<Job>>
│   │   └── retry_webhook(queue_id: i64) -> Result<()>
│   ├── notification/
│   │   ├── send_notification(job: &Job, prefs: &NotificationPrefs)
│   │   └── check_quiet_hours(prefs: &NotificationPrefs) -> bool
│   └── staleness/
│       ├── is_field_stale(company: &Company, field: Field, threshold: Duration) -> bool
│       └── refresh_priority(company: &Company) -> Vec<Field>

lazyjob-cli/
├── src/
│   └── webhook_server.rs  -- HTTP server for webhook reception
```

## Key Technical Decisions

### 1. Webhook Security
**Decision**: Use HMAC-SHA256 signatures with per-source shared secrets
**Rationale**: Industry standard for webhook authentication. Simple to implement, hard to forge.
**Rejected alternatives**:
- Bearer tokens: Less secure, susceptible to replay attacks
- IP whitelist: Too rigid for cloud deployment

### 2. Company Name Resolution
**Decision**: Normalize first, then fuzzy match with configurable threshold (default 0.85)
**Rationale**: Balances false positives (different companies merged) with false negatives (same company split)
**Rejected alternatives**:
- Exact match only: Too brittle, misses "Stripe Inc." vs "Stripe"
- Full semantic embedding: Overkill for company name matching

### 3. Staleness Tracking
**Decision**: Per-field timestamps, configurable thresholds per use case
**Rationale**: Different data changes at different rates; one-size-fits-all staleness doesn't work
**Rejected alternatives**:
- Single is_stale flag: Already proven insufficient (GAP-20 acknowledges this)
- Real-time refresh: API cost prohibitive

### 4. Notification Batching
**Decision**: Batch into single notification if >5 jobs arrive within 15 minutes
**Rationale**: Prevents notification spam while keeping alerts timely
**Rejected alternatives**:
- Immediate notification for each job: Spam risk
- Daily digest only: Too slow for time-sensitive opportunities

## File Structure

```
lazyjob/
├── lazyjob-core/
│   ├── src/
│   │   ├── company.rs           # MODIFIED: Add CompanyNameNormalizer
│   │   ├── discovery.rs         # MODIFIED: Add SourceHealth, retry logic
│   │   ├── notification.rs      # NEW: Notification preferences and sending
│   │   ├── staleness.rs         # NEW: Per-field staleness tracking
│   │   └── schema.sql           # MODIFIED: Add new tables/columns
│   └── migrations/
│       └── 002_add_discovery.sql  # NEW: Source health, staleness fields
├── lazyjob-cli/
│   ├── src/
│   │   ├── main.rs              # MODIFIED: Add webhook endpoint registration
│   │   └── webhook_server.rs    # NEW: HTTP webhook receiver
├── lazyjob-tui/
│   └── src/
│       ├── views/
│       │   └── notifications.rs  # NEW: Notification preferences TUI
│       └── commands/
│           └── discovery.rs      # MODIFIED: Add source health display
└── specs/
    ├── XX-job-alert-webhooks.md           # NEW: Created in Phase 1
    ├── XX-authenticated-job-sources.md    # NEW: Created in Phase 1
    ├── XX-company-name-resolution.md      # NEW: Created in Phase 2
    ├── XX-pay-transparency-jurisdictions-dynamic.md  # NEW: Created in Phase 2
    ├── XX-company-staleness-per-field.md   # NEW: Created in Phase 2
    ├── XX-job-notification-system.md       # NEW: Created in Phase 2
    ├── XX-semantic-query-expansion.md      # NEW: Created in Phase 3
    ├── XX-discovery-failure-recovery.md   # NEW: Created in Phase 3
    ├── XX-cross-source-priority.md         # NEW: Created in Phase 3
    ├── XX-job-type-filtering.md            # NEW: Created in Phase 3
    └── XX-rate-limit-deep-design.md        # NEW: Created in Phase 3
```

## Dependencies

### External Crates
- `tracing` (already in use): For webhook request logging
- `reqwest` (already in use): For webhook verification requests
- `tokio-cron-scheduler` or `tokio::time::interval`: For retry queue processing
- `HMAC`/`sha2` crates: For webhook signature verification (already in deps)

### Spec Dependencies (must be implemented first)
1. `01-architecture.md` — Core crate structure
2. `04-sqlite-persistence.md` — Database schema foundation
3. `05-job-discovery-layer.md` — Discovery service baseline
4. `job-search-discovery-engine.md` — Discovery engine baseline

### Cross-Spec Dependencies
- `job-search-semantic-matching.md` — Embedding model migration (Cross-Gap E)
- `job-search-company-research.md` — CompanyRecord model (Cross-Gap D)
- `job-search-ghost-job-detection.md` — Ghost score integration (Cross-Gap D)

## Testing Strategy

### Unit Tests
- `CompanyNameNormalizer::normalize()`: Test all suffix variations, whitespace, case
- `CompanyNameNormalizer::fuzzy_match()`: Test threshold boundaries
- `is_field_stale()`: Test various durations and timestamps
- `check_quiet_hours()`: Test time boundaries and timezone handling

### Integration Tests
- Webhook server: Send test HMAC-signed webhook, verify job created
- Source health: Simulate failures, verify auto-disable after threshold
- Retry queue: Simulate processing failure, verify exponential backoff
- Deduplication: Send same job via webhook + polling, verify single entry

### Edge Cases
- Webhook received with invalid HMAC signature → Reject with 401
- Webhook for unknown company → Create company with normalized name
- All sources fail → Notify user, preserve last successful data
- Conflicting salary data from two sources → Use priority ordering
- Job type extraction ambiguous → Default to full-time with low confidence flag

## Open Questions

1. **Webhook relay for local dev**: Should nGROK be auto-configured, or manual setup required?
2. **LinkedIn cookie extraction**: Should LazyJob provide a browser extension or manual copy-paste?
3. **Email forwarding**: What email provider for email-based alerts? Custom domain or user-provided SMTP?
4. **Notification delivery**: TUI notifications only, or external (email/push) for MVP?
5. **Embedding migration UX**: How to handle large job corpus re-embedding without blocking TUI?

## Effort Estimate

**Rough estimate**: 3-4 weeks

**Breakdown**:
- Phase 1 (Critical specs): 1 week — Webhooks and authenticated sources are complex (security, session management)
- Phase 2 (Important gaps): 1 week — Company name resolution and staleness require schema migrations
- Phase 3 (Moderate gaps): 5 days — Mostly feature additions without major schema changes
- Phase 4 (Cross-spec resolution): 3 days — Integration work, testing, migration scripts

**Note**: This gap analysis spec doesn't implement features directly—it spawns 11 new specs that themselves require implementation. The plan above is for creating those new specs, not implementing the features themselves.