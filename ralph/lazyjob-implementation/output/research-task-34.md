# Research: Task 34 — DOCX Generator

## What Exists

- `crates/lazyjob-core/src/resume/types.rs` — All domain types: `ResumeContent`, `ExperienceSection`, `SkillsSection`, `EducationEntry`, `ProjectEntry`
- `crates/lazyjob-core/src/resume/mod.rs` — `ResumeTailor` orchestrator, produces `(ResumeContent, GapReport, FabricationReport)` tuple
- `crates/lazyjob-core/src/life_sheet/types.rs` — `Basics` struct (name, email, phone, url, location) serves as personal info; no `PersonalInfo` type exists
- No `docx-rs` dependency yet (adding it now)
- No `docx.rs` file exists in the resume module

## docx-rs API (v0.4.19)

- Import: `use docx_rs::*`
- `Docx::new()` — creates document builder
- `doc.add_paragraph(Paragraph)` — returns `Docx` (builder pattern)
- `Paragraph::new().add_run(Run).align(AlignmentType::Center)`
- `Run::new().add_text("text").bold().size(24)` — size in half-points (24 = 12pt)
- `doc.build().pack(writer)` — `pack()` needs `Write + Seek`, use `Cursor<&mut Vec<u8>>`
- `AlignmentType::Center`, `AlignmentType::Left`

## Design Decisions

1. Use `Basics` from life_sheet as personal info (not a new PersonalInfo type)
2. Font sizes: Name = 28 half-points (14pt), section headers = 24 (12pt bold), body = 22 (11pt)
3. Format: Calibri-style (docx-rs default font), consistent margins
4. `DocxGenerator::generate(resume: &ResumeContent, basics: &Basics) -> Result<Vec<u8>>`
5. `DocxGenerator::save_to_file(resume, basics, path)` — convenience method
6. Contact line: "email | phone | url | city, region" — only non-None fields
7. Skills formatted as comma-separated lists: "Primary: X, Y, Z | Secondary: A, B"
