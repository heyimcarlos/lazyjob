# Research: Task 6 — life-sheet-yaml

## Existing Infrastructure
- `life_sheet_items` table exists in migration 001 with schema: `(id UUID PK, section TEXT, key TEXT, value JSONB, created_at, updated_at, UNIQUE(section, key))`
- `CoreError` in `lazyjob-core/src/error.rs` has variants for Db, Io, Parse, Validation, Serialization — sufficient for this task
- `serde_yaml = "0.9"` is in workspace deps but NOT yet added to lazyjob-core/Cargo.toml
- Database struct in `lazyjob-core/src/db.rs` provides `pool()` returning `&PgPool`

## Design Decisions
1. **Storage model**: Each LifeSheet section (basics, experience, education, etc.) stored as one row in `life_sheet_items` with section=category, key=identifier, value=JSONB blob
2. **Import strategy**: Parse YAML -> store each section as JSONB in life_sheet_items using INSERT...ON CONFLICT(section, key) DO UPDATE
3. **No separate DB tables per section**: The spec mentions 14+ tables but the existing migration uses a single `life_sheet_items` table with JSONB. This is simpler and already exists — we'll use it.
4. **JSON Resume export**: Map LifeSheet types to JSON Resume 1.0 schema fields (basics, work, education, skills, etc.)

## serde_yaml Notes
- `serde_yaml` 0.9 uses `yaml-rust2` under the hood
- API: `serde_yaml::from_str(&str) -> Result<T>`, `serde_yaml::to_string(&T) -> Result<String>`
- Need `From<serde_yaml::Error>` impl for CoreError

## Key Types to Define
- `LifeSheet` — top-level container
- `Basics` — name, label, email, phone, url, summary, location
- `Location` — city, region, country, remote_preference
- `WorkExperience` — company, position, dates, achievements, context
- `Achievement` — description, metric_type, metric_value, metric_unit
- `Education` — institution, degree, field, dates, score
- `Skill` — name, level, years, keywords
- `SkillCategory` — category name + Vec<Skill>
- `Certification` — name, authority, dates, url
- `Language` — name, proficiency
- `Project` — name, description, url, dates, highlights
- `JobPreferences` — types, locations, salary range, remote
- `CareerGoal` — short_term, long_term, timeline
