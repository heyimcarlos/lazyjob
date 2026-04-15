# LazyJob Specifications

This directory contains the architectural specifications and design documents for LazyJob — a terminal-based job search command center powered by AI agents.

## Quick Navigation

| Spec | Topic | Status |
|------|-------|--------|
| [01-architecture.md](./01-architecture.md) | Crate layout, TUI structure, domain models | Researching |
| [02-llm-provider-abstraction.md](./02-llm-provider-abstraction.md) | Multi-provider LLM support (Anthropic, OpenAI, Ollama) | Researching |
| [03-life-sheet-data-model.md](./03-life-sheet-data-model.md) | YAML schema + SQLite model for job seeker profile | Researching |
| [04-sqlite-persistence.md](./04-sqlite-persistence.md) | SQLite with rusqlite/sqlx, migrations, backup | Researching |
| [05-job-discovery-layer.md](./05-job-discovery-layer.md) | Greenhouse/Lever API, semantic matching, embeddings | Researching |
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

## Research Documents

Additional market and feature research:

| Document | Topic |
|----------|-------|
| `job-platforms-comparison.md` | Job board feature comparison |
| `networking-referrals-agentic.md` | Agentic networking strategies |
| `agentic-job-matching.md` | AI job matching approaches |
| `premium-monetization.md` | SaaS pricing tiers |
| `competitor-analysis.md` | (see 19-competitor-analysis.md) |

## Status Legend

- **Researching** — Gathering information, evaluating options
- **Draft** — Initial design complete, needs review
- **Implemented** — Code exists in repository
- **Complete** — Fully implemented and tested

## Contributing

When adding or modifying specs:
1. Use `XX-name.md` numbering for ordering (e.g., `21-new-feature.md`)
2. Include `## Status` header (Researching/Draft/Implemented/Complete)
3. Include `## Problem Statement` section
4. Link related specs at the bottom
