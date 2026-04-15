# Job Discovery Layer

## Status
Researching

## Problem Statement

LazyJob needs to discover job opportunities from multiple sources:
1. **Job Boards**: Greenhouse, Lever, LinkedIn, Indeed, Glassdoor
2. **Direct Sources**: Company career pages (scraped or API-fed)
3. **Semantic Search**: Match jobs to user's skills/experience using embeddings

The discovery layer must:
- Aggregate jobs from multiple sources
- Deduplicate near-identical postings
- Enrich job data with company information
- Match jobs to user's life sheet profile
- Track discovered jobs in the database
- Avoid scraping (often against ToS, unreliable)

---

## Research Findings

### Greenhouse Job Board API

Greenhouse provides a public, authentication-free API for companies using their job board:

**Endpoint**: `GET https://boards-api.greenhouse.io/v1/boards/{board_token}/jobs`

**Parameters**:
- `content=true`: Include full job description, departments, offices

**Response** (truncated):
```json
{
  "jobs": [
    {
      "id": 127817,
      "title": "Senior Software Engineer",
      "location": { "name": "San Francisco, CA" },
      "absolute_url": "https://boards.greenhouse.io/company/jobs/127817",
      "departments": [{ "id": 13583, "name": "Engineering" }],
      "offices": [{ "id": 8787, "name": "San Francisco" }]
    }
  ],
  "meta": { "total": 1 }
}
```

**With content=true**:
```json
{
  "jobs": [{
    "id": 127817,
    "title": "Senior Software Engineer",
    "content": "<p>Job description HTML...</p>",
    "location": { "name": "San Francisco, CA" },
    "departments": [...],
    "offices": [...],
    "metadata": null
  }]
}
```

**Key Points**:
- Public - no API key required
- Rate limiting: Reasonable (avoid hammering)
- Job content includes HTML that needs sanitization
- Board token is typically the company name/identifier

### Lever API

Lever provides job board APIs for their customers:

**Endpoint**: `GET https://api.lever.co/v0/postings/{company}?mode=json`

**Response**:
```json
{
  "data": [
    {
      "id": "abc123",
      "title": "Software Engineer",
      "location": "San Francisco",
      "description": "<p>Job description...</p>",
      "lists": [],
      "categories": {
        "commitment": "Full-time",
        "team": "Engineering",
        "location": "San Francisco",
        "department": "Engineering"
      },
      "additional": {
        "description": "Plain text description",
        "requirements": "Requirements...",
        "benefits": "Benefits..."
      }
    }
  ]
}
```

**Key Points**:
- Public - no API key required for job listings
- Structured data (categories, commitment type, etc.)
- Plain text alternatives to HTML descriptions

### JobSpy (Scraping Library)

JobSpy is a Python library that scrapes LinkedIn, Indeed, and Glassdoor:

**Limitations**:
- LinkedIn scraping is against ToS and requires authentication
- Indeed may block automated access
- Glassdoor scraping is technically challenging
- Results are unreliable and change frequently

**Recommendation**: Do NOT rely on JobSpy for production. Use legitimate APIs instead.

### Semantic Embedding for Job Matching

**Concept**: Convert job descriptions and user profiles into vector embeddings, then compute similarity scores.

**Embedding Models**:
- OpenAI `text-embedding-ada-002` (1536 dimensions)
- OpenAI `text-embedding-3-small` (1536 dim, smaller)
- OpenAI `text-embedding-3-large` (3072 dim, best quality)
- Ollama: `nomic-embed-text` (768 dim, fast, local)
- Ollama: `mxbai-embed-large` (1024 dim, high quality)

**Vector Storage Options**:

| Option | Type | Pros | Cons |
|--------|------|------|------|
| **Chroma** | Dedicated Vector DB | Feature-rich, good API | Python-first, not Rust-native |
| **Qdrant** | Dedicated Vector DB | Rust-native, high performance | Requires separate service |
| **pgvector** | PostgreSQL extension | Already in stack, SQL integration | Adds PostgreSQL dependency |
| **In-memory** | Simple | No extra service, fast for small scale | No persistence, limited scale |
| **SQLite + math** | Simple | Local, persistent | Slower for large datasets |

**For LazyJob's scale** (~100s-1000s of jobs, single user):
- In-memory embedding similarity is sufficient
- No need for dedicated vector database
- SQLite can store embeddings as BLOBs or JSON

### Vector Similarity Calculation

**Cosine Similarity** (most common):
```rust
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm_a * norm_b)
}
```

**Top-K Selection**:
```rust
fn top_k<T>(scores: &[(T, f32)], k: usize) -> Vec<(T, f32)> {
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    sorted.into_iter().take(k).collect()
}
```

---

## Design Options

### Option A: API Aggregation Only (No Scraping)

**Description**: Support only legitimate job APIs (Greenhouse, Lever) and direct URL entry. No scraping.

**Pros**:
- Reliable, stable data sources
- No ToS violations
- Lower maintenance burden
- Easier to implement

**Cons**:
- Limited job coverage (only companies using these platforms)
- Missing many opportunities

**Best for**: MVP, production stability

### Option B: API + Controlled Scraping

**Description**: API aggregation plus scraping of company career pages with consent.

**Pros**:
- Broader job coverage
- Can discover jobs not on job boards
- Competitive advantage

**Cons**:
- Fragile (sites change structure)
- Legal complexity (ToS, robots.txt)
- Maintenance burden
- May require headless browser

**Best for**: Comprehensive job discovery (later phase)

### Option C: Community-Contributed Job Board

**Description**: Users contribute job listings, building a shared database.

**Pros**:
- No scraping needed
- Curated, high-quality data
- Viral growth potential

**Cons**:
- Chicken-and-egg problem
- Requires network effects
- Quality control challenges

**Best for**: Future growth, marketplace

---

## Recommended Approach

**Phase 1 (MVP)**: Option A - API Aggregation Only
- Support Greenhouse and Lever APIs
- Allow manual job entry (URL, title, company)
- In-memory or SQLite-stored embeddings for matching
- No scraping

**Phase 2**: Option B - Add controlled scraping via Playwright
- Target specific companies on request
- Respect robots.txt
- Headless browser for rendering JavaScript

---

## Architecture

### Source Registry

```rust
// lazyjob-core/src/discovery/sources/mod.rs

pub trait JobSource: Send + Sync {
    fn name(&self) -> &str;
    fn fetch_jobs(&self, company_id: &str) -> impl Future<Output = Result<Vec<DiscoveredJob>>> + Send;
    fn normalize_job(&self, raw: RawJob) -> Result<DiscoveredJob>;
}

pub struct GreenhouseSource {
    board_token: String,
    client: reqwest::Client,
}

pub struct LeverSource {
    company_id: String,
    client: reqwest::Client,
}

pub struct JobSourceRegistry {
    sources: HashMap<String, Box<dyn JobSource>>,
}

impl JobSourceRegistry {
    pub fn new() -> Self { ... }
    pub fn register(&mut self, name: String, source: Box<dyn JobSource>) { ... }
    pub async fn fetch_from_all(&self) -> Result<Vec<DiscoveredJob>> { ... }
    pub async fn fetch_from(&self, source: &str, company_id: &str) -> Result<Vec<DiscoveredJob>> { ... }
}
```

### Discovered Job Structure

```rust
// lazyjob-core/src/discovery/models.rs

pub struct DiscoveredJob {
    pub id: Uuid,
    pub source: String,
    pub source_id: String,  // ID from the source (e.g., Greenhouse job ID)
    pub title: String,
    pub company_name: String,
    pub company_id: Option<String>,  // Link to company in DB
    pub location: Option<String>,
    pub remote: Option<RemoteType>,
    pub url: String,
    pub description: String,  // Cleaned HTML or plain text
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    pub salary_currency: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,  // full-time, contract, etc.
    pub posted_at: Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
    pub embedding: Option<Vec<f32>>,  // For semantic search
}

pub enum RemoteType {
    Yes,
    No,
    Hybrid,
    Unknown,
}
```

### Company Registry

```rust
// lazyjob-core/src/discovery/companies.rs

pub struct CompanyRegistry {
    companies: HashMap<String, CompanyConfig>,  // company_name -> config
}

pub struct CompanyConfig {
    pub name: String,
    pub greenhouse_board_token: Option<String>,
    pub lever_company_id: Option<String>,
    pub career_page_url: Option<String>,
    pub industry: Option<String>,
    pub size: Option<CompanySize>,
}

impl CompanyRegistry {
    pub fn from_config(config: &Config) -> Result<Self> { ... }

    pub async fn discover_company_jobs(&self, company_name: &str) -> Result<Vec<DiscoveredJob>> {
        let config = self.companies.get(company_name)?;
        let mut jobs = Vec::new();

        if let Some(token) = &config.greenhouse_board_token {
            let source = GreenhouseSource::new(token);
            jobs.extend(source.fetch_jobs().await?);
        }

        if let Some(id) = &config.lever_company_id {
            let source = LeverSource::new(id);
            jobs.extend(source.fetch_jobs().await?);
        }

        Ok(jobs)
    }

    pub fn discover_all(&self) -> impl Future<Output = Result<Vec<DiscoveredJob>>> + Send {
        // Fan out to all configured companies
    }
}
```

### Semantic Matching

```rust
// lazyjob-core/src/discovery/matching.rs

pub struct JobMatcher {
    embedder: Arc<dyn LLMProvider>,  // For generating embeddings
}

impl JobMatcher {
    pub fn new(embedder: Arc<dyn LLMProvider>) -> Self {
        Self { embedder }
    }

    /// Generate embedding for a job description
    pub async fn embed_job(&self, job: &DiscoveredJob) -> Result<Vec<f32>> {
        let text = format!(
            "{}\n{}\n{}",
            job.title,
            job.company_name,
            job.description
        );
        self.embedder.embed(&text).await.map_err(Into::into)
    }

    /// Generate embedding for user's life sheet skills/experience
    pub fn embed_life_sheet(&self, sheet: &LifeSheet) -> Vec<f32> {
        // Combine skills, experience titles, education
        let text = format!(
            "{} {} {}",
            sheet.skills.iter().map(|s| s.name).join(" "),
            sheet.experience.iter().map(|e| format!("{} {}", e.position, e.summary)).join(" "),
            sheet.education.iter().map(|e| e.field.clone()).join(" ")
        );
        // This would call embed() - but we need async
        // In practice, use block_on or make this async
    }

    /// Compute similarity between a job and life sheet
    pub fn similarity(&self, job_embedding: &[f32], sheet_embedding: &[f32]) -> f32 {
        cosine_similarity(job_embedding, sheet_embedding)
    }

    /// Find jobs most relevant to user's profile
    pub async fn find_matching_jobs(
        &self,
        jobs: &[DiscoveredJob],
        sheet: &LifeSheet,
        top_k: usize,
    ) -> Result<Vec<(DiscoveredJob, f32)>> {
        let sheet_text = self.life_sheet_to_text(sheet);
        let sheet_embedding = self.embedder.embed(&sheet_text).await?;

        let mut scores: Vec<_> = futures::future::join_all(
            jobs.iter().map(|job| async {
                let job_text = format!("{}\n{}\n{}", job.title, job.company_name, job.description);
                let job_embedding = self.embedder.embed(&job_text).await;
                (job.clone(), job_embedding)
            })
        )
        .await
        .into_iter()
        .filter_map(|(job, emb)| {
            emb.map(|e| (job, self.similarity(&e, &sheet_embedding))).ok()
        })
        .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok(scores.into_iter().take(top_k).collect())
    }

    fn life_sheet_to_text(&self, sheet: &LifeSheet) -> String {
        let skills = sheet.skills.iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(" ");

        let experience = sheet.experience.iter()
            .map(|e| format!("{} at {}", e.position, e.company_name))
            .collect::<Vec<_>>()
            .join(" ");

        let education = sheet.education.iter()
            .map(|e| format!("{} in {}", e.degree, e.field))
            .collect::<Vec<_>>()
            .join(" ");

        format!("Skills: {}\nExperience: {}\nEducation: {}", skills, experience, education)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}
```

### Discovery Service

```rust
// lazyjob-core/src/discovery/service.rs

pub struct DiscoveryService {
    company_registry: CompanyRegistry,
    job_repository: Arc<JobRepository>,
    matcher: JobMatcher,
    embedder: Arc<dyn LLMProvider>,
}

impl DiscoveryService {
    pub async fn refresh_all_companies(&self) -> Result<DiscoveryReport> {
        let jobs = self.company_registry.discover_all().await?;

        let mut new_jobs = 0;
        let mut updated = 0;
        let mut duplicates = 0;

        for job in jobs {
            match self.job_repository.find_by_source(&job.source, &job.source_id) {
                Ok(Some(existing)) => {
                    // Check if significantly different
                    if existing.title != job.title || existing.description != job.description {
                        self.job_repository.update(&job).await?;
                        updated += 1;
                    } else {
                        duplicates += 1;
                    }
                }
                Ok(None) => {
                    self.job_repository.insert(&job).await?;
                    new_jobs += 1;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(DiscoveryReport { new_jobs, updated, duplicates })
    }

    pub async fn search_by_text(&self, query: &str) -> Result<Vec<Job>> {
        // Simple text search (can be enhanced with FTS5)
        self.job_repository.search(query).await
    }

    pub async fn find_similar_jobs(&self, job_id: &str, top_k: usize) -> Result<Vec<(Job, f32)>> {
        let job = self.job_repository.get(job_id).await?;
        let sheet = self.job_repository.get_life_sheet().await?;
        self.matcher.find_matching_jobs(&[job], &sheet, top_k).await
    }
}

pub struct DiscoveryReport {
    pub new_jobs: usize,
    pub updated: usize,
    pub duplicates: usize,
}
```

### User Configuration

```yaml
# ~/.lazyjob/config.yaml
discovery:
  companies:
    - name: "Stripe"
      greenhouse_board_token: "stripe"
      lever_company_id: null
    - name: "Airbnb"
      greenhouse_board_token: "airbnb"
    - name: "Notion"
      lever_company_id: "notion"
    - name: "Vercel"
      greenhouse_board_token: "vercel"

  polling:
    enabled: true
    interval_minutes: 60  # Refresh every hour

  matching:
    top_k: 20  # Number of matching jobs to surface
```

### Platform Support Matrix

| Platform | API Type | Auth Required | Data Quality | Coverage |
|----------|----------|---------------|---------------|----------|
| Greenhouse | REST (public) | No | High | Medium (enterprise) |
| Lever | REST (public) | No | High | Medium (growth-stage) |
| Workday | Scraping | Yes | High | High (enterprise) |
| LinkedIn | Scraping | Yes | High | Very High (ToS violation) |
| Indeed | Scraping | No | Medium | High (ToS unclear) |
| Glassdoor | Scraping | No | Medium | Medium (reviews bias) |

### Data Enrichment Pipeline

```rust
// When a new job is discovered:

async fn enrich_job(&self, job: &mut DiscoveredJob) -> Result<()> {
    // 1. Clean HTML from description
    job.description = self.strip_html(&job.description);

    // 2. Extract salary if present (regex)
    if let Some((min, max)) = self.extract_salary(&job.description) {
        job.salary_min = min;
        job.salary_max = max;
    }

    // 3. Classify remote status
    job.remote = self.classify_remote(&job.description, &job.location);

    // 4. Generate embedding (async, done separately)
    // job.embedding = self.embedder.embed(&job.description).await.ok();

    // 5. Link to existing company if known
    if let Some(company) = self.find_company(&job.company_name) {
        job.company_id = Some(company.id);
    }

    Ok(())
}
```

---

## Job Matching Flow

```
┌─────────────────────────────────────────────────────────────┐
│ User configures companies in config.yaml                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ DiscoveryService::refresh_all_companies()                   │
│   - For each company, fetch from Greenhouse/Lever           │
│   - Collect raw job data                                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ EnrichmentPipeline::process(job)                            │
│   - Strip HTML                                              │
│   - Extract salary/benefits                                 │
│   - Classify remote/hybrid/onsite                          │
│   - Normalize location                                      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Deduplication                                               │
│   - Compare (title, company, location, description_hash)   │
│   - Skip if similar job already exists                      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Storage                                                     │
│   - Insert new jobs into SQLite                            │
│   - Generate embeddings asynchronously                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Matching (on demand or scheduled)                          │
│   - Embed life sheet (skills + experience)                 │
│   - Compute similarity with all discovered jobs            │
│   - Surface top K matches in TUI                           │
└─────────────────────────────────────────────────────────────┘
```

---

## Failure Modes

1. **API Rate Limiting**: Implement exponential backoff, cache responses, respect rate limits
2. **Company Not on Job Board**: Show user that company isn't configured, offer manual add
3. **Invalid Board Token**: Validate tokens, provide clear error messages
4. **Embedding Service Down**: Use cached embeddings, fall back to keyword search
5. **Duplicate Jobs**: Deduplicate by source+source_id, update existing if content changed
6. **HTML Sanitization**: Use ammonia or similar crate to safely strip HTML

---

## Open Questions

1. **LinkedIn Integration**: Some users may want LinkedIn job search. Should we offer a browser extension approach?
2. **Full-Text Search**: Should we use SQLite FTS5 for keyword search alongside semantic search?
3. **Job Alert Frequency**: How often to poll for new jobs? Configurable per company?
4. **Matching Algorithm**: Cosine similarity is simple. Should we weight skills vs experience vs education differently?
5. **Job Description Length**: Long descriptions create large embeddings. Should we truncate or summarize first?

---

## Dependencies

```toml
# lazyjob-core/Cargo.toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
scraper = "0.20"           # HTML parsing
ammonia = "4"              # HTML sanitization
regex = "1"                # Salary extraction
futures = "0.3"            # Async utilities
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
serde = { version = "1", features = ["derive"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"

[dev-dependencies]
wiremock = "1"             # HTTP mocking for tests
```

---

## Sources

- [Greenhouse Job Board API](https://developers.greenhouse.io/job-board)
- [Lever API Documentation](https://docs.lever.co/)
- [JobSpy GitHub](https://github.com/jobspy-dev/jobspy)
- [Chroma Documentation](https://docs.trychroma.com/)
- [OpenAI Embeddings API](https://platform.openai.com/docs/guides/embeddings)
- [Qdrant Vector Database](https://qdrant.tech/)
