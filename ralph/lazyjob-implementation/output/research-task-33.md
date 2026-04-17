# Research: Task 33 — Resume Tailoring Pipeline

## Architecture Constraint
- lazyjob-llm depends on lazyjob-core (for LifeSheet types in anti_fabrication)
- lazyjob-core CANNOT depend on lazyjob-llm (circular dependency)
- Solution: Use the existing `Completer` trait pattern from `lazyjob_core::discovery::company`
- `Completer::complete(&self, system: &str, user: &str) -> Result<String>` is already public

## Existing Infrastructure
- `ResumeTailorContext` / `ResumeTailorOutput` in lazyjob-llm::prompts::resume_tailor — prompt layer
- `validate_grounding()` in lazyjob-llm — anti-fabrication integration point
- `check_grounding()` / `prohibited_phrase_detector()` in lazyjob-llm::anti_fabrication
- MockLlmProvider in lazyjob-llm (not usable from lazyjob-core due to circular dep)
- Must create MockCompleter locally in lazyjob-core for tests

## Key Types from Codebase
- `Job` has `description: Option<String>` — raw JD text
- `LifeSheet` has `work_experience: Vec<WorkExperience>`, `skills: Vec<SkillCategory>`, etc.
- `WorkExperience` has `achievements: Vec<Achievement>`, `tech_stack: Vec<String>`
- `Achievement` has `description: String`, optional metrics
- `SkillCategory` has `skills: Vec<Skill>` where `Skill` has `name: String`

## Existing Migrations
- 001_initial_schema.sql (jobs, applications, etc.)
- 002_unique_job_source.sql
- 003_job_embeddings.sql
- Next: 004_resume_versions.sql

## Dependencies Needed
- `strsim` for Jaro-Winkler fuzzy matching (new)
- All other deps (serde, serde_json, uuid, chrono, sqlx, async-trait) already in lazyjob-core
