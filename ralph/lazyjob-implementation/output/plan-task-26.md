# Plan: Task 26 — company-research

## Files to Create/Modify

1. **NEW** `crates/lazyjob-core/src/discovery/company.rs`
   - `Completer` trait: `async fn complete(&self, system: &str, user: &str) -> Result<String>`
   - `EnrichmentData` struct: `{ industry: String, size: String, tech_stack: Vec<String>, culture_keywords: Vec<String>, recent_news: Vec<String> }`
   - `CompanyResearcher` struct: `{ completer: Arc<dyn Completer>, client: reqwest::Client }`
   - `CompanyResearcher::new(completer, client) -> Self`
   - `CompanyResearcher::enrich(&self, company_id: &CompanyId, pool: &PgPool) -> Result<EnrichmentData>`
   - `pub fn enrichment_badge(industry: Option<&str>) -> &'static str`

2. **MODIFY** `crates/lazyjob-core/src/discovery/mod.rs`
   - Add `pub mod company;`
   - Re-export `CompanyResearcher`, `Completer`, `EnrichmentData`, `enrichment_badge`

3. **MODIFY** `crates/lazyjob-tui/src/views/jobs_list.rs`
   - Import `enrichment_badge` from `lazyjob_core::discovery`
   - Update render stub to include badge in placeholder text
   - Add render test verifying badge shows "[E]" for enriched company

4. **MODIFY** `crates/lazyjob-cli/src/main.rs`
   - Add `RalphCommand::CompanyResearch { company_id: String }` variant
   - Add `LlmProviderCompleter` wrapper struct implementing `Completer`
   - Add `handle_company_research()` handler
   - Add CLI test `parse_ralph_company_research`

## Types/Functions

```rust
// company.rs
#[async_trait]
pub trait Completer: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentData {
    pub industry: Option<String>,
    pub size: Option<String>,
    pub tech_stack: Vec<String>,
    pub culture_keywords: Vec<String>,
    pub recent_news: Vec<String>,
}

pub struct CompanyResearcher {
    completer: Arc<dyn Completer>,
    client: reqwest::Client,
}

impl CompanyResearcher {
    pub fn new(completer: Arc<dyn Completer>, client: reqwest::Client) -> Self
    pub async fn enrich(&self, company_id: &CompanyId, pool: &PgPool) -> Result<EnrichmentData>
}

pub fn enrichment_badge(industry: Option<&str>) -> &'static str  // "[E]" or "[ ]"
```

## enrich() flow
1. `CompanyRepository::find_by_id(company_id)` — return NotFound if missing
2. If no website set, return `Err(CoreError::Validation("company has no website"))`
3. `self.client.get(website).send().await` — map error to `CoreError::Http`
4. `.text().await` to get body string
5. Strip HTML with ammonia (reuse `strip_html` helper from sources/mod.rs)
6. Truncate to 3000 chars
7. Build system + user prompts asking for JSON output
8. `self.completer.complete(system, user).await`
9. Extract JSON from response (use `serde_json::from_str` on the response)
10. Populate company fields: `industry`, `size`, `tech_stack`, `culture_keywords`, `notes` (recent_news joined)
11. `CompanyRepository::update(&company)`
12. Return `EnrichmentData`

## Tests to Write

### Learning test
- `reqwest_client_gets_html_from_mock` — proves reqwest::Client can fetch a URL served by wiremock MockServer

### Unit tests (no DB)
- `enrichment_data_deserializes_from_json` — proves serde_json can parse LLM JSON response into EnrichmentData
- `enrichment_badge_returns_E_for_enriched` — `enrichment_badge(Some("Tech")) == "[E]"`
- `enrichment_badge_returns_empty_for_unenriched` — `enrichment_badge(None) == "[ ]"`

### Integration tests (require DATABASE_URL)
- `enrich_company_with_mock_http_and_mock_llm` — uses wiremock MockServer for website, MockCompleter, real PgPool

### CLI tests
- `parse_ralph_company_research` — clap parses `ralph company-research --company-id <uuid>`
