# Research: Task 25 — semantic-matching

## Task Description
Implement MatchScorer (cosine similarity via EmbeddingProvider), job_embeddings migration, and GhostDetector (7-signal heuristic).

## Key Findings

### Circular Dependency Issue
`lazyjob-llm` already depends on `lazyjob-core`. Adding the reverse would create a circular dependency. Solution: define a local `Embedder` trait in `lazyjob-core::discovery::matching` with the same `embed(&str) -> Result<Vec<f32>>` signature. When the CLI/TUI needs to wire a real embedding provider, they pass an adapter.

### Existing Schema
- `jobs.match_score DOUBLE PRECISION` — already in migration 001
- `jobs.ghost_score DOUBLE PRECISION` — already in migration 001
- No `job_embeddings` table yet — need migration 003

### Job Domain Type
Key fields for GhostDetector:
- `title: String` — detect generic titles
- `company_name: Option<String>` — None = no company signal
- `salary_min/max: Option<i64>` — None = salary missing signal
- `url: Option<String>` — URL pattern check
- `discovered_at: DateTime<Utc>` — proxy for posting age

### Life Sheet Fields for Embedding
For building profile text: `basics.summary`, `work_experience[].tech_stack`, `work_experience[].achievements[].description`, `skills[].skills[].name`, `certifications[].name`

### Embedding Storage
Store as BYTEA in `job_embeddings` table. Serialize `Vec<f32>` as raw little-endian bytes (bytemuck-style transmutation using safe stdlib iteration).

### No New Dependencies Needed
- `async_trait` already in workspace
- `Arc` is std
- Cosine similarity is pure math, no external crate needed
