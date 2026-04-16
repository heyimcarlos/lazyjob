# LazyJob — Audiences & Jobs To Be Done

Synthesized from 36 research specs in `specs/`. This is the anchor document for all downstream spec and task work.

---

## Audiences

LazyJob has four distinct audiences. Each has unique JTBDs. The product must serve Audience 1 excellently; it should accommodate Audiences 2 and 3; it should anticipate Audience 4 in architecture decisions.

---

### Audience 1: Active Job Seeker (Primary)

**Who:** Someone actively searching for a new role — laid off, voluntarily leaving, or pivoting. Likely a tech professional (software engineer, PM, designer, data scientist). Technically comfortable with terminal tools. May be applying to 10–100+ roles simultaneously. Time-poor, emotionally stressed (72% report negative mental health impacts), and overwhelmed by a broken system.

**Pain context from research:**
- 75% of applications receive zero response (37.5M/month ghosted in the US)
- 27–30% of job listings are ghost jobs (never intended to hire)
- Median time to offer: 68.5 days
- Tailored resumes convert at 5.75% vs. 2.68% for generic — but tailoring takes 15–30 min/application
- Referrals are 7–18x more likely to result in a hire, but only 7% of applicants use this channel
- 40–50% of candidates who negotiate receive a better offer; most don't try

---

#### JTBD A-1: Find relevant job opportunities without wasting time on ghost jobs or mismatched roles

**Activities:**
- configure target companies with Greenhouse/Lever board tokens
- run ralph job-discovery loop in the background
- browse curated, scored job feed
- filter jobs by status, skills match, salary range, remote preference
- dismiss ghost jobs and irrelevant listings
- save and star interesting opportunities
- refresh job data on demand

---

#### JTBD A-2: Apply to jobs efficiently without repetitive manual work

**Activities:**
- trigger resume tailoring for a specific job description
- review AI-generated tailored resume before approving
- trigger cover letter generation for a specific company and role
- review and approve AI-generated cover letter
- submit application through ATS (Greenhouse, Lever) via agent or manually
- store submitted resume and cover letter version against the application record
- fill screening questions using profile data

---

#### JTBD A-3: Track where I stand in every hiring process at a glance

**Activities:**
- view all active applications in a kanban pipeline (Discovered → Offer)
- move an application to the next stage
- log interview details (type, scheduled time, interviewers)
- add notes to an application after each touchpoint
- set and receive follow-up reminders
- view pipeline health metrics (response rate, interview rate, stale applications)
- archive dead-end applications

---

#### JTBD A-4: Get warm introductions that beat cold applications

**Activities:**
- map 1st/2nd-degree connections to target companies
- identify alumni, shared communities, or former colleagues at target firms
- draft personalized outreach messages for specific contacts
- review and approve outreach before sending
- track which contacts have been approached and their response status
- request referrals at the right moment in the relationship

---

#### JTBD A-5: Prepare for interviews systematically

**Activities:**
- generate a question set tailored to a specific role and interview type
- research a company's mission, values, recent news, and team structure
- run a mock interview loop (question → user response → AI feedback)
- review STAR-format behavioral stories against a target company's culture signals
- export a one-page company cheat sheet for interview day

---

#### JTBD A-6: Negotiate the best possible compensation offer

**Activities:**
- look up market compensation for the role, level, and location
- evaluate an offer letter's total comp (base + equity + bonus + signing)
- identify negotiation leverage points (competing offers, market gap, skills scarcity)
- generate a counter-offer email draft
- compare two or more competing offers side by side
- track the negotiation history and outcome

---

### Audience 2: Passive Job Seeker / Power User

**Who:** Currently employed but open to compelling opportunities. Likely a senior technical professional who values keyboard-driven tools (uses lazygit, tmux, neovim). Not urgently searching, but wants ambient market awareness and is happy to configure a sophisticated tool. May also be a power user who runs LazyJob as a background daemon.

---

#### JTBD B-1: Monitor the job market continuously without active manual effort

**Activities:**
- configure ralph loops on a schedule (hourly, daily)
- receive a morning digest of new matched jobs
- save and review interesting roles without urgency
- set threshold for notification (only alert when match score > X)

---

#### JTBD B-2: Understand my skill gaps relative to target roles before they become urgent

**Activities:**
- run gap analysis against a target role family or company tier
- view a heat map of matched vs. missing skills across N job descriptions
- identify skills that appear frequently in target JDs but are absent from my profile
- plan a learning path prioritized by market demand

---

#### JTBD B-3: Access all major job platforms from one tool without context switching

**Activities:**
- configure platform tokens (Greenhouse, Lever, Adzuna) in a single config file
- aggregate and normalize job listings from multiple sources
- deduplicate jobs that appear on multiple boards
- search across all platforms with a single query

---

### Audience 3: Career Changer / Re-entrant

**Who:** Someone transitioning fields (e.g., military → tech, finance → product), returning after a gap (caregiver, health), or entering a new seniority level. Their background doesn't map cleanly to standard JDs. They are systematically underserved by keyword-based ATS systems and existing AI tools.

---

#### JTBD C-1: Frame my non-linear background as a strength for a specific target role

**Activities:**
- identify transferable skills from non-traditional experience
- reframe past job titles and experience bullets for a new domain
- tailor the profile summary to tell a pivot narrative
- generate cover letters that proactively address career change
- flag which gaps are genuine blockers vs. noise

---

#### JTBD C-2: Know which skill gaps are actual blockers before applying

**Activities:**
- run gap analysis against a target role family
- distinguish required skills from nice-to-have
- see which missing skills most frequently cause rejection at screen stage
- get a prioritized upskilling recommendation

---

### Audience 4: Tool Author / SaaS Operator (Future)

**Who:** The team building and operating LazyJob as a product. Not a user of job search features — a builder of the platform. JTBDs here drive architecture decisions that must be baked in now to avoid rewrites later.

---

#### JTBD D-1: Migrate from local-first CLI to a cloud SaaS product without rewriting core logic

**Activities:**
- extract persistence behind a `Repository` trait (SQLite now, PostgreSQL later)
- add optional cloud sync layer (Supabase) without changing business logic
- implement auth (OAuth, email magic links) as a pluggable layer
- add multi-tenancy with row-level security to the data model

---

#### JTBD D-2: Offer premium AI features to users without requiring them to manage API keys

**Activities:**
- implement a server-side LLM proxy (the loom pattern)
- gate LLM provider access behind subscription tier checks
- route requests to the cheapest capable provider at runtime
- track per-user token usage for billing

---

## JTBD Index (quick reference)

| ID  | JTBD Summary | Audience | Domain |
|-----|--------------|----------|--------|
| A-1 | Find relevant jobs without wasting time | Active seeker | job-search |
| A-2 | Apply efficiently without repetitive work | Active seeker | application-tracking |
| A-3 | Track hiring process status at a glance | Active seeker | application-tracking |
| A-4 | Get warm introductions that beat cold apps | Active seeker | networking |
| A-5 | Prepare for interviews systematically | Active seeker | interview-prep |
| A-6 | Negotiate the best possible offer | Active seeker | salary-negotiation |
| B-1 | Monitor job market without active effort | Passive seeker | job-search |
| B-2 | Understand skill gaps before they're urgent | Passive seeker | profile-resume |
| B-3 | Access all platforms from one tool | Power user | platform-integrations |
| C-1 | Frame non-linear background as a strength | Career changer | profile-resume |
| C-2 | Know which skill gaps are actual blockers | Career changer | profile-resume |
| D-1 | Migrate CLI to SaaS without rewrite | Tool author | architecture |
| D-2 | Offer premium AI without key management | Tool author | saas |

---

## Cross-Cutting Concerns (affect all JTBDs)

These are not JTBDs themselves but are constraints that all features must respect:

- **Privacy**: all job search data is sensitive (target companies, salary expectations, career moves). Local-first SQLite, OS keychain for API keys, no telemetry by default. Refs: `16-privacy-security.md`
- **Offline-first**: must work without internet for all read operations. LLM features degrade gracefully when offline. Refs: `01-architecture.md`, `04-sqlite-persistence.md`
- **Human in the loop**: the agent drafts, the human approves, the human acts. Nothing is sent or submitted without explicit user confirmation. Refs: `cover-letters-applications.md`, `networking-referrals-agentic.md`
- **Anti-fabrication**: the system must never invent skills, experiences, or achievements. All AI-generated content must be traceable to real profile data. Refs: `07-resume-tailoring-pipeline.md`, `17-ralph-prompt-templates.md`
- **Ghost job filtering**: 27–30% of listings are not real. The system should detect and deprioritize ghost jobs proactively. Refs: `agentic-job-matching.md`, `job-platforms-comparison.md`

---

## Architecture Notes for Spec Writers

When writing domain specs, assume this crate structure (from `01-architecture.md`):

```
lazyjob-core      — domain models, SQLite persistence traits, discovery logic
lazyjob-llm       — LlmProvider trait + Anthropic/OpenAI/Ollama impls, prompt templates
lazyjob-ralph     — ralph subprocess IPC, process management, state sync
lazyjob-tui       — ratatui app, views, widgets, keymap, theme
lazyjob-cli       — binary entry point
```

Dependency order: `lazyjob-cli → lazyjob-tui → lazyjob-ralph → lazyjob-llm → lazyjob-core`

The loom pattern for LLM integration: server-side proxy, provider-agnostic `LlmProvider` trait, SSE streaming to TUI, token usage tracking.

Ralph subprocess protocol: spawn child process, communicate via newline-delimited JSON over stdio, TUI handles `RalphEvent` (Started, Status, Results, Error, Done), ralph writes results directly to shared SQLite (WAL mode).
