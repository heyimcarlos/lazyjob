# Gap Analysis: Cover Letter / Interview (08, profile-*, interview-prep-*, 12-15 specs)

## Specs Reviewed
- `08-cover-letter-generation.md` - Research on cover letter generation
- `cover-letters-applications.md` - Research on CL effectiveness and tools landscape
- `profile-cover-letter-generation.md` - Full CL generation pipeline spec
- `interview-prep-agentic.md` - Research on interview prep landscape and agentic opportunities
- `interview-prep-question-generation.md` - Question generation pipeline spec
- `interview-prep-mock-loop.md` - Mock interview loop spec with evaluation rubrics
- `12-15-interview-salary-networking-notifications.md` - Interview prep, salary, networking, notifications

---

## What's Well-Covered

### profile-cover-letter-generation.md
- 6-stage pipeline design (company research → angle selection → outline → draft → review → version)
- FabricationLevel enum (Safe, Acceptable, Risky, Forbidden) shared with resume tailoring
- Voice preservation via few-shot examples from LifeSheet
- CoverLetterVersion tracking linked to applications
- Anti-fabrication constraint system (Level 1-2-3)
- Tone calibration per company culture

### interview-prep-question-generation.md
- PrepContext computed pre-LLM (verified facts, not invented) - excellent anti-fabrication pattern
- Question mix ratios by InterviewType (PhoneScreen, TechnicalScreen, Behavioral, OnSite, SystemDesign)
- STAR story mapping to candidate_story_ref
- CompanyRecord.interview_signals as grounding (offline, no invented data)
- Gap-aware question generation based on candidate skill gaps
- Clean separation: lazyjob-core domain, no TUI dependency

### interview-prep-mock-loop.md
- Comprehensive evaluation rubrics: STAR (0-10), Technical (0-10), Culture/Situational (0-10)
- Per-category scoring with sub-components (situation, task, action, result, accuracy, depth, communication)
- Behavioral fabrication detection against candidate_story_ref
- SessionScore aggregation with per-category averages
- Progress trend tracking (score history across sessions)
- Anti-overconfidence disclaimer copy (mandatory in UI)

### interview-prep-agentic.md
- Comprehensive landscape analysis (Pramp, Interviewing.io, LeetCode, Exponent, etc.)
- Company Research Agent opportunity identified (2-4 hours manual → minutes)
- Cost estimates per agent operation
- Failure modes documented (feedback hallucinations, overconfidence, stale data, homogenization)
- Integration strategy with existing tools (LeetCode API, Exponent content)

### 12-15-interview-salary-networking-notifications.md
- InterviewPrepService with question generation + company insights + talking points
- SalaryService with multi-source aggregation (Levels.fyi, Glassdoor, Blind)
- OfferEvaluation with gap analysis and negotiation leverage
- NetworkingService with contact finding and outreach templates
- NotificationService with MorningBrief generator
- Full Rust struct definitions for all services

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-49: Cover Letter Version Tracking and Sent-State Management (CRITICAL)

**Location**: `profile-cover-letter-generation.md` - CoverLetterVersion defined but management system missing

**What's missing**:
1. **Sent state tracking**: Is there a "sent" state for cover letters? When was it sent? Via what channel?
2. **Version comparison**: Can user diff two cover letter versions? What changed from v1 → v2?
3. **Version naming**: Can users name versions (e.g., "Engineering Manager v1", "Google - Technical Focus")?
4. **Submission channel tracking**: Email vs LinkedIn vs direct portal - different versions for different channels?
5. **Sent-resume linkage**: If user sends cover letter, which ResumeVersion was sent alongside it?
6. **Confirmation tracking**: Did the recipient actually receive the cover letter? (Email read receipt?)
7. **Version cleanup**: How many versions to keep? Auto-prune after successful hire?

**Why critical**: Cover letters are often revised multiple times. Without tracking, users lose visibility into what was sent where.

**What could go wrong**:
- User has 12 cover letter versions, no idea which was sent to which company
- User sends wrong version (e.g., "I am excited about Google" in a letter for Meta)
- Sent cover letter not tracked, can't reference it in follow-up emails

---

### GAP-50: Interview Prep Session Resumability (CRITICAL)

**Location**: `interview-prep-mock-loop.md` - Open Question #1: partial sessions, no resolution

**What's missing**:
1. **Partial session saving**: If user quits mid-session (SIGTERM, :q, crash), session is currently lost - should it be saved as incomplete?
2. **Session resumption**: Can user resume a partial session? Requires re-sending Q&A history to LLM for context (token cost)
3. **Resume UX**: How does user know a partial session exists? How do they pick up?
4. **Partial session timeout**: If partial session is >24 hours old, should it be auto-archived?
5. **Resumption cost transparency**: Should user be told "resuming will cost ~X tokens" before proceeding?

**Why critical**: Users will inevitably quit mid-session. Without resumability, all progress is lost and prep sessions become high-stakes (must complete or lose all work).

**What could go wrong**:
- User practices for 30 minutes, accidentally closes TUI, entire session lost
- No way to do "one question at a time" across multiple sittings
- User feels pressured to complete session in one go, increasing anxiety

---

### GAP-51: Real-Time Company Interview Question Aggregation (IMPORTANT)

**Location**: `interview-prep-question-generation.md` - relies on CompanyRecord.interview_signals; `interview-prep-agentic.md` - Company Research Agent identified but no spec

**What's missing**:
1. **Fresh question sourcing**: CompanyRecord.interview_signals may be stale (Glassdoor data notoriously old). How to get real-time questions?
2. **Glassdoor/Blind scraping spec**: How should LazyJob scrape interview questions from these sites? What's the technical approach?
3. **ToS risk handling**: Blind explicitly prohibits scraping. What's the mitigation strategy?
4. **Question verification**: How to distinguish real recent questions from old/ fabricated ones?
5. **LeetCode Discuss integration**: Real interview experiences from candidates - how to aggregate?
6. **Question freshness scoring**: Should questions have a "last reported" date? How old is too old?

**Why important**: Interview prep is only as good as the questions available. Stale or fake questions waste prep time.

**What could go wrong**:
- User prepped for questions that haven't been asked in 2 years
- Scraping gets LazyJob IP banned from Glassdoor/Blind
- Company interview process changes but LazyJob still using old format data

---

### GAP-52: Interview Fatigue Management (IMPORTANT)

**Location**: None of the interview prep specs address session frequency or fatigue

**What's missing**:
1. **Session frequency recommendations**: How many mock interviews per week is optimal? Is there diminishing returns?
2. **Session duration limits**: MockInterviewLoop has no maximum session length. Should there be a suggested limit?
3. **Break reminders**: Between questions or sections, should the TUI suggest breaks?
4. **Quality vs quantity tracking**: Track whether sessions are becoming rote (high quantity, low improvement)
5. **Mental health signals**: If user is grinding interviews daily, should LazyJob suggest rest?
6. **Readiness scoring**: Based on recent session scores, is the candidate ready for real interview?

**Why important**: Interview prep is stressful. Over-preparation can backfire (burnout, anxiety). No spec addresses this human factor.

**What could go wrong**:
- User grinds 10 mock interviews in one day, performance degrades
- User develops anxiety from constant interview pressure
- No rest periods, real interview arrives but candidate is exhausted

---

### GAP-53: Async Video Interview Preparation (IMPORTANT)

**Location**: `interview-prep-agentic.md` - mentions video analysis (Refactored AI) but no spec

**What's missing**:
1. **Recorded response storage**: If user records video answers, where are they stored? (Local encrypted? Cloud?)
2. **Video evaluation rubric**: Refactored AI does emotion/facial detection. Should LazyJob do this? What's the privacy implication?
3. **Text-based async alternative**: Since video is complex, what about typed async responses that get evaluated?
4. **Platform-specific prep**: Async video interviews (HireVue, Pillar) have specific formats - should LazyJob prep for these?
5. **Screen recording handling**: Some async interviews record screen. How to handle this?
6. **Feedback on non-verbal cues**: Can AI evaluate tone, pace, confidence from text alone?

**Why important**: Many companies use async video interviews (HireVue, etc.) as first-round screens. Prep for these is different from live interviews.

**What could go wrong**:
- User doesn't know async video format, fumbles the recording
- Video stored insecurely, privacy violation
- AI evaluation of video creates false confidence

---

### GAP-54: Whiteboard System Design Evaluation (IMPORTANT)

**Location**: `interview-prep-mock-loop.md` - Open Question #3 acknowledges system design text-evaluation is underspecified

**What's missing**:
1. **System design rubric for text**: User types their architecture design narrative. What evaluation criteria?
2. **Structured templates**: Should system design answers follow a template (define requirements → estimate scale → name components)?
3. **Diagram alternatives**: Can user submit ASCII diagrams? Images? How are they evaluated?
4. **Depth scaling by seniority**: "Staff engineer" vs "Senior" - different scope expectations - how to adjust?
5. **Follow-up question simulation**: Real system design interviews have probing follow-ups. Can mock simulate this?
6. **Time allocation**: System design questions need more time. No spec for timed sections within mock loop.

**Why important**: System design is a major interview component for senior roles. Text-based evaluation is the hardest to get right.

**What could go wrong**:
- User gets high score on typed system design but freezes when actually drawing diagrams
- Scope inappropriate for seniority level (too simple for Staff, too complex for Senior)
- No follow-up probing, leaving gaps undiscovered

---

### GAP-55: Cover Letter Anti-Ghosting Detection (MODERATE)

**Location**: `profile-cover-letter-generation.md` - fabrications guarded but uniformity not addressed

**What's missing**:
1. **Genericness detection**: Is this cover letter starting to sound like every AI-generated cover letter?
2. **Uniqueness scoring**: Compare against a corpus of successful cover letters - is this distinctive?
3. **Overused phrase detection**: Are certain phrases (e.g., "I am excited about") appearing too frequently?
4. **Personalization verification**: Does the cover letter reference specific company facts (from CompanyRecord)?
5. **Human-like variance**: Add randomness/warmth to avoid AI-form-letter feel

**Why important**: Recruiters see hundreds of cover letters. Generic AI-sounding letters are immediately spotted and discounted.

**What could go wrong**:
- Cover letter sounds exactly like every other LazyJob user's cover letter
- All cover letters use the same "winning" phrases, killing authenticity
- No way to know if personalization is actually working

---

### GAP-56: Interview Feedback Aggregation and Pattern Detection (MODERATE)

**Location**: `interview-prep-mock-loop.md` - session scores stored individually, no cross-session analysis

**What's missing**:
1. **Feedback pattern detection**: Which question types consistently get low scores across sessions?
2. **Improvement trajectory**: Is candidate actually improving over time? What's the trend?
3. **Weakest link identification**: After N sessions, what's the single biggest area for improvement?
4. **Spaced repetition suggestions**: Based on weak areas, which topics to revisit?
5. **Comparative benchmarking**: How does candidate compare to others at similar levels?
6. **Real interview correlation**: Do mock interview scores correlate with real interview outcomes?

**Why important**: Individual session feedback is useful. Pattern analysis across sessions is more valuable for focused improvement.

**What could go wrong**- User gets feedback but doesn't know what to focus on next
- Improvement not visible, user loses motivation
- No way to measure ROI of interview prep investment

---

### GAP-57: Salary Data Freshness and Staleness (MODERATE)

**Location**: `12-15-interview-salary-networking-notifications.md` - SalaryData defined but freshness not addressed

**What's missing**:
1. **Data timestamp**: When was this salary data collected? Is there a "as_of" date?
2. **Staleness thresholds**: For what purposes is data considered "fresh enough"? (Negotiation vs research vs exploration)
3. **Source attribution**: Which source provided which data point? (Levels.fyi vs Blind vs Glassdoor)
4. **Update scheduling**: When should salary data be re-fetched? On demand? Scheduled?
5. **Confidence scoring**: Some roles have rich data (Google SWE), others have sparse (startup CFO). How to represent this?
6. **Geographic granularity**: Tech salaries vary wildly by city. Is location-level data available?

**Why important**: Using stale or low-confidence salary data for negotiation could lead to poor outcomes or embarrassment.

**What could go wrong**- Candidate negotiates based on outdated data, undersells or overshoots
- Sparse data presented with same confidence as rich data
- User doesn't know how recent the data is

---

### GAP-58: Networking Outreach Warm Personalization at Scale (MODERATE)

**Location**: `12-15-interview-salary-networking-notifications.md` - OutreachTemplate defined but personalization system underspecified

**What's missing**:
1. **Personalization depth levels**: What level of personalization is possible at scale? (Hi {name} vs shared connection mentioned vs recent work discussed)
2. **Contact research integration**: Can outreach personalization draw from CompanyRecord and candidate's LinkedIn?
3. **Mutual connection highlighting**: When mutual connection exists, how prominently to feature it?
4. **Value proposition customization**: How to tailor the "offering value" aspect per contact?
5. **Channel-specific templates**: LinkedIn InMail vs email vs warm intro - different formats and lengths
6. **Personalization quality scoring**: Can we estimate how personalized a message is before sending?
7. **Scale limits**: At what volume does personalization quality degrade? Should there be limits?

**Why important**: The differentiation between cold outreach and warm outreach is personalization depth. At scale, depth drops.

**What could go wrong**- "Personalized" messages all sound the same ("I noticed you work at X")
- At 50 outreach messages, quality degrades, response rates plummet
- User wastes time on personalization that doesn't move the needle

---

## Cross-Spec Gaps

### Cross-Spec K: Fabrication Detection Shared Module

Fabrication detection is defined separately in:
- `profile-cover-letter-generation.md`: CoverLetter FabricationLevel
- `profile-resume-tailoring.md`: Resume FabricationLevel
- `interview-prep-mock-loop.md`: fabrication_warning on QuestionFeedback

These should be a shared `fabrication.rs` module with a unified `FabricationLevel` enum and detection logic, not duplicated per feature.

**Affected specs**: All generation specs (resume, cover letter, future content generation)

### Cross-Spec L: CompanyRecord Dependency Explosion

`interview-prep-question-generation.md` depends on `CompanyRecord.interview_signals`
- If company research hasn't run, interview_signals is empty
- If signals are stale, questions are low-quality
- Company research pipeline (job-search-company-research.md) must complete before interview prep can work well

**Affected specs**: `interview-prep-question-generation.md`, `job-search-company-research.md`, `12-15-interview-salary-networking-notifications.md` (salary data sourcing)

### Cross-Spec M: Session State ↔ Application State

Mock interview sessions link to applications (`mock_interview_sessions.application_id`) but there's no spec for:
- What happens to sessions when an application is rejected/withdrawn?
- Should sessions survive application state changes?
- How to handle multiple sessions for the same application over time?

**Affected specs**: `interview-prep-mock-loop.md`, `application-state-machine.md` (not yet deeply reviewed)

---

## Specs to Create

### Critical Priority

1. **XX-cover-letter-version-management.md** - Version tracking, sent state, comparison, naming, submission channel tracking
2. **XX-interview-session-resumability.md** - Partial session saving, session resumption UX, timeout handling

### Important Priority

3. **XX-company-interview-question-aggregation.md** - Real-time question sourcing from Glassdoor/Blind/LeetCode, freshness tracking, ToS risk handling
4. **XX-interview-fatigue-management.md** - Session frequency recommendations, break reminders, readiness scoring
5. **XX-async-video-interview-prep.md** - Recorded response storage, evaluation rubric, platform-specific prep
6. **XX-whiteboard-system-design-evaluation.md** - Text-based rubric, structured templates, seniority scaling

### Moderate Priority

7. **XX-cover-letter-anti-ghosting.md** - Genericness detection, uniqueness scoring, overused phrase detection
8. **XX-interview-feedback-aggregation.md** - Cross-session patterns, improvement trajectory, weakest link identification
9. **XX-salary-data-freshness.md** - Data timestamps, staleness thresholds, confidence scoring
10. **XX-networking-warm-personalization-scale.md** - Personalization depth levels, quality scoring, scale limits

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-49: Cover Letter Version Tracking | Critical | Medium | UX/efficiency |
| GAP-50: Interview Session Resumability | Critical | Medium | User experience |
| GAP-51: Real-Time Question Aggregation | Important | High | Prep quality |
| GAP-52: Interview Fatigue Management | Important | Low | User wellbeing |
| GAP-53: Async Video Interview Prep | Important | High | Platform coverage |
| GAP-54: System Design Evaluation | Important | Medium | Senior role prep |
| GAP-55: Cover Letter Anti-Ghosting | Moderate | Medium | Authenticity |
| GAP-56: Feedback Aggregation | Moderate | Medium | Improvement focus |
| GAP-57: Salary Data Freshness | Moderate | Low | Negotiation accuracy |
| GAP-58: Warm Personalization at Scale | Moderate | Medium | Outreach effectiveness |

---

## Spec Files for Gap Analysis Task 5

**Total gaps identified**: 10 (GAP-49 through GAP-58)

**Specs to create for critical gaps**:
- `specs/XX-cover-letter-version-management.md`
- `specs/XX-interview-session-resumability.md`

**Specs to create for important gaps**:
- `specs/XX-company-interview-question-aggregation.md`
- `specs/XX-interview-fatigue-management.md`
- `specs/XX-async-video-interview-prep.md`
- `specs/XX-whiteboard-system-design-evaluation.md`

**Specs to create for moderate gaps**:
- `specs/XX-cover-letter-anti-ghosting.md`
- `specs/XX-interview-feedback-aggregation.md`
- `specs/XX-salary-data-freshness.md`
- `specs/XX-networking-warm-personalization-scale.md`
