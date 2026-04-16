# Spec: Platform Aggregation & Deduplication

**JTBD**: Access all major job platforms from one tool without context switching
**Topic**: Aggregate job listings from multiple sources into a unified view, deduplicate cross-platform duplicates, and maintain a consistent job identity across sources
**Domain**: platform-integrations

---

## What

A cross-platform aggregation layer that pulls job listings from Greenhouse, Lever, and Adzuna into a unified `DiscoveryService`, deduplicates listings that appear on multiple boards using normalized company + title + location matching, and writes normalized `DiscoveredJob` records to the `jobs` table. The deduplication engine operates in two tiers: exact-match dedup on `(source, source_id)` for same-platform duplicates, and fuzzy dedup on `(company_id, title_normalized, location_normalized)` for cross-platform duplicates.

## Why

Job seekers who use multiple platforms see the same job listed on Greenhouse, LinkedIn, Indeed, and an aggregation site simultaneously. A user who has 5 platform integrations active and searches "Senior Software Engineer" gets 4-8 copies of the same Stripe listing. Without deduplication:
- The job feed is noisy (same listing appearing 5 times)
- Pipeline metrics are inflated (same application counted 3 times)
- User wastes time reviewing duplicates
- Ghost detection signals are diluted across multiple copies

The deduplication spec is a **cross-cutting concern** that every discovery source feeds into. It must be designed once and applied consistently.

## How

### Architecture

```
lazyjob-core/src/discovery/
├── mod.rs                    # DiscoveryService, re-exports
├── aggregation.rs           # Multi-source fetch, merge, deduplicate
├── deduplication.rs         # DedupEngine: exact + fuzzy matching
└── normalizers.rs           # Company name normalization, title normalization
```

### DiscoveryService

```rust
// lazyjob-core/src/discovery/aggregation.rs

pub struct DiscoveryService {
    platform_registry: PlatformRegistry,
    job_repo: Arc<JobRepository>,
    company_repo: Arc<CompanyRepository>,
    dedup_engine: DedupEngine,
}

impl DiscoveryService {
    pub async fn run_discovery(&self, query: &DiscoveryQuery) -> Result<DiscoveryResult> {
        // 1. Fetch from all enabled platforms concurrently
        let jobs = self.fetch_all_sources(query).await?;

        // 2. Normalize company names (resolve Stripe → stripe and stripe.com → Stripe Inc.)
        let normalized = self.normalize_jobs(jobs).await?;

        // 3. Deduplicate
        let (unique, duplicates) = self.dedup_engine.deduplicate(normalized);

        // 4. Enrich with company data
        let enriched = self.enrich_with_companies(unique).await?;

        // 5. Score and rank
        let scored = self.score_jobs(enriched).await?;

        Ok(DiscoveryResult { jobs: scored, duplicates_removed: duplicates.len() })
    }

    async fn fetch_all_sources(&self, query: &DiscoveryQuery) -> Result<Vec<DiscoveredJob>> {
        let mut handles = Vec::new();
        for (platform_name, client) in self.platform_registry.clients() {
            let handle = tokio::spawn(async move {
                client.fetch_jobs(&query.board_token_for(platform_name)).await
            });
            handles.push(handle);
        }
        let mut jobs = Vec::new();
        for handle in handles {
            if let Ok(Ok(Ok(jobs_result))) = handle.await {
                jobs.extend(jobs_result);
            }
        }
        Ok(jobs)
    }
}
```

### Two-Tier Deduplication Engine

```rust
// lazyjob-core/src/discovery/deduplication.rs

pub struct DedupEngine {
    normalizer: TitleNormalizer,
    company_resolver: CompanyResolver,
}

pub enum DedupResult {
    Unique(DiscoveredJob),
    DuplicateOf { canonical_job_id: Uuid, source: &'static str, source_id: &'static str },
}

impl DedupEngine {
    /// Tier 1: Same-source exact dedup — same platform, same source_id
    /// Tier 2: Cross-source fuzzy dedup — normalized (company, title, location)
    pub fn deduplicate(&self, jobs: Vec<DiscoveredJob>) -> (Vec<DiscoveredJob>, Vec<DedupResult>) {
        let mut canonical: HashMap<String, DiscoveredJob> = HashMap::new();
        let mut dupes: Vec<DedupResult> = Vec::new();

        for job in jobs {
            let key = format!("{}:{}", job.source, job.source_id);
            // Tier 1: exact same-source match
            if canonical.contains_key(&key) {
                dupes.push(DedupResult::DuplicateOf {
                    canonical_job_id: canonical[&key].id,
                    source: job.source,
                    source_id: &job.source_id,
                });
                continue;
            }

            // Tier 2: cross-source fuzzy dedup
            let fuzzy_key = self.fuzzy_key(&job);
            if let Some(existing) = self.find_fuzzy_match(&fuzzy_key, &canonical) {
                // Keep the one with better source_quality (api > aggregated > scraped)
                let survivor = self.best_quality(&job, existing);
                dupes.push(DedupResult::DuplicateOf {
                    canonical_job_id: survivor.id,
                    source: job.source,
                    source_id: &job.source_id,
                });
                continue;
            }

            canonical.insert(key, job);
        }

        let unique: Vec<DiscoveredJob> = canonical.into_values().collect();
        (unique, dupes)
    }

    fn fuzzy_key(&self, job: &DiscoveredJob) -> String {
        let company_norm = self.company_resolver.normalize(&job.company_name);
        let title_norm = self.normalizer.normalize(&job.title);
        let location_norm = job.location
            .as_ref()
            .map(|l| self.normalizer.normalize_location(l))
            .unwrap_or_default();
        format!("{}|{}|{}", company_norm, title_norm, location_norm)
    }

    fn find_fuzzy_match(&self, key: &str, canonical: &HashMap<String, DiscoveredJob>) -> Option<DiscoveredJob> {
        canonical.values().find(|existing| self.fuzzy_key(existing) == key).cloned()
    }

    fn best_quality(&self, a: &DiscoveredJob, b: &DiscoveredJob) -> &DiscoveredJob {
        // API sources > aggregated > scraped
        let quality = |j: &DiscoveredJob| -> u8 {
            match j.source_quality.as_deref() {
                Some("api") => 3,
                Some("aggregated") => 2,
                Some("scraped") => 1,
                _ => 0,
            }
        };
        if quality(a) > quality(b) { a } else { b }
    }
}
```

### Normalization Utilities

Company name normalization is critical for cross-source dedup. The same company appears as "Stripe", "stripe", "Stripe Inc.", and "stripe.com" across sources:

```rust
// lazyjob-core/src/discovery/normalizers.rs

pub struct CompanyResolver {
    known_aliases: HashMap<String, String>, // canonical name → normalized key
}

impl CompanyResolver {
    pub fn normalize(&self, name: &str) -> String {
        let lower = name.to_lowercase();
        // Strip common suffixes
        let stripped = lower
            .replace(" inc.", "")
            .replace(" inc", "")
            .replace(" llc", "")
            .replace(" ltd.", "")
            .replace(" ltd", "")
            .replace(" corp.", "")
            .replace(" corp", "")
            .replace(" (company)", "")
            .replace(" pvt.", "")
            .trim()
            .to_string();
        // Check aliases
        self.known_aliases.get(&stripped).cloned().unwrap_or(stripped)
    }
}

pub struct TitleNormalizer {
    stop_words: HashSet<&'static str>,
}

impl TitleNormalizer {
    pub fn normalize(&self, title: &str) -> String {
        let lower = title.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        let filtered: Vec<&str> = words
            .iter()
            .filter(|w| !self.stop_words.contains(w))
            .copied()
            .collect();
        filtered.join(" ")
    }

    pub fn normalize_location(&self, location: &str) -> String {
        location.to_lowercase()
            .replace("remote", "")
            .replace("hybrid", "")
            .replace(", united states", "")
            .trim()
            .to_string()
    }
}
```

### Title Normalization Stop Words

Common words that don't distinguish jobs and must be stripped during fuzzy matching:
- `["senior", "junior", "staff", "principal", "lead", "mid-level", "entry-level", "i", "ii", "iii", "sr.", "jr.", "remote", "hybrid"]`

### Cross-Platform Deduplication Example

```
Job A (Greenhouse): source="greenhouse", source_id="1234", company="Stripe", title="Senior Software Engineer", location="Remote"
Job B (Adzuna):     source="adzuna", source_id="5678", company="stripe.com", title="Senior SWE", location="United States"

fuzzy_key(A) = "stripe|software engineer|"
fuzzy_key(B) = "stripe|software engineer|"

Match! fuzzy_key(B) == fuzzy_key(A)
→ Job B is marked duplicate of Job A (canonical)
→ Job A is retained (Greenhouse API source_quality = "api" > Adzuna aggregated)
```

### Job Source Quality Tracking

```rust
// DiscoveredJob.source_quality field
pub enum SourceQuality {
    Api,        // Direct platform API (Greenhouse, Lever) — highest quality
    Aggregated, // Adzuna, unified APIs — medium quality
    Scraped,    // Apify, JobSpy — lowest quality, needs ghost detection weighting
}
```

### Ralph Loop Integration

The `JobDiscoveryLoop` (from `agentic-ralph-orchestration.md`) calls `DiscoveryService::run_discovery()` on a configurable schedule. The loop writes results directly to the jobs table via `JobRepository::insert_batch()`:

```rust
impl JobDiscoveryLoop {
    pub async fn execute(&self, ctx: &DiscoveryLoopContext) -> Result<LoopOutput> {
        let result = self.discovery_service.run_discovery(&ctx.query).await?;
        for job in &result.jobs {
            self.job_repo.upsert(job).await?;
        }
        Ok(LoopOutput::JobsDiscovered { count: result.jobs.len(), dupes_removed: result.duplicates_removed })
    }
}
```

### Duplicate Tracking for User Feedback

When a duplicate is detected, we track it for analytics:

```sql
CREATE TABLE duplicate_log (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    canonical_job_id TEXT NOT NULL,
    duplicate_source TEXT NOT NULL,
    duplicate_source_id TEXT NOT NULL,
    fuzzy_key TEXT NOT NULL,
    detected_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (canonical_job_id) REFERENCES jobs(id) ON DELETE CASCADE
);
```

This table is append-only (no delete) and used for analytics: "You have 47 duplicates from last week's discovery runs." Not shown in the primary TUI (reduces noise), but accessible via `lazyjob stats --dupes`.

## Open Questions

- **Dedup confidence threshold**: For fuzzy matching, a 3-way match on (company, title, location) is high confidence. But what if location is missing on one source? Should we accept 2-way (company + title) as sufficient? Current spec requires all 3 for a match — revisit if false negatives are reported.
- **Title abbreviation handling**: "SWE" vs "Software Engineer" vs "Software Dev" — current stop-word list handles common seniority prefixes but doesn't handle domain abbreviations. Phase 2 could add LLM-based title canonicalization.
- **Known aliases init**: The `known_aliases` map is empty at init. It should be populated from existing company data in `companies` table on startup, so that previous imports set the canonical form. Phase 2: persist aliases to `company_aliases` table.

## Implementation Tasks

- [ ] Implement `DedupEngine::deduplicate()` in `lazyjob-core/src/discovery/deduplication.rs` with two-tier dedup (exact on source+source_id, fuzzy on normalized key)
- [ ] Implement `CompanyResolver::normalize()` in `lazyjob-core/src/discovery/normalizers.rs` with alias map and common suffix stripping
- [ ] Implement `TitleNormalizer::normalize()` with stop-word list, and `normalize_location()` for fuzzy location matching
- [ ] Build `DiscoveryService::run_discovery()` in `lazyjob-core/src/discovery/aggregation.rs` fetching from all enabled platforms concurrently, normalizing, deduplicating, and enriching
- [ ] Add `source_quality` field to `jobs` table DDL: `TEXT DEFAULT 'api' CHECK(source_quality IN ('api', 'aggregated', 'scraped'))`
- [ ] Add `JobRepository::insert_batch()` method for efficient bulk inserts during Ralph discovery loops
- [ ] Create `duplicate_log` table DDL, track dedup events during `DiscoveryService::run_discovery()`, add `lazyjob stats --dupes` CLI command
