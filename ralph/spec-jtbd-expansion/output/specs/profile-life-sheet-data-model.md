# Spec: LifeSheet Data Model

**JTBD**: C-1 — Frame my non-linear background as a strength for a specific target role
**Topic**: Define the canonical schema for storing a user's complete professional identity as both a human-editable YAML file and a queryable SQLite database.
**Domain**: profile-resume

---

## What

The LifeSheet is LazyJob's master professional profile — a structured, queryable representation of everything a person has ever done professionally. It lives at `~/.lazyjob/life-sheet.yaml` as a human-editable YAML document and is imported into SQLite for programmatic access. It is the single source of truth from which resumes are tailored, cover letters are personalized, skill gaps are computed, and applications are pre-filled. Every AI feature in LazyJob draws exclusively from the LifeSheet, and all AI-generated content must be traceable to a LifeSheet entity (anti-fabrication constraint).

## Why

Without a structured, machine-readable career profile, every tailoring operation must start from scratch: parsing a PDF resume, extracting skills from free text, re-identifying relevant experience for each new job. This is slow, error-prone, and opens the door to AI hallucination. The LifeSheet solves this by front-loading structure: the user invests once in a well-formed YAML profile, and all downstream AI operations work against validated structured data. The YAML format gives users full transparency and control — they can read it, edit it, version-control it, and trust that the AI only works with what they've explicitly declared.

Career changers (JTBD C-1) are particularly served by the LifeSheet's `relevance_tags` and `goals` fields, which let users annotate their non-linear experience to highlight transferable value — something a conventional resume parser cannot infer.

## How

### Dual-Layer Architecture

The LifeSheet has two representations:
1. **YAML** (`~/.lazyjob/life-sheet.yaml`): Human-facing. Edited directly. Versioned with git if desired. Source of truth for `import_life_sheet`.
2. **SQLite tables** (`lazyjob_core` database): Machine-facing. Queried by resume tailoring, gap analysis, cover letter generation. Populated by `import_life_sheet` and kept synchronized.

Import is triggered manually via `lazyjob profile import` or automatically when the YAML file modification time is newer than `life_sheet_meta.updated_at`. Full re-import (truncate + re-insert) is the default strategy for correctness; incremental diffing is a Phase 2 enhancement (tracked by `life_sheet_meta.version_hash`).

### Contact Table Naming Convention

The `profile_contacts` table here is distinct from the `application_contacts` table in `04-sqlite-persistence.md`. They serve different purposes:
- `profile_contacts` (this spec): relationship network — former managers, mentors, peers. Feeds JTBD A-4 (networking).
- `application_contacts` (spec 04): hiring-process contacts — recruiters, interviewers. Feeds JTBD A-3 (tracking).

Both must coexist. Never merge them.

### ESCO/O*NET Skill Codes

Skills carry optional `esco_code` (URI like `http://data.europa.eu/esco/skill/abc123`) and `onet_code` fields. These are used by the gap analysis module to perform semantic skill matching beyond keyword overlap. In Phase 1, codes are optional and user-set. In Phase 2, a background job auto-suggests codes using the ESCO REST API or the embedded O*NET taxonomy bundle.

The `idx_skill_esco` and `idx_skill_onet` indexes are partial indexes (WHERE NOT NULL) — cheap to maintain, fast to query when gap analysis needs to do taxonomy lookups.

### Import/Export Paths

- `import_life_sheet(yaml: &LifeSheetYaml) -> Result<()>`: Parses YAML, truncates all life sheet tables, re-inserts all entities. Returns error on schema violation. Does NOT touch job/application tables.
- `export_json_resume(personal_id: &str) -> Result<JsonResume>`: Reads SQLite, maps to JSON Resume schema. Used for export and as input to `docx-rs` resume generator.

### YAML Schema (abbreviated)

```yaml
meta:
  version: "1.0"
  created_at: "YYYY-MM-DD"
  updated_at: "YYYY-MM-DD"

basics:
  name: string
  label: string          # Current/target title
  email: string
  phone: string
  url: string
  location: { city, region, country, remote: bool }
  summary: string
  profiles: [{ network, username, url }]

experience:
  - company: string
    position: string
    location: string
    url: string
    start_date: "YYYY-MM"
    end_date: "YYYY-MM" | null  # null = current
    current: bool
    summary: string
    context:
      team_size: int
      org_size: int
      industry: string
      tech_stack: [string]
    achievements:
      - description: string
        metrics: { improvement_percent?, timeframe_months?, ... }
        evidence: string
    skills:
      esco_codes: [uri]
      onet_codes: [code]
    relevance_tags: [string]    # User-defined tags for filtering

education:
  - institution, degree, field, start_date, end_date, score, thesis, courses, relevant_tags

skills:
  - name: string           # Category name
    level: string          # Advanced, Intermediate, etc.
    keywords:
      - name: string
        years: int
        proficiency: "expert" | "advanced" | "intermediate" | "beginner"

certifications: [{ name, authority, date, expiry_date, credential_id, url }]
languages: [{ name, proficiency }]

projects:
  - name, description, url, start_date, end_date, highlights, skills

preferences:
  job_types: [string]
  locations: [{ city, region, country, remote_ok }]
  industries: [string]
  salary: { currency, min, max, base_or_total }
  notice_period_weeks: int
  visa_sponsorship: bool

goals:
  short_term: string
  long_term: string
  timeline: string

profile_contacts:
  - name, relationship, company, email, linkedin_url, notes
```

## Interface

```rust
// lazyjob-core/src/life_sheet/mod.rs

/// The in-memory representation of the parsed YAML LifeSheet.
pub struct LifeSheet {
    pub basics: PersonalInfo,
    pub experience: Vec<WorkExperience>,
    pub education: Vec<EducationEntry>,
    pub skills: Vec<SkillCategory>,
    pub certifications: Vec<Certification>,
    pub languages: Vec<Language>,
    pub projects: Vec<Project>,
    pub preferences: JobPreferences,
    pub goals: Option<CareerGoal>,
    pub profile_contacts: Vec<ProfileContact>,
}

/// Repository trait — SQLite impl in lazyjob-core, PostgreSQL impl for SaaS.
#[async_trait]
pub trait LifeSheetRepository: Send + Sync {
    async fn get(&self) -> Result<LifeSheet>;
    async fn import(&self, yaml: &LifeSheetYaml) -> Result<()>;
    async fn export_json_resume(&self) -> Result<JsonResume>;
    async fn get_skills_flat(&self) -> Result<Vec<Skill>>;
    async fn get_experience_for_tailoring(&self) -> Result<Vec<WorkExperience>>;
}

/// Fabrication ground-truth check: is this claim traceable to the LifeSheet?
pub fn is_grounded_claim(claim: &str, life_sheet: &LifeSheet) -> bool;

// SQLite DDL lives in: lazyjob-core/migrations/001_life_sheet.sql
// YAML parsing struct lives in: lazyjob-core/src/life_sheet/yaml.rs
// Import logic lives in: lazyjob-core/src/life_sheet/import.rs
// Export logic lives in: lazyjob-core/src/life_sheet/export.rs
```

```sql
-- Key tables (full DDL in lazyjob-core/migrations/001_life_sheet.sql)
CREATE TABLE personal_info (id TEXT PK, name, label, email, phone, url, summary, city, region, country, remote_preference, updated_at);
CREATE TABLE work_experience (id TEXT PK, company_name, position, location, company_url, start_date, end_date, is_current, summary, team_size, org_size, industry, tech_stack JSON, updated_at);
CREATE TABLE achievement (id TEXT PK, experience_id FK, description, metric_type, metric_value, metric_unit, evidence);
CREATE TABLE skill_category (id TEXT PK, name, level, updated_at);
CREATE TABLE skill (id TEXT PK, category_id FK, name, years_experience, proficiency, esco_code, onet_code);
CREATE TABLE profile_contacts (id TEXT PK, name, relationship, company, email, linkedin_url, notes, updated_at);
-- + education, course, certification, language, project, project_skill, job_preferences, career_goal, life_sheet_meta

CREATE INDEX idx_skill_esco ON skill(esco_code) WHERE esco_code IS NOT NULL;
CREATE INDEX idx_skill_onet ON skill(onet_code) WHERE onet_code IS NOT NULL;
CREATE INDEX idx_experience_current ON work_experience(is_current) WHERE is_current = 1;
```

## Open Questions

- **Incremental YAML sync**: Should `import_life_sheet` diff the YAML against the current DB state and only update changed entities, or always do a full truncate+reimport? Full reimport is simpler and safer (no diff bugs), but it invalidates any application-linked resume version pointers if experience IDs change. Proposal: use deterministic IDs (hash of company+position+start_date) instead of random UUIDs so IDs are stable across re-imports.
- **GitHub integration**: Auto-populate `projects` by querying the GitHub API for public repos. This is additive (user can supplement), doesn't require scraping, and the API is free. Gating question: is the OAuth token scope worth adding in Phase 1?
- **LinkedIn import**: LinkedIn's data export (HTML+CSV) could be parsed to bootstrap the LifeSheet for new users. Against LinkedIn's ToS if we scrape, but parsing a user-downloaded export is legal. Worth implementing a `lazyjob profile import-linkedin-export <path>` command in Phase 2.
- **Multi-variant LifeSheets**: Should users maintain separate life sheets for "engineering track" vs "management track" applications? Proposal: defer — use `relevance_tags` for filtering, which accomplishes 80% of this without schema complexity.

## Implementation Tasks

- [ ] Define `LifeSheetYaml` serde structs in `lazyjob-core/src/life_sheet/yaml.rs` matching the YAML schema above
- [ ] Write SQLite DDL migration `lazyjob-core/migrations/001_life_sheet.sql` with all life sheet tables and indexes
- [ ] Implement `SqliteLifeSheetRepository` in `lazyjob-core/src/life_sheet/sqlite.rs` with `get`, `import`, `export_json_resume`, `get_skills_flat`, `get_experience_for_tailoring`
- [ ] Implement `import_life_sheet` in `lazyjob-core/src/life_sheet/import.rs` — parse YAML, truncate tables, re-insert all entities with deterministic IDs
- [ ] Implement `export_json_resume` in `lazyjob-core/src/life_sheet/export.rs` — map SQLite rows to JSON Resume schema for downstream consumers
- [ ] Add `is_grounded_claim` predicate in `lazyjob-core/src/life_sheet/fabrication.rs` used as the anti-fabrication check in resume tailoring
- [ ] Wire `lazyjob-cli` `profile import` and `profile export` subcommands to the repository trait
