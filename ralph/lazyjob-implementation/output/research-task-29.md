# Research: Task 29 — Applications Kanban TUI

## Task Description
Implement ApplicationsView as a horizontal kanban board with 9 columns (one per ApplicationStage), card navigation (h/l between columns, j/k within), stage transitions (m forward, M backward), confirmation dialog, and days-in-stage coloring.

## Existing Code

### ApplicationsView (stub)
- `crates/lazyjob-tui/src/views/applications.rs` — empty stub with placeholder text
- Implements View trait: render(), handle_key(), name()

### Domain Types
- `ApplicationStage` (9 variants): Interested, Applied, PhoneScreen, Technical, Onsite, Offer, Accepted, Rejected, Withdrawn
- `Application` struct: id, job_id, stage, submitted_at, updated_at, resume_version, cover_letter_version, notes
- `StageTransition`: id, application_id, from_stage, to_stage, transitioned_at, notes
- State machine: forward-only transitions + any non-terminal → Withdrawn/Rejected

### Repositories
- `ApplicationRepository`: insert, find_by_id, list, update, delete, transition_stage, transition_history
- `transition_stage(id, next_stage, reason)` — validates transition, runs atomic PG transaction

### Widgets Available
- `ConfirmDialog` — centered overlay with Yes/No buttons, uses modal_dialog::centered_rect
- `ProgressBar`, `StatBlock`, `JobCard`, `ModalDialog`

### Action Enum
- Needs new variants for stage transitions
- Current: Quit, NavigateTo, NavigateBack, ToggleHelp, Refresh, ScrollDown, ScrollUp, Select, OpenJob, ApplyToJob, TailorResume, GenerateCoverLetter, OpenUrl, CancelRalphLoop, RalphDetail, EnterSearch, ExitSearch

### App
- `pool: Option<PgPool>` — available for DB queries
- `handle_action()` dispatches actions
- `active_view_mut()` returns mutable view reference
- Pattern for DB loading: `load_jobs()` queries repo, sets data on view

## Key Design Decisions

1. **Kanban columns**: All 9 stages displayed. Non-terminal stages get wider columns, terminal stages are narrower.
2. **Card data**: Need job title + company name. Application only has job_id, so either:
   - Join with jobs table when loading, or
   - Store denormalized data in a card struct
   Decision: Store denormalized ApplicationCard with title+company from a join query.
3. **Days in stage**: Computed from `updated_at` field (which tracks last stage change).
4. **Confirmation dialog**: Rendered as overlay within the view's render() method using ConfirmDialog widget.
5. **m/M keys**: m advances to natural next stage (next in forward chain); M has limited use since transitions are forward-only, but can trigger Withdrawn.
6. **Stage transition action**: New `TransitionApplication(ApplicationId, ApplicationStage)` action variant.
7. **DB loading**: Add `load_applications()` to App, similar to `load_jobs()`.
