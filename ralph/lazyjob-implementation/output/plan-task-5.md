# Plan: Task 5 — Application State Machine

## Files to modify

1. `lazyjob-core/src/domain/application.rs` — add `can_transition_to`, `valid_transitions`, `is_terminal`, `StageTransition` struct
2. `lazyjob-core/src/domain/mod.rs` — re-export `StageTransition`
3. `lazyjob-core/src/repositories/application.rs` — add `transition_stage` method + `transition_history` method

## Types/functions to define

### In `domain/application.rs`:
- `ApplicationStage::is_terminal(&self) -> bool` — true for Accepted, Rejected, Withdrawn
- `ApplicationStage::valid_transitions(&self) -> &'static [ApplicationStage]` — returns valid next states
- `ApplicationStage::can_transition_to(&self, next: ApplicationStage) -> bool` — checks if transition is valid
- `StageTransition` struct — id, application_id, from_stage, to_stage, transitioned_at, notes

### In `repositories/application.rs`:
- `ApplicationRepository::transition_stage(&self, id: &ApplicationId, next_stage: ApplicationStage, reason: Option<&str>) -> Result<StageTransition>` — atomic PG transaction
- `ApplicationRepository::transition_history(&self, id: &ApplicationId) -> Result<Vec<StageTransition>>` — fetch all transitions for an application

## Tests to write

### Unit tests (in domain/application.rs):
- `test_is_terminal` — Accepted, Rejected, Withdrawn are terminal; others are not
- `test_valid_forward_transitions` — each forward step works
- `test_any_to_withdrawn` — all non-terminal states can transition to Withdrawn
- `test_any_to_rejected` — all non-terminal states can transition to Rejected
- `test_terminal_has_no_transitions` — Accepted/Rejected/Withdrawn have empty valid_transitions
- `test_cannot_skip_stages` — e.g., Interested cannot go to Technical
- `test_exhaustive_matrix` — test every pair (from, to) against expected result

### Integration tests (in repositories/mod.rs):
- `transition_stage_succeeds` — insert app, transition, verify stage updated + transition record created
- `transition_stage_invalid_rejects` — attempt invalid transition, verify error
- `transition_history_returns_ordered` — multiple transitions, verify chronological order

## No new migrations needed
The `application_transitions` table already exists in migration 001.
