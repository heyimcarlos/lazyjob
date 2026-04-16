# Gap Analysis: Application Workflow (10, application-*, 12-15 specs)

## Specs Reviewed
- `10-application-workflow.md` - Application Workflow (research)
- `application-state-machine.md` - Full state machine spec with 10 stages
- `application-workflow-actions.md` - Workflow actions (Apply, MoveStage, ScheduleInterview, LogContact)
- `application-pipeline-metrics.md` - Pipeline metrics, reminders, morning digest
- `12-15-interview-salary-networking-notifications.md` - Interview/salary/networking/notifications (partial)

---

## What's Well-Covered

### application-state-machine.md
- 10-stage ApplicationStage enum with explicit transition matrix
- Terminal/non-terminal stage separation (Accepted, Rejected, Withdrawn)
- application_transitions history log (immutable append-only)
- Full SQLite schema: applications, application_transitions, application_contacts, interviews, offers
- ApplicationRepository trait with update_stage enforcing transition validation
- Clean separation between profile_contacts (networking) and application_contacts (hiring process)
- Interview and Offer structs with comprehensive fields

### application-workflow-actions.md
- ApplyWorkflow: duplicate check, resume decision, cover letter decision, screening questions, application record creation
- MoveStageWorkflow: validation, pre/post side effects, suggestion system (non-blocking)
- ScheduleInterviewWorkflow: interview detail collection, reminder creation
- LogContactWorkflow: lightweight contact logging, resets stale clock
- Human-in-the-loop boundaries explicitly defined (always automated vs. confirmation vs. never automated)
- Anti-spam architecture (duplicate gate, ghost job gate, daily count metric, tailoring gate)
- WorkflowEvent system with tokio broadcast for TUI updates

### application-pipeline-metrics.md
- PipelineMetrics computation from SQLite (fresh computation, no materialized views)
- Stale detection query (14 days default, 30 days for Discovered/Interested)
- Action Required queue (overdue follow-ups, stale apps, expiring offers, upcoming interviews, new rejections)
- Morning digest (printed once per day, respects privacy_mode)
- Velocity metrics from application_transitions timestamps
- ReminderPoller background task (5-minute interval)
- Funnel chart, stage distribution, stale list, velocity table in TUI metrics view

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-59: Cross-Source Application Deduplication (CRITICAL)

**Location**: `application-workflow-actions.md` - ApplyWorkflow duplicate check only checks same job_id; `10-application-workflow.md` - mentions deduplication but no design

**What's missing**:
1. **Same job, different source IDs**: Job posted on Greenhouse, LinkedIn, and company website - three different job IDs but same job. LazyJob treats as three applications.
2. **Fuzzy company+title matching**: "Senior Software Engineer at Stripe" and "Senior SWE - Payments at Stripe" - same job or different?
3. **Normalized job fingerprint**: What's the algorithm for detecting duplicate jobs across sources? (company name + title + description similarity?)
4. **Merge vs. link**: When duplicate detected, should applications be merged or linked as "same job, different source"?
5. **Source priority for data**: If same job on LinkedIn has salary but Greenhouse doesn't, which data wins?
6. **Application consolidation UI**: How does user see "3 applications for same job, consolidated view"?

**Why critical**: Users apply to the same job via multiple channels (careers page, LinkedIn, Greenhouse). Without deduplication, pipeline metrics are inflated and UX is confusing.

**What could go wrong**:
- User appears to have 3 applications to Stripe when actually it's 1 job
- Pipeline metrics show 30% interview rate when 2 of 3 "interviews" are for same job
- User wastes time tailoring 3 resumes for same job

---

### GAP-60: Multi-Offer Comparison UI (CRITICAL)

**Location**: `application-workflow-actions.md` - Offer received workflow creates Offer record; `application-pipeline-metrics.md` - Open Question #4 mentions offer comparison; `12-15-interview-salary-networking-notifications.md` - OfferEvaluation struct but no comparison view

**What's missing**:
1. **Side-by-side comparison**: When user has 2+ offers, show comparison table (base, equity, bonus, signing, total comp)
2. **Total comp calculation**: Normalize equity (annual vest value), bonus (as dollar amount), benefits into single comparable number
3. **Weighted comparison**: Factors beyond money: remote policy, company growth, role fit, commute, PTO
4. **Offer expiration tracking**: If one offer expires before another, surface this urgency
5. **Recommendation engine**: Based on user's stated priorities, which offer is "better"?
6. **Negotiation scenario modeling**: "If you negotiate 10K more from Company A, here's how it compares to Company B"

**Why critical**: Candidates with multiple offers need to make informed decisions quickly. This is a high-stakes, time-sensitive moment.

**What could go wrong**:
- User can't easily compare offers, misses deadline on one while deciding
- EquityVest calculations wrong, user makes decision on bad data
- No framework for non-monetary factors, user optimizes only salary

---

### GAP-61: Rejection Email Response Automation (IMPORTANT)

**Location**: `application-workflow-actions.md` - MoveStageWorkflow handles Rejected stage but only prompts for reason

**What's missing**:
1. **Rejection response templates**: When rejected, user may want to send a "keep the door open" response
2. **Template personalization**: Can AI personalize rejection responses using company context?
3. **Request feedback automation**: When rejected, should LazyJob suggest/request interview feedback?
4. **Stay-in-touch cadence**: After rejection, when to reach out again? (3 months? 6 months?)
5. **LinkedIn connection post-rejection**: Should LazyJob suggest connecting with the interviewer after rejection?
6. **Future opportunity tracking**: When same company posts new role, alert the user who was previously rejected

**Why important**: Rejection is a wasted opportunity if not followed up. A "keep the door open" response and future tracking can lead to future opportunities.

**What could go wrong**:
- User burns bridge by not responding to rejection at all
- Generic "thank you for the opportunity" email that doesn't differentiate
- User applies again to same company 2 weeks later, rejected for same reason

---

### GAP-62: Bulk Application Operations (IMPORTANT)

**Location**: `application-workflow-actions.md` - Open Question #3 mentions bulk stage transitions but no spec

**What's missing**:
1. **Bulk stage transition**: Move 5 rejected applications to Rejected in one action
2. **Bulk archive**: Archive all applications older than X days in one action
3. **Bulk delete**: Delete applications (with confirmation)
4. **Selective bulk**: Select applications matching filter (stage = Applied, last_contact > 30 days) and apply bulk action
5. **Undo support**: If bulk action was a mistake, can it be undone?
6. **Progress indication**: When bulk action affects 50 apps, show progress bar

**Why important**: After an auto-rejection wave (ATS sends 10 rejections in one day), user needs efficient cleanup. Manual one-by-one is tedious.

**What could go wrong**:
- User accidentally bulk-deletes applications they meant to keep
- Bulk archive removes applications user was still interested in
- No undo, bulk action is permanent and user loses data

---

### GAP-63: Application Response Deadline Tracking (IMPORTANT)

**Location**: `application-workflow-actions.md` - Open Question #1: application deadline tracking not addressed; `application-state-machine.md` - mentions offers.expiry_date but not company response deadlines

**What's missing**:
1. **Company response deadline**: When user applies, when should company respond by? (Some job postings state "responding within 2 weeks")
2. **Deadline reminder**: If company hasn't responded by deadline, suggest follow-up
3. **Offer response deadline**: Beyond offers.expiry_date, what about verbal offer deadlines?
4. **Custom deadline per application**: Can user set a custom deadline for any application?
5. **Deadline conflict detection**: If user has 3 offers expiring the same week, surface this

**Why important**: Missing a deadline means losing an opportunity. Tracking deadlines prevents this.

**What could go wrong**:
- Verbal offer deadline forgotten, company moves to next candidate
- Company says "we'll respond within 2 weeks" but LazyJob doesn't track this
- User accepts offer without knowing another company's response is imminent

---

### GAP-64: Async Technical Challenge Sub-State (MODERATE)

**Location**: `application-state-machine.md` - Open Question #1: async challenge tracking, no resolution

**What's missing**:
1. **Challenge sent state**: Technical stage - has challenge been sent to candidate?
2. **Challenge deadline**: When is the coding challenge due?
3. **Challenge submitted state**: Has candidate submitted?
4. **Challenge link storage**: Where is the challenge URL/instructions stored?
5. **Auto-advance on submission**: Does the system auto-detect when challenge is submitted (via email parsing)?
6. **Challenge reminder**: Reminder N days before deadline if not submitted

**Why important**: Async coding challenges (HackerRank, Karat, etc.) are common first-round technical screens. They have explicit deadlines that need tracking.

**What could go wrong**:
- User forgets challenge deadline, misses opportunity
- Challenge link stored in email, not in LazyJob, user can't find it
- Applied → Technical but no visibility into whether challenge was sent

---

### GAP-65: Interview Feedback Recording (MODERATE)

**Location**: `application-workflow-actions.md` - ScheduleInterviewWorkflow creates Interview record but no post-interview feedback

**What's missing**:
1. **Post-interview feedback form**: After interview, prompt user to record feedback (how did it go?)
2. **Interviewer feedback tracking**: If recruiter shares interviewer feedback (verbatim or summary), where is it stored?
3. **Self-assessment scoring**: Can user rate their own performance (1-5) per interview?
4. **Hiring manager feedback storage**: Where does feedback from the hiring manager go?
5. **Feedback sentiment analysis**: Can we detect patterns in feedback over time?
6. **Interview → outcome correlation**: Which interview performance correlated with offer/rejection?

**Why important**: Interview feedback (own and recruiter's) informs future prep. Without recording, patterns are invisible.

**What could go wrong**- User forgets what they were asked 2 weeks later
- Recruiter says "great interview" but candidate doesn't record it, can't reference it
- Hiring manager feedback lost in email thread

---

### GAP-66: Application Priority/Ranking System (MODERATE)

**Location**: None of the specs address prioritization of applications beyond stage

**What's missing**:
1. **Priority score per application**: User-defined or AI-computed priority (1-5 stars? High/Med/Low?)
2. **Sort by priority**: Kanban view sorted by priority within stage
3. **Priority factors**: What contributes to priority? (match score, company interest, timeline, salary)
4. **AI priority suggestions**: Can AI suggest "you should prioritize this application" based on signals?
5. **Priority decay**: Applications not touched in X days automatically decrease in priority

**Why important**: With 10-50+ applications, not all are equally important. Priority helps user focus effort.

**What could go wrong**- User spends time on low-priority applications while high-priority ones stagnate
- No framework for deciding "which application should I focus on today"
- All applications treated equally, user overwhelmed

---

### GAP-67: Application Archive and Pipeline Cleanup (MODERATE)

**Location**: `application-pipeline-metrics.md` - mentions "archive" but only as optional action after offer accepted; no systematic cleanup

**What's missing**:
1. **Archive criteria**: When should applications be archived? (Terminal stage + X days? User manually?)
2. **Archive vs. delete**: Archive hides from active view, delete removes entirely. When each?
3. **Archive visibility**: Can archived applications be searched/seen? Or are they hidden by default?
4. **Auto-archive suggestions**: "Application to Figma has been rejected for 30 days. Archive?"
5. **Archived metrics**: Do archived applications count in pipeline metrics? (Probably not for active metrics)
6. **Export before archive**: Before archiving, can user export application data?

**Why important**: Active application list grows over time. Without cleanup, it's unwieldy.

**What could go wrong**- User has 200 applications, 180 are old rejections, can't find anything
- Archived applications still appearing in metrics, skewing data
- Important historical data lost if applications are deleted rather than archived

---

### GAP-68: Application Contact Relationship Tracking (MODERATE)

**Location**: `application-state-machine.md` - application_contacts table but relationship management not specced

**What's missing**:
1. **Contact relationship to stage**: Which contact was involved at which stage? (Recruiter for phone screen, HM for on-site)
2. **Contact history per application**: Full history of all contacts for this application
3. **Contact quality tracking**: Which contacts have been responsive? Which ghosted?
4. **Contact overlap detection**: Same recruiter across multiple applications at same company
5. **Contact future alerts**: When new role opens at company where candidate has contact, alert user

**Why important**: Recruiters change companies. Knowing who you know at target companies is key networking intelligence.

**What could go wrong**- User talks to Sarah (recruiter) at Company A, Sarah moves to Company B, user doesn't know
- Same recruiter at 3 different roles at same company, user reaches out to all 3 separately
- Contact information lost when application goes stale

---

## Cross-Spec Gaps

### Cross-Spec N: Application ↔ Job Discovery De-duplication

The deduplication problem spans:
- `job-search-discovery-engine.md`: cross-source deduplication strategy for jobs
- `application-workflow-actions.md`: duplicate check for applications

No unified spec for: if same job appears across sources, should there be one Job record and multiple Applications (one per source), or one Job and one Application?

**Affected specs**: All job discovery and application specs

### Cross-Spec O: Application Metrics ↔ Salary Service

`application-pipeline-metrics.md` Open Question #4 mentions offer comparison may overlap with `salary-market-intelligence.md`. There's no shared `Offer` type or comparison utility.

**Affected specs**: `application-pipeline-metrics.md`, `12-15-interview-salary-networking-notifications.md`

---

## Specs to Create

### Critical Priority

1. **XX-application-cross-source-deduplication.md** - Fuzzy job matching across sources, merge vs. link strategy, data priority resolution
2. **XX-multi-offer-comparison.md** - Side-by-side comparison, total comp calculation, weighted comparison, recommendation engine

### Important Priority

3. **XX-rejection-email-automation.md** - Response templates, request feedback, stay-in-touch cadence, future opportunity tracking
4. **XX-bulk-application-operations.md** - Bulk stage transitions, selective bulk, undo support, progress indication
5. **XX-application-deadline-tracking.md** - Response deadlines, custom deadlines, conflict detection

### Moderate Priority

6. **XX-async-challenge-tracking.md** - Challenge sent/submitted/deadline sub-states, auto-advance, reminders
7. **XX-interview-feedback-recording.md** - Post-interview form, recruiter feedback storage, self-assessment, outcome correlation
8. **XX-application-priority-ranking.md** - Priority score, AI suggestions, decay, focus recommendations
9. **XX-application-archive-cleanup.md** - Archive criteria, auto-archive suggestions, archived visibility
10. **XX-application-contact-relationship-tracking.md** - Contact-stage mapping, quality tracking, overlap detection, future alerts

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-59: Cross-Source Deduplication | Critical | High | Metrics accuracy |
| GAP-60: Multi-Offer Comparison | Critical | Medium | High-stakes decision |
| GAP-61: Rejection Email Automation | Important | Medium | Relationship maintenance |
| GAP-62: Bulk Operations | Important | Low | UX efficiency |
| GAP-63: Deadline Tracking | Important | Low | Opportunity protection |
| GAP-64: Async Challenge Sub-State | Moderate | Medium | Technical screen tracking |
| GAP-65: Interview Feedback Recording | Moderate | Low | Learning/improvement |
| GAP-66: Priority Ranking | Moderate | Low | Focus optimization |
| GAP-67: Archive/Cleanup | Moderate | Low | Pipeline manageability |
| GAP-68: Contact Relationship Tracking | Moderate | Low | Networking intelligence |
