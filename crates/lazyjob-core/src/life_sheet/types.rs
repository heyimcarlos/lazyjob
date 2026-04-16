use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifeSheet {
    pub basics: Basics,
    #[serde(default)]
    pub work_experience: Vec<WorkExperience>,
    #[serde(default)]
    pub education: Vec<Education>,
    #[serde(default)]
    pub skills: Vec<SkillCategory>,
    #[serde(default)]
    pub certifications: Vec<Certification>,
    #[serde(default)]
    pub languages: Vec<Language>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub preferences: Option<JobPreferences>,
    #[serde(default)]
    pub goals: Option<CareerGoals>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Basics {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Location {
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub remote_preference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkExperience {
    pub company: String,
    pub position: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub start_date: String,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub is_current: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub achievements: Vec<Achievement>,
    #[serde(default)]
    pub team_size: Option<u32>,
    #[serde(default)]
    pub industry: Option<String>,
    #[serde(default)]
    pub tech_stack: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Achievement {
    pub description: String,
    #[serde(default)]
    pub metric_type: Option<String>,
    #[serde(default)]
    pub metric_value: Option<String>,
    #[serde(default)]
    pub metric_unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Education {
    pub institution: String,
    #[serde(default)]
    pub degree: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub score: Option<String>,
    #[serde(default)]
    pub thesis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillCategory {
    pub name: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub skills: Vec<Skill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub years_experience: Option<u32>,
    #[serde(default)]
    pub proficiency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Certification {
    pub name: String,
    #[serde(default)]
    pub authority: Option<String>,
    #[serde(default)]
    pub issue_date: Option<String>,
    #[serde(default)]
    pub expiry_date: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Language {
    pub name: String,
    #[serde(default)]
    pub proficiency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobPreferences {
    #[serde(default)]
    pub job_types: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub salary_currency: Option<String>,
    #[serde(default)]
    pub salary_min: Option<i64>,
    #[serde(default)]
    pub salary_max: Option<i64>,
    #[serde(default)]
    pub remote: Option<bool>,
    #[serde(default)]
    pub notice_period_weeks: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CareerGoals {
    #[serde(default)]
    pub short_term: Option<String>,
    #[serde(default)]
    pub long_term: Option<String>,
    #[serde(default)]
    pub timeline: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies serde_yaml can round-trip a nested struct
    #[test]
    fn serde_yaml_round_trip() {
        let yaml = r#"
basics:
  name: Test User
  email: test@example.com
work_experience:
  - company: TestCo
    position: Engineer
    start_date: "2020-01"
education: []
"#;
        let sheet: LifeSheet = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(sheet.basics.name, "Test User");
        assert_eq!(sheet.work_experience.len(), 1);

        let serialized = serde_yaml::to_string(&sheet).unwrap();
        let reparsed: LifeSheet = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(sheet, reparsed);
    }

    // learning test: verifies serde_yaml handles missing optional fields with defaults
    #[test]
    fn serde_yaml_defaults_for_optional_fields() {
        let yaml = r#"
basics:
  name: Minimal User
work_experience:
  - company: Co
    position: Dev
    start_date: "2023-01"
"#;
        let sheet: LifeSheet = serde_yaml::from_str(yaml).unwrap();
        assert!(sheet.basics.email.is_none());
        assert!(sheet.basics.location.is_none());
        assert!(sheet.education.is_empty());
        assert!(sheet.skills.is_empty());
        assert!(sheet.preferences.is_none());
        assert!(sheet.goals.is_none());
    }

    #[test]
    fn parse_fixture_yaml() {
        let content = include_str!("../../tests/fixtures/life-sheet.yaml");
        let sheet: LifeSheet = serde_yaml::from_str(content).unwrap();

        assert_eq!(sheet.basics.name, "Jane Doe");
        assert_eq!(
            sheet.basics.label.as_deref(),
            Some("Senior Software Engineer")
        );
        assert_eq!(sheet.basics.email.as_deref(), Some("jane@example.com"));

        let loc = sheet.basics.location.as_ref().unwrap();
        assert_eq!(loc.city.as_deref(), Some("San Francisco"));
        assert_eq!(loc.country.as_deref(), Some("US"));

        assert_eq!(sheet.work_experience.len(), 2);
        assert_eq!(sheet.work_experience[0].company, "Acme Corp");
        assert!(sheet.work_experience[0].is_current);
        assert_eq!(sheet.work_experience[0].achievements.len(), 2);
        assert_eq!(sheet.work_experience[0].tech_stack.len(), 4);

        assert_eq!(sheet.education.len(), 1);
        assert_eq!(sheet.education[0].institution, "MIT");

        assert_eq!(sheet.skills.len(), 2);
        assert_eq!(sheet.skills[0].skills.len(), 3);

        assert_eq!(sheet.certifications.len(), 1);
        assert_eq!(sheet.languages.len(), 2);
        assert_eq!(sheet.projects.len(), 1);

        let prefs = sheet.preferences.as_ref().unwrap();
        assert_eq!(prefs.salary_min, Some(180000));
        assert_eq!(prefs.remote, Some(true));

        let goals = sheet.goals.as_ref().unwrap();
        assert!(goals.short_term.is_some());
    }

    #[test]
    fn achievement_with_metrics() {
        let yaml = r#"
description: Increased revenue by 25%
metric_type: percentage
metric_value: "25"
metric_unit: percent
"#;
        let a: Achievement = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(a.description, "Increased revenue by 25%");
        assert_eq!(a.metric_type.as_deref(), Some("percentage"));
        assert_eq!(a.metric_value.as_deref(), Some("25"));
    }
}
