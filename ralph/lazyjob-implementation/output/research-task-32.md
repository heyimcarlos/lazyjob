# Research: Task 32 — Anti-Fabrication

## Existing Code Analysis

### LifeSheet (lazyjob-core/src/life_sheet/types.rs)
- `LifeSheet` contains: `basics`, `work_experience: Vec<WorkExperience>`, `education: Vec<Education>`, `skills: Vec<SkillCategory>`, `certifications: Vec<Certification>`, `projects: Vec<Project>`, `languages: Vec<Language>`
- `WorkExperience` has `company`, `position`, `achievements: Vec<Achievement>`, `tech_stack: Vec<String>`
- `Achievement` has `description: String`, optional `metric_type`, `metric_value`, `metric_unit`
- `SkillCategory` has `name`, `skills: Vec<Skill>` where `Skill` has `name`, optional `years_experience`, `proficiency`
- All types derive `Serialize, Deserialize, Debug, Clone, PartialEq`

### Existing Sanitizer (lazyjob-llm/src/prompts/sanitizer.rs)
- `sanitize_user_value()` replaces injection patterns like `\n\nSystem:`, `\n\nAssistant:`, `Ignore previous instructions`, `###`
- This is a template-variable level sanitizer — `prompt_injection_guard` in this task is a broader input-level detector that returns bool (detect vs. sanitize)

### Prompt Modules
- `resume_tailor.rs`: `ResumeTailorContext` → `to_template_vars()`, `validate_output()` → `ResumeTailorOutput { summary, experience: Vec<ExperienceEntry>, skills }`
- `cover_letter.rs`: `CoverLetterContext` → `to_template_vars()`, `validate_output()` → `CoverLetterOutput { paragraphs: Vec<String>, template_type, subject_line, key_themes }`
- Both modules have `system_prompt()`, `user_prompt()`, `validate_output()` functions

### Module Structure
- `lazyjob-llm/src/lib.rs` declares `pub mod prompts` (no re-export of prompts internals)
- `prompts/mod.rs` has 11 submodules; anti_fabrication should live at `lazyjob-llm/src/anti_fabrication.rs` per task description

## Design Decisions

1. **Placement**: `lazyjob-llm/src/anti_fabrication.rs` as a top-level module in lazyjob-llm (not under prompts/) since it's used by multiple pipeline stages
2. **is_grounded_claim**: Text-matching approach — extract key terms (company names, skills, positions, metrics) from LifeSheet and check if the claim references them. No LLM needed.
3. **FabricationLevel scoring**: Grounded = claim contains identifiable LifeSheet evidence; Embellished = some evidence but with added claims; Fabricated = no traceable evidence
4. **prohibited_phrase_detector**: Static list of overused cover letter clichés. Returns Vec<ProhibitedPhrase> with the phrase text and its position.
5. **prompt_injection_guard**: Broader than sanitizer — case-insensitive detection of role-switching, instruction overrides, encoding tricks. Returns bool (true = injection detected).
6. **Pipeline integration**: Add `check_fabrication()` and `check_prohibited_phrases()` functions that take output text + LifeSheet and return validation results. Wire into validate_output in resume_tailor and cover_letter modules.
