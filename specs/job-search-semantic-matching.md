# Spec: Semantic Job Matching

**JTBD**: Find relevant job opportunities without wasting time on ghost jobs or mismatched roles
**Topic**: Score each discovered job against the user's LifeSheet profile using embedding-based semantic similarity
**Domain**: job-search

---

## What

The Semantic Matching subsystem computes a relevance score for each discovered job against the user's LifeSheet profile. It generates dense vector embeddings for job descriptions and the user's experience/skills profile, computes cosine similarity, and stores the resulting `match_score` on the job record. The score drives feed ranking and filters in the TUI. It also supports ESCO-aligned skill inference: from the user's experience text, it infers skills not explicitly listed on their profile, enabling matches for jobs that use different terminology for the same capability.

## Why

Today's job platforms rely on keyword overlap: "Do you list 'Kubernetes' on your resume?" This fails candidates who describe their work differently than HR wrote the JD. Research shows semantic matching finds 60% more relevant profiles than Boolean queries (SpotSaaS, 2026) and 78% of initial ATS screenings now use NLP. LazyJob must apply this capability on the job-seeker side — not just ranking employer databases, but ranking jobs for the candidate. Without this, the discovery feed is just a chronological list; with it, the feed surfaces the 5-7 jobs per day worth actually evaluating.

## How

### Two-phase architecture

**Phase 1: Embedding generation** (async background, post-ingestion)
- Triggered by ralph after each `DiscoveryService::run_discovery()` run
- For each new/updated job with no embedding: generate embedding from `job.title + "\n" + job.description`
- For the LifeSheet: generate embedding from structured text representation of experience, skills, education
- Store embeddings as `BLOB` (serialized `Vec<f32>`) in SQLite

**Phase 2: Scoring** (on-demand or batch)
- Load all job embeddings and the current LifeSheet embedding from SQLite
- Compute cosine similarity: `dot(a, b) / (|a| * |b|)`
- Write `match_score` back to each job record
- Re-run scoring whenever the LifeSheet is updated or new embeddings arrive

### Embedding model choices

| Provider | Model | Dimensions | Latency | Privacy |
|---|---|---|---|---|
| Ollama (local) | `nomic-embed-text` | 768 | ~10ms | Full — no data leaves device |
| Ollama (local) | `mxbai-embed-large` | 1024 | ~20ms | Full — no data leaves device |
| OpenAI (remote) | `text-embedding-3-small` | 1536 | ~200ms | Data sent to OpenAI |
| Anthropic (remote) | No embedding model | — | — | N/A |

**Default for MVP**: Ollama `nomic-embed-text` (768 dims) — offline-first, no data leaves the device, sufficient quality for single-user scale (~1000s of jobs). Remote providers are available if the user opts in via config.

### Scale considerations

For a single user with 500-5000 jobs in SQLite:
- All embeddings fit in memory (~500 jobs × 768 floats × 4 bytes = ~1.5 MB)
- No vector database needed; cosine similarity computed in-process
- Full re-scoring of 5000 jobs against one profile embedding: < 5ms in Rust
- No need for ANN (approximate nearest neighbor); exact search is fast enough

### ESCO-aligned skill inference

A separate, optional enrichment step uses an LLM (via `LlmProvider`) to extract implied skills from the user's experience descriptions and map them to ESCO skill IDs. This expanded skill set is appended to the LifeSheet embedding text, improving recall for career transitioners who describe their work differently than standard JD terminology.

Example: "led a team of 8 engineers shipping distributed services on AWS" → infers: Kubernetes, Docker, AWS ECS, distributed systems, technical leadership, incident response.

This step is gated behind a config flag (`[matching] esco_inference = true`) and requires an active LLM provider.

### Feedback-driven score adjustment

User interactions update a per-job `feedback_signal`:
- `JobStatus::Saved` → boost: `effective_score = match_score * 1.2` (clamped to 1.0)
- `JobStatus::Dismissed` → penalty: record dismissed job embedding for negative sampling
- Future: contrastive learning pass to fine-tune the profile representation using saved vs. dismissed jobs

### Feed ranking formula

Final feed ordering:
```
feed_score = match_score * (1 - ghost_score) * recency_decay * feedback_multiplier
```
Where:
- `ghost_score` ∈ [0.0, 1.0] — from ghost detection spec
- `recency_decay` = `exp(-days_since_posted / 30)` — exponential decay over 30 days
- `feedback_multiplier` ∈ {0.5 (dismissed), 1.0 (neutral), 1.2 (saved)}

## Interface

```rust
// lazyjob-core/src/matching/embedder.rs

#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub struct OllamaEmbedder {
    model: String,      // "nomic-embed-text"
    base_url: String,   // "http://localhost:11434"
    client: reqwest::Client,
}

// lazyjob-core/src/matching/scorer.rs

pub struct MatchScorer {
    embedder: Arc<dyn Embedder>,
    repo: Arc<dyn JobRepository>,
}

impl MatchScorer {
    /// Generate embedding for a job and store it
    pub async fn embed_job(&self, job_id: Uuid) -> Result<Vec<f32>>;

    /// Generate LifeSheet embedding from structured profile text
    pub async fn embed_life_sheet(&self, sheet: &LifeSheet) -> Result<Vec<f32>>;

    /// Score all jobs against the given profile embedding, write back to DB
    pub async fn score_all_jobs(&self, profile_embedding: &[f32]) -> Result<usize>;

    /// Score a single job (for just-discovered jobs)
    pub fn score_one(job_embedding: &[f32], profile_embedding: &[f32]) -> f32;
}

// lazyjob-core/src/matching/skill_inference.rs

pub struct SkillInferenceEngine {
    llm: Arc<dyn LlmProvider>,
}

impl SkillInferenceEngine {
    /// Extract ESCO-aligned skill tags from free-text experience description
    pub async fn infer_skills(&self, experience_text: &str) -> Result<Vec<EscoSkill>>;

    /// Expand LifeSheet with inferred skills and return augmented embedding text
    pub fn augment_life_sheet_text(&self, sheet: &LifeSheet, inferred: &[EscoSkill]) -> String;
}

pub struct EscoSkill {
    pub id: String,       // ESCO URI, e.g. "http://data.europa.eu/esco/skill/S5.8.1"
    pub label: String,    // Human-readable, e.g. "container orchestration"
    pub confidence: f32,  // 0.0–1.0
    pub source: SkillSource,
}

pub enum SkillSource {
    Explicit,        // listed in LifeSheet.skills
    LlmInferred,     // derived from experience text by LLM
    EscoExpanded,    // adjacent skill in ESCO graph
}

// lazyjob-core/src/matching/feed.rs

pub struct FeedRanker;

impl FeedRanker {
    pub fn compute_feed_score(job: &DiscoveredJob, days_since_posted: f32) -> f32 {
        let recency = (-days_since_posted / 30.0_f32).exp();
        let ghost_penalty = 1.0 - job.ghost_score.unwrap_or(0.0);
        let feedback = match job.status {
            JobStatus::Dismissed => 0.5,
            JobStatus::Saved     => 1.2_f32.min(1.0),
            _                    => 1.0,
        };
        (job.match_score.unwrap_or(0.0) * ghost_penalty * recency * feedback).min(1.0)
    }

    pub fn rank_jobs(jobs: &mut Vec<DiscoveredJob>) {
        jobs.sort_by(|a, b| {
            let sa = Self::compute_feed_score(a, /* days */ 0.0);
            let sb = Self::compute_feed_score(b, /* days */ 0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}
```

## Open Questions

- **Ollama availability**: If Ollama is not installed, the match score column stays NULL and the feed falls back to reverse-chronological order. Should we warn the user on startup that matching is unavailable, or silently degrade?
- **Re-scoring trigger**: After the user updates their LifeSheet, re-scoring all ~5000 jobs is fast but generates I/O. Should this happen inline (blocking the TUI briefly) or be dispatched to a ralph subprocess?
- **ESCO inference cost**: Calling the LLM for skill inference on every LifeSheet update is expensive (potentially 5-20 LLM calls for a full work history). Cache the results per experience entry keyed on text hash; only re-infer when text changes.
- **Feedback loop fidelity**: Using `JobStatus::Dismissed` as a negative signal assumes the dismissal was about fit, not about the company being a bad actor or the role being too senior. Should we ask for a dismiss reason?
- **Embedding dimensionality mismatch**: If the user switches embedding models (Ollama → OpenAI), stored embeddings become incompatible. Need a migration path: re-embed all jobs on model change, keyed by `(model_name, model_version)`.

## Implementation Tasks

- [ ] Define `Embedder` trait and implement `OllamaEmbedder` using the `/api/embeddings` endpoint in `lazyjob-core/src/matching/` — refs: `05-job-discovery-layer.md`
- [ ] Add `embedding BLOB` and `match_score REAL` columns to the `jobs` table migration and implement `JobRepository::update_embedding` and `update_match_score` — refs: `04-sqlite-persistence.md`
- [ ] Implement `MatchScorer::embed_life_sheet()` that converts `LifeSheet` struct to normalized text and generates embedding — refs: `03-life-sheet-data-model.md`, `05-job-discovery-layer.md`
- [ ] Implement `MatchScorer::score_all_jobs()` as a batch cosine similarity pass over all unscored/stale job embeddings — refs: `05-job-discovery-layer.md`
- [ ] Implement `FeedRanker::compute_feed_score()` combining match_score, ghost_score, recency decay, and feedback multiplier — refs: `agentic-job-matching.md`
- [ ] Implement `SkillInferenceEngine::infer_skills()` with LLM prompt and caching by experience text hash (skip if Ollama unavailable) — refs: `agentic-job-matching.md`, `17-ralph-prompt-templates.md`
- [ ] Wire embedding generation + scoring into the ralph discovery loop (post-ingestion step) — refs: `06-ralph-loop-integration.md`
