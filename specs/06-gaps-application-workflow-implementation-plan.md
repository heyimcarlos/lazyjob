# Gap Analysis: Application Workflow — Implementation Plan

## Spec Reference
- **Spec file**: `specs/06-gaps-application-workflow.md`
- **Status**: Gap Analysis (Researching)
- **Last updated**: 2026-04-15

## Executive Summary

This gap analysis identifies 10 critical gaps in the application workflow system (GAP-59 through GAP-68), spanning cross-source deduplication, multi-offer comparison, rejection email automation, bulk operations, deadline tracking, async challenge tracking, interview feedback recording, priority ranking, archive/cleanup, and contact relationship tracking. The implementation plan organizes these into a phased approach, with critical gaps addressed first and moderate gaps following.

## Problem Statement

The application workflow system (application-state-machine, application-workflow-actions, application-pipeline-metrics, 12-15-interview-salary-networking-notifications) is well-specified for core operations but has significant gaps in: cross-source deduplication, multi-offer comparison, rejection follow-up automation, bulk operations, deadline tracking, async challenge sub-states, interview feedback recording, priority ranking, archive policies, and contact relationship management.

## Implementation Phases

### Phase 1: Critical Gaps (Foundational Infrastructure)

#### GAP-59: Cross-Source Application Deduplication
1. Define `JobFingerprint` struct: normalized company name + title + description hash
2. Add `source_job_id` column to `applications` table to track original source IDs
3. Create `deduplication_service` module with fuzzy matching (Levenshtein distance for title, company name normalization)
4. Implement "merge vs. link" strategy: link as `same_job_sources[]` array, don't merge (preserves per-source data)
5. Add `application_group_id` to group duplicate applications
6. Build consolidated view in TUI showing grouped applications with source breakdown
7. Deduplication happens at apply-time, not discovery-time (Jobs remain separate, Applications link)

#### GAP-60: Multi-Offer Comparison UI
1. Extend `Offer` struct with `equity_annual_vest_value`, `benefits_value`, `signing_bonus` fields
2. Create `OfferComparison` struct with `total_comp`, `weighted_score`, `recommendation`
3. Build `OfferComparisonService` with `compare_offers(offer_ids: Vec<i64>) -> ComparisonResult`
4. Add weighted factor scoring (remote_policy, growth, role_fit, commute, PTO as configurable weights)
5. Implement expiration tracking with urgent surfaced in TUI
6. Create `NegotiationScenario` for "what-if" modeling
7. TUI: side-by-side comparison table view with total comp normalization

### Phase 2: Important Gaps

#### GAP-61: Rejection Email Response Automation
1. Create `RejectionResponseTemplate` struct with personalization fields
2. Define `StayInTouchCadence` enum: ThreeMonth, SixMonth, OneYear
3. Build `RejectionFollowupService` with template generation and scheduling
4. Integrate with CompanyRecord for personalization context
5. Track future_opportunity alerts when same company posts new role
6. LinkedIn connection suggestion after rejection

#### GAP-62: Bulk Application Operations
1. Add `BulkOperation` enum: BulkStageTransition, BulkArchive, BulkDelete
2. Implement selective filters: stage, last_contact_age, priority, source
3. Add `BulkOperationHistory` table for undo support
4. Implement undo window (5 minutes) with rollback capability
5. TUI: progress bar for bulk operations, confirmation dialog with count preview

#### GAP-63: Application Response Deadline Tracking
1. Add `response_deadline` column to `applications` table (nullable date)
2. Add `verbal_offer_deadline` to `offers` table (distinct from `expiry_date`)
3. Implement `DeadlineService` with conflict detection
4. Custom deadline override per application
5. Reminder triggers: 7 days before, 1 day before, day of

### Phase 3: Moderate Gaps

#### GAP-64: Async Technical Challenge Sub-State
1. Add `TechnicalChallenge` table linked to `application_id`
2. Sub-states: Sent, Submitted, Due, Expired
3. Auto-advance trigger on email parsing (future work, requires email integration)
4. Challenge reminder at N days before deadline
5. Store challenge link/instructions in `challenge_url` field

#### GAP-65: Interview Feedback Recording
1. Add `InterviewFeedback` struct: self_assessment (1-5), recruiter_feedback_text, hirer_feedback_text, sentiment_score
2. Create post-interview prompt in TUI (optional, non-blocking)
3. Feedback → outcome correlation stored in `interviews` table
4. Pattern detection in `FeedbackAnalysisService`

#### GAP-66: Application Priority/Ranking System
1. Add `priority` column to `applications` table: enum (High, Medium, Low, Unset)
2. Add `priority_decay_days` config (default 14)
3. Implement AI priority suggestions via `PrioritySuggestionService`
4. TUI: sort by priority within kanban stage
5. Priority decay job runs daily, reduces priority of stale apps

#### GAP-67: Application Archive and Pipeline Cleanup
1. Add `is_archived` column to `applications` table
2. Implement auto-archive suggestion job (terminal stage + 30 days)
3. Archive search capability (not hidden completely)
4. Archived apps excluded from active pipeline metrics
5. Export capability before archive

#### GAP-68: Application Contact Relationship Tracking
1. Add `stage_at_contact` to `application_contacts` (which stage was contact involved)
2. Add `contact_quality` enum (Responsive, Ghosted, Mixed) to `profile_contacts`
3. Detect contact overlap across applications
4. Future alert when contact moves to new company
5. Full contact history per application view

## Data Model

### New Tables / Columns

```sql
-- applications table additions
ALTER TABLE applications ADD COLUMN application_group_id INTEGER REFERENCES application_groups(id);
ALTER TABLE applications ADD COLUMN response_deadline DATE;
ALTER TABLE applications ADD COLUMN priority TEXT DEFAULT 'unset'; -- high, medium, low, unset
ALTER TABLE applications ADD COLUMN is_archived BOOLEAN DEFAULT FALSE;

-- application_groups for deduplication
CREATE TABLE application_groups (
  id INTEGER PRIMARY KEY,
  canonical_company TEXT NOT NULL,
  canonical_title TEXT NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- technical_challenges table
CREATE TABLE technical_challenges (
  id INTEGER PRIMARY KEY,
  application_id INTEGER NOT NULL REFERENCES applications(id),
  challenge_url TEXT,
  sent_at TIMESTAMP,
  submitted_at TIMESTAMP,
  deadline TIMESTAMP,
  status TEXT NOT NULL -- sent, submitted, due, expired
);

-- interview_feedback table
CREATE TABLE interview_feedback (
  id INTEGER PRIMARY KEY,
  interview_id INTEGER NOT NULL REFERENCES interviews(id),
  self_assessment INTEGER CHECK(self_assessment BETWEEN 1 AND 5),
  recruiter_feedback TEXT,
  hirer_feedback TEXT,
  sentiment_score REAL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- bulk_operation_history for undo
CREATE TABLE bulk_operation_history (
  id INTEGER PRIMARY KEY,
  operation_type TEXT NOT NULL,
  application_ids JSON NOT NULL,
  previous_state JSON NOT NULL,
  performed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  undone_at TIMESTAMP
);

-- offer comparison extensions
ALTER TABLE offers ADD COLUMN equity_annual_vest_value INTEGER;
ALTER TABLE offers ADD COLUMN benefits_value INTEGER;
ALTER TABLE offers ADD COLUMN verbal_offer_deadline TIMESTAMP;
```

## API Surface

### New Modules
- `lazyjob-core/src/deduplication/` — Job fingerprinting, fuzzy matching, application grouping
- `lazyjob-core/src/offer_comparison/` — Offer comparison service, total comp calculation, scenario modeling
- `lazyjob-core/src/bulk_operations/` — Bulk operation execution, undo system, selective filters
- `lazyjob-core/src/deadline_tracking/` — Deadline management, conflict detection, reminders
- `lazyjob-core/src/rejection_followup/` — Response templates, stay-in-touch scheduling, future opportunity tracking
- `lazyjob-core/src/application_priority/` — Priority scoring, decay, AI suggestions

### New Traits
```rust
pub trait DeduplicationService {
    fn find_duplicates(&self, application: &NewApplication) -> Vec<ApplicationGroup>;
    fn link_application(&self, application_id: i64, group_id: i64) -> Result<()>;
}

pub trait OfferComparisonService {
    fn compare_offers(&self, offer_ids: Vec<i64>) -> ComparisonResult;
    fn model_scenario(&self, offer_id: i64, adjustment: CompAdjustment) -> ScenarioResult;
}

pub trait BulkOperationService {
    fn execute(&self, op: BulkOperation, filters: ApplicationFilters) -> BulkResult;
    fn undo(&self, operation_id: i64) -> Result<()>;
}
```

## Key Technical Decisions

1. **Merge vs. Link for deduplication**: Linked (not merged) — preserves per-source metadata while grouping for UX
2. **Deduplication happens at apply-time**: Jobs remain separate entities; only Applications get grouped
3. **Offer comparison weights are user-configurable**: No hardcoded priorities; stored in user preferences
4. **Bulk undo window is 5 minutes**: After window, operation is final (keeps complexity bounded)
5. **Priority decay is opt-in**: Not automatic; user configures if wanted
6. **Archive doesn't delete**: Historical data preserved; excluded from active metrics only

## File Structure

```
lazyjob/
├── lazyjob-core/
│   └── src/
│       ├── application/
│       │   ├── mod.rs
│       │   ├── deduplication/
│       │   │   ├── mod.rs
│       │   │   ├── fingerprint.rs      # JobFingerprint, normalization
│       │   │   ├── matcher.rs          # Fuzzy matching, Levenshtein
│       │   │   └── group.rs            # Application grouping
│       │   ├── offer_comparison/
│       │   │   ├── mod.rs
│       │   │   ├── comparison.rs       # ComparisonResult, scoring
│       │   │   └── scenario.rs        # NegotiationScenario
│       │   ├── bulk_operations/
│       │   │   ├── mod.rs
│       │   │   ├── executor.rs         # Bulk operation execution
│       │   │   └── undo.rs             # Undo tracking
│       │   ├── deadline_tracking/
│       │   │   ├── mod.rs
│       │   │   └── conflict.rs         # Deadline conflict detection
│       │   ├── rejection_followup/
│       │   │   ├── mod.rs
│       │   │   └── templates.rs        # Rejection response templates
│       │   └── priority/
│       │       ├── mod.rs
│       │       └── decay.rs            # Priority decay logic
│       └── schema/
│           └── migrations/
│               ├── 0035_application_groups.sql
│               ├── 0036_technical_challenges.sql
│               ├── 0037_interview_feedback.sql
│               ├── 0038_bulk_operation_history.sql
│               └── 0039_offer_extensions.sql
├── lazyjob-tui/
│   └── src/
│       └── views/
│           ├── application_detail.rs   # Add comparison tab, feedback form
│           ├── kanban.rs               # Priority sorting, bulk ops
│           └── offer_comparison.rs     # Side-by-side comparison view
```

## Dependencies

- **External crates**: `levenshtein` crate for fuzzy string matching
- **Other specs that must be implemented first**:
  - `04-sqlite-persistence.md` — Schema migrations infrastructure
  - `application-state-machine.md` — ApplicationRepository trait (extended)
  - `application-workflow-actions.md` — ApplyWorkflow (extended with deduplication check)
- **Cross-spec dependencies**:
  - GAP-60 (offer comparison) depends on `12-15-interview-salary-networking-notifications.md` OfferEvaluation
  - GAP-59 (deduplication) depends on `job-search-discovery-engine.md` cross-source strategy

## Testing Strategy

1. **Unit tests for fuzzy matching**: Company name normalization edge cases ("Stripe Inc" vs "Stripe" vs "stripe")
2. **Unit tests for offer comparison**: Total comp calculation correctness
3. **Integration tests for bulk operations**: Undo within window, permanent after window
4. **Edge cases**:
   - Duplicate detection when company name slightly different ("Google" vs "Google LLC")
   - Offer comparison with missing equity data
   - Bulk operation with 0 matching filters (no-op)
   - Archive then restore visibility

## Open Questions

1. **Q1**: For cross-source deduplication, should we store the original job IDs from all sources? Or just the "canonical" source?
2. **Q2**: For offer comparison, should total comp calculation be a simple sum or a present value (PV of future cash flows)?
3. **Q3**: For bulk undo, should we support partial undo (undo 3 of 5 operations)?
4. **Q4**: For priority AI suggestions, what signals should affect priority? (time since applied, match score, company reputation?)

## Effort Estimate

- **Phase 1 (Critical)**: 3-4 weeks
  - GAP-59 (Deduplication): 1.5 weeks — complex fuzzy matching, UI consolidation view
  - GAP-60 (Offer Comparison): 1.5 weeks — scoring model, comparison UI, scenario modeling
- **Phase 2 (Important)**: 2-3 weeks
  - GAP-61 (Rejection Followup): 1 week
  - GAP-62 (Bulk Operations): 0.5 week
  - GAP-63 (Deadline Tracking): 0.5 week
- **Phase 3 (Moderate)**: 2-3 weeks
  - GAP-64: 0.5 week
  - GAP-65: 0.5 week
  - GAP-66: 0.5 week
  - GAP-67: 0.5 week
  - GAP-68: 0.5 week

**Total**: 7-10 weeks for full implementation