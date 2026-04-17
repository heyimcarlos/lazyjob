pub mod cache;
pub mod company_research;
pub mod cover_letter;
pub mod engine;
pub mod error;
pub mod interview_prep;
pub mod job_discovery;
pub mod outreach;
pub mod registry;
pub mod resume_tailor;
pub mod sanitizer;
pub mod types;

pub use engine::SimpleTemplateEngine;
pub use error::{Result as TemplateResult, TemplateError};
pub use registry::DefaultPromptRegistry;
pub use sanitizer::sanitize_user_value;
pub use types::{FewShotExample, LoopType, PromptTemplate, RenderedPrompt, TemplateVars};
