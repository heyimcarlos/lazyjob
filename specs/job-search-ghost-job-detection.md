# Spec: Ghost Job Detection

**JTBD**: Find relevant job opportunities without wasting time on ghost jobs or mismatched roles
**Topic**: Classify each discovered job listing as likely-real or likely-ghost using heuristic signals stored locally
**Domain**: job-search

---

## What

The Ghost Job Detection subsystem assigns a `ghost_score` (0.0 = almost certainly real, 1.0 = almost certainly ghost) to each discovered job. It uses a rule-based heuristic classifier operating entirely on locally-available data — no external API calls required. The score is consumed by `FeedRanker` to suppress ghost listings from the top of the feed and by the TUI to display a warning badge. Users can override any classification.

## Why

27–30% of all online job listings are ghost jobs — postings with no genuine intent to hire (ResumeUp.AI 2025 analysis; Fonzi AI 2026). 93% of HR professionals admit to posting them. This means ~1 in 3 applications sent by LazyJob users goes into a permanent black hole even before recruiter bias enters the picture. Ghost jobs are the single highest-impact quality problem in job discovery. Detecting and suppressing them proactively protects user time and mental health. Even a rough classifier that catches 60% of ghosts while falsely flagging <10% of real jobs is a significant win.

**Design constraint**: Ghost detection must never silently hide a job. It suppresses listing visibility in the ranked feed but must always be overridable. The user sees a badge, not a deletion.

## How

### Heuristic signal set

Each signal produces a partial score contribution. The final `ghost_score` is a weighted sum, clamped to [0.0, 1.0].

| Signal | Weight | Description |
|---|---|---|
| `days_since_posted` | 0.20 | > 60 days → high ghost probability |
| `days_since_updated` | 0.15 | Never updated since first posted → suspicious |
| `repost_count` | 0.20 | Same (company, title, location) posted > 2 times → likely ghost |
| `description_vagueness` | 0.15 | Short description with no tech stack, no team context, no specific requirements |
| `salary_absent_in_transparency_state` | 0.10 | No salary in a jurisdiction with pay-transparency law → suspicious |
| `no_named_contact` | 0.05 | No recruiter name, no team name, no hiring manager mention |
| `company_headcount_declining` | 0.10 | Company has recent layoffs in layoffs_db (if available) while posting aggressively |
| `posting_age_repost_ratio` | 0.05 | Posted many times over a long window without closing → classic ghost signal |

**Total weight: 1.00**

Threshold rules:
- `ghost_score < 0.3` → show normally
- `0.3 ≤ ghost_score < 0.6` → show with "possibly stale" badge
- `ghost_score ≥ 0.6` → show with "likely ghost" badge; deprioritized in feed
- User can mark any job `ghost_overridden = true` to force it back to the top

### Signal computation details

**days_since_posted scoring** (logarithmic):
```
score = min(1.0, log(days + 1) / log(61))
```
- 0 days → 0.0, 30 days → 0.75, 60 days → 1.0, 90+ days → clamped at 1.0

**description_vagueness score**:
- Count of "required skill tokens" (programming languages, tools, frameworks detected by regex lexicon)
- Count of "specificity markers" (team name, project name, tech stack mentions, "we use X")
- `vagueness = 1.0 - min(1.0, (skill_tokens + specificity_markers) / 8.0)`
- A JD with 8+ specific technical terms scores near 0; a JD saying "competitive compensation, growth mindset, passionate about innovation" with no tools scores near 1.0

**repost_count**: Track `(company_name_normalized, title_normalized, location_normalized)` composite key in a `job_postings_history` table. Increment counter on each discovery that matches an existing record. Ghost threshold: > 2 reposts of identical listing.

**salary_absent_in_transparency_state**: Maintain a static list of jurisdictions with active pay-transparency laws (as of 2026: California, New York, Colorado, Washington, Illinois, New Jersey; UK; EU member states for roles > 5 employees). If job location matches and no salary is present: score = 1.0.

**company_headcount_declining**: If `layoffs_fyi` data is loaded as a local SQLite table (optional feature, populated by a separate ralph loop), check if the company had layoffs in the last 90 days. A company posting 10+ listings while having recent layoffs scores high.

### Data sources required

1. **Local jobs table** — `posted_at`, `updated_at`, `description`, `salary_min`, `salary_max`, `location` (all already populated by discovery engine)
2. **job_postings_history table** — new table tracking composite deduplication key + repost count
3. **pay_transparency_jurisdictions** — static embedded data, updated with binary releases
4. **layoffs_db** — optional, loaded from a periodic ralph loop scraping layoffs.fyi (Phase 2)

### Classification update schedule

Ghost scores are recomputed:
1. At discovery time (initial score from available signals)
2. Daily re-run for all jobs in `status != Dismissed` and `ghost_score != null` to catch listings that have aged into ghost territory
3. Immediately when user manually overrides via TUI

## Interface

```rust
// lazyjob-core/src/discovery/ghost_detection.rs

pub struct GhostDetector {
    transparency_jurisdictions: HashSet<String>, // loaded at startup from embedded data
    layoffs_repo: Option<Arc<dyn LayoffsRepository>>, // None if not configured
}

impl GhostDetector {
    pub fn new() -> Self;
    pub fn with_layoffs_db(mut self, repo: Arc<dyn LayoffsRepository>) -> Self;

    /// Compute ghost_score ∈ [0.0, 1.0] for a single job
    pub fn score(&self, job: &DiscoveredJob, history: &PostingHistory) -> f32;

    /// Score all unscored jobs in batch
    pub async fn score_batch(
        &self,
        jobs: &[DiscoveredJob],
        repo: &dyn JobRepository,
    ) -> Result<Vec<(Uuid, f32)>>;
}

pub struct PostingHistory {
    pub repost_count: u32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub update_count: u32,
}

pub struct GhostSignals {
    pub days_since_posted: f32,
    pub days_since_updated: f32,
    pub repost_count: u32,
    pub description_vagueness: f32,
    pub salary_absent_in_transparency_state: bool,
    pub no_named_contact: bool,
    pub company_headcount_declining: bool,
}

impl GhostSignals {
    pub fn compute_score(&self) -> f32;
    pub fn explain(&self) -> Vec<String>; // human-readable reasons for TUI tooltip
}

// TUI badge type
pub enum GhostBadge {
    None,
    PossiblyStalе,    // 0.3–0.59
    LikelyGhost,     // 0.60+
    UserOverridden,  // user marked as real
}

impl GhostBadge {
    pub fn from_score(score: f32, overridden: bool) -> Self;
}
```

## Open Questions

- **False positive rate target**: What's acceptable? 5% false positive rate means 1 in 20 real jobs gets a ghost badge — probably acceptable. 15% would lose user trust. Need a labeled dataset of confirmed ghost vs. real jobs from user feedback to calibrate weights.
- **layoffs.fyi integration**: Scraping layoffs.fyi is a ToS gray area. Should this signal be entirely optional (user enables with a warning), always-off by default, or dropped in favor of simpler signals?
- **Salary transparency jurisdictions**: The static list will drift. Should it live in a user-updatable config file, be embedded in the binary, or be fetched from a LazyJob-hosted static JSON? Embedded in binary with periodic releases is simplest.
- **Ghost job notification**: When a job the user has already applied to gets classified as a ghost (posts age > 60 days after application), should LazyJob warn them? This could be anxiety-inducing. Opt-in notification vs. a passive badge on the application record?
- **Repost detection granularity**: "Same job" detection by `(company, title, location)` may be too coarse — companies legitimately open the same role in multiple locations. Should location be excluded from the composite key, or should location variation count as a legitimate distinct posting?

## Implementation Tasks

- [ ] Create `job_postings_history` table tracking `(company_name_normalized, title_normalized, location_normalized, first_seen_at, last_seen_at, repost_count)` — refs: `04-sqlite-persistence.md`
- [ ] Implement `GhostDetector::score()` with the 7-signal weighted heuristic and `GhostSignals::explain()` for TUI tooltip — refs: `agentic-job-matching.md`, `job-platforms-comparison.md`
- [ ] Embed `pay_transparency_jurisdictions` as a static `HashSet<&str>` in `lazyjob-core/src/discovery/ghost_detection.rs` — refs: `job-platforms-comparison.md`
- [ ] Add `ghost_score REAL` and `ghost_overridden BOOLEAN` columns to `jobs` table migration — refs: `04-sqlite-persistence.md`
- [ ] Implement `description_vagueness` scorer using a regex-based technical term lexicon (programming languages, tools, frameworks) — refs: `agentic-job-matching.md`
- [ ] Integrate `GhostDetector::score_batch()` into the ralph discovery loop (runs after enrichment, before final SQLite write) — refs: `06-ralph-loop-integration.md`
- [ ] Add `GhostBadge` display to the job feed TUI widget with tooltip showing `explain()` reasons — refs: `09-tui-design-keybindings.md`
