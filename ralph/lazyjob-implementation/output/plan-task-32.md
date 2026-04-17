# Plan: Task 32 — Anti-Fabrication

## Files to Create/Modify

### Create
- `crates/lazyjob-llm/src/anti_fabrication.rs` — main module

### Modify
- `crates/lazyjob-llm/src/lib.rs` — add `pub mod anti_fabrication` + re-exports
- `crates/lazyjob-llm/src/prompts/resume_tailor.rs` — add `validate_grounding()` function
- `crates/lazyjob-llm/src/prompts/cover_letter.rs` — add `validate_grounding()` function

## Types/Functions to Define

### anti_fabrication.rs
- `FabricationLevel` enum: `Grounded`, `Embellished`, `Fabricated` (derive Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)
- `ProhibitedPhrase` struct: `phrase: String`, `position: usize`
- `GroundingReport` struct: `level: FabricationLevel`, `evidence: Vec<String>`, `ungrounded_claims: Vec<String>`
- `is_grounded_claim(claim: &str, life_sheet: &LifeSheet) -> FabricationLevel`
- `check_grounding(claims: &[String], life_sheet: &LifeSheet) -> GroundingReport`
- `prohibited_phrase_detector(text: &str) -> Vec<ProhibitedPhrase>`
- `prompt_injection_guard(user_input: &str) -> bool`
- `PROHIBITED_PHRASES: &[&str]` — const list of clichés

### resume_tailor.rs additions
- `validate_grounding(output: &ResumeTailorOutput, life_sheet: &LifeSheet) -> Result<GroundingReport>`

### cover_letter.rs additions
- `validate_grounding(output: &CoverLetterOutput, life_sheet: &LifeSheet) -> Result<GroundingReport>`

## Tests to Write

### Unit tests in anti_fabrication.rs
- `grounded_claim_with_matching_company` — claim mentioning a company from LifeSheet → Grounded
- `grounded_claim_with_matching_skill` — claim mentioning a skill → Grounded
- `grounded_claim_with_matching_achievement` — claim close to achievement description → Grounded
- `embellished_claim_partial_match` — claim with some evidence but added details → Embellished
- `fabricated_claim_no_evidence` — completely unrelated claim → Fabricated
- `check_grounding_report` — multiple claims produce correct report
- `prohibited_phrases_detected` — text with clichés returns correct phrases
- `prohibited_phrases_clean_text` — normal text returns empty vec
- `prohibited_phrases_case_insensitive` — uppercase clichés still detected
- `injection_guard_detects_role_switch` — "\n\nSystem:" detected
- `injection_guard_detects_ignore_instructions` — "ignore previous instructions" detected
- `injection_guard_clean_input` — normal text returns false
- `injection_guard_case_insensitive` — "SYSTEM:" variant detected

### Tests in resume_tailor.rs
- `validate_grounding_all_grounded` — output grounded in life sheet

### Tests in cover_letter.rs  
- `validate_grounding_detects_prohibited` — output with clichés detected

## No migrations needed
