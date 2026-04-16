# Plan: Task 6 — life-sheet-yaml

## Files to Create/Modify

### Create
- `lazyjob-core/src/life_sheet/mod.rs` — public API re-exports
- `lazyjob-core/src/life_sheet/types.rs` — LifeSheet YAML serde types
- `lazyjob-core/src/life_sheet/service.rs` — LifeSheetService (import/export)
- `lazyjob-core/src/life_sheet/json_resume.rs` — JSON Resume export types + conversion
- `lazyjob-core/tests/fixtures/life-sheet.yaml` — sample fixture

### Modify
- `lazyjob-core/Cargo.toml` — add serde_yaml dependency
- `lazyjob-core/src/lib.rs` — add `pub mod life_sheet`
- `lazyjob-core/src/error.rs` — add From<serde_yaml::Error>

## Types to Define

### life_sheet/types.rs
- `LifeSheet { basics, work_experience, education, skills, certifications, languages, projects, preferences, goals }`
- `Basics { name, label, email, phone, url, summary, location }`
- `Location { city, region, country, remote_preference }`
- `WorkExperience { company, position, location, url, start_date, end_date, is_current, summary, achievements, team_size, industry, tech_stack }`
- `Achievement { description, metric_type, metric_value, metric_unit }`
- `Education { institution, degree, field, start_date, end_date, score, thesis }`
- `SkillCategory { name, level, skills }`
- `Skill { name, years_experience, proficiency }`
- `Certification { name, authority, issue_date, expiry_date, url }`
- `Language { name, proficiency }`
- `Project { name, description, url, start_date, end_date, highlights }`
- `JobPreferences { job_types, locations, salary_currency, salary_min, salary_max, remote, notice_period_weeks }`
- `CareerGoal { short_term, long_term, timeline }`

### life_sheet/service.rs
- `LifeSheetService` (stateless, methods take PgPool)
- `parse_yaml(content: &str) -> Result<LifeSheet>`
- `serialize_yaml(sheet: &LifeSheet) -> Result<String>`
- `import_from_yaml(path: &Path, pool: &PgPool) -> Result<LifeSheet>`
- `load_from_db(pool: &PgPool) -> Result<LifeSheet>`

### life_sheet/json_resume.rs
- `JsonResume` struct matching JSON Resume 1.0 schema
- `LifeSheet::to_json_resume() -> JsonResume`

## Tests
- **Learning test**: serde_yaml round-trip — parse YAML string, serialize back, verify fields
- **Unit tests**: parse fixture YAML, validate all fields populated; serialize and re-parse roundtrip; JSON Resume export maps correctly
- **Integration tests**: import_from_yaml + load_from_db roundtrip (DB-dependent, skip if no DATABASE_URL)
