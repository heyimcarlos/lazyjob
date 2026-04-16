# Implementation Plan: LLM Prompt Versioning and Testing

## Status
Draft

## Related Spec
[specs/XX-llm-prompt-versioning.md](./XX-llm-prompt-versioning.md)

## Overview

LazyJob uses LLM prompts across every subsystem — job discovery, resume tailoring, cover
letter generation, interview prep, company research, and more. These prompts evolve over time
as the product matures and users surface quality problems. Without a versioning system, every
change is opaque (no diff, no blame), silent regressions are impossible to detect, and rolling
back requires grepping code to find the old wording.

This plan implements a prompt versioning system inside `lazyjob-llm` with five capabilities:
(1) Prompt templates stored as versioned YAML files with structured variable declarations;
(2) A `PromptRegistry` that tracks which version is active for each named prompt, persisted
in a `prompt_registry.toml` sidecar (no symlinks — cross-platform safe);
(3) A `PromptRenderer` that resolves the active version, validates all required variables,
renders the Jinja2-style template via `minijinja`, and returns a `RenderedPrompt` ready for
the LLM provider; (4) A `OutputValidator` that checks LLM responses against a JSON Schema
using the `jsonschema` crate; (5) A `PromptTester` harness that renders a template against
a stored sample input, calls the LLM, and reports a `TestResult` with a similarity score —
usable both from `cargo test` integration tests and from `lazyjob prompt test` CLI.

The spec proposes symlinks for the `active/` directory. This plan replaces that with a
`prompt_registry.toml` file because symlinks are fragile on NFS mounts, Windows, and when
moving the config directory. The TOML file achieves the same mapping with fewer failure modes.

## Prerequisites

### Must be implemented first
- `specs/agentic-llm-provider-abstraction.md` — `LlmProvider`, `ChatMessage`, `TokenUsage`,
  `LlmResponse` must exist. The renderer calls `LlmProvider::chat()`.
- `specs/17-ralph-prompt-templates.md` — defines the base `RenderedPrompt` type and
  `SimpleTemplateEngine`. This plan layers on top: the YAML versioning registry feeds
  rendered text into the existing prompt infrastructure.
- `specs/04-sqlite-persistence-implementation-plan.md` — `SqlitePool` and migration runner
  for the `prompt_activations` audit table.

### Crates to add to workspace `Cargo.toml`

```toml
[workspace.dependencies]
# already present from earlier plans:
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
serde_yaml         = "0.9"
thiserror          = "1"
anyhow             = "1"
tracing            = "0.1"
chrono             = { version = "0.4", features = ["serde"] }
tokio              = { version = "1", features = ["macros", "rt-multi-thread"] }
once_cell          = "1"
sqlx               = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "chrono"] }

# new for this plan:
minijinja          = "2"          # Jinja2-compatible template engine, pure Rust
jsonschema         = "0.18"       # JSON Schema draft-07 validator
sha2               = "0.10"       # SHA-256 for input/output hashing in TestResult
hex                = "0.4"        # encode hash bytes to hex strings
similar            = "2"          # unified diff for version comparison display
toml               = "0.8"        # prompt_registry.toml serialization
```

`minijinja` 2.x implements the Jinja2 template language including `{{ var }}`, `{% if %}`,
`{% for %}`, and `{% set %}` blocks in ~80KB of pure Rust with no runtime dependencies.
`jsonschema 0.18` supports draft-07 schemas (required by the spec's schema examples).

---

## Architecture

### Crate Placement

`lazyjob-llm/src/prompts/` owns the full prompt versioning system. This crate already owns
the LLM provider abstraction and the rendered-prompt types, so the versioning registry
lives here naturally.

`lazyjob-core/src/prompt_log/` owns `SqlitePromptActivationRepository` for audit persistence.
`lazyjob-cli/src/commands/prompt.rs` owns the `lazyjob prompt` CLI subcommand.

Dependency direction: `lazyjob-cli` → `lazyjob-core` → `lazyjob-llm`.

### Module Structure

```
lazyjob-llm/
  src/
    prompts/
      mod.rs             # re-exports PromptRegistry, PromptRenderer, PromptTester,
                         # PromptError, PromptVariables, RenderedPromptV2, TestResult
      registry.rs        # PromptRegistry: load, get, list_versions, activate, rollback
      template.rs        # VersionedTemplate, PromptVariableDecl, VariableType deserialization
      renderer.rs        # PromptRenderer: render(), validate_variables(), cache_system_prompt()
      validator.rs       # OutputValidator: validate() against JSON Schema
      tester.rs          # PromptTester: test_version(), compare_versions()
      sample.rs          # SampleOutput, SampleInput I/O helpers
      error.rs           # PromptError enum

  prompts/               # versioned prompt files (not under src/ to avoid recompile on edit)
    versions/
      job_discovery/
        v1.yaml
        v1.sample.json
        v2.yaml
        v2.sample.json
      resume_tailoring/
        v1.yaml
        v1.sample.json
      cover_letter/
        v1.yaml
      ...
    schemas/
      job_discovery_output.json
      resume_tailoring_output.json
      cover_letter_output.json
      ...
    prompt_registry.toml   # active version mapping

lazyjob-core/
  src/
    prompt_log/
      mod.rs             # re-exports SqlitePromptActivationRepository, ActivationRecord
      repository.rs      # SqlitePromptActivationRepository: insert + list_recent
  migrations/
    XXX_prompt_activations.sql

lazyjob-cli/
  src/
    commands/
      prompt.rs          # PromptCmd: list, versions, activate, rollback, test, compare
```

### Core Types

```rust
// lazyjob-llm/src/prompts/template.rs

/// A prompt template deserialized from `vN.yaml`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VersionedTemplate {
    pub version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by: Option<String>,
    pub parent_version: Option<String>,
    pub changelog: String,
    pub template: String,
    pub variables: Vec<PromptVariableDecl>,
    pub output_schema: Option<String>,
    /// If true, instructs the renderer to set cache_control on the system prompt
    /// for Anthropic prompt caching.
    #[serde(default)]
    pub cache_system_prompt: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PromptVariableDecl {
    pub name: String,
    #[serde(rename = "type")]
    pub var_type: VariableType,
    #[serde(default)]
    pub required: bool,
    /// JSON value used when the variable is absent.
    pub default: Option<serde_json::Value>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    String,
    Integer,
    Float,
    Boolean,
    #[serde(rename = "array[string]")]
    ArrayString,
    #[serde(rename = "array[object]")]
    ArrayObject,
    Object,
}
```

```rust
// lazyjob-llm/src/prompts/registry.rs

/// Registry mapping prompt name → active version.
/// Serialized to/from `prompt_registry.toml`.
#[derive(Debug, Clone)]
pub struct PromptRegistry {
    prompts_dir: std::path::PathBuf,
    /// name → active version string (e.g. "3")
    active: std::collections::HashMap<String, String>,
}

/// The TOML file format for the active-version map.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RegistryFile {
    /// key = prompt name, value = version string
    active: std::collections::HashMap<String, String>,
}

/// Metadata about a single prompt version (from its YAML header, not the full template).
#[derive(Debug, Clone)]
pub struct PromptVersion {
    pub name: String,
    pub version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub changelog: String,
    pub parent_version: Option<String>,
    pub is_active: bool,
}
```

```rust
// lazyjob-llm/src/prompts/renderer.rs

/// A prompt that has been fully rendered and is ready to send to the LLM.
#[derive(Debug, Clone)]
pub struct RenderedPromptV2 {
    pub prompt_name: String,
    pub version: String,
    pub content: String,
    pub output_schema: Option<serde_json::Value>,
    pub cache_system_prompt: bool,
}

/// Caller-provided variable bindings for template rendering.
/// Uses a JSON object as the bag of values — flexible enough for all variable types.
#[derive(Debug, Clone, Default)]
pub struct PromptVariables(serde_json::Map<String, serde_json::Value>);

impl PromptVariables {
    pub fn new() -> Self { Self(serde_json::Map::new()) }
    pub fn insert(&mut self, key: impl Into<String>, val: impl serde::Serialize) -> &mut Self {
        self.0.insert(key.into(), serde_json::to_value(val).expect("serialize variable"));
        self
    }
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> { self.0.get(key) }
    pub(crate) fn as_map(&self) -> &serde_json::Map<String, serde_json::Value> { &self.0 }
}
```

```rust
// lazyjob-llm/src/prompts/validator.rs

/// Validates structured LLM output against a JSON Schema.
pub struct OutputValidator {
    compiled: jsonschema::JSONSchema,
    schema_name: String,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub passed: bool,
    /// Each error is a (instance_path, message) pair.
    pub errors: Vec<(String, String)>,
}
```

```rust
// lazyjob-llm/src/prompts/tester.rs

pub struct PromptTester {
    registry: PromptRegistry,
    llm: std::sync::Arc<dyn crate::LlmProvider>,
}

#[derive(Debug)]
pub struct TestResult {
    pub prompt_name: String,
    pub version: String,
    pub input_hash: String,        // hex(SHA-256(sample_input JSON))
    pub output_hash: String,       // hex(SHA-256(llm_output))
    pub schema_valid: bool,
    pub schema_errors: Vec<(String, String)>,
    pub similarity_score: f32,     // 0.0..=1.0 against stored sample output
    pub passed: bool,
    pub token_usage: crate::TokenUsage,
    pub cost_microdollars: i64,
}

#[derive(Debug)]
pub struct VersionComparison {
    pub prompt_name: String,
    pub v1: String,
    pub v2: String,
    pub v1_output: String,
    pub v2_output: String,
    pub diff: String,              // unified diff via `similar` crate
    pub similarity_score: f32,
    pub v1_tokens: u32,
    pub v2_tokens: u32,
}
```

```rust
// lazyjob-llm/src/prompts/sample.rs

/// Stored sample input/output pair for a prompt version.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SampleOutput {
    pub version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub sample_input: serde_json::Value,
    pub sample_output: String,
    pub parsed_output: Option<serde_json::Value>,
    pub validation: SampleValidationStatus,
    pub quality_notes: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleValidationStatus { Passed, Failed, Unchecked }
```

```rust
// lazyjob-core/src/prompt_log/repository.rs

/// Audit log entry for a prompt version activation or rollback.
#[derive(Debug, sqlx::FromRow)]
pub struct ActivationRecord {
    pub id: i64,
    pub prompt_name: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub action: String,            // "activate" | "rollback"
    pub triggered_by: Option<String>, // CLI user or "system"
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct SqlitePromptActivationRepository {
    pool: sqlx::SqlitePool,
}
```

### Trait Definitions

```rust
// lazyjob-llm/src/prompts/registry.rs

impl PromptRegistry {
    /// Load registry from `{prompts_dir}/prompt_registry.toml`.
    /// Creates the TOML with an empty active map if it does not exist.
    pub fn load(prompts_dir: std::path::PathBuf) -> Result<Self, PromptError>;

    /// Return the active `VersionedTemplate` for a named prompt.
    /// Returns `PromptError::UnknownPrompt` if not in the active map.
    pub fn get(&self, name: &str) -> Result<VersionedTemplate, PromptError>;

    /// Return the specific `VersionedTemplate` for a named prompt and version.
    /// Returns `PromptError::VersionNotFound` if the file does not exist.
    pub fn get_version(&self, name: &str, version: &str) -> Result<VersionedTemplate, PromptError>;

    /// Return metadata for all versions of a prompt, sorted by version number ascending.
    pub fn list_versions(&self, name: &str) -> Result<Vec<PromptVersion>, PromptError>;

    /// Return all prompt names discovered from the `versions/` directory.
    pub fn list_prompts(&self) -> Result<Vec<String>, PromptError>;

    /// Set `name` → `version` in the active map and persist the TOML.
    /// Returns `PromptError::VersionNotFound` if the target version YAML does not exist.
    pub fn activate(&mut self, name: &str, version: &str) -> Result<(), PromptError>;

    /// Activate the previous version (the one before the current active version).
    /// Returns `PromptError::CannotRollback` if there is only one version.
    pub fn rollback(&mut self, name: &str) -> Result<String, PromptError>;

    fn load_template_yaml(path: &std::path::Path) -> Result<VersionedTemplate, PromptError>;
    fn version_path(&self, name: &str, version: &str) -> std::path::PathBuf;
    fn sample_path(&self, name: &str, version: &str) -> std::path::PathBuf;
    fn save(&self) -> Result<(), PromptError>;
}
```

```rust
// lazyjob-llm/src/prompts/renderer.rs

impl PromptRenderer {
    pub fn new(registry: PromptRegistry) -> Self;

    /// Render the active version of a prompt with the given variables.
    /// Steps: (1) load template, (2) validate variables, (3) render via minijinja.
    pub fn render(
        &self,
        name: &str,
        variables: &PromptVariables,
    ) -> Result<RenderedPromptV2, PromptError>;

    /// Same as render() but targets a specific version (for testing/comparison).
    pub fn render_version(
        &self,
        name: &str,
        version: &str,
        variables: &PromptVariables,
    ) -> Result<RenderedPromptV2, PromptError>;

    /// Validate that all required variables are present and types are compatible.
    fn validate_variables(
        template: &VersionedTemplate,
        variables: &PromptVariables,
    ) -> Result<(), PromptError>;

    /// Load the JSON Schema from `schemas/{schema_name}` relative to prompts_dir.
    fn load_schema(prompts_dir: &std::path::Path, schema_path: &str)
        -> Result<serde_json::Value, PromptError>;
}
```

```rust
// lazyjob-llm/src/prompts/validator.rs

impl OutputValidator {
    /// Compile the schema once at construction.
    pub fn new(schema: serde_json::Value, schema_name: String)
        -> Result<Self, PromptError>;

    /// Validate raw LLM output text (must be valid JSON) against the compiled schema.
    pub fn validate(&self, raw_output: &str) -> Result<ValidationResult, PromptError>;
}
```

```rust
// lazyjob-llm/src/prompts/tester.rs

impl PromptTester {
    pub fn new(registry: PromptRegistry, llm: std::sync::Arc<dyn crate::LlmProvider>) -> Self;

    /// Test the active (or specified) version against its stored sample input.
    /// Returns `PromptError::NoSampleOutput` if no `.sample.json` exists.
    pub async fn test_version(
        &self,
        name: &str,
        version: Option<&str>,  // None = active version
    ) -> Result<TestResult, PromptError>;

    /// Render and call LLM for both versions using v1's sample input, then diff outputs.
    pub async fn compare_versions(
        &self,
        name: &str,
        v1: &str,
        v2: &str,
    ) -> Result<VersionComparison, PromptError>;

    /// Capture a new sample output for a version (write `.sample.json` to disk).
    /// Called after a human has reviewed the output and is satisfied with quality.
    pub async fn capture_sample(
        &self,
        name: &str,
        version: &str,
        input: serde_json::Value,
        quality_notes: Option<String>,
    ) -> Result<(), PromptError>;

    fn similarity_score(a: &str, b: &str) -> f32;
}
```

### SQLite Schema

```sql
-- lazyjob-core/migrations/XXX_prompt_activations.sql

CREATE TABLE IF NOT EXISTS prompt_activations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_name   TEXT NOT NULL,
    from_version  TEXT,
    to_version    TEXT NOT NULL,
    action        TEXT NOT NULL CHECK(action IN ('activate', 'rollback')),
    triggered_by  TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_prompt_activations_name
    ON prompt_activations(prompt_name, created_at DESC);
```

No table for output samples — those are stored on disk as `.sample.json` files alongside
the version YAML. Keeping them as files makes them git-diffable and avoids binary blobs in
SQLite. Only the activation audit trail lives in the database.

---

## Implementation Phases

### Phase 1 — Template Loading and Registry (MVP)

**Goal**: `PromptRegistry::load()` works; callers can `get()` a named prompt and get back
a rendered template with variables substituted.

#### Step 1.1 — YAML template struct and deserialization

File: `lazyjob-llm/src/prompts/template.rs`

Implement `VersionedTemplate` and `PromptVariableDecl` with `serde::Deserialize`.
Handle the `"array[string]"` enum variant (Serde's `rename` attribute on the enum variant).

```rust
// The key deserialization challenge: `type: array[string]` is not a valid Rust identifier.
// Solution: use #[serde(rename = "array[string]")] on the variant.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    String,
    Integer,
    Float,
    Boolean,
    #[serde(rename = "array[string]")]
    ArrayString,
    #[serde(rename = "array[object]")]
    ArrayObject,
    Object,
}
```

Verification: `serde_yaml::from_str::<VersionedTemplate>(include_str!("test_v1.yaml"))` in a
unit test parses all fields correctly including optional ones.

#### Step 1.2 — `prompt_registry.toml` load and save

File: `lazyjob-llm/src/prompts/registry.rs`

`RegistryFile` derives `serde::Serialize` + `serde::Deserialize`.
`PromptRegistry::load()`:
1. Attempts to read `{prompts_dir}/prompt_registry.toml`.
2. If the file does not exist, writes an empty `RegistryFile` and returns an empty `active` map.
3. Parses via `toml::from_str::<RegistryFile>(&content)?`.

`PromptRegistry::save()`:
1. Serializes `RegistryFile { active: self.active.clone() }` via `toml::to_string_pretty`.
2. Writes atomically: write to `prompt_registry.toml.tmp`, then `std::fs::rename()`.
   Atomic rename prevents registry corruption on crash during write.

```rust
fn save(&self) -> Result<(), PromptError> {
    let content = toml::to_string_pretty(&RegistryFile { active: self.active.clone() })
        .map_err(|e| PromptError::RegistrySave(e.to_string()))?;
    let tmp_path = self.prompts_dir.join("prompt_registry.toml.tmp");
    std::fs::write(&tmp_path, content.as_bytes())
        .map_err(|e| PromptError::Io(e))?;
    std::fs::rename(&tmp_path, self.prompts_dir.join("prompt_registry.toml"))
        .map_err(|e| PromptError::Io(e))?;
    Ok(())
}
```

#### Step 1.3 — `list_versions()` and `get_version()`

`list_versions()` reads directory entries from `{prompts_dir}/versions/{name}/`,
filters files matching `v*.yaml`, sorts by parsed version number (parse the `N` in `vN.yaml`
as `u32` — purely numeric versions only), loads the YAML header for each (only needs the
`version`, `created_at`, `changelog`, `parent_version` fields — full template body not required
for listing).

`get_version()` constructs the path `{prompts_dir}/versions/{name}/v{version}.yaml` and
calls `load_template_yaml()`.

```rust
fn load_template_yaml(path: &std::path::Path) -> Result<VersionedTemplate, PromptError> {
    let content = std::fs::read_to_string(path)
        .map_err(|_| PromptError::VersionNotFound {
            name: path.to_string_lossy().to_string(),
            version: "?".into(),
        })?;
    serde_yaml::from_str::<VersionedTemplate>(&content)
        .map_err(|e| PromptError::TemplateParse(e.to_string()))
}
```

#### Step 1.4 — `activate()` and `rollback()`

`activate(name, version)`:
1. Constructs version path and checks it exists (returns `VersionNotFound` if not).
2. Updates `self.active.insert(name, version)`.
3. Calls `self.save()`.

`rollback(name)`:
1. Calls `list_versions(name)` and sorts by version number ascending.
2. Finds the index of the current active version.
3. If index is 0 (or there is only one version), returns `PromptError::CannotRollback`.
4. Activates `versions[index - 1]`.
5. Returns the new active version string.

Verification:
```rust
#[test]
fn test_registry_activate_rollback() {
    let dir = tempfile::tempdir().unwrap();
    // create v1.yaml and v2.yaml
    let mut reg = PromptRegistry::load(dir.path().to_path_buf()).unwrap();
    reg.activate("test_prompt", "2").unwrap();
    assert_eq!(reg.get("test_prompt").unwrap().version, "2");
    let prev = reg.rollback("test_prompt").unwrap();
    assert_eq!(prev, "1");
    assert_eq!(reg.get("test_prompt").unwrap().version, "1");
}
```

---

### Phase 2 — Template Rendering via minijinja

**Goal**: `PromptRenderer::render()` substitutes all variables and returns a `RenderedPromptV2`.

#### Step 2.1 — Variable validation

File: `lazyjob-llm/src/prompts/renderer.rs`

`validate_variables()` iterates `template.variables`:
- For each `required: true` variable, check `variables.get(name).is_some()`.
  Collect all missing names and return `PromptError::MissingVariables(Vec<String>)` at once
  (not short-circuit) so the caller sees all errors in one shot.
- For each `required: false` variable with a `default`, insert the default into a working
  copy of the variables map before rendering.
- Type checking is intentionally shallow in Phase 1 (presence only, not type validation).
  Phase 3 adds type coercion.

#### Step 2.2 — minijinja rendering

```rust
use minijinja::{Environment, Value};

fn render_template(
    template_str: &str,
    variables: &PromptVariables,
) -> Result<String, PromptError> {
    let mut env = Environment::new();
    // Register the template string under a synthetic name.
    env.add_template("t", template_str)
        .map_err(|e| PromptError::TemplateCompile(e.to_string()))?;
    let tmpl = env.get_template("t").unwrap();

    // Build minijinja context from the JSON map.
    let ctx: std::collections::HashMap<&str, Value> = variables
        .as_map()
        .iter()
        .map(|(k, v)| (k.as_str(), json_to_minijinja_value(v)))
        .collect();

    tmpl.render(ctx)
        .map_err(|e| PromptError::TemplateRender(e.to_string()))
}

fn json_to_minijinja_value(v: &serde_json::Value) -> minijinja::Value {
    match v {
        serde_json::Value::Null => minijinja::Value::UNDEFINED,
        serde_json::Value::Bool(b) => minijinja::Value::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { minijinja::Value::from(i) }
            else { minijinja::Value::from(n.as_f64().unwrap_or(0.0)) }
        }
        serde_json::Value::String(s) => minijinja::Value::from(s.as_str()),
        serde_json::Value::Array(arr) => {
            minijinja::Value::from(arr.iter().map(json_to_minijinja_value).collect::<Vec<_>>())
        }
        serde_json::Value::Object(obj) => {
            let map: std::collections::BTreeMap<String, minijinja::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_minijinja_value(v)))
                .collect();
            minijinja::Value::from_serialize(map)
        }
    }
}
```

#### Step 2.3 — Schema loading and `RenderedPromptV2` construction

`PromptRenderer::render()` full implementation:

```rust
pub fn render(
    &self,
    name: &str,
    variables: &PromptVariables,
) -> Result<RenderedPromptV2, PromptError> {
    let template = self.registry.get(name)?;
    let mut effective_vars = variables.clone();

    // Apply defaults for missing optional variables.
    for decl in &template.variables {
        if !decl.required {
            if effective_vars.get(&decl.name).is_none() {
                if let Some(default) = &decl.default {
                    effective_vars.insert(decl.name.clone(), default.clone());
                }
            }
        }
    }

    Self::validate_variables(&template, &effective_vars)?;

    let content = render_template(&template.template, &effective_vars)?;

    let output_schema = if let Some(schema_path) = &template.output_schema {
        Some(Self::load_schema(&self.prompts_dir, schema_path)?)
    } else {
        None
    };

    Ok(RenderedPromptV2 {
        prompt_name: name.to_string(),
        version: template.version.clone(),
        content,
        output_schema,
        cache_system_prompt: template.cache_system_prompt,
    })
}
```

Verification:
```rust
#[test]
fn test_renderer_substitutes_all_variables() {
    // Given a template "Hello {{ name }}, you are {{ age }} years old."
    // with variables name=required, age=optional/default=0
    // When render() is called with name="Alice"
    // Then output is "Hello Alice, you are 0 years old."
}
```

---

### Phase 3 — Output Validation

**Goal**: LLM JSON responses are validated against the prompt's output schema.

#### Step 3.1 — `OutputValidator` construction

File: `lazyjob-llm/src/prompts/validator.rs`

```rust
impl OutputValidator {
    pub fn new(schema: serde_json::Value, schema_name: String) -> Result<Self, PromptError> {
        let compiled = jsonschema::JSONSchema::compile(&schema)
            .map_err(|e| PromptError::SchemaCompile {
                name: schema_name.clone(),
                reason: e.to_string(),
            })?;
        Ok(Self { compiled, schema_name })
    }

    pub fn validate(&self, raw_output: &str) -> Result<ValidationResult, PromptError> {
        let parsed: serde_json::Value = serde_json::from_str(raw_output)
            .map_err(|e| PromptError::OutputNotJson(e.to_string()))?;

        let errors: Vec<(String, String)> = self
            .compiled
            .validate(&parsed)
            .err()
            .map(|errors| {
                errors
                    .map(|e| (
                        e.instance_path.to_string(),
                        e.to_string(),
                    ))
                    .collect()
            })
            .unwrap_or_default();

        Ok(ValidationResult {
            passed: errors.is_empty(),
            errors,
        })
    }
}
```

`jsonschema::JSONSchema::compile()` returns a `Result<JSONSchema, ValidationError>`.
`compiled.validate(&instance)` returns `Result<(), ValidationErrors>` where
`ValidationErrors` is an iterator over individual `ValidationError` items.

#### Step 3.2 — Wire validator into `PromptRenderer`

Add a method `PromptRenderer::validate_output(rendered: &RenderedPromptV2, raw_output: &str)`:

```rust
pub fn validate_output(
    &self,
    rendered: &RenderedPromptV2,
    raw_output: &str,
) -> Result<ValidationResult, PromptError> {
    match &rendered.output_schema {
        None => Ok(ValidationResult { passed: true, errors: vec![] }),
        Some(schema) => {
            let validator = OutputValidator::new(schema.clone(), rendered.prompt_name.clone())?;
            validator.validate(raw_output)
        }
    }
}
```

Callers (ralph workers, pipeline stages) call this after receiving the LLM response and
before writing to SQLite. On `passed: false`, the worker logs validation errors via
`tracing::warn!` and may retry or return a structured error depending on the context.

Verification:
```rust
#[test]
fn test_validator_rejects_missing_required_field() {
    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "required": ["title"],
        "properties": { "title": { "type": "string" } }
    });
    let validator = OutputValidator::new(schema, "test".into()).unwrap();
    let result = validator.validate(r#"{"company": "Acme"}"#).unwrap();
    assert!(!result.passed);
    assert_eq!(result.errors[0].0, ""); // root-level missing property
}
```

---

### Phase 4 — Testing Harness

**Goal**: `PromptTester::test_version()` and `compare_versions()` work. `capture_sample()` writes
a `.sample.json` file to disk.

#### Step 4.1 — Sample I/O helpers

File: `lazyjob-llm/src/prompts/sample.rs`

```rust
impl SampleOutput {
    pub fn load(path: &std::path::Path) -> Result<Self, PromptError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| PromptError::Io(e))?;
        serde_json::from_str::<Self>(&content)
            .map_err(|e| PromptError::SampleParse(e.to_string()))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), PromptError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| PromptError::SampleSave(e.to_string()))?;
        std::fs::write(path, content.as_bytes())
            .map_err(|e| PromptError::Io(e))
    }
}
```

Sample file path convention: `versions/{name}/v{version}.sample.json`.
`PromptRegistry::sample_path(name, version)` returns this path.

#### Step 4.2 — Similarity scoring

`PromptTester::similarity_score(a: &str, b: &str) -> f32`:

For structured outputs (both are valid JSON): compare field-by-field presence/value equality.
For free text: compute Jaccard similarity on word-level token sets.

```rust
fn similarity_score(a: &str, b: &str) -> f32 {
    // Try JSON comparison first.
    if let (Ok(va), Ok(vb)) = (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        return json_similarity(&va, &vb);
    }
    // Fall back to word-level Jaccard similarity.
    word_jaccard(a, b)
}

fn word_jaccard(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if words_a.is_empty() && words_b.is_empty() { return 1.0; }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    intersection as f32 / union as f32
}

fn json_similarity(a: &serde_json::Value, b: &serde_json::Value) -> f32 {
    // Recursively count matching leaf values.
    let total = count_leaves(a) + count_leaves(b);
    if total == 0 { return 1.0; }
    let matching = count_matching_leaves(a, b) * 2;
    matching as f32 / total as f32
}
```

The similarity threshold for `TestResult::passed` is `>= 0.75`. This is deliberately lower
than the spec's suggested 0.85 — LLM outputs have natural variation even for identical inputs.
0.75 catches regressions (prompt no longer extracts required fields) without false-failing on
wording differences.

#### Step 4.3 — `test_version()` implementation

```rust
pub async fn test_version(
    &self,
    name: &str,
    version: Option<&str>,
) -> Result<TestResult, PromptError> {
    let renderer = PromptRenderer::new(self.registry.clone());
    let effective_version = match version {
        Some(v) => v.to_string(),
        None => self.registry.get(name)?.version,
    };

    let sample_path = self.registry.sample_path(name, &effective_version);
    let sample = SampleOutput::load(&sample_path)
        .map_err(|_| PromptError::NoSampleOutput {
            name: name.to_string(),
            version: effective_version.clone(),
        })?;

    // Convert sample_input JSON object to PromptVariables.
    let mut vars = PromptVariables::new();
    if let serde_json::Value::Object(map) = &sample.sample_input {
        for (k, v) in map { vars.insert(k.clone(), v.clone()); }
    }

    let rendered = renderer.render_version(name, &effective_version, &vars)?;

    let messages = vec![crate::ChatMessage {
        role: crate::MessageRole::User,
        content: rendered.content.clone(),
    }];
    let response = self.llm.chat(messages).await
        .map_err(|e| PromptError::LlmCall(e.to_string()))?;

    let raw_output = response.content.clone();
    let input_hash = hex_sha256(sample.sample_input.to_string().as_bytes());
    let output_hash = hex_sha256(raw_output.as_bytes());
    let similarity = Self::similarity_score(&raw_output, &sample.sample_output);

    let (schema_valid, schema_errors) = match renderer.validate_output(&rendered, &raw_output) {
        Ok(vr) => (vr.passed, vr.errors),
        Err(_) => (false, vec![("".into(), "output is not valid JSON".into())]),
    };

    Ok(TestResult {
        prompt_name: name.to_string(),
        version: effective_version,
        input_hash,
        output_hash,
        schema_valid,
        schema_errors,
        similarity_score: similarity,
        passed: schema_valid && similarity >= 0.75,
        token_usage: response.usage,
        cost_microdollars: 0, // caller may fill via BudgetEnforcer
    })
}
```

#### Step 4.4 — `compare_versions()` implementation

```rust
pub async fn compare_versions(
    &self,
    name: &str,
    v1: &str,
    v2: &str,
) -> Result<VersionComparison, PromptError> {
    let renderer = PromptRenderer::new(self.registry.clone());
    let sample_path = self.registry.sample_path(name, v1);
    let sample = SampleOutput::load(&sample_path)
        .map_err(|_| PromptError::NoSampleOutput {
            name: name.to_string(),
            version: v1.to_string(),
        })?;

    let mut vars = PromptVariables::new();
    if let serde_json::Value::Object(map) = &sample.sample_input {
        for (k, v) in map { vars.insert(k.clone(), v.clone()); }
    }

    let rendered_v1 = renderer.render_version(name, v1, &vars)?;
    let rendered_v2 = renderer.render_version(name, v2, &vars)?;

    let messages_v1 = vec![crate::ChatMessage {
        role: crate::MessageRole::User,
        content: rendered_v1.content.clone(),
    }];
    let messages_v2 = vec![crate::ChatMessage {
        role: crate::MessageRole::User,
        content: rendered_v2.content.clone(),
    }];

    let (resp_v1, resp_v2) = tokio::join!(
        self.llm.chat(messages_v1),
        self.llm.chat(messages_v2),
    );
    let resp_v1 = resp_v1.map_err(|e| PromptError::LlmCall(e.to_string()))?;
    let resp_v2 = resp_v2.map_err(|e| PromptError::LlmCall(e.to_string()))?;

    let diff = {
        use similar::{TextDiff, ChangeTag};
        let d = TextDiff::from_lines(&resp_v1.content, &resp_v2.content);
        let mut out = String::new();
        for group in d.grouped_ops(3) {
            for op in &group {
                for change in d.iter_inline_changes(op) {
                    let sign = match change.tag() {
                        ChangeTag::Equal => " ",
                        ChangeTag::Delete => "-",
                        ChangeTag::Insert => "+",
                    };
                    out.push_str(sign);
                    out.push_str(change.value());
                }
            }
        }
        out
    };

    let similarity = Self::similarity_score(&resp_v1.content, &resp_v2.content);

    Ok(VersionComparison {
        prompt_name: name.to_string(),
        v1: v1.to_string(),
        v2: v2.to_string(),
        v1_output: resp_v1.content,
        v2_output: resp_v2.content,
        diff,
        similarity_score: similarity,
        v1_tokens: resp_v1.usage.total_tokens,
        v2_tokens: resp_v2.usage.total_tokens,
    })
}
```

Verification:
```rust
// Integration test gated by LAZYJOB_TEST_LLM=1 env var to avoid CI cost.
#[tokio::test]
#[cfg_attr(not(feature = "integration"), ignore)]
async fn test_job_discovery_v1_passes() {
    let prompts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts");
    let registry = PromptRegistry::load(prompts_dir).unwrap();
    let llm = make_test_llm(); // AnthropicProvider from env ANTHROPIC_API_KEY
    let tester = PromptTester::new(registry, Arc::new(llm));
    let result = tester.test_version("job_discovery", None).await.unwrap();
    assert!(result.passed, "schema_errors: {:?}", result.schema_errors);
}
```

---

### Phase 5 — Audit Persistence and CLI

**Goal**: All version activations are logged to SQLite. `lazyjob prompt` CLI subcommands work.

#### Step 5.1 — Migration and repository

File: `lazyjob-core/src/prompt_log/repository.rs`

```rust
impl SqlitePromptActivationRepository {
    pub fn new(pool: sqlx::SqlitePool) -> Self { Self { pool } }

    pub async fn record_activation(
        &self,
        prompt_name: &str,
        from_version: Option<&str>,
        to_version: &str,
        action: &str,
        triggered_by: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"INSERT INTO prompt_activations
               (prompt_name, from_version, to_version, action, triggered_by)
               VALUES (?, ?, ?, ?, ?)"#,
            prompt_name,
            from_version,
            to_version,
            action,
            triggered_by,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_recent(
        &self,
        prompt_name: &str,
        limit: i64,
    ) -> Result<Vec<ActivationRecord>, sqlx::Error> {
        sqlx::query_as!(
            ActivationRecord,
            r#"SELECT id, prompt_name, from_version, to_version, action,
                      triggered_by, created_at
               FROM prompt_activations
               WHERE prompt_name = ?
               ORDER BY created_at DESC
               LIMIT ?"#,
            prompt_name,
            limit,
        )
        .fetch_all(&self.pool)
        .await
    }
}
```

#### Step 5.2 — CLI subcommands

File: `lazyjob-cli/src/commands/prompt.rs`

```rust
#[derive(clap::Subcommand)]
pub enum PromptCmd {
    /// List all known prompts and their active version.
    List,
    /// List all versions of a prompt.
    Versions { name: String },
    /// Activate a specific version of a prompt.
    Activate { name: String, version: String },
    /// Roll back to the previous version of a prompt.
    Rollback { name: String },
    /// Run a prompt against its sample input and report results.
    Test {
        name: String,
        #[arg(long)] version: Option<String>,
    },
    /// Compare two versions of a prompt against the same sample input.
    Compare { name: String, v1: String, v2: String },
    /// Capture LLM output as the sample for a version.
    CaptureSample {
        name: String,
        version: String,
        #[arg(long)] notes: Option<String>,
    },
    /// Show activation history for a prompt.
    History { name: String },
}
```

`PromptCmd::Activate` calls `registry.activate()` then `activation_repo.record_activation()`.
`PromptCmd::Rollback` calls `registry.rollback()` then `activation_repo.record_activation()`.
`PromptCmd::Test` calls `tester.test_version()` and prints a table:

```
Prompt: job_discovery (v3)
Input hash:  3a7f9c...
Output hash: 8b2e41...
Schema:      PASSED
Similarity:  0.83  [PASSED]
Tokens:      412 (est. $0.0003)
```

`PromptCmd::Compare` prints the diff and a side-by-side token cost comparison.

---

### Phase 6 — TUI Prompt Dev View (Deferred)

A `PromptDevView` accessible from `lazyjob prompt tui` subcommand:
- Left panel: prompt list with active version badge
- Right panel: version history table with activation log
- `[t]` to run test, `[a]` to activate, `[r]` to rollback
- `[c]` to open a comparison diff viewer (two panels using `ratatui::layout::Constraint::Ratio`)

Deferred because the CLI subcommands cover the developer workflow for MVP. The TUI view
is a QoL improvement for power users and can be added without changing any underlying logic.

---

## Key Crate APIs

- `minijinja::Environment::new()` — create a Jinja2 environment
- `minijinja::Environment::add_template(name, src)` — register a template string
- `minijinja::Template::render(ctx: HashMap<&str, Value>)` — render to `String`
- `minijinja::Value::from(x)` — convert primitives; `Value::from_serialize(x)` for objects
- `serde_yaml::from_str::<T>(s)` — deserialize YAML template files
- `jsonschema::JSONSchema::compile(schema: &serde_json::Value)` — compile once at construction
- `compiled.validate(instance)` — returns `Result<(), ValidationErrors>` where
  `ValidationErrors: IntoIterator<Item = ValidationError>`; each `ValidationError` has
  `.instance_path` and implements `Display` for the message
- `sha2::Sha256::digest(data)` from the `sha2` crate — returns `GenericArray<u8, _>`;
  format with `hex::encode(digest)` to get the hex string
- `similar::TextDiff::from_lines(old, new)` — compute line-level diff
- `similar::ChangeTag::{Equal, Delete, Insert}` — tag each line in the diff
- `toml::from_str::<T>(s)` / `toml::to_string_pretty(v)` — registry TOML I/O
- `std::fs::rename(from, to)` — atomic registry file write (rename from tmp)
- `sqlx::query!()` / `sqlx::query_as!()` — type-checked SQL for the activation table

---

## Error Handling

```rust
// lazyjob-llm/src/prompts/error.rs

#[derive(thiserror::Error, Debug)]
pub enum PromptError {
    #[error("unknown prompt: {0}")]
    UnknownPrompt(String),

    #[error("version not found: prompt={name}, version={version}")]
    VersionNotFound { name: String, version: String },

    #[error("cannot rollback prompt '{0}': only one version exists")]
    CannotRollback(String),

    #[error("template parse error: {0}")]
    TemplateParse(String),

    #[error("template compile error: {0}")]
    TemplateCompile(String),

    #[error("template render error: {0}")]
    TemplateRender(String),

    #[error("missing required variables: {0:?}")]
    MissingVariables(Vec<String>),

    #[error("schema compile error for '{name}': {reason}")]
    SchemaCompile { name: String, reason: String },

    #[error("LLM output is not valid JSON: {0}")]
    OutputNotJson(String),

    #[error("no sample output for prompt={name}, version={version}")]
    NoSampleOutput { name: String, version: String },

    #[error("sample parse error: {0}")]
    SampleParse(String),

    #[error("sample save error: {0}")]
    SampleSave(String),

    #[error("registry save failed: {0}")]
    RegistrySave(String),

    #[error("LLM call failed: {0}")]
    LlmCall(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

---

## Testing Strategy

### Unit Tests

**Registry:**
```rust
#[test]
fn test_activate_writes_toml() {
    // tempdir, create v1.yaml, call activate("prompt", "1")
    // read prompt_registry.toml and assert active["prompt"] == "1"
}

#[test]
fn test_rollback_previous_version() {
    // activate v2, then rollback — assert active is v1
}

#[test]
fn test_rollback_single_version_fails() {
    // only v1 exists, rollback returns CannotRollback
}
```

**Renderer:**
```rust
#[test]
fn test_missing_required_variable_error() {
    // template has required "name", render without it → MissingVariables(["name"])
}

#[test]
fn test_optional_variable_uses_default() {
    // template has optional "count" with default=0
    // render without it → "0" appears in output
}

#[test]
fn test_jinja2_if_block() {
    // template "{% if show %}yes{% endif %}"
    // variables {show: true} → "yes"
    // variables {show: false} → ""
}
```

**Validator:**
```rust
#[test]
fn test_validate_passes_on_valid_json() { ... }

#[test]
fn test_validate_fails_on_missing_required_field() { ... }

#[test]
fn test_validate_no_schema_always_passes() { ... }
```

**Similarity scorer:**
```rust
#[test]
fn test_identical_strings_score_1() { assert_eq!(similarity("abc", "abc"), 1.0); }

#[test]
fn test_disjoint_strings_score_0() { assert_eq!(similarity("apple", "orange"), 0.0); }
```

### Integration Tests (feature-gated)

Add a `features = ["integration"]` flag to `lazyjob-llm/Cargo.toml`. Integration tests are
`#[cfg_attr(not(feature = "integration"), ignore)]` and require `ANTHROPIC_API_KEY` env var.

```rust
// lazyjob-llm/tests/prompt_tests.rs

#[tokio::test]
#[cfg_attr(not(feature = "integration"), ignore)]
async fn job_discovery_v1_schema_passes() {
    // Load registry from test fixture directory.
    // Run test_version("job_discovery", None).
    // Assert result.schema_valid == true.
}
```

For CI: run without the integration feature so no API calls are made. A separate manual CI
job (or developer script `scripts/test-prompts.sh`) runs with `--features integration`.

### Mock LLM for Unit Tests

```rust
struct EchoLlm { response: String }

#[async_trait::async_trait]
impl crate::LlmProvider for EchoLlm {
    async fn chat(&self, _: Vec<crate::ChatMessage>) -> Result<crate::LlmResponse, crate::LlmError> {
        Ok(crate::LlmResponse {
            content: self.response.clone(),
            usage: crate::TokenUsage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
            model: "mock".into(),
        })
    }
    // ...other methods as no-ops
}
```

Use `EchoLlm` in `PromptTester` unit tests so no real LLM calls are made.

### CLI Tests

```bash
# In a test fixture directory with v1.yaml and v1.sample.json:
cargo run -p lazyjob-cli -- prompt list
cargo run -p lazyjob-cli -- prompt versions job_discovery
cargo run -p lazyjob-cli -- prompt activate job_discovery 1
cargo run -p lazyjob-cli -- prompt rollback job_discovery
```

---

## Open Questions

1. **Prompt file location**: The spec places prompts under `lazyjob-llm/src/prompts/versions/`.
   This plan moves them to `lazyjob-llm/prompts/` (outside `src/`) so they are not recompiled
   on each change. The prompts directory path must be configurable (via `config.toml`
   `[prompts] dir = "~/.config/lazyjob/prompts"`) so users can override them. The default
   fallback should ship a baseline set of prompts in the binary via `include_dir!` or as
   files installed alongside the binary by the package manager.

2. **Similarity threshold**: 0.75 is chosen as a reasonable starting point. Should be
   configurable per-prompt via a `min_similarity_threshold` field in the YAML. Some prompts
   (structured JSON extraction) are more deterministic and can be set to 0.90; free-text
   prompts may need 0.60.

3. **A/B testing**: The spec mentions A/B testing but provides no design. A full A/B framework
   would require routing some percentage of real ralph loop calls to the candidate version and
   collecting quality feedback. This is deferred post-MVP — the `compare_versions()` method
   provides a manual version comparison flow that covers the core use case.

4. **Per-user customization storage**: The spec asks about per-user prompt customization.
   The simplest mechanism: a user can place override YAML files in
   `~/.config/lazyjob/prompts/versions/{name}/` that shadow the bundled versions. The registry
   searches user dir first, then the bundled dir. This requires no database changes.

5. **Schema evolution**: When the output schema changes (e.g., a new required field is added),
   old version sample outputs will fail validation against the new schema. The
   `.sample.json` file references the schema path used at the time of capture via a
   `schema_version` field. The validator must resolve the schema path from the template, not
   from the sample — so re-running `test_version` on an old sample with a new schema will
   naturally produce the updated validation result.

6. **Template inheritance / partials**: The spec does not mention it but future prompts may
   share a system prompt boilerplate. `minijinja` supports `{% include "partial.jinja" %}`
   with a configured `source` loader. Adding a `PromptSource` loader that looks up other
   template files is a Phase 6 extension — Phase 1-5 assume single-file templates.

---

## Related Specs

- [specs/agentic-llm-provider-abstraction.md](./agentic-llm-provider-abstraction.md) — `LlmProvider` trait
- [specs/17-ralph-prompt-templates.md](./17-ralph-prompt-templates.md) — base prompt infrastructure
- [specs/XX-llm-cost-budget-management.md](./XX-llm-cost-budget-management.md) — cost attribution per prompt call
- [specs/XX-llm-function-calling.md](./XX-llm-function-calling.md) — structured output via function calling
- [specs/04-sqlite-persistence-implementation-plan.md](./04-sqlite-persistence-implementation-plan.md) — migration runner
