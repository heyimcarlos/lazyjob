# Research: Task 23 — job-sources

## Summary
Task 23 requires implementing `GreenhouseClient` and `LeverClient` in `lazyjob-core/src/discovery/sources/`.
Each client fetches job listings from the respective public APIs (no auth), strips HTML with `ammonia`,
maps results to the `Job` domain type, and stores into PostgreSQL. Per-source `RateLimiter` (1 req/s
Greenhouse, 2 req/s Lever) is required.

---

## Existing Codebase State

### `Job` domain type (lazyjob-core/src/domain/job.rs)
Fields directly needed:
- `id: JobId` — UUID newtype, set by `Job::new(title)` 
- `title: String`
- `company_name: Option<String>` — board_token / company_id used here
- `location: Option<String>`
- `url: Option<String>`
- `description: Option<String>` — HTML stripped to plain text
- `source: Option<String>` — `"greenhouse"` or `"lever"`
- `source_id: Option<String>` — external ID from source
- `discovered_at: DateTime<Utc>` — set to `Utc::now()` on construction

### `jobs` table (migrations/001_initial_schema.sql)
- Has `source TEXT` and `source_id TEXT` columns with an index `idx_jobs_source`
- The index is NOT unique — deduplication requires adding a partial unique index

### `JobRepository` (lazyjob-core/src/repositories/job.rs)
- Has `insert()` — plain INSERT, no upsert
- Need to add `upsert_discovered()` with `ON CONFLICT(source, source_id) WHERE ... DO UPDATE`

### Workspace Dependencies Already Present
- `reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"] }` ✓
- `async-trait = "0.1"` ✓
- `tokio = { version = "1.0", features = ["full"] }` ✓

### Missing Dependencies
- `ammonia = "4"` — HTML sanitization (strip tags from job descriptions)
- `wiremock = "0.6"` — HTTP mocking for unit tests (dev-dep only)

---

## API Shapes

### Greenhouse Boards API
`GET https://boards-api.greenhouse.io/v1/boards/{board_token}/jobs?content=true`

```json
{
  "jobs": [
    {
      "id": 127817,
      "title": "Senior Software Engineer",
      "content": "<div><p>Build things with Rust</p></div>",
      "location": {"name": "San Francisco, CA"},
      "departments": [{"name": "Engineering", "id": 1}],
      "updated_at": "2024-01-15T10:30:00-05:00",
      "absolute_url": "https://boards.greenhouse.io/stripe/jobs/127817"
    }
  ]
}
```

### Lever Postings API
`GET https://api.lever.co/v0/postings/{company}?mode=json`

```json
[
  {
    "id": "abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a",
    "text": "Senior Software Engineer",
    "description": "<p>We are looking for a skilled engineer...</p>",
    "categories": {
      "location": "San Francisco, CA",
      "team": "Engineering",
      "commitment": "Full-time"
    },
    "createdAt": 1706123456000,
    "hostedUrl": "https://jobs.lever.co/notion/abe0c1ec-9ffe-4e00-9e71-4d3e63f3c45a"
  }
]
```

---

## Rate Limiter Design
- Token bucket simplified as "minimum interval enforcer"
- `RateLimiter { interval: Duration, last_call: Mutex<Option<Instant>> }`
- `wait(&self)` acquires lock → computes sleep duration → releases lock → awaits sleep
- Interior mutability via `std::sync::Mutex` (not async Mutex) since lock is held briefly, not across await

---

## HTML Stripping with ammonia
Using `ammonia::Builder::new().tags(HashSet::new()).clean(html).to_string()`
strips all HTML tags, keeping only text content.

---

## Migration 002
Add partial unique index for ON CONFLICT upsert:
```sql
CREATE UNIQUE INDEX idx_jobs_source_id_unique
    ON jobs(source, source_id)
    WHERE source IS NOT NULL AND source_id IS NOT NULL;
```
This allows `ON CONFLICT (source, source_id) WHERE source IS NOT NULL AND source_id IS NOT NULL DO UPDATE SET ...`

---

## Test Strategy
- **Learning test: ammonia_strips_html_tags** — proves `ammonia::Builder` with empty tags set returns plain text
- **Learning test: wiremock_responds_with_json** — proves `MockServer` intercepts HTTP and returns fixture body
- **Unit: greenhouse_parses_response** — wiremock mock of Greenhouse API, verify Job fields
- **Unit: lever_parses_response** — wiremock mock of Lever API, verify Job fields
- **Unit: rate_limiter_enforces_interval** — basic timing test
- **Unit: strip_html_various_inputs** — empty string, plain text, nested tags, entities
