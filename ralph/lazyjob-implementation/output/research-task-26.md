# Research: Task 26 — company-research

## What Exists

### Company domain type (lazyjob-core/src/domain/company.rs)
```rust
pub struct Company {
    pub id: CompanyId,
    pub name: String,
    pub website: Option<String>,
    pub industry: Option<String>,
    pub size: Option<String>,
    pub tech_stack: Vec<String>,
    pub culture_keywords: Vec<String>,
    pub notes: Option<String>,
}
```
All fields needed for enrichment already exist. No `enriched_at` timestamp, but `industry.is_some()` serves as a proxy for enrichment status.

### CompanyRepository (lazyjob-core/src/repositories/company.rs)
Has `insert`, `find_by_id`, `list`, `update`, `delete`. The `update()` method updates all company fields. Returns `CoreError::NotFound` if no rows affected.

### Discovery module (lazyjob-core/src/discovery/)
Files: `mod.rs`, `matching.rs`, `service.rs`, `sources/mod.rs`, `sources/greenhouse.rs`, `sources/lever.rs`.
No `company.rs` exists yet.

### Key pattern from matching.rs (Task 25)
- Defines local `Embedder` trait to avoid circular dep with lazyjob-llm
- `MatchScorer` holds `Arc<dyn Embedder>`
- To wire a real embedding provider, implement `Embedder` for a wrapper struct in CLI layer

### Circular Dependency Constraint
lazyjob-llm → lazyjob-core (for sqlx logging). So lazyjob-core CANNOT import lazyjob-llm.
Solution: define a local `Completer` trait in `lazyjob-core::discovery::company`.

### reqwest (already in lazyjob-core deps)
Available for HTTP fetching of company websites.

### CoreError
Has `Http(String)` variant — already suitable for HTTP errors from website fetching.

### LlmProvider (lazyjob-llm)
`async fn complete(messages: Vec<ChatMessage>, opts: CompletionOptions) -> Result<LlmResponse>`
- `MockLlmProvider::with_content(str)` for tests

### JobsListView (lazyjob-tui/src/views/jobs_list.rs)
Currently a stub rendering a placeholder Paragraph. Task requires adding an enrichment status badge.

## Key Design Decisions

1. **Local `Completer` trait** in `discovery::company` — mirrors `Embedder` pattern, avoids circular dep
2. **CompanyResearcher::new(completer, client)** — takes HTTP client separate from completer for flexibility
3. **JSON extraction from LLM** — ask LLM to return structured JSON; parse with serde_json
4. **Truncate website content** — limit to 3000 chars before sending to LLM to avoid token overflow  
5. **Update via existing CompanyRepository** — no new repository methods needed
6. **Enrichment badge helper** — `enrichment_badge(industry: Option<&str>) -> &'static str` returns "[E]" or "[ ]"
