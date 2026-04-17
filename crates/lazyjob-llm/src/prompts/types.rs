use crate::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrep,
    SalaryNegotiation,
    Networking,
    ErrorResponse,
    BaseSystem,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    pub version: String,
    pub loop_type: LoopType,
    pub system: String,
    pub user: String,
    #[serde(default)]
    pub few_shot_examples: Vec<FewShotExample>,
    #[serde(default = "default_true")]
    pub cache_system_prompt: bool,
    #[serde(default)]
    pub output_schema: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct FewShotExample {
    pub user: String,
    pub assistant: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub system: String,
    pub user: String,
    pub few_shot: Vec<FewShotExample>,
    pub cache_system_prompt: bool,
    pub template_name: String,
    pub template_version: String,
}

impl RenderedPrompt {
    pub fn into_chat_messages(self) -> Vec<ChatMessage> {
        let mut msgs = Vec::new();
        msgs.push(ChatMessage::System(self.system));
        for ex in self.few_shot {
            msgs.push(ChatMessage::User(ex.user));
            msgs.push(ChatMessage::Assistant(ex.assistant));
        }
        msgs.push(ChatMessage::User(self.user));
        msgs
    }
}

pub type TemplateVars = std::collections::BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_type_serde_round_trip() {
        let lt = LoopType::JobDiscovery;
        let json = serde_json::to_string(&lt).unwrap();
        assert_eq!(json, "\"job_discovery\"");
        let parsed: LoopType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, lt);
    }

    #[test]
    fn loop_type_all_variants_serialize() {
        let variants = [
            (LoopType::JobDiscovery, "job_discovery"),
            (LoopType::CompanyResearch, "company_research"),
            (LoopType::ResumeTailoring, "resume_tailoring"),
            (LoopType::CoverLetterGeneration, "cover_letter_generation"),
            (LoopType::InterviewPrep, "interview_prep"),
            (LoopType::SalaryNegotiation, "salary_negotiation"),
            (LoopType::Networking, "networking"),
            (LoopType::ErrorResponse, "error_response"),
            (LoopType::BaseSystem, "base_system"),
        ];
        for (variant, expected) in variants {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{}\"", expected));
        }
    }

    #[test]
    fn rendered_prompt_into_chat_messages() {
        let rendered = RenderedPrompt {
            system: "You are a helper.".into(),
            user: "Do something.".into(),
            few_shot: vec![FewShotExample {
                user: "Example input".into(),
                assistant: "Example output".into(),
            }],
            cache_system_prompt: true,
            template_name: "test_v1".into(),
            template_version: "1.0.0".into(),
        };
        let msgs = rendered.into_chat_messages();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role(), "system");
        assert_eq!(msgs[0].content(), "You are a helper.");
        assert_eq!(msgs[1].role(), "user");
        assert_eq!(msgs[1].content(), "Example input");
        assert_eq!(msgs[2].role(), "assistant");
        assert_eq!(msgs[2].content(), "Example output");
        assert_eq!(msgs[3].role(), "user");
        assert_eq!(msgs[3].content(), "Do something.");
    }

    #[test]
    fn rendered_prompt_without_few_shot() {
        let rendered = RenderedPrompt {
            system: "System.".into(),
            user: "User.".into(),
            few_shot: vec![],
            cache_system_prompt: false,
            template_name: "test".into(),
            template_version: "1.0.0".into(),
        };
        let msgs = rendered.into_chat_messages();
        assert_eq!(msgs.len(), 2);
    }
}
