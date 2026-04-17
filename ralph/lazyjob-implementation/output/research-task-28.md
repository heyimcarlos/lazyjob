# Research: Task 28 — Job Detail TUI

## Current State

### JobDetailView Stub
- `crates/lazyjob-tui/src/views/job_detail.rs`: Zero-field unit struct, renders placeholder text, handles no keys
- Already a field on `Views` struct (`views.job_detail`)
- Not routed in `active_view_mut()` — never rendered or receives input

### Navigation Model
- `ViewId` is a 6-variant enum (Dashboard, Jobs, Applications, Contacts, Ralph, Settings) — no JobDetail variant
- `ViewId` is `Copy + Eq + Hash` with `tab_index()` — adding a payload-carrying variant would break this
- `App::active_view_mut()` maps ViewId → &mut dyn View — no arm for job_detail
- `Action::OpenJob(JobId)` exists but is a no-op in `handle_action`

### Key Routing
- `event_loop::map_key_to_action` uses `KeyContext::from_view_id(app.active_view)` to determine context
- When `active_view == ViewId::Jobs`, context is `KeyContext::Jobs`
- Keys not handled by keymap fall through to `active_view_mut().handle_key()`

### Domain Types
- `Job`: 14 fields including title, company_name, location, url, description (Option<String>), salary_min/max, match_score, ghost_score, discovered_at, notes
- `Application`: id, job_id, stage, submitted_at, updated_at, resume/cover_letter versions, notes
- `StageTransition`: from_stage, to_stage, transitioned_at, notes

## Design Decision: Sub-view Pattern

**Approach**: Add `viewing_job_detail: bool` to App instead of a new ViewId variant.

Rationale:
- ViewId stays simple (Copy, no payload, clean tab_index)
- Header tab stays highlighted on "Jobs" when viewing detail (natural UX)
- KeyContext stays as Jobs — job detail keys fall through to view's handle_key
- NavigateBack first clears viewing_job_detail before popping prev_view

Flow:
1. OpenJob(id) → set job on job_detail, set viewing_job_detail = true
2. active_view_mut() → if Jobs && viewing_job_detail → &mut views.job_detail
3. NavigateBack → if viewing_job_detail → clear flag, don't pop prev_view
4. Tab switching (1-6) → clears viewing_job_detail automatically

## Layout Plan

```
┌─────────────────────────────────────────┐
│ Job Detail: Senior Rust Engineer        │
├────────────────────┬────────────────────┤
│ METADATA           │ DESCRIPTION        │
│ Company: Acme      │ (scrollable)       │
│ Location: Remote   │                    │
│ Salary: $120-180k  │                    │
│ Posted: 2d ago     │                    │
│ Match: 85%         │                    │
│ Ghost: Low         │                    │
│ Stage: Applied     │                    │
│                    │                    │
│ APPLICATION HIST   │                    │
│ ● Applied (2d ago) │                    │
│ ● Interested (5d)  │                    │
├────────────────────┴────────────────────┤
│ a=Apply r=Resume c=Cover o=Open Esc=Back│
└─────────────────────────────────────────┘
```

## Dependencies
- No new external crates needed
- Uses existing ratatui widgets (Paragraph, Block, Layout)
- Uses existing domain types from lazyjob-core
