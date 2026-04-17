# Research: Task 27 — jobs-list-tui (Completion Pass)

## Current State

A previous iteration implemented 90% of JobsListView (875 lines, 44 tests). This iteration completes the remaining gaps.

### What's Already Done
- JobsListView struct with jobs, filtered indices, table_state, filter, sort, search
- JobFilter enum: All, New, HighMatch, Applied (Applied always returns false)
- SortBy enum: Date, Match, Company with cycling
- Table rendering: 5 columns (Title, Company, Match, G, Posted)
- Search mode: /, Esc, Enter, Backspace, Char handling
- Filter/sort cycling: f, s keys
- Scrolling: j/k via keymap → ScrollDown/ScrollUp → handle_key(Down/Up)
- Status bar with search/normal mode display
- 44 passing tests including render tests

### Gaps Identified
1. **Missing Stage column** — Task requires: Title, Company, Match%, Ghost?, **Stage**, Posted. Currently only 5 columns, no Stage.
2. **Enter key is broken** — Line 473: `KeyCode::Enter | _ => None` — a catch-all that swallows Enter AND all unmatched keys. Enter should open the selected job.
3. **Applied filter always returns false** — Line 175: `JobFilter::Applied => false` — needs application stage data.
4. **No DB loading** — `Action::Refresh` is a no-op in App (line 105). No way to populate jobs from DB.
5. **No OpenJob action** — Action enum lacks a variant for opening a job detail view.

### Architecture Constraints
- `App::handle_action` is synchronous — cannot do async DB queries directly
- `ViewId` has no `JobDetail` variant (that's task 28)
- Need to store per-job application stages somehow for Stage column and Applied filter
- `Job` domain type has no stage field — stage is on `Application` which references `job_id`

### Design Decisions for This Iteration
1. Add `OpenJob(JobId)` to Action enum — returned by Enter key, handled as no-op until task 28
2. Add `application_stages: HashMap<JobId, String>` to JobsListView — populated externally
3. Add Stage column between Ghost and Posted columns
4. Wire Applied filter to check application_stages
5. Add async `load_jobs` method to App that queries JobRepository
6. Fix the `Enter | _` catch-all pattern
