use serde::{Deserialize, Serialize};

use super::types::LifeSheet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResume {
    pub basics: JsonResumeBasics,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work: Vec<JsonResumeWork>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub education: Vec<JsonResumeEducation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<JsonResumeSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub certificates: Vec<JsonResumeCertificate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<JsonResumeLanguage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<JsonResumeProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeBasics {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<JsonResumeLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeLocation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "countryCode"
    )]
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeWork {
    pub name: String,
    pub position: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "startDate")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeEducation {
    pub institution: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "studyType")]
    pub study_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "startDate")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeSkill {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeCertificate {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeLanguage {
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fluency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonResumeProject {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "startDate")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<String>,
}

impl LifeSheet {
    pub fn to_json_resume(&self) -> JsonResume {
        let basics = JsonResumeBasics {
            name: self.basics.name.clone(),
            label: self.basics.label.clone(),
            email: self.basics.email.clone(),
            phone: self.basics.phone.clone(),
            url: self.basics.url.clone(),
            summary: self.basics.summary.clone(),
            location: self.basics.location.as_ref().map(|loc| JsonResumeLocation {
                city: loc.city.clone(),
                region: loc.region.clone(),
                country_code: loc.country.clone(),
            }),
        };

        let work = self
            .work_experience
            .iter()
            .map(|exp| {
                let highlights: Vec<String> = exp
                    .achievements
                    .iter()
                    .map(|a| a.description.clone())
                    .collect();
                JsonResumeWork {
                    name: exp.company.clone(),
                    position: exp.position.clone(),
                    url: exp.url.clone(),
                    start_date: Some(exp.start_date.clone()),
                    end_date: exp.end_date.clone(),
                    summary: exp.summary.clone(),
                    highlights,
                }
            })
            .collect();

        let education = self
            .education
            .iter()
            .map(|edu| JsonResumeEducation {
                institution: edu.institution.clone(),
                area: edu.field.clone(),
                study_type: edu.degree.clone(),
                start_date: edu.start_date.clone(),
                end_date: edu.end_date.clone(),
                score: edu.score.clone(),
            })
            .collect();

        let skills = self
            .skills
            .iter()
            .map(|cat| JsonResumeSkill {
                name: cat.name.clone(),
                level: cat.level.clone(),
                keywords: cat.skills.iter().map(|s| s.name.clone()).collect(),
            })
            .collect();

        let certificates = self
            .certifications
            .iter()
            .map(|cert| JsonResumeCertificate {
                name: cert.name.clone(),
                issuer: cert.authority.clone(),
                date: cert.issue_date.clone(),
                url: cert.url.clone(),
            })
            .collect();

        let languages = self
            .languages
            .iter()
            .map(|lang| JsonResumeLanguage {
                language: lang.name.clone(),
                fluency: lang.proficiency.clone(),
            })
            .collect();

        let projects = self
            .projects
            .iter()
            .map(|proj| JsonResumeProject {
                name: proj.name.clone(),
                description: proj.description.clone(),
                url: proj.url.clone(),
                start_date: proj.start_date.clone(),
                end_date: proj.end_date.clone(),
                highlights: proj.highlights.clone(),
            })
            .collect();

        JsonResume {
            basics,
            work,
            education,
            skills,
            certificates,
            languages,
            projects,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::parse_yaml;

    fn fixture_sheet() -> LifeSheet {
        let content = include_str!("../../tests/fixtures/life-sheet.yaml");
        parse_yaml(content).unwrap()
    }

    #[test]
    fn json_resume_basics_mapped() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.basics.name, "Jane Doe");
        assert_eq!(jr.basics.label.as_deref(), Some("Senior Software Engineer"));
        assert_eq!(jr.basics.email.as_deref(), Some("jane@example.com"));

        let loc = jr.basics.location.as_ref().unwrap();
        assert_eq!(loc.city.as_deref(), Some("San Francisco"));
        assert_eq!(loc.country_code.as_deref(), Some("US"));
    }

    #[test]
    fn json_resume_work_mapped() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.work.len(), 2);
        assert_eq!(jr.work[0].name, "Acme Corp");
        assert_eq!(jr.work[0].position, "Senior Software Engineer");
        assert_eq!(jr.work[0].highlights.len(), 2);
        assert_eq!(
            jr.work[0].highlights[0],
            "Reduced API latency by 40% through caching layer redesign"
        );
    }

    #[test]
    fn json_resume_education_mapped() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.education.len(), 1);
        assert_eq!(jr.education[0].institution, "MIT");
        assert_eq!(
            jr.education[0].study_type.as_deref(),
            Some("Bachelor of Science")
        );
        assert_eq!(jr.education[0].area.as_deref(), Some("Computer Science"));
    }

    #[test]
    fn json_resume_skills_mapped() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.skills.len(), 2);
        assert_eq!(jr.skills[0].name, "Backend");
        assert_eq!(jr.skills[0].keywords, vec!["Rust", "Python", "PostgreSQL"]);
    }

    #[test]
    fn json_resume_serializes_to_valid_json() {
        let jr = fixture_sheet().to_json_resume();
        let json = serde_json::to_string_pretty(&jr).unwrap();
        assert!(json.contains("\"name\": \"Jane Doe\""));
        assert!(json.contains("\"startDate\""));

        let reparsed: JsonResume = serde_json::from_str(&json).unwrap();
        assert_eq!(jr, reparsed);
    }

    #[test]
    fn json_resume_certificates_and_languages() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.certificates.len(), 1);
        assert_eq!(jr.certificates[0].name, "AWS Solutions Architect");
        assert_eq!(
            jr.certificates[0].issuer.as_deref(),
            Some("Amazon Web Services")
        );

        assert_eq!(jr.languages.len(), 2);
        assert_eq!(jr.languages[0].language, "English");
        assert_eq!(jr.languages[0].fluency.as_deref(), Some("native"));
    }

    #[test]
    fn json_resume_projects_mapped() {
        let jr = fixture_sheet().to_json_resume();
        assert_eq!(jr.projects.len(), 1);
        assert_eq!(jr.projects[0].name, "LazyJob");
        assert_eq!(jr.projects[0].highlights.len(), 2);
    }
}
