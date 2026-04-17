# Research: Task 31 — Prompt Templates

## Task Description
Create per-loop-type prompt modules in `lazyjob-llm/src/prompts/` with system_prompt, user_prompt, and validate_output functions. Define context structs per loop type. Validate outputs with serde_json.

## Spec Analysis (specs/17-ralph-prompt-templates-implementation-plan.md)

### Architecture
- All template code lives in `lazyjob-llm/src/prompts/`
- Core types: LoopType enum, PromptTemplate (TOML-deserializable), RenderedPrompt, TemplateVars (BTreeMap)
- SimpleTemplateEngine: `{variable}` interpolation with MissingVariable errors
- DefaultPromptRegistry: loads embedded TOML templates via include_str!
- Sanitizer: strips prompt injection patterns from user-supplied values
- Cache helper: Anthropic cache_control injection for system prompts
- 9 TOML template files embedded at compile time

### Template Variables per Loop Type
| Loop Type | Variables |
|---|---|
| BaseSystem | none (static) |
| JobDiscovery | companies, skills, experience_summary, preferences |
| CompanyResearch | company_name, target_role |
| ResumeTailoring | job_description, user_experience, requirements_analysis |
| CoverLetterGeneration | user_name, company_name, job_title, company_research, relevant_experience, job_description_summary |
| InterviewPrep | interview_type, company_name, job_title, job_description, company_research, user_background |
| SalaryNegotiation | offer_details, market_data, target_compensation |
| Networking | company_name, user_background, goal, contacts |
| ErrorResponse | none (static) |

### Output Validation
Each loop type should define an output struct that can be deserialized from the LLM's JSON response. validate_output() parses the raw string into the typed struct.

## Existing Codebase Context

### lazyjob-llm current state
- 7 modules: cost, error, message, mock, provider, providers, registry
- ChatMessage enum: System(String), User(String), Assistant(String)
- LlmProvider trait with complete() method
- No prompts/ directory exists yet

### lazyjob-core types needed
- LifeSheet (life_sheet/types.rs): basics, work_experience, education, skills, etc.
- Job (domain/job.rs): title, company_name, description, location, salary_min/max, etc.

### Dependencies already available
- serde, serde_json, toml (in workspace), thiserror, anyhow, tracing (in workspace)
- No new crates needed

## Key Decisions
1. Use TOML templates embedded via include_str! per spec
2. Context structs per loop type with to_template_vars() methods
3. validate_output() returns typed output structs via serde_json::from_str
4. Sanitizer strips injection patterns before interpolation
5. RenderedPrompt::into_chat_messages() converts to Vec<ChatMessage> for LlmProvider
