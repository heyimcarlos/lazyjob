# Life Sheet Data Model — Implementation Plan

## Spec Reference
- **Spec file**: `specs/03-life-sheet-data-model.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
The Life Sheet is LazyJob's structured career profile — a machine-readable, human-editable YAML document that replaces the traditional static resume. It stores work experience with quantified achievements, education, skills with ESCO/O*NET taxonomy codes, certifications, languages, projects, job preferences, and career goals. The implementation spans a YAML parser/serializer, SQLite data layer with conversion logic, and export to JSON Resume format.

## Problem Statement
Job seekers need a career profile that is both human-editable (for direct manipulation) and machine-readable (for AI processing). Traditional resumes are static documents. The Life Sheet solves this by being a YAML-first, SQLite-backed data repository that supports rich metadata, taxonomy mapping, and application linking.

## Implementation Phases

### Phase 1: Foundation — Data Structures & YAML Layer
1. Define `LifeSheetYaml` structs in `lazyjob-core/src/life_sheet/yaml.rs`
2. Use **serde_yaml** for YAML parsing/serialization
3. Implement `LifeSheetYaml::parse()` and `LifeSheetYaml::serialize()`
4. Create `~/.lazyjob/life-sheet.yaml` default template on first run
5. Add validation for required fields (name, at least one experience or education)

### Phase 2: Core Implementation — SQLite Data Model
1. Define all SQL tables from spec in `lazyjob-core/src/life_sheet/schema.sql`
2. Create `LifeSheetRepository` struct with CRUD operations
3. Implement `import_life_sheet(yaml: &LifeSheetYaml)` — full YAML → SQLite conversion
4. Implement `export_life_sheet() -> LifeSheetYaml` — SQLite → YAML round-trip
5. Implement `export_json_resume() -> JsonResume` — for JSON Resume compatibility
6. Add migrations using rusqlite's migrations pattern (see `04-sqlite-persistence.md`)
7. Partial update logic: detect changed YAML sections vs full re-import

### Phase 3: Integration & Polish
1. Wire LifeSheet into `lazyjob-cli` for `life-sheet edit`, `life-sheet import`, `life-sheet export`
2. Add ESCO/O*NET code validation (optional API call, graceful fallback if offline)
3. Link work experience to job applications (foreign key relationship)
4. Contact network CRUD operations
5. Skills gap analysis queries (skills for job vs skills in profile)
6. Unit tests for YAML parsing, round-trip conversion, SQL queries

## Data Model

### New SQLite Tables
All tables defined in spec section "SQLite Data Model":
- `life_sheet_meta` — version metadata
- `personal_info` — name, contact, summary, location
- `work_experience` — companies, positions, dates, context (team size, org size, industry, tech stack)
- `achievement` — quantified achievements with metrics (type, value, unit, evidence)
- `education` — schools, degrees, fields, thesis
- `course` — per-education course list
- `skill_category` — skill group name and level
- `skill` — individual skills with ESCO/O*NET codes
- `certification` — credentials with expiry
- `language` — proficiency levels
- `project` — portfolio projects
- `project_skill` — many-to-many project-skill mapping
- `profile` — social network links (GitHub, LinkedIn, Twitter)
- `job_preferences` — job types, locations, industries, salary range
- `career_goal` — short/long term goals and timeline
- `contact` — networking contact relationships

### New Rust Types
```rust
// lazyjob-core/src/life_sheet/mod.rs
pub struct LifeSheetYaml { meta, basics, experience, education, skills, certifications, languages, projects, preferences, goals, contact_network }
pub struct PersonalInfo { id, name, label, email, phone, url, summary, city, region, country, remote_preference }
pub struct WorkExperience { id, company_name, position, location, url, start_date, end_date, is_current, summary, team_size, org_size, industry, tech_stack }
pub struct Achievement { id, experience_id, description, metric_type, metric_value, metric_unit, evidence }
pub struct Education { id, institution, degree, field, area, start_date, end_date, score, thesis }
pub struct Skill { id, category_id, name, years_experience, proficiency, esco_code, onet_code }
pub struct Certification { id, name, authority, issue_date, expiry_date, credential_id, url }
pub struct Language { id, name, proficiency }
pub struct Project { id, name, description, url, start_date, end_date, highlights }
pub struct JobPreference { id, job_types, locations, industries, salary_currency, salary_min, salary_max, base_or_total, notice_period_weeks, visa_sponsorship }
pub struct CareerGoal { id, short_term, long_term, timeline }
pub struct Contact { id, name, relationship, company, email, linkedin_url, twitter_handle, notes }
```

### Migration Approach
- Migrations stored in `lazyjob-core/migrations/`
- Use **rusqlite-migrations** or manual migration runner
- Migration `001_create_life_sheet_tables` creates all tables above
- Subsequent migrations for schema changes (add columns, new tables)

## API Surface

### Public Crate: `lazyjob_core::life_sheet`
```rust
// YAML interface
pub fn parse_life_sheet_yaml(content: &str) -> Result<LifeSheetYaml>
pub fn serialize_life_sheet_yaml(sheet: &LifeSheetYaml) -> Result<String>

// SQLite interface
pub struct LifeSheetRepository { db: Connection }
impl LifeSheetRepository {
    pub fn new(db: &Connection) -> Self
    pub fn import(&self, yaml: &LifeSheetYaml) -> Result<()>
    pub fn export(&self) -> Result<LifeSheetYaml>
    pub fn get_personal_info(&self) -> Result<PersonalInfo>
    pub fn get_work_experiences(&self) -> Result<Vec<WorkExperience>>
    pub fn get_achievements(&self, experience_id: &str) -> Result<Vec<Achievement>
    pub fn get_skills(&self) -> Result<Vec<Skill>>
    pub fn get_job_preferences(&self) -> Result<JobPreference>
    pub fn get_contact(&self, id: &str) -> Result<Contact>
    pub fn update_contact(&self, contact: &Contact) -> Result<()>
}

// Export
pub fn to_json_resume(repo: &LifeSheetRepository, personal_id: &str) -> Result<JsonResume>
```

### Integration Points
- `lazyjob-cli` — `life-sheet` subcommand (edit, import, export)
- `lazyjob-ralph` — reads LifeSheet for prompt context (resume tailoring, cover letter generation)
- `lazyjob-tui` — displays profile view, edit dialogs

## Key Technical Decisions

1. **YAML via serde_yaml (yaml-rust2)**: Rust-native, no C bindings. Handles our complex nested structure.
2. **Partial import vs full re-import**: On YAML change, compute diff and only update changed sections. This preserves SQLite IDs for foreign key relationships.
3. **ESCO/O*NET codes stored but not validated by default**: Validation requires external API calls. Store codes for future lookup; validate lazily on-demand.
4. **tech_stack stored as JSON array**: Simple array in SQLite TEXT column, parsed on read.
5. **No graph database**: Despite `contact_network` being graph-like, use simple flat contact table. Graph queries not needed for single-user local tool.
6. **JSON Resume export for compatibility**: Existing tooling (resume builders, ATS parsers) understands JSON Resume. Provide export path even though our schema is richer.

## File Structure
```
lazyjob/
  lazyjob-core/
    src/
      life_sheet/
        mod.rs          # Public API re-exports
        yaml.rs         # YAML structs, parse, serialize
        repository.rs   # SQLite repository
        export.rs       # JSON Resume export
        schema.sql      # Table definitions
    migrations/
      001_create_life_sheet_tables.sql
  lazyjob-cli/
    src/
      commands/
        life_sheet.rs   # life-sheet edit/import/export CLI
  lazyjob-ralph/
    src/
      context.rs        # Reads LifeSheet for prompt context
  lazyjob-tui/
    src/
      views/
        profile.rs      # Profile TUI view
```

## Dependencies
- **serde_yaml** (~0.9): YAML parsing and serialization
- **rusqlite** (~0.31): SQLite access
- **chrono** (~0.4): Date handling (ISO 8601 parsing for `start_date`, `end_date`)
- **serde** (~1.0): Serialization derives for all structs
- **thiserror**: Error handling
- **uuid**: Generate table IDs

## Testing Strategy
1. **Unit tests for YAML parsing**: Parse known YAML, verify fields
2. **Round-trip test**: YAML → SQLite → YAML, verify equivalence
3. **JSON Resume export test**: Export and validate against json-resume schema
4. **Partial update test**: Update one experience, verify others unchanged
5. **Repository tests**: Mock DB connection, test each CRUD method
6. **Integration test with TUI**: Load life-sheet view, verify rendering

## Open Questions

1. **YAML Validation for ESCO/O*NET codes**: Should we validate against official taxonomies? Requires API calls to ESCO portal or O*NET. Decision: Validate lazily on-demand, not on import.
2. **Resume Versioning**: Multiple resume variants (Engineering vs Management focus)? Decision: Defer to Phase 2. Store variant in `life_sheet_meta` for now.
3. **LinkedIn Import**: Direct import technically possible but ToS issues. Decision: Offer manual YAML editing, future CSV import from LinkedIn export.
4. **GitHub Integration**: Auto-pull repos? Decision: Phase 2. Manual for MVP.
5. **Partial Updates**: Diff-based incremental import vs full re-import? Decision: Implement diff-based for performance, but full re-import as fallback.

## Effort Estimate
**Rough: 3-4 days**

- Phase 1 (YAML layer): 0.5 day — straightforward serde derive
- Phase 2 (SQLite layer): 2 days — 14 tables, import/export logic, migrations
- Phase 3 (CLI/TUI integration): 1 day — commands, views, tests
- Buffer: 0.5 day — open questions resolution, edge cases
