# Plan: Task 33 — Resume Tailoring Pipeline

## Files to Create
1. `crates/lazyjob-core/src/resume/mod.rs` — module root + ResumeTailor orchestrator
2. `crates/lazyjob-core/src/resume/types.rs` — all domain types (TailoredResume, GapReport, etc.)
3. `crates/lazyjob-core/src/resume/jd_parser.rs` — LLM-backed JD parser + regex fallback
4. `crates/lazyjob-core/src/resume/gap_analyzer.rs` — pure gap analysis logic
5. `crates/lazyjob-core/src/resume/content_drafter.rs` — LLM bullet rewriting + summary
6. `crates/lazyjob-core/src/resume/fabrication.rs` — fabrication auditor (pure logic)
7. `crates/lazyjob-core/src/resume/repository.rs` — ResumeVersionRepository (PgPool)
8. `crates/lazyjob-core/migrations/004_resume_versions.sql` — PostgreSQL DDL

## Files to Modify
1. `crates/lazyjob-core/src/lib.rs` — add `pub mod resume`
2. `crates/lazyjob-core/Cargo.toml` — add `strsim` dependency
3. `Cargo.toml` (workspace root) — add `strsim` to workspace deps

## Types/Structs
- `TailoredResume` — final output with all sections
- `ResumeContent` — structured resume (summary, experience, skills, education)
- `ExperienceSection`, `SkillsSection`, `EducationEntry`, `ProjectEntry`
- `JobDescriptionAnalysis` — parsed JD output
- `SkillRequirement` — individual skill from JD
- `GapReport`, `MatchedSkill`, `MissingSkill`, `SkillEvidenceSource`
- `FabricationReport`, `FabricationItem`, `FabricationRisk`
- `TailoringOptions` — user-configurable options
- `ResumeVersionId` — newtype for UUID
- `ProgressEvent` — progress reporting enum

## Pipeline Stages (6-stage)
1. **JD Parse** — LLM call to extract skills/requirements, regex fallback
2. **Gap Analysis** — Pure logic: match LifeSheet skills to JD requirements
3. **Fabrication Pre-check** — Verify LifeSheet bullets are truthful
4. **Bullet Rewriting** — LLM rewrites bullets for matched skills
5. **Summary Generation** — LLM generates 3-sentence summary
6. **Assembly** — Combine into TailoredResume struct

## Tests
- JD parser: mock completer returns canned JSON, verify parsed skills
- JD parser regex fallback: parse without LLM
- Gap analyzer: fixture LifeSheet + JD, verify match score
- Gap analyzer: missing skills detected correctly
- Gap analyzer: fuzzy matching with strsim
- Fabrication auditor: clean resume passes
- Fabrication auditor: fabricated skill flagged
- Content drafter: mock completer returns canned bullets
- Orchestrator: full pipeline with mock completer
- Repository: save and list (integration test with TestDb)

## Migration (PostgreSQL)
```sql
CREATE TABLE resume_versions (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id UUID REFERENCES applications(id) ON DELETE SET NULL,
    label TEXT NOT NULL DEFAULT 'v1',
    content_json JSONB NOT NULL,
    gap_report_json JSONB NOT NULL,
    fabrication_report_json JSONB NOT NULL,
    options_json JSONB NOT NULL,
    is_submitted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```
