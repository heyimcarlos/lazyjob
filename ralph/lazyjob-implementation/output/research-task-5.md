# Research: Task 5 — Application State Machine

## What exists

- `ApplicationStage` enum in `lazyjob-core/src/domain/application.rs` with 9 variants: Interested, Applied, PhoneScreen, Technical, Onsite, Offer, Accepted, Rejected, Withdrawn
- `ApplicationStage` has `as_str()`, `FromStr`, `Display`, `all()` — but NO transition logic yet
- `ApplicationRepository` in `lazyjob-core/src/repositories/application.rs` has basic CRUD but NO `transition_stage`
- `application_transitions` table exists in migration 001 with: id, application_id, from_stage, to_stage, transitioned_at, notes
- The table uses `notes` column (not `reason`)

## Transition rules (from task description)

Forward path: Interested → Applied → PhoneScreen → Technical → Onsite → Offer → Accepted
Terminal exits: any non-terminal → Withdrawn, any non-terminal → Rejected
Terminal states: Accepted, Rejected, Withdrawn (no transitions out)

The spec has a richer matrix with backward transitions (e.g., Applied → Interested), but the task description specifies a simpler forward-only model with universal Withdrawn/Rejected. I'll follow the task description.

## What needs to happen

1. Add `can_transition_to`, `valid_transitions`, `is_terminal` to `ApplicationStage`
2. Add `StageTransition` domain type (or just use inline — the table already exists)
3. Add `transition_stage(id, next_stage, reason)` to `ApplicationRepository`:
   - Fetch current application
   - Validate transition via `can_transition_to`
   - In a PG transaction: UPDATE applications SET stage, INSERT application_transitions
   - Return error if invalid transition
4. Write exhaustive tests

## Key decisions

- The `application_transitions` table has `notes` not `reason` — I'll use `notes` as the optional reason field
- No new migration needed — table already exists
- `StageTransition` struct: simple struct for returning transition records, not a full domain entity
- Error: use `CoreError::Validation` for invalid transitions (keeps error.rs unchanged)
