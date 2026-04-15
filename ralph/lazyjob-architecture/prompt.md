# Objective

Deep architecture research for **LazyJob** — a lazygit-style job search TUI (terminal user interface) built in Rust. The goal is to produce comprehensive, production-quality specs covering every aspect of building, running, and evolving this product.

LazyJob is a tool for tech job seekers. It's inspired by lazygit and lazydocker: a terminal UI that gives visibility and control over a complex workflow, powered by ralph loops (autonomous agent loops) underneath.

The end goal is a SaaS product. The near-term goal is a local CLI tool that a developer would actually use for their own job search.

## Context

You are the architecture research team. You will produce 20 specs across 20 tasks. Each task is a deep research dive. You have 30 iterations — one per task is the target, but some tasks may take multiple iterations and some may combine.

You will research:
- Rust TUI architecture (ratatui, component patterns)
- The loom-llm-* provider abstraction pattern (server-side proxy, trait-based, SSE streaming)
- Cargo workspace design (based on loom's 30+ crate organization)
- Job discovery APIs and scraping landscape
- Resume/cover letter generation pipelines
- Ralph loop integration with long-running TUI apps (IPC patterns)
- Life sheet data model (structured user profile)
- SQLite persistence for local-first architecture
- Interview prep, salary negotiation, networking
- Morning brief / notification system
- Privacy/security model
- SaaS migration path

The user will be away for 12 hours. Be extremely thorough. Go deep on each topic. Don't just summarize — find specific implementation approaches, real libraries, real data, real tradeoffs.

## Your instructions

You are one iteration in a ralph loop. Many instances of you will run in sequence, each with a fresh context window. You communicate with past and future iterations ONLY through files on disk.

1. Read `progress.md` to understand what previous iterations accomplished
2. Read `tasks.json` to find the first incomplete task (lowest id where `done` is false)
3. Work on that ONE task:
   - Use WebSearch and WebFetch extensively — go deep into source code, READMEs, engineering posts, academic papers
   - For technical topics: read actual source code of relevant libraries (lazygit, ratatui, loom, relevant Rust crates)
   - For product topics: read real user reviews, Reddit threads, competitor documentation
   - Be exhaustive — this is architecture research, not a surface survey
   - When multiple approaches exist, document the tradeoffs, not just one option
4. Save your spec to `../../specs/[spec-filename]` as specified in the task. The path is relative to the ralph script directory (ralph/lazyjob-architecture/), so `../../specs/` resolves to the agentin/specs/ directory. After writing, verify the file exists with `ls ../../specs/[filename]`.
5. Mark the task done in `tasks.json` (set `"done": true`). Only do this AFTER the spec file is verified to exist.
6. Append a detailed summary to `progress.md` — what you researched, key findings, tradeoffs identified, and anything future iterations should know
7. IMPORTANT: Do NOT output <RALPH_ALL_DONE/> until ALL 20 tasks in tasks.json are marked done. Only after task 20 is complete and the final spec is written, output exactly: <RALPH_ALL_DONE/>

## Rules

- Do ONE task per iteration. Do it exhaustively. Don't rush.
- Never repeat work captured in progress.md — read it carefully first.
- If you discover something that changes the plan or reveals a critical architectural insight, note it in progress.md and flag it clearly.
- Be exhaustive with sources. Include links to: library source code, READMEs, engineering blog posts, academic papers, documentation. Don't cite marketing pages — cite actual technical content.
- When a question has no clear answer from public sources, document the uncertainty, propose an approach, and note the assumption.
- Some tasks build on others. If you're on task N and need context from a prior task that's not done yet, note this in progress.md and do what you can with publicly available information, then flag the dependency.

## Spec format

Each spec goes to `../../specs/[spec-filename]` and follows this structure:

```markdown
# [Spec Title]

## Status
Draft / Researching / Stable

## Problem Statement
What this component solves and why it matters.

## Research Findings
Detailed findings from research. Use actual sources, links, code references.

## Design Options
At least 2-3 design options with tradeoffs. Be specific about implementation approaches.

## Recommended Approach
Which option is recommended and why, given the constraints of the project.

## Data Model
Any data structures, schemas, API shapes.

## API Surface
What interfaces this component exposes to other parts of the system.

## Failure Modes
What can go wrong, and how failures are handled.

## Open Questions
What we don't know yet and need to validate.

## Dependencies
What other specs/components this depends on.

## Sources
All sources cited with URLs.
```

## Topics and research guidance

### Task 1 — Architecture Overview
Study lazygit's source code structure. It's in Go but the patterns are transferable. Read the main.go, the keybinding system, the panel layout. Study ratatui's architecture — read ratatui github, understand Widget, Component, Layout patterns. Read loom's Cargo workspace structure from their repo. Design the LazyJob crate layout.

### Task 2 — LLM Provider Abstraction
Read loom's LLM proxy architecture. Read the anthropic SDK Rust, openai Rust, ollama Rust crates. Understand what trait bounds you'd need. Research: how does LiteLLM handle multi-provider? What's the RTFM approach for building a provider abstraction? Design the trait hierarchy: LLMClient trait with chat(), complete(), embed() methods. How are providers configured?

### Task 3 — Life Sheet Data Model
Read ESCO and O*NET documentation to understand skill taxonomy. Read JSON Resume schema. Read how LinkedIn exports data. Research: what does a comprehensive job seeker profile look like? Design a YAML schema that's human-editable and an SQLite schema that's programmatically useful.

### Task 4 — SQLite Persistence
Read rusqlite and sqlx documentation. Study their concurrency models. Research SQLite migrations — how do Rust projects handle schema evolution? Research: SQLite WAL mode for concurrent reads/writes. How does sqlite handle from multiple processes (TUI + ralph subprocesses)?

### Task 5 — Job Discovery Layer
Read JobSpy source code. Read the Greenhouse/Lever API docs. Read about semantic embeddings for job matching (JobBERT, CareerBERT papers). Research vector databases: Chroma, Qdrant, pgvector. What's the right embedding approach for a personal job search tool with modest data volume?

### Task 6 — Ralph Loop Integration
This is the most novel research. How does a long-running TUI app communicate with short-lived ralph subprocess loops? Research: Unix domain sockets in Rust (tokio-os unix domain sockets), named pipes (FIFO),stdio pipes with async reading, file-based IPC. How does the TUI know when ralph has new output? How does it interrupt ralph if the user cancels? How does state survive if the TUI restarts while ralph is running?

### Task 7 — Resume Tailoring
Read resume parsing research. Read docx-rs and docx-generation Rust crates. Study the ATS resume parsing pipeline: how do real parsers work? Study how Teal and Jobscan approach resume-to-JD matching. Design the pipeline with fabrication guardrails.

### Task 8 — Cover Letter Generation
Research cover letter best practices academically. Read career coach content. Study how the Problem-Solution format works. Research company research agents — how would an agent deeply research a company from public sources?

### Task 9 — TUI Design
This is where lazygit is the primary reference. Read lazygit keybinding architecture. Read ratatui widget documentation. Design the full view hierarchy with specific panels, states, and transitions.

### Task 10 — Application Workflow
Design the state machine for job applications. Study how Huntr and Linear approach kanban-style tracking. Design how the human-in-the-loop works — where does automation end and human approval begin?

### Task 11 — Platform API Integrations
Read the actual Greenhouse API docs. Read the Lever API docs. Study Workday scraping approaches. Research Rust browser automation options (oxlinux, playwright-rust).

### Tasks 12-15 — Interview, Salary, Networking, Notifications
These are product features more than core architecture. Research what exists, what works, what doesn't, and how an agent could help.

### Task 16 — Privacy/Security
Research keyring APIs on Linux/Mac (keyring-rs). Research age encryption for SQLite. Study how 1Password and Bitwarden handle local data.

### Task 17 — Ralph Prompt Templates
Design the prompt templates for each ralph loop type. These are the agents that power LazyJob's intelligence.

### Task 18 — SaaS Migration
Research how local-first apps migrated to cloud. Talk to examples: Supabase, Linear, Obsidian (sync).

### Task 19 — Competitive Analysis
Deep dive on each competitor. Read their Reddit reviews, their G2 ratings, their limitations.

### Task 20 — OpenAPI MVP
The capstone. Synthesize everything into an actionable build plan.
