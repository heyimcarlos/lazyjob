# Gap Analysis: Ralph AI — Implementation Plan

## Spec Reference
- **Spec file**: `specs/03-gaps-ralph-ai.md`
- **Status**: Gap Analysis (Researching)
- **Last updated**: 2026-04-15

## Executive Summary
This gap analysis identifies 12 critical holes across the Ralph AI system specs (06, 17, agentic-*). The most urgent are process orphan cleanup and LLM call interruption during cancellation — both can leave the system in inconsistent states. This plan addresses each gap through targeted spec creation and implementation, prioritizing by criticality.

## Problem Statement
The Ralph AI specs collectively describe a sophisticated autonomous agent system, but lack concrete designs for: process lifecycle management (orphans, zombies), LLM call cancellation mid-flight, progress persistence mid-execution, queue visibility, concurrency governance, mock interview timeouts, scheduled loop overlap prevention, API key management, retry logic, log rotation, config hot-reload, and structured logging.

## Implementation Phases

### Phase 1: Critical Gaps — Process & Cancellation (Weeks 1-2)
Address the two critical gaps that can leave resource leaks and poor UX.

**GAP-27: Process Orphan Cleanup**
- Create `specs/XX-ralph-process-orphan-cleanup.md`
- Implementation in `lazyjob-ralph/`:
  - `src/process/orphan.rs` — PID tracking, startup locks via file locks
  - `src/process/group.rs` — Process group cleanup on SIGTERM
  - `src/process/resource.rs` — Temp file cleanup, partial write rollback
  - `src/process/cleanup.rs` — StartupGuardian runs on Ralph spawn

**GAP-28: LLM Call Interruption**
- Create `specs/XX-ralph-llm-call-interruption.md`
- Implementation in `lazyjob-llm/`:
  - Add `abort()` method to `LlmProvider` trait
  - HTTP client must support request cancellation (reqwest::Builder::timeout per-request)
  - `lazyjob-ralph/` `src/loop/cancellation.rs` — distinguish graceful vs forced cancel
  - Timeout for drain phase configurable (default 5s)

### Phase 2: Important Gaps — State & UI (Weeks 2-4)

**GAP-29: Loop State Persistence**
- Create `specs/XX-ralph-loop-state-persistence.md`
- SQLite checkpoint table design with frequency strategy
- Implementation in `lazyjob-ralph/` `src/checkpoint/`

**GAP-30: Queue Management UI**
- Create `specs/XX-ralph-queue-management-ui.md`
- TUI panel for queue visibility with CRUD operations
- Implementation in `lazyjob-tui/` `src/panels/queue.rs`

**GAP-31: Concurrency Governor**
- Create `specs/XX-ralph-concurrency-governor.md`
- Global limits via System resource monitoring
- Implementation in `lazyjob-ralph/` `src/governor/`

**GAP-32: MockInterviewLoop Timeout**
- Create `specs/XX-ralph-mock-interview-timeout.md`
- Implementation in `lazyjob-ralph/` `src/loops/mock_interview.rs`

### Phase 3: Moderate Gaps — Reliability & Ops (Weeks 4-6)

**GAP-33: Scheduled Loop Overlap Prevention** — Add to scheduler
**GAP-34: API Key Management** — Keyring integration
**GAP-35: Loop Retry Logic** — Exponential backoff
**GAP-36: Log Management** — Rotation, compression
**GAP-37: Config Hot-Reload** — File watching
**GAP-38: Structured Logging** — JSON format, correlation IDs

### Cross-Cutting: Cross-Gap G & H
**Cross-Gap G** (Loop State Consistency): Address as part of GAP-29 checkpoint design — TUI cache invalidation via SQLite WAL notifications or poll-on-interval.
**Cross-Gap H** (Budget Enforcement): Coordinate with `XX-llm-cost-budget-management.md` spec creation.

## Data Model
New tables needed in `lazyjob-core/`:

```sql
-- For GAP-27: Process tracking
CREATE TABLE ralph_process_locks (
    loop_type TEXT NOT NULL,
    pid INTEGER NOT NULL,
    started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (loop_type)
);

-- For GAP-29: Checkpoint persistence
CREATE TABLE ralph_loop_checkpoints (
    loop_id TEXT PRIMARY KEY,
    loop_type TEXT NOT NULL,
    params_json TEXT NOT NULL,
    current_position INTEGER DEFAULT 0,
    partial_results_json TEXT,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- For GAP-36: Log management
CREATE TABLE ralph_loop_logs (
    loop_id TEXT PRIMARY KEY,
    log_path TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (loop_id) REFERENCES ralph_loop_runs(loop_id)
);
```

## API Surface
New modules in `lazyjob-ralph/`:
- `process::orphan` — orphan detection and cleanup
- `process::group` — process group management
- `loop::cancellation` — graceful/forced cancellation
- `loop::checkpoint` — state persistence
- `governor::` — concurrency limiting
- `scheduler::` — with overlap prevention

New modules in `lazyjob-tui/`:
- `panels::queue` — queue management UI
- `views::ralph_status` — status bar integration

## Key Technical Decisions

1. **Cancellation strategy**: Use `AbortHandle` from reqwest for HTTP call cancellation. Ralph process receives SIGTERM first (graceful drain), then SIGKILL after timeout. Distinguish user-cancel vs system-cancel (budget exceeded) — system-cancel skips drain phase.

2. **Checkpoint frequency**: Save after each "atomic unit" — for JobDiscovery this is after each company. Checkpoints are additive (don't overwrite final output until loop completes).

3. **Orphan detection**: On Ralph spawn, acquire exclusive file lock for `loop_type`. If lock held by dead PID, stale lock cleanup on startup.

4. **Concurrency governor**: Use `sysinfo` crate for memory/CPU monitoring. Global limit default 3 Ralph processes. Per-type limits remain per `LoopType::concurrency_limit()`.

5. **Queue persistence**: Queue stored in SQLite `ralph_loop_queue` table, survives TUI restart.

## File Structure
```
lazyjob/
├── lazyjob-core/
│   └── migrations/  (new checkpoint + queue tables)
├── lazyjob-llm/
│   └── src/provider.rs  (add AbortHandle support)
├── lazyjob-ralph/
│   └── src/
│       ├── process/  (new: orphan.rs, group.rs, resource.rs, cleanup.rs)
│       ├── loop/  (new: cancellation.rs, checkpoint.rs)
│       ├── governor/  (new: mod.rs)
│       ├── scheduler/  (enhanced: overlap prevention)
│       └── ralph.rs  (enhanced: process lifecycle)
└── lazyjob-tui/
    └── src/
        └── panels/  (new: queue.rs)
```

## Dependencies
- **External crates**:
  - `sysinfo` — system resource monitoring for governor
  - `notify` — config file watching for hot-reload
  - `rustsec`/`keyring` — OS keyring access for API key management
  - `tracing` + `tracing-subscriber` — structured logging with JSON format
  - `filetime` — log rotation timestamps
- **Spec dependencies** (must implement first):
  - `06-ralph-loop-integration.md` — Ralph process spawning baseline
  - `agentic-ralph-subprocess-protocol.md` — IPC protocol baseline
  - `agentic-ralph-orchestration.md` — Queue and dispatch baseline
  - `04-sqlite-persistence.md` — Database schema baseline

## Testing Strategy
- **Unit tests**: Process orphan detection (mock PIDs), cancellation state machine, governor thresholds
- **Integration tests**: Spawn Ralph with cancellation mid-LLM-call, verify process terminates within timeout
- **Property-based tests**: Checkpoint serialization roundtrips, queue ordering invariants
- **Edge cases**:
  - Zombie process: kill parent, verify child reaped
  - LLM call cancelled: verify partial output not used
  - Checkpoint during crash: verify resume picks up at correct position
  - Queue persistence: kill TUI during queue operation, verify queue intact on restart

## Open Questions
1. **Should Ralph logs be combined per-session or separate per-loop?** Current spec says separate per loop_id. Consider log aggregation for debugging.
2. **What triggers stale lock cleanup?** Only on startup, or also periodic health-check?
3. **Budget enforcement signal**: Should budget-exceeded cancel via SIGKILL (no drain) or SIGTERM with 1s drain max?
4. **Checkpoint compression**: Checkpoints for long loops (50 companies) could be large. Should we compress partial results JSON?
5. **Mock interview inactivity timeout default**: 10 min is suggested. Should this be configurable?

## Effort Estimate
- **Critical gaps (GAP-27, GAP-28)**: ~3-4 days — process management is well-understood, cancellation requires HTTP client changes
- **Important gaps (GAP-29 through GAP-32)**: ~1 week — checkpoint design needs careful schema planning, TUI queue panel is medium effort
- **Moderate gaps (GAP-33 through GAP-38)**: ~5-7 days — much is configuration and ops tooling, relatively contained
- **Total**: ~2-3 weeks for all 12 gaps

**Note**: This gap analysis doesn't implement Ralph loops themselves — it fills critical missing pieces that make the Ralph system production-ready.
