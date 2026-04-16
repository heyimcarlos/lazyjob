# Gap Analysis: Job Discovery (05, job-search-* specs)

## Specs Reviewed
- `05-job-discovery-layer.md` - Job Discovery Layer
- `job-search-discovery-engine.md` - Job Discovery Engine
- `job-search-semantic-matching.md` - Semantic Job Matching
- `job-search-ghost-job-detection.md` - Ghost Job Detection
- `job-search-company-research.md` - Company Research Pipeline

---

## What's Well-Covered

### job-search-ghost-job-detection.md
- 7-signal heuristic classifier (repost_count, description_vagueness, salary_absent, etc.)
- Ghost score thresholds (0.3 = possibly stale, 0.6 = likely ghost)
- TUI badge integration with human-readable explanations
- User override capability
- Classification update schedule (at discovery, daily, on override)
- Clear design constraint: never silently hide a job

### job-search-semantic-matching.md
- Two-phase architecture (embedding generation + scoring)
- Embedding model choices with privacy/cost tradeoff table
- ESCO-aligned skill inference with confidence scores
- Feed ranking formula combining match_score, ghost_score, recency, feedback
- Feedback-driven score adjustment (saved → boost, dismissed → penalty)
- Open question about Ollama availability fallback

### job-search-company-research.md
- Data source matrix with Phase 1 vs Phase 2 split
- LLM extraction prompt design with anti-fabrication constraint
- Tech stack inference from job descriptions (offline, no API)
- CompanyRecord model with comprehensive fields
- Staleness tracking (is_stale flag)

### 05-job-discovery-layer.md
- Greenhouse and Lever API specs
- Enrichment pipeline (HTML sanitization, salary extraction, remote classification)
- Source registry and company registry patterns
- Platform support matrix (Greenhouse, Lever, Workday, LinkedIn, Indeed, Glassdoor)
- Discovery service architecture

### job-search-discovery-engine.md
- Data flow diagram
- Multi-source fan-out with tokio::spawn
- Rate limiter per source
- Deduplication strategy with cross-source fuzzy matching
- Config format with polling_interval_minutes

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-16: Real-Time Job Alert Webhooks (CRITICAL)

**Location**: All job discovery specs - only polling is covered

**What's missing**:
1. **Webhook receivers**: How does LazyJob receive real-time alerts when companies post new jobs? Many ATS platforms (Greenhouse, Lever) don't support webhooks natively
2. **Webhook endpoint**: How is the webhook endpoint exposed? (e.g., nGROK tunnel for local dev, or LazyJob cloud relay?)
3. **Webhook security**: How to authenticate webhook requests? HMAC signatures? Bearer tokens?
4. **Webhook retry queue**: What if webhook received but processing fails? Retry with exponential backoff?
5. **Email-based alerts fallback**: For companies without webhooks, can user forward job alert emails to a LazyJob inbox?

**Why critical**: Polling every hour means users miss jobs for up to 59 minutes. Real-time alerts give competitive advantage.

**What could go wrong**:
- Webhook endpoint exposed without auth, receives spam/malicious requests
- Webhook queue grows unbounded during processing failures
- User gets duplicate notifications (webhook + polling both fire)

---

### GAP-17: Authenticated Job Source Integration (CRITICAL)

**Location**: `05-job-discovery-layer.md` - mentions LinkedIn/Indeed scraping as problematic

**What's missing**:
1. **LinkedIn session cookies**: How to authenticate to LinkedIn via cookies? What's the session management?
2. **Indeed authentication**: Does Indeed require login for full job access?
3. **Glassdoor login**: Can Glassdoor data be accessed with free account?
4. **Session rotation**: When cookies expire, how does LazyJob re-authenticate?
5. **Captcha handling**: How are captchas handled during scraping/auth?
6. **2FA accounts**: What if user's LinkedIn has 2FA enabled?

**Why critical**: The highest-value job sources (LinkedIn, Indeed) require authentication. Without this, LazyJob misses most job listings.

**What could go wrong**:
- User provides cookies, LinkedIn detects and invalidates them
- Session expires mid-discovery run, partial data collected
- Captcha blocks all requests, discovery fails completely
- Storing cookies securely - if LazyJob DB is compromised, session tokens leaked

---

### GAP-18: Unified Company Name Resolution (IMPORTANT)

**Location**: All specs reference company names but normalization is underspecified

**What's missing**:
1. **Normalization algorithm**: What's the exact algorithm? Strip Inc/LLC/Corp suffixes? Lowercase? Trim whitespace?
2. **Fuzzy matching**: "Stripe Inc." and "stripe" - are they the same? What's the threshold?
3. **Acronym handling**: "FAANG" → "Meta"? "M" → "Meta"? Who decides?
4. **Subsidiaries/parent companies**: "Google" vs "Alphabet" - same or different?
5. **Cross-source name reconciliation**: Greenhouse returns "Stripe", Lever returns "Stripe Inc.", config has "stripe" - how merged?
6. **Case sensitivity**: "apple" vs "Apple" - case-insensitive match?

**Why important**: Company records are split across discovery (CompanyConfig) and research (CompanyRecord). Without unified resolution, user sees duplicate company entries.

**What could go wrong**:
- User sees "Stripe" and "stripe" as two separate companies in TUI
- Job from Greenhouse with company name "Stripe Inc." doesn't link to existing Stripe CompanyRecord
- A company acquisition causes name change, old jobs orphaned

---

### GAP-19: Dynamic Pay Transparency Jurisdiction Updates (IMPORTANT)

**Location**: `job-search-ghost-job-detection.md` - mentions static embedded list

**What's missing**:
1. **How to update jurisdictions**: When a new state/country passes pay transparency law, how is LazyJob updated?
2. **Binary release cadence**: Users may be on old versions with stale jurisdiction lists
3. **User-provided updates**: Can users add jurisdictions manually? Is there a config file?
4. **Jurisdiction granularity**: States in US have different laws - should it be state-level, not just country?
5. **Historical enforcement**: For jobs posted before law effective date, should they be exempt?

**Why important**: Pay transparency laws are rapidly expanding (EU Pay Transparency Directive 2026, more US states). Static list quickly becomes outdated.

**What could go wrong**:
- User in new jurisdiction gets false ghost warnings because binary hasn't been updated
- Binary release delay means users on old versions are underserved
- No way for power users to add their jurisdiction before binary update

---

### GAP-20: Per-Field Company Data Staleness (IMPORTANT)

**Location**: `job-search-company-research.md` - has single `is_stale` flag but Open Question #4 acknowledges this is too coarse

**What's missing**:
1. **Per-field staleness**: Different data has different refresh frequencies - how to track each field?
2. **Staleness thresholds by use case**: Ghost detection needs monthly; interview prep needs weekly; mission alignment needs quarterly?
3. **Refresh prioritization**: When re-running enrichment, which fields get refreshed first?
4. **Partial updates**: If only tech_stack is stale, should we refresh only that field?
5. **History tracking**: Should we track previous values of fields over time?

**Why important**: Company data changes at different rates. Tech stack changes monthly; mission statement yearly. A single staleness flag can't capture this.

**What could go wrong**:
- Company tech stack updated but mission statement is 2 years old, both marked equally "stale"
- Re-enriching everything wastes API calls; not enriching enough leaves stale data

---

### GAP-21: Job Alert Notification System (MODERATE)

**Location**: None of the specs address notifications for new job alerts

**What's missing**:
1. **Notification channels**: How does user receive alerts? TUI notification? Email? Push?
2. **Notification filtering**: Only notify for high-match-score jobs? Above certain threshold?
3. **Quiet hours**: Can user set "don't notify between 10pm-8am"?
4. **Notification batching**: Batch multiple new jobs into single notification?
5. **Notification preferences per company**: User may want instant alerts for "dream companies", daily digest for others?

**Why important**: Without notifications, users must remember to open LazyJob to check for new jobs.

**What could go wrong**:
- User gets spammed with notifications for every new job, turns them off entirely
- Notification sent for low-quality ghost job, user loses trust in system
- Email notifications land in spam folder

---

### GAP-22: Semantic Search Query Expansion (MODERATE)

**Location**: `job-search-semantic-matching.md` - only covers job-to-profile matching

**What's missing**:
1. **User text query expansion**: When user searches "backend engineer", expand to "backend", "backend engineer", "fullstack", "software engineer", "SRE"?
2. **Synonym handling**: "React" vs "React.js" vs "ReactJS" - same job should match
3. **Acronym expansion**: "SRE" → "Site Reliability Engineer", "SWE" → "Software Engineer"
4. **Localization**: UK vs US job titles ("Software Engineer" vs "Developer")
5. **Career level expansion**: "Senior" jobs should also match "Staff", "Principal", "Lead"

**Why important**: Users don't always know the exact job title. Query expansion helps them find relevant jobs they didn't search for.

**What could go wrong**:
- Over-expansion returns too many irrelevant results
- Under-expansion misses relevant jobs
- Expansion synonyms become stale as job market terminology evolves

---

### GAP-23: Job Discovery Failure Recovery (MODERATE)

**Location**: All discovery specs - failures mentioned but recovery underspecified

**What's missing**:
1. **Partial failure handling**: If 3 of 10 company sources fail, what happens to the 7 that succeeded?
2. **Rate limit backoff**: When rate limited, what's the backoff strategy? Minutes? Hours?
3. **Source health tracking**: If a source consistently fails, should it be auto-disabled?
4. **Manual retry**: Can user manually trigger retry for failed discovery runs?
5. **Failure notification**: Does user get notified when discovery fails? Only persistent failures?
6. **Circuit breaker pattern**: Should we implement circuit breaker to stop calling a failing source?

**Why important**: Discovery will fail occasionally (network issues, API changes). How the system recovers determines user experience.

**What could go wrong**:
- One failing source causes entire discovery run to abort, 9 other sources not checked
- Rate limited, immediately retries, gets more rate limited
- Source broken but user doesn't know, assumes discovery is working

---

### GAP-24: Cross-Source Job Priority (MODERATE)

**Location**: `job-search-discovery-engine.md` - mentions cross-source deduplication but not priority

**What's missing**:
1. **Source priority**: When same job appears on LinkedIn AND Greenhouse, which data wins?
2. **Data quality by source**: Which source has more complete salary data? Better descriptions?
3. **Trust scoring by source**: Should some sources have lower default match_score adjustments?
4. **User preference**: Can user prefer "LinkedIn jobs over Greenhouse" or is it arbitrary?
5. **Conflict resolution**: When two sources have conflicting salary data, which wins?

**Why important**: Same job appears on multiple platforms with different data. User sees confusing duplicates if not resolved.

**What could go wrong**:
- User sees same job twice with slightly different titles, doesn't realize they're the same
- LinkedIn has salary range but Greenhouse doesn't - which is shown in TUI?
- User applies via LazyJob to Greenhouse URL but original intent was LinkedIn

---

### GAP-25: Job Type and Experience Level Filtering (MODERATE)

**Location**: `05-job-discovery-layer.md` - no mention of filtering by job type

**What's missing**:
1. **Job type filtering**: Full-time vs part-time vs contract vs internship - can user filter?
2. **Experience level**: Entry-level vs senior vs staff - how classified?
3. **Security clearance**: Can user filter by clearance requirements?
4. **Travel requirements**: % travel needed - filterable?
5. **Job type from description**: How is job type extracted? Keywords? Does it exist in data?

**Why important**: Users may specifically want contract roles or only entry-level positions.

**What could go wrong**:
- User gets spammed with senior-level jobs when they're entry-level
- Job type extracted incorrectly from description ("fast-paced environment" ≠ contract)
- No way to filter out contract roles if user wants only full-time

---

### GAP-26: Job Board API Rate Limit Deep Design (MODERATE)

**Location**: `05-job-discovery-layer.md` - mentions rate limiting but no deep design

**What's missing**:
1. **Per-source rate limit values**: What are actual limits for Greenhouse? Lever? Adzuna?
2. **Rate limit headers**: Do these APIs return rate limit headers? How to parse?
3. **Global rate limiting**: Should LazyJob have a global rate limit across all sources?
4. **Burst handling**: Can requests burst above the per-minute limit if average is within limit?
5. **Rate limit monitoring**: How does user see current rate limit status?

**Why important**: Hitting rate limits causes discovery failures. Understanding limits is critical for reliability.

**What could go wrong**:
- Discovery hits rate limit, fails silently, user thinks discovery is working
- Too aggressive rate limiting, discovery takes hours to complete
- No visibility into rate limit consumption, can't optimize

---

## Cross-Spec Gaps

### Cross-Gap D: Company Name Resolution Fragmentation

The company name resolution problem spans:
- `05-job-discovery-layer.md`: `CompanyConfig` with company names
- `job-search-company-research.md`: `CompanyRecord` with `name_normalized`
- `job-search-ghost-job-detection.md`: Uses company name in repost detection

There's no unified company name matching strategy across the entire system.

**Affected specs**: All that reference company names

### Cross-Gap E: Embedding Model Migration

`job-search-semantic-matching.md` mentions embedding model migration but doesn't design it:
- What happens when user switches from Ollama to OpenAI embeddings?
- How are old embeddings invalidated?
- What's the migration timeline?

**Affected specs**: `job-search-semantic-matching.md`, `05-job-discovery-layer.md`

### Cross-Gap F: Real-Time vs Batch Discovery Tension

The specs describe both real-time discovery (webhooks mentioned as future) and batch discovery (polling). There's no design for how these interact:
- If webhook fires AND polling runs, can duplicate jobs result?
- Should webhooks update immediately or still go through enrichment pipeline?
- What's the consistency model when webhooks + polling both run?

**Affected specs**: All discovery specs

---

## Specs to Create

### Critical Priority

1. **XX-job-alert-webhooks.md** - Real-time job alert webhook receivers, security, retry queue
2. **XX-authenticated-job-sources.md** - LinkedIn/Indeed/Glassdoor authentication via cookies, session management

### Important Priority

3. **XX-company-name-resolution.md** - Unified company name normalization, fuzzy matching, cross-source reconciliation
4. **XX-pay-transparency-jurisdictions-dynamic.md** - Dynamic jurisdiction updates beyond static binary
5. **XX-company-staleness-per-field.md** - Per-field staleness tracking, refresh prioritization
6. **XX-job-notification-system.md** - Alert channels, filtering, batching, quiet hours

### Moderate Priority

7. **XX-semantic-query-expansion.md** - Query expansion for user text searches, synonym handling
8. **XX-discovery-failure-recovery.md** - Circuit breakers, partial failure handling, manual retry
9. **XX-cross-source-priority.md** - Source priority, data quality, conflict resolution
10. **XX-job-type-filtering.md** - Job type, experience level, security clearance filtering
11. **XX-rate-limit-deep-design.md** - Per-source limits, headers, global limiting, monitoring

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-17: Authenticated Job Sources | Critical | High | Coverage majorly limited without |
| GAP-16: Real-Time Webhooks | Critical | High | Competitive disadvantage |
| GAP-18: Company Name Resolution | Important | Medium | Data fragmentation |
| GAP-20: Per-Field Staleness | Important | Medium | Data quality |
| GAP-19: Pay Transparency Dynamic | Important | Low | Legal compliance |
| GAP-21: Notification System | Moderate | Medium | User experience |
| GAP-22: Query Expansion | Moderate | Medium | Search quality |
| GAP-23: Failure Recovery | Moderate | Medium | Reliability |
| GAP-24: Cross-Source Priority | Moderate | Low | UX confusion |
| GAP-25: Job Type Filtering | Moderate | Low | Search quality |
| GAP-26: Rate Limit Deep Design | Moderate | Low | Operational visibility |
