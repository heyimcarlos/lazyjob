use std::path::Path;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

use super::types::LifeSheet;

pub fn parse_yaml(content: &str) -> Result<LifeSheet> {
    let sheet: LifeSheet = serde_yaml::from_str(content)?;
    validate(&sheet)?;
    Ok(sheet)
}

pub fn serialize_yaml(sheet: &LifeSheet) -> Result<String> {
    let yaml = serde_yaml::to_string(sheet)?;
    Ok(yaml)
}

fn validate(sheet: &LifeSheet) -> Result<()> {
    if sheet.basics.name.trim().is_empty() {
        return Err(crate::error::CoreError::Validation(
            "basics.name is required".into(),
        ));
    }
    if sheet.work_experience.is_empty() && sheet.education.is_empty() {
        return Err(crate::error::CoreError::Validation(
            "at least one work_experience or education entry is required".into(),
        ));
    }
    Ok(())
}

pub async fn import_from_yaml(path: &Path, pool: &PgPool) -> Result<LifeSheet> {
    let content = std::fs::read_to_string(path)?;
    let sheet = parse_yaml(&content)?;
    upsert_to_db(&sheet, pool).await?;
    Ok(sheet)
}

pub async fn load_from_db(pool: &PgPool) -> Result<LifeSheet> {
    let rows: Vec<(String, String, serde_json::Value)> =
        sqlx::query_as("SELECT section, key, value FROM life_sheet_items ORDER BY section, key")
            .fetch_all(pool)
            .await?;

    let mut basics = None;
    let mut work_experience = Vec::new();
    let mut education = Vec::new();
    let mut skills = Vec::new();
    let mut certifications = Vec::new();
    let mut languages = Vec::new();
    let mut projects = Vec::new();
    let mut preferences = None;
    let mut goals = None;

    for (section, _key, value) in &rows {
        match section.as_str() {
            "basics" => basics = Some(serde_json::from_value(value.clone())?),
            "work_experience" => work_experience = serde_json::from_value(value.clone())?,
            "education" => education = serde_json::from_value(value.clone())?,
            "skills" => skills = serde_json::from_value(value.clone())?,
            "certifications" => certifications = serde_json::from_value(value.clone())?,
            "languages" => languages = serde_json::from_value(value.clone())?,
            "projects" => projects = serde_json::from_value(value.clone())?,
            "preferences" => preferences = Some(serde_json::from_value(value.clone())?),
            "goals" => goals = Some(serde_json::from_value(value.clone())?),
            _ => {}
        }
    }

    let basics = basics.ok_or_else(|| crate::error::CoreError::NotFound {
        entity: "LifeSheet",
        id: "basics".into(),
    })?;

    Ok(LifeSheet {
        basics,
        work_experience,
        education,
        skills,
        certifications,
        languages,
        projects,
        preferences,
        goals,
    })
}

async fn upsert_to_db(sheet: &LifeSheet, pool: &PgPool) -> Result<()> {
    let sections: Vec<(&str, &str, serde_json::Value)> = vec![
        ("basics", "default", serde_json::to_value(&sheet.basics)?),
        (
            "work_experience",
            "default",
            serde_json::to_value(&sheet.work_experience)?,
        ),
        (
            "education",
            "default",
            serde_json::to_value(&sheet.education)?,
        ),
        ("skills", "default", serde_json::to_value(&sheet.skills)?),
        (
            "certifications",
            "default",
            serde_json::to_value(&sheet.certifications)?,
        ),
        (
            "languages",
            "default",
            serde_json::to_value(&sheet.languages)?,
        ),
        (
            "projects",
            "default",
            serde_json::to_value(&sheet.projects)?,
        ),
        (
            "preferences",
            "default",
            serde_json::to_value(&sheet.preferences)?,
        ),
        ("goals", "default", serde_json::to_value(&sheet.goals)?),
    ];

    for (section, key, value) in sections {
        sqlx::query(
            "INSERT INTO life_sheet_items (id, section, key, value)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (section, key) DO UPDATE SET value = $4, updated_at = now()",
        )
        .bind(Uuid::new_v4())
        .bind(section)
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_yaml() -> &'static str {
        include_str!("../../tests/fixtures/life-sheet.yaml")
    }

    #[test]
    fn parse_yaml_succeeds_for_valid_input() {
        let sheet = parse_yaml(fixture_yaml()).unwrap();
        assert_eq!(sheet.basics.name, "Jane Doe");
        assert_eq!(sheet.work_experience.len(), 2);
    }

    #[test]
    fn parse_yaml_rejects_empty_name() {
        let yaml = r#"
basics:
  name: "   "
work_experience:
  - company: Co
    position: Dev
    start_date: "2023-01"
"#;
        let err = parse_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("basics.name is required"));
    }

    #[test]
    fn parse_yaml_rejects_no_experience_or_education() {
        let yaml = r#"
basics:
  name: Test User
"#;
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            err.to_string()
                .contains("at least one work_experience or education")
        );
    }

    #[test]
    fn parse_yaml_accepts_education_only() {
        let yaml = r#"
basics:
  name: Student
education:
  - institution: MIT
"#;
        let sheet = parse_yaml(yaml).unwrap();
        assert!(sheet.work_experience.is_empty());
        assert_eq!(sheet.education.len(), 1);
    }

    #[test]
    fn serialize_and_reparse_roundtrip() {
        let original = parse_yaml(fixture_yaml()).unwrap();
        let serialized = serialize_yaml(&original).unwrap();
        let reparsed = parse_yaml(&serialized).unwrap();
        assert_eq!(original, reparsed);
    }

    #[tokio::test]
    async fn import_and_load_roundtrip() {
        let db = crate::test_db::TestDb::spawn().await;

        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/life-sheet.yaml");
        let imported = import_from_yaml(&fixture_path, db.pool()).await.unwrap();
        let loaded = load_from_db(db.pool()).await.unwrap();

        assert_eq!(imported.basics.name, loaded.basics.name);
        assert_eq!(imported.work_experience.len(), loaded.work_experience.len());
        assert_eq!(imported.education.len(), loaded.education.len());
        assert_eq!(imported.skills.len(), loaded.skills.len());
    }
}
