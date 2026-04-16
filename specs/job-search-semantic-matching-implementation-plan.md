# Implementation Plan: Job Search Semantic Matching

## Status
Draft

## Related Spec
`specs/job-search-semantic-matching.md`

## Overview

The Semantic Matching subsystem scores every discovered job against the user's LifeSheet
profile using dense vector embeddings and cosine similarity. It consists of three
cooperating components: an **Embedder** that generates 768-dimensional float vectors from
free text (default: Ollama `nomic-embed-text`, fallback: OpenAI `text-embedding-3-small`);
a **MatchScorer** that batch-computes cosine similarity between all stored job embeddings
and the current profile embedding, writing `match_score` back to SQLite; and a
**FeedRanker** that combines `match_score` with `ghost_score`, recency decay, and
user-feedback multipliers into a single `feed_score` used to sort the TUI job feed.

At single-user scale (500-5000 jobs), all embeddings fit comfortably in RAM (~1.5-7.5 MB),
making exact cosine similarity fast enough (~5 ms for 5000 jobs) without any ANN index.
The entire pipeline runs as an async background loop in `lazyjob-ralph` after each
discovery run, keeping the TUI responsive and match scores up to date within seconds.

An optional, LLM-backed `SkillInferenceEngine` expands the profile embedding text with
ESCO-aligned skill tags inferred from the user's free-text experience descriptions,
improving recall for career transitioners who use different vocabulary than HR job
descriptions. Results are cached per experience-text SHA-256 hash to avoid redundant
LLM calls on LifeSheet updates.

## Prerequisites

### Implementation Plans Required First
- `specs/04-sqlite-persistence-implementation-plan.md` — `Database`, migrations
- `specs/job-search-discovery-engine-implementation-plan.md` — `DiscoveredJob`, `JobRepository`
- `specs/agentic-llm-provider-abstraction-implementation-plan.md` — `LlmProvider` trait
- `specs/profile-life-sheet-data-model-implementation-plan.md` — `LifeSheet` struct

### Crates to Add to Cargo.toml

```toml
# lazyjob-core/Cargo.toml

[dependencies]
# Existing (assumed present from previous plans)
reqwest      = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
tokio        = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
async-trait  = "0.1"
thiserror    = "2"
anyhow       = "1"
tracing      = "0.1"
uuid         = { version = "1", features = ["v4"] }
chrono       = { version = "0.4", features = ["serde"] }
once_cell    = "1"

# New for semantic matching
sha2         = "0.10"    # SHA-256 hash of experience text for cache key
hex          = "0.4"     # hex-encode SHA-256 digests for SQLite TEXT storage
bytemuck     = "1"       # safe &[f32] ↔ &[u8] transmutation for BLOB storage

[dev-dependencies]
wiremock     = "0.6"
sqlx         = { version = "0.8", features = ["sqlite", "runtime-tokio", "migrate", "macros"] }
tempfile     = "3"
tokio        = { version = "1", features = ["full"] }
```

> **Note on `bytemuck`**: We store embeddings as little-endian IEEE 754 `f32` bytes in
> SQLite `BLOB`. `bytemuck::cast_slice::<f32, u8>` and `cast_slice::<u8, f32>` provide
> zero-copy round-trips. No compression is applied at this scale; 5000 × 768 × 4 = 15 MB
> fits well within SQLite's default page cache.

> **Note on ANN / `sqlite-vec`**: The spec explicitly rules out ANN for this scale. We do
> NOT add the `sqlite-vec` extension. All cosine similarity is computed in-process in Rust
> via the `score_all_jobs` path. We revisit when job count exceeds 50,000 or latency
> becomes a user-visible problem.

## Architecture

### Crate Placement

All semantic matching code lives in `lazyjob-core/src/matching/`. This module is
imported by:

- **`lazyjob-ralph`** — `ralph_discovery_loop.rs` calls `MatchingService::run_post_discovery()`
  after `DiscoveryService::run_discovery()` completes.
- **`lazyjob-tui`** — `jobs_feed_view.rs` calls `FeedRanker::rank_jobs()` to sort the feed
  before rendering; it reads pre-computed `match_score` and `ghost_score` from SQLite, so
  the TUI never calls the Embedder directly.

The `JobRepository` (in `lazyjob-core/src/persistence/jobs.rs`) is extended with new
methods: `update_embedding`, `update_match_score`, `list_unembedded_jobs`,
`get_all_embeddings`, and `get_life_sheet_embedding`. All matching logic stays in
`src/matching/`; the repository is a pure I/O boundary.

### Core Types

```rust
// lazyjob-core/src/matching/types.rs

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A 768-float dense embedding vector.
/// Dimensions depend on `EmbeddingModel`; always stored as little-endian f32 bytes.
#[derive(Debug, Clone)]
pub struct Embedding {
    pub model:  EmbeddingModel,
    pub vector: Vec<f32>,
}

impl Embedding {
    /// Cosine similarity ∈ [-1.0, 1.0]; clamped to [0.0, 1.0] for scoring.
    pub fn cosine_similarity(&self, other: &Embedding) -> f32 {
        debug_assert_eq!(
            self.vector.len(),
            other.vector.len(),
            "dimension mismatch: {} vs {}",
            self.vector.len(),
            other.vector.len()
        );
        let dot: f32 = self.vector.iter().zip(&other.vector).map(|(a, b)| a * b).sum();
        let norm_a: f32 = self.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        (dot / (norm_a * norm_b)).max(0.0).min(1.0)
    }

    /// Serialize to bytes for SQLite BLOB storage (little-endian f32 array).
    pub fn to_bytes(&self) -> Vec<u8> {
        bytemuck::cast_slice::<f32, u8>(&self.vector).to_vec()
    }

    /// Deserialize from SQLite BLOB bytes. Returns Err if length not divisible by 4.
    pub fn from_bytes(bytes: &[u8], model: EmbeddingModel) -> Result<Self, MatchingError> {
        if bytes.len() % 4 != 0 {
            return Err(MatchingError::InvalidEmbeddingBytes(bytes.len()));
        }
        let vector: Vec<f32> = bytemuck::cast_slice::<u8, f32>(bytes).to_vec();
        Ok(Embedding { model, vector })
    }
}

/// Supported embedding models. Stored as TEXT in SQLite so we can detect mismatch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingModel {
    OllamaNomic768,   // "nomic-embed-text", 768 dims — default
    OllamaMxbai1024,  // "mxbai-embed-large", 1024 dims
    OpenAiSmall1536,  // "text-embedding-3-small", 1536 dims
}

impl EmbeddingModel {
    pub fn dimensions(&self) -> usize {
        match self {
            Self::OllamaNomic768  => 768,
            Self::OllamaMxbai1024 => 1024,
            Self::OpenAiSmall1536 => 1536,
        }
    }

    pub fn to_db_str(&self) -> &'static str {
        match self {
            Self::OllamaNomic768  => "ollama:nomic-embed-text:768",
            Self::OllamaMxbai1024 => "ollama:mxbai-embed-large:1024",
            Self::OpenAiSmall1536 => "openai:text-embedding-3-small:1536",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ollama:nomic-embed-text:768"         => Some(Self::OllamaNomic768),
            "ollama:mxbai-embed-large:1024"       => Some(Self::OllamaMxbai1024),
            "openai:text-embedding-3-small:1536"  => Some(Self::OpenAiSmall1536),
            _                                     => None,
        }
    }
}

/// Result of scoring a batch of jobs.
#[derive(Debug, Default)]
pub struct ScoringReport {
    pub jobs_scored:    usize,
    pub jobs_skipped:   usize,  // no embedding yet
    pub jobs_errors:    usize,
    pub elapsed_ms:     u64,
}

/// An inferred ESCO skill derived from free-text experience.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EscoSkill {
    /// ESCO URI, e.g. "http://data.europa.eu/esco/skill/S5.8.1"
    pub id:         String,
    /// Human-readable label, e.g. "container orchestration"
    pub label:      String,
    /// Model confidence 0.0–1.0
    pub confidence: f32,
    pub source:     SkillSource,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Explicit,     // listed in LifeSheet.skills
    LlmInferred,  // derived from experience text by LLM
    EscoExpanded, // adjacent skill in ESCO taxonomy graph (Phase 3)
}

/// Per-job aggregated scoring inputs, used by FeedRanker.
#[derive(Debug, Clone)]
pub struct ScoredJob {
    pub job_id:           Uuid,
    pub match_score:      f32,   // cosine similarity ∈ [0.0, 1.0]; None → 0.0 fallback
    pub ghost_score:      f32,   // from ghost detection module; None → 0.0 fallback
    pub days_since_posted: f32,
    pub feedback:         FeedbackSignal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeedbackSignal {
    Neutral,
    Saved,
    Dismissed,
}

impl FeedbackSignal {
    pub fn multiplier(self) -> f32 {
        match self {
            Self::Neutral   => 1.0,
            Self::Saved     => 1.2_f32.min(1.0), // spec: clamped to 1.0
            Self::Dismissed => 0.5,
        }
    }
}
```

### Trait Definitions

```rust
// lazyjob-core/src/matching/embedder.rs

use async_trait::async_trait;
use super::types::{Embedding, EmbeddingModel};
use super::error::MatchingError;

/// Generates dense embeddings from free text.
/// Implementations: `OllamaEmbedder`, `OpenAiEmbedder`, `MockEmbedder` (test).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Generate an embedding for the given text.
    async fn embed(&self, text: &str) -> Result<Embedding, MatchingError>;

    /// Returns the model this embedder uses. Used to detect dimension mismatch on re-config.
    fn model(&self) -> EmbeddingModel;
}

/// Batch variant — more efficient for providers that support batching.
/// Optional extension; types that don't batch can use the default impl.
#[async_trait]
pub trait BatchEmbedder: Embedder {
    /// Embed multiple texts in a single provider call (up to `batch_size` items).
    /// Default: sequential fallback via `self.embed()`.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>, MatchingError> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    fn batch_size(&self) -> usize { 1 }
}

/// Repository extension for embedding I/O.
/// Extends the existing `JobRepository` interface.
#[async_trait]
pub trait EmbeddingRepository: Send + Sync {
    /// Return job IDs with no stored embedding.
    async fn list_unembedded_job_ids(&self, limit: usize) -> Result<Vec<Uuid>, anyhow::Error>;

    /// Return job IDs whose stored embedding model differs from `current_model`.
    async fn list_stale_embedding_ids(
        &self,
        current_model: &str,
        limit: usize,
    ) -> Result<Vec<Uuid>, anyhow::Error>;

    /// Fetch embedding text (title + description) for a job.
    async fn get_job_text(&self, job_id: Uuid) -> Result<Option<String>, anyhow::Error>;

    /// Persist a job's embedding.
    async fn update_job_embedding(
        &self,
        job_id:  Uuid,
        model:   &str,
        bytes:   &[u8],
    ) -> Result<(), anyhow::Error>;

    /// Load all stored job embeddings into memory. Used for batch scoring.
    async fn load_all_job_embeddings(&self) -> Result<Vec<(Uuid, Vec<u8>, String)>, anyhow::Error>;

    /// Write match_score back to a job record.
    async fn update_match_score(&self, job_id: Uuid, score: f32) -> Result<(), anyhow::Error>;

    /// Persist the LifeSheet embedding.
    async fn save_life_sheet_embedding(
        &self,
        profile_hash: &str,
        model:        &str,
        bytes:        &[u8],
    ) -> Result<(), anyhow::Error>;

    /// Load the most recent LifeSheet embedding.
    async fn load_life_sheet_embedding(
        &self,
    ) -> Result<Option<(Vec<u8>, String, String)>, anyhow::Error>;
    //                   ^bytes   ^model  ^profile_hash
}
```

### SQLite Schema

Migration `011_semantic_matching.sql`:

```sql
-- Embedding storage per job
ALTER TABLE jobs ADD COLUMN embedding      BLOB;
ALTER TABLE jobs ADD COLUMN embedding_model TEXT;   -- e.g. "ollama:nomic-embed-text:768"
ALTER TABLE jobs ADD COLUMN match_score     REAL;   -- cosine similarity [0.0, 1.0] or NULL
ALTER TABLE jobs ADD COLUMN feed_score      REAL;   -- computed feed ranking score or NULL

-- Index: find jobs missing embeddings quickly
CREATE INDEX IF NOT EXISTS idx_jobs_no_embedding
    ON jobs (id)
    WHERE embedding IS NULL;

-- Index: find jobs with stale embedding model
CREATE INDEX IF NOT EXISTS idx_jobs_embedding_model
    ON jobs (embedding_model)
    WHERE embedding_model IS NOT NULL;

-- LifeSheet embedding cache (one active row per model)
CREATE TABLE IF NOT EXISTS life_sheet_embeddings (
    id            INTEGER PRIMARY KEY,
    profile_hash  TEXT    NOT NULL,  -- SHA-256 hex of the serialized LifeSheet text
    model         TEXT    NOT NULL,  -- "ollama:nomic-embed-text:768"
    embedding     BLOB    NOT NULL,
    created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE (model)
);

-- ESCO skill inference cache
-- Keyed on SHA-256 of the experience text + model name
CREATE TABLE IF NOT EXISTS esco_inference_cache (
    id              INTEGER PRIMARY KEY,
    text_hash       TEXT    NOT NULL,   -- SHA-256 hex of experience_text
    llm_model       TEXT    NOT NULL,   -- e.g. "claude-3-5-haiku-20241022"
    inferred_skills TEXT    NOT NULL,   -- JSON array of EscoSkill
    created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE (text_hash, llm_model)
);

-- Per-job feedback signals (separate from application status)
CREATE TABLE IF NOT EXISTS job_feedback (
    id          INTEGER PRIMARY KEY,
    job_id      TEXT    NOT NULL REFERENCES jobs(id),
    signal      TEXT    NOT NULL CHECK (signal IN ('saved', 'dismissed', 'neutral')),
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE (job_id)  -- one active signal per job; UPDATE on conflict
);
CREATE INDEX IF NOT EXISTS idx_job_feedback_job_id ON job_feedback (job_id);
```

### Module Structure

```
lazyjob-core/
  src/
    matching/
      mod.rs              # re-exports: Embedder, MatchScorer, FeedRanker, SkillInferenceEngine
      types.rs            # Embedding, EmbeddingModel, EscoSkill, ScoredJob, FeedbackSignal
      error.rs            # MatchingError (thiserror)
      embedder.rs         # Embedder + BatchEmbedder traits
      ollama_embedder.rs  # OllamaEmbedder: reqwest → /api/embeddings
      openai_embedder.rs  # OpenAiEmbedder: async-openai embeddings create
      mock_embedder.rs    # MockEmbedder: deterministic test vectors
      scorer.rs           # MatchScorer — orchestrates embed_job + score_all_jobs
      feed.rs             # FeedRanker::compute_feed_score + rank_jobs
      skill_inference.rs  # SkillInferenceEngine — LLM skill extraction with cache
      service.rs          # MatchingService — top-level entry point for ralph loop
    persistence/
      jobs.rs             # Extended with EmbeddingRepository impl
      life_sheet.rs       # Extended with LifeSheet → text conversion
```

## Implementation Phases

### Phase 1 — Embedder Trait + OllamaEmbedder (MVP Core)

#### Step 1.1 — Define `MatchingError`

File: `lazyjob-core/src/matching/error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum MatchingError {
    #[error("Ollama embedding request failed: {0}")]
    OllamaRequest(#[from] reqwest::Error),

    #[error("Ollama returned unexpected status {status}: {body}")]
    OllamaStatus { status: u16, body: String },

    #[error("OpenAI embedding request failed: {0}")]
    OpenAiRequest(String),

    #[error("Embedding bytes have invalid length {0} (not divisible by 4)")]
    InvalidEmbeddingBytes(usize),

    #[error("Embedding model mismatch: stored {stored}, current {current}")]
    ModelMismatch { stored: String, current: String },

    #[error("Embedder unavailable: {0}")]
    EmbedderUnavailable(String),

    #[error("Database error: {0}")]
    Database(#[from] anyhow::Error),

    #[error("Skill inference failed: {0}")]
    SkillInference(String),
}
```

**Verification**: `cargo check -p lazyjob-core`

---

#### Step 1.2 — Define `Embedding` and `EmbeddingModel` types

File: `lazyjob-core/src/matching/types.rs` — as shown in Core Types section above.

Key invariants to enforce:
- `Embedding::from_bytes` returns `Err(InvalidEmbeddingBytes)` if `bytes.len() % 4 != 0`
- `cosine_similarity` handles zero-norm vectors by returning `0.0` (not NaN/panic)
- `to_bytes` / `from_bytes` round-trip: `assert_eq!(Embedding::from_bytes(&e.to_bytes(), model).unwrap().vector, e.vector)`

**Verification**: Unit test `test_embedding_roundtrip` and `test_cosine_zero_norm`.

---

#### Step 1.3 — Implement `OllamaEmbedder`

File: `lazyjob-core/src/matching/ollama_embedder.rs`

The Ollama `/api/embeddings` endpoint:
- `POST http://localhost:11434/api/embeddings`
- Body: `{"model": "nomic-embed-text", "prompt": "<text>"}`
- Response: `{"embedding": [0.123, -0.456, ...]}`

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use super::{embedder::Embedder, error::MatchingError, types::{Embedding, EmbeddingModel}};

pub struct OllamaEmbedder {
    model_name: String,     // "nomic-embed-text"
    model_enum: EmbeddingModel,
    base_url:   String,     // "http://localhost:11434"
    client:     Client,
}

impl OllamaEmbedder {
    pub fn new(model: EmbeddingModel, base_url: impl Into<String>) -> Self {
        let model_name = match &model {
            EmbeddingModel::OllamaNomic768  => "nomic-embed-text".to_string(),
            EmbeddingModel::OllamaMxbai1024 => "mxbai-embed-large".to_string(),
            _ => panic!("OllamaEmbedder: unsupported model {:?}", model),
        };
        Self {
            model_name,
            model_enum: model,
            base_url: base_url.into(),
            client: Client::new(),
        }
    }

    /// Constructor for tests: override base URL (wiremock target).
    pub fn with_base_url(model: EmbeddingModel, base_url: impl Into<String>) -> Self {
        Self::new(model, base_url)
    }
}

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model:  &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embedding: Vec<f32>,
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    #[tracing::instrument(skip(self, text), fields(model = %self.model_name, text_len = text.len()))]
    async fn embed(&self, text: &str) -> Result<Embedding, MatchingError> {
        let url = format!("{}/api/embeddings", self.base_url);
        let body = OllamaEmbedRequest { model: &self.model_name, prompt: text };

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(MatchingError::OllamaStatus { status, body: body_text });
        }

        let parsed: OllamaEmbedResponse = resp.json().await?;
        Ok(Embedding { model: self.model_enum.clone(), vector: parsed.embedding })
    }

    fn model(&self) -> EmbeddingModel { self.model_enum.clone() }
}
```

**Verification**: Integration test using `wiremock::MockServer` returning a canned
embedding vector; assert returned `Embedding.vector.len() == 768`.

---

#### Step 1.4 — Extend `JobRepository` with `EmbeddingRepository`

File: `lazyjob-core/src/persistence/jobs.rs`

Add `impl EmbeddingRepository for SqliteJobRepository`. Key query patterns:

```rust
// list_unembedded_job_ids
sqlx::query_scalar!(
    "SELECT id FROM jobs WHERE embedding IS NULL LIMIT ?",
    limit as i64
)
.fetch_all(&self.pool).await

// update_job_embedding — INSERT OR REPLACE approach:
sqlx::query!(
    "UPDATE jobs SET embedding = ?, embedding_model = ? WHERE id = ?",
    bytes, model, job_id.to_string()
)
.execute(&self.pool).await

// load_all_job_embeddings
sqlx::query_as!(
    EmbeddingRow,
    "SELECT id, embedding, embedding_model FROM jobs WHERE embedding IS NOT NULL"
)
.fetch_all(&self.pool).await

// update_match_score
sqlx::query!(
    "UPDATE jobs SET match_score = ? WHERE id = ?",
    score, job_id.to_string()
)
.execute(&self.pool).await
```

For `life_sheet_embeddings` table:
```rust
// save_life_sheet_embedding — upsert by model
sqlx::query!(
    "INSERT INTO life_sheet_embeddings (profile_hash, model, embedding)
     VALUES (?, ?, ?)
     ON CONFLICT(model) DO UPDATE SET
       profile_hash = excluded.profile_hash,
       embedding    = excluded.embedding,
       created_at   = datetime('now')",
    profile_hash, model, bytes
)
.execute(&self.pool).await
```

**Verification**: `#[sqlx::test(migrations = "migrations")]` test; insert a job, call
`update_job_embedding`, then `list_unembedded_job_ids` — assert the updated job no longer
appears.

---

### Phase 2 — MatchScorer: Batch Cosine Similarity

#### Step 2.1 — LifeSheet-to-Text Conversion

File: `lazyjob-core/src/persistence/life_sheet.rs` (new method: `to_embedding_text`)

The LifeSheet must be serialized to a flat text string that captures all skills,
experience descriptions, and education for embedding. Format:

```
Skills: Rust, Python, Kubernetes, distributed systems, technical leadership
Experience: Senior Software Engineer at Acme Corp (2021-2024): Built a distributed job scheduling system handling 1M jobs/day. Led a team of 6 engineers. Introduced gRPC-based service mesh.
Experience: Software Engineer at Widgets Inc (2018-2021): Developed REST APIs in Python/Django. Reduced query latency by 40% via PostgreSQL index optimization.
Education: BSc Computer Science, University of Wherever (2014-2018)
Certifications: AWS Solutions Architect Associate
Target roles: Senior Software Engineer, Staff Engineer
Location preference: Remote, San Francisco Bay Area
```

Key decisions:
- Skills come first (highest semantic weight in embedding models)
- Experience entries include company name, title, and full description text
- Target roles are included so the embedding represents what the user *wants*, not just what they *have*
- NO PII like email/phone — not needed for semantic matching

```rust
// lazyjob-core/src/life_sheet/embedding_text.rs

pub fn life_sheet_to_embedding_text(sheet: &LifeSheet) -> String {
    let mut parts: Vec<String> = Vec::new();

    if !sheet.skills.is_empty() {
        parts.push(format!("Skills: {}", sheet.skills.join(", ")));
    }

    for exp in &sheet.experience {
        let entry = format!(
            "Experience: {} at {} ({}-{}): {}",
            exp.title,
            exp.company,
            exp.start_year,
            exp.end_year.map_or_else(|| "present".to_string(), |y| y.to_string()),
            exp.description.as_deref().unwrap_or("")
        );
        parts.push(entry);
    }

    for edu in &sheet.education {
        parts.push(format!(
            "Education: {}, {} ({})",
            edu.degree, edu.institution, edu.year
        ));
    }

    for cert in &sheet.certifications {
        parts.push(format!("Certification: {}", cert));
    }

    if !sheet.target_roles.is_empty() {
        parts.push(format!("Target roles: {}", sheet.target_roles.join(", ")));
    }

    if let Some(ref loc) = sheet.location_preference {
        parts.push(format!("Location: {}", loc));
    }

    parts.join("\n")
}

/// Stable SHA-256 hash of the embedding text, used as cache key.
pub fn life_sheet_profile_hash(text: &str) -> String {
    use sha2::{Sha256, Digest};
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(digest)
}
```

**Verification**: Unit test: construct a minimal `LifeSheet`, call `life_sheet_to_embedding_text`,
assert output contains expected substrings; assert `profile_hash` changes when `skills` changes.

---

#### Step 2.2 — Implement `MatchScorer`

File: `lazyjob-core/src/matching/scorer.rs`

```rust
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{info, warn, debug};
use uuid::Uuid;
use super::{
    embedder::Embedder,
    error::MatchingError,
    types::{Embedding, EmbeddingModel, ScoringReport},
};
use crate::{
    life_sheet::{LifeSheet, embedding_text::life_sheet_to_embedding_text},
    persistence::jobs::EmbeddingRepository,
};

pub struct MatchScorer {
    embedder:  Arc<dyn Embedder>,
    job_repo:  Arc<dyn EmbeddingRepository>,
}

impl MatchScorer {
    pub fn new(embedder: Arc<dyn Embedder>, job_repo: Arc<dyn EmbeddingRepository>) -> Self {
        Self { embedder, job_repo }
    }

    /// Embed all jobs that have no current embedding (or stale model), in batches of 50.
    #[tracing::instrument(skip(self))]
    pub async fn embed_new_jobs(&self) -> Result<usize, MatchingError> {
        const BATCH: usize = 50;
        let model_str = self.embedder.model().to_db_str();

        // Find jobs missing embedding OR with a different model
        let unembedded = self.job_repo.list_unembedded_job_ids(BATCH).await?;
        let stale      = self.job_repo.list_stale_embedding_ids(model_str, BATCH).await?;
        let ids_to_embed: Vec<Uuid> = [unembedded, stale].concat()
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let mut count = 0usize;
        for job_id in &ids_to_embed {
            let Some(text) = self.job_repo.get_job_text(*job_id).await? else {
                warn!(%job_id, "job text not found, skipping embedding");
                continue;
            };
            match self.embedder.embed(&text).await {
                Ok(embedding) => {
                    self.job_repo
                        .update_job_embedding(*job_id, model_str, &embedding.to_bytes())
                        .await?;
                    count += 1;
                }
                Err(e) => {
                    warn!(%job_id, error = %e, "embedding failed, skipping");
                }
            }
        }
        info!(count, "embedded jobs");
        Ok(count)
    }

    /// Generate (or load from cache) the LifeSheet embedding.
    #[tracing::instrument(skip(self, sheet))]
    pub async fn embed_life_sheet(&self, sheet: &LifeSheet) -> Result<Embedding, MatchingError> {
        use crate::life_sheet::embedding_text::{life_sheet_to_embedding_text, life_sheet_profile_hash};

        let text  = life_sheet_to_embedding_text(sheet);
        let hash  = life_sheet_profile_hash(&text);
        let model = self.embedder.model().to_db_str();

        // Check if we have a fresh cached embedding with the same hash and model
        if let Some((bytes, cached_model, cached_hash)) =
            self.job_repo.load_life_sheet_embedding().await?
        {
            if cached_model == model && cached_hash == hash {
                debug!("life sheet embedding cache hit");
                return Embedding::from_bytes(&bytes, self.embedder.model());
            }
        }

        let embedding = self.embedder.embed(&text).await?;
        self.job_repo
            .save_life_sheet_embedding(&hash, model, &embedding.to_bytes())
            .await?;
        Ok(embedding)
    }

    /// Batch-score all jobs with stored embeddings against `profile_embedding`.
    /// Writes `match_score` back to the DB. Returns a `ScoringReport`.
    #[tracing::instrument(skip(self, profile_embedding))]
    pub async fn score_all_jobs(
        &self,
        profile_embedding: &Embedding,
    ) -> Result<ScoringReport, MatchingError> {
        let start = Instant::now();
        let rows  = self.job_repo.load_all_job_embeddings().await?;

        let model_str = self.embedder.model().to_db_str();
        let mut report = ScoringReport::default();

        for (job_id, bytes, model) in &rows {
            if model != model_str {
                report.jobs_skipped += 1;
                continue;
            }
            match Embedding::from_bytes(bytes, self.embedder.model()) {
                Ok(job_emb) => {
                    let score = profile_embedding.cosine_similarity(&job_emb);
                    if let Err(e) = self.job_repo.update_match_score(*job_id, score).await {
                        warn!(%job_id, error = %e, "failed to write match_score");
                        report.jobs_errors += 1;
                    } else {
                        report.jobs_scored += 1;
                    }
                }
                Err(e) => {
                    warn!(%job_id, error = %e, "invalid embedding bytes");
                    report.jobs_errors += 1;
                }
            }
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            scored  = report.jobs_scored,
            skipped = report.jobs_skipped,
            errors  = report.jobs_errors,
            elapsed_ms = report.elapsed_ms,
            "scoring complete"
        );
        Ok(report)
    }

    /// Score a single job against a profile embedding. Pure function; no I/O.
    pub fn score_one(job_embedding: &Embedding, profile_embedding: &Embedding) -> f32 {
        profile_embedding.cosine_similarity(job_embedding)
    }
}
```

**Verification**:
- `test_score_one_identical`: embed("hello") cosine with itself == 1.0 (approx)
- `test_score_all_jobs_empty_db`: on empty DB, returns `ScoringReport { jobs_scored: 0 }`
- Integration test: insert 3 jobs with canned byte embeddings, call `score_all_jobs`,
  verify all 3 records have `match_score IS NOT NULL`.

---

### Phase 3 — FeedRanker

#### Step 3.1 — Implement `FeedRanker`

File: `lazyjob-core/src/matching/feed.rs`

```rust
use super::types::{ScoredJob, FeedbackSignal};

pub struct FeedRanker;

impl FeedRanker {
    /// Compute the final feed ranking score.
    ///
    /// Formula: `match_score * (1 - ghost_score) * recency_decay * feedback_multiplier`
    ///
    /// - `match_score`    ∈ [0.0, 1.0] — cosine similarity; defaults to 0.5 if NULL (unknown)
    /// - `ghost_score`    ∈ [0.0, 1.0] — 0.0 if ghost detection unavailable
    /// - `recency_decay`  = exp(-days_since_posted / 30)
    /// - `feedback_mult`  ∈ {0.5, 1.0, 1.2} → clamped to [0.0, 1.0] after multiplication
    pub fn compute_feed_score(job: &ScoredJob) -> f32 {
        // Unknown match score: treat as neutral (0.5) rather than zero, so unscored
        // jobs appear in the middle of the feed rather than being buried.
        let match_score = if job.match_score == 0.0 { 0.5 } else { job.match_score };

        let ghost_penalty = 1.0_f32 - job.ghost_score.max(0.0).min(1.0);
        let recency = (-job.days_since_posted / 30.0_f32).exp();
        let feedback = job.feedback.multiplier();

        (match_score * ghost_penalty * recency * feedback).max(0.0).min(1.0)
    }

    /// Sort a slice of `ScoredJob` by `feed_score` descending in-place.
    pub fn rank_jobs(jobs: &mut [ScoredJob]) {
        jobs.sort_by(|a, b| {
            let sa = Self::compute_feed_score(a);
            let sb = Self::compute_feed_score(b);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}
```

**Design notes**:
- Jobs with no match score (Ollama unavailable) receive `match_score = 0.5` — they appear
  in the middle of the feed rather than last. This matches the spec's fallback to
  reverse-chronological when matching is unavailable.
- `feedback_multiplier` for `Saved` is 1.2 but the per-multiplier application means
  `0.5 * 1.0 * 1.0 * 1.2 = 0.6` — well within [0.0, 1.0] when other factors are moderate.
  The final `.min(1.0)` ensures no overflow.

**Verification**:
- `test_recency_decay_30_days`: `compute_feed_score` with `days_since_posted = 30.0`,
  `match_score = 1.0`, `ghost_score = 0.0`, neutral → `exp(-1) ≈ 0.368`
- `test_dismissed_job_demoted`: saved job ranks above identical dismissed job
- `test_ghost_job_penalized`: job with `ghost_score = 0.9` ranks below fresh job with
  `ghost_score = 0.0` even if match_score is higher

---

#### Step 3.2 — Feed Score Persistence

Extend `jobs` table with `feed_score REAL` (already in migration 011) and add a
`update_feed_score` method to `SqliteJobRepository`.

`MatchingService` writes `feed_score` after computing it so the TUI can sort purely
from SQLite without recomputing in the render loop:

```sql
UPDATE jobs
SET feed_score = ?
WHERE id = ?
```

The TUI `JobsFeedView` sorts by `ORDER BY feed_score DESC NULLS LAST, posted_at DESC`.

---

### Phase 4 — SkillInferenceEngine (Optional, Config-Gated)

#### Step 4.1 — LLM Prompt for Skill Extraction

File: `lazyjob-core/src/matching/skill_inference.rs`

The prompt instructs the LLM to return a JSON array of `{"id", "label", "confidence"}`
objects. We use `serde_json::from_str` to parse the response.

```rust
use std::sync::Arc;
use sha2::{Sha256, Digest};
use hex;
use serde_json;
use crate::llm::LlmProvider;
use super::{error::MatchingError, types::{EscoSkill, SkillSource}};

const SKILL_EXTRACTION_PROMPT: &str = r#"You are a skills extraction assistant.
Given the following work experience description, identify all technical and professional skills mentioned or strongly implied.
For each skill, output a JSON array entry with:
  - "id": an ESCO skill URI (use "esco:unknown" if unsure)
  - "label": the canonical skill name in English
  - "confidence": a float 0.0–1.0

Return ONLY the JSON array, no other text.

Experience: {experience_text}
"#;

pub struct SkillInferenceEngine {
    llm: Arc<dyn LlmProvider>,
}

impl SkillInferenceEngine {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self { Self { llm } }

    /// Compute a stable cache key for an experience text + model pair.
    fn cache_key(text: &str, model_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        hasher.update(b"|");
        hasher.update(model_id.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Extract ESCO-aligned skills from a free-text experience description.
    /// Returns up to 20 skills with confidence >= 0.5.
    #[tracing::instrument(skip(self, text))]
    pub async fn infer_skills(&self, text: &str) -> Result<Vec<EscoSkill>, MatchingError> {
        let prompt = SKILL_EXTRACTION_PROMPT.replace("{experience_text}", text);
        let response = self.llm
            .chat_completion(&[crate::llm::ChatMessage::user(prompt)])
            .await
            .map_err(|e| MatchingError::SkillInference(e.to_string()))?;

        let raw_json = response.content.trim();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(raw_json)
            .map_err(|e| MatchingError::SkillInference(
                format!("LLM returned invalid JSON: {e}: {raw_json}")
            ))?;

        let skills: Vec<EscoSkill> = parsed.into_iter().filter_map(|v| {
            let id         = v["id"].as_str()?.to_string();
            let label      = v["label"].as_str()?.to_string();
            let confidence = v["confidence"].as_f64()? as f32;
            if confidence < 0.5 { return None; }
            Some(EscoSkill { id, label, confidence, source: SkillSource::LlmInferred })
        }).take(20).collect();

        Ok(skills)
    }

    /// Augment LifeSheet embedding text by appending inferred skills.
    /// Called only when `[matching] esco_inference = true` in config.
    pub fn augment_life_sheet_text(
        base_text: &str,
        inferred: &[EscoSkill],
    ) -> String {
        if inferred.is_empty() { return base_text.to_string(); }
        let labels: Vec<&str> = inferred.iter().map(|s| s.label.as_str()).collect();
        format!("{}\nInferred skills: {}", base_text, labels.join(", "))
    }
}
```

#### Step 4.2 — ESCO Inference Cache (SQLite)

`EscoInferenceRepository` methods on `SqliteJobRepository`:

```rust
// Check cache before calling LLM
async fn get_cached_inference(
    &self,
    text_hash: &str,
    llm_model:  &str,
) -> Result<Option<Vec<EscoSkill>>, anyhow::Error>

// Store after successful LLM call
async fn cache_inference(
    &self,
    text_hash:  &str,
    llm_model:  &str,
    skills:     &[EscoSkill],
) -> Result<(), anyhow::Error>
```

Query:
```sql
-- get_cached_inference
SELECT inferred_skills FROM esco_inference_cache
WHERE text_hash = ? AND llm_model = ?
LIMIT 1

-- cache_inference
INSERT INTO esco_inference_cache (text_hash, llm_model, inferred_skills)
VALUES (?, ?, ?)
ON CONFLICT(text_hash, llm_model) DO UPDATE SET inferred_skills = excluded.inferred_skills
```

**Integration**: `SkillInferenceService` (a thin wrapper around `SkillInferenceEngine` + cache repo)
checks cache first; on miss, calls LLM and stores. Called once per experience entry whose
`text_hash` is absent from cache. A full LifeSheet re-inference only touches entries whose
text changed.

---

### Phase 5 — `MatchingService` Top-Level Orchestrator

File: `lazyjob-core/src/matching/service.rs`

This is the entry point called by the ralph discovery loop.

```rust
use std::sync::Arc;
use tracing::{info, warn};
use crate::{
    config::MatchingConfig,
    life_sheet::LifeSheetRepository,
    matching::{
        embedder::Embedder,
        scorer::MatchScorer,
        feed::FeedRanker,
        skill_inference::SkillInferenceEngine,
        types::ScoredJob,
    },
    persistence::jobs::{EmbeddingRepository, JobRepository},
};
use super::error::MatchingError;

pub struct MatchingService {
    scorer:           MatchScorer,
    life_sheet_repo:  Arc<dyn LifeSheetRepository>,
    job_repo:         Arc<dyn JobRepository>,
    config:           MatchingConfig,
    skill_engine:     Option<SkillInferenceEngine>,  // None if esco_inference = false
}

impl MatchingService {
    pub fn new(
        embedder:        Arc<dyn Embedder>,
        embedding_repo:  Arc<dyn EmbeddingRepository>,
        life_sheet_repo: Arc<dyn LifeSheetRepository>,
        job_repo:        Arc<dyn JobRepository>,
        config:          MatchingConfig,
        skill_engine:    Option<SkillInferenceEngine>,
    ) -> Self {
        Self {
            scorer: MatchScorer::new(embedder, embedding_repo),
            life_sheet_repo,
            job_repo,
            config,
            skill_engine,
        }
    }

    /// Full post-discovery pipeline:
    /// 1. Embed new/stale jobs
    /// 2. Embed LifeSheet (possibly augmented with ESCO inference)
    /// 3. Score all jobs
    /// 4. Compute and persist feed_score for all scored jobs
    #[tracing::instrument(skip(self))]
    pub async fn run_post_discovery(&self) -> Result<(), MatchingError> {
        // Step 1: embed new jobs
        let embedded = self.scorer.embed_new_jobs().await?;
        info!(embedded, "step 1: job embedding complete");

        // Step 2: build LifeSheet embedding text
        let sheet = self.life_sheet_repo
            .load()
            .await
            .map_err(MatchingError::Database)?;

        let base_text = crate::life_sheet::embedding_text::life_sheet_to_embedding_text(&sheet);
        let embedding_text = if self.config.esco_inference {
            if let Some(ref engine) = self.skill_engine {
                let augmented = self.augment_with_esco(&sheet, engine).await;
                augmented.unwrap_or_else(|e| {
                    warn!(error = %e, "ESCO inference failed, using base text");
                    base_text.clone()
                })
            } else {
                base_text
            }
        } else {
            base_text
        };

        let profile_embedding = self.scorer
            .embed_life_sheet_from_text(&embedding_text)
            .await?;
        info!("step 2: life sheet embedded");

        // Step 3: batch score all jobs
        let report = self.scorer.score_all_jobs(&profile_embedding).await?;
        info!(
            scored = report.jobs_scored,
            elapsed_ms = report.elapsed_ms,
            "step 3: scoring complete"
        );

        // Step 4: compute and persist feed_score for all scored jobs
        self.persist_feed_scores().await?;
        info!("step 4: feed scores persisted");

        Ok(())
    }

    async fn augment_with_esco(
        &self,
        sheet:  &crate::life_sheet::LifeSheet,
        engine: &SkillInferenceEngine,
    ) -> Result<String, MatchingError> {
        use crate::life_sheet::embedding_text::life_sheet_to_embedding_text;
        let base = life_sheet_to_embedding_text(sheet);
        let mut all_inferred = Vec::new();
        for exp in &sheet.experience {
            let text = exp.description.as_deref().unwrap_or("");
            if text.is_empty() { continue; }
            let skills = engine.infer_skills(text).await?;
            all_inferred.extend(skills);
        }
        Ok(SkillInferenceEngine::augment_life_sheet_text(&base, &all_inferred))
    }

    async fn persist_feed_scores(&self) -> Result<(), MatchingError> {
        use chrono::Utc;
        let jobs = self.job_repo
            .list_scored_jobs()
            .await
            .map_err(MatchingError::Database)?;

        for j in jobs {
            let days = (Utc::now() - j.posted_at).num_days().max(0) as f32;
            let scored = ScoredJob {
                job_id:            j.id,
                match_score:       j.match_score.unwrap_or(0.0),
                ghost_score:       j.ghost_score.unwrap_or(0.0),
                days_since_posted: days,
                feedback:          j.feedback_signal.into(),
            };
            let feed_score = FeedRanker::compute_feed_score(&scored);
            self.job_repo
                .update_feed_score(j.id, feed_score)
                .await
                .map_err(MatchingError::Database)?;
        }
        Ok(())
    }
}
```

**Config struct** in `lazyjob-core/src/config.rs`:

```rust
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct MatchingConfig {
    /// Embedding model to use. Default: OllamaNomic768.
    #[serde(default = "default_embedding_model")]
    pub embedding_model: EmbeddingModel,

    /// Ollama base URL. Default: "http://localhost:11434".
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    /// Enable ESCO skill inference via LLM. Default: false.
    #[serde(default)]
    pub esco_inference: bool,
}
```

---

### Phase 6 — TUI Integration

#### Step 6.1 — Feed Sorting in `JobsFeedView`

File: `lazyjob-tui/src/views/jobs_feed.rs`

The TUI reads `feed_score` from SQLite — it does NOT call the embedder or scorer directly.
The feed is sorted by `ORDER BY feed_score DESC NULLS LAST, posted_at DESC` in the
repository query. When `feed_score IS NULL` (Ollama unavailable), jobs appear sorted by
date (reverse-chronological), satisfying the graceful degradation requirement.

```rust
// In SqliteJobRepository
pub async fn list_feed_jobs(&self, limit: usize) -> Result<Vec<FeedJobRow>, anyhow::Error> {
    sqlx::query_as!(
        FeedJobRow,
        r#"
        SELECT
            id,
            title,
            company_name,
            location,
            salary_min,
            salary_max,
            remote_type,
            match_score,
            feed_score,
            status,
            posted_at
        FROM jobs
        WHERE status NOT IN ('dismissed')
        ORDER BY feed_score DESC NULLS LAST, posted_at DESC
        LIMIT ?
        "#,
        limit as i64
    )
    .fetch_all(&self.pool)
    .await
}
```

#### Step 6.2 — Match Score Display in Job Detail Panel

In the job detail panel (`lazyjob-tui/src/views/job_detail.rs`), render the match score
as a percentage bar (0-100%) using `ratatui::widgets::Gauge` or a text sparkline:

```
Match:  ████████████░░░░░░░░  62%
Ghost:  ██░░░░░░░░░░░░░░░░░░  8%
```

Show "Match: N/A" if `match_score IS NULL` with a tooltip: "Install Ollama to enable semantic matching".

#### Step 6.3 — Matching Status in Status Bar

In the TUI status bar (`lazyjob-tui/src/views/status_bar.rs`), show a one-character
indicator:

| Condition | Indicator | Meaning |
|---|---|---|
| Ollama running, scoring done | `●` (green) | Matching active |
| Ollama running, scoring in progress | `◌` (yellow) | Scoring… |
| Ollama unavailable | `○` (gray) | Matching unavailable |

This is driven by a `MatchingStatus` value stored in `App` state, updated via the
`tokio::sync::watch` channel from `MatchingService`.

---

### Phase 7 — Model Migration (Embedding Dimension Change)

When the user changes `[matching] embedding_model` in config, stored embeddings become
incompatible (different dimensions). The migration strategy:

1. On startup, `MatchingService::check_model_compatibility()` compares the configured
   model to `life_sheet_embeddings.model` and the most common `jobs.embedding_model`.
2. If a model mismatch is detected, log a warning: "Embedding model changed. Re-embedding
   all jobs. This may take a few minutes."
3. Set ALL `jobs.embedding = NULL, jobs.match_score = NULL, jobs.feed_score = NULL`
   in a single `UPDATE jobs SET embedding = NULL ...` transaction.
4. Delete the `life_sheet_embeddings` row for the old model.
5. The next `run_post_discovery()` call will re-embed everything from scratch.

```rust
// lazyjob-core/src/matching/service.rs

pub async fn check_and_migrate_model(&self) -> Result<(), MatchingError> {
    let current_model = self.scorer.embedder.model().to_db_str();
    let stored_model  = self.get_active_embedding_model().await?;

    if stored_model.as_deref() == Some(current_model) {
        return Ok(()); // no migration needed
    }

    if stored_model.is_some() {
        tracing::warn!(
            from = ?stored_model,
            to   = %current_model,
            "embedding model changed — clearing all stored embeddings"
        );
        self.job_repo.clear_all_embeddings().await.map_err(MatchingError::Database)?;
    }
    Ok(())
}
```

`JobRepository::clear_all_embeddings()`:
```sql
UPDATE jobs SET embedding = NULL, embedding_model = NULL, match_score = NULL, feed_score = NULL
```

---

## Key Crate APIs

- `reqwest::Client::post(url).json(&body).send().await` — POST to Ollama `/api/embeddings`
- `serde_json::from_str::<Vec<Value>>(&raw)` — parse LLM skill extraction response
- `bytemuck::cast_slice::<f32, u8>(vec)` — `Vec<f32>` → `&[u8]` for BLOB storage
- `bytemuck::cast_slice::<u8, f32>(bytes)` — `&[u8]` → `Vec<f32>` from BLOB
- `sha2::Sha256::digest(text)` — SHA-256 hash for LifeSheet and inference cache keys
- `hex::encode(digest)` — hex-encode SHA-256 bytes for TEXT storage in SQLite
- `(-days / 30.0_f32).exp()` — standard library `f32::exp()`, no external crate needed
- `sqlx::query_as!` macro for typed SQLite queries with compile-time checking
- `tokio::sync::watch::channel::<MatchingStatus>()` — TUI watches for matching progress updates
- `tracing::instrument(skip(self, text))` — instrument public methods, skip large parameters

## Error Handling

```rust
// lazyjob-core/src/matching/error.rs

#[derive(thiserror::Error, Debug)]
pub enum MatchingError {
    #[error("Ollama request failed: {0}")]
    OllamaRequest(#[from] reqwest::Error),

    #[error("Ollama returned status {status}: {body}")]
    OllamaStatus { status: u16, body: String },

    #[error("OpenAI embedding request failed: {0}")]
    OpenAiRequest(String),

    #[error("Invalid embedding bytes: length {0} not divisible by 4")]
    InvalidEmbeddingBytes(usize),

    #[error("Embedding model mismatch: stored={stored}, current={current}")]
    ModelMismatch { stored: String, current: String },

    #[error("Embedder unavailable: {0}")]
    EmbedderUnavailable(String),

    #[error("Skill inference failed: {0}")]
    SkillInference(String),

    #[error("Database error: {0}")]
    Database(#[from] anyhow::Error),
}
```

**Graceful degradation policy**:
- `OllamaRequest` during `embed_new_jobs` → log warn, skip that job (don't fail the whole batch)
- `OllamaRequest` during `embed_life_sheet` → propagate; abort the scoring pass
- `EmbedderUnavailable` on startup → set `MatchingStatus::Unavailable`, TUI shows "○ Matching disabled"
- `SkillInference` failure → log warn, proceed with base LifeSheet text (no ESCO augmentation)
- `ModelMismatch` → trigger migration flow (clear + re-embed), not a fatal error

## Testing Strategy

### Unit Tests

**`test_embedding_roundtrip`** (`src/matching/types.rs`):
```rust
#[test]
fn test_embedding_roundtrip() {
    let original = Embedding { model: EmbeddingModel::OllamaNomic768, vector: vec![0.1, -0.5, 0.9] };
    let bytes = original.to_bytes();
    let restored = Embedding::from_bytes(&bytes, EmbeddingModel::OllamaNomic768).unwrap();
    assert_eq!(original.vector, restored.vector);
}
```

**`test_cosine_similarity_identical`**:
```rust
#[test]
fn test_cosine_similarity_identical() {
    let e = Embedding { model: EmbeddingModel::OllamaNomic768, vector: vec![1.0, 0.0, 0.0] };
    assert!((e.cosine_similarity(&e) - 1.0).abs() < 1e-6);
}
```

**`test_cosine_similarity_orthogonal`**:
```rust
#[test]
fn test_cosine_similarity_orthogonal() {
    let a = Embedding { model: EmbeddingModel::OllamaNomic768, vector: vec![1.0, 0.0] };
    let b = Embedding { model: EmbeddingModel::OllamaNomic768, vector: vec![0.0, 1.0] };
    assert_eq!(a.cosine_similarity(&b), 0.0);
}
```

**`test_cosine_zero_norm`**: assert no panic, returns 0.0.

**`test_feed_ranker_dismissed_demoted`**:
```rust
#[test]
fn test_feed_ranker_dismissed_demoted() {
    let saved     = ScoredJob { match_score: 0.8, feedback: FeedbackSignal::Saved, ghost_score: 0.0, days_since_posted: 5.0, job_id: Uuid::new_v4() };
    let dismissed = ScoredJob { match_score: 0.8, feedback: FeedbackSignal::Dismissed, ghost_score: 0.0, days_since_posted: 5.0, job_id: Uuid::new_v4() };
    assert!(FeedRanker::compute_feed_score(&saved) > FeedRanker::compute_feed_score(&dismissed));
}
```

**`test_life_sheet_profile_hash_changes_on_skills_change`**:
```rust
#[test]
fn test_life_sheet_profile_hash_stable() {
    let text = "Skills: Rust, Python";
    let h1 = life_sheet_profile_hash(text);
    let h2 = life_sheet_profile_hash(text);
    assert_eq!(h1, h2);

    let h3 = life_sheet_profile_hash("Skills: Rust, Go");
    assert_ne!(h1, h3);
}
```

### Integration Tests (using `#[sqlx::test]`)

**`test_embed_new_jobs_inserts_embeddings`**:
- Insert 3 jobs into in-memory SQLite (all with `embedding IS NULL`)
- Construct `MockEmbedder` returning deterministic 768-dim vectors
- Call `scorer.embed_new_jobs()`
- Assert: `SELECT COUNT(*) FROM jobs WHERE embedding IS NOT NULL` == 3

**`test_score_all_jobs_writes_scores`**:
- Insert 3 jobs with pre-computed canned embeddings (all same model)
- Construct a profile embedding
- Call `scorer.score_all_jobs(&profile_embedding)`
- Assert: all 3 jobs have `match_score IS NOT NULL`; scores ∈ [0.0, 1.0]

**`test_life_sheet_embedding_cache_hit`**:
- Generate LifeSheet embedding (calls MockEmbedder once)
- Call `embed_life_sheet` again with identical LifeSheet
- Assert MockEmbedder was called exactly ONCE (second call hit cache)

### Wiremock Integration Tests (OllamaEmbedder)

**`test_ollama_embedder_success`**:
```rust
#[tokio::test]
async fn test_ollama_embedder_success() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .and(path("/api/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "embedding": vec![0.1f32; 768]
        })))
        .mount(&mock_server)
        .await;

    let embedder = OllamaEmbedder::with_base_url(EmbeddingModel::OllamaNomic768, mock_server.uri());
    let emb = embedder.embed("test text").await.unwrap();
    assert_eq!(emb.vector.len(), 768);
}
```

**`test_ollama_embedder_503_returns_err`**: wiremock returns 503; assert `MatchingError::OllamaStatus { status: 503, .. }`.

### TUI Tests

The TUI feed view does not call the embedder; it only reads pre-computed `feed_score` from
SQLite. Test the sort order via `list_feed_jobs()` with canned data:
- Insert 3 jobs with different `feed_score` values
- Call `list_feed_jobs(10)`
- Assert returned slice is sorted descending by `feed_score`

## Open Questions

1. **Ollama unavailability warning**: Should the TUI show a startup modal ("Ollama not
   found — semantic matching disabled") or a silent status bar indicator? Recommend the
   status bar indicator (less intrusive) with a `:matching-help` command explaining
   installation.

2. **Re-scoring trigger**: After a LifeSheet edit, should re-scoring happen synchronously
   (blocking TUI briefly, < 5ms at 5000 jobs) or via a ralph subprocess? Recommend
   dispatching to ralph to keep TUI snappy, noting that the delay is negligible in practice.

3. **Feedback loop fidelity**: `Dismissed` is treated as a fit signal, but users may dismiss
   due to bad company, too senior, or wrong location. Consider a dismissal reason enum
   (`DismissReason::BadFit | TooSenior | WrongLocation | BadCompany`) and only apply
   negative signal for `BadFit`.

4. **`candle` vs `fastembed`**: The spec mentions `candle` or `fastembed` as in-process
   embedding options. Phase 1 relies on Ollama as a separate process. A future Phase 4
   could add `fastembed-rs` (ONNX-based, no separate server required) as an embedded
   fallback. This would eliminate the Ollama dependency for offline-first operation.
   Decision deferred to after Phase 3 ships.

5. **Dimension-checked trait**: Currently `cosine_similarity` uses `debug_assert_eq!` for
   dimension checking. A future improvement is a const-generic `Embedding<N: usize>` type
   that enforces dimension matching at compile time, eliminating the runtime check entirely.
   Deferred due to ergonomic complexity with trait objects.

6. **Negative sampling / contrastive fine-tuning**: The spec mentions "contrastive learning
   pass to fine-tune the profile representation using saved vs. dismissed jobs" as a future
   direction. This requires storing dismissed job embeddings in a `negative_examples` table
   and running a gradient update step (not practical without `candle` integration). Deferred
   to a Phase 5+ extension.

## Related Specs

- `specs/job-search-discovery-engine.md` — produces the `DiscoveredJob` records scored here
- `specs/job-search-ghost-job-detection.md` — provides `ghost_score` used in `FeedRanker`
- `specs/profile-life-sheet-data-model.md` — defines `LifeSheet` struct consumed by `embed_life_sheet`
- `specs/agentic-llm-provider-abstraction.md` — `LlmProvider` trait used by `SkillInferenceEngine`
- `specs/agentic-ralph-orchestration.md` — ralph loop that triggers `MatchingService::run_post_discovery()`
- `specs/09-tui-design-keybindings-implementation-plan.md` — `App` state + status bar integration
