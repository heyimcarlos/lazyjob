# Implementation Plan: Profile Skills Gap Analysis

## Status
Draft

## Related Spec
[specs/profile-skills-gap-analysis.md](./profile-skills-gap-analysis.md)

## Overview

The skills gap analysis module answers three questions: (1) Which skills appear in target job descriptions that I don't currently have? (2) Which missing skills are genuine blockers vs. noise? (3) What should I learn first? It operates by extracting skills from both the user's LifeSheet and a corpus of target job descriptions, computing a frequency-weighted gap matrix, scoring each gap by a priority formula, and surfacing results as a heat-map widget in the TUI.

The module is architecturally positioned between the LifeSheet layer and the resume tailoring pipeline. It reads from `lazyjob-core`'s LifeSheet and job repositories, produces a `GapReport` persisted in a `gap_analysis_cache` SQLite table, and exposes both a programmatic service API and a TUI view. Phase 2 adds ESCO alias expansion (parent/sibling skill inference) and embedding-based semantic matching for career transitioners.

A key differentiator of this module is first-class support for career transitioners — people whose prior experience vocabularies diverge from target-role terminology. The LLM-powered transferable skill finder bridges that vocabulary gap (e.g., "led platoon of 40 soldiers" → "team management") without auto-adding anything to the LifeSheet or resume; all suggestions are shown to the user for deliberate review.

## Prerequisites

### Specs/Plans that must precede this
- `specs/profile-life-sheet-data-model-implementation-plan.md` — provides `LifeSheet`, `LifeSheetRepository`, `is_grounded_claim()`, `SkillEntry` with ESCO codes
- `specs/04-sqlite-persistence-implementation-plan.md` — provides `Database`, migration runner, `sqlx::Pool<Sqlite>`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — provides `Arc<dyn LlmProvider>`, `ChatMessage`
- `specs/job-search-discovery-engine-implementation-plan.md` — provides `JobRepository`, `Job`, `JobId`
- `specs/09-tui-design-keybindings-implementation-plan.md` — provides `App`, `EventLoop`, panel focus/keybinding system

### Crates to add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml
[dependencies]
strsim             = "0.11"           # Jaro-Winkler fuzzy skill matching
regex              = "1"
once_cell          = "1"              # Lazy<Regex> for compiled lexicon patterns
sha2               = "0.10"           # Cache key: hash of input job IDs + lifesheet version
serde_yaml         = "0.9"            # Learning resources YAML asset deserialization
ammonia            = "3"              # Strip HTML from raw JD text before lexicon pass

# Already present from prior plans:
sqlx               = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros", "chrono", "uuid"] }
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
chrono             = { version = "0.4", features = ["serde"] }
uuid               = { version = "1", features = ["v4", "serde"] }
tokio              = { version = "1", features = ["full"] }
thiserror          = "2"
anyhow             = "1"
tracing            = "0.1"
async-trait        = "0.1"
```

---

## Architecture

### Crate Placement

| Crate | Responsibility |
|-------|----------------|
| `lazyjob-core` | All gap analysis domain logic: `SkillNormalizer`, `UserSkillExtractor`, `JdSkillExtractor`, `GapMatrix`, `GapAnalysisService`, `TransferableSkillFinder`, `GapAnalysisRepository`, SQLite migrations |
| `lazyjob-llm` | Prompt template for transferable skill analysis (`LoopType::TransferableSkillFinder`) |
| `lazyjob-tui` | `GapAnalysisView`, `SkillHeatMapWidget`, `GapDetailPanel`, `LearningResourceWidget` |
| `lazyjob-cli` | `lazyjob gap analyze [--jobs <ids>] [--career-transition]` subcommand |

`lazyjob-core` must have zero dependency on `lazyjob-tui`. Domain types flow outward.

### Core Types

```rust
// lazyjob-core/src/gap_analysis/types.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Normalized canonical representation of a skill name.
/// Created only via `SkillNormalizer::normalize()` — parse, don't validate (§2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalSkill(String);

impl CanonicalSkill {
    /// The normalized string value.
    pub fn as_str(&self) -> &str { &self.0 }
}

/// Where a skill token was sourced from in the LifeSheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSource {
    ExplicitSkillTable,
    TechStackJson { experience_id: Uuid },
    RegexLexicon { experience_id: Uuid },
    Achievement { achievement_id: Uuid },
}

/// A user skill extracted and normalized from the LifeSheet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToken {
    pub canonical: CanonicalSkill,
    pub raw_name: String,
    pub esco_code: Option<String>,
    pub source: SkillSource,
}

/// Severity classification for a skill gap.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapSeverity {
    Minor,       // <20% of target JDs
    Significant, // 20–50%
    Critical,    // >50% AND in "required" section
}

/// A single skill gap entry in a computed GapReport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGap {
    pub skill_name: String,                // display name (un-normalized)
    pub canonical: CanonicalSkill,
    pub esco_code: Option<String>,
    pub frequency: f32,                    // fraction of target JDs requiring this skill
    pub required_in_jds: usize,            // count of JDs listing it as "required"
    pub preferred_in_jds: usize,           // count listing it as "preferred"/"nice-to-have"
    pub gap_severity: GapSeverity,
    pub priority_score: f32,               // computed by PriorityScorer
    pub present_in_jobs: Vec<Uuid>,        // job IDs that list this skill
    pub learning_resource: Option<LearningResource>,
}

/// A bridged transferable skill (LLM-generated, shown for user review only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferableSkill {
    pub source_experience: String,    // excerpt from work_experience.summary
    pub target_skill: String,         // target role skill vocabulary
    pub bridge_explanation: String,   // LLM explanation, shown to user, never auto-applied
    pub confidence: f32,              // 0.0–1.0 from LLM structured output
}

/// Full computed gap analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    pub id: Uuid,
    pub user_skills: Vec<SkillToken>,
    pub skill_gaps: Vec<SkillGap>,                   // sorted by priority_score desc
    pub upskilling_priorities: Vec<SkillGap>,        // top 5 from skill_gaps
    pub transferable_skills: Vec<TransferableSkill>, // empty if career_transition=false
    pub match_coverage: f32,                         // % of required skills covered
    pub corpus_size: usize,                          // number of JDs analyzed
    pub computed_at: DateTime<Utc>,
    pub ttl_hours: u32,                              // default 24
}

/// Options passed to GapAnalysisService::analyze().
#[derive(Debug, Clone)]
pub struct GapAnalysisOptions {
    pub min_frequency: f32,           // default 0.2 (20%)
    pub include_nice_to_have: bool,   // default false (required skills only for severity)
    pub use_esco_expansion: bool,     // Phase 2 flag, default false
    pub max_gaps_to_surface: usize,   // default 20
    pub career_transition: bool,      // if true, run TransferableSkillFinder
    pub target_role: Option<String>,  // used when career_transition=true
}

impl Default for GapAnalysisOptions {
    fn default() -> Self {
        Self {
            min_frequency: 0.2,
            include_nice_to_have: false,
            use_esco_expansion: false,
            max_gaps_to_surface: 20,
            career_transition: false,
            target_role: None,
        }
    }
}

/// Static learning resource for a skill gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningResource {
    pub skill: String,
    pub title: String,
    pub url: String,
    pub resource_type: ResourceType,
    pub is_free: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceType {
    Book, Course, Documentation, Tutorial, Video,
}
```

### Trait Definitions

```rust
// lazyjob-core/src/gap_analysis/mod.rs

use async_trait::async_trait;

/// Stores and retrieves cached GapReports keyed by content hash.
#[async_trait]
pub trait GapAnalysisRepository: Send + Sync {
    async fn get_cached(
        &self,
        cache_key: &str,
    ) -> Result<Option<GapReport>, GapAnalysisError>;

    async fn save(
        &self,
        cache_key: &str,
        report: &GapReport,
    ) -> Result<(), GapAnalysisError>;

    async fn delete_expired(&self) -> Result<u64, GapAnalysisError>;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/007_gap_analysis.sql

CREATE TABLE IF NOT EXISTS gap_analysis_cache (
    id           TEXT NOT NULL PRIMARY KEY,    -- UUID
    cache_key    TEXT NOT NULL UNIQUE,          -- SHA-256 of (sorted job IDs + life_sheet_version_hash)
    report_json  TEXT NOT NULL,                 -- serde_json serialized GapReport
    computed_at  TEXT NOT NULL,                 -- ISO-8601 UTC
    ttl_hours    INTEGER NOT NULL DEFAULT 24,
    expires_at   TEXT NOT NULL                  -- computed_at + ttl_hours
);

CREATE INDEX idx_gap_cache_expires ON gap_analysis_cache (expires_at);

-- Invalidation trigger: touch life_sheet_meta.updated_at whenever any
-- life_sheet_* table row changes (existing from life-sheet-data-model plan).
-- GapAnalysisService checks if life_sheet_meta.version_hash != the hash
-- embedded in the cached report before serving from cache.
```

### Module Structure

```
lazyjob-core/
  src/
    gap_analysis/
      mod.rs          ← re-exports GapAnalysisService, GapReport, SkillGap, etc.
      types.rs        ← all domain types defined above
      normalizer.rs   ← SkillNormalizer: lowercase/strip/alias lookup
      extractor.rs    ← UserSkillExtractor: reads LifeSheet → Vec<SkillToken>
      jd_extractor.rs ← JdSkillExtractor: lexicon pass over raw JD text
      matrix.rs       ← GapMatrix::compute() → Vec<SkillGap>
      scorer.rs       ← PriorityScorer: priority_score formula
      transfer.rs     ← TransferableSkillFinder: LLM prompt + structured parse
      service.rs      ← GapAnalysisService: orchestrates all steps + cache
      repository.rs   ← SqliteGapAnalysisRepository
      learning.rs     ← LearningResourceIndex: load YAML asset at startup
    lexicon/
      mod.rs
      tech_terms.rs   ← TechTermLexicon: compiled Regex for 500+ tech terms
      skill_aliases.rs← embedded skill_aliases YAML, lazily loaded
  assets/
    tech_terms.txt      ← one canonical term per line, seeded from ESCO subset
    skill_aliases.yaml  ← canonical_name → [aliases...] map
    learning_resources.yaml ← skill → [LearningResource...] map

lazyjob-tui/
  src/
    views/
      gap_analysis.rs ← GapAnalysisView (full screen)
    widgets/
      skill_heat_map.rs  ← SkillHeatMapWidget (ratatui custom widget)
      gap_detail.rs      ← GapDetailPanel (right-pane drill-down)
      learning_list.rs   ← LearningResourceWidget (inline list in detail panel)
```

---

## Implementation Phases

### Phase 1 — Core Gap Computation (MVP)

**Step 1.1 — SkillNormalizer** (`lazyjob-core/src/gap_analysis/normalizer.rs`)

```rust
use once_cell::sync::Lazy;
use std::collections::HashMap;

static ALIAS_MAP: Lazy<HashMap<String, String>> = Lazy::new(|| {
    let raw: HashMap<String, Vec<String>> =
        serde_yaml::from_str(include_str!("../../assets/skill_aliases.yaml"))
            .expect("skill_aliases.yaml is bundled at compile time");
    let mut map = HashMap::new();
    for (canonical, aliases) in raw {
        for alias in aliases {
            map.insert(normalize_raw(&alias), canonical.clone());
        }
    }
    map
});

/// Strip punctuation, lowercase, collapse whitespace, apply alias table.
/// Returns `CanonicalSkill` — the only constructor for that type.
pub fn normalize(raw: &str) -> CanonicalSkill {
    let normalized = normalize_raw(raw);
    let canonical = ALIAS_MAP
        .get(&normalized)
        .cloned()
        .unwrap_or_else(|| normalized.clone());
    CanonicalSkill(canonical)
}

fn normalize_raw(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
```

`skill_aliases.yaml` initial seed: ~500 common tech aliases covering "Node.js"/"nodejs"/"node js", "ML"/"machine learning"/"ml", "k8s"/"kubernetes", "cpp"/"c++", etc.

- **Verification**: `cargo test` on `normalizer::tests` — `normalize("Node.js")` == `normalize("NodeJS")` == `CanonicalSkill("node js")`; `normalize("ML")` == `normalize("machine learning")`.

**Step 1.2 — TechTermLexicon** (`lazyjob-core/src/lexicon/tech_terms.rs`)

The lexicon regex scans free-form text for 500+ canonical skill names. Built once per process via `once_cell::sync::Lazy`.

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static TECH_TERMS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    // Load term list from bundled asset at compile time.
    let terms_txt = include_str!("../../assets/tech_terms.txt");
    // Build alternation: longest terms first to prevent partial matches.
    let mut terms: Vec<&str> = terms_txt.lines().filter(|l| !l.trim().is_empty()).collect();
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let pattern = terms
        .iter()
        .map(|t| regex::escape(t))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r"(?i)\b(?:{})\b", pattern))
        .expect("tech_terms.txt is bundled at compile time")
});

/// Extract all tech term occurrences from text, returns unique normalized names.
pub fn extract_skills_from_text(text: &str) -> Vec<CanonicalSkill> {
    let mut seen = std::collections::HashSet::new();
    TECH_TERMS_PATTERN
        .find_iter(text)
        .map(|m| crate::gap_analysis::normalizer::normalize(m.as_str()))
        .filter(|s| seen.insert(s.clone()))
        .collect()
}
```

- **Verification**: `extract_skills_from_text("Experience with Python, React, and some Kubernetes")` returns a vec containing `CanonicalSkill("python")`, `CanonicalSkill("react")`, `CanonicalSkill("kubernetes")`.

**Step 1.3 — UserSkillExtractor** (`lazyjob-core/src/gap_analysis/extractor.rs`)

Pulls skills from the LifeSheet via three passes:

```rust
pub struct UserSkillExtractor {
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
}

impl UserSkillExtractor {
    pub async fn extract(&self, life_sheet: &LifeSheet) -> Vec<SkillToken> {
        let mut tokens: Vec<SkillToken> = Vec::new();

        // Pass 1: explicit skill table entries
        for skill in &life_sheet.skills {
            tokens.push(SkillToken {
                canonical: normalizer::normalize(&skill.name),
                raw_name: skill.name.clone(),
                esco_code: skill.esco_code.clone(),
                source: SkillSource::ExplicitSkillTable,
            });
        }

        // Pass 2: tech_stack JSON arrays on work experiences
        for exp in &life_sheet.work_experience {
            for tech in &exp.tech_stack {
                tokens.push(SkillToken {
                    canonical: normalizer::normalize(tech),
                    raw_name: tech.clone(),
                    esco_code: None,
                    source: SkillSource::TechStackJson { experience_id: exp.id },
                });
            }
        }

        // Pass 3: regex lexicon over experience summaries and achievement text
        for exp in &life_sheet.work_experience {
            let combined = format!("{} {}", exp.summary.as_deref().unwrap_or(""),
                exp.achievements.iter().map(|a| a.description.as_str()).collect::<Vec<_>>().join(" "));
            let stripped = ammonia::Builder::default().clean(&combined).to_string();
            for canonical in extract_skills_from_text(&stripped) {
                tokens.push(SkillToken {
                    raw_name: canonical.as_str().to_string(),
                    canonical,
                    esco_code: None,
                    source: SkillSource::RegexLexicon { experience_id: exp.id },
                });
            }
        }

        // Deduplicate by canonical skill (keep first occurrence, prefer ExplicitSkillTable source)
        let mut seen: HashSet<CanonicalSkill> = HashSet::new();
        tokens.retain(|t| {
            if seen.contains(&t.canonical) { false } else { seen.insert(t.canonical.clone()); true }
        });
        // Re-sort: ExplicitSkillTable first so dedup retains highest-quality source
        tokens.sort_by_key(|t| match &t.source { SkillSource::ExplicitSkillTable => 0, SkillSource::TechStackJson {..} => 1, _ => 2 });

        tokens
    }
}
```

- **Verification**: Unit test with a mock `LifeSheet` containing one explicit skill "Python", one tech_stack entry "Django", and a summary containing "built CI/CD pipelines" → tokens include `CanonicalSkill("python")`, `CanonicalSkill("django")`, `CanonicalSkill("cicd")`.

**Step 1.4 — JdSkillExtractor** (`lazyjob-core/src/gap_analysis/jd_extractor.rs`)

Extracts skills from a job description, also detecting whether each skill is in a "required" vs. "preferred" context.

```rust
/// Section classification of where a skill was found in the JD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementSection { Required, Preferred, Unknown }

pub struct JdSkillEntry {
    pub canonical: CanonicalSkill,
    pub raw_name: String,
    pub section: RequirementSection,
}

static REQUIRED_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)^\s*(required|must[- ]have|you must|you need|required qualifications?)\s*:?\s*$").unwrap()
});
static PREFERRED_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)^\s*(preferred|nice[- ]to[- ]have|bonus|plus|desirable)\s*:?\s*$").unwrap()
});

pub fn extract_from_jd(raw_jd: &str) -> Vec<JdSkillEntry> {
    let clean = ammonia::Builder::default()
        .allowed_tag_attributes(std::collections::HashMap::new())
        .tags(std::collections::HashSet::new())
        .clean(raw_jd)
        .to_string();

    // Split into sections by scanning for required/preferred headers.
    // Default section is Unknown until a header is found.
    let mut current_section = RequirementSection::Unknown;
    let mut entries: Vec<JdSkillEntry> = Vec::new();

    for line in clean.lines() {
        if REQUIRED_HEADER.is_match(line) {
            current_section = RequirementSection::Required;
            continue;
        }
        if PREFERRED_HEADER.is_match(line) {
            current_section = RequirementSection::Preferred;
            continue;
        }
        for canonical in extract_skills_from_text(line) {
            entries.push(JdSkillEntry {
                raw_name: canonical.as_str().to_string(),
                canonical,
                section: current_section.clone(),
            });
        }
    }

    // Dedup: if a skill appears in both Required and Unknown, keep Required.
    deduplicate_skill_entries(entries)
}

fn deduplicate_skill_entries(mut entries: Vec<JdSkillEntry>) -> Vec<JdSkillEntry> {
    use std::collections::HashMap;
    let mut best: HashMap<CanonicalSkill, JdSkillEntry> = HashMap::new();
    for entry in entries.drain(..) {
        best.entry(entry.canonical.clone())
            .and_modify(|existing| {
                if entry.section == RequirementSection::Required {
                    existing.section = RequirementSection::Required;
                }
            })
            .or_insert(entry);
    }
    best.into_values().collect()
}
```

- **Verification**: Test with a JD containing "**Required:** Rust, Kubernetes\n**Nice to have:** Terraform" → Rust and Kubernetes have `section: Required`, Terraform has `section: Preferred`.

**Step 1.5 — GapMatrix** (`lazyjob-core/src/gap_analysis/matrix.rs`)

Computes frequency-weighted gap scores across the JD corpus.

```rust
pub struct GapMatrix;

impl GapMatrix {
    /// Compute gaps given user skills and a vec of per-JD extracted skills.
    pub fn compute(
        user_skills: &HashSet<CanonicalSkill>,
        jd_skill_sets: &[(Uuid, Vec<JdSkillEntry>)], // (job_id, skills)
        options: &GapAnalysisOptions,
        user_goals_text: &str,
    ) -> Vec<SkillGap> {
        let total_jds = jd_skill_sets.len() as f32;
        if total_jds == 0.0 {
            return vec![];
        }

        // Aggregate: skill → (required_count, preferred_count, job_ids)
        let mut agg: HashMap<CanonicalSkill, SkillAgg> = HashMap::new();
        for (job_id, skills) in jd_skill_sets {
            for entry in skills {
                let e = agg.entry(entry.canonical.clone()).or_insert_with(|| SkillAgg {
                    canonical: entry.canonical.clone(),
                    raw_name: entry.raw_name.clone(),
                    required_count: 0,
                    preferred_count: 0,
                    job_ids: vec![],
                });
                match entry.section {
                    RequirementSection::Required | RequirementSection::Unknown => e.required_count += 1,
                    RequirementSection::Preferred => e.preferred_count += 1,
                }
                e.job_ids.push(*job_id);
            }
        }

        let goals_skills: HashSet<CanonicalSkill> = extract_skills_from_text(user_goals_text)
            .into_iter()
            .collect();

        let learning_index = LearningResourceIndex::global();

        let mut gaps: Vec<SkillGap> = agg
            .into_values()
            .filter(|a| {
                // Not already in user skills
                !user_skills.contains(&a.canonical)
            })
            .filter_map(|a| {
                let frequency = (a.required_count + a.preferred_count) as f32 / total_jds;
                if frequency < options.min_frequency && !options.include_nice_to_have {
                    return None;
                }
                if a.required_count == 0 && !options.include_nice_to_have {
                    return None;
                }
                let gap_severity = GapSeverity::classify(frequency, a.required_count, total_jds as usize);
                let priority_score = PriorityScorer::score(
                    frequency,
                    a.required_count,
                    a.preferred_count,
                    &a.canonical,
                    &goals_skills,
                );
                Some(SkillGap {
                    skill_name: a.raw_name.clone(),
                    canonical: a.canonical.clone(),
                    esco_code: None, // Phase 2: look up from ESCO index
                    frequency,
                    required_in_jds: a.required_count,
                    preferred_in_jds: a.preferred_count,
                    gap_severity,
                    priority_score,
                    present_in_jobs: a.job_ids,
                    learning_resource: learning_index.lookup(&a.canonical),
                })
            })
            .collect();

        gaps.sort_by(|a, b| b.priority_score.partial_cmp(&a.priority_score).unwrap_or(Equal));
        gaps.truncate(options.max_gaps_to_surface);
        gaps
    }
}

impl GapSeverity {
    fn classify(frequency: f32, required_count: usize, total_jds: usize) -> Self {
        if frequency > 0.5 && required_count > total_jds / 2 {
            GapSeverity::Critical
        } else if frequency >= 0.2 {
            GapSeverity::Significant
        } else {
            GapSeverity::Minor
        }
    }
}
```

**Step 1.6 — PriorityScorer** (`lazyjob-core/src/gap_analysis/scorer.rs`)

```rust
/// priority_score = frequency_weight * required_multiplier * career_relevance
///
/// frequency_weight: 0.0–1.0 fraction of target JDs requiring the skill
/// required_multiplier: 2.0 if required_count > preferred_count, else 1.0
/// career_relevance: 1.5 if skill appears in user's goals text, else 1.0
pub struct PriorityScorer;

impl PriorityScorer {
    pub fn score(
        frequency: f32,
        required_count: usize,
        preferred_count: usize,
        canonical: &CanonicalSkill,
        goals_skills: &HashSet<CanonicalSkill>,
    ) -> f32 {
        let required_multiplier = if required_count > preferred_count { 2.0_f32 } else { 1.0 };
        let career_relevance = if goals_skills.contains(canonical) { 1.5_f32 } else { 1.0 };
        frequency * required_multiplier * career_relevance
    }
}
```

**Step 1.7 — LearningResourceIndex** (`lazyjob-core/src/gap_analysis/learning.rs`)

Static resource index loaded once from bundled YAML asset.

```rust
use once_cell::sync::Lazy;
use std::collections::HashMap;

static LEARNING_INDEX: Lazy<LearningResourceIndex> = Lazy::new(|| {
    let raw = include_str!("../../assets/learning_resources.yaml");
    let map: HashMap<String, Vec<LearningResourceRaw>> =
        serde_yaml::from_str(raw).expect("learning_resources.yaml is bundled");
    LearningResourceIndex { map }
});

pub struct LearningResourceIndex {
    map: HashMap<String, Vec<LearningResourceRaw>>,
}

impl LearningResourceIndex {
    pub fn global() -> &'static Self { &LEARNING_INDEX }

    pub fn lookup(&self, canonical: &CanonicalSkill) -> Option<LearningResource> {
        self.map.get(canonical.as_str())
            .and_then(|resources| resources.first())
            .map(|r| LearningResource {
                skill: canonical.as_str().to_string(),
                title: r.title.clone(),
                url: r.url.clone(),
                resource_type: r.resource_type.clone(),
                is_free: r.is_free,
            })
    }
}

#[derive(Deserialize)]
struct LearningResourceRaw {
    title: String,
    url: String,
    resource_type: ResourceType,
    is_free: bool,
}
```

Initial `learning_resources.yaml` seeds ~50 skills with free resources (Rust → The Rust Book, Kubernetes → k8s.io/docs, React → react.dev, etc.).

**Step 1.8 — SqliteGapAnalysisRepository** (`lazyjob-core/src/gap_analysis/repository.rs`)

```rust
pub struct SqliteGapAnalysisRepository {
    pool: sqlx::Pool<sqlx::Sqlite>,
}

#[async_trait]
impl GapAnalysisRepository for SqliteGapAnalysisRepository {
    async fn get_cached(&self, cache_key: &str) -> Result<Option<GapReport>, GapAnalysisError> {
        let row = sqlx::query!(
            r#"
            SELECT report_json FROM gap_analysis_cache
            WHERE cache_key = ?1
              AND expires_at > datetime('now')
            "#,
            cache_key,
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => {
                let report: GapReport = serde_json::from_str(&r.report_json)
                    .map_err(GapAnalysisError::DeserializationFailed)?;
                Ok(Some(report))
            }
        }
    }

    async fn save(&self, cache_key: &str, report: &GapReport) -> Result<(), GapAnalysisError> {
        let json = serde_json::to_string(report)?;
        let expires_at = report.computed_at
            + chrono::Duration::hours(report.ttl_hours as i64);
        sqlx::query!(
            r#"
            INSERT INTO gap_analysis_cache (id, cache_key, report_json, computed_at, ttl_hours, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(cache_key) DO UPDATE SET
                report_json = excluded.report_json,
                computed_at = excluded.computed_at,
                expires_at = excluded.expires_at
            "#,
            report.id,
            cache_key,
            json,
            report.computed_at,
            report.ttl_hours,
            expires_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_expired(&self) -> Result<u64, GapAnalysisError> {
        let result = sqlx::query!(
            "DELETE FROM gap_analysis_cache WHERE expires_at <= datetime('now')"
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
```

**Step 1.9 — GapAnalysisService** (`lazyjob-core/src/gap_analysis/service.rs`)

```rust
pub struct GapAnalysisService {
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub gap_repo: Arc<dyn GapAnalysisRepository>,
    pub llm: Option<Arc<dyn LlmProvider>>,
}

impl GapAnalysisService {
    /// Run full gap analysis against the specified job IDs.
    /// If `target_job_ids` is empty, uses all starred/saved jobs.
    #[tracing::instrument(skip(self), fields(job_count = target_job_ids.len()))]
    pub async fn analyze(
        &self,
        target_job_ids: &[Uuid],
        options: GapAnalysisOptions,
    ) -> Result<GapReport, GapAnalysisError> {
        // 1. Resolve job IDs
        let job_ids = if target_job_ids.is_empty() {
            self.job_repo.list_starred().await?.iter().map(|j| j.id).collect::<Vec<_>>()
        } else {
            target_job_ids.to_vec()
        };

        if job_ids.len() < 5 {
            tracing::warn!("corpus too small: {} JDs (recommend 10+)", job_ids.len());
        }

        // 2. Compute cache key
        let mut sorted_ids = job_ids.clone();
        sorted_ids.sort();
        let life_sheet = self.life_sheet_repo.load().await?;
        let cache_key = compute_cache_key(&sorted_ids, &life_sheet.version_hash);

        // 3. Cache read
        if let Some(cached) = self.gap_repo.get_cached(&cache_key).await? {
            tracing::debug!("returning cached gap report (computed at {})", cached.computed_at);
            return Ok(cached);
        }

        // 4. Extract user skills
        let extractor = UserSkillExtractor { life_sheet_repo: self.life_sheet_repo.clone() };
        let user_tokens = extractor.extract(&life_sheet).await;
        let user_skill_set: HashSet<CanonicalSkill> = user_tokens.iter().map(|t| t.canonical.clone()).collect();

        // 5. Extract JD skills (parallel per job)
        let jobs = futures::future::try_join_all(
            job_ids.iter().map(|id| self.job_repo.get(*id))
        ).await?;
        let jd_skill_sets: Vec<(Uuid, Vec<JdSkillEntry>)> = jobs.iter()
            .filter_map(|j| j.as_ref())
            .map(|j| (j.id, extract_from_jd(&j.description)))
            .collect();

        // 6. Compute gap matrix
        let goals_text = life_sheet.goals.as_ref()
            .and_then(|g| g.short_term.as_deref())
            .unwrap_or("");
        let skill_gaps = GapMatrix::compute(&user_skill_set, &jd_skill_sets, &options, goals_text);
        let upskilling_priorities = skill_gaps.iter().take(5).cloned().collect();

        // 7. Coverage metric
        let total_required: usize = jd_skill_sets.iter()
            .flat_map(|(_, skills)| skills.iter())
            .filter(|s| s.section == RequirementSection::Required)
            .map(|s| s.canonical.clone())
            .collect::<HashSet<_>>()
            .len();
        let covered: usize = jd_skill_sets.iter()
            .flat_map(|(_, skills)| skills.iter())
            .filter(|s| s.section == RequirementSection::Required && user_skill_set.contains(&s.canonical))
            .map(|s| s.canonical.clone())
            .collect::<HashSet<_>>()
            .len();
        let match_coverage = if total_required == 0 { 1.0 } else { covered as f32 / total_required as f32 };

        // 8. Career transition step (optional)
        let transferable_skills = if options.career_transition {
            if let Some(llm) = &self.llm {
                let target_role = options.target_role.as_deref().unwrap_or("target role");
                let finder = TransferableSkillFinder { llm: llm.clone() };
                finder.find(&life_sheet, &skill_gaps, target_role).await
                    .unwrap_or_else(|e| { tracing::warn!("transferable skill finder failed: {e}"); vec![] })
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let report = GapReport {
            id: Uuid::new_v4(),
            user_skills: user_tokens,
            skill_gaps,
            upskilling_priorities,
            transferable_skills,
            match_coverage,
            corpus_size: jd_skill_sets.len(),
            computed_at: Utc::now(),
            ttl_hours: 24,
        };

        // 9. Cache write
        self.gap_repo.save(&cache_key, &report).await?;

        Ok(report)
    }

    /// Identify transferable skills for a career transitioner (explicit entry point).
    pub async fn find_transferable_skills(
        &self,
        target_role: &str,
    ) -> Result<Vec<TransferableSkill>, GapAnalysisError> {
        let llm = self.llm.as_ref().ok_or(GapAnalysisError::LlmRequired)?;
        let life_sheet = self.life_sheet_repo.load().await?;
        let gaps = self.analyze(&[], GapAnalysisOptions {
            career_transition: false,
            target_role: Some(target_role.to_string()),
            ..Default::default()
        }).await?;
        let finder = TransferableSkillFinder { llm: llm.clone() };
        finder.find(&life_sheet, &gaps.skill_gaps, target_role).await
    }

    /// Evict expired cache entries (run on TUI startup and discovery loop completion).
    pub async fn evict_expired_cache(&self) -> Result<(), GapAnalysisError> {
        let n = self.gap_repo.delete_expired().await?;
        if n > 0 {
            tracing::debug!("evicted {n} expired gap analysis cache entries");
        }
        Ok(())
    }
}

fn compute_cache_key(sorted_ids: &[Uuid], life_sheet_version_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for id in sorted_ids {
        hasher.update(id.as_bytes());
    }
    hasher.update(life_sheet_version_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

- **Verification**: Integration test using `#[sqlx::test(migrations = "migrations")]` — create a LifeSheet with skills ["Python"], create 3 jobs with descriptions containing "Rust, Python, Kubernetes", run `analyze()` — report should contain "Rust" and "Kubernetes" as gaps, coverage should be ~33%.

---

### Phase 2 — Transferable Skills & Career Transition Support

**Step 2.1 — TransferableSkillFinder** (`lazyjob-core/src/gap_analysis/transfer.rs`)

The LLM is asked to identify bridges between the user's experience text and the identified skill gaps. The output is a structured JSON array, never auto-applied to the LifeSheet.

```rust
pub struct TransferableSkillFinder {
    pub llm: Arc<dyn LlmProvider>,
}

/// Structured output expected from the LLM.
#[derive(Deserialize)]
struct TransferableSkillLlmOutput {
    transferable_skills: Vec<TransferableSkillRaw>,
}

#[derive(Deserialize)]
struct TransferableSkillRaw {
    source_experience: String,
    target_skill: String,
    bridge_explanation: String,
    confidence: f32,
}

impl TransferableSkillFinder {
    #[tracing::instrument(skip(self, life_sheet), fields(target_role))]
    pub async fn find(
        &self,
        life_sheet: &LifeSheet,
        gaps: &[SkillGap],
        target_role: &str,
    ) -> Result<Vec<TransferableSkill>, GapAnalysisError> {
        if gaps.is_empty() {
            return Ok(vec![]);
        }

        // Only include Critical and Significant gaps — don't pad LLM context with Minor gaps.
        let significant_gaps: Vec<&str> = gaps.iter()
            .filter(|g| g.gap_severity >= GapSeverity::Significant)
            .take(15)
            .map(|g| g.skill_name.as_str())
            .collect();

        // Build experience context (most recent 3 positions, summarized)
        let experience_text: String = life_sheet.work_experience.iter().take(3)
            .map(|e| format!("- {} at {}: {}", e.position, e.company,
                e.summary.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"You are helping a career professional understand how their existing experience maps to new role requirements.

Experience background:
{experience_text}

Target role: {target_role}

Skills gaps identified (the user lacks these skills in their profile):
{gaps}

For each gap where you can identify a genuine transferable skill bridge from the experience background, provide:
1. The specific experience excerpt that demonstrates the transferable skill
2. The target skill name
3. A brief explanation of the bridge (1-2 sentences max)
4. A confidence score (0.0-1.0) — use 0.0 for weak or speculative bridges

Return ONLY a JSON object with this schema:
{{"transferable_skills": [{{"source_experience": "...", "target_skill": "...", "bridge_explanation": "...", "confidence": 0.0}}]}}

IMPORTANT: Only include genuine transferable skills. Do not fabricate or exaggerate bridges. If there are no genuine bridges for a gap, omit it. These suggestions will be shown to the user for their own review — they are not auto-added to any resume."#,
            experience_text = experience_text,
            target_role = target_role,
            gaps = significant_gaps.join(", "),
        );

        let messages = vec![ChatMessage { role: Role::User, content: prompt }];
        let response = self.llm.chat(&messages, None).await
            .map_err(|e| GapAnalysisError::LlmError(e.to_string()))?;

        // Parse structured JSON output
        let output: TransferableSkillLlmOutput = serde_json::from_str(&response.content)
            .map_err(|e| GapAnalysisError::LlmOutputParseError(e.to_string()))?;

        Ok(output.transferable_skills.into_iter()
            .filter(|t| t.confidence > 0.3) // filter low-confidence
            .map(|t| TransferableSkill {
                source_experience: t.source_experience,
                target_skill: t.target_skill,
                bridge_explanation: t.bridge_explanation,
                confidence: t.confidence,
            })
            .collect())
    }
}
```

- **Verification**: Unit test with a mock `LlmProvider` that returns a hardcoded JSON response — assert that `TransferableSkill.bridge_explanation` is not empty, `confidence > 0.3`, and the result is not auto-added to any LifeSheet table.

**Step 2.2 — ESCO Alias Expansion** (`lazyjob-core/src/gap_analysis/esco.rs`)

Gated on `GapAnalysisOptions::use_esco_expansion = true`.

```rust
/// ESCO alias index: canonical_name → Vec<related_names> (parent concepts, child variants).
/// Loaded from bundled ESCO subset (~2MB compressed to ~500KB trimmed subset).
static ESCO_INDEX: Lazy<EscoIndex> = Lazy::new(|| {
    let raw = include_str!("../../assets/esco_skills_subset.yaml");
    EscoIndex::from_yaml(raw).expect("esco_skills_subset.yaml bundled at compile time")
});

pub struct EscoExpander;

impl EscoExpander {
    /// Expand a set of user skills with ESCO parent/sibling concepts.
    /// "Python" → adds "scripting languages", "programming", etc.
    pub fn expand(user_skills: &HashSet<CanonicalSkill>) -> HashSet<CanonicalSkill> {
        let mut expanded = user_skills.clone();
        for skill in user_skills.iter() {
            if let Some(related) = ESCO_INDEX.related(skill) {
                for r in related {
                    expanded.insert(normalizer::normalize(&r));
                }
            }
        }
        expanded
    }
}
```

The `esco_skills_subset.yaml` asset is generated by a one-time script that filters the ESCO JSON dump to the ~5K most common tech skills and their immediate parent/child relationships. The filtered file is committed to `lazyjob-core/assets/`.

**Step 2.3 — Embedding-Based Fuzzy Skill Matching** (Phase 2, optional)

When two skills have Jaro-Winkler < 0.85 but semantic similarity > 0.85 via embeddings:

```rust
/// Called only when `use_esco_expansion = false` and Ollama is available.
pub async fn merge_semantically_similar(
    gaps: &mut Vec<SkillGap>,
    user_skills: &HashSet<CanonicalSkill>,
    embedder: &Arc<dyn EmbeddingProvider>,
) -> Result<(), GapAnalysisError> {
    // Embed all gap skill names
    let gap_names: Vec<String> = gaps.iter().map(|g| g.skill_name.clone()).collect();
    let gap_embeddings = embedder.embed_batch(&gap_names).await?;

    // Embed user skill names
    let user_names: Vec<String> = user_skills.iter().map(|s| s.as_str().to_string()).collect();
    let user_embeddings = embedder.embed_batch(&user_names).await?;

    // For each gap, check if any user skill has cosine_similarity > 0.85
    gaps.retain(|gap| {
        let gap_emb = &gap_embeddings[&gap.skill_name];
        user_embeddings.values().all(|user_emb| cosine_similarity(gap_emb, user_emb) < 0.85)
    });
    Ok(())
}
```

---

### Phase 3 — TUI Skills Heat Map

**Step 3.1 — GapAnalysisView** (`lazyjob-tui/src/views/gap_analysis.rs`)

Full-screen view with a left pane (heat map) and right pane (gap detail + learning resource).

```rust
pub struct GapAnalysisView {
    pub report: Option<GapReport>,
    pub selected_idx: usize,
    pub focus: GapAnalysisFocus,
    pub loading: bool,
}

#[derive(PartialEq, Eq)]
pub enum GapAnalysisFocus { HeatMap, Detail }

impl GapAnalysisView {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if self.loading {
            let spinner = Paragraph::new("Computing gap analysis…")
                .alignment(Alignment::Center);
            frame.render_widget(spinner, area);
            return;
        }
        if self.report.is_none() {
            let prompt = Paragraph::new("No gap report available. Press 'r' to run analysis.")
                .alignment(Alignment::Center);
            frame.render_widget(prompt, area);
            return;
        }
        let report = self.report.as_ref().unwrap();

        // Warning for small corpus
        let (main_area, warning_area) = if report.corpus_size < 10 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(2)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        // Split into left (heat map) + right (detail) panes
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(main_area);

        SkillHeatMapWidget::new(report, self.selected_idx, self.focus == GapAnalysisFocus::HeatMap)
            .render(panes[0], frame.buffer_mut());

        if let Some(gap) = report.skill_gaps.get(self.selected_idx) {
            GapDetailPanel::new(gap, report).render(panes[1], frame.buffer_mut());
        }

        if let Some(warn_area) = warning_area {
            let msg = format!(
                " Warning: only {} JDs in corpus — results may be noisy. Recommend 10+ jobs.",
                report.corpus_size
            );
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().fg(Color::Yellow)),
                warn_area,
            );
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &tokio::sync::mpsc::Sender<AppEvent>) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(report) = &self.report {
                    self.selected_idx = (self.selected_idx + 1).min(report.skill_gaps.len().saturating_sub(1));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected_idx = self.selected_idx.saturating_sub(1);
            }
            KeyCode::Char('r') => {
                let _ = tx.try_send(AppEvent::RunGapAnalysis);
            }
            KeyCode::Tab => {
                self.focus = if self.focus == GapAnalysisFocus::HeatMap {
                    GapAnalysisFocus::Detail
                } else {
                    GapAnalysisFocus::HeatMap
                };
            }
            _ => {}
        }
    }
}
```

**Step 3.2 — SkillHeatMapWidget** (`lazyjob-tui/src/widgets/skill_heat_map.rs`)

Renders a table of skill gaps color-coded by severity. Implements `ratatui::widgets::Widget`.

```rust
pub struct SkillHeatMapWidget<'a> {
    report: &'a GapReport,
    selected_idx: usize,
    focused: bool,
}

impl<'a> Widget for SkillHeatMapWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let header = Row::new(vec!["Skill", "Freq", "Severity", "Priority"])
            .style(Style::default().bold());

        let rows: Vec<Row> = self.report.skill_gaps.iter().enumerate().map(|(i, gap)| {
            let severity_cell = Cell::from(match gap.gap_severity {
                GapSeverity::Critical    => "Critical",
                GapSeverity::Significant => "Significant",
                GapSeverity::Minor       => "Minor",
            }).style(Style::default().fg(match gap.gap_severity {
                GapSeverity::Critical    => Color::Red,
                GapSeverity::Significant => Color::Yellow,
                GapSeverity::Minor       => Color::DarkGray,
            }));

            let row = Row::new(vec![
                Cell::from(gap.skill_name.as_str()),
                Cell::from(format!("{:.0}%", gap.frequency * 100.0)),
                severity_cell,
                Cell::from(format!("{:.2}", gap.priority_score)),
            ]);

            if i == self.selected_idx {
                row.style(Style::default().bg(Color::DarkGray).bold())
            } else {
                row
            }
        }).collect();

        let widths = [
            Constraint::Percentage(45),
            Constraint::Percentage(12),
            Constraint::Percentage(23),
            Constraint::Percentage(20),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default()
                .title(format!(" Skills Gap ({} gaps, {:.0}% covered) ",
                    self.report.skill_gaps.len(),
                    self.report.match_coverage * 100.0))
                .borders(Borders::ALL)
                .border_style(if self.focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                }));
        Widget::render(table, area, buf);
    }
}
```

**Step 3.3 — GapDetailPanel** (`lazyjob-tui/src/widgets/gap_detail.rs`)

Shows: skill name, full list of jobs requiring it (with clickable navigation), transferable skill bridges (if any), and learning resource.

```rust
pub struct GapDetailPanel<'a> {
    gap: &'a SkillGap,
    report: &'a GapReport,
}

impl<'a> Widget for GapDetailPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("Skill: ", Style::default().bold()),
                Span::raw(self.gap.skill_name.as_str()),
            ]),
            Line::from(format!("Frequency: {:.0}% of target JDs", self.gap.frequency * 100.0)),
            Line::from(format!("Required in: {} JDs | Preferred in: {} JDs",
                self.gap.required_in_jds, self.gap.preferred_in_jds)),
            Line::raw(""),
        ];

        if !self.gap.present_in_jobs.is_empty() {
            lines.push(Line::from(Span::styled("Found in jobs:", Style::default().bold())));
            for job_id in &self.gap.present_in_jobs {
                lines.push(Line::from(format!("  • {}", job_id)));
            }
            lines.push(Line::raw(""));
        }

        // Transferable skill suggestions
        let bridges: Vec<&TransferableSkill> = self.report.transferable_skills.iter()
            .filter(|t| t.target_skill == self.gap.skill_name)
            .collect();
        if !bridges.is_empty() {
            lines.push(Line::from(Span::styled("Transferable from your background:", Style::default().fg(Color::Green).bold())));
            for bridge in bridges {
                lines.push(Line::from(format!("  \"{}\"", bridge.source_experience)));
                lines.push(Line::from(Span::styled(
                    format!("  → {} (confidence: {:.0}%)", bridge.bridge_explanation, bridge.confidence * 100.0),
                    Style::default().fg(Color::Green),
                )));
                lines.push(Line::raw(""));
            }
        }

        // Learning resource
        if let Some(resource) = &self.gap.learning_resource {
            lines.push(Line::from(Span::styled("Learn:", Style::default().bold())));
            lines.push(Line::from(format!(
                "  {} — {} ({})",
                resource.title,
                resource.url,
                if resource.is_free { "Free" } else { "Paid" }
            )));
        }

        let para = Paragraph::new(lines)
            .block(Block::default().title(" Gap Detail ").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        Widget::render(para, area, buf);
    }
}
```

---

### Phase 4 — CLI Integration

**Step 4.1 — CLI Subcommand** (`lazyjob-cli/src/commands/gap.rs`)

```rust
#[derive(clap::Args)]
pub struct GapArgs {
    /// Specific job IDs to analyze against (space-separated UUIDs).
    /// If omitted, uses all starred/saved jobs.
    #[arg(long, num_args = 0..)]
    jobs: Vec<Uuid>,

    /// Enable career transition mode (requires --target-role).
    #[arg(long)]
    career_transition: bool,

    /// Target role description for career transition analysis.
    #[arg(long)]
    target_role: Option<String>,

    /// Include "nice-to-have" skills in the gap report.
    #[arg(long)]
    include_preferred: bool,

    /// Output format: table (default) or json.
    #[arg(long, default_value = "table")]
    format: OutputFormat,
}

pub async fn run_gap_command(args: GapArgs, service: GapAnalysisService) -> anyhow::Result<()> {
    let options = GapAnalysisOptions {
        include_nice_to_have: args.include_preferred,
        career_transition: args.career_transition,
        target_role: args.target_role.clone(),
        ..Default::default()
    };

    println!("Running gap analysis...");
    let report = service.analyze(&args.jobs, options).await?;

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Table => {
            println!("Coverage: {:.0}%", report.match_coverage * 100.0);
            println!("Corpus: {} JDs\n", report.corpus_size);
            println!("{:<30} {:<10} {:<14} {:>8}", "Skill", "Freq", "Severity", "Priority");
            println!("{}", "─".repeat(66));
            for gap in &report.skill_gaps {
                println!("{:<30} {:<10} {:<14} {:>8.2}",
                    gap.skill_name,
                    format!("{:.0}%", gap.frequency * 100.0),
                    format!("{:?}", gap.gap_severity),
                    gap.priority_score,
                );
            }
        }
    }
    Ok(())
}
```

**Step 4.2 — Cache Invalidation Hook**

In `lazyjob-core/src/discovery/service.rs`, after `run_discovery()` completes, call:

```rust
gap_analysis_service.evict_expired_cache().await.ok();
```

This ensures stale gap reports are cleared whenever new jobs are discovered.

---

### Phase 5 — Corpus Size Warning + Polish

- Surface "Not enough data" warning in TUI when `corpus_size < 10` (already in `GapAnalysisView::render` above via the yellow warning bar).
- Add `lazyjob gap analyze --min-corpus 10` flag that exits with error if corpus is too small.
- Add a `LifeSheetChangeListener` tokio task that watches for `LifeSheetUpdated` events (broadcast channel) and calls `evict_expired_cache()`.
- Add `lazyjob gap clear-cache` CLI subcommand for manual invalidation.

---

## Key Crate APIs

Concrete function signatures that will be called:

```rust
// skill normalization
regex::Regex::new(&pattern) -> Result<Regex, regex::Error>
regex_instance.find_iter(text: &str) -> impl Iterator<Item = Match<'_>>
regex::escape(pattern: &str) -> String           // escapes special chars in skill names

// fuzzy matching (Phase 2 jaro-winkler fallback)
strsim::jaro_winkler(a: &str, b: &str) -> f64    // returns 0.0–1.0

// HTML sanitization
ammonia::Builder::default().clean(html: &str) -> ammonia::Document
ammonia_doc.to_string() -> String

// content hashing (cache key)
sha2::Sha256::new() -> sha2::Sha256
sha2::Digest::update(&mut hasher, data: &[u8])
sha2::Digest::finalize(hasher: sha2::Sha256) -> sha2::digest::Output<sha2::Sha256>
format!("{:x}", digest_output)                   // hex string

// YAML asset loading
serde_yaml::from_str::<T>(s: &str) -> Result<T, serde_yaml::Error>

// SQLx queries
sqlx::query!("...", params).fetch_optional(&pool).await -> Result<Option<Row>, sqlx::Error>
sqlx::query!("...", params).execute(&pool).await -> Result<SqliteQueryResult, sqlx::Error>

// once_cell
once_cell::sync::Lazy<T>::new(f: fn() -> T) -> Lazy<T>

// tokio parallel job fetching
futures::future::try_join_all(futures) -> impl Future<Output = Result<Vec<T>, E>>
```

---

## Error Handling

```rust
// lazyjob-core/src/gap_analysis/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GapAnalysisError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("life sheet not found")]
    LifeSheetNotFound,

    #[error("LLM provider required for this operation but none is configured")]
    LlmRequired,

    #[error("LLM call failed: {0}")]
    LlmError(String),

    #[error("failed to parse LLM structured output: {0}")]
    LlmOutputParseError(String),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("deserialization of cached report failed: {0}")]
    DeserializationFailed(serde_json::Error),

    #[error("gap analysis corpus too small: {found} JDs (minimum: {minimum})")]
    CorpusTooSmall { found: usize, minimum: usize },
}

// Type alias for convenience
pub type Result<T> = std::result::Result<T, GapAnalysisError>;
```

---

## Testing Strategy

### Unit Tests

Each component tested in isolation:

```rust
// normalizer tests
#[test]
fn normalize_nodejs_variants() {
    assert_eq!(normalize("Node.js"), normalize("NodeJS"));
    assert_eq!(normalize("C++"), normalize("cpp"));  // via alias table
    assert_eq!(normalize("ML"), normalize("machine learning")); // via alias table
}

// lexicon tests
#[test]
fn extract_skills_from_text_basic() {
    let skills = extract_skills_from_text("5+ years Python, experience with Kubernetes and Terraform");
    let canonical_names: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
    assert!(canonical_names.contains(&"python"));
    assert!(canonical_names.contains(&"kubernetes"));
    assert!(canonical_names.contains(&"terraform"));
}

// jd extractor section detection
#[test]
fn jd_extractor_classifies_sections() {
    let jd = "Required:\nRust\nKubernetes\n\nNice to have:\nTerraform";
    let skills = extract_from_jd(jd);
    let rust = skills.iter().find(|s| s.canonical.as_str() == "rust").unwrap();
    assert_eq!(rust.section, RequirementSection::Required);
    let terraform = skills.iter().find(|s| s.canonical.as_str() == "terraform").unwrap();
    assert_eq!(terraform.section, RequirementSection::Preferred);
}

// gap matrix
#[test]
fn gap_matrix_excludes_user_skills() {
    let user_skills: HashSet<_> = [normalize("python")].into();
    let jd_skills = vec![(
        Uuid::new_v4(),
        vec![
            JdSkillEntry { canonical: normalize("python"), raw_name: "Python".into(), section: RequirementSection::Required },
            JdSkillEntry { canonical: normalize("rust"), raw_name: "Rust".into(), section: RequirementSection::Required },
        ],
    )];
    let gaps = GapMatrix::compute(&user_skills, &jd_skills, &Default::default(), "");
    assert!(!gaps.iter().any(|g| g.canonical == normalize("python")));
    assert!(gaps.iter().any(|g| g.canonical == normalize("rust")));
}

// priority scorer
#[test]
fn priority_scorer_career_relevance_boost() {
    let goals_skills: HashSet<_> = [normalize("rust")].into();
    let score_with_goal = PriorityScorer::score(0.5, 3, 1, &normalize("rust"), &goals_skills);
    let score_without_goal = PriorityScorer::score(0.5, 3, 1, &normalize("rust"), &HashSet::new());
    assert!((score_with_goal / score_without_goal - 1.5).abs() < 0.01);
}
```

### Integration Tests

```rust
// Using #[sqlx::test(migrations = "migrations")]
#[sqlx::test(migrations = "migrations")]
async fn test_gap_analysis_end_to_end(pool: sqlx::Pool<sqlx::Sqlite>) {
    // Arrange: insert a LifeSheet with Python skill
    // Insert 3 jobs requiring Rust + Python
    // Act: run GapAnalysisService::analyze()
    // Assert: report contains Rust as Critical gap, Python is NOT in gaps
    //         match_coverage ≈ 0.33 (Python covered, Rust not)
    //         report is cached in gap_analysis_cache table
}

#[sqlx::test(migrations = "migrations")]
async fn test_cache_ttl_expiry(pool: sqlx::Pool<sqlx::Sqlite>) {
    // Insert a report with computed_at = now() - 25h → expires_at in the past
    // Assert: get_cached returns None
    // Assert: delete_expired removes the row
}
```

### TUI Tests

```rust
// Render test using ratatui TestBackend
#[test]
fn skill_heat_map_renders_without_panic() {
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let report = make_test_gap_report(5); // factory fn
    terminal.draw(|f| {
        SkillHeatMapWidget::new(&report, 0, true).render(f.size(), f.buffer_mut());
    }).unwrap();
}

#[test]
fn gap_analysis_view_shows_warning_for_small_corpus() {
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let mut report = make_test_gap_report(2); // corpus_size = 2
    report.corpus_size = 2;
    let mut view = GapAnalysisView { report: Some(report), selected_idx: 0, focus: GapAnalysisFocus::HeatMap, loading: false };
    terminal.draw(|f| view.render(f, f.size())).unwrap();
    let buffer_str: String = terminal.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect();
    assert!(buffer_str.contains("Warning"));
}
```

### LLM Integration Tests

The `TransferableSkillFinder` is tested with a `MockLlmProvider` that returns a pre-built JSON response:

```rust
#[tokio::test]
async fn transferable_skill_finder_parses_llm_response() {
    let mock_llm = Arc::new(MockLlmProvider::returning(json!({
        "transferable_skills": [{
            "source_experience": "Led platoon of 40 soldiers",
            "target_skill": "team management",
            "bridge_explanation": "Managing a team of 40 in high-stakes conditions maps directly to people management.",
            "confidence": 0.85
        }]
    }).to_string()));
    let finder = TransferableSkillFinder { llm: mock_llm };
    let life_sheet = make_test_life_sheet();
    let results = finder.find(&life_sheet, &[], "engineering manager").await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].confidence > 0.3);
    // Assert never auto-added to LifeSheet:
    assert!(life_sheet.skills.iter().all(|s| s.name != "team management"));
}
```

---

## Open Questions

1. **Corpus size floor**: The spec recommends minimum 10 JDs. Should `GapAnalysisService::analyze()` return an error (`GapAnalysisError::CorpusTooSmall`) or a warning? Current plan: return the report but include a `corpus_size` field; TUI and CLI both surface a yellow warning. Only error if `--min-corpus` flag is explicitly passed.

2. **ESCO bundle size**: Including the trimmed 5K skill subset adds ~500KB to the binary. For Phase 1, the alias table (~50KB) is sufficient. The ESCO subset can be added as an optional feature flag: `features = ["esco-expansion"]` in `Cargo.toml`, gating the `ESCO_INDEX` static and the `EscoExpander` struct.

3. **Learning resources staleness**: The static YAML is committed to the repo. To keep it fresh, a `lazyjob-community` GitHub repo could host the YAML and LazyJob could optionally pull updates. For MVP: static bundled YAML, no network call for resources.

4. **Career transition LLM cost**: ~$0.03 per analysis run (approx 2000 input tokens + 500 output tokens at Claude Haiku pricing). This is low enough to not gate behind premium in Phase 1, but the `TransferableSkillFinder` should still be an explicit opt-in via `--career-transition` flag, not run automatically.

5. **Phase 2 embedding matching**: The `merge_semantically_similar()` function requires Ollama to be running. If Ollama is unavailable, the function should no-op (not error) — the gap list may contain false positives but remains functional.

6. **SkillToken deduplication**: The current `UserSkillExtractor::extract()` deduplicates by canonical skill, preferring `ExplicitSkillTable` over lexicon sources. However, a user might list "Python" in the explicit table AND have "python" extracted from experience text. The deduplication retains the explicit entry — this is correct behavior.

---

## Related Specs

- [specs/profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) — source of truth for user skills
- [specs/profile-resume-tailoring.md](./profile-resume-tailoring.md) — consumes GapReport for ATS keyword injection
- [specs/job-search-semantic-matching.md](./job-search-semantic-matching.md) — shares embedding infrastructure (Phase 2)
- [specs/07-resume-tailoring-pipeline.md](./07-resume-tailoring-pipeline.md) — GapReport flows into the tailoring pipeline
- [specs/agentic-prompt-templates.md](./agentic-prompt-templates.md) — `TransferableSkillFinder` prompt registered as `LoopType::TransferableSkillFinder`
