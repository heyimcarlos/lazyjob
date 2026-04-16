# Gap Analysis: Core Architecture (01-04 specs)

## Specs Reviewed
- `01-architecture.md` - LazyJob Architecture Overview
- `02-llm-provider-abstraction.md` - LLM Provider Abstraction
- `03-life-sheet-data-model.md` - Life Sheet Data Model
- `04-sqlite-persistence.md` - SQLite Persistence Layer

---

## What's Well-Covered

### 01-architecture.md
- Crate organization (lazyjob-core, lazyjob-llm, lazyjob-tui, lazyjob-ralph, lazyjob-cli)
- Ratatui architecture patterns
- Lazygit-inspired keybinding philosophy
- High-level data model (Job, Company, Application, LifeSheet, Contact)
- View hierarchy (Dashboard, Jobs List, Job Detail, Search, Applications, Contacts, Settings, Help)
- Failure modes (LLM Provider Failure, Ralph Subprocess Crash, SQLite Corruption, Terminal Resize, etc.)

### 02-llm-provider-abstraction.md
- Trait-based LLM provider design (LLMProvider trait)
- Message types (ChatMessage, ChatResponse, ChatStreamChunk, TokenUsage)
- Provider configurations (AnthropicConfig, OpenAIConfig, OllamaConfig)
- Provider registry pattern
- Builder pattern for setup
- Streaming in TUI context with cancellation
- Failure modes (Auth, Rate Limited, Model Not Found, Context Length, etc.)
- Key open questions: Embedding provider strategy, cost tracking, caching, rate limits, batching

### 03-life-sheet-data-model.md
- Rich domain model design (recommended approach)
- YAML schema with personal info, experience, achievements, education, skills, certifications, languages, projects, preferences, goals, contact network
- ESCO and O*NET taxonomy mapping
- SQLite data model (separate from YAML format)
- Conversion between YAML and SQLite
- Open questions: YAML validation, resume versioning, LinkedIn import, GitHub integration, partial updates

### 04-sqlite-persistence.md
- sqlx with SQLite recommendation (Option B)
- Database schema (jobs, companies, applications, contacts, interviews, offers, reminders, activity_log tables)
- Connection management with pool settings
- Repository pattern for data access
- Migration files structure
- Backup strategy with auto-backup on startup
- Ralph subprocess database access options

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-1: Prompt Versioning, Testing, and Rollback (CRITICAL)

**Location**: `02-llm-provider-abstraction.md` - only mentions prompts in module hierarchy, no spec for how to manage them

**What's missing**:
1. **Prompt versioning**: How are prompt templates stored, versioned, and rolled back? What happens when a prompt update produces worse output?
2. **Prompt testing/validation**: How do we test prompts before deploying? What metrics define "good enough"?
3. **Prompt A/B testing**: How to compare two prompt versions against real outputs?
4. **Prompt diff**: How to visualize changes between prompt versions?
5. **Structured output validation**: The spec shows `ChatStreamChunk` but doesn't address JSON mode / structured outputs that ralph loops would need for tool calling
6. **Prompt variables/slotting**: How are dynamic values (company name, job title, etc.) injected into prompts?

**Why critical**: Ralph loops run autonomously. If prompts degrade, the agent produces bad outputs silently. There's no way to recover without prompt versioning.

**What could go wrong**:
- Prompt drift: over time, prompts are modified incrementally without tracking
- Silent failures: a bad prompt change produces subtly wrong outputs that aren't caught
- No rollback: when a bad prompt is deployed, must manually rewrite
- Context window waste: poorly structured prompts consume excess tokens

---

### GAP-2: LLM Cost Tracking, Budget Limits, and Usage Attribution (CRITICAL)

**Location**: `02-llm-provider-abstraction.md` - Open Question #2 mentions cost tracking but no design

**What's missing**:
1. **Per-request cost calculation**: How are token costs calculated and totaled?
2. **Budget limits**: What happens when user sets a $10/month budget and hits it?
3. **Usage attribution**: Which ralph loop triggered which LLM call? For debugging and optimization
4. **Provider cost comparison**: Different models have different costs - guidance on model selection
5. **Cost estimation before request**: Can we estimate cost before making an LLM call?
6. **Rate limiting by cost**: Can user set a cost cap per day/week?

**Why critical**: LLM API costs can spiral quickly with autonomous ralph loops. Without budget controls, users get surprise bills.

**What could go wrong**:
- Ralph loop runs amok and makes thousands of LLM calls
- No way to attribute costs to specific features/loops
- User sets budget but there's no enforcement mechanism
- No visibility into cost breakdown by operation type

---

### GAP-3: Ralph Subprocess IPC Protocol (CRITICAL)

**Location**: `01-architecture.md` - mentions IPC but only as `lazyjob-ralph/src/ipc.rs`, `06-ralph-loop-integration.md` referenced but not yet analyzed

**What's missing**:
1. **Message types**: What messages can TUI send to Ralph? What messages can Ralph send back?
2. **State synchronization**: When Ralph modifies data, how does TUI get notified?
3. **Lifecycle protocol**: How does TUI tell Ralph to start a loop, pause, resume, cancel?
4. **Heartbeat/keepalive**: How does TUI know Ralph is still alive vs. hung?
5. **Startup sequence**: Does Ralph connect to TUI or does TUI spawn Ralph?
6. **Unix socket vs pipe**: Which IPC mechanism and why?
7. **Error propagation**: How do errors in Ralph surface to TUI?

**Why critical**: The entire agentic workflow depends on reliable TUI-Ralph communication. If this breaks, Ralph runs blind.

**What could go wrong**:
- Ralph crashes but TUI doesn't notice, user thinks loop is running
- TUI sends cancel but Ralph ignores it, continues wasting resources
- State changes in Ralph don't appear in TUI, user sees stale data
- Multiple Ralph loops conflict when writing to same database

---

### GAP-4: Database Migration Strategy and Schema Evolution (IMPORTANT)

**Location**: `04-sqlite-persistence.md` - mentions migrations but design is incomplete

**What's missing**:
1. **Migration ordering**: How are migrations numbered and ordered? What's the naming convention?
2. **Down migrations**: Are down migrations required? What's the rollback strategy?
3. **Data migration**: When schema changes, how is existing data migrated?
4. **Migration testing**: How are migrations tested before deployment?
5. **Zero-downtime migrations**: For future SaaS, how to migrate without downtime?
6. **Migration conflicts**: When two branches add migrations, how resolved?
7. **Schema versioning**: How does the app know which migrations have been applied?

**Why important**: Schema evolves as features are added. Poor migration strategy leads to data loss or corruption.

**What could go wrong**:
- Migration runs but fails mid-way, leaves schema in inconsistent state
- Old migrations become incompatible with newer SQLite versions
- No down migration means can't rollback a bad schema change
- Data migration for large tables blocks the application

---

### GAP-5: Multi-Process SQLite Concurrency Deep Design (IMPORTANT)

**Location**: `04-sqlite-persistence.md` - mentions "Option 1 (direct SQLite) for MVP" but no deep design

**What's missing**:
1. **Write locking**: When TUI and Ralph both write, how is write conflicts resolved?
2. **Read-your-writes consistency**: When Ralph writes and TUI reads immediately, does TUI see the write?
3. **Connection lifecycle**: Ralph opens connection, uses it, when does it close?
4. **WAL checkpoint coordination**: Who triggers WAL checkpoints? How is it coordinated?
5. **busy_timeout tuning**: What's the actual timeout value? What happens on timeout?
6. **Corruption detection**: How is database corruption detected and recovered?
7. **Future: connection pooling for Ralph**: Should Ralph subprocesses share a connection pool?

**Why important**: SQLite with multiple writers is notoriously tricky. The spec hand-waves this as "WAL mode handles it."

**What could go wrong**:
- Ralph holds write lock too long, TUI freezes
- WAL file grows unbounded without checkpointing
- TUI reads stale data because Ralph's write hasn't checkpointed yet
- Database locked error surfaces to user in confusing way

---

### GAP-6: Application State Machine Deep Design (IMPORTANT)

**Location**: `01-architecture.md` - mentions `AppState` and `StateMachine` trait but no implementation details

**What's missing**:
1. **State events**: What events trigger state transitions? (user action, external event, time-based)
2. **Transition validation**: Can the system prevent invalid transitions? (e.g., Applied → Discovered)
3. **Side effects**: What happens on transitions? (database writes, notifications, LLM calls)
4. **Undo/redo**: Can users undo state changes?
5. **Persistence of state**: Is state in memory only, or persisted to SQLite?
6. **Rehydration**: When app restarts, how is state restored?
7. **Concurrency**: When TUI and Ralph both modify state, how resolved?

**Why important**: Job search involves complex state (applied → phone screen → technical → offer → accepted). Without clear state machine, transitions are ad-hoc and buggy.

**What could go wrong**:
- Job marked "Applied" when it should be "Phone Screen"
- State changes lost on crash (not persisted)
- Two simultaneous state changes conflict, one lost
- No clear path for reverse transitions (rejection, withdrawal)

---

### GAP-7: Startup/Shutdown Lifecycle (IMPORTANT)

**Location**: `01-architecture.md` - no mention of initialization or shutdown

**What's missing**:
1. **Initialization sequence**: What order are components initialized? (DB, LLM, Ralph, TUI)
2. **Graceful startup**: How does app wait for dependencies (DB file, network) before starting?
3. **Configuration loading**: When and how is config.yaml loaded and validated?
4. **Ralph startup**: Does Ralph auto-start on app launch or on-demand?
5. **Shutdown sequence**: How does app gracefully stop? (Ralph first? TUI first? DB last?)
6. **Crash recovery**: What happens if app crashes mid-operation?
7. **Startup failures**: What if DB is corrupted? LLM key invalid? How does app recover?

**Why important**: Apps must start and stop reliably. Poor lifecycle management causes data loss and confusing errors.

**What could go wrong**:
- App starts before DB file is ready, crashes
- Ralph starts before TUI is ready,IPC fails
- App crashes and leaves Ralph processes orphaned
- Config with invalid LLM key causes cryptic error at runtime instead of at startup

---

### GAP-8: Panic Handling, Error Boundaries, and Recovery (IMPORTANT)

**Location**: `01-architecture.md` - mentions "Ralph Subprocess Crash" in failure modes but no design

**What's missing**:
1. **Panic catching**: Does LazyJob catch panics in Ralph subprocesses?
2. **Error logging**: Where do errors go? Is there a structured log file?
3. **Error categories**: Which errors are recoverable vs. fatal?
4. **User notification**: When errors occur, how is user informed?
5. **Ralph restart**: After Ralph crashes, does system auto-restart? How many times?
6. **DAG verification**: Is there a health check that Ralph loop is functioning?

**Why important**: Autonomous agents can fail in unexpected ways. Without error boundaries, failures cascade.

**What could go wrong**:
- Ralph panics, leaves zombie process, leaks memory
- Error occurs but no log entry, impossible to debug
- User not notified of background failures, thinks loop is working
- Ralph continuously crashes in loop, wasting resources

---

### GAP-9: Logging and Telemetry Infrastructure (IMPORTANT)

**Location**: Not mentioned anywhere in reviewed specs

**What's missing**:
1. **Log levels**: DEBUG, INFO, WARN, ERROR - when to use each?
2. **Log destinations**: stdout, file, or both? Rotating log files?
3. **Structured logging**: JSON logs or human-readable?
4. **Tracing**: OpenTelemetry or similar for distributed tracing across TUI ↔ Ralph?
5. **Metrics**: Token usage, loop duration, job processing rate
6. **Ralph-specific logging**: How to distinguish logs from different Ralph loops?
7. **Sensitive data**: How to avoid logging API keys, personal data?

**Why important**: Debugging distributed async systems without logs is nearly impossible.

**What could go wrong**:
- Ralph fails but no logs captured, can't debug
- Logs contain API keys or personal job data
- Log volume overwhelms disk on long-running system
- Can't distinguish which Ralph loop generated which log line

---

### GAP-10: Function Calling / Tool Use for LLM (IMPORTANT)

**Location**: `02-llm-provider-abstraction.md` - mentions ollama-rs function calling but no spec design

**What's missing**:
1. **Tool schema definition**: How are tools (functions) defined and exposed to LLM?
2. **Tool result processing**: How are tool call results fed back to LLM?
3. **Multi-step reasoning**: How does LLM decide when to call tools vs. respond directly?
4. **Tool timeout handling**: What if a tool call takes too long?
5. **Tool error handling**: What if a tool call fails? Does LLM retry?
6. **Security**: Can LLM call arbitrary tools? What's the permission model?

**Why important**: ralph loops need to take actions (search jobs, update database, send email). This requires function calling.

**What could go wrong**:
- LLM calls tool with wrong parameters, corrupts data
- Tool call loops infinitely, hangs LLM
- No timeout on tool calls, LLM waits forever
- LLM calls sensitive tools without user consent

---

### GAP-11: LLM Context Window and Conversation Management (IMPORTANT)

**Location**: `02-llm-provider-abstraction.md` - `context_length()` method but no conversation management

**What's missing**:
1. **Conversation history**: How is message history maintained across multiple turns?
2. **Context truncation**: When context window fills, how is history truncated?
3. **Summary-based truncation**: Does system summarize old messages instead of dropping?
4. **Per-conversation limits**: Different models have different limits - how handled?
5. **Cross-conversation state**: Can Ralph maintain state across different loops?

**Why important**: Job search involves long conversations (research → tailoring → outreach → follow-up). Context management is critical.

**What could go wrong**:
- Context window exceeded, error thrown mid-loop
- Important context dropped during truncation, loop loses track
- No summary of previous turns, user can't review what LLM "remembered"
- Summary quality degrades, loop behavior changes over time

---

### GAP-12: Life Sheet Import/Export and LinkedIn Integration (MODERATE)

**Location**: `03-life-sheet-data-model.md` - mentions LinkedIn import as open question

**What's missing**:
1. **LinkedIn profile import**: What's the import workflow? PDF? API? Scraping?
2. **Resume PDF parsing**: How are PDF resumes converted to Life Sheet?
3. **Resume → Life Sheet extraction**: LLM-based extraction of achievements, skills?
4. **Export formats**: JSON Resume, PDF, Markdown - which and how?
5. **Bi-directional sync**: If user edits PDF externally, how handled?
6. **Partial import**: What if user only wants to import certain sections?

**Why important**: Job seekers have existing resumes/LinkedIn profiles. Manual re-entry is painful.

**What could go wrong**:
- Import fails silently, user doesn't know
- Import loses formatting/achievement context
- LinkedIn ToS prohibits scraping - what's the legal approach?
- Multiple import sources conflict (LinkedIn vs. PDF resume)

---

### GAP-13: YAML Life Sheet Validation and Error Reporting (MODERATE)

**Location**: `03-life-sheet-data-model.md` - Open Question #1 mentions validation

**What's missing**:
1. **Schema validation**: How is YAML validated against the Life Sheet schema?
2. **Error messages**: When YAML has errors, how are they reported to user?
3. **Incremental validation**: Validate as user types? On save? On import?
4. **Unknown fields**: What if YAML has unknown fields? Ignore silently or warn?
5. **Type coercion**: Can YAML have loose types that get coerced?

**Why important**: Users edit YAML directly. Bad YAML should surface as clear errors, not cryptic crashes.

**What could go wrong**:
- User typos field name, silently ignored, feature doesn't work
- YAML parses but semantically invalid, causes runtime errors later
- No error location info, user can't find the bad line

---

### GAP-14: Database Backup Verification and Integrity Checking (MODERATE)

**Location**: `04-sqlite-persistence.md` - backup strategy but no verification

**What's missing**:
1. **Backup verification**: How is a backup verified to be valid?
2. **Integrity checks**: Does SQLite have integrity check commands?
3. **Corruption detection**: How is database corruption detected before backup?
4. **Backup rotation**: How many backups kept? Daily vs. weekly retention?
5. **Point-in-time recovery**: Can user restore to a specific point in time?
6. **Cross-device backup**: Can backups be stored on different device?

**Why important**: If backups are corrupt, they're worthless when needed.

**What could go wrong**:
- Backup created from corrupted database, restore fails when needed
- WAL file partially written, backup has inconsistent state
- Disk full during backup, partial backup worse than no backup
- User doesn't know backup succeeded, assumes it did

---

### GAP-15: Rate Limiting Per-Provider Deep Design (MODERATE)

**Location**: `02-llm-provider-abstraction.md` - Open Question #4 mentions rate limits

**What's missing**:
1. **Rate limit values**: What are actual rate limits per provider?
2. **Global rate limiter**: Should LazyJob implement a global rate limiter?
3. **Retry strategy**: Exponential backoff - what are the exact parameters?
4. **429 handling**: What's the behavior on 429? Queue? Fail? Switch provider?
5. **Rate limit monitoring**: How does user see current rate limit status?
6. **Burst handling**: Can requests burst or must they be evenly spaced?

**Why important**: Providers have strict rate limits. Poor handling causes failed requests.

**What could go wrong**:
- Rate limited, retry immediately, get more rate limited
- No visibility into rate limit status, user surprised by errors
- Global rate limiter not coordinated, different parts of app conflict

---

## Cross-Spec Gaps

These issues span multiple specs:

### Cross-Gap A: Ralph ↔ TUI ↔ Database Concurrency (Critical)

The interaction between Ralph (autonomous subprocess), TUI (main process), and SQLite (database) is glossed over in all specs. Key questions:
- When Ralph writes a job update, does TUI immediately see it?
- Can Ralph and TUI write to the same record simultaneously?
- How is the WAL mode actually configured for this use case?

**Affected specs**: 01-architecture.md, 04-sqlite-persistence.md, (06-ralph-loop-integration.md not yet reviewed)

### Cross-Gap B: LLM Cost Attribution to Ralph Loops

There's no system for attributing LLM costs to specific Ralph loops or operations. This affects:
- User understanding of which operations cost the most
- Debugging why a loop consumed many tokens
- Setting budget limits per feature

**Affected specs**: 02-llm-provider-abstraction.md, (06-ralph-loop-integration.md not yet reviewed)

### Cross-Gap C: Structured Data Flow Between Layers

The data flow from LLM (raw text) → Structured Data (Job, Application) → SQLite → TUI display has gaps:
- How does LLM output get parsed into structured data?
- What validation happens before DB write?
- How are parsing errors handled?

**Affected specs**: 02-llm-provider-abstraction.md, 03-life-sheet-data-model.md, 04-sqlite-persistence.md

---

## Specs to Create

Based on this gap analysis, the following new specs should be created:

### Critical Priority

1. **XX-llm-prompt-versioning.md** - Prompt versioning, testing, rollback, and validation system
2. **XX-llm-cost-budget-management.md** - LLM cost tracking, budget limits, usage attribution
3. **XX-ralph-ipc-protocol.md** - Ralph subprocess IPC message types, lifecycle, state sync

### Important Priority

4. **XX-database-migration-strategy.md** - Schema migration design, ordering, rollback, testing
5. **XX-sqlite-concurrency-deep-design.md** - Multi-process WAL mode tuning, write conflict resolution
6. **XX-application-state-machine.md** - State events, transitions, side effects, persistence
7. **XX-startup-shutdown-lifecycle.md** - Initialization sequence, graceful shutdown, crash recovery
8. **XX-error-handling-panic-recovery.md** - Panic catching, error boundaries, logging, auto-restart
9. **XX-logging-telemetry-infrastructure.md** - Log levels, destinations, structured logging, tracing
10. **XX-llm-function-calling.md** - Tool schema, result processing, security, timeout handling
11. **XX-llm-context-management.md** - Conversation history, truncation, summarization strategies

### Moderate Priority

12. **XX-lifesheet-import-export.md** - LinkedIn import, PDF resume parsing, export formats
13. **XX-yaml-validation-error-reporting.md** - Schema validation, error messages, type coercion
14. **XX-database-backup-verification.md** - Backup integrity checks, corruption detection, rotation
15. **XX-rate-limiting-deep-design.md** - Per-provider limits, global limiter, retry strategy

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-3: Ralph IPC Protocol | Critical | High | Architecture foundation |
| GAP-1: Prompt Versioning | Critical | Medium | Prevents silent failures |
| GAP-2: Cost Budget | Critical | Low | Prevents surprise bills |
| GAP-6: State Machine | Important | High | Core app logic |
| GAP-7: Startup/Shutdown | Important | Medium | Reliability |
| GAP-8: Panic Handling | Important | Low | Debugging, stability |
| GAP-10: Function Calling | Important | Medium | Core ralph capability |
| GAP-11: Context Management | Important | Medium | LLM quality |
| GAP-5: SQLite Concurrency | Important | Medium | Data integrity |
| GAP-4: DB Migrations | Important | Medium | Schema evolution |
| GAP-9: Logging/Telemetry | Important | Medium | Debugging |
| GAP-12: Life Sheet Import | Moderate | High | User experience |
| GAP-13: YAML Validation | Moderate | Low | User experience |
| GAP-14: Backup Verification | Moderate | Low | Reliability |
| GAP-15: Rate Limiting | Moderate | Medium | API reliability |
