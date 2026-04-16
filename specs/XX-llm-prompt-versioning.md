# LLM Prompt Versioning and Testing

## Status
Researching

## Problem Statement

LazyJob uses LLM prompts extensively for job discovery, company research, resume tailoring, cover letter generation, and interview prep. These prompts are embedded in code and modified over time. Currently, there is no system for:
1. Tracking prompt changes (what changed, when, why)
2. Testing prompts before deployment
3. Rolling back to previous prompt versions when they cause degradation
4. Comparing prompt outputs across versions
5. Validating structured outputs from prompts

As ralph loops run autonomously, prompt degradation can cause silently wrong outputs that waste user time and API budget.

---

## Solution Overview

A prompt versioning system that:
1. Stores prompts as versioned, structured files (not embedded in code)
2. Provides a testing harness for evaluating prompts against sample inputs
3. Supports rollback to any previous prompt version
4. Captures output samples for each prompt version
5. Validates structured outputs (JSON) against schemas

---

## Design Decisions

### Prompt Storage Format

Prompts are stored as versioned YAML files in `lazyjob-llm/src/prompts/versions/`:

```
lazyjob-llm/src/prompts/
├── Cargo.toml
├── lib.rs
├── versions/
│   ├── job_discovery/
│   │   ├── v1.yaml      # Version 1 of job discovery prompt
│   │   ├── v2.yaml
│   │   ├── v3.yaml      # Current
│   │   └── v3.sample-output.json
│   ├── company_research/
│   │   ├── v1.yaml
│   │   └── v2.yaml
│   └── resume_tailoring/
│       ├── v1.yaml
│       └── v2.yaml
├── active/               # Symlinks to current version
│   ├── job_discovery.yaml -> ../versions/job_discovery/v3.yaml
│   └── ...
└── schemas/             # JSON schemas for structured output validation
    ├── job_discovery_output.json
    └── company_research_output.json
```

### Version File Format

```yaml
# v3.yaml
version: "3"
created_at: "2024-11-15T10:30:00Z"
created_by: "carlos@example.com"
parent_version: "2"
changelog: "Added emphasis on remote-friendly companies per user feedback"

template: |
  You are a job search assistant helping a candidate find their next role.

  Candidate preferences:
  - Remote: {{ remote_preference }}
  - Locations: {{ locations }}
  - Industries: {{ industries }}
  - Salary range: {{ salary_min }} - {{ salary_max }} USD

  {% if resume_summary %}
  Candidate background:
  {{ resume_summary }}
  {% endif %}

  Find jobs matching these criteria. For each job, provide:
  - Job title and company
  - Location and remote policy
  - Salary range if available
  - Why this matches the candidate's preferences

  Return results as JSON:
  {{ schema_output }}

variables:
  - name: remote_preference
    type: string
    required: true
  - name: locations
    type: array[string]
    required: true
  - name: industries
    type: array[string]
    required: false
    default: []
  - name: salary_min
    type: integer
    required: false
  - name: salary_max
    type: integer
    required: false
  - name: resume_summary
    type: string
    required: false

output_schema: "schemas/job_discovery_output.json"
```

### Sample Output Format

```json
// v3.sample-output.json
{
  "version": "3",
  "created_at": "2024-11-15T10:30:00Z",
  "sample_input": {
    "remote_preference": "yes",
    "locations": ["San Francisco", "New York"],
    "industries": ["FinTech"],
    "salary_min": 150000,
    "salary_max": 250000,
    "resume_summary": "5 years backend engineering at Stripe..."
  },
  "sample_output": "...",
  "parsed_output": {
    "jobs": [...]
  },
  "validation": "passed",
  "quality_notes": "Good match reasoning, slight tendency to repeat job titles"
}
```

---

## Prompt Registry

A registry tracks all prompt versions and manages active versions:

```rust
// lazyjob-llm/src/prompts/registry.rs

#[derive(Clone)]
pub struct PromptRegistry {
    prompts_dir: PathBuf,
    active: HashMap<String, String>, // name -> version
}

impl PromptRegistry {
    pub fn new(prompts_dir: PathBuf) -> Result<Self> {
        let active = Self::discover_active(&prompts_dir)?;
        Ok(Self { prompts_dir, active })
    }

    pub fn get(&self, name: &str) -> Result<PromptTemplate> {
        let version = self.active.get(name)
            .ok_or_else(|| PromptError::UnknownPrompt(name.to_string()))?;
        self.load(name, version)
    }

    pub fn get_version(&self, name: &str, version: &str) -> Result<PromptTemplate> {
        self.load(name, version)
    }

    pub fn activate(&mut self, name: &str, version: &str) -> Result<()> {
        let path = self.prompts_dir
            .join("versions")
            .join(name)
            .join(format!("v{}.yaml", version));
        if !path.exists() {
            return Err(PromptError::VersionNotFound(name.to_string(), version.to_string()));
        }

        // Update symlink
        let active_link = self.prompts_dir.join("active").join(format!("{}.yaml", name));
        if active_link.exists() {
            std::fs::remove_file(&active_link)?;
        }
        std::os::unix::fs::symlink(&path, &active_link)?;

        self.active.insert(name.to_string(), version.to_string());
        Ok(())
    }

    pub fn list_versions(&self, name: &str) -> Result<Vec<PromptVersion>> {
        // Return metadata for all versions
    }
}
```

---

## Prompt Renderer

Prompts are rendered with variables before being sent to LLM:

```rust
// lazyjob-llm/src/prompts/renderer.rs

pub struct PromptRenderer {
    registry: PromptRegistry,
    schema_validator: jsonschema::Validator,
}

impl PromptRenderer {
    pub async fn render(
        &self,
        name: &str,
        variables: &PromptVariables,
    ) -> Result<RenderedPrompt> {
        let template = self.registry.get(name)?;

        // Validate variables
        template.validate_variables(variables)?;

        // Render template
        let rendered = template.render(variables)?;

        // Load output schema if specified
        let schema = if let Some(schema_path) = &template.output_schema {
            Some(self.load_schema(schema_path)?)
        } else {
            None
        };

        Ok(RenderedPrompt {
            name: name.to_string(),
            version: template.version.clone(),
            rendered,
            schema,
        })
    }

    pub async fn render_and_call(
        &self,
        name: &str,
        variables: &PromptVariables,
        llm: &dyn LLMProvider,
    ) -> Result<LLMOutput> {
        let prompt = self.render(name, variables).await?;

        // For structured output, use JSON mode
        let response = if let Some(schema) = &prompt.schema {
            llm.chat_structured(&prompt.rendered, schema).await?
        } else {
            llm.chat(vec![ChatMessage::user(&prompt.rendered)]).await?
        };

        Ok(LLMOutput {
            prompt_version: prompt.version,
            raw_output: response.content.clone(),
            parsed_output: self.parse_output(&response, &prompt.schema)?,
        })
    }
}
```

---

## Prompt Testing Harness

A test harness evaluates prompts against sample inputs:

```rust
// lazyjob-llm/src/prompts/testing.rs

pub struct PromptTester {
    renderer: PromptRenderer,
    llm: Arc<dyn LLMProvider>,
}

impl PromptTester {
    /// Run a prompt against its sample input and compare to sample output
    pub async fn test_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<TestResult> {
        let sample = self.load_sample_output(name, version)?;
        let rendered = self.renderer.render(name, &sample.sample_input.into()).await?;

        // Call LLM
        let output = self.llm.chat(vec![ChatMessage::user(&rendered.rendered)]).await?;

        // Compare to sample output (semantic similarity for text, exact match for structured)
        let similarity = self.compare(&output.content, &sample.sample_output);

        Ok(TestResult {
            version: version.to_string(),
            input_hash: hash(&sample.sample_input),
            output_hash: hash(&output.content),
            similarity_score: similarity,
            passed: similarity > 0.85, // Threshold
        })
    }

    /// Compare two versions against the same input
    pub async fn compare_versions(
        &self,
        name: &str,
        v1: &str,
        v2: &str,
    ) -> Result<VersionComparison> {
        let sample = self.load_sample_output(name, v1)?; // Use v1's sample

        let output1 = self.render_and_call(name, v1, &sample.sample_input).await?;
        let output2 = self.render_and_call(name, v2, &sample.sample_input).await?;

        Ok(VersionComparison {
            v1: v1.to_string(),
            v2: v2.to_string(),
            v1_output: output1.raw_output,
            v2_output: output2.raw_output,
            semantic_similarity: self.semantic_compare(&output1.raw_output, &output2.raw_output)?,
            token_cost_v1: output1.usage.total_tokens,
            token_cost_v2: output2.usage.total_tokens,
        })
    }
}
```

### Test Runner

```bash
# Run all prompt tests
cargo test -p lazyjob-llm prompt_tests

# Run specific prompt tests
cargo test -p lazyjob-llm prompt_tests::job_discovery

# Compare versions
cargo run -p lazyjob-llm -- compare-versions job_discovery v2 v3
```

---

## Structured Output Validation

JSON schemas define expected output structure:

```json
// schemas/job_discovery_output.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "required": ["jobs"],
  "properties": {
    "jobs": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["title", "company", "url"],
        "properties": {
          "title": { "type": "string" },
          "company": { "type": "string" },
          "url": { "type": "string", "format": "uri" },
          "location": { "type": "string" },
          "remote_policy": { "type": "string", "enum": ["remote", "hybrid", "onsite"] },
          "salary_min": { "type": "integer" },
          "salary_max": { "type": "integer" },
          "match_score": { "type": "number", "minimum": 0, "maximum": 1 },
          "match_reasoning": { "type": "string" }
        }
      }
    }
  }
}
```

---

## Rollback Mechanism

```rust
// Rollback to previous version
pub fn rollback(name: &str) -> Result<()> {
    let registry = PromptRegistry::load()?;
    let versions = registry.list_versions(name)?;

    if versions.len() < 2 {
        return Err(PromptError::CannotRollback(name.to_string()));
    }

    // Activate previous version
    let previous = versions[versions.len() - 2].version.clone();
    registry.activate(name, &previous)?;

    // Log rollback for audit
    tracing::info!(
        prompt = name,
        rolled_back_to = previous,
        "Prompt rolled back"
    );

    Ok(())
}
```

---

## CI/CD Integration

Prompts are versioned with git. A CI pipeline runs tests on PRs:

```yaml
# .github/workflows/prompt-tests.yml
name: Prompt Tests
on:
  pull_request:
    paths:
      - 'lazyjob-llm/src/prompts/versions/**'

jobs:
  test-prompts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Run prompt tests
        run: cargo test -p lazyjob-llm prompt_tests

      - name: Comment results
        uses: actions/github-script@v7
        with:
          script: |
            // Post test results as PR comment
```

---

## Open Questions

1. **Sample output storage**: Should sample outputs be stored in git or separate storage?
2. **Similarity threshold**: What threshold defines "passing" for prompt tests?
3. **Automatic promotion**: Should vN+1 auto-promote to active if tests pass?
4. **Prompt templating language**: Use Askama, Handlebars, or custom?
5. **Schema evolution**: When output schema changes, how are old versions handled?

---

## Related Specs

- `02-llm-provider-abstraction.md` - LLM Provider trait
- `XX-llm-function-calling.md` - Tool schema and structured outputs
- `XX-llm-cost-budget-management.md` - Cost tracking per prompt
