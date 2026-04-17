use super::types::RenderedPrompt;

pub fn build_anthropic_system_field(rendered: &RenderedPrompt) -> serde_json::Value {
    if rendered.cache_system_prompt {
        serde_json::json!([
            {
                "type": "text",
                "text": rendered.system,
                "cache_control": { "type": "ephemeral" }
            }
        ])
    } else {
        serde_json::json!(rendered.system)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::types::FewShotExample;

    fn make_rendered(cache: bool) -> RenderedPrompt {
        RenderedPrompt {
            system: "You are a helpful assistant.".into(),
            user: "Help me.".into(),
            few_shot: vec![],
            cache_system_prompt: cache,
            template_name: "test".into(),
            template_version: "1.0.0".into(),
        }
    }

    #[test]
    fn build_with_cache_control() {
        let rendered = make_rendered(true);
        let value = build_anthropic_system_field(&rendered);
        let arr = value.as_array().expect("should be array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "You are a helpful assistant.");
        assert_eq!(arr[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn build_without_cache_control() {
        let rendered = make_rendered(false);
        let value = build_anthropic_system_field(&rendered);
        assert_eq!(value.as_str().unwrap(), "You are a helpful assistant.");
    }

    #[test]
    fn few_shot_not_included_in_system_field() {
        let rendered = RenderedPrompt {
            system: "System prompt".into(),
            user: "User prompt".into(),
            few_shot: vec![FewShotExample {
                user: "example".into(),
                assistant: "response".into(),
            }],
            cache_system_prompt: true,
            template_name: "test".into(),
            template_version: "1.0.0".into(),
        };
        let value = build_anthropic_system_field(&rendered);
        let json_str = serde_json::to_string(&value).unwrap();
        assert!(!json_str.contains("example"));
    }
}
