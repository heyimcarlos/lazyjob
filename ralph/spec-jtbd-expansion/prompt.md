# Objective

Expand the lazyjob project's existing specs into a proper JTBD hierarchy:
**JTBD → Topics of Concern → Specs → Implementation Tasks**

LazyJob is a lazygit-style terminal UI (Rust + Ratatui) for autonomous AI-powered job search, built using the ralph loop methodology. The existing `specs/` directory has 36+ markdown research documents. Your job is to synthesize, organize, and expand those into well-scoped specs anchored to concrete Jobs To Be Done.

## Context

LazyJob runs ralph loops in the background for AI-powered job tasks (discovery, resume tailoring, cover letters, networking, interview prep). The TUI is the human control plane; ralph subprocesses are the AI work plane. The LLM integration follows the loom pattern: server-side proxy, provider-agnostic trait, SSE streaming.

The hierarchy you're building toward:
- **JTBD**: High-level outcome a user wants to achieve (e.g., "Find relevant jobs without wasting time")
- **Topic of Concern**: One distinct aspect of a JTBD — passes the scope test: expressible in ONE sentence WITHOUT the word "and"
- **Spec**: One markdown file per topic of concern — covers what/why/how/design-decisions/open-questions
- **Task**: One bullet in IMPLEMENTATION_PLAN.md — one concrete implementation unit derived from a spec

## Your Instructions

You are one iteration in a ralph loop. Many instances of you run in sequence, each with a fresh context window. You communicate with past and future iterations ONLY through files on disk.

1. **Read `ralph/spec-jtbd-expansion/progress.md`** to understand what previous iterations did.
2. **Read `ralph/spec-jtbd-expansion/tasks.json`** to find the first incomplete task (`"done": false`).
3. **Work on that ONE task thoroughly.** Use the task description as your guide.

### Per-task instructions:

**For task 1 (JTBD extraction):**
- Use up to 10 parallel subagents to read all existing specs in `specs/`
- Identify distinct user audiences (job seeker, power user, recruiter-facing features)
- Extract JTBDs: what outcome does each audience need? Apply the one-sentence test
- Decompose each JTBD into verb-form activities (upload resume, search jobs, track applications, etc.)
- Write `ralph/spec-jtbd-expansion/output/AUDIENCE_JTBD.md` following this structure:
  ```
  ## Audiences
  ### [Audience Name]
  #### JTBD: [one-sentence outcome]
  Activities: [verb, verb, verb, ...]
  ```

**For task 2 (spec inventory):**
- Read all existing specs + `output/AUDIENCE_JTBD.md`
- Map each existing spec file to a JTBD domain
- Identify: overlapping specs (should merge), topics referenced in research but no dedicated spec exists, specs that cover multiple concerns (should split)
- Propose a clean file structure for `output/specs/` grouped by domain
- Write `ralph/spec-jtbd-expansion/output/spec-inventory.md`

**For tasks 3–11 (spec expansion by domain):**
- Read `output/AUDIENCE_JTBD.md` and `output/spec-inventory.md` first
- Read the relevant existing specs listed in the task description using parallel subagents
- For each topic of concern in this domain, write a spec file to `ralph/spec-jtbd-expansion/output/specs/[domain]-[topic].md`
- Each spec must:
  - Open with: the JTBD it serves + the topic of concern (one sentence, passes scope test)
  - **What**: What this component does
  - **Why**: Why it exists, what user pain it solves
  - **How**: Architecture, data flow, key design decisions, tradeoffs
  - **Interface**: API/trait/struct signatures (Rust) where applicable
  - **Open Questions**: Unresolved decisions that need human input
  - **Implementation Tasks**: 3–8 concrete bullet-point tasks that directly feed IMPLEMENTATION_PLAN.md
  - Be specific — reference file paths, crate names, existing patterns from the research specs

**For task 12 (IMPLEMENTATION_PLAN.md):**
- Read all spec files from `ralph/spec-jtbd-expansion/output/specs/` using parallel subagents
- Read `ralph/spec-jtbd-expansion/output/AUDIENCE_JTBD.md`
- Collect all "Implementation Tasks" sections from each spec
- Synthesize into a single prioritized bullet list at `/home/ren/repos/lazyjob/IMPLEMENTATION_PLAN.md`
- Ordering: (1) crate scaffolding + data model + SQLite, (2) TUI skeleton, (3) LLM provider abstraction, (4) core features (search, profile, application tracking), (5) agentic/ralph features, (6) platform integrations, (7) premium/SaaS
- Each bullet: `- [ ] [action verb] [what] — refs: [spec filename]`

4. **Save your output** to the path specified in the task description.
5. **Mark the task done** in `ralph/spec-jtbd-expansion/tasks.json` — set `"done": true` for the completed task.
6. **Append a concise summary to `ralph/spec-jtbd-expansion/progress.md`**:
   ```
   ## Iteration N — Task [id]: [name]
   - What I produced: [file paths]
   - Key findings: [2-3 bullet points]
   - What next iteration should know: [anything that changes the plan or surfaces surprises]
   ```
7. **If ALL tasks are done**, output: `<promise>COMPLETE</promise>`

## Rules

- Do ONE task per iteration. Do it thoroughly. Don't skip ahead.
- Read `progress.md` first every time — never repeat work already done.
- Apply the scope test ruthlessly: if you need "and" to describe a topic, split it into two specs.
- Never write placeholder specs. Each spec must be actionable — a developer could implement from it.
- Use parallel subagents (up to 10) to read multiple existing specs simultaneously.
- If you discover a new topic of concern while working, add a task to `tasks.json` and note it in `progress.md`.
- If an existing task is irrelevant or already covered, mark it done and explain in `progress.md`.
- The existing specs are research artifacts — they contain valuable findings but are not organized around JTBDs. Synthesize their content; don't just copy it.

## Output Format for Spec Files

```markdown
# Spec: [Topic of Concern]

**JTBD**: [The high-level user outcome this serves]
**Topic**: [One-sentence description of this spec's scope — no "and"]
**Domain**: [job-search | profile-resume | application-tracking | networking | interview-salary | agentic | platform-integrations | architecture | saas]

---

## What

[What this component does — 2-4 sentences]

## Why

[User pain solved, why it matters, what's broken without it]

## How

[Architecture, data flow, key design decisions, tradeoffs]
[Include Rust types/traits/structs where applicable]
[Reference crate paths: lazyjob-[crate]/src/...]

## Interface

```rust
// Key types, traits, or API signatures
```

## Open Questions

- [Unresolved decision that needs human input]

## Implementation Tasks

- [ ] [Concrete action] — e.g., "Implement LlmClient trait with AnthropicClient and OpenAIClient variants"
- [ ] ...
```

## Working Directory

All file paths are relative to `/home/ren/repos/lazyjob/`.
Existing specs: `specs/*.md`
Output: `ralph/spec-jtbd-expansion/output/`
Final plan: `IMPLEMENTATION_PLAN.md`
