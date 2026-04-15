# Progress Log

Started: 2026-04-15
Objective: Expand lazyjob specs into JTBD → Topics of Concern → Specs → Tasks hierarchy

Existing specs (36 files in specs/): 01-architecture.md, 02-llm-provider-abstraction.md,
03-life-sheet-data-model.md, 04-sqlite-persistence.md, 05-job-discovery-layer.md,
06-ralph-loop-integration.md, 07-resume-tailoring-pipeline.md, 08-cover-letter-generation.md,
09-tui-design-keybindings.md, 10-application-workflow.md, 11-platform-api-integrations.md,
12-15-interview-salary-networking-notifications.md, 16-privacy-security.md,
17-ralph-prompt-templates.md, 18-saas-migration-path.md, 19-competitor-analysis.md,
20-openapi-mvp.md, agentic-job-matching.md, agent-interfaces-job-platforms.md,
company-pages.md, cover-letters-applications.md, feed-algorithm.md,
gap-analysis-and-critique.md, interview-prep-agentic.md, job-platforms-comparison.md,
job-search-marketplace.md, job-search-workflow-today.md, messaging-inmail.md,
network-graph.md, networking-referrals-agentic.md, premium-monetization.md,
profile-system.md, projects-portfolio.md, recruiter-workflow.md,
resume-optimization.md, salary-negotiation-offers.md, search-and-discovery.md,
skills-endorsements.md, x-professional-features.md

---

## Iteration 1 — Task 1: jtbd-extraction-audience

- **What I produced:** `ralph/spec-jtbd-expansion/output/AUDIENCE_JTBD.md`
- **Key findings:**
  - Four distinct audiences: Active Job Seeker (primary), Passive Seeker/Power User, Career Changer, Tool Author/SaaS Operator. The first is the primary user; the others shape architecture.
  - 13 JTBDs extracted (A-1 through D-2), each scoped to a single outcome sentence. Six serve the active job seeker (find jobs, apply, track, network, interview prep, salary negotiation). Three serve passive/power users (monitor market, understand gaps, multi-platform access). Two serve career changers. Two serve the tool author (SaaS migration, LLM proxy).
  - Five cross-cutting constraints identified that must apply to ALL specs: privacy/local-first, offline-first, human-in-the-loop (agent drafts, human approves), anti-fabrication guardrails, ghost job filtering. These are not JTBDs but hard constraints.
- **What next iteration should know:**
  - The AUDIENCE_JTBD.md includes a JTBD→Domain mapping table that maps each JTBD to a spec domain (job-search, application-tracking, networking, interview-prep, salary-negotiation, profile-resume, platform-integrations, architecture, saas). Use this as the primary reference when organizing spec inventory.
  - The `gap-analysis-and-critique.md` spec explicitly called out ghost job detection as the single highest-impact missing topic. This should get its own spec in the job-search domain (not just a mention in others).
  - Career changers are explicitly underserved by all existing tools — this audience should get its own section in the profile/resume domain specs rather than being folded into the general job seeker flow.
  - The recruiter-workflow.md spec makes clear LazyJob must NOT be an auto-apply spam tool. The value proposition is quality over volume. This is a design principle worth restating in the application-tracking and agentic specs.
  - The existing 36 specs are a mix of LinkedIn/platform research (many files) and LazyJob-specific technical specs (the numbered files). The numbered files (01–20) are the core architecture specs. The unnumbered files are market research. Task 2 (spec inventory) needs to distinguish these clearly.

---

## Iteration 2 — Task 2: spec-inventory-consolidation-plan

- **What I produced:** `ralph/spec-jtbd-expansion/output/spec-inventory.md`
- **Key findings:**
  - All 38 source specs classified into two tiers: Tier A (16 LazyJob-specific technical specs, numbered 01–20 plus a few others) and Tier B (22 LinkedIn/platform market research specs). Tier B files are research artifacts that must be synthesized into output specs, not copied verbatim.
  - Five major redundancies identified: `profile-system.md` + `projects-portfolio.md` overlap on the same LinkedIn profile material; `12-15-interview-salary-networking-notifications.md` **violates the scope test** (4 distinct concerns) and must be split; `06-ralph-loop-integration.md` conflates IPC protocol and orchestration logic and should split into two specs. Two contacts-table schema conflicts between specs 03 and 04 exist intentionally (different tables for different purposes — must be named distinctly).
  - Four new specs identified that have no source file: `job-search-ghost-job-detection.md`, `job-search-company-research.md`, `architecture-config-management.md`, `profile-skills-gap-analysis.md`. These address genuinely missing concerns.
- **What next iteration should know:**
  - Final proposed output: **33 spec files** across 10 domains. See Section 4 of `spec-inventory.md` for the complete file list and source material for each output spec.
  - Task 3 (job-search domain) must produce **4 specs**: `job-search-discovery-engine.md`, `job-search-semantic-matching.md`, `job-search-ghost-job-detection.md`, `job-search-company-research.md`. The last two are NEW (no dedicated source file) — synthesize from `agentic-job-matching.md` (ghost jobs) and `08-cover-letter-generation.md` + `interview-prep-agentic.md` + `company-pages.md` (company research).
  - `CompanyResearch` struct should live in `lazyjob-core`, not in either of its consumers (cover letter or interview prep). Establish this placement in the company research spec.
  - Section 7 of `spec-inventory.md` documents the implementation ordering — architecture first, then LifeSheet, then LLM abstraction, then Ralph protocol, then features.

---

## Iteration 3 — Task 3: specs-job-search-discovery

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/job-search-discovery-engine.md`
  - `ralph/spec-jtbd-expansion/output/specs/job-search-semantic-matching.md`
  - `ralph/spec-jtbd-expansion/output/specs/job-search-ghost-job-detection.md`
  - `ralph/spec-jtbd-expansion/output/specs/job-search-company-research.md`

- **Key findings:**
  - The discovery engine spec establishes `JobSource` trait, `EnrichmentPipeline`, `CompanyRegistry`, and `DiscoveryService` in `lazyjob-core/src/discovery/`. It sets the Greenhouse + Lever APIs as Phase 1 sources (both public, no auth), and Adzuna as Phase 2 (free tier rate limits make hourly polling borderline). Deduplication is two-tier: primary key `(source, source_id)` for same-source, plus fuzzy `(company_id, title_normalized, location_normalized)` for cross-source.
  - Semantic matching defaults to Ollama `nomic-embed-text` (768 dims, fully offline). For single-user scale (~5000 jobs), all embeddings fit in ~1.5 MB RAM — no vector DB needed. Feed ranking formula established: `feed_score = match_score * (1 - ghost_score) * recency_decay * feedback_multiplier`. ESCO skill inference is optional, config-gated, and cached by experience-text hash.
  - Ghost detection uses 7 weighted heuristics (posting age, repost count, description vagueness, salary-absent-in-transparency-state, no named contact, company headcount declining). Critical design rule: ghost detection **never silently hides** a job — it badges and deprioritizes but the user can override. The `description_vagueness` scorer reuses the same technical term regex lexicon as ghost detection, establishing a shared vocabulary asset.
  - Company Research established `CompanyRecord` as the canonical company entity in `lazyjob-core/src/companies/` — this is an explicit architecture ruling: cover letter generation, ghost detection, and interview prep all query `CompanyRepository` rather than maintaining their own company data. Phase 1 covers company website + tech-stack inference from job descriptions (offline, no API). Phase 2 adds Glassdoor, Crunchbase, news RSS, layoffs.fyi.

- **What next iteration should know:**
  - The `description_vagueness` regex lexicon (technical terms: programming languages, tools, frameworks) is used by both ghost detection and tech-stack extraction in company research. Task 4 (profile/resume domain) and task 8 (agentic) may also need this lexicon. Consider extracting it to a shared asset in `lazyjob-core/src/lexicon/tech_terms.rs` early.
  - `CompanyRecord.tech_stack` is populated both by LLM extraction (company website) and offline regex inference (job descriptions). The offline pass is free and should always run; the LLM pass is optional and should be gated. Future specs must not add a third company data store — all company data goes through `CompanyRepository`.
  - The `feed_score` formula (`match_score * (1 - ghost_score) * recency_decay * feedback_multiplier`) is defined in `job-search-semantic-matching.md`. The TUI's job feed view must sort by `feed_score`, not `match_score` alone. Alert the TUI spec writer (task 10, architecture domain) to this dependency.
  - Glassdoor scraping for company research is explicitly deferred with a recommendation to use a clipboard-paste power-user approach in Phase 2 rather than automated scraping. This avoids ToS risk.
  - Ghost detection's `salary_absent_in_transparency_state` signal requires a static `HashSet<&str>` of pay-transparency jurisdictions. This data is stable enough to embed in the binary. The `pay_transparency_jurisdictions` set should be placed in a dedicated module so it can be shared if salary-negotiation specs also need it.

---

## Iteration 4 — Task 4: specs-profile-resume

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/profile-life-sheet-data-model.md`
  - `ralph/spec-jtbd-expansion/output/specs/profile-resume-tailoring.md`
  - `ralph/spec-jtbd-expansion/output/specs/profile-skills-gap-analysis.md`
  - `ralph/spec-jtbd-expansion/output/specs/profile-cover-letter-generation.md`

- **Key findings:**
  - **Deterministic IDs for LifeSheet entities**: The YAML import spec establishes that `work_experience.id` should be a hash of `(company, position, start_date)` rather than random UUID — this ensures IDs are stable across re-imports, so `resume_versions.experience_id` FKs remain valid. All downstream specs that store LifeSheet entity references should adopt this pattern.
  - **CompanyRepository as the shared company data source**: Cover letter generation is an explicit second consumer of `CompanyRecord` (alongside ghost detection and interview prep from task 3). The architecture ruling is now confirmed by two domain specs: cover letter generation queries `CompanyRepository` rather than doing its own research. Task 8 (agentic) and task 7 (interview-salary) should also reference `CompanyRepository` rather than creating new company fetching logic.
  - **Skill normalization is foundational**: The `SkillNormalizer` and embedded skill alias table built for gap analysis are also needed by resume tailoring (for gap computation in the tailoring pipeline) and by job search semantic matching (for the same vocabulary bridging problem). There should be ONE `SkillNormalizer` in `lazyjob-core/src/lexicon/` shared across all three consumers — not three independent normalization implementations.

- **What next iteration should know:**
  - The `profile-resume-tailoring.md` spec references `JobDescriptionAnalysis` (from the JD parser) as a shared struct. Both resume tailoring (task 4) and gap analysis (task 4) use this struct. It should live in `lazyjob-core/src/resume/jd_parser.rs` and be re-exported, not duplicated. Gap analysis imports it when running against saved jobs.
  - Cover letter generation's fabrication check for narrative text (extract quantified metric claims → verify against LifeSheet) is a different mechanism than the resume fabrication check (skill-by-skill comparison). Both use `is_grounded_claim` from `lazyjob-core/src/life_sheet/fabrication.rs` as the ground truth predicate.
  - The `profile_contacts` table (in LifeSheet) and `application_contacts` table (in spec 04) naming distinction is now formally established. Task 6 (networking specs) and task 5 (application-tracking specs) must use these distinct names and should NOT propose merging them.
  - Career changer templates (cover letter Template 3, transferable skill analysis in gap analysis) require `LifeSheet.goals` to be populated. The onboarding flow (TUI) must prompt users to fill `goals.short_term` when they first run LazyJob, otherwise these features silently degrade. Task 10 (architecture/TUI spec) should document this as a required onboarding step.

---

## Iteration 5 — Task 5: specs-application-tracking

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/application-state-machine.md`
  - `ralph/spec-jtbd-expansion/output/specs/application-workflow-actions.md`
  - `ralph/spec-jtbd-expansion/output/specs/application-pipeline-metrics.md`

- **Key findings:**
  - **`application_contacts` vs `profile_contacts` naming is now formally enforced in DDL**: The `application_contacts` table stores per-hiring-process contacts (recruiter, panel interviewers, hiring manager). `profile_contacts` (in LifeSheet) stores the user's professional network for networking (JTBD A-4). Same person may appear in both; they serve different lookup patterns. This distinction was established in task 4 and is now reflected in the SQL schema.
  - **Human-in-the-loop boundaries are explicit product policy, not just safety checks**: The recruiter research establishes that 91% of recruiters have spotted AI deception, and 34% spend half their week filtering spam. The application workflow spec lists a precise 3-tier boundary: (1) always automated, (2) requires confirmation, (3) never automated. Direct ATS submission (Greenhouse/Lever API) is Phase 2 only, requires per-company opt-in, and always shows a full data preview — never auto-submits. This boundary map must NOT be weakened by future spec writers.
  - **Pipeline metrics denominator is explicitly defined**: Conversion rates (response_rate, interview_rate, offer_rate) exclude `Discovered` and `Interested` stage applications from the denominator. Only applications where a user actually submitted (Applied+) count. This prevents artificially deflated metrics that would discourage users.

- **What next iteration should know:**
  - The `ReminderPoller` (tokio background task, 5-minute interval) emits `WorkflowEvent::ReminderDue` events. The TUI's main event loop must consume `WorkflowEvent` from the broadcast channel — this is a TUI architecture concern. Task 10 (architecture/TUI spec) must document the event subscription pattern.
  - The `MoveStageWorkflow::execute` returns `PostTransitionSuggestion` variants (GenerateInterviewPrep, RunSalaryComparison, GenerateCompanyCheatSheet). These suggestions trigger ralph loops in other domains. Task 8 (agentic) must define how the TUI dispatches a `PostTransitionSuggestion` into a `RalphLoopType` — the mapping should live in `lazyjob-ralph/src/dispatch.rs`.
  - The `application-state-machine.md` spec defines `InterviewRepository` and `OfferRepository` as separate traits. The metrics spec depends on both. Task writers for networking (task 6) and interview-salary (task 7) domains should NOT create new `Interview` or `Offer` structs — they must reference the ones defined here in `lazyjob-core/src/application/model.rs`.
  - The morning digest (`DigestService`) depends on `JobRepository::count_new_matches_since(DateTime)` — a method not yet defined. Task 10 (architecture) or task 3 (already done, job-search domain) should verify this query exists. If not, add it to `job-search-discovery-engine.md`'s Implementation Tasks.
  - The anti-spam gate in `ApplyWorkflow` (ghost score check, daily application count warning) requires calling `GhostDetector` from the job-search domain across crate boundaries. This is clean if both are in `lazyjob-core`. Confirm crate placement when task 10 writes the architecture spec.

---

## Iteration 6 — Task 6: specs-networking-referrals

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/networking-connection-mapping.md`
  - `ralph/spec-jtbd-expansion/output/specs/networking-outreach-drafting.md`
  - `ralph/spec-jtbd-expansion/output/specs/networking-referral-management.md`

- **Key findings:**
  - **No LinkedIn API — hard constraint**: LinkedIn's ToS explicitly prohibits automation; there is no public API for messaging or connection management. LazyJob's entire networking system is built around user-imported data (LinkedIn CSV export) and manual-send workflows. All three specs explicitly document this as a non-negotiable product constraint, not a future roadmap item. The agent drafts, the human copies and sends.
  - **`profile_contacts` DDL must grow significantly**: The existing `profile_contacts` table (established in task 4) needs `previous_companies_json`, `schools_json`, `relationship_stage`, `interaction_count`, `follow_up_exhausted`, `outreach_status`, `last_draft_text` columns. Three distinct migration blocks are defined across the three specs. Task 10 (architecture/sqlite) and task 12 (implementation plan) should aggregate these DDL additions into a single schema migration.
  - **`ReferralReadinessChecker` crosses domain boundaries**: It calls `GhostDetector` (from `job-search-ghost-job-detection.md`) to verify a job is real before suggesting a referral ask. Both must live in `lazyjob-core` for this to work without cross-crate circular dependencies. The ghost score threshold for blocking referral suggestions is 0.6 (consistent with the ghost detection spec's "probable ghost" tier).
  - **New DDL table required**: `referral_asks` table created to track per-(contact, job) referral state with unique constraint on `(contact_id, job_id)`. This was not in any prior spec.

- **What next iteration should know:**
  - The `NetworkingReminderPoller` emits `WorkflowEvent::NetworkingReminderDue` on the same broadcast channel as the `ReminderPoller` from application-workflow-actions.md. Task 10 (architecture/TUI) must document that the TUI subscribes to ONE `WorkflowEvent` broadcast channel and handles multiple event variants (application reminders, networking reminders, digest events). Don't let multiple pollers create separate channels.
  - The outreach drafting spec establishes `SharedContext` as a pure structural computation (no LLM) used to ground the LLM prompt. This is the same grounding-before-generation pattern used in resume tailoring (`JobDescriptionAnalysis` is pre-computed, then fed to LLM). Task 8 (agentic/prompt templates) should document this as a standard pattern for all LLM-generating specs: compute structured facts first, then pass them to LLM as verified ground truth.
  - Company name normalization for contact-company matching reuses `normalize_company_name()` from the discovery engine (task 3). This function must be in a shared module in `lazyjob-core/src/utils/` — not duplicated in networking and discovery separately. Task 10 (architecture) should call this out.
  - The referral management spec's anti-spam cap (max 2 follow-up reminders per contact per role) and the outreach spec's weekly new-contact cap (max 5 new outreaches per week) are both configurable in `lazyjob.toml`. Task 10 (architecture-config-management spec) should include these as named config keys under a `[networking]` section.
  - `PostTransitionSuggestion::UpdateReferralOutcome` is a new variant added to the `PostTransitionSuggestion` enum (established in application-workflow-actions.md). Task 8 (agentic) needs to update `lazyjob-ralph/src/dispatch.rs` to handle this variant.

---

## Iteration 7 — Task 7: specs-interview-salary

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/interview-prep-question-generation.md`
  - `ralph/spec-jtbd-expansion/output/specs/interview-prep-mock-loop.md`
  - `ralph/spec-jtbd-expansion/output/specs/salary-market-intelligence.md`
  - `ralph/spec-jtbd-expansion/output/specs/salary-counter-offer-drafting.md`

- **Key findings:**
  - **`pay_transparency_jurisdictions` is now formally shared**: The static `HashSet<&str>` of pay-transparency jurisdictions lives in `lazyjob-core/src/salary/jurisdictions.rs` — shared by both salary market intelligence (offer-vs-posted-range check) and ghost detection (`salary_absent_in_transparency_state` signal from task 3). These must reference the same module; task 10 (architecture) should note this shared utility.
  - **No salary data API — clipboard import is the pattern**: levels.fyi, Glassdoor, and Blind all lack public APIs. Phase 1 uses H1B LCA public DOL data (downloadable CSV, offline SQLite import) + user-entered reference points. Phase 2 uses user-pasted levels.fyi table text parsed by `LevelsFyiParser`. No scraping. This mirrors the LinkedIn CSV import pattern established in networking specs.
  - **`OfferRepository` extended, not duplicated**: `OfferRepository` was established in `lazyjob-core/src/application/model.rs` (task 5). The salary market intelligence spec adds `save_offer_details` and `get_offers_for_application` to it — no new repository trait was created. Task 8 (agentic) and task 10 (architecture) should be aware of this extension.
  - **`offer_details` table explicitly excluded from SaaS cloud sync**: Offer details are sensitive (may violate offer letter confidentiality terms). The salary spec formally declares `offer_details` excluded from the SaaS sync scope. Task 11 (saas-migration-path spec) must list this table in its "never sync" exclusion list.

- **What next iteration should know:**
  - **Mock interview loop uses a NEW ralph subprocess loop type**: `MockInterviewLoop` is an interactive loop (question → user types response → feedback). This is different from all other ralph loops (which are fire-and-forget background processes). The IPC protocol spec (task 8, `agentic-ralph-subprocess-protocol.md`) must account for a bidirectional interactive mode: the subprocess waits for user input after emitting a question. This is the only loop type that requires a user-input return channel.
  - **`PostTransitionSuggestion::GenerateCompanyCheatSheet` from task 5** (established in application-workflow-actions.md) is consumed by interview prep — it triggers `InterviewPrepService::generate_prep_session` via a ralph loop dispatch. Task 8 (agentic) must wire this in `lazyjob-ralph/src/dispatch.rs`.
  - **`NegotiationHistory.comp_delta` is aggregate-ready**: The delta between initial and final negotiated offer (in cents) is stored per application. Future SaaS analytics ("median comp delta for Software Engineer roles") should query this column. Task 11 (saas) can note this as a low-effort analytics signal.
  - **Counter-offer fabrication constraint is the strictest in the codebase**: The counter-offer draft prompt must NEVER invent a competing offer. This constraint is stricter than the resume and cover letter fabrication guards (which prevent inventing skills/metrics). Task 8 (agentic-prompt-templates spec) should document this as a dedicated constraint class: "negotiation context fabrication" — a distinct category from "profile fabrication."
  - **Behavioral question → LifeSheet story linkage** (`candidate_story_ref` in `InterviewQuestion`) enables the mock loop's STAR fabrication detection. The mock evaluator checks whether the user's typed response introduces claims not present in the linked story. This cross-spec dependency (question_gen → mock_loop → life_sheet) must be documented in the agentic orchestration spec (task 8) when defining the `InterviewPrep` loop type.

---

## Iteration 8 — Task 8: specs-agentic-ai-layer

- **What I produced:**
  - `ralph/spec-jtbd-expansion/output/specs/agentic-ralph-subprocess-protocol.md`
  - `ralph/spec-jtbd-expansion/output/specs/agentic-ralph-orchestration.md`
  - `ralph/spec-jtbd-expansion/output/specs/agentic-llm-provider-abstraction.md`
  - `ralph/spec-jtbd-expansion/output/specs/agentic-prompt-templates.md`

- **Key findings:**
  - **MockInterviewLoop is the only bidirectional loop**: The IPC protocol spec formally establishes `WorkerCommand::UserInput` and `WorkerEvent::AwaitingInput` as the two message types used exclusively by `MockInterviewLoop`. All other loops are fire-and-forget. The protocol spec documents a state machine: `Idle → Ready → [AwaitingInput → blocked on stdin → Status]*  → Done`. No inactivity timeout should kill this loop — only user-initiated cancel or the user's own inactivity timer.
  - **Three-tier fabrication constraint system is now formally defined**: Tier 1 (profile fabrication) applies to resume/cover letter; Tier 2 (narrative fabrication) adds cover letter and networking outreach; Tier 3 (negotiation context fabrication) is the strictest and applies only to counter-offer drafts. The counter-offer template system prompt contains an absolute prohibition on inventing competing offers and the `validate_output()` function regex-scans for competing-offer phrases and blocks output entirely if found. This is documented in `agentic-prompt-templates.md` as a hardcoded, non-user-configurable constraint.
  - **`dispatch.rs` is now fully specified**: `LoopDispatch::dispatch_suggestion()` maps all four `PostTransitionSuggestion` variants (from application-tracking, task 5, and networking, task 6) to their corresponding `LoopType` + `params` objects. The `UpdateReferralOutcome` variant added in task 6 is wired to `LoopType::NetworkingOutreachDraft` with `mode="referral_outcome"`.

- **What next iteration should know:**
  - **`LlmBuilder` must support a `LoomProxyProvider` in SaaS mode**: The spec leaves this as an open question, but task 11 (saas) should resolve it. The cleanest approach: `[llm.proxy]` config section triggers `LlmBuilder::build()` to return a `LoomProxyProvider` impl that routes all `LlmProvider` calls through the server-side proxy HTTP endpoint, transparently replacing the direct-to-provider path. No callers need to change.
  - **`EmbeddingProvider` is a separate sub-trait**: Anthropic does NOT implement it. The default embedding provider for offline use is `OllamaProvider` with `nomic-embed-text` (768 dims). This must be consistent with `job-search-semantic-matching.md` (task 3) which established `nomic-embed-text` as the embedding model for offline job scoring. Task 10 (architecture) should note that `OllamaProvider` serves double duty: chat (fallback) and embeddings (primary for offline users).
  - **`token_usage_log` table and `cost.rs` microdollar estimator are new schema additions**: Task 10 (architecture-sqlite spec) should include `token_usage_log` in its DDL inventory. Task 11 (saas) can use `token_usage_log` as the billing signal for per-user token metering in the SaaS tier.
  - **Prompt templates are in `lazyjob-llm/src/prompts/`, not in `lazyjob-ralph/`**: This keeps prompt logic in the LLM crate (testable without spawning workers). Workers import `lazyjob-llm` and call `prompts::resume_tailoring::user_prompt(ctx)` before calling `LlmProvider::chat()`. Task 10 should note this crate dependency clearly: `lazyjob-ralph` depends on `lazyjob-llm` depends on `lazyjob-core`.
  - **Grounding context structs pull from multiple prior specs**: `ResumeTailoringContext` needs `JobDescriptionAnalysis` (task 4 profile-resume-tailoring.md), `NetworkingContext` needs `SharedHistory` (task 6 networking-outreach-drafting.md), `InterviewContext` needs `InterviewQuestion.candidate_story_ref` (task 7). These cross-domain dependencies are now fully documented in the prompt templates spec.

---
