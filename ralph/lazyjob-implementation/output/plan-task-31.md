# Plan: Task 31 — Prompt Templates

## Files to Create

### Infrastructure
1. `crates/lazyjob-llm/src/prompts/mod.rs` — module declarations, re-exports
2. `crates/lazyjob-llm/src/prompts/types.rs` — LoopType, PromptTemplate, RenderedPrompt, TemplateVars, FewShotExample
3. `crates/lazyjob-llm/src/prompts/error.rs` — TemplateError enum
4. `crates/lazyjob-llm/src/prompts/engine.rs` — SimpleTemplateEngine, interpolate()
5. `crates/lazyjob-llm/src/prompts/registry.rs` — DefaultPromptRegistry
6. `crates/lazyjob-llm/src/prompts/sanitizer.rs` — sanitize_user_value(), template_vars! macro
7. `crates/lazyjob-llm/src/prompts/cache.rs` — build_anthropic_system_field()

### TOML Templates
8. `crates/lazyjob-llm/src/templates/base_system.toml`
9. `crates/lazyjob-llm/src/templates/job_discovery.toml`
10. `crates/lazyjob-llm/src/templates/company_research.toml`
11. `crates/lazyjob-llm/src/templates/resume_tailoring.toml`
12. `crates/lazyjob-llm/src/templates/cover_letter.toml`
13. `crates/lazyjob-llm/src/templates/interview_prep.toml`
14. `crates/lazyjob-llm/src/templates/salary_negotiation.toml`
15. `crates/lazyjob-llm/src/templates/networking.toml`
16. `crates/lazyjob-llm/src/templates/error_response.toml`

### Per-Loop Context + Validation Modules
17. `crates/lazyjob-llm/src/prompts/job_discovery.rs` — JobDiscoveryContext, JobDiscoveryOutput
18. `crates/lazyjob-llm/src/prompts/company_research.rs` — CompanyResearchContext, CompanyResearchOutput
19. `crates/lazyjob-llm/src/prompts/resume_tailor.rs` — ResumeTailorContext, ResumeTailorOutput
20. `crates/lazyjob-llm/src/prompts/cover_letter.rs` — CoverLetterContext, CoverLetterOutput
21. `crates/lazyjob-llm/src/prompts/interview_prep.rs` — InterviewPrepContext, InterviewPrepOutput

### Modified Files
22. `crates/lazyjob-llm/src/lib.rs` — add `pub mod prompts`
23. `crates/lazyjob-llm/Cargo.toml` — add tracing dep

## Types/Structs to Define

### Core Types (types.rs)
- `LoopType` enum (9 variants)
- `PromptTemplate` struct (TOML-deserializable)
- `FewShotExample` struct
- `RenderedPrompt` struct with `into_chat_messages()`
- `TemplateVars` type alias (BTreeMap<String, String>)

### Error (error.rs)
- `TemplateError` enum (MissingVariable, ParseError, NotRegistered, etc.)

### Per-Loop Context Structs
- `JobDiscoveryContext` { companies, skills, experience_summary, preferences }
- `CompanyResearchContext` { company_name, target_role }
- `ResumeTailorContext` { job_description, user_experience, requirements_analysis }
- `CoverLetterContext` { user_name, company_name, job_title, company_research, relevant_experience, job_description_summary }
- `InterviewPrepContext` { interview_type, company_name, job_title, job_description, company_research, user_background }

### Per-Loop Output Structs
- `JobDiscoveryOutput` { jobs: Vec<DiscoveredJob>, summary }
- `CompanyResearchOutput` { industry, size, tech_stack, culture, recent_news }
- `ResumeTailorOutput` { summary, experience_bullets, skills_section }
- `CoverLetterOutput` { paragraphs: Vec<String>, template_type }
- `InterviewPrepOutput` { questions: Vec<InterviewQuestion> }

## Tests
- Learning test: toml_parses_prompt_template (verify TOML deserialization of PromptTemplate)
- Engine: interpolate_all_vars, interpolate_missing_var, interpolate_literal_braces, interpolate_empty_value
- Sanitizer: strips_injection, preserves_normal_text, trims_whitespace
- Registry: loads_all_templates, get_returns_template, override_replaces, all_returns_sorted
- Cache: with_cache_control, without_cache_control
- Per-loop: context_to_vars, validate_valid_output, validate_invalid_output (x5 loops)
- RenderedPrompt: into_chat_messages

## Migrations
None needed.
