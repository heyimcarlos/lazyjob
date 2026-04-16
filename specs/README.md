# LazyJob Specifications

This directory contains the architectural specifications and design documents for LazyJob — a terminal-based job search command center powered by AI agents.

## Quick Navigation

### Core Architecture (20 specs)
| Spec | Topic | Status |
|------|-------|--------|
| [01-architecture.md](./01-architecture.md) | Crate layout, TUI structure, domain models | Researching |
| [02-llm-provider-abstraction.md](./02-llm-provider-abstraction.md) | Multi-provider LLM support (Anthropic, OpenAI, Ollama) | Researching |
| [03-life-sheet-data-model.md](./03-life-sheet-data-model.md) | YAML schema + SQLite model for job seeker profile | Researching |
| [04-sqlite-persistence.md](./04-sqlite-persistence.md) | SQLite with rusqlite/sqlx, migrations, backup | Researching |
| [05-job-discovery-layer.md](./05-job-discovery-layer.md) | Greenhouse/Lever API, semantic matching, embeddings | Researching |
| [05-job-discovery-layer-implementation-plan.md](./05-job-discovery-layer-implementation-plan.md) | Implementation plan for Job Discovery Layer | Draft |
| [06-ralph-loop-integration.md](./06-ralph-loop-integration.md) | Autonomous AI agent subprocesses, JSON protocol | Researching |
| [07-resume-tailoring-pipeline.md](./07-resume-tailoring-pipeline.md) | AI resume customization with gap analysis | Researching |
| [08-cover-letter-generation.md](./08-cover-letter-generation.md) | AI cover letter writing with company research | Researching |
| [09-tui-design-keybindings.md](./09-tui-design-keybindings.md) | TUI layout, views, vim-inspired keybindings | Researching |
| [10-application-workflow.md](./10-application-workflow.md) | Application state machine, pipeline kanban | Researching |
| [11-platform-api-integrations.md](./11-platform-api-integrations.md) | Job board clients (Greenhouse, Lever, Workday) | Researching |
| [12-15-interview-salary-networking-notifications.md](./12-15-interview-salary-networking-notifications.md) | Interview prep, salary negotiation, networking, notifications | Researching |
| [16-privacy-security.md](./16-privacy-security.md) | Encryption, keyring integration, data export | Researching |
| [17-ralph-prompt-templates.md](./17-ralph-prompt-templates.md) | LLM prompts for all Ralph loop types | Researching |
| [18-saas-migration-path.md](./18-saas-migration-path.md) | Local-first to cloud SaaS migration architecture | Researching |
| [19-competitor-analysis.md](./19-competitor-analysis.md) | Huntr, Teal, LazyGit analysis | Researching |
| [20-openapi-mvp.md](./20-openapi-mvp.md) | MVP build plan synthesizing all specs | Draft |

### Agentic Ralph System
| Spec | Topic | Status |
|------|-------|--------|
| [agentic-llm-provider-abstraction.md](./agentic-llm-provider-abstraction.md) | LLM provider traits for Ralph loops | Researching |
| [agentic-prompt-templates.md](./agentic-prompt-templates.md) | Prompt templates for agentic tasks | Researching |
| [agentic-ralph-orchestration.md](./agentic-ralph-orchestration.md) | Orchestrating multiple Ralph loops | Researching |
| [agentic-ralph-subprocess-protocol.md](./agentic-ralph-subprocess-protocol.md) | JSON protocol for subprocess communication | Researching |

### Application & Pipeline
| Spec | Topic | Status |
|------|-------|--------|
| [application-state-machine.md](./application-state-machine.md) | Application lifecycle state transitions | Researching |
| [application-workflow-actions.md](./application-workflow-actions.md) | Actions triggered by workflow events | Researching |
| [application-pipeline-metrics.md](./application-pipeline-metrics.md) | Pipeline analytics and metrics | Researching |

### Job Search
| Spec | Topic | Status |
|------|-------|--------|
| [job-search-discovery-engine.md](./job-search-discovery-engine.md) | Job search discovery architecture | Researching |
| [job-search-semantic-matching.md](./job-search-semantic-matching.md) | Semantic matching algorithms | Researching |
| [job-search-ghost-job-detection.md](./job-search-ghost-job-detection.md) | Detecting fake/ghost job listings | Researching |
| [job-search-company-research.md](./job-search-company-research.md) | Company research integration | Researching |

### Resume & Profile
| Spec | Topic | Status |
|------|-------|--------|
| [profile-life-sheet-data-model.md](./profile-life-sheet-data-model.md) | Life sheet YAML schema | Researching |
| [profile-resume-tailoring.md](./profile-resume-tailoring.md) | Resume customization per job | Researching |
| [profile-cover-letter-generation.md](./profile-cover-letter-generation.md) | Cover letter generation | Researching |
| [profile-skills-gap-analysis.md](./profile-skills-gap-analysis.md) | Skills gap analysis | Researching |

### Interview Preparation
| Spec | Topic | Status |
|------|-------|--------|
| [interview-prep-question-generation.md](./interview-prep-question-generation.md) | Question bank generation | Researching |
| [interview-prep-mock-loop.md](./interview-prep-mock-loop.md) | Mock interview loop system | Researching |
| [interview-prep-agentic.md](./interview-prep-agentic.md) | Agentic interview preparation | Researching |

### Salary & Compensation
| Spec | Topic | Status |
|------|-------|--------|
| [salary-market-intelligence.md](./salary-market-intelligence.md) | Market salary data aggregation | Researching |
| [salary-negotiation-offers.md](./salary-negotiation-offers.md) | Negotiation tactics and offer evaluation | Researching |
| [salary-counter-offer-drafting.md](./salary-counter-offer-drafting.md) | Counter offer letter drafting | Researching |

### Networking & Outreach
| Spec | Topic | Status |
|------|-------|--------|
| [networking-connection-mapping.md](./networking-connection-mapping.md) | Connection graph mapping | Researching |
| [networking-outreach-drafting.md](./networking-outreach-drafting.md) | Outreach message drafting | Researching |
| [networking-referral-management.md](./networking-referral-management.md) | Referral tracking and management | Researching |
| [networking-referrals-agentic.md](./networking-referrals-agentic.md) | Agentic referral generation | Researching |

---

## Architecture Overview

```
lazyjob/
├── lazyjob-core/      # Domain models, SQLite persistence
├── lazyjob-llm/       # LLM provider abstraction (Anthropic, OpenAI, Ollama)
├── lazyjob-ralph/     # Ralph loop integration (subprocess manager)
├── lazyjob-tui/       # Terminal UI (ratatui + crossterm)
├── lazyjob-cli/       # Binary entry point
└── specs/             # This directory
```

## Core Concepts

### Ralph Loops
Autonomous AI agent loops that run in subprocesses, powered by LLM prompts. Each loop type handles a specific job search task:

- **Job Discovery** — Fetches jobs from Greenhouse/Lever, matches to profile
- **Company Research** — Gathers mission, culture, recent news, tech stack
- **Resume Tailoring** — Rewrites resume bullets with job description keywords
- **Cover Letter Generation** — Writes personalized cover letters
- **Interview Prep** — Generates practice questions, mock interview loops
- **Salary Negotiation** — Market data analysis, negotiation strategy
- **Networking** — Contact finding, warm outreach templates

### Life Sheet
A structured YAML representation of the job seeker's complete career profile:
- Personal info, work experience with achievements and metrics
- Education, skills with ESCO/O*NET taxonomy codes
- Certifications, languages, projects
- Job preferences, career goals, contact network

### TUI Views
Inspired by lazygit with vim-style navigation:
- Dashboard, Jobs List, Job Detail, Applications Pipeline (kanban)
- Contacts, Ralph Panel, Settings, Help Overlay

### Data Model
- **Jobs** — Discovered opportunities with status, salary, remote type
- **Applications** — Pipeline stages (Discovered → Applied → Phone Screen → Technical → On-site → Offer)
- **Contacts** — Networking relationships with quality ratings
- **Interviews, Offers, Reminders** — Full application lifecycle tracking

---

## Research Documents

### Platform Research

| Document | Topic |
|---------|-------|
| [agent-interfaces-job-platforms.md](./agent-interfaces-job-platforms.md) | APIs, browser automation, legal boundaries for platform integration |
| [job-platforms-comparison.md](./job-platforms-comparison.md) | Indeed, Glassdoor, ZipRecruiter, Wellfound, Handshake, Hired |
| [job-search-marketplace.md](./job-search-marketplace.md) | LinkedIn job search marketplace, JYMBII recommendation system |
| [search-and-discovery.md](./search-and-discovery.md) | LinkedIn search architecture, Galene, semantic retrieval |
| [network-graph.md](./network-graph.md) | LinkedIn connection system, LIquid graph database |
| [messaging-inmail.md](./messaging-inmail.md) | LinkedIn messaging, InMail, credit system |
| [profile-system.md](./profile-system.md) | LinkedIn profile architecture, component system |
| [skills-endorsements.md](./skills-endorsements.md) | LinkedIn skills taxonomy, Skills Graph, endorsements |
| [company-pages.md](./company-pages.md) | LinkedIn Company Pages, employer branding |
| [projects-portfolio.md](./projects-portfolio.md) | LinkedIn portfolio features, Featured section |
| [feed-algorithm.md](./feed-algorithm.md) | LinkedIn Feed ranking, LiRank, Feed-SR |
| [premium-monetization.md](./premium-monetization.md) | LinkedIn Premium tiers, subscription revenue |
| [x-professional-features.md](./x-professional-features.md) | X.com/Twitter professional and hiring features |

### Competitive & Market Analysis

| Document | Topic |
|---------|-------|
| [job-search-workflow-today.md](./job-search-workflow-today.md) | How job seekers actually search and apply |
| [recruiter-workflow.md](./recruiter-workflow.md) | Recruiter tools, ATS systems, sourcing |
| [agentic-job-matching.md](./agentic-job-matching.md) | AI semantic matching, ghost job detection |
| [resume-optimization.md](./resume-optimization.md) | ATS parsing, resume tailoring, Jobscan/Teal |
| [cover-letters-applications.md](./cover-letters-applications.md) | Cover letter effectiveness, AI generation |

### Feature-Specific Research

| Document | Topic |
|---------|-------|
| [interview-prep-agentic.md](./interview-prep-agentic.md) | Mock interviews, company research, STAR coaching |
| [salary-negotiation-offers.md](./salary-negotiation-offers.md) | Negotiation tactics, offer evaluation, levels.fyi |
| [networking-referrals-agentic.md](./networking-referrals-agentic.md) | Agentic networking, warm outreach, referral generation |

### Process & Critique

| Document | Topic |
|---------|-------|
| [spec-inventory.md](./spec-inventory.md) | Spec inventory, consolidation plan, implementation order |
| [gap-analysis-and-critique.md](./gap-analysis-and-critique.md) | Gap analysis of existing specs, missing topics |

### Supporting Documents

| Document | Topic |
|---------|-------|
| [AUDIENCE_JTBD.md](./AUDIENCE_JTBD.md) | Audiences, JTBDs synthesized from all specs |
| [rust-patterns.md](./rust-patterns.md) | Rust idioms and patterns for the codebase |

---

## Key Design Decisions

1. **Local-first**: All data stored locally in SQLite; no network dependency
2. **Trait-based LLM abstraction**: Provider-agnostic chat, completion, embeddings
3. **Ralph as subprocess**: JSON protocol over stdio; language-agnostic; crash-resilient
4. **YAML life sheet**: Human-editable, machine-readable career profile
5. **Opinionated workflow**: Kanban pipeline with state machine transitions
6. **Path to SaaS**: Shared data model enables PostgreSQL migration later

## Reading Order

For understanding the full system, recommended reading order:

1. **[01-architecture.md](./01-architecture.md)** — High-level overview, crate organization
2. **[20-openapi-mvp.md](./20-openapi-mvp.md)** — MVP scope and implementation phases
3. **[03-life-sheet-data-model.md](./03-life-sheet-data-model.md)** — Core data definition
4. **[04-sqlite-persistence.md](./04-sqlite-persistence.md)** — How data is stored
5. **[06-ralph-loop-integration.md](./06-ralph-loop-integration.md)** — AI agent architecture
6. **[02-llm-provider-abstraction.md](./02-llm-provider-abstraction.md)** — LLM provider traits
7. **[09-tui-design-keybindings.md](./09-tui-design-keybindings.md)** — User interface spec
8. **[05-job-discovery-layer.md](./05-job-discovery-layer.md)** — Job aggregation
9. **[07-resume-tailoring-pipeline.md](./07-resume-tailoring-pipeline.md)** — Resume AI
10. **[17-ralph-prompt-templates.md](./17-ralph-prompt-templates.md)** — LLM prompts

---

## Status Legend

- **Researching** — Gathering information, evaluating options
- **Draft** — Initial design complete, needs review
- **Implemented** — Code exists in repository
- **Complete** — Fully implemented and tested

## Implementation Plans

### Core Architecture

| File | Description | Status |
|------|-------------|--------|
| [01-architecture-implementation-plan.md](./01-architecture-implementation-plan.md) | LazyJob crate layout, workspace setup, TUI scaffold, domain model bootstrap | Draft |
| [02-llm-provider-abstraction-implementation-plan.md](./02-llm-provider-abstraction-implementation-plan.md) | LLM provider traits, Anthropic/OpenAI/Ollama clients, streaming, cost tracking | Draft |
| [03-life-sheet-data-model-implementation-plan.md](./03-life-sheet-data-model-implementation-plan.md) | Life sheet YAML schema, serde types, SQLite mirror, ESCO tagging | Draft |
| [04-sqlite-persistence-implementation-plan.md](./04-sqlite-persistence-implementation-plan.md) | SQLite via rusqlite/sqlx, migration runner, repository traits, backup | Draft |
| [05-job-discovery-layer-implementation-plan.md](./05-job-discovery-layer-implementation-plan.md) | Job aggregation, dedup, enrichment pipeline, Greenhouse/Lever clients | Draft |
| [06-ralph-loop-integration-implementation-plan.md](./06-ralph-loop-integration-implementation-plan.md) | Ralph subprocess manager, JSON-stdio protocol, lifecycle, TUI panel | Draft |
| [07-resume-tailoring-pipeline-implementation-plan.md](./07-resume-tailoring-pipeline-implementation-plan.md) | Resume tailoring pipeline: JD parsing, gap analysis, DOCX generation, fabrication audit | Draft |
| [08-cover-letter-generation-implementation-plan.md](./08-cover-letter-generation-implementation-plan.md) | Cover letter generation: company research, multi-draft, version history | Draft |
| [09-tui-design-keybindings-implementation-plan.md](./09-tui-design-keybindings-implementation-plan.md) | ratatui widget hierarchy, vim-modal keybindings, event loop, overlay system | Draft |
| [10-application-workflow-implementation-plan.md](./10-application-workflow-implementation-plan.md) | Application state machine, kanban TUI, reminder poller, event sourcing | Draft |
| [11-platform-api-integrations-implementation-plan.md](./11-platform-api-integrations-implementation-plan.md) | Greenhouse, Lever, Workday clients; rate limiting; credential keyring storage | Draft |
| [12-15-interview-salary-networking-notifications-implementation-plan.md](./12-15-interview-salary-networking-notifications-implementation-plan.md) | Interview prep, salary intelligence, networking contacts, desktop notifications | Draft |
| [16-privacy-security-implementation-plan.md](./16-privacy-security-implementation-plan.md) | At-rest encryption (age), Argon2id master password, zeroize, privacy modes | Draft |
| [17-ralph-prompt-templates-implementation-plan.md](./17-ralph-prompt-templates-implementation-plan.md) | TOML prompt templates, variable interpolation, prompt caching, user overrides | Draft |
| [18-saas-migration-path-implementation-plan.md](./18-saas-migration-path-implementation-plan.md) | Feature flags, Supabase sync, axum API, JWT auth, multi-tenancy, plan gates | Draft |
| [20-openapi-mvp-implementation-plan.md](./20-openapi-mvp-implementation-plan.md) | End-to-end MVP implementation across 5 phases: workspace, core, TUI, Ralph, full pipeline | Draft |

### Agentic Ralph System

| File | Description | Status |
|------|-------------|--------|
| [agentic-llm-provider-abstraction-implementation-plan.md](./agentic-llm-provider-abstraction-implementation-plan.md) | lazyjob-llm crate: async provider traits, SSE streaming, embedding batching, cost table | Draft |
| [agentic-prompt-templates-implementation-plan.md](./agentic-prompt-templates-implementation-plan.md) | Product-layer prompt context structs, fabrication detection, mock interview turns | Draft |
| [agentic-ralph-orchestration-implementation-plan.md](./agentic-ralph-orchestration-implementation-plan.md) | LoopType priority queue, concurrency limits, cron scheduler, post-transition dispatch | Draft |
| [agentic-ralph-subprocess-protocol-implementation-plan.md](./agentic-ralph-subprocess-protocol-implementation-plan.md) | NDJSON codec, stdin writer / stdout reader tasks, cancel token, graceful shutdown | Draft |

### Application & Pipeline

| File | Description | Status |
|------|-------------|--------|
| [application-state-machine-implementation-plan.md](./application-state-machine-implementation-plan.md) | ApplicationStage enum, transition matrix, append-only log, broadcast events | Draft |
| [application-workflow-actions-implementation-plan.md](./application-workflow-actions-implementation-plan.md) | Apply/MoveStage/ScheduleInterview/LogContact workflows, ghost detector injection | Draft |
| [application-pipeline-metrics-implementation-plan.md](./application-pipeline-metrics-implementation-plan.md) | MetricsService SQL queries, ActionItem priority, stale-snooze logic, ratatui BarChart | Draft |

### Job Search

| File | Description | Status |
|------|-------------|--------|
| [job-search-discovery-engine-implementation-plan.md](./job-search-discovery-engine-implementation-plan.md) | JobSource trait, enrichment pipeline, SHA-256 change detection, cross-source dedup | Draft |
| [job-search-semantic-matching-implementation-plan.md](./job-search-semantic-matching-implementation-plan.md) | Embedder trait, cosine similarity, model-mismatch migration, feed score formula | Draft |
| [job-search-ghost-job-detection-implementation-plan.md](./job-search-ghost-job-detection-implementation-plan.md) | 7-signal ghost scorer, repost tracking, daily rescore loop, TUI badge + modal | Draft |
| [job-search-company-research-implementation-plan.md](./job-search-company-research-implementation-plan.md) | CompanyRecord canonical entity, normalizer, tech-stack lexicon, COALESCE upsert | Draft |

### Resume & Profile

| File | Description | Status |
|------|-------------|--------|
| [profile-life-sheet-data-model-implementation-plan.md](./profile-life-sheet-data-model-implementation-plan.md) | Deterministic IDs, SHA-256 import gating, is_grounded_claim, ESCO REST API tagging | Draft |
| [profile-resume-tailoring-implementation-plan.md](./profile-resume-tailoring-implementation-plan.md) | 6-stage pipeline, SkillNormalizer, FabricationLevel Ord enum, voice preservation | Draft |
| [profile-cover-letter-generation-implementation-plan.md](./profile-cover-letter-generation-implementation-plan.md) | 8-step orchestrator, TemplateSelector, ToneSelector, anti-fabrication numeric check | Draft |
| [profile-skills-gap-analysis-implementation-plan.md](./profile-skills-gap-analysis-implementation-plan.md) | SkillNormalizer, TechTermLexicon regex, GapMatrix, cache keyed by SHA-256 | Draft |

### Interview Preparation

| File | Description | Status |
|------|-------------|--------|
| [interview-prep-question-generation-implementation-plan.md](./interview-prep-question-generation-implementation-plan.md) | PrepContextBuilder, QuestionMix by interview type, seniority inference, TUI 60/40 split | Draft |
| [interview-prep-mock-loop-implementation-plan.md](./interview-prep-mock-loop-implementation-plan.md) | Bidirectional Ralph loop, per-question LLM eval, STAR feedback, TOML cached prompts | Draft |
| [interview-prep-agentic-implementation-plan.md](./interview-prep-agentic-implementation-plan.md) | InterviewDossierLoop, StarBankExtractionLoop, readiness score, prep checkpoints | Draft |

### Salary & Compensation

| File | Description | Status |
|------|-------------|--------|
| [salary-market-intelligence-implementation-plan.md](./salary-market-intelligence-implementation-plan.md) | i64 cents model, H1B XLSX importer, LevelsFyiParser state machine, FTS5 fuzzy match | Draft |
| [salary-negotiation-offers-implementation-plan.md](./salary-negotiation-offers-implementation-plan.md) | compute_batna, rank_offers, CounterOfferLoop, 3-view TUI (form/comparison/panel) | Draft |
| [salary-counter-offer-drafting-implementation-plan.md](./salary-counter-offer-drafting-implementation-plan.md) | build_context pure fn, 3 TOML tone templates, round-limit warning, draft-only UI | Draft |

### Networking & Outreach

| File | Description | Status |
|------|-------------|--------|
| [networking-connection-mapping-implementation-plan.md](./networking-connection-mapping-implementation-plan.md) | ConnectionTier enum, LinkedIn CSV importer, classify_approach exhaustive match | Draft |
| [networking-outreach-drafting-implementation-plan.md](./networking-outreach-drafting-implementation-plan.md) | SharedContextBuilder, 4 TOML tone templates, MediumLengthEnforcer, fabrication check | Draft |
| [networking-referral-management-implementation-plan.md](./networking-referral-management-implementation-plan.md) | RelationshipStage Ord, ReferralReadinessChecker 5-gate ordered check, rolling window SQL | Draft |
| [networking-referrals-agentic-implementation-plan.md](./networking-referrals-agentic-implementation-plan.md) | WarmPathFinderLoop, OutreachBriefLoop, RelationshipHealthLoop, alumni inference | Draft |

### Gap Analysis Plans

| File | Description | Status |
|------|-------------|--------|
| [01-gaps-core-architecture-implementation-plan.md](./01-gaps-core-architecture-implementation-plan.md) | Gap Analysis: Core Architecture — 15 new component specs | Draft |
| [02-gaps-job-discovery-implementation-plan.md](./02-gaps-job-discovery-implementation-plan.md) | Gap Analysis: Job Discovery — discovery engine, semantic matching, ghost detection | Draft |
| [03-gaps-ralph-ai-implementation-plan.md](./03-gaps-ralph-ai-implementation-plan.md) | Gap Analysis: Ralph AI — 12 gaps across orchestration, protocol, prompts | Draft |
| [04-gaps-resume-profile-implementation-plan.md](./04-gaps-resume-profile-implementation-plan.md) | Gap Analysis: Resume/Profile — GAP-39 to GAP-48 | Draft |
| [05-gaps-cover-letter-interview-implementation-plan.md](./05-gaps-cover-letter-interview-implementation-plan.md) | Gap Analysis: Cover Letter & Interview — GAP-49 to GAP-58 + cross-spec K/L/M | Draft |
| [06-gaps-application-workflow-implementation-plan.md](./06-gaps-application-workflow-implementation-plan.md) | Gap Analysis: Application Workflow — GAP-59 to GAP-68 | Draft |
| [07-gaps-networking-outreach-implementation-plan.md](./07-gaps-networking-outreach-implementation-plan.md) | Gap Analysis: Networking & Outreach — GAP-69 to GAP-77 + cross-spec P/Q | Draft |
| [08-gaps-salary-tui-implementation-plan.md](./08-gaps-salary-tui-implementation-plan.md) | Gap Analysis: Salary & TUI — GAP-78 to GAP-87 + cross-spec R/S | Draft |
| [09-gaps-platform-privacy-implementation-plan.md](./09-gaps-platform-privacy-implementation-plan.md) | Gap Analysis: Platform & Privacy — GAP-89 to GAP-98 + cross-spec T/U | Draft |
| [10-gaps-saas-mvp-implementation-plan.md](./10-gaps-saas-mvp-implementation-plan.md) | Gap Analysis: SaaS & MVP — GAP-99 to GAP-109 + cross-spec V/W | Draft |

### Extended Features

| File | Description | Status |
|------|-------------|--------|
| [XX-master-password-app-unlock-implementation-plan.md](./XX-master-password-app-unlock-implementation-plan.md) | Argon2id KDF, Session (Zeroizing key), UnlockFlow lockout, biometric (macOS) | Draft |
| [XX-encrypted-backup-export-implementation-plan.md](./XX-encrypted-backup-export-implementation-plan.md) | age-encrypted .tar.gz backup, rusqlite online backup API, BackupScheduler, restore flow | Draft |
| [XX-application-cross-source-deduplication-implementation-plan.md](./XX-application-cross-source-deduplication-implementation-plan.md) | JobFingerprint, SimilarityBreakdown, greedy dedup engine, TUI review view | Draft |
| [XX-cover-letter-version-management-implementation-plan.md](./XX-cover-letter-version-management-implementation-plan.md) | VersionService, content_hash UNIQUE, similar-crate diff, CleanupExecutor policies | Draft |
| [XX-tui-vim-mode-implementation-plan.md](./XX-tui-vim-mode-implementation-plan.md) | Pure VimState FSM, VimAction output enum, RegisterBank, MacroRecorder, unicode-seg | Draft |
| [XX-ralph-process-orphan-cleanup-implementation-plan.md](./XX-ralph-process-orphan-cleanup-implementation-plan.md) | PID tracking, sysinfo orphan scan, SIGTERM→SIGKILL escalation, StartupLock (fs2) | Draft |
| [XX-job-alert-webhooks-implementation-plan.md](./XX-job-alert-webhooks-implementation-plan.md) | Greenhouse/Lever webhook handlers, HMAC-SHA256 verify, SQLite retry queue, IMAP watcher | Draft |
| [XX-llm-cost-budget-management-implementation-plan.md](./XX-llm-cost-budget-management-implementation-plan.md) | Microdollar cost table, BudgetEnforcer, idempotent threshold alerts, CostPill widget | Draft |
| [XX-llm-prompt-versioning-implementation-plan.md](./XX-llm-prompt-versioning-implementation-plan.md) | prompt_registry.toml, minijinja rendering, jsonschema validation, Jaccard similarity | Draft |
| [XX-multi-offer-comparison-implementation-plan.md](./XX-multi-offer-comparison-implementation-plan.md) | ComparableOffer, MinMaxTable normalization, scenario preview, ExpiryUrgency, dynamic table | Draft |
| [XX-resume-version-management-implementation-plan.md](./XX-resume-version-management-implementation-plan.md) | version_number/status columns, partial unique index for pinning, section-level diff | Draft |
| [XX-ralph-ipc-protocol-implementation-plan.md](./XX-ralph-ipc-protocol-implementation-plan.md) | Unix domain socket, LengthDelimitedCodec, correlated replies (oneshot), push events (broadcast) | Draft |
| [XX-tui-accessibility-implementation-plan.md](./XX-tui-accessibility-implementation-plan.md) | SemanticColor + ThemePalette, color-blind palettes, StatusSymbol shapes, FocusRing | Draft |
| [XX-authenticated-job-sources-implementation-plan.md](./XX-authenticated-job-sources-implementation-plan.md) | Cookie import, AES-256-GCM credential store, Voyager API, PerSourceRateLimiter (governor) | Draft |
| [XX-interview-session-resumability-implementation-plan.md](./XX-interview-session-resumability-implementation-plan.md) | MockInterviewService, auto-save (per-Q + 5-min tick), inactivity watch, PartialSessionBanner | Draft |
| [XX-contact-multi-source-import-implementation-plan.md](./XX-contact-multi-source-import-implementation-plan.md) | ContactParser trait, vCard/CSV/Gmail/Apple Contacts/business card vision importers | Draft |

---

## Contributing

When adding or modifying specs:
1. Use `XX-name.md` numbering for ordering (e.g., `21-new-feature.md`)
2. Include `## Status` header (Researching/Draft/Implemented/Complete)
3. Include `## Problem Statement` section
4. Link related specs at the bottom