# Plan: Task 34 — DOCX Generator

## Files to Create/Modify

1. **Create** `crates/lazyjob-core/src/resume/docx.rs` — DocxGenerator implementation
2. **Modify** `crates/lazyjob-core/src/resume/mod.rs` — add `pub mod docx;`
3. **Modified** `Cargo.toml` — add `docx-rs = "0.4"` to workspace deps (done)
4. **Modified** `crates/lazyjob-core/Cargo.toml` — add `docx-rs = { workspace = true }` (done)

## Types/Functions to Define

### In `docx.rs`:
- `pub struct DocxGenerator` — zero-sized struct, stateless renderer
- `DocxGenerator::generate(resume: &ResumeContent, basics: &Basics) -> Result<Vec<u8>>` — main method
- `DocxGenerator::save_to_file(resume: &ResumeContent, basics: &Basics, path: &Path) -> Result<()>`
- `fn build_contact_line(basics: &Basics) -> String` — helper
- `fn add_section_heading(doc: Docx, title: &str) -> Docx` — helper
- `fn add_experience_entry(doc: Docx, exp: &ExperienceSection) -> Docx` — helper
- `fn add_education_entry(doc: Docx, edu: &EducationEntry) -> Docx` — helper
- `fn add_project_entry(doc: Docx, proj: &ProjectEntry) -> Docx` — helper
- `fn format_skills(skills: &SkillsSection) -> String` — helper

## Tests to Write

### Learning tests:
- `docx_rs_creates_valid_zip` — proves Docx::new().build().pack() produces valid ZIP bytes starting with PK magic
- `docx_rs_paragraph_with_bold_run` — proves Run::new().add_text().bold().size() compiles and produces non-empty bytes

### Unit tests:
- `generate_returns_non_empty_bytes` — minimal ResumeContent + Basics, assert non-empty output
- `generate_starts_with_pk_magic_bytes` — ZIP magic bytes verification
- `generate_with_full_content` — full resume with experience, skills, education, projects, certifications
- `generate_with_empty_sections` — handles empty experience/education/projects gracefully
- `build_contact_line_all_fields` — email + phone + url + location
- `build_contact_line_partial_fields` — only email
- `build_contact_line_no_fields` — empty Basics
- `save_to_file_creates_file` — writes to tempdir, verify file exists and is non-empty
- `format_skills_both_categories` — primary + secondary
- `format_skills_primary_only` — no secondary
