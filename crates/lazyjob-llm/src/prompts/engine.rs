use super::error::{Result, TemplateError};
use super::types::{FewShotExample, PromptTemplate, RenderedPrompt, TemplateVars};

pub struct SimpleTemplateEngine;

impl SimpleTemplateEngine {
    pub fn render(&self, template: &PromptTemplate, vars: &TemplateVars) -> Result<RenderedPrompt> {
        let system = interpolate(&template.system, vars, &template.name)?;
        let user = interpolate(&template.user, vars, &template.name)?;

        let few_shot = template
            .few_shot_examples
            .iter()
            .map(|ex| {
                Ok(FewShotExample {
                    user: interpolate(&ex.user, vars, &template.name)?,
                    assistant: interpolate(&ex.assistant, vars, &template.name)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(RenderedPrompt {
            system,
            user,
            few_shot,
            cache_system_prompt: template.cache_system_prompt,
            template_name: template.name.clone(),
            template_version: template.version.clone(),
        })
    }
}

fn interpolate(text: &str, vars: &TemplateVars, template_name: &str) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == '}' {
                    closed = true;
                    break;
                }
                key.push(inner);
            }
            if !closed {
                result.push('{');
                result.push_str(&key);
                continue;
            }
            let value = vars
                .get(&key)
                .ok_or_else(|| TemplateError::MissingVariable {
                    name: key.clone(),
                    template: template_name.to_owned(),
                })?;
            result.push_str(value);
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(system: &str, user: &str) -> PromptTemplate {
        PromptTemplate {
            name: "test_template".into(),
            version: "1.0.0".into(),
            loop_type: super::super::types::LoopType::JobDiscovery,
            system: system.into(),
            user: user.into(),
            few_shot_examples: vec![],
            cache_system_prompt: false,
            output_schema: None,
        }
    }

    #[test]
    fn interpolate_all_vars() {
        let mut vars = TemplateVars::new();
        vars.insert("name".into(), "Alice".into());
        vars.insert("role".into(), "engineer".into());
        let result = interpolate("Hello {name}, you are a {role}.", &vars, "test").unwrap();
        assert_eq!(result, "Hello Alice, you are a engineer.");
    }

    #[test]
    fn interpolate_missing_var_errors() {
        let vars = TemplateVars::new();
        let err = interpolate("Hello {name}", &vars, "test_tmpl").unwrap_err();
        match err {
            TemplateError::MissingVariable { name, template } => {
                assert_eq!(name, "name");
                assert_eq!(template, "test_tmpl");
            }
            _ => panic!("expected MissingVariable"),
        }
    }

    #[test]
    fn interpolate_literal_unmatched_brace() {
        let vars = TemplateVars::new();
        let result = interpolate("JSON example: {key without close", &vars, "test").unwrap();
        assert_eq!(result, "JSON example: {key without close");
    }

    #[test]
    fn interpolate_empty_value() {
        let mut vars = TemplateVars::new();
        vars.insert("name".into(), "".into());
        let result = interpolate("Hello {name}!", &vars, "test").unwrap();
        assert_eq!(result, "Hello !");
    }

    #[test]
    fn engine_renders_system_and_user() {
        let template = make_template("System for {role}", "User wants {task}");
        let mut vars = TemplateVars::new();
        vars.insert("role".into(), "assistant".into());
        vars.insert("task".into(), "help".into());
        let engine = SimpleTemplateEngine;
        let rendered = engine.render(&template, &vars).unwrap();
        assert_eq!(rendered.system, "System for assistant");
        assert_eq!(rendered.user, "User wants help");
        assert_eq!(rendered.template_name, "test_template");
    }

    #[test]
    fn engine_renders_few_shot_examples() {
        let template = PromptTemplate {
            name: "test".into(),
            version: "1.0.0".into(),
            loop_type: super::super::types::LoopType::JobDiscovery,
            system: "System".into(),
            user: "User".into(),
            few_shot_examples: vec![FewShotExample {
                user: "Input for {name}".into(),
                assistant: "Output for {name}".into(),
            }],
            cache_system_prompt: false,
            output_schema: None,
        };
        let mut vars = TemplateVars::new();
        vars.insert("name".into(), "test".into());
        let engine = SimpleTemplateEngine;
        let rendered = engine.render(&template, &vars).unwrap();
        assert_eq!(rendered.few_shot.len(), 1);
        assert_eq!(rendered.few_shot[0].user, "Input for test");
        assert_eq!(rendered.few_shot[0].assistant, "Output for test");
    }

    #[test]
    fn interpolate_no_placeholders() {
        let vars = TemplateVars::new();
        let result = interpolate("No variables here.", &vars, "test").unwrap();
        assert_eq!(result, "No variables here.");
    }
}
