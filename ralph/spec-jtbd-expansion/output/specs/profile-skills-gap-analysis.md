# Spec: Skills Gap Analysis

**JTBD**: B-2 — Understand my skill gaps relative to target roles before they become urgent; C-2 — Know which skill gaps are actual blockers before applying
**Topic**: Compute a user's missing skills by comparing their LifeSheet against a corpus of target job descriptions, then surface an actionable prioritized gap report.
**Domain**: profile-resume

---

## What

The skills gap analysis module reads a user's LifeSheet skills and a set of target job descriptions, then produces a gap report that answers three questions: (1) Which skills appear in target JDs that I don't have? (2) Which gaps are genuine blockers vs. noise? (3) What should I learn first? The module also handles career transitioners, who need to map non-standard experience to target-role skill vocabulary using transferable skill inference. Results are displayed in the TUI as a skills heat map and priority list.

## Why

Without gap analysis, job seekers apply blindly, waste time on roles they can't get, and have no signal for upskilling direction. The career transitioner case is particularly underserved: a military officer, finance analyst, or teacher transitioning to tech has real skills that standard ATS keyword matching completely misses. Gap analysis that can bridge vocabulary ("budget management" → "financial modeling", "troop leadership" → "team management") makes LazyJob genuinely valuable for the career-change audience (JTBD C-1, C-2) vs. every other tool on the market.

From `gap-analysis-and-critique.md`: "Career transition support is the hardest and most underserved case." The existing tools (Jobscan, Teal, Resume Worded) are purely keyword-based and systematically fail career changers. Semantic gap analysis using embeddings is the differentiator.

## How

### Gap Computation Architecture

```
Input:
  - LifeSheet skills (from SQLite: skill.name, skill.esco_code, work_experience.tech_stack)
  - Target JD corpus: N job descriptions (either saved jobs in DB, or a target role query)

Step 1: Skill Extraction from LifeSheet
  Explicit skills: SELECT * FROM skill JOIN skill_category
  Implicit skills: extract skill names from work_experience.tech_stack (JSON array)
  Context skills: run regex lexicon (lazyjob-core/src/lexicon/tech_terms.rs) over
                  work_experience.summary and achievement.description text
  Result: HashSet<SkillToken> where SkillToken { canonical_name, esco_code?, source }

Step 2: Skill Extraction from JD Corpus
  For each JD: run same regex lexicon pass to extract SkillToken set
  Also parse structured requirements from JobDescriptionAnalysis (if already computed)
  Result: HashMap<SkillToken, FrequencyWeight>
  FrequencyWeight = count of JDs requiring this skill / total JDs in corpus

Step 3: ESCO Alias Expansion (optional, Phase 2)
  If ESCO codes are present: expand user skills with ESCO parent/child relationships
  Example: user has "Python" → infer "scripting languages" (parent), "pandas" (sibling)
  This closes false negatives: JD says "scripting experience" and user has "Python"
  Source: embedded ESCO skills subset (trimmed to ~5K most relevant tech skills, ~500KB)

Step 4: Gap Matrix
  For each JD skill with frequency > THRESHOLD (default 0.2 = appears in 20%+ of JDs):
    if skill NOT IN user_skills_expanded:
      → gap entry { skill, frequency, required_level, gap_severity }
  GapSeverity:
    Critical → appears in >50% JDs AND is in "required" not "nice-to-have" section
    Significant → appears in 20-50% JDs
    Minor → appears in <20% JDs (often just one employer's preference)

Step 5: Career Transitioner Framing (if goals.short_term contains pivot signal)
  Input: work_experience text + target role from user preferences
  LLM prompt: "Given these experience descriptions from [source field], identify
               transferable skills that map to these target role requirements: [target skills].
               For each match, explain the bridge."
  Output: TransferableSkillMap { source_experience, target_skill, bridge_explanation }
  This map is DISPLAYED to the user (not used to fabricate resume content).
  Anti-fabrication: these are suggestions for the user to consider, not auto-added skills.
```

### Heat Map Display

In the TUI skills gap view:
- Row: skill name
- Column: frequency bucket (appears in >50% | 20–50% | <20% of target JDs)
- Cell fill: GREEN if in user profile, RED if missing
- Interactive: select a skill to see which JDs require it and a learning resource suggestion

The heat map is re-computed on demand (`lazyjob gap-analysis run`) and cached in a `gap_analysis_cache` table with a TTL of 24 hours. Running a new discovery loop can trigger a cache invalidation.

### Skill Normalization

The biggest challenge in gap analysis is vocabulary mismatches: "Node.js" vs "NodeJS" vs "Node", "ML" vs "machine learning" vs "artificial intelligence." Normalization strategy:
1. **Lowercase + strip punctuation**: "Node.js" → "nodejs", "C++" → "cpp"
2. **Alias table** (static, embedded): common aliases per skill, maintained in `lazyjob-core/src/lexicon/skill_aliases.rs`. Seeded from ESCO aliases (374K aliases for 39K skills — trimmed to relevant subset).
3. **Embedding similarity** (Phase 2, optional): for skills that survive normalization without matching, compute cosine similarity using the same Ollama embeddings as the job search module. If similarity > 0.85, treat as same skill.

### Prioritization Algorithm

The prioritization score for each skill gap:

```
priority_score = frequency_weight * required_multiplier * career_relevance
  where:
    frequency_weight = fraction of target JDs requiring this skill (0.0–1.0)
    required_multiplier = 2.0 if "required" qualifier detected in JDs, 1.0 if "preferred"
    career_relevance = 1.5 if skill appears in user's stated goals.short_term text, 1.0 otherwise
```

Top 5 skills by priority_score are surfaced as "Your upskilling priorities" in the TUI dashboard.

### Learning Resource Suggestions (Phase 2)

For each high-priority gap, suggest a learning path. Phase 1: static map of skill → best-known free resource (e.g., "Rust" → "The Rust Book", "Kubernetes" → "Kubernetes The Hard Way"). Phase 2: query a curated learning resource database. No AI generation of learning plans to avoid hallucinated course recommendations.

## Interface

```rust
// lazyjob-core/src/gap_analysis/mod.rs

pub struct GapAnalysisService {
    pub life_sheet_repo: Arc<dyn LifeSheetRepository>,
    pub job_repo: Arc<dyn JobRepository>,
    pub llm: Option<Arc<dyn LlmProvider>>,  // None = skip LLM steps
}

impl GapAnalysisService {
    /// Run full gap analysis against N recent target jobs.
    pub async fn analyze(
        &self,
        target_job_ids: &[Uuid],   // empty = use all saved/starred jobs
        options: GapAnalysisOptions,
    ) -> Result<GapReport>;

    /// Identify transferable skills for a career transitioner.
    pub async fn find_transferable_skills(
        &self,
        target_role: &str,
    ) -> Result<Vec<TransferableSkill>>;
}

pub struct GapAnalysisOptions {
    pub min_frequency: f32,           // default 0.2
    pub include_nice_to_have: bool,   // default false (required skills only)
    pub use_esco_expansion: bool,     // default false (Phase 2)
    pub max_gaps_to_surface: usize,   // default 20
}

pub struct GapReport {
    pub user_skills: Vec<SkillToken>,
    pub skill_gaps: Vec<SkillGap>,     // sorted by priority_score desc
    pub upskilling_priorities: Vec<SkillGap>,  // top 5
    pub transferable_skills: Vec<TransferableSkill>,
    pub match_coverage: f32,   // % of required skills the user covers
    pub computed_at: DateTime<Utc>,
}

pub struct SkillGap {
    pub skill_name: String,
    pub esco_code: Option<String>,
    pub frequency: f32,
    pub gap_severity: GapSeverity,
    pub priority_score: f32,
    pub present_in_jds: Vec<Uuid>,     // which job IDs require this
    pub learning_resource: Option<String>,
}

pub enum GapSeverity { Critical, Significant, Minor }

pub struct TransferableSkill {
    pub source_experience: String,    // e.g., "Led platoon of 40 soldiers"
    pub target_skill: String,         // e.g., "team management"
    pub bridge_explanation: String,   // LLM-generated, shown to user for review
    pub confidence: f32,
}

// Cache table for computed gap reports
// lazyjob-core/migrations/003_gap_analysis.sql
// CREATE TABLE gap_analysis_cache (
//   id TEXT PK, computed_at TEXT, report_json TEXT, ttl_hours INT DEFAULT 24
// );
```

## Open Questions

- **Corpus size**: How many JDs are needed for a statistically meaningful gap report? With N=10 jobs from the user's saved feed, frequency is volatile. Proposal: minimum 10 JDs, recommend 30+. Surface a "not enough data" warning when corpus is small.
- **ESCO bundle size**: Embedding the full ESCO skills taxonomy (39K skills, 374K aliases) would be ~15MB. A trimmed tech-relevant subset (~5K skills) is ~2MB. Proposal: ship the trimmed subset in Phase 1; let users opt into the full taxonomy in settings.
- **Learning resource quality**: Static skill → resource maps become stale. Who maintains them? Proposal: a community-maintained YAML file in the repo (similar to `awesome-*` lists) that LazyJob bundles. Users can override with their own learning resource mappings.
- **Gap analysis for career changers**: The LLM-powered transferable skill finder is expensive (~$0.03 per analysis). Should it be a premium feature or always-on? Proposal: gated behind a manual "Run career transition analysis" action, not run automatically on every gap analysis.

## Implementation Tasks

- [ ] Create `SkillNormalizer` in `lazyjob-core/src/lexicon/skill_normalizer.rs` — lowercase/strip punctuation + alias lookup from embedded alias table seeded from ESCO aliases
- [ ] Implement `UserSkillExtractor` in `lazyjob-core/src/gap_analysis/extractor.rs` — pulls explicit skills from SQLite + runs regex lexicon over experience/achievement text
- [ ] Implement `GapMatrix::compute` in `lazyjob-core/src/gap_analysis/matrix.rs` — computes frequency weights per JD skill, applies severity thresholds, produces sorted `Vec<SkillGap>`
- [ ] Implement `GapAnalysisService::find_transferable_skills` in `lazyjob-core/src/gap_analysis/transfer.rs` — LLM prompt that maps source experience text to target skill vocabulary, returns `Vec<TransferableSkill>` for user review (never auto-applied to LifeSheet)
- [ ] Add `gap_analysis_cache` table to SQLite migrations and implement cache read/write in `GapAnalysisService`
- [ ] Build TUI skills heat map widget in `lazyjob-tui/src/views/gap_analysis.rs` — skill × frequency matrix with color coding, interactive drill-down to job IDs
- [ ] Add static learning resources YAML at `lazyjob-core/assets/learning_resources.yaml` and integrate lookup into `GapReport.learning_resource` field
