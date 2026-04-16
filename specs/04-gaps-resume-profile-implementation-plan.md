# 04-gaps-resume-profile — Implementation Plan

## Spec Reference
- **Spec file**: `specs/04-gaps-resume-profile.md`
- **Status**: Gap analysis (research document)
- **Last updated**: 2026-04-15

## Executive Summary
This gap analysis identifies 10 critical gaps across resume/profile specs (GAP-39 through GAP-48). The implementation plan addresses all gaps in priority order, with critical gaps (ATS optimization, version management) requiring new spec creation and implementation, while others may be addressed as part of existing spec iterations.

## Problem Statement
The resume/profile feature set has significant gaps in ATS optimization, version management, multi-targeting per job, achievement extraction, cold-start data import, template system, feedback loop tracking, PDF export, voice preservation, and LifeSheet incremental sync.

## Implementation Phases

### Phase 1: Critical Gaps — ATS Optimization & Version Management

#### GAP-39: ATS-Specific Resume Optimization (CRITICAL)
Create new spec `XX-ats-specific-optimization.md` covering:
- **Per-ATS parsing profiles**: Workday (strict 255-char limits, section parsing), Greenhouse (keyword matching, exact terminology), Lever (relaxed format), iCIMS
- **ATS parse simulation**: Before submission, parse generated resume through ATS library to verify indexing
- **Format detection**: CompanyRecord.ats_type field; Greenhouse/Lever publicly known; Workday detectable via URL patterns
- **Implementation**: `lazyjob-ats` crate with `AtsProfile` trait, implementations for each ATS type

#### GAP-40: Resume Version Management System (CRITICAL)
Create new spec `XX-resume-version-management.md` covering:
- **Version naming**: User-defined names (e.g., "Engineering Manager - Stripe v1")
- **Version comparison**: Diff view between any two versions (textual or unified)
- **Version branching**: Create new version based on existing with modifications
- **Version tagging**: Tag versions as "sent to X company" linked to application_id
- **Version templates**: Use any version as template for new tailoring session
- **Auto-pruning**: Keep last N versions per job application, archive older ones

#### GAP-43: Master Resume Cold Start (IMPORTANT)
Create new spec `XX-life-sheet-cold-start.md` covering:
- **LinkedIn import**: Parse LinkedIn profile export (HTML/CSV) into LifeSheet structure
- **Resume parsing**: Extract work history, education, skills from PDF/DOCX
- **Conversational bootstrap**: Chat-based data entry with AI structuring
- **GitHub integration**: Pull repos, contributions via GitHub API
- **Progressive enrichment**: Start minimal, enrich incrementally

### Phase 2: Important Gaps

#### GAP-41: Multiple Resume Targets Per Job
- Extend ResumeVersion schema: add `target_angle` field (enum: Technical, Leadership, Management, etc.)
- One job can have multiple ResumeVersions with different angles
- Track which angle led to interview/outcome
- A/B testing infrastructure: expose angle as experimental variable

#### GAP-42: Automated Achievement Extraction
- LLM extraction from job descriptions: given JD, extract quantifiable achievements
- Generate strong resume bullets from job descriptions + user input
- Achievement library: reusable achievement snippets keyed by skill/category
- Evidence quality scoring: strong metrics vs vague claims
- Implementation: `lazyjob-core/achievements.rs` module

#### GAP-44: Resume Template System
- Multiple templates: ATS-minimal, clean modern, executive
- Template customization: fonts, colors, spacing via YAML config
- Template versioning: user custom templates versioned alongside built-ins
- Template preview: ASCII/WYSIWYG preview before DOCX generation
- Implementation: `lazyjob-templates` crate with Handlebars templating

### Phase 3: Moderate Gaps

#### GAP-45: Resume Feedback Loop Tracking
- Link ResumeVersion to Application on apply
- Record interview outcome per resume version
- Score effectiveness by outcome (rejection → interview → offer → accepted)
- Learn from outcomes to adjust future keyword targeting

#### GAP-46: PDF Export Pipeline
- DOCX → PDF via LibreOffice headless or cloud API
- Preserve formatting quality
- ATS compatibility verification
- Hybrid: submit DOCX to ATS, PDF to hiring manager

#### GAP-47: Voice and Writing Style Preservation
- Writing style extraction from user's existing resume/experience descriptions
- Style fingerprint: formality, sentence length, vocabulary level metrics
- Style consistency checking on generated content
- Anti-uniformity detection: compare against generic AI-resume corpus

#### GAP-48: Incremental LifeSheet Sync
- YAML diff detection: detect changes between imports
- Selective update: only changed entities
- Deterministic entity IDs: hash of (company + position + start_date)
- Application impact analysis: propagate ID changes to linked applications
- Conflict resolution: handle concurrent YAML edit + DB import

## Data Model

### New Tables (SQLite)

```sql
-- Resume version management
CREATE TABLE resume_versions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT,                    -- User-defined name
    target_angle TEXT,            -- Technical/Leadership/Management
    content_docx BLOB,
    content_yaml TEXT,
    source_spec TEXT,              -- life_sheet/job_description
    parent_version_id TEXT,       -- For branching
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (parent_version_id) REFERENCES resume_versions(id)
);

-- Resume version tags (sent to X company)
CREATE TABLE resume_version_tags (
    id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    tag_name TEXT NOT NULL,
    application_id TEXT,
    tagged_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (resume_version_id) REFERENCES resume_versions(id),
    FOREIGN KEY (application_id) REFERENCES applications(id)
);

-- Achievement library
CREATE TABLE achievements (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    skill_category TEXT,
    bullet_text TEXT NOT NULL,     -- "Reduced latency by 40%"
    metric_value REAL,
    metric_unit TEXT,              -- "%"/"dollars"/"hours"
    source_job_id TEXT,            -- Which job this was extracted from
    quality_score REAL,           -- 0.0-1.0
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Resume template customization
CREATE TABLE resume_templates (
    id TEXT PRIMARY KEY,
    user_id TEXT,                  -- NULL for built-in templates
    name TEXT NOT NULL,
    config_yaml TEXT NOT NULL,      -- Font, color, spacing config
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- ATS-specific optimization profiles
CREATE TABLE ats_profiles (
    id TEXT PRIMARY KEY,
    ats_type TEXT NOT NULL,         -- workday/greenhouse/lever/icims
    company_pattern TEXT,           -- URL or name pattern to detect
    section_limits_yaml TEXT,
    keyword_weights_yaml TEXT,
    parsing_rules_yaml TEXT
);
```

### Schema Extensions

```sql
-- Add to existing applications or resume_versions
ALTER TABLE applications ADD COLUMN primary_resume_version_id TEXT;
ALTER TABLE resume_versions ADD COLUMN ats_optimized_for TEXT;
```

## API Surface

### New Crate: lazyjob-ats
```rust
pub trait AtsProfile {
    fn parse(&self, docx_bytes: &[u8]) -> AtsParseResult;
    fn score_keyword(&self, keyword: &str) -> f32;
    fn format_for_submission(&self, docx_bytes: &[u8]) -> FormattedResume;
}

pub struct AtsSimulator {
    profiles: Vec<Box<dyn AtsProfile>>,
}
```

### New Crate: lazyjob-resume (core logic)
```rust
pub mod versions;
pub mod achievements;
pub mod templates;
pub mod feedback;

pub struct ResumeService {
    repo: Arc<dyn ResumeVersionRepo>,
    achievements: Arc<AchievementLibrary>,
    ats_simulator: Arc<AtsSimulator>,
}
```

### TUI Commands
- `resume version list <job_id>` — list all versions for a job
- `resume version diff <v1_id> <v2_id>` — show diff
- `resume version branch <v1_id> --name "New Angle"` — branch
- `resume version tag <v1_id> --sent-to <app_id>` — tag as sent
- `resume achievement add` — add to library
- `resume achievement list` — browse library
- `resume template preview <template_id>` — ASCII preview
- `resume coldstart import-linkedin <file>` — LinkedIn import

## Key Technical Decisions

1. **ATS parse simulation**: Parse DOCX with our own implementation vs external service. Decision: Rust docx parsing + rule-based simulation (no external deps, works offline).

2. **Version branching model**: Git-style branching. Parent pointer creates version graph. Circular refs prevented at DB level.

3. **Achievement library storage**: SQLite keyed by user_id. Not YAML (too large). LLM extracts and stores; user can edit.

4. **Template engine**: Use Handlebars for Rust (`handlebars` crate). Templates are YAML-configurable DOCX generation.

5. **Voice preservation**: Store user's writing samples encrypted. Extract style fingerprint on demand. Apply via prompt engineering (few-shot).

6. **Incremental sync ID stability**: Deterministic IDs via SHA256(company + position + start_date). Survives re-import. Manual override if collision.

## File Structure

```
lazyjob/
├── lazyjob-core/
│   ├── src/
│   │   ├── resume/
│   │   │   ├── mod.rs
│   │   │   ├── versions.rs
│   │   │   ├── achievements.rs
│   │   │   └── feedback.rs
│   │   └── lib.rs
│   └── Cargo.toml
├── lazyjob-ats/                      # NEW
│   ├── src/
│   │   ├── mod.rs
│   │   ├── workday.rs
│   │   ├── greenhouse.rs
│   │   ├── lever.rs
│   │   └── simulator.rs
│   └── Cargo.toml
├── lazyjob-templates/                # NEW
│   ├── src/
│   │   ├── mod.rs
│   │   ├── engine.rs
│   │   └── config.rs
│   ├── templates/
│   │   ├── ats-minimal.hbs
│   │   ├── clean-modern.hbs
│   │   └── executive.hbs
│   └── Cargo.toml
├── lazyjob-tui/
│   └── src/
│       ├── commands/
│       │   └── resume.rs
│       └── views/
│           └── resume/
│               ├── version_list.rs
│               ├── version_diff.rs
│               └── template_preview.rs
└── lazyjob-importers/                 # NEW
    ├── src/
    │   ├── linkedin.rs
    │   └── pdf_parser.rs
    └── Cargo.toml
```

## Dependencies

- `handlebars` — Rust template engine for DOCX generation
- `docx` — DOCX reading/writing (verify ATS parsing)
- `similar` — Text diff for version comparison
- `sha2` — Deterministic ID hashing for incremental sync
- `serde_yaml` — Template configuration parsing
- `rusqlite` with `rusqlite-migration` — Schema migrations

## Testing Strategy

- **ATS parsing tests**: Use known-good resumes, verify parsing output matches expected structure
- **Version branching tests**: Create branch, verify parent pointer, prevent circular refs
- **Achievement extraction tests**: Run LLM on sample JDs, verify quantifiable metrics extracted
- **Cold start tests**: Import LinkedIn HTML export, verify life_sheet YAML correctness
- **Template rendering tests**: Generate DOCX, verify structure integrity
- **TUI integration tests**: Test command output, verify diff view renders correctly

## Open Questions

1. **ATS detection accuracy**: How to reliably detect which ATS a company uses? (URL patterns, LinkedIn company page, manual override)
2. **Achievement deduplication**: When same achievement appears across multiple jobs, merge or keep separate?
3. **Template marketplace**: Should we support community-shared templates? Requires content moderation.
4. **Voice fingerprint storage**: Encrypt at rest? Key derivation from master password?
5. **PDF conversion quality**: Which converter (LibreOffice headless vs cloud API) for best quality/privacy tradeoff?

## Effort Estimate

- **Phase 1 (Critical)**: 3-4 weeks
  - ATS optimization: 1.5 weeks (new crate, parsing rules, simulation)
  - Version management: 1 week (UI, diff, branching)
  - Cold start: 1.5 weeks (LinkedIn parser, GitHub integration)
- **Phase 2 (Important)**: 2-3 weeks
  - Multi-target: 0.5 weeks (schema + UI)
  - Achievement extraction: 1.5 weeks (LLM pipeline + library)
  - Template system: 1 week (Handlebars + preview)
- **Phase 3 (Moderate)**: 2-3 weeks
  - Feedback loop: 0.5 weeks
  - PDF export: 1 week
  - Voice preservation: 1 week
  - Incremental sync: 0.5 weeks

**Total**: 7-10 weeks across all phases
