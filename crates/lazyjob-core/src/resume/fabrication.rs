use crate::life_sheet::LifeSheet;

use super::types::{FabricationItem, FabricationReport, FabricationRisk, ResumeContent};

pub struct DefaultFabricationAuditor;

impl DefaultFabricationAuditor {
    pub fn audit(&self, content: &ResumeContent, life_sheet: &LifeSheet) -> FabricationReport {
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();

        for skill in content
            .skills
            .primary
            .iter()
            .chain(content.skills.secondary.iter())
        {
            let risk = assess_skill_risk(skill, life_sheet);
            match risk {
                FabricationRisk::Forbidden => {
                    errors.push(format!(
                        "Cannot claim credential/license without evidence: '{skill}'"
                    ));
                }
                FabricationRisk::High => {
                    warnings.push(format!("No evidence for skill '{skill}' in life sheet"));
                }
                _ => {}
            }
            items.push(FabricationItem {
                description: skill.clone(),
                risk,
                source: "skills_section".to_string(),
            });
        }

        for exp in &content.experience {
            for (i, bullet) in exp.bullets.iter().enumerate() {
                if exp.rewritten_indices.contains(&i)
                    && let Some(claim) = detect_unsupported_claim(bullet, life_sheet)
                {
                    warnings.push(format!(
                        "Bullet in {} may contain unsupported claim: '{claim}'",
                        exp.company
                    ));
                }
            }
        }

        let is_safe_to_submit = errors.is_empty();

        FabricationReport {
            items,
            warnings,
            errors,
            is_safe_to_submit,
        }
    }
}

fn assess_skill_risk(skill: &str, life_sheet: &LifeSheet) -> FabricationRisk {
    let skill_lower = skill.to_lowercase();

    let cert_keywords = [
        "certified",
        "certification",
        "license",
        "licensed",
        "cpa",
        "cfa",
        "pmp",
        "cissp",
    ];

    for kw in &cert_keywords {
        if skill_lower.contains(kw) {
            for cert in &life_sheet.certifications {
                if cert.name.to_lowercase().contains(&skill_lower)
                    || skill_lower.contains(&cert.name.to_lowercase())
                {
                    return FabricationRisk::None;
                }
            }
            return FabricationRisk::Forbidden;
        }
    }

    for cat in &life_sheet.skills {
        for s in &cat.skills {
            if s.name.to_lowercase() == skill_lower {
                return FabricationRisk::None;
            }
        }
    }

    for exp in &life_sheet.work_experience {
        for tech in &exp.tech_stack {
            if tech.to_lowercase() == skill_lower {
                return FabricationRisk::None;
            }
        }
    }

    for proj in &life_sheet.projects {
        for highlight in &proj.highlights {
            if highlight.to_lowercase().contains(&skill_lower) {
                return FabricationRisk::None;
            }
        }
    }

    let all_names: Vec<String> = life_sheet
        .skills
        .iter()
        .flat_map(|cat| cat.skills.iter().map(|s| s.name.to_lowercase()))
        .chain(
            life_sheet
                .work_experience
                .iter()
                .flat_map(|e| e.tech_stack.iter().map(|t| t.to_lowercase())),
        )
        .collect();

    for name in &all_names {
        if strsim::jaro_winkler(name, &skill_lower) >= 0.88 {
            return FabricationRisk::Low;
        }
    }

    FabricationRisk::High
}

fn detect_unsupported_claim(bullet: &str, life_sheet: &LifeSheet) -> Option<String> {
    for claim in extract_numeric_claims(bullet) {
        let found_in_original = life_sheet.work_experience.iter().any(|exp| {
            exp.achievements.iter().any(|a| {
                a.description.contains(&claim)
                    || a.metric_value
                        .as_ref()
                        .is_some_and(|v| claim.contains(v.as_str()))
            })
        });
        if !found_in_original {
            return Some(claim);
        }
    }

    None
}

fn extract_numeric_claims(text: &str) -> Vec<String> {
    let mut claims = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '$' && i + 1 < len && chars[i + 1].is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == ',' || chars[i] == '.') {
                i += 1;
            }
            if i < len && "kKmMbB".contains(chars[i]) {
                i += 1;
            }
            claims.push(chars[start..i].iter().collect());
            continue;
        }

        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == ',' || chars[i] == '.') {
                i += 1;
            }
            if i < len && (chars[i] == '%' || chars[i] == 'x') {
                i += 1;
                claims.push(chars[start..i].iter().collect());
            } else if i < len && "kKmMbB".contains(chars[i]) {
                i += 1;
                if i < len && chars[i] == '+' {
                    i += 1;
                }
                claims.push(chars[start..i].iter().collect());
            }
            continue;
        }

        i += 1;
    }

    claims
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::{
        Achievement, Basics, Certification, Project, Skill, SkillCategory, WorkExperience,
    };
    use crate::resume::types::{ExperienceSection, SkillsSection};

    fn make_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Jane".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "Acme".into(),
                position: "Engineer".into(),
                start_date: "2020-01".into(),
                end_date: None,
                location: None,
                url: None,
                summary: None,
                is_current: true,
                achievements: vec![Achievement {
                    description: "Reduced latency by 40%".into(),
                    metric_type: None,
                    metric_value: Some("40".into()),
                    metric_unit: None,
                }],
                tech_stack: vec!["Rust".into(), "PostgreSQL".into()],
                team_size: None,
                industry: None,
            }],
            education: vec![],
            skills: vec![SkillCategory {
                name: "Backend".into(),
                level: None,
                skills: vec![Skill {
                    name: "Rust".into(),
                    years_experience: Some(4),
                    proficiency: None,
                }],
            }],
            certifications: vec![Certification {
                name: "AWS Solutions Architect".into(),
                authority: None,
                issue_date: None,
                expiry_date: None,
                url: None,
            }],
            languages: vec![],
            projects: vec![Project {
                name: "LazyJob".into(),
                description: None,
                url: None,
                start_date: None,
                end_date: None,
                highlights: vec!["Built with Rust and Ratatui".into()],
            }],
            preferences: None,
            goals: None,
        }
    }

    fn make_content(skills: Vec<&str>, bullets: Vec<&str>, rewritten: Vec<usize>) -> ResumeContent {
        ResumeContent {
            summary: "A summary".into(),
            experience: vec![ExperienceSection {
                company: "Acme".into(),
                title: "Engineer".into(),
                date_range: "2020 - Present".into(),
                bullets: bullets.into_iter().map(String::from).collect(),
                rewritten_indices: rewritten,
            }],
            skills: SkillsSection {
                primary: skills.into_iter().map(String::from).collect(),
                secondary: vec![],
            },
            education: vec![],
            projects: vec![],
            certifications: vec![],
        }
    }

    #[test]
    fn clean_resume_passes_audit() {
        let sheet = make_life_sheet();
        let content = make_content(
            vec!["Rust", "PostgreSQL"],
            vec!["Reduced latency by 40%"],
            vec![],
        );
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(report.is_safe_to_submit);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn unknown_skill_produces_warning() {
        let sheet = make_life_sheet();
        let content = make_content(vec!["Scala"], vec![], vec![]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(!report.warnings.is_empty());
        assert!(report.warnings[0].contains("Scala"));
    }

    #[test]
    fn fabricated_certification_produces_error() {
        let sheet = LifeSheet {
            basics: Basics {
                name: "Test".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![],
            education: vec![],
            skills: vec![],
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        };
        let content = make_content(vec!["PMP Certified"], vec![], vec![]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(!report.is_safe_to_submit);
        assert!(!report.errors.is_empty());
        assert!(report.errors[0].contains("credential"));
    }

    #[test]
    fn real_certification_passes() {
        let sheet = make_life_sheet();
        let content = make_content(vec!["AWS Solutions Architect"], vec![], vec![]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(report.is_safe_to_submit);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn unsupported_metric_in_rewritten_bullet() {
        let sheet = make_life_sheet();
        let content = make_content(vec!["Rust"], vec!["Improved throughput by 500%"], vec![0]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(!report.warnings.is_empty());
        assert!(report.warnings[0].contains("500%"));
    }

    #[test]
    fn original_metric_preserved_in_rewrite() {
        let sheet = make_life_sheet();
        let content = make_content(
            vec!["Rust"],
            vec!["Reduced API latency by 40% through Rust caching"],
            vec![0],
        );
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        let metric_warnings: Vec<&String> = report
            .warnings
            .iter()
            .filter(|w| w.contains("unsupported"))
            .collect();
        assert!(metric_warnings.is_empty());
    }

    #[test]
    fn skill_from_tech_stack_passes() {
        let sheet = make_life_sheet();
        let content = make_content(vec!["PostgreSQL"], vec![], vec![]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        let pg_items: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.description == "PostgreSQL")
            .collect();
        assert_eq!(pg_items[0].risk, FabricationRisk::None);
    }

    #[test]
    fn skill_from_project_highlights_passes() {
        let sheet = make_life_sheet();
        let content = make_content(vec!["Ratatui"], vec![], vec![]);
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        let item = report
            .items
            .iter()
            .find(|i| i.description == "Ratatui")
            .unwrap();
        assert_eq!(item.risk, FabricationRisk::None);
    }

    #[test]
    fn empty_content_safe() {
        let sheet = make_life_sheet();
        let content = ResumeContent::default();
        let report = DefaultFabricationAuditor.audit(&content, &sheet);
        assert!(report.is_safe_to_submit);
    }
}
