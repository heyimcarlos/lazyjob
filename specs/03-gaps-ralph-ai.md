# Gap Analysis: Ralph AI (06, 17, agentic-* specs)

## Specs Reviewed
- `06-ralph-loop-integration.md` - Ralph Loop Integration
- `17-ralph-prompt-templates.md` - Ralph Prompt Templates
- `agentic-llm-provider-abstraction.md` - LLM Provider Abstraction
- `agentic-prompt-templates.md` - Ralph Prompt Templates and Anti-Fabrication Rules
- `agentic-ralph-orchestration.md` - Ralph Loop Orchestration
- `agentic-ralph-subprocess-protocol.md` - Ralph Subprocess IPC Protocol

---

## What's Well-Covered

### agentic-ralph-subprocess-protocol.md
- Clean WorkerCommand/WorkerEvent enum design with serde tag
- Interactive mode (MockInterviewLoop) with AwaitingInput state
- Crash recovery via ralph_loop_runs table
- Cancellation protocol (3-step: signal → drain → kill)
- Stderr → log file redirection
- Process management with broadcast channel

### agentic-ralph-orchestration.md
- LoopType enum with concurrency_limit() and priority()
- LoopDispatch with enqueue() and dispatch_suggestion()
- Priority queue with BinaryHeap
- Scheduled loops with cron expression
- SQLite result writes pattern
- Queue bounded at 20 entries

### agentic-prompt-templates.md
- Grounding-before-generation pattern - excellent
- Three-tier fabrication constraint system (profile, narrative, negotiation)
- Context structs per loop type
- Prompt injection defense pattern
- Output validation with FabricationLevel
- JSON output schemas per loop type

### agentic-llm-provider-abstraction.md
- LlmProvider + EmbeddingProvider trait split
- Token usage tracking with token_usage_log table
- Ollama fallback chain (Anthropic → Ollama → error)
- Provider registry with default selection
- Open questions about batching and SaaS proxy

### 06-ralph-loop-integration.md
- Tokio process management patterns
- Stdio vs Unix sockets comparison
- Ralph CLI interface design
- TUI Ralph Panel design
- Crash recovery patterns

### 17-ralph-prompt-templates.md
- Prompt design principles
- All 7 loop type templates
- System prompts with error handling
- JSON output schemas
- Prompt injection defense

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-27: Ralph Process Orphan Cleanup (CRITICAL)

**Location**: `06-ralph-loop-integration.md` - mentions cleanup but not comprehensive; `agentic-ralph-subprocess-protocol.md` - crash recovery only handles TUI crash, not orphaned processes

**What's missing**:
1. **Orphaned Ralph processes**: If TUI kills Ralph (SIGTERM or kill) but process doesn't exit cleanly, it becomes orphaned
2. **Zombie process handling**: What if Ralph child becomes zombie (parent died but process still in process table)?
3. **Process group cleanup**: If Ralph spawns sub-processes, killing Ralph should kill the group
4. **Resource cleanup**: What cleanup happens when Ralph is killed? Temp files? Partial DB writes?
5. **Startup lock**: How to prevent two Ralph processes for same loop? File lock? PID file?

**Why critical**: Without orphan cleanup, over time the system accumulates zombie Ralph processes that waste resources.

**What could go wrong**:
- Ralph subprocess hangs, TUI kills it, but child process survives as zombie
- Multiple Ralph processes for same loop type run simultaneously (violates concurrency limit)
- Temp files left behind, disk space consumed
- Partial job discovery results written but not marked complete

---

### GAP-28: LLM Call Interruption During Cancellation (CRITICAL)

**Location**: `06-ralph-loop-integration.md` - cancellation described but not for in-flight LLM calls; `agentic-prompt-templates.md` - mentions cancellation but no interrupt mechanism

**What's missing**:
1. **In-flight LLM call cancellation**: If Ralph is mid-LLM call when Cancel received, does it wait for completion or abort?
2. **Partial result handling**: If LLM call is 80% complete, should Ralph wait or use partial result?
3. **Timeout for drain**: After Cancel signal, Ralph drains "current atomic unit" - how long is too long?
4. **LLM provider cancel**: Does the LLM HTTP client support request cancellation?
5. **Graceful vs forced cancel**: User cancel vs system cancel (budget exceeded) - different behavior?

**Why critical**: LLM calls can take 10+ seconds. Users who cancel expect immediate response, not waiting for LLM to finish.

**What could go wrong**:
- User cancels, waits 15 seconds for LLM call to complete, Ralph finally exits
- Partial LLM response used, producing garbled output
- LLM call cancellation causes connection pool issues
- Cancel during streaming leaves partial text in output

---

### GAP-29: Ralph Loop State Persistence Mid-Execution (IMPORTANT)

**Location**: `06-ralph-loop-integration.md` - Open Question #3 mentions progress persistence but no design

**What's missing**:
1. **Checkpoint frequency**: How often should Ralph save progress to SQLite? After each company? Each LLM call?
2. **Checkpoint data**: What state needs to be persisted? Loop type, params, current position, partial results?
3. **Resume from checkpoint**: If Ralph resumes after crash, how does it know where it was?
4. **Stale checkpoint cleanup**: If Ralph completes successfully, when is checkpoint deleted?
5. **Checkpoint vs output**: Should checkpoints overwrite final output or be kept separate?

**Why important**: Without progress persistence, long-running loops (job discovery across 50 companies) lose all progress on crash.

**What could go wrong**:
- Ralph 80% through discovery, crashes, user must re-run from beginning
- Checkpoint frequency too high, excessive DB writes slow down loop
- Checkpoint data inconsistent with actual state
- Old checkpoints never cleaned up, database grows unbounded

---

### GAP-30: Ralph Queue Visibility and Management UI (IMPORTANT)

**Location**: `agentic-ralph-orchestration.md` - Open Question: "Should queued loops be visible in TUI?"

**What's missing**:
1. **Queue display**: Show queued loops in TUI with position, estimated wait time
2. **Queue removal**: Can user remove a queued loop before it starts?
3. **Queue reordering**: Can user change priority of queued loops?
4. **Queue notification**: When queued loop finally starts, notify user?
5. **Queue persistence**: If TUI restarts while loops are queued, what happens to queue?

**Why important**: Without queue visibility, users don't know when their requested loops will run.

**What could go wrong**:
- User queues 5 resume tailoring runs, doesn't know they're #3 in queue
- User wants to cancel queued loop but has no way to see it
- TUI restarts, queue is lost, user thinks loop will run but it's gone
- Loop starts hours after user requested, context no longer relevant

---

### GAP-31: Ralph Loop Concurrency Governor (IMPORTANT)

**Location**: `agentic-ralph-orchestration.md` - concurrency_limit() per type but no global governor

**What's missing**:
1. **Global concurrency limit**: Max total Ralph processes regardless of type
2. **Memory governor**: If total Ralph memory exceeds threshold, pause new spawns
3. **CPU governor**: If system CPU high, throttle new spawns
4. **I/O governor**: If disk I/O saturated, pause disk-heavy loops
5. **Priority inversion**: High-priority loop blocked by low-priority loop due to governor

**Why important**: Running all loops at full concurrency could overwhelm a laptop.

**What could go wrong**:
- User triggers all 7 loop types simultaneously, system becomes unresponsive
- JobDiscovery + ResumeTailoring + CompanyResearch all running = high memory/CPU
- No mechanism to pause less important loops during resource shortage

---

### GAP-32: MockInterviewLoop Timeout and State Recovery (IMPORTANT)

**Location**: `agentic-ralph-subprocess-protocol.md` - AwaitingInput state mentioned; `agentic-ralph-orchestration.md` - MockInterviewLoop priority 10 (highest)

**What's missing**:
1. **User inactivity timeout**: If user doesn't respond for 10 minutes, what happens?
2. **Timeout action**: Cancel loop? Pause? Notify user?
3. **Loop state during await**: Is conversation history persisted? Can user resume after timeout?
4. **Reconnection**: If TUI disconnects during mock interview, can user rejoin?
5. **Session persistence**: Is mock interview saved so user can resume later?

**Why important**: MockInterviewLoop is interactive and user-facing. Poor timeout handling ruins UX.

**What could go wrong**:
- User starts mock interview, gets called away, loop hangs for hours
- TUI crashes during mock interview, user loses all progress
- User walks away, comes back, doesn't know interview is waiting or timed out

---

### GAP-33: Scheduled Loop Overlap Prevention (MODERATE)

**Location**: `agentic-ralph-orchestration.md` - JobDiscovery cron scheduling, but no overlap prevention

**What's missing**:
1. **Overlap detection**: If previous JobDiscovery still running when cron fires, skip or queue?
2. **Missed schedule recovery**: If laptop was asleep when cron fired, catch up on startup?
3. **Maximum runtime**: If JobDiscovery exceeds some threshold, force-kill and log warning
4. **Backfill prevention**: Don't backfill discovery runs that were missed while offline
5. **Schedule jitter**: Add random jitter to prevent all users' LazyJob from hitting APIs simultaneously

**Why important**: If cron fires while previous run is still going, could have two simultaneous discovery loops.

**What could go wrong**:
- JobDiscovery takes 2 hours (50 companies), cron fires for next run, two run simultaneously
- User closes laptop Friday, opens Monday, gets 3 discovery runs queued up
- Discovery runs too long, consuming battery and API quota

---

### GAP-34: Ralph API Key Management (MODERATE)

**Location**: `06-ralph-loop-integration.md` - Open Question #2; `agentic-llm-provider-abstraction.md` - mentions "read from OS keyring"

**What's missing**:
1. **Keyring access**: How does Ralph read from OS keyring? Which keyring? (macOS Keychain? Windows Credential Manager? Linux secret-service?)
2. **Keyring fallback**: If keyring unavailable, what? Config file? Env var?
3. **Key rotation**: When user rotates API key, how does LazyJob pick up new key?
4. **Multi-key support**: Can user have different API keys for different providers?
5. **SaaS key management**: In cloud mode, how are keys managed differently?

**Why important**: API keys are sensitive. Poor key management is a security risk.

**What could go wrong**:
- Keys stored in config file in plaintext
- Keyring access fails silently, Ralph can't call LLM
- User rotates key, LazyJob still using old key

---

### GAP-35: Ralph Loop Retry Logic (MODERATE)

**Location**: `06-ralph-loop-integration.md` - mentions retries but no spec

**What's missing**:
1. **Retry on failure**: When loop fails, retry automatically? How many times?
2. **Retry backoff**: Exponential backoff between retries?
3. **Retry budget**: Max retries per hour? Per day?
4. **Non-retryable errors**: Which errors should never retry? (auth failure, invalid params)
5. **User notification**: When loop retried, does user know?

**Why important**: Network flakiness causes transient failures. Loops should retry intelligently.

**What could go wrong**:
- Loop fails due to transient network error, no retry, user thinks LazyJob is broken
- Loop stuck in retry loop for auth failure, wasting resources
- User doesn't know loop is being retried, expects immediate failure

---

### GAP-36: Ralph Log Management and Rotation (MODERATE)

**Location**: `agentic-ralph-subprocess-protocol.md` - stderr → log file, 7-day retention

**What's missing**:
1. **Log rotation**: When does log rotate? Size-based? Time-based?
2. **Log compression**: Should old logs be gzipped?
3. **Log location**: Where are logs stored? ~/.lazyjob/logs/?
4. **Log level configuration**: Can user set log level (DEBUG, INFO, WARN)?
5. **Log aggregation**: For multiple Ralph runs, are logs separate or combined per loop_id?
6. **Log cleanup**: 7-day retention - enforced how? On startup? Continuously?

**Why important**: Logs are critical for debugging Ralph issues.

**What could go wrong**:
- Logs grow unbounded, fill disk
- Logs auto-deleted before user can debug issue
- User can't find logs for a specific failed run

---

### GAP-37: Ralph Configuration Hot-Reload (MODERATE)

**Location**: `agentic-ralph-orchestration.md` - SchedulerConfig from lazyjob.toml

**What's missing**:
1. **Config file watching**: Does Ralph re-read config when lazyjob.toml changes?
2. **Per-loop config**: Can user configure per-loop behavior (e.g., disable certain loops)?
3. **Feature flags**: Can user disable certain loop types entirely?
4. **Dynamic scheduler**: Can user add/modify cron schedules without restart?
5. **Config validation**: If config invalid, fail fast or use defaults?

**Why important**: Users should be able to change LazyJob behavior without restarting processes.

**What could go wrong**:
- User changes cron schedule, discovery still runs on old schedule
- User disables JobDiscovery, it still runs on schedule
- Invalid config causes cryptic failure

---

### GAP-38: Structured Logging and Tracing (MODERATE)

**Location**: Not mentioned in any Ralph spec

**What's missing**:
1. **Structured log format**: JSON logs vs human-readable?
2. **Correlation IDs**: loop_id attached to all logs for a run
3. **Trace propagation**: From TUI through IPC to Ralph logs
4. **Performance tracing**: How long did each step take?
5. **Log levels**: DEBUG for development, INFO for production
6. **Sensitive data redaction**: API keys, personal data redacted from logs

**Why important**: Debugging distributed TUI↔Ralph system without structured logs is difficult.

**What could go wrong**:
- Ralph fails but logs don't capture the error context
- Can't correlate TUI events with Ralph events for same loop_id
- Logs contain sensitive data (API keys in debug mode)

---

## Cross-Spec Gaps

### Cross-Gap G: Loop State Consistency

The interaction between:
- Ralph writing directly to SQLite
- TUI reading from SQLite
- Crash recovery via ralph_loop_runs table

There's no spec for:
- What happens if TUI reads while Ralph is mid-write?
- WAL checkpoint timing coordination
- How TUI invalidates its cache after Ralph writes

**Affected specs**: `agentic-ralph-subprocess-protocol.md`, `agentic-ralph-orchestration.md`, (04-sqlite-persistence.md)

### Cross-Gap H: Budget Enforcement Integration

`agentic-llm-provider-abstraction.md` defines token_usage_log but doesn't integrate with budget enforcement. If budget is exceeded mid-loop, how does Ralph receive this signal?

**Affected specs**: `agentic-llm-provider-abstraction.md`, `agentic-ralph-orchestration.md`, (XX-llm-cost-budget-management.md)

---

## Specs to Create

### Critical Priority

1. **XX-ralph-process-orphan-cleanup.md** - Orphaned process detection, zombie handling, process group cleanup
2. **XX-ralph-llm-call-interruption.md** - In-flight LLM call cancellation, graceful vs forced cancel

### Important Priority

3. **XX-ralph-loop-state-persistence.md** - Checkpoint frequency, resume from checkpoint, stale cleanup
4. **XX-ralph-queue-management-ui.md** - Queue display, removal, reordering, persistence
5. **XX-ralph-concurrency-governor.md** - Global limits, memory/CPU/IO governors, priority inversion
6. **XX-ralph-mock-interview-timeout.md** - User inactivity timeout, session persistence, reconnection

### Moderate Priority

7. **XX-ralph-scheduled-loop-overlap.md** - Overlap prevention, missed schedule recovery, backfill prevention
8. **XX-ralph-api-key-management.md** - Keyring integration, fallback, rotation
9. **XX-ralph-loop-retry-logic.md** - Retry policy, backoff, non-retryable errors
10. **XX-ralph-log-management.md** - Rotation, compression, location, retention
11. **XX-ralph-config-hot-reload.md** - Config watching, feature flags, dynamic scheduler
12. **XX-ralph-structured-logging.md** - JSON logs, correlation IDs, tracing

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-27: Process Orphan Cleanup | Critical | Medium | Resource leaks |
| GAP-28: LLM Call Interruption | Critical | Medium | User experience |
| GAP-29: Loop State Persistence | Important | High | Reliability |
| GAP-30: Queue Management UI | Important | Medium | User experience |
| GAP-31: Concurrency Governor | Important | Medium | System stability |
| GAP-32: MockInterview Timeout | Important | Medium | User experience |
| GAP-33: Scheduled Loop Overlap | Moderate | Low | Reliability |
| GAP-34: API Key Management | Moderate | Medium | Security |
| GAP-35: Loop Retry Logic | Moderate | Low | Reliability |
| GAP-36: Log Management | Moderate | Low | Debugging |
| GAP-37: Config Hot-Reload | Moderate | Low | User experience |
| GAP-38: Structured Logging | Moderate | Medium | Debugging |
