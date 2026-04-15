# Objective

Research the complete job-seeking workflow — from "I need a job" to "I accepted an offer" — and how AI agents can transform every step. This is NOT about building a social media platform. This is about the practical, tactical reality of getting a job in tech, and how agents change the game.

## Context

A parallel ralph loop is researching LinkedIn's platform features (profiles, feed, network graph, etc.) and writing specs to `../../ralph/specs/` and `../../specs/`. That research covers the *platform* side. THIS loop covers the *job-seeker* side: the actual workflows, tools, pain points, and opportunities for agentic automation.

The end goal is building a product that helps people get jobs using AI agents. Not a social network — a job-getting machine. We need to deeply understand:
- How people actually find and get jobs today (the messy reality, not the idealized version)
- Where agents can add real value vs where they'd be theater
- How agents would technically interface with existing platforms
- The full workflow: search, apply, resume, cover letter, network, interview, negotiate

## Your instructions

You are one iteration in a ralph loop. Fresh context each time. You communicate with other iterations ONLY through files on disk.

1. Read `progress.md` to see what previous iterations found
2. Read `tasks.json` to find the first incomplete task (lowest id where `done` is false)
3. Work on that ONE task:
   - Use WebSearch and WebFetch extensively — go deep, not broad
   - Search for Reddit threads, Hacker News discussions, career coach content, real user experiences — not just corporate marketing
   - Look for data: success rates, conversion rates, time spent, cost
   - Find existing tools and products in each space — what's been tried, what works, what failed
   - Be skeptical of conventional wisdom — question whether "best practices" actually work
   - When researching agentic approaches, think concretely: what API calls, what data, what UX, what failure modes
4. Save your spec to the path in the task description (under `../../specs/`)
5. Mark the task done in `tasks.json`
6. Append findings summary to `progress.md`
7. If ALL tasks done, output: <promise>COMPLETE</promise>

## Rules

- ONE task per iteration. Go deep.
- Read progress.md first — don't repeat prior work.
- If you find that a task should be split, or a new task is needed, update tasks.json and note it in progress.md.
- Prioritize primary sources: actual job seekers' experiences, hiring manager interviews, recruiter tool documentation, published research with sample sizes.
- When you find conflicting information, document both sides with sources.
- For the critique task (#10), read the first ralph's specs and be genuinely critical — don't just validate what's already written.

## Spec format

Each spec goes to `../../specs/[name].md`:

```markdown
# [Topic]

## The reality today
How this actually works right now. Real numbers, real workflows, real pain points. Not the idealized version.

## What tools and products exist
Current solutions, their strengths, limitations, pricing, user reception.

## The agentic opportunity
What an AI agent could concretely do here. Be specific:
- What inputs does the agent need?
- What actions does it take?
- What APIs/data sources does it use?
- What does the human still need to do?
- What are the failure modes and risks?

## Technical considerations
APIs available, legal/ToS constraints, data access challenges, automation detection.

## Open questions
What we still don't know and need to figure out.

## Sources
URLs, studies, data points referenced.
```
