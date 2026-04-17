# Plan: Task 28 — Job Detail TUI

## Files to Modify

1. **crates/lazyjob-tui/src/views/job_detail.rs** — Full rewrite: add fields, layout, key handling, tests
2. **crates/lazyjob-tui/src/app.rs** — Add viewing_job_detail flag, wire OpenJob action, modify active_view_mut and NavigateBack
3. **crates/lazyjob-tui/src/action.rs** — Add ApplyToJob(JobId), TailorResume(JobId), GenerateCoverLetter(JobId), OpenUrl(String) action variants

## Types/Functions/Structs

### JobDetailView (job_detail.rs)
- `struct JobDetailView` with fields: job: Option<Job>, application: Option<Application>, transitions: Vec<StageTransition>, scroll_offset: u16
- `pub fn set_job(&mut self, job: Job)` — sets job, resets scroll
- `pub fn set_application(&mut self, application: Option<Application>, transitions: Vec<StageTransition>)`
- `pub fn clear(&mut self)` — clears all data
- `fn render_metadata(frame, area, theme, job, application)` — left panel
- `fn render_description(frame, area, theme, description, scroll_offset)` — right panel with scroll
- `fn render_action_bar(frame, area, theme)` — bottom action hints
- `fn render_history(lines, theme, transitions)` — application timeline within metadata panel
- View trait impl: render (two-column layout), handle_key (scroll + action keys), name

### App changes (app.rs)
- `viewing_job_detail: bool` field
- `OpenJob(id)` handler: find job in jobs_list.jobs by id, call job_detail.set_job(), set flag
- `active_view_mut()`: if Jobs && viewing_job_detail → &mut views.job_detail
- `NavigateBack`: if viewing_job_detail → clear flag, return early

### Action additions (action.rs)
- `ApplyToJob(JobId)` — future task will implement
- `TailorResume(JobId)` — future task will implement
- `GenerateCoverLetter(JobId)` — future task will implement
- `OpenUrl(String)` — opens URL in default browser

## Tests to Write

### Unit tests (job_detail.rs)
- set_job_stores_data — verifies set_job populates the job field
- set_job_resets_scroll — verifies scroll_offset resets to 0
- clear_removes_data — verifies clear sets job to None
- handle_key_esc_returns_navigate_back — Esc key returns NavigateBack
- handle_key_j_scrolls_down — j key increments scroll
- handle_key_k_scrolls_up — k key decrements scroll (clamped to 0)
- handle_key_o_returns_open_url — o key returns OpenUrl with job's URL
- handle_key_o_returns_none_when_no_url — o key returns None if job has no URL
- handle_key_a_returns_apply — a key returns ApplyToJob
- handle_key_r_returns_tailor — r key returns TailorResume
- handle_key_c_returns_cover_letter — c key returns GenerateCoverLetter
- renders_with_job_data — render with a job set doesn't panic
- renders_empty_state — render with no job doesn't panic
- renders_with_application_history — render with transitions doesn't panic

### App tests (app.rs)
- open_job_activates_detail_view — OpenJob sets viewing_job_detail = true
- navigate_back_from_detail_returns_to_jobs — NavigateBack clears viewing_job_detail
- tab_switch_clears_detail_view — NavigateTo clears viewing_job_detail

## Migrations
None needed.
