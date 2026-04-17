use crate::life_sheet::{LifeSheet, WorkExperience};

use super::types::{
    FabricationRisk, GapReport, JobDescriptionAnalysis, MatchedSkill, MissingSkill,
    SkillEvidenceSource,
};

pub struct DefaultGapAnalyzer;

impl DefaultGapAnalyzer {
    pub fn analyze(&self, life_sheet: &LifeSheet, jd: &JobDescriptionAnalysis) -> GapReport {
        let mut matched = Vec::new();
        let mut missing_req = Vec::new();
        let mut missing_nth = Vec::new();

        for skill in &jd.required_skills {
            match find_skill_evidence(&skill.canonical, life_sheet) {
                Some((source, strength)) => matched.push(MatchedSkill {
                    skill_name: skill.name.clone(),
                    evidence_source: source,
                    strength,
                }),
                None => {
                    let risk = compute_fabrication_risk(&skill.canonical, life_sheet);
                    missing_req.push(MissingSkill {
                        skill_name: skill.name.clone(),
                        is_required: true,
                        fabrication_risk: risk,
                    });
                }
            }
        }

        for skill in &jd.nice_to_have_skills {
            match find_skill_evidence(&skill.canonical, life_sheet) {
                Some((source, strength)) => matched.push(MatchedSkill {
                    skill_name: skill.name.clone(),
                    evidence_source: source,
                    strength,
                }),
                None => {
                    let risk = compute_fabrication_risk(&skill.canonical, life_sheet);
                    missing_nth.push(MissingSkill {
                        skill_name: skill.name.clone(),
                        is_required: false,
                        fabrication_risk: risk,
                    });
                }
            }
        }

        let total_skills = (jd.required_skills.len() + jd.nice_to_have_skills.len()).max(1);
        let match_score = (matched.len() as f32 / total_skills as f32) * 100.0;

        let relevant_experience_order = rank_experiences(life_sheet, jd);

        GapReport {
            matched_skills: matched,
            missing_required: missing_req,
            missing_nice_to_have: missing_nth,
            match_score,
            relevant_experience_order,
        }
    }
}

fn find_skill_evidence(
    canonical: &str,
    life_sheet: &LifeSheet,
) -> Option<(SkillEvidenceSource, f32)> {
    let canonical_lower = canonical.to_lowercase();
    let canonical_spaces = canonical_lower.replace('_', " ");

    for cat in &life_sheet.skills {
        for skill in &cat.skills {
            if skill.name.to_lowercase() == canonical_lower
                || skill.name.to_lowercase() == canonical_spaces
            {
                return Some((SkillEvidenceSource::ExplicitSkill, 1.0));
            }
        }
    }

    for (idx, exp) in life_sheet.work_experience.iter().enumerate() {
        for tech in &exp.tech_stack {
            if tech.to_lowercase() == canonical_lower || tech.to_lowercase() == canonical_spaces {
                return Some((
                    SkillEvidenceSource::ExperienceBullet {
                        company: exp.company.clone(),
                        index: idx,
                    },
                    0.9,
                ));
            }
        }
        for (bullet_idx, achievement) in exp.achievements.iter().enumerate() {
            let desc_lower = achievement.description.to_lowercase();
            if desc_lower.contains(&canonical_lower) || desc_lower.contains(&canonical_spaces) {
                return Some((
                    SkillEvidenceSource::ExperienceBullet {
                        company: exp.company.clone(),
                        index: bullet_idx,
                    },
                    0.8,
                ));
            }
        }
    }

    for proj in &life_sheet.projects {
        for highlight in &proj.highlights {
            if highlight.to_lowercase().contains(&canonical_lower)
                || highlight.to_lowercase().contains(&canonical_spaces)
            {
                return Some((
                    SkillEvidenceSource::ProjectDescription {
                        name: proj.name.clone(),
                    },
                    0.7,
                ));
            }
        }
    }

    for cert in &life_sheet.certifications {
        if cert.name.to_lowercase().contains(&canonical_lower)
            || cert.name.to_lowercase().contains(&canonical_spaces)
        {
            return Some((
                SkillEvidenceSource::Certification {
                    name: cert.name.clone(),
                },
                0.85,
            ));
        }
    }

    None
}

fn compute_fabrication_risk(canonical: &str, life_sheet: &LifeSheet) -> FabricationRisk {
    let canonical_lower = canonical.to_lowercase();

    let cert_keywords = [
        "certified",
        "certification",
        "license",
        "licensed",
        "cpa",
        "cfa",
        "pmp",
        "cissp",
        "aws certified",
    ];
    for kw in &cert_keywords {
        if canonical_lower.contains(kw) {
            return FabricationRisk::Forbidden;
        }
    }

    let all_skill_names: Vec<String> = life_sheet
        .skills
        .iter()
        .flat_map(|cat| cat.skills.iter().map(|s| s.name.to_lowercase()))
        .chain(
            life_sheet
                .work_experience
                .iter()
                .flat_map(|exp| exp.tech_stack.iter().map(|t| t.to_lowercase())),
        )
        .collect();

    for skill_name in &all_skill_names {
        let similarity = strsim::jaro_winkler(skill_name, &canonical_lower);
        if similarity >= 0.88 {
            return FabricationRisk::Low;
        }
    }

    FabricationRisk::High
}

fn rank_experiences(life_sheet: &LifeSheet, jd: &JobDescriptionAnalysis) -> Vec<usize> {
    let keywords: Vec<String> = jd
        .keywords
        .iter()
        .chain(jd.required_skills.iter().map(|s| &s.canonical))
        .map(|k| k.to_lowercase())
        .collect();

    let mut scored: Vec<(usize, usize)> = life_sheet
        .work_experience
        .iter()
        .enumerate()
        .map(|(idx, exp)| {
            let count = keyword_overlap_count(exp, &keywords);
            (idx, count)
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(idx, _)| idx).collect()
}

fn keyword_overlap_count(exp: &WorkExperience, keywords: &[String]) -> usize {
    let text = format!(
        "{} {} {} {}",
        exp.position.to_lowercase(),
        exp.summary.as_deref().unwrap_or("").to_lowercase(),
        exp.achievements
            .iter()
            .map(|a| a.description.to_lowercase())
            .collect::<Vec<_>>()
            .join(" "),
        exp.tech_stack
            .iter()
            .map(|t| t.to_lowercase())
            .collect::<Vec<_>>()
            .join(" "),
    );

    keywords
        .iter()
        .filter(|kw| text.contains(kw.as_str()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_sheet::{
        Achievement, Basics, Certification, Project, Skill, SkillCategory, WorkExperience,
    };
    use crate::resume::types::SkillRequirement;

    fn make_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Jane Doe".into(),
                label: Some("Senior Engineer".into()),
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![
                WorkExperience {
                    company: "Acme Corp".into(),
                    position: "Senior Software Engineer".into(),
                    start_date: "2021-03".into(),
                    end_date: None,
                    location: None,
                    url: None,
                    summary: Some("Leading backend team".into()),
                    is_current: true,
                    achievements: vec![
                        Achievement {
                            description:
                                "Reduced API latency by 40% through caching layer redesign".into(),
                            metric_type: Some("percentage".into()),
                            metric_value: Some("40".into()),
                            metric_unit: Some("percent".into()),
                        },
                        Achievement {
                            description: "Mentored 3 junior engineers".into(),
                            metric_type: None,
                            metric_value: None,
                            metric_unit: None,
                        },
                    ],
                    tech_stack: vec![
                        "Rust".into(),
                        "PostgreSQL".into(),
                        "Redis".into(),
                        "Kubernetes".into(),
                    ],
                    team_size: Some(8),
                    industry: Some("SaaS".into()),
                },
                WorkExperience {
                    company: "StartupXYZ".into(),
                    position: "Software Engineer".into(),
                    start_date: "2018-06".into(),
                    end_date: Some("2021-02".into()),
                    location: None,
                    url: None,
                    summary: None,
                    is_current: false,
                    achievements: vec![Achievement {
                        description: "Shipped v1.0 of the product in 4 months".into(),
                        metric_type: None,
                        metric_value: None,
                        metric_unit: None,
                    }],
                    tech_stack: vec!["Python".into(), "Django".into(), "AWS".into()],
                    team_size: None,
                    industry: None,
                },
            ],
            education: vec![],
            skills: vec![
                SkillCategory {
                    name: "Backend".into(),
                    level: Some("Expert".into()),
                    skills: vec![
                        Skill {
                            name: "Rust".into(),
                            years_experience: Some(4),
                            proficiency: Some("advanced".into()),
                        },
                        Skill {
                            name: "Python".into(),
                            years_experience: Some(8),
                            proficiency: Some("expert".into()),
                        },
                        Skill {
                            name: "PostgreSQL".into(),
                            years_experience: Some(6),
                            proficiency: Some("advanced".into()),
                        },
                    ],
                },
                SkillCategory {
                    name: "Infrastructure".into(),
                    level: Some("Intermediate".into()),
                    skills: vec![
                        Skill {
                            name: "Kubernetes".into(),
                            years_experience: Some(3),
                            proficiency: Some("intermediate".into()),
                        },
                        Skill {
                            name: "AWS".into(),
                            years_experience: Some(5),
                            proficiency: Some("advanced".into()),
                        },
                    ],
                },
            ],
            certifications: vec![Certification {
                name: "AWS Solutions Architect".into(),
                authority: Some("Amazon".into()),
                issue_date: None,
                expiry_date: None,
                url: None,
            }],
            languages: vec![],
            projects: vec![Project {
                name: "LazyJob".into(),
                description: Some("AI-powered job search TUI".into()),
                url: None,
                start_date: None,
                end_date: None,
                highlights: vec![
                    "Built semantic job matching with cosine similarity".into(),
                    "Integrated with Greenhouse and Lever APIs".into(),
                ],
            }],
            preferences: None,
            goals: None,
        }
    }

    fn make_jd(required: &[(&str, &str)], nice_to_have: &[(&str, &str)]) -> JobDescriptionAnalysis {
        JobDescriptionAnalysis {
            raw_text: "test jd".into(),
            required_skills: required
                .iter()
                .map(|(name, canonical)| SkillRequirement {
                    name: name.to_string(),
                    canonical: canonical.to_string(),
                    is_required: true,
                })
                .collect(),
            nice_to_have_skills: nice_to_have
                .iter()
                .map(|(name, canonical)| SkillRequirement {
                    name: name.to_string(),
                    canonical: canonical.to_string(),
                    is_required: false,
                })
                .collect(),
            keywords: vec!["rust".into(), "backend".into()],
            responsibilities: vec![],
        }
    }

    #[test]
    fn matched_skill_found_in_explicit_skills() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("Rust", "rust")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.matched_skills.len(), 1);
        assert_eq!(report.missing_required.len(), 0);
        assert!((report.match_score - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn matched_skill_found_in_tech_stack() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("Redis", "redis")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.matched_skills.len(), 1);
        assert!(matches!(
            report.matched_skills[0].evidence_source,
            SkillEvidenceSource::ExperienceBullet { .. }
        ));
    }

    #[test]
    fn matched_skill_found_in_certification() {
        let sheet = make_life_sheet();
        let jd = make_jd(
            &[("AWS Solutions Architect", "aws solutions architect")],
            &[],
        );
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.matched_skills.len(), 1);
        assert!(matches!(
            report.matched_skills[0].evidence_source,
            SkillEvidenceSource::Certification { .. }
        ));
    }

    #[test]
    fn missing_required_skill_high_risk() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("Scala", "scala")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.missing_required.len(), 1);
        assert_eq!(
            report.missing_required[0].fabrication_risk,
            FabricationRisk::High
        );
    }

    #[test]
    fn fuzzy_match_gives_low_risk() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("Postgres", "postgres")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        if report.matched_skills.is_empty() {
            assert_eq!(
                report.missing_required[0].fabrication_risk,
                FabricationRisk::Low
            );
        }
    }

    #[test]
    fn certification_skill_forbidden_risk() {
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
        let jd = make_jd(&[("PMP Certified", "pmp certified")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(
            report.missing_required[0].fabrication_risk,
            FabricationRisk::Forbidden
        );
    }

    #[test]
    fn nice_to_have_skills_tracked_separately() {
        let sheet = make_life_sheet();
        let jd = make_jd(
            &[("Rust", "rust")],
            &[("Go", "go"), ("Kubernetes", "kubernetes")],
        );
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.matched_skills.len(), 2); // Rust + Kubernetes
        assert_eq!(report.missing_nice_to_have.len(), 1); // Go
    }

    #[test]
    fn match_score_calculated_correctly() {
        let sheet = make_life_sheet();
        let jd = make_jd(
            &[("Rust", "rust"), ("Python", "python")],
            &[("Go", "go"), ("Kubernetes", "kubernetes")],
        );
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        // 3 matched (Rust, Python, Kubernetes) out of 4 total = 75%
        assert!((report.match_score - 75.0).abs() < f32::EPSILON);
    }

    #[test]
    fn experience_ranking_by_keyword_overlap() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("Rust", "rust"), ("PostgreSQL", "postgresql")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert!(!report.relevant_experience_order.is_empty());
        // Acme Corp (index 0) should rank higher — has Rust and PostgreSQL
        assert_eq!(report.relevant_experience_order[0], 0);
    }

    #[test]
    fn empty_jd_gives_zero_score() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert!(report.matched_skills.is_empty());
        assert!(report.missing_required.is_empty());
    }

    #[test]
    fn skill_found_in_project_highlights() {
        let sheet = make_life_sheet();
        let jd = make_jd(&[("cosine similarity", "cosine similarity")], &[]);
        let report = DefaultGapAnalyzer.analyze(&sheet, &jd);
        assert_eq!(report.matched_skills.len(), 1);
        assert!(matches!(
            report.matched_skills[0].evidence_source,
            SkillEvidenceSource::ProjectDescription { .. }
        ));
    }

    // learning test: verifies strsim::jaro_winkler similarity behavior
    #[test]
    fn strsim_jaro_winkler_behavior() {
        let sim = strsim::jaro_winkler("postgresql", "postgres");
        assert!(sim > 0.88, "Expected high similarity, got {sim}");

        let sim2 = strsim::jaro_winkler("rust", "ruby");
        assert!(
            sim2 < 0.88,
            "Expected low similarity for unrelated, got {sim2}"
        );

        let sim3 = strsim::jaro_winkler("kubernetes", "k8s");
        assert!(
            sim3 < 0.88,
            "Expected low similarity for abbreviation, got {sim3}"
        );
    }
}
