use std::io::Cursor;
use std::path::Path;

use docx_rs::{AlignmentType, Docx, Paragraph, Run};

use crate::error::Result;
use crate::life_sheet::Basics;

use super::types::{EducationEntry, ExperienceSection, ProjectEntry, ResumeContent, SkillsSection};

const NAME_SIZE: usize = 28;
const SECTION_HEADING_SIZE: usize = 24;
const BODY_SIZE: usize = 22;
const CONTACT_SIZE: usize = 20;

pub struct DocxGenerator;

impl DocxGenerator {
    pub fn generate(resume: &ResumeContent, basics: &Basics) -> Result<Vec<u8>> {
        let mut doc = Docx::new();

        doc = doc.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text(&basics.name).bold().size(NAME_SIZE))
                .align(AlignmentType::Center),
        );

        let contact_line = build_contact_line(basics);
        if !contact_line.is_empty() {
            doc = doc.add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text(&contact_line).size(CONTACT_SIZE))
                    .align(AlignmentType::Center),
            );
        }

        if !resume.summary.is_empty() {
            doc = add_section_heading(doc, "Professional Summary");
            doc = doc.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text(&resume.summary).size(BODY_SIZE)),
            );
        }

        if !resume.experience.is_empty() {
            doc = add_section_heading(doc, "Experience");
            for exp in &resume.experience {
                doc = add_experience_entry(doc, exp);
            }
        }

        if !resume.skills.primary.is_empty() || !resume.skills.secondary.is_empty() {
            doc = add_section_heading(doc, "Skills");
            let skills_text = format_skills(&resume.skills);
            doc = doc.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text(&skills_text).size(BODY_SIZE)),
            );
        }

        if !resume.education.is_empty() {
            doc = add_section_heading(doc, "Education");
            for edu in &resume.education {
                doc = add_education_entry(doc, edu);
            }
        }

        if !resume.projects.is_empty() {
            doc = add_section_heading(doc, "Projects");
            for proj in &resume.projects {
                doc = add_project_entry(doc, proj);
            }
        }

        if !resume.certifications.is_empty() {
            doc = add_section_heading(doc, "Certifications");
            let certs_text = resume.certifications.join(", ");
            doc = doc.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text(&certs_text).size(BODY_SIZE)),
            );
        }

        let mut buf = Vec::new();
        doc.build()
            .pack(Cursor::new(&mut buf))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(buf)
    }

    pub fn save_to_file(resume: &ResumeContent, basics: &Basics, path: &Path) -> Result<()> {
        let bytes = Self::generate(resume, basics)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

fn build_contact_line(basics: &Basics) -> String {
    let mut parts = Vec::new();
    if let Some(email) = &basics.email {
        parts.push(email.clone());
    }
    if let Some(phone) = &basics.phone {
        parts.push(phone.clone());
    }
    if let Some(url) = &basics.url {
        parts.push(url.clone());
    }
    if let Some(location) = &basics.location {
        let loc_parts: Vec<&str> = [
            location.city.as_deref(),
            location.region.as_deref(),
            location.country.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !loc_parts.is_empty() {
            parts.push(loc_parts.join(", "));
        }
    }
    parts.join(" | ")
}

fn add_section_heading(doc: Docx, title: &str) -> Docx {
    doc.add_paragraph(
        Paragraph::new().add_run(Run::new().add_text(title).bold().size(SECTION_HEADING_SIZE)),
    )
}

fn add_experience_entry(mut doc: Docx, exp: &ExperienceSection) -> Docx {
    let title_line = format!("{} — {}", exp.title, exp.company);
    doc = doc.add_paragraph(
        Paragraph::new()
            .add_run(Run::new().add_text(&title_line).bold().size(BODY_SIZE))
            .add_run(
                Run::new()
                    .add_text(format!("  {}", exp.date_range))
                    .italic()
                    .size(BODY_SIZE),
            ),
    );

    for bullet in &exp.bullets {
        doc = doc.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(format!("  \u{2022} {bullet}"))
                    .size(BODY_SIZE),
            ),
        );
    }

    doc
}

fn add_education_entry(doc: Docx, edu: &EducationEntry) -> Docx {
    let mut line = format!("{} in {}", edu.degree, edu.field);
    if let Some(year) = edu.graduation_year {
        line.push_str(&format!(", {year}"));
    }
    doc.add_paragraph(
        Paragraph::new()
            .add_run(Run::new().add_text(&line).bold().size(BODY_SIZE))
            .add_run(
                Run::new()
                    .add_text(format!("  {}", edu.institution))
                    .size(BODY_SIZE),
            ),
    )
}

fn add_project_entry(mut doc: Docx, proj: &ProjectEntry) -> Docx {
    let mut title_run = Run::new().add_text(&proj.name).bold().size(BODY_SIZE);
    if let Some(url) = &proj.url {
        title_run = title_run.add_text(format!(" ({url})"));
    }
    doc = doc.add_paragraph(Paragraph::new().add_run(title_run));

    if !proj.description.is_empty() {
        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&proj.description).size(BODY_SIZE)),
        );
    }

    if !proj.technologies.is_empty() {
        let tech_text = format!("Technologies: {}", proj.technologies.join(", "));
        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&tech_text).italic().size(BODY_SIZE)),
        );
    }

    doc
}

fn format_skills(skills: &SkillsSection) -> String {
    let mut parts = Vec::new();
    if !skills.primary.is_empty() {
        parts.push(skills.primary.join(", "));
    }
    if !skills.secondary.is_empty() {
        parts.push(format!("Also: {}", skills.secondary.join(", ")));
    }
    parts.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::Location;

    fn minimal_basics() -> Basics {
        Basics {
            name: "Jane Doe".into(),
            label: None,
            email: Some("jane@example.com".into()),
            phone: None,
            url: None,
            summary: None,
            location: None,
        }
    }

    fn full_basics() -> Basics {
        Basics {
            name: "Jane Doe".into(),
            label: Some("Senior Engineer".into()),
            email: Some("jane@example.com".into()),
            phone: Some("+1-555-1234".into()),
            url: Some("https://janedoe.dev".into()),
            summary: None,
            location: Some(Location {
                city: Some("San Francisco".into()),
                region: Some("CA".into()),
                country: Some("US".into()),
                remote_preference: None,
            }),
        }
    }

    fn full_resume() -> ResumeContent {
        ResumeContent {
            summary: "Experienced engineer specializing in Rust and distributed systems.".into(),
            experience: vec![ExperienceSection {
                company: "Acme Corp".into(),
                title: "Senior Software Engineer".into(),
                date_range: "2021-03 \u{2013} Present".into(),
                bullets: vec![
                    "Reduced API latency by 40% through caching layer redesign".into(),
                    "Mentored 3 junior engineers".into(),
                ],
                rewritten_indices: vec![0],
            }],
            skills: SkillsSection {
                primary: vec!["Rust".into(), "PostgreSQL".into(), "Kubernetes".into()],
                secondary: vec!["Python".into(), "TypeScript".into()],
            },
            education: vec![EducationEntry {
                degree: "B.S.".into(),
                field: "Computer Science".into(),
                institution: "MIT".into(),
                graduation_year: Some(2018),
                gpa: Some(3.8),
            }],
            projects: vec![ProjectEntry {
                name: "LazyJob".into(),
                description: "AI-powered job search TUI".into(),
                technologies: vec!["Rust".into(), "ratatui".into()],
                url: Some("https://github.com/example/lazyjob".into()),
            }],
            certifications: vec!["AWS Solutions Architect".into()],
        }
    }

    // learning test: verifies docx-rs creates a valid ZIP archive
    #[test]
    fn docx_rs_creates_valid_zip() {
        let doc = Docx::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Hello, World!")));
        let mut buf = Vec::new();
        doc.build()
            .pack(Cursor::new(&mut buf))
            .expect("pack should succeed");
        assert!(!buf.is_empty());
        assert_eq!(
            &buf[..2],
            b"PK",
            "DOCX files are ZIP archives starting with PK"
        );
    }

    // learning test: verifies Run builder methods chain correctly
    #[test]
    fn docx_rs_paragraph_with_bold_run() {
        let doc = Docx::new().add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text("Bold Text").bold().size(28))
                .align(AlignmentType::Center),
        );
        let mut buf = Vec::new();
        doc.build()
            .pack(Cursor::new(&mut buf))
            .expect("pack should succeed");
        assert!(buf.len() > 100, "DOCX should have substantial content");
    }

    #[test]
    fn generate_returns_non_empty_bytes() {
        let resume = ResumeContent {
            summary: "Test summary".into(),
            ..Default::default()
        };
        let basics = minimal_basics();
        let bytes = DocxGenerator::generate(&resume, &basics).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn generate_starts_with_pk_magic_bytes() {
        let resume = ResumeContent::default();
        let basics = minimal_basics();
        let bytes = DocxGenerator::generate(&resume, &basics).unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }

    #[test]
    fn generate_with_full_content() {
        let resume = full_resume();
        let basics = full_basics();
        let bytes = DocxGenerator::generate(&resume, &basics).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..2], b"PK");
        assert!(
            bytes.len() > 1000,
            "Full resume should produce substantial DOCX"
        );
    }

    #[test]
    fn generate_with_empty_sections() {
        let resume = ResumeContent::default();
        let basics = Basics {
            name: "Empty Resume".into(),
            label: None,
            email: None,
            phone: None,
            url: None,
            summary: None,
            location: None,
        };
        let bytes = DocxGenerator::generate(&resume, &basics).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..2], b"PK");
    }

    #[test]
    fn build_contact_line_all_fields() {
        let basics = full_basics();
        let line = build_contact_line(&basics);
        assert!(line.contains("jane@example.com"));
        assert!(line.contains("+1-555-1234"));
        assert!(line.contains("https://janedoe.dev"));
        assert!(line.contains("San Francisco"));
        assert!(line.contains("CA"));
        assert!(line.contains(" | "));
    }

    #[test]
    fn build_contact_line_partial_fields() {
        let basics = minimal_basics();
        let line = build_contact_line(&basics);
        assert_eq!(line, "jane@example.com");
    }

    #[test]
    fn build_contact_line_no_fields() {
        let basics = Basics {
            name: "Nobody".into(),
            label: None,
            email: None,
            phone: None,
            url: None,
            summary: None,
            location: None,
        };
        let line = build_contact_line(&basics);
        assert!(line.is_empty());
    }

    #[test]
    fn save_to_file_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_resume.docx");
        let resume = full_resume();
        let basics = full_basics();
        DocxGenerator::save_to_file(&resume, &basics, &path).unwrap();
        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    fn format_skills_both_categories() {
        let skills = SkillsSection {
            primary: vec!["Rust".into(), "Go".into()],
            secondary: vec!["Python".into()],
        };
        let text = format_skills(&skills);
        assert_eq!(text, "Rust, Go | Also: Python");
    }

    #[test]
    fn format_skills_primary_only() {
        let skills = SkillsSection {
            primary: vec!["Rust".into()],
            secondary: vec![],
        };
        let text = format_skills(&skills);
        assert_eq!(text, "Rust");
    }
}
