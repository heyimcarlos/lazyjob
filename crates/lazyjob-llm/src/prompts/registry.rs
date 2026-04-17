use std::collections::HashMap;

use super::error::{Result, TemplateError};
use super::types::{LoopType, PromptTemplate};

pub struct DefaultPromptRegistry {
    templates: HashMap<LoopType, PromptTemplate>,
}

impl DefaultPromptRegistry {
    pub fn new() -> Result<Self> {
        let embedded: &[(&str, &str)] = &[
            ("base_system", include_str!("../templates/base_system.toml")),
            (
                "job_discovery",
                include_str!("../templates/job_discovery.toml"),
            ),
            (
                "company_research",
                include_str!("../templates/company_research.toml"),
            ),
            (
                "resume_tailoring",
                include_str!("../templates/resume_tailoring.toml"),
            ),
            (
                "cover_letter",
                include_str!("../templates/cover_letter.toml"),
            ),
            (
                "interview_prep",
                include_str!("../templates/interview_prep.toml"),
            ),
            (
                "salary_negotiation",
                include_str!("../templates/salary_negotiation.toml"),
            ),
            ("networking", include_str!("../templates/networking.toml")),
            (
                "error_response",
                include_str!("../templates/error_response.toml"),
            ),
        ];

        let mut map = HashMap::new();
        for (file, src) in embedded {
            let tmpl: PromptTemplate =
                toml::from_str(src).map_err(|e| TemplateError::ParseError {
                    file: file.to_string(),
                    source: e,
                })?;
            map.insert(tmpl.loop_type, tmpl);
        }
        Ok(Self { templates: map })
    }

    pub fn get(&self, loop_type: LoopType) -> Result<&PromptTemplate> {
        self.templates
            .get(&loop_type)
            .ok_or(TemplateError::NotRegistered(loop_type))
    }

    pub fn all(&self) -> Vec<&PromptTemplate> {
        let mut v: Vec<_> = self.templates.values().collect();
        v.sort_by_key(|t| format!("{:?}", t.loop_type));
        v
    }

    pub fn override_template(
        &mut self,
        loop_type: LoopType,
        template: PromptTemplate,
    ) -> Result<()> {
        self.templates.insert(loop_type, template);
        Ok(())
    }

    pub fn load_user_overrides(
        &mut self,
        config_dir: &std::path::Path,
    ) -> std::result::Result<usize, Box<dyn std::error::Error>> {
        let prompt_dir = config_dir.join("prompts");
        if !prompt_dir.exists() {
            return Ok(0);
        }

        let mut count = 0usize;
        for entry in std::fs::read_dir(&prompt_dir)? {
            let path = entry?.path();
            if path.extension().map(|e| e != "toml").unwrap_or(true) {
                continue;
            }
            let src = std::fs::read_to_string(&path)?;
            let tmpl: PromptTemplate =
                toml::from_str(&src).map_err(|e| TemplateError::OverrideParseError {
                    path: path.clone(),
                    source: e,
                })?;
            self.templates.insert(tmpl.loop_type, tmpl);
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_parses_prompt_template() {
        let toml_str = r#"
name = "test_v1"
version = "1.0.0"
loop_type = "job_discovery"
system = "System prompt"
user = "User prompt"
cache_system_prompt = false
"#;
        let tmpl: PromptTemplate = toml::from_str(toml_str).unwrap();
        assert_eq!(tmpl.name, "test_v1");
        assert_eq!(tmpl.loop_type, LoopType::JobDiscovery);
        assert!(!tmpl.cache_system_prompt);
    }

    #[test]
    fn toml_defaults_for_optional_fields() {
        let toml_str = r#"
name = "test_v1"
version = "1.0.0"
loop_type = "base_system"
system = "System"
user = "User"
"#;
        let tmpl: PromptTemplate = toml::from_str(toml_str).unwrap();
        assert!(tmpl.cache_system_prompt);
        assert!(tmpl.few_shot_examples.is_empty());
        assert!(tmpl.output_schema.is_none());
    }

    #[test]
    fn registry_new_loads_all_loop_types() {
        let registry = DefaultPromptRegistry::new().unwrap();
        let all = registry.all();
        assert_eq!(all.len(), 9);

        let expected_types = [
            LoopType::BaseSystem,
            LoopType::JobDiscovery,
            LoopType::CompanyResearch,
            LoopType::ResumeTailoring,
            LoopType::CoverLetterGeneration,
            LoopType::InterviewPrep,
            LoopType::SalaryNegotiation,
            LoopType::Networking,
            LoopType::ErrorResponse,
        ];
        for lt in expected_types {
            assert!(registry.get(lt).is_ok(), "missing template for {:?}", lt);
        }
    }

    #[test]
    fn registry_get_returns_correct_template() {
        let registry = DefaultPromptRegistry::new().unwrap();
        let tmpl = registry.get(LoopType::JobDiscovery).unwrap();
        assert_eq!(tmpl.name, "job_discovery_v1");
        assert_eq!(tmpl.version, "1.0.0");
    }

    #[test]
    fn registry_get_not_registered() {
        let registry = DefaultPromptRegistry {
            templates: HashMap::new(),
        };
        let err = registry.get(LoopType::JobDiscovery).unwrap_err();
        matches!(err, TemplateError::NotRegistered(LoopType::JobDiscovery));
    }

    #[test]
    fn registry_override_replaces_template() {
        let mut registry = DefaultPromptRegistry::new().unwrap();
        let custom = PromptTemplate {
            name: "custom_v2".into(),
            version: "2.0.0".into(),
            loop_type: LoopType::JobDiscovery,
            system: "Custom system".into(),
            user: "Custom user".into(),
            few_shot_examples: vec![],
            cache_system_prompt: false,
            output_schema: None,
        };
        registry
            .override_template(LoopType::JobDiscovery, custom)
            .unwrap();
        let tmpl = registry.get(LoopType::JobDiscovery).unwrap();
        assert_eq!(tmpl.name, "custom_v2");
    }

    #[test]
    fn load_user_overrides_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompt_dir).unwrap();
        let mut registry = DefaultPromptRegistry::new().unwrap();
        let count = registry.load_user_overrides(dir.path()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn load_user_overrides_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompt_dir).unwrap();
        std::fs::write(
            prompt_dir.join("job_discovery.toml"),
            r#"
name = "job_discovery_custom"
version = "2.0.0"
loop_type = "job_discovery"
system = "Custom system"
user = "Custom user"
"#,
        )
        .unwrap();
        let mut registry = DefaultPromptRegistry::new().unwrap();
        let count = registry.load_user_overrides(dir.path()).unwrap();
        assert_eq!(count, 1);
        let tmpl = registry.get(LoopType::JobDiscovery).unwrap();
        assert_eq!(tmpl.name, "job_discovery_custom");
    }

    #[test]
    fn load_user_overrides_nonexistent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = DefaultPromptRegistry::new().unwrap();
        let count = registry.load_user_overrides(dir.path()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn all_templates_have_non_empty_system_prompt() {
        let registry = DefaultPromptRegistry::new().unwrap();
        for tmpl in registry.all() {
            assert!(
                !tmpl.system.trim().is_empty(),
                "template {:?} has empty system prompt",
                tmpl.loop_type
            );
        }
    }

    #[test]
    fn all_templates_have_non_empty_user_prompt() {
        let registry = DefaultPromptRegistry::new().unwrap();
        for tmpl in registry.all() {
            assert!(
                !tmpl.user.trim().is_empty(),
                "template {:?} has empty user prompt",
                tmpl.loop_type
            );
        }
    }
}
