# LazyJob — Spec Inventory & Consolidation Plan

Synthesized from 38 source specs in `specs/`. This document anchors tasks 3–11 (spec expansion by domain).

---

## 1. Source Spec → JTBD Domain Mapping

### Tier A: Core Architecture Specs (LazyJob-specific technical)

| Source File | Primary Domain | JTBDs Served | Notes |
|---|---|---|---|
| `01-architecture.md` | architecture | D-1 (SaaS migration shapes architecture) | Master spec: crate layout, domain models, TUI hierarchy. Sketches overlap with 02, 03, 04 |
| `02-llm-provider-abstraction.md` | agentic | D-2 (LLM proxy) | Defines `LlmProvider` trait + AnthropicProvider/OpenAIProvider/OllamaProvider. SSE streaming |
| `03-life-sheet-data-model.md` | profile-resume | C-1 (career framing), A-2 (apply) | LifeSheet YAML schema + SQLite tables. Contact table conflicts with spec 04 contacts |
| `04-sqlite-persistence.md` | architecture | D-1 (portability), A-3 (tracking) | Full DDL: jobs/applications/contacts/interviews/offers. Repository trait. WAL mode |
| `05-job-discovery-layer.md` | job-search | A-1 (find jobs), B-1 (monitor) | JobSource trait, DiscoveryService, semantic embedding match, CompanyRegistry |
| `06-ralph-loop-integration.md` | agentic | All A JTBDs | IPC protocol (stdio JSON), RalphProcessManager, crash recovery, loop types enum |
| `07-resume-tailoring-pipeline.md` | profile-resume | A-2 (apply), C-1 (framing) | 6-step LLM pipeline, docx-rs, fabrication guardrails (FabricationLevel enum), versioning |
| `08-cover-letter-generation.md` | profile-resume | A-2 (apply), C-1 (framing) | CompanyResearcher, tone/length variants, 3 templates including career-changer |
| `09-tui-design-keybindings.md` | architecture | All user JTBDs | Complete TUI blueprint: view hierarchy, keybindings, wireframes, widgets, themes |
| `10-application-workflow.md` | application-tracking | A-2 (apply), A-3 (track) | 10-stage state machine, transition rules, ApplyWorkflow, PipelineMetrics |
| `11-platform-api-integrations.md` | platform-integrations | B-3 (multi-platform), A-1 | PlatformClient trait, Greenhouse/Lever impls, RateLimiter, normalization mapper |
| `12-15-interview-salary-networking-notifications.md` | **SPLIT REQUIRED** | A-4, A-5, A-6, B-1 | **Violates scope test** — covers 4 distinct concerns. See Section 3 |
| `16-privacy-security.md` | architecture | All (cross-cutting) | CredentialManager (keyring-rs), FileEncryption (age crate), PrivacyMode enum |
| `17-ralph-prompt-templates.md` | agentic | All agentic JTBDs | All 7 loop type prompts + JSON schemas, anti-fabrication, injection defense |
| `18-saas-migration-path.md` | saas | D-1 (SaaS migration) | 3-phase roadmap, Repository trait, AuthProvider, SyncOperation, multi-tenancy |
| `20-openapi-mvp.md` | architecture | All | Master MVP definition, 12-week phases, Cargo dependencies, success milestones |

### Tier B: Market Research Specs (LinkedIn/platform analysis → inform LazyJob design)

| Source File | Primary Domain | Key Insight for LazyJob | Dedicate New Spec? |
|---|---|---|---|
| `agentic-job-matching.md` | agentic/job-search | Ghost job detection heuristics, ESCO/O*NET, CareerBERT | Extract into `job-search-ghost-job-detection.md` + `job-search-semantic-matching.md` |
| `agent-interfaces-job-platforms.md` | platform-integrations | Four-tier data access strategy, auto-apply product landscape, ToS/legal | Extract into `platform-closed-platforms.md` |
| `company-pages.md` | platform-integrations | Company entity model (employee count unreliable, verified badge) | Inform `job-search-company-research.md` |
| `cover-letters-applications.md` | application-tracking | Human-in-the-loop principles, quality-over-volume positioning | Inform `application-workflow-actions.md` |
| `feed-algorithm.md` | platform-integrations | LLM-based dual-encoder retrieval pattern (relevant to job scoring) | Background research only |
| `gap-analysis-and-critique.md` | meta | Ghost jobs, career transitions, offer negotiation — all were missing specs | Spawned tasks in this program |
| `interview-prep-agentic.md` | interview-prep | AI mock interview, STAR coaching, LLM cost estimates, ToS risks | Primary source for `interview-prep-mock-loop.md` |
| `job-platforms-comparison.md` | platform-integrations | Ghost job regulation, cross-platform behavior (Indeed/Glassdoor/Wellfound) | Inform platform integration specs |
| `job-search-marketplace.md` | job-search | JYMBII two-tower model, ghost job epidemic data, Easy Apply crisis stats | Background research — statistics |
| `job-search-workflow-today.md` | job-search | End-to-end workflow research, response rate data, referral channel stats | Background research — statistics |
| `messaging-inmail.md` | networking | Access-tier hierarchy, credit-back model, async delivery semantics | Inform `networking-outreach-drafting.md` |
| `network-graph.md` | networking | Degree-gated access, triangle-closing for discovery, warm-path finding | Inform `networking-connection-mapping.md` |
| `networking-referrals-agentic.md` | networking | Referral paradox data, professional CRM landscape, agentic use cases | Primary source for all 3 networking specs |
| `premium-monetization.md` | saas | LinkedIn pricing benchmarks, manufactured-scarcity model, agent pricing alternatives | Inform `saas-pricing-strategy.md` |
| `profile-system.md` | profile-resume | Component-based profile architecture, search ranking signals | Inform `profile-life-sheet-data-model.md` |
| `projects-portfolio.md` | profile-resume | Projects section API, portfolio capabilities | Inform `profile-life-sheet-data-model.md` |
| `recruiter-workflow.md` | application-tracking | Recruiter funnel conversion rates, ATS market share, anti-spam positioning | Inform `application-workflow-actions.md` |
| `resume-optimization.md` | profile-resume | ATS parsing mechanics, tailoring conversion data (55-115% uplift), semantic matcher | Primary source for `profile-resume-tailoring.md` |
| `salary-negotiation-offers.md` | salary-negotiation | Total comp math, equity types, counter-offer strategy, privacy constraints | Primary source for all 2 salary specs |
| `search-and-discovery.md` | job-search | Galene search architecture, embedding-based retrieval patterns | Inform `job-search-semantic-matching.md` |
| `skills-endorsements.md` | profile-resume | ESCO skills taxonomy (39K nodes), skill extraction ML pipeline | Inform `profile-skills-gap-analysis.md` |
| `x-professional-features.md` | platform-integrations | X's inbound discovery model vs. outbound application model | Background research only |
| `19-competitor-analysis.md` | saas | Huntr/Teal/Notion competitive positioning | Inform `saas-pricing-strategy.md` |

---

## 2. Redundancy & Overlap Analysis

### High-Priority Merges Required

**R-1: `profile-system.md` + `projects-portfolio.md`**
- Both are LinkedIn profile reverse-engineering. `projects-portfolio.md` is a subset of `profile-system.md`.
- Resolution: Both inform `profile-life-sheet-data-model.md`. Neither produces a standalone spec — they are research artifacts.

**R-2: `agentic-job-matching.md` + `agent-interfaces-job-platforms.md`**
- 60% topical overlap: both cover Greenhouse/Lever APIs, JobSpy, legal landscape, Merge.dev, data source tiers.
- Key difference: `agentic-job-matching.md` focuses on the matching problem; `agent-interfaces-job-platforms.md` focuses on access methods and auto-apply products.
- Resolution: Each informs different output specs. No merge needed — they are research artifacts feeding different domains.

**R-3: `job-platforms-comparison.md` + `job-search-marketplace.md` + `search-and-discovery.md`**
- All three deeply analyze LinkedIn's infrastructure and competitive landscape.
- All include "agent platform relevance" sections with overlapping conclusions.
- Resolution: These are all Tier B research; extract statistics and design patterns into output specs. Do not write a consolidated output spec for these — the overlap is in the research source, not the design.

**R-4: `messaging-inmail.md` + `network-graph.md` + `networking-referrals-agentic.md`**
- InMail credit system and access hierarchy documented in both `messaging-inmail.md` and `network-graph.md`.
- "Agent platform" sections are structurally parallel across all three.
- Resolution: `networking-referrals-agentic.md` is the primary LazyJob-specific source. The other two are LinkedIn infrastructure research.

**R-5: Contacts table — `03-life-sheet-data-model.md` vs `04-sqlite-persistence.md`**
- `spec 03`: defines `contact` table in Life Sheet (relationship-based network contacts).
- `spec 04`: defines `contacts` table in application tracking (recruiters, hiring managers, interviewers).
- Resolution: These are **two distinct tables serving two distinct purposes**. Both must exist. The `profile-contacts` table serves JTBD A-4 (networking); the `application-contacts` table serves A-3 (tracking). Spec writers must name them distinctly.

### Medium-Priority: Single Source Covering Multiple Concerns

**R-6: `12-15-interview-salary-networking-notifications.md` — MUST SPLIT**
- This file covers 4 distinct product features that each fail the scope test: interview prep, salary negotiation, networking service, and morning notifications.
- Each will become a dedicated output spec. The source file is research input only.

**R-7: `06-ralph-loop-integration.md` — two concerns**
- Covers both the IPC protocol (how TUI talks to Ralph) AND the orchestration logic (what loops exist, their types, scheduling).
- Resolution: Split into `agentic-ralph-subprocess-protocol.md` (IPC/process management) and `agentic-ralph-orchestration.md` (loop types, scheduling, cancellation).

---

## 3. Topics Without Dedicated Specs (Gaps)

These topics appear in research but have no dedicated LazyJob-specific spec:

| Gap | Evidence | Proposed Spec |
|---|---|---|
| Ghost job detection | `gap-analysis-and-critique.md`, `agentic-job-matching.md`, `job-platforms-comparison.md` | `job-search-ghost-job-detection.md` |
| Career transition/pivot support | `gap-analysis-and-critique.md`, `networking-referrals-agentic.md` | Add section to `profile-skills-gap-analysis.md` |
| Semantic job-to-profile matching | `05-job-discovery-layer.md`, `agentic-job-matching.md`, `resume-optimization.md` | `job-search-semantic-matching.md` |
| Company research for apply/interview | `08-cover-letter-generation.md` (CompanyResearcher), `interview-prep-agentic.md` | `job-search-company-research.md` |
| Morning digest / notification system | `12-15-interview-salary-networking-notifications.md` | `application-pipeline-metrics.md` (include digest) |
| Configuration management (API keys, platform tokens, preferences) | `16-privacy-security.md`, `11-platform-api-integrations.md`, `18-saas-migration-path.md` | `architecture-config-management.md` |
| Multi-platform deduplication | JTBD B-3, `05-job-discovery-layer.md` (mentioned briefly) | Add section to `job-search-discovery-engine.md` |
| Application notifications / reminders | `10-application-workflow.md` (follow-up logic), `12-15` (NotificationService) | Add section to `application-workflow-actions.md` |

---

## 4. Proposed Output Spec Structure

**33 output spec files** grouped by domain. Each file is one topic of concern (passes scope test).

### Domain: `architecture` (5 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `architecture-crate-layout.md` | `01-architecture.md`, `20-openapi-mvp.md` | Workspace org, crate boundaries, dependency graph, Cargo.toml |
| `architecture-sqlite-persistence.md` | `04-sqlite-persistence.md`, `01-architecture.md` | sqlx setup, WAL mode, DDL schema, Repository trait, migrations, backup |
| `architecture-tui-skeleton.md` | `09-tui-design-keybindings.md`, `01-architecture.md` | Ratatui layout, view state machine, widget system, keybindings, themes |
| `architecture-privacy-security.md` | `16-privacy-security.md` | OS keyring, optional encryption (age crate), PrivacyMode enum, data export |
| `architecture-config-management.md` | `16-privacy-security.md`, `11-platform-api-integrations.md`, `18-saas-migration-path.md` | TOML config file, API key storage, user preferences, platform token management [NEW] |

### Domain: `profile-resume` (4 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `profile-life-sheet-data-model.md` | `03-life-sheet-data-model.md`, `profile-system.md`, `projects-portfolio.md` | LifeSheet YAML schema, SQLite normalized tables, import/export, ESCO skills |
| `profile-resume-tailoring.md` | `07-resume-tailoring-pipeline.md`, `resume-optimization.md` | JD parsing, gap analysis, LLM bullet rewriting, docx-rs, fabrication guardrails |
| `profile-skills-gap-analysis.md` | `gap-analysis-and-critique.md`, `skills-endorsements.md`, `agentic-job-matching.md` | Skill gap detection, ESCO/O*NET taxonomy, career transitioner framing [NEW] |
| `profile-cover-letter-generation.md` | `08-cover-letter-generation.md`, `cover-letters-applications.md` | CompanyResearcher, tone/length, 3 templates, DOCX output, human-in-the-loop |

### Domain: `job-search` (4 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `job-search-discovery-engine.md` | `05-job-discovery-layer.md`, `11-platform-api-integrations.md` | JobSource trait, DiscoveryService, enrichment pipeline, deduplication, multi-source |
| `job-search-semantic-matching.md` | `05-job-discovery-layer.md`, `agentic-job-matching.md`, `search-and-discovery.md` | Embeddings, cosine similarity, match scoring, ESCO skill inference, feedback loop |
| `job-search-ghost-job-detection.md` | `agentic-job-matching.md`, `job-platforms-comparison.md`, `gap-analysis-and-critique.md` | Heuristics for ghost job classification, age/engagement/employer signals [NEW] |
| `job-search-company-research.md` | `08-cover-letter-generation.md`, `company-pages.md`, `interview-prep-agentic.md` | Company research pipeline: scraping, normalization, CompanyRecord struct [NEW] |

### Domain: `application-tracking` (3 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `application-state-machine.md` | `10-application-workflow.md` | ApplicationStage enum (10 states), transition rules, history table |
| `application-workflow-actions.md` | `10-application-workflow.md`, `cover-letters-applications.md`, `recruiter-workflow.md` | ApplyWorkflow, ScheduleInterviewWorkflow, human-in-the-loop boundaries, anti-spam |
| `application-pipeline-metrics.md` | `10-application-workflow.md`, `12-15-interview-salary-networking-notifications.md` | PipelineMetrics, response rates, stale detection, morning digest/notifications |

### Domain: `networking` (3 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `networking-connection-mapping.md` | `networking-referrals-agentic.md`, `network-graph.md` | Contact graph model, warm-path finding, 1st/2nd-degree targeting |
| `networking-outreach-drafting.md` | `networking-referrals-agentic.md`, `messaging-inmail.md`, `12-15-interview-salary-networking-notifications.md` | Personalized outreach generation, tone calibration, contact status tracking |
| `networking-referral-management.md` | `networking-referrals-agentic.md`, `job-search-workflow-today.md` | Referral request timing, relationship maintenance loop, anti-spam guardrails |

### Domain: `interview-prep` (2 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `interview-prep-question-generation.md` | `interview-prep-agentic.md`, `12-15-interview-salary-networking-notifications.md` | Personalized question sets, company research integration, STAR method |
| `interview-prep-mock-loop.md` | `interview-prep-agentic.md`, `12-15-interview-salary-networking-notifications.md` | AI mock interview flow, Q→response→feedback loop, progress tracking |

### Domain: `salary-negotiation` (2 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `salary-market-intelligence.md` | `salary-negotiation-offers.md`, `12-15-interview-salary-networking-notifications.md` | levels.fyi data, total comp math (base/RSU/signing/bonus), competing offer leverage |
| `salary-counter-offer-drafting.md` | `salary-negotiation-offers.md` | Counter-offer email generation, negotiation strategy, outcome tracking |

### Domain: `agentic` (4 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `agentic-ralph-subprocess-protocol.md` | `06-ralph-loop-integration.md` | IPC design, tokio::process, stdio JSON framing, crash recovery, cancellation |
| `agentic-ralph-orchestration.md` | `06-ralph-loop-integration.md`, `17-ralph-prompt-templates.md` | Loop types, scheduling, parallelism limits, loop priority, SQLite result writes |
| `agentic-llm-provider-abstraction.md` | `02-llm-provider-abstraction.md` | LlmProvider trait, three provider impls, SSE streaming, token tracking, error types |
| `agentic-prompt-templates.md` | `17-ralph-prompt-templates.md`, `07-resume-tailoring-pipeline.md` | All 7 loop type prompts, JSON output schemas, anti-fabrication rules, injection defense |

### Domain: `platform-integrations` (3 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `platform-ats-open-apis.md` | `11-platform-api-integrations.md`, `05-job-discovery-layer.md` | Greenhouse API, Lever API, PlatformClient trait, rate limiting, data normalization |
| `platform-closed-platforms.md` | `agent-interfaces-job-platforms.md`, `11-platform-api-integrations.md` | LinkedIn/Workday access limitations, browser automation risks, ToS/legal, rebrowser |
| `platform-aggregation-deduplication.md` | `agentic-job-matching.md`, `agent-interfaces-job-platforms.md` | Adzuna API, Merge.dev/Unified.to unified APIs, cross-source deduplication logic |

### Domain: `saas` (3 specs)

| Output File | Source Material | Scope |
|---|---|---|
| `saas-migration-path.md` | `18-saas-migration-path.md` | Repository trait abstraction, Supabase sync layer, multi-tenancy, CRDT sync |
| `saas-llm-proxy.md` | `18-saas-migration-path.md`, `02-llm-provider-abstraction.md` | Server-side LLM proxy, subscription tier gating, token usage tracking, billing |
| `saas-pricing-strategy.md` | `19-competitor-analysis.md`, `premium-monetization.md`, `18-saas-migration-path.md` | Tier definitions (Free/Pro/Team), pricing rationale, competitor benchmarks, free tier limits |

---

## 5. Spec Count Summary

| Domain | Source Specs | Output Specs |
|---|---|---|
| architecture | 5 (01, 04, 09, 16, 20) | 5 |
| profile-resume | 7 (03, 07, 08, profile-system, projects-portfolio, resume-optimization, skills-endorsements) | 4 |
| job-search | 7 (05, feed-algorithm, search-discovery, agentic-job-matching, company-pages, job-platforms-comparison, job-search-marketplace) | 4 |
| application-tracking | 4 (04 partial, 10, cover-letters, recruiter-workflow) | 3 |
| networking | 4 (networking-referrals, network-graph, messaging-inmail, 12-15 partial) | 3 |
| interview-prep | 2 (interview-prep-agentic, 12-15 partial) | 2 |
| salary-negotiation | 2 (salary-negotiation-offers, 12-15 partial) | 2 |
| agentic | 3 (06, 02, 17) | 4 |
| platform-integrations | 4 (11, agent-interfaces, job-platforms-comparison, company-pages partial) | 3 |
| saas | 3 (18, 19-competitor, premium-monetization) | 3 |
| **Total** | **38 source** | **33 output** |

---

## 6. Cross-Cutting Constraints (apply to ALL specs)

Every output spec must acknowledge these constraints where relevant:

1. **Privacy/local-first** — no telemetry, SQLite on disk, OS keyring for secrets. Source: `16-privacy-security.md`
2. **Offline-first** — read ops work without internet; LLM features degrade gracefully. Source: `01-architecture.md`
3. **Human in the loop** — agent drafts, human approves, nothing sent without confirm. Source: `cover-letters-applications.md`, `10-application-workflow.md`
4. **Anti-fabrication** — all AI output traceable to real LifeSheet data. Source: `07-resume-tailoring-pipeline.md`, `17-ralph-prompt-templates.md`
5. **Ghost job filtering** — 27–30% of listings are noise; detect and deprioritize proactively. Source: `agentic-job-matching.md`
6. **Quality over volume** — LazyJob is NOT a spray-and-pray tool. Source: `recruiter-workflow.md`, `cover-letters-applications.md`

---

## 7. Implementation Order Notes (for task 12)

When writing IMPLEMENTATION_PLAN.md from the output specs, order tasks as:
1. `architecture-*` specs first — crate scaffolding, SQLite, TUI skeleton, config
2. `profile-life-sheet-data-model.md` — required by all other features
3. `agentic-llm-provider-abstraction.md` — required by all AI features
4. `agentic-ralph-subprocess-protocol.md` + `agentic-ralph-orchestration.md` — required by agentic flows
5. `job-search-discovery-engine.md` → `job-search-semantic-matching.md` → `job-search-ghost-job-detection.md` → `job-search-company-research.md`
6. `application-state-machine.md` → `application-workflow-actions.md` → `profile-resume-tailoring.md` → `profile-cover-letter-generation.md`
7. `networking-*`, `interview-prep-*`, `salary-*` — build on established infrastructure
8. `platform-*` — integration layer
9. `saas-*` — cloud migration last
