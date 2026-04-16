# Plan: Task 25 — semantic-matching

## Files to Create
1. `crates/lazyjob-core/migrations/003_job_embeddings.sql` — new table
2. `crates/lazyjob-core/src/discovery/matching.rs` — MatchScorer, GhostDetector, Embedder trait

## Files to Modify
3. `crates/lazyjob-core/src/discovery/mod.rs` — add `pub mod matching` + re-exports

## Types / Functions to Define

### matching.rs
```rust
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub struct GhostScore { pub score: u8 }
impl GhostScore {
    pub fn is_likely_ghost(&self) -> bool { self.score >= 5 }
}

pub struct GhostDetector {
    pub duplicate_description: bool,
    pub high_application_count: bool,
}
impl GhostDetector {
    pub fn score(&self, job: &Job) -> GhostScore  // 7 signals
}

pub struct MatchScorer { embedder: Arc<dyn Embedder> }
impl MatchScorer {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self
    pub async fn embed_life_sheet(&self, sheet: &LifeSheet) -> Result<Vec<f32>>
    pub async fn score_job(&self, job: &Job, profile_embedding: &[f32]) -> Result<f64>
    pub async fn score_all(&self, jobs: &mut [Job], sheet: &LifeSheet) -> Result<()>
    pub async fn store_embedding(&self, pool: &PgPool, job_id: &JobId, embedding: &[f32]) -> Result<()>
    pub async fn load_embedding(&self, pool: &PgPool, job_id: &JobId) -> Result<Option<Vec<f32>>>
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32
pub fn life_sheet_to_text(sheet: &LifeSheet) -> String
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8>
fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32>
```

## GhostDetector Signal Scoring
1. Posting age > 60d: `Utc::now() - job.discovered_at > 60 days` → +3
2. Generic title: match against list of generic titles → +2
3. No company name: `job.company_name.is_none()` → +2
4. Salary missing: both salary_min and salary_max are None → +1
5. URL pattern: suspicious URL patterns (no URL, or boards.greenhouse.io without company) → +1
6. Duplicate description: `self.duplicate_description` flag → +1
7. High application count: `self.high_application_count` flag → +1
Score >= 5 = likely ghost

## Tests to Write

### Learning Tests
- `cosine_similarity_orthogonal` — [1,0,0] vs [0,1,0] = 0.0
- `cosine_similarity_identical` — same vector = 1.0
- `cosine_similarity_known` — [3,4] vs [3,4] = 1.0, [1,0] vs [0,1] = 0.0

### Unit Tests
- `cosine_similarity_zero_vector_returns_zero` — guard against division by zero
- `ghost_detector_old_job` — discovered_at 65 days ago → score includes +3
- `ghost_detector_generic_title` — "Software Engineer" → score includes +2
- `ghost_detector_no_company_name` — company_name=None → score includes +2
- `ghost_detector_missing_salary` → score includes +1
- `ghost_detector_combined_likely_ghost` → score >= 5
- `ghost_detector_clean_job_not_ghost` — real job with all fields → score < 5
- `ghost_is_likely_ghost_threshold` — score 5 is ghost, score 4 is not
- `life_sheet_to_text_includes_skills` — output contains skill names
- `embedding_round_trip` — bytes→Vec<f32>→bytes is lossless
- `score_job_uses_cosine_similarity` — mock embedder, check score is [0,1]
- `score_all_sets_match_score_on_jobs` — jobs.match_score is set after score_all

### Integration Tests (skip without DATABASE_URL)
- `store_and_load_embedding` — store [0.1, 0.2, 0.3], load back, values match within epsilon

## Migration 003
```sql
CREATE TABLE job_embeddings (
    job_id UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    embedding BYTEA NOT NULL,
    embedded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## No New Dependencies
All required types already in workspace: `async_trait`, `tokio`, `sqlx`, `chrono`, `uuid`
