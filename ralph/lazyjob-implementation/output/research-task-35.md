# Research — Task 35: Cover Letter Generation

## Existing Infrastructure

### Completer Trait (lazyjob-core)
- Defined in `crate::discovery::company` — `async fn complete(&self, system: &str, user: &str) -> Result<String>`
- Same pattern used by resume module — avoids circular dep with lazyjob-llm

### Anti-Fabrication (lazyjob-llm)
- `check_grounding(claims: &[String], life_sheet: &LifeSheet) -> GroundingReport`
- `prohibited_phrase_detector(text: &str) -> Vec<ProhibitedPhrase>`
- `prompt_injection_guard(user_input: &str) -> bool`
- Already wired into `lazyjob_llm::prompts::cover_letter::validate_grounding()`

### Prompt Templates (lazyjob-llm)
- `CoverLetterContext` with `to_template_vars()` — has user_name, company_name, job_title, company_research, relevant_experience, job_description_summary
- `CoverLetterOutput` — paragraphs: Vec<String>, template_type: String, subject_line, key_themes
- `system_prompt()`, `user_prompt(&ctx)`, `validate_output(raw)`

### Resume Module Pattern (reference)
- ResumeTailor takes `Arc<dyn Completer>`, clones to sub-components
- Progress via `Option<mpsc::Sender<ProgressEvent>>`
- Repository uses JSONB columns with `serde_json::to_value`/`from_value`
- `#[derive(sqlx::FromRow)]` private row structs

### Domain Types
- `Job` — id: JobId, title: String, company_name: Option<String>, description: Option<String>, url: Option<String>
- `LifeSheet` — basics: Basics, work_experience: Vec<WorkExperience>, skills: Vec<SkillCategory>, etc.
- `Basics` — name, email, phone, url, location
- `WorkExperience` — company, position, achievements: Vec<Achievement>, tech_stack: Vec<String>

### Migration Numbering
- Existing: 001, 002, 003, 004 — next is 005

### Dependencies
- `similar` crate needed for version diffs — not yet in workspace
- `docx-rs` already in workspace deps
- All other deps (serde, sqlx, tokio, async-trait, chrono, uuid) already available

## Key Design Decisions

1. **Three templates**: StandardProfessional, ProblemSolution, CareerChanger — each gets a different system prompt structure
2. **Anti-fabrication is mandatory**: Run `check_grounding` + `prohibited_phrase_detector` on every generated letter
3. **Version management**: Each generation creates a new version with monotonic version number per job
4. **Diff tracking**: Use `similar::TextDiff` to compute unified diff from previous version
5. **DOCX export**: Reuse pattern from resume docx.rs with Basics for contact info
6. **PostgreSQL storage**: JSONB for options, TEXT for content/plain_text, version number via MAX+1 query
