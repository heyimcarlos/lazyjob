# Objective

Ultra-analyze all LazyJob specs in /home/lab-admin/repos/lazyjob/specs/ to find gaps, missing topics, and things that need deeper research before implementation. For every gap found, create a new spec and research it deeply.

## Context

This is a gap-finding mission. Every spec in specs/ was written with a certain scope. Your job is to look at each spec with fresh eyes and ask:
- What's mentioned but not deeply specced?
- What implementation details are glossed over?
- What could go wrong during implementation that isn't warned about?
- What is completely missing that a job seeker would actually need?
- What would a competitor do better?
- What would make this feature actually great vs just functional?

This is an **ultra-think** exercise. Don't surface-level critique. Go deep. Challenge assumptions.

## Your instructions

You are one iteration in a ralph loop. Many instances of you will run in sequence, each with a fresh context window. You communicate with past and future iterations ONLY through files on disk.

1. Read `progress.md` to understand what previous iterations accomplished
2. Read `tasks.json` to find the highest-priority incomplete task
3. Work on that ONE task:
   - Read ALL the relevant spec files mentioned in the task
   - For each spec, ultra-think: what gaps exist? What's missing?
   - Research using WebSearch and WebFetch to understand what best-in-class solutions look like
   - Write a gap analysis documenting every missing topic
   - For any gap that warrants a full spec (not just a note), create a proper spec file in /home/lab-admin/repos/lazyjob/specs/ with name format XX-gap-name.md
   - Be specific: don't write "needs error handling" — write "needs retry logic with exponential backoff for Greenhouse API rate limits"
4. Mark the task done in `tasks.json` (set `"done": true`)
5. Append a detailed summary to `progress.md` — all gaps found, all specs created, what the next iteration should focus on
6. If ALL tasks in tasks.json are done, output: <promise>COMPLETE</promise>

## Rules

- Do ONE task per iteration. Do it thoroughly. Don't rush to do multiple.
- Never repeat work captured in progress.md — read it carefully first.
- If you discover something that changes the plan (new tasks needed, a task is irrelevant, scope changed), update tasks.json accordingly.
- Be thorough — you have a full context window. Use it.
- Web research is REQUIRED for gap analysis. Don't just theorize — look at how real tools solve these problems.
- Save ALL findings to files. Your memory dies when you exit.
- When creating new specs, write them to /home/lab-admin/repos/lazyjob/specs/XX-[gap-name].md following the format of existing specs (## Status, ## Problem Statement, ## Design Decision, etc.)

## Gap analysis output format

Each gap analysis file should contain:
- List of all specs reviewed
- For each spec: what's well-covered, what's missing
- Cross-spec gaps (issues that span multiple specs)
- New specs to create for each gap
- Priority: critical / important / nice-to-have

## New spec format

When creating a new spec, follow the LazyJob spec format:
```markdown
# [Spec Name]

## Status
Researching

## Problem Statement
[What problem does this solve?]

## Solution Overview
[What should LazyJob do?]

## Design Decisions
[Key architectural choices]

## Implementation Notes
[How to build it]

## Open Questions
[What needs more research]

## Related Specs
[Links to related specs]
```