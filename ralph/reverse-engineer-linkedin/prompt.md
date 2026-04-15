# Objective

Reverse-engineer LinkedIn, job search platforms, and X.com's professional features. For each major feature, produce a detailed research spec that documents how it works, why it works, and what makes it successful.

## Context

We're building a professional platform for AI agents — think "LinkedIn for AI Agents." Before designing anything, we need to deeply understand what LinkedIn and competing platforms actually do. This isn't about copying LinkedIn — it's about understanding the design decisions, mechanics, and network effects that make professional platforms work, so we can adapt the right ideas for an agent-centric world.

## Your instructions

You are one iteration in a ralph loop. Many instances of you will run in sequence, each with a fresh context window. You communicate with past and future iterations ONLY through files on disk.

1. Read `progress.md` to understand what previous iterations accomplished
2. Read `tasks.json` to find the first incomplete task (lowest id where `done` is false)
3. Work on that ONE task:
   - Use WebSearch and WebFetch extensively to research the feature
   - Study the target systematically — how it works for users, how it likely works technically, what design decisions were made and why
   - Look for engineering blog posts, patents, leaked internal docs, public API documentation, teardowns by analysts
   - Document: how it works, why it works that way, what makes it successful, what's weak or missing
   - Include concrete examples, not just abstractions
   - Compare to how competitors handle the same feature where relevant
4. Save your research spec to the path specified in the task description (under `../../specs/`)
5. Mark the task done in `tasks.json` (set `"done": true`)
6. Append a concise summary to `progress.md` — what you researched, key findings, and anything the next iteration should know
7. If ALL tasks in tasks.json are done, output: <promise>COMPLETE</promise>

## Rules

- Do ONE task per iteration. Do it thoroughly. Don't rush to do multiple.
- Never repeat work captured in progress.md — read it carefully first.
- If you discover something that changes the plan (a feature you hadn't considered, a task that should be split, something irrelevant), update tasks.json. Add new tasks, remove obsolete ones. Note the change in progress.md.
- Be thorough — you have a full context window. Use it. Each spec should be comprehensive.
- Save ALL findings to files. Your memory dies when you exit.
- When researching, prioritize primary sources (LinkedIn's own engineering blog, official help docs, API docs) over secondhand summaries.

## Research spec format

Each spec should follow this structure and be saved to `../../specs/[feature-name].md`:

```markdown
# [Feature Name]

## What it is
One-paragraph summary of the feature and its role in the platform.

## How it works — User perspective
Walk through the feature as a user experiences it. Screens, flows, interactions.

## How it works — Technical perspective
Architecture, algorithms, data models, infrastructure (as much as can be determined from public information). Reference engineering blog posts and patents where available.

## What makes it successful
The design decisions, network effects, behavioral hooks, or technical innovations that make this feature work well. Why users engage with it.

## Weaknesses and gaps
What's missing, broken, or poorly designed. Common user complaints. Opportunities a competitor could exploit.

## Competitive landscape
How other platforms handle the same need. What they do differently or better.

## Relevance to agent platforms
Initial thoughts on how this feature concept could translate to a platform for AI agents. What transfers directly, what needs reimagining, what's irrelevant.

## Sources
Links to engineering blogs, patents, documentation, analysis used in research.
```
