use std::io::Cursor;

use docx_rs::{Docx, Paragraph, Run};

use crate::error::Result;
use crate::life_sheet::Basics;

use super::types::CoverLetterVersion;

pub struct CoverLetterDocxGenerator;

impl CoverLetterDocxGenerator {
    pub fn generate(version: &CoverLetterVersion, basics: &Basics) -> Result<Vec<u8>> {
        let contact_line = build_contact_line(basics);
        let date_line = chrono::Utc::now().format("%B %d, %Y").to_string();

        let mut doc = Docx::new();

        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&basics.name).bold().size(28)),
        );

        if !contact_line.is_empty() {
            doc = doc.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text(&contact_line).size(20)),
            );
        }

        doc = doc.add_paragraph(Paragraph::new());

        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(&date_line).size(22)));

        doc = doc.add_paragraph(Paragraph::new());

        for para in version.plain_text.split("\n\n") {
            let trimmed = para.trim();
            if !trimmed.is_empty() {
                doc = doc
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text(trimmed).size(22)));
            }
        }

        let mut buf = Vec::new();
        doc.build()
            .pack(&mut Cursor::new(&mut buf))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(buf)
    }

    pub fn save_to_file(
        version: &CoverLetterVersion,
        basics: &Basics,
        path: &std::path::Path,
    ) -> Result<()> {
        let bytes = Self::generate(version, basics)?;
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
    if let Some(loc) = &basics.location {
        let mut loc_parts = Vec::new();
        if let Some(city) = &loc.city {
            loc_parts.push(city.clone());
        }
        if let Some(region) = &loc.region {
            loc_parts.push(region.clone());
        }
        if let Some(country) = &loc.country {
            loc_parts.push(country.clone());
        }
        if !loc_parts.is_empty() {
            parts.push(loc_parts.join(", "));
        }
    }
    parts.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover_letter::types::{
        CoverLetterId, CoverLetterLength, CoverLetterOptions, CoverLetterTemplate, CoverLetterTone,
    };
    use crate::life_sheet::Location;

    fn mock_basics() -> Basics {
        Basics {
            name: "Alice Smith".into(),
            label: None,
            email: Some("alice@example.com".into()),
            phone: Some("555-1234".into()),
            url: None,
            summary: None,
            location: Some(Location {
                city: Some("Portland".into()),
                region: Some("OR".into()),
                country: Some("US".into()),
                remote_preference: None,
            }),
        }
    }

    fn mock_version() -> CoverLetterVersion {
        CoverLetterVersion {
            id: CoverLetterId::new(),
            job_id: uuid::Uuid::new_v4(),
            application_id: None,
            version: 1,
            template: CoverLetterTemplate::StandardProfessional,
            content: "Dear Hiring Manager,\n\nI am writing to apply.\n\nThank you for your time."
                .into(),
            plain_text:
                "Dear Hiring Manager,\n\nI am writing to apply.\n\nThank you for your time.".into(),
            key_points: vec![],
            tone: CoverLetterTone::Professional,
            length: CoverLetterLength::Standard,
            options: CoverLetterOptions::default(),
            diff_from_previous: None,
            is_submitted: false,
            label: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn generate_returns_non_empty_bytes() {
        let bytes = CoverLetterDocxGenerator::generate(&mock_version(), &mock_basics()).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn generate_starts_with_pk_magic() {
        let bytes = CoverLetterDocxGenerator::generate(&mock_version(), &mock_basics()).unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }

    #[test]
    fn build_contact_line_all_fields() {
        let basics = mock_basics();
        let line = build_contact_line(&basics);
        assert!(line.contains("alice@example.com"));
        assert!(line.contains("555-1234"));
        assert!(line.contains("Portland"));
    }

    #[test]
    fn build_contact_line_no_fields() {
        let basics = Basics {
            name: "Bob".into(),
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
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_cover_letter_{}.docx", uuid::Uuid::new_v4()));
        CoverLetterDocxGenerator::save_to_file(&mock_version(), &mock_basics(), &path).unwrap();
        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
        std::fs::remove_file(&path).ok();
    }
}
