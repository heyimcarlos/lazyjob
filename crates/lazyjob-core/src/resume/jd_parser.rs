use std::sync::Arc;

use async_trait::async_trait;

use crate::discovery::Completer;
use crate::error::Result;

use super::types::{JobDescriptionAnalysis, SkillRequirement};

const JD_PARSE_SYSTEM_PROMPT: &str = r#"You are a job description analyzer. Given a job description, extract structured data as JSON.

Return ONLY a JSON object with these fields:
{
  "required_skills": [{"name": "Skill Name", "canonical": "skill_name_lowercase"}],
  "nice_to_have_skills": [{"name": "Skill Name", "canonical": "skill_name_lowercase"}],
  "keywords": ["keyword1", "keyword2"],
  "responsibilities": ["responsibility1", "responsibility2"]
}

Rules:
- "canonical" is the lowercase, normalized form of the skill name
- Put clearly required skills in required_skills, preferred/bonus skills in nice_to_have_skills
- Extract all important technical terms as keywords
- Extract key responsibilities as short phrases
- Return ONLY valid JSON, no markdown fences"#;

#[async_trait]
pub trait JobDescriptionParser: Send + Sync {
    async fn parse(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis>;
}

pub struct LlmJdParser {
    completer: Arc<dyn Completer>,
}

impl LlmJdParser {
    pub fn new(completer: Arc<dyn Completer>) -> Self {
        Self { completer }
    }
}

#[derive(serde::Deserialize)]
struct JdLlmOutput {
    required_skills: Vec<JdSkill>,
    nice_to_have_skills: Vec<JdSkill>,
    keywords: Vec<String>,
    responsibilities: Vec<String>,
}

#[derive(serde::Deserialize)]
struct JdSkill {
    name: String,
    canonical: String,
}

fn extract_json(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
    {
        return &trimmed[start..=end];
    }
    trimmed
}

#[async_trait]
impl JobDescriptionParser for LlmJdParser {
    async fn parse(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis> {
        let user_msg = format!("JOB DESCRIPTION:\n{raw_jd}");
        let response = self
            .completer
            .complete(JD_PARSE_SYSTEM_PROMPT, &user_msg)
            .await?;

        let json_str = extract_json(&response);
        let output: JdLlmOutput = serde_json::from_str(json_str)?;

        Ok(JobDescriptionAnalysis {
            raw_text: raw_jd.to_string(),
            required_skills: output
                .required_skills
                .into_iter()
                .map(|s| SkillRequirement {
                    name: s.name,
                    canonical: s.canonical,
                    is_required: true,
                })
                .collect(),
            nice_to_have_skills: output
                .nice_to_have_skills
                .into_iter()
                .map(|s| SkillRequirement {
                    name: s.name,
                    canonical: s.canonical,
                    is_required: false,
                })
                .collect(),
            keywords: output.keywords,
            responsibilities: output.responsibilities,
        })
    }
}

pub struct RegexJdParser;

impl RegexJdParser {
    pub fn parse_sync(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis> {
        let lines: Vec<&str> = raw_jd.lines().collect();
        let mut required_skills = Vec::new();
        let mut nice_to_have_skills = Vec::new();
        let mut keywords = Vec::new();
        let mut responsibilities = Vec::new();

        let mut in_requirements = false;
        let mut in_nice_to_have = false;
        let mut in_responsibilities = false;

        for line in &lines {
            let trimmed = line.trim();
            let lower = trimmed.to_lowercase();

            if lower.contains("require")
                || lower.contains("must have")
                || lower.contains("qualifications")
            {
                in_requirements = true;
                in_nice_to_have = false;
                in_responsibilities = false;
                continue;
            }
            if lower.contains("nice to have")
                || lower.contains("preferred")
                || lower.contains("bonus")
            {
                in_nice_to_have = true;
                in_requirements = false;
                in_responsibilities = false;
                continue;
            }
            if lower.contains("responsibilit")
                || lower.contains("what you")
                || lower.contains("you will")
            {
                in_responsibilities = true;
                in_requirements = false;
                in_nice_to_have = false;
                continue;
            }

            let bullet = trimmed
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim_start_matches('•')
                .trim();

            if bullet.is_empty() {
                continue;
            }

            if in_requirements {
                let canonical = bullet.to_lowercase().replace(' ', "_");
                required_skills.push(SkillRequirement {
                    name: bullet.to_string(),
                    canonical,
                    is_required: true,
                });
            } else if in_nice_to_have {
                let canonical = bullet.to_lowercase().replace(' ', "_");
                nice_to_have_skills.push(SkillRequirement {
                    name: bullet.to_string(),
                    canonical,
                    is_required: false,
                });
            } else if in_responsibilities {
                responsibilities.push(bullet.to_string());
            }
        }

        let all_text = raw_jd.to_lowercase();
        let tech_patterns = [
            "rust",
            "python",
            "java",
            "javascript",
            "typescript",
            "go",
            "c++",
            "kubernetes",
            "docker",
            "aws",
            "gcp",
            "azure",
            "react",
            "node",
            "postgresql",
            "redis",
            "kafka",
            "graphql",
            "rest",
            "grpc",
            "terraform",
            "ci/cd",
            "linux",
            "git",
            "sql",
            "nosql",
        ];
        for pattern in &tech_patterns {
            if all_text.contains(pattern) {
                keywords.push(pattern.to_string());
            }
        }

        Ok(JobDescriptionAnalysis {
            raw_text: raw_jd.to_string(),
            required_skills,
            nice_to_have_skills,
            keywords,
            responsibilities,
        })
    }
}

#[async_trait]
impl JobDescriptionParser for RegexJdParser {
    async fn parse(&self, raw_jd: &str) -> Result<JobDescriptionAnalysis> {
        self.parse_sync(raw_jd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;

    struct MockCompleter {
        response: String,
    }

    impl MockCompleter {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingCompleter;

    #[async_trait]
    impl Completer for FailingCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            Err(CoreError::Http("LLM unavailable".into()))
        }
    }

    const CANNED_JD_JSON: &str = r#"{
        "required_skills": [
            {"name": "Rust", "canonical": "rust"},
            {"name": "PostgreSQL", "canonical": "postgresql"},
            {"name": "Distributed Systems", "canonical": "distributed_systems"}
        ],
        "nice_to_have_skills": [
            {"name": "Kubernetes", "canonical": "kubernetes"}
        ],
        "keywords": ["backend", "microservices", "scalable"],
        "responsibilities": ["Design and build backend services", "Mentor junior engineers"]
    }"#;

    #[tokio::test]
    async fn llm_parser_extracts_skills() {
        let completer = Arc::new(MockCompleter::new(CANNED_JD_JSON));
        let parser = LlmJdParser::new(completer);
        let result = parser.parse("Some job description").await.unwrap();

        assert_eq!(result.required_skills.len(), 3);
        assert_eq!(result.required_skills[0].name, "Rust");
        assert_eq!(result.required_skills[0].canonical, "rust");
        assert!(result.required_skills[0].is_required);

        assert_eq!(result.nice_to_have_skills.len(), 1);
        assert_eq!(result.nice_to_have_skills[0].name, "Kubernetes");
        assert!(!result.nice_to_have_skills[0].is_required);

        assert_eq!(result.keywords.len(), 3);
        assert_eq!(result.responsibilities.len(), 2);
    }

    #[tokio::test]
    async fn llm_parser_handles_markdown_fences() {
        let response = format!("```json\n{CANNED_JD_JSON}\n```");
        let completer = Arc::new(MockCompleter::new(&response));
        let parser = LlmJdParser::new(completer);
        let result = parser.parse("Job desc").await.unwrap();
        assert_eq!(result.required_skills.len(), 3);
    }

    #[tokio::test]
    async fn llm_parser_handles_preamble_text() {
        let response = format!("Here is the analysis:\n{CANNED_JD_JSON}\nDone.");
        let completer = Arc::new(MockCompleter::new(&response));
        let parser = LlmJdParser::new(completer);
        let result = parser.parse("Job desc").await.unwrap();
        assert_eq!(result.required_skills.len(), 3);
    }

    #[tokio::test]
    async fn llm_parser_fails_on_invalid_json() {
        let completer = Arc::new(MockCompleter::new("not json at all"));
        let parser = LlmJdParser::new(completer);
        assert!(parser.parse("Job desc").await.is_err());
    }

    #[tokio::test]
    async fn llm_parser_fails_on_completer_error() {
        let completer = Arc::new(FailingCompleter);
        let parser = LlmJdParser::new(completer);
        assert!(parser.parse("Job desc").await.is_err());
    }

    #[test]
    fn regex_parser_extracts_requirements() {
        let jd = r#"
About the Role
We are looking for a Senior Backend Engineer.

Responsibilities:
- Design and build scalable backend services
- Lead architecture decisions

Requirements:
- 5+ years of Rust experience
- Strong PostgreSQL knowledge
- Experience with distributed systems

Nice to Have:
- Kubernetes experience
- GraphQL knowledge
"#;
        let parser = RegexJdParser;
        let result = parser.parse_sync(jd).unwrap();

        assert_eq!(result.required_skills.len(), 3);
        assert_eq!(result.nice_to_have_skills.len(), 2);
        assert_eq!(result.responsibilities.len(), 2);
        assert!(result.keywords.contains(&"rust".to_string()));
        assert!(result.keywords.contains(&"postgresql".to_string()));
    }

    #[test]
    fn regex_parser_extracts_keywords_from_body() {
        let jd = "We use Rust, Python, PostgreSQL, and deploy on Kubernetes with Docker.";
        let parser = RegexJdParser;
        let result = parser.parse_sync(jd).unwrap();

        assert!(result.keywords.contains(&"rust".to_string()));
        assert!(result.keywords.contains(&"python".to_string()));
        assert!(result.keywords.contains(&"kubernetes".to_string()));
        assert!(result.keywords.contains(&"docker".to_string()));
    }

    #[test]
    fn regex_parser_handles_empty_jd() {
        let parser = RegexJdParser;
        let result = parser.parse_sync("").unwrap();
        assert!(result.required_skills.is_empty());
        assert!(result.keywords.is_empty());
    }

    #[tokio::test]
    async fn regex_parser_implements_trait() {
        let parser = RegexJdParser;
        let result = parser
            .parse("Requirements:\n- Rust\n- Python")
            .await
            .unwrap();
        assert_eq!(result.required_skills.len(), 2);
    }

    #[test]
    fn extract_json_strips_preamble() {
        assert_eq!(extract_json("text {\"a\": 1} more"), "{\"a\": 1}");
    }

    #[test]
    fn extract_json_returns_input_when_no_braces() {
        assert_eq!(extract_json("no json here"), "no json here");
    }
}
