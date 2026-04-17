# Plan — Task 35: Cover Letter Generation

## Files to Create

1. `crates/lazyjob-core/migrations/005_cover_letter_versions.sql` — PostgreSQL DDL
2. `crates/lazyjob-core/src/cover_letter/mod.rs` — CoverLetterService orchestrator
3. `crates/lazyjob-core/src/cover_letter/types.rs` — Domain types
4. `crates/lazyjob-core/src/cover_letter/generator.rs` — LLM cover letter generation
5. `crates/lazyjob-core/src/cover_letter/repository.rs` — PostgreSQL persistence
6. `crates/lazyjob-core/src/cover_letter/docx.rs` — DOCX export

## Files to Modify

7. `Cargo.toml` — Add `similar = "2"` to workspace deps
8. `crates/lazyjob-core/Cargo.toml` — Add `similar = { workspace = true }`
9. `crates/lazyjob-core/src/lib.rs` — Add `pub mod cover_letter`
10. `crates/lazyjob-cli/src/main.rs` — Add `CoverLetter` command

## Types (types.rs)

- `CoverLetterId(pub Uuid)` — newtype
- `CoverLetterTemplate` — enum: StandardProfessional, ProblemSolution, CareerChanger
- `CoverLetterTone` — enum: Professional, Casual, Creative
- `CoverLetterLength` — enum: Short(200), Standard(300), Detailed(400)
- `CoverLetterOptions` — tone, length, template, quick_mode
- `CoverLetterVersion` — id, job_id, application_id, version, content, plain_text, key_points, tone, length, template, diff_from_previous, is_submitted, label, created_at
- `ProgressEvent` — enum: Researching, Generating, Checking, Persisting, Done, Error

## Generator (generator.rs)

- `CoverLetterGenerator::new(completer: Arc<dyn Completer>)`
- `generate(job, life_sheet, template, tone, length) -> Result<String>` — builds template-specific prompts
- `extract_key_points(content) -> Vec<String>`
- `to_plain_text(content) -> String`
- `format_relevant_experience(life_sheet, job) -> String`

## Repository (repository.rs)

- `CoverLetterRepository::new(pool: PgPool)`
- `save(version: &CoverLetterVersion) -> Result<()>`
- `get(id: &CoverLetterId) -> Result<Option<CoverLetterVersion>>`
- `list_for_job(job_id: &Uuid) -> Result<Vec<CoverLetterVersionSummary>>`
- `pin_to_application(id, application_id) -> Result<()>`
- `count_for_job(job_id: &Uuid) -> Result<i64>`

## Service (mod.rs)

- `CoverLetterService::new(completer: Arc<dyn Completer>)`
- `generate(job, life_sheet, options, progress_tx) -> Result<CoverLetterVersion>` — orchestrates: generate → anti-fab check → persist
- Static helper: `build_version(...)` to assemble CoverLetterVersion

## Tests

- Learning test: `similar_text_diff_produces_unified_diff`
- Unit: template prompt construction, key_points extraction, plain_text stripping
- Unit: CoverLetterTemplate::description(), CoverLetterLength::word_target()
- Repository: save_and_get, list_for_job, pin_to_application, count_for_job
- Service: generate creates version with anti-fab check
- CLI: parse_cover_letter_generate
