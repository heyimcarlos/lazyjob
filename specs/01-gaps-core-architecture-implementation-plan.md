# Gap Analysis: Core Architecture (01-04) — Implementation Plan

## Spec Reference
- **Spec file**: `specs/01-gaps-core-architecture.md`
- **Status**: Researching
- **Last updated**: 2026-04-15

## Executive Summary
This gap analysis document reviews the first four architecture specs and identifies 15 critical gaps in the LazyJob design. The implementation plan prioritizes creating 15 new specification documents to address these gaps, organized into three phases: Critical (Ralph IPC, Prompt Versioning, Cost Budget), Important (State Machine, Lifecycle, Error Handling, etc.), and Moderate (Import/Export, YAML Validation, etc.).

## Problem Statement
The core architecture specs (01-04) provide a solid foundation but leave critical gaps in four key areas: autonomous agent reliability (prompt versioning, cost control), inter-process communication (Ralph IPC protocol), data integrity under concurrency (SQLite multi-process), and operational resilience (startup/shutdown, panic handling, logging).

## Implementation Phases

### Phase 1: Critical Gaps — New Spec Creation

This phase creates the 15 new specification documents identified in the gap analysis. These specs will be created in priority order.

#### 1.1 Critical Priority Specs (Create First)

**1. `XX-ralph-ipc-protocol.md`** — Ralph Subprocess IPC Protocol
- Define message types for TUI ↔ Ralph communication
- Design lifecycle protocol (start, pause, resume, cancel, heartbeat)
- Specify state synchronization mechanism
- Decide on pipe vs unix socket
- Dependencies: `06-ralph-loop-integration.md` (existing)

**2. `XX-llm-prompt-versioning.md`** — Prompt Versioning System
- Design prompt template storage and versioning
- Define prompt testing/validation framework
- Specify A/B testing mechanism
- Design rollback capability
- Dependencies: `02-llm-provider-abstraction.md` (existing), `XX-ralph-ipc-protocol.md` (concurrent)

**3. `XX-llm-cost-budget-management.md`** — LLM Cost Tracking & Budget
- Design per-request cost calculation
- Define budget limit enforcement
- Specify usage attribution to Ralph loops
- Create cost estimation before request
- Dependencies: `02-llm-provider-abstraction.md` (existing), `XX-ralph-ipc-protocol.md` (concurrent)

#### 1.2 Important Priority Specs (Create Second)

**4. `XX-application-state-machine.md`** — Application State Machine
- Define state events and transition rules
- Specify transition validation
- Design side effects on transitions
- Plan undo/redo capability
- Dependencies: `10-application-workflow.md` (existing)

**5. `XX-startup-shutdown-lifecycle.md`** — Startup/Shutdown Lifecycle
- Design initialization sequence (DB, LLM, Ralph, TUI)
- Specify graceful shutdown order
- Define crash recovery mechanism
- Dependencies: `XX-ralph-ipc-protocol.md` (critical)

**6. `XX-error-handling-panic-recovery.md`** — Panic Handling & Recovery
- Define panic catching strategy for Ralph subprocesses
- Design error logging infrastructure
- Specify auto-restart behavior
- Dependencies: `XX-startup-shutdown-lifecycle.md` (concurrent), `XX-logging-telemetry-infrastructure.md` (next)

**7. `XX-logging-telemetry-infrastructure.md`** — Logging Infrastructure
- Define log levels and destinations
- Specify structured logging format
- Design cross-process tracing (TUI ↔ Ralph)
- Dependencies: `06-ralph-loop-integration.md` (existing)

**8. `XX-llm-function-calling.md`** — LLM Function Calling / Tool Use
- Define tool schema structure
- Specify tool result processing
- Design security model for tool access
- Dependencies: `02-llm-provider-abstraction.md` (existing), `XX-llm-prompt-versioning.md` (concurrent)

**9. `XX-llm-context-management.md`** — LLM Context Window Management
- Define conversation history structure
- Specify truncation strategy
- Design summary-based compression
- Dependencies: `02-llm-provider-abstraction.md` (existing)

**10. `XX-sqlite-concurrency-deep-design.md`** — SQLite Multi-Process Concurrency
- Deep design for WAL mode tuning
- Define write lock strategy
- Specify read-your-writes consistency
- Dependencies: `04-sqlite-persistence.md` (existing)

**11. `XX-database-migration-strategy.md`** — Database Migration Strategy
- Define migration ordering and naming
- Specify down-migration approach
- Design migration testing strategy
- Dependencies: `04-sqlite-persistence.md` (existing)

#### 1.3 Moderate Priority Specs (Create Third)

**12. `XX-lifesheet-import-export.md`** — Life Sheet Import/Export
- Define LinkedIn import approach
- Specify PDF resume parsing
- Design export formats
- Dependencies: `03-life-sheet-data-model.md` (existing)

**13. `XX-yaml-validation-error-reporting.md`** — YAML Validation
- Define schema validation rules
- Specify error message format
- Design incremental validation
- Dependencies: `03-life-sheet-data-model.md` (existing)

**14. `XX-database-backup-verification.md`** — Backup Verification
- Define backup integrity checks
- Specify corruption detection
- Design backup rotation
- Dependencies: `04-sqlite-persistence.md` (existing)

**15. `XX-rate-limiting-deep-design.md`** — Rate Limiting Deep Design
- Define per-provider rate limits
- Specify global rate limiter
- Design retry strategy
- Dependencies: `02-llm-provider-abstraction.md` (existing)

### Phase 2: Critical Gaps — Implementation

After the 15 specs are created, implement the critical infrastructure:

#### 2.1 Ralph IPC Protocol Implementation
```
lazyjob-ralph/src/ipc/
├── protocol.rs        # Message types, serialization
├── lifecycle.rs        # Start, pause, resume, cancel, heartbeat
├── state_sync.rs       # Database change notification
└── transport.rs       # Stdio transport (or unix socket if chosen)
```

#### 2.2 Prompt Versioning Implementation
```
lazyjob-ralph/src/prompts/
├── registry.rs         # Prompt template storage, versioning
├── validator.rs        # Output quality validation
├── diff.rs             # Visual diff between versions
└── rollback.rs         # Revert to previous version
```

#### 2.3 Cost Budget Implementation
```
lazyjob-core/src/billing/
├── tracker.rs          # Per-request cost calculation
├── budget.rs           # Budget limit enforcement
├── attribution.rs     # Usage attribution to loops
└── limits.rs          # Rate limiting by cost
```

### Phase 3: Important Gaps — Implementation

#### 3.1 Application State Machine
- Implement state machine trait in `lazyjob-core`
- Add transition validation rules
- Implement side effect handlers
- Add undo/redo stack

#### 3.2 Lifecycle Management
```
lazyjob-cli/src/
├── init.rs             # Initialization sequence
├── shutdown.rs         # Graceful shutdown handlers
└── crash.rs           # Recovery from dirty state
```

#### 3.3 Error Handling & Logging
```
lazyjob-core/src/error/
├── panic.rs            # Panic catching, recovery
├── log.rs              # Structured logging setup
└── telemetry.rs        # Cross-process tracing
```

## Data Model

### New Database Tables

```sql
-- Ralph loop audit trail for attribution
CREATE TABLE llm_usage (
    id INTEGER PRIMARY KEY,
    loop_id TEXT NOT NULL,
    loop_type TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    duration_ms INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Prompt version history
CREATE TABLE prompt_versions (
    id INTEGER PRIMARY KEY,
    loop_type TEXT NOT NULL,
    version INTEGER NOT NULL,
    template TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Budget tracking
CREATE TABLE budget_usage (
    id INTEGER PRIMARY KEY,
    period TEXT NOT NULL,  -- 'daily', 'weekly', 'monthly'
    amount_usd REAL NOT NULL,
    reset_at TEXT NOT NULL
);
```

### Migration Approach
- Use `rusqlite_migration` with sequential versioning
- Each gap spec implementation adds a migration file
- Test migrations on realistic data volumes before release

## API Surface

### New Modules

```rust
// lazyjob-core
pub mod billing;        // Cost tracking, budgets
pub mod state_machine;  // Application state transitions
pub mod lifecycle;     // Init, shutdown, crash recovery
pub mod error;          // Panic handling, structured errors

// lazyjob-ralph
pub mod ipc;           // TUI ↔ Ralph protocol
pub mod prompts;       // Prompt versioning, testing
pub mod telemetry;      // Cross-process tracing
```

## Key Technical Decisions

1. **IPC: Stdio JSON (not Unix sockets)** — Simpler, debuggable, survives TUI restart via SQLite
2. **Prompt versioning: Git-style with tags** — Familiar model, easy rollback
3. **Cost tracking: Per-call attribution** — Enables budget limits and optimization
4. **SQLite concurrency: WAL mode + busy_timeout** — Conservative approach; upgrade to(sqlx if needed
5. **Logging: Structured JSON to file** — Machine-parseable, can pipe to external systems
6. **State machine: Event-sourced** — All transitions logged; enables undo/redo and audit trail

## File Structure

```
lazyjob/
├── lazyjob-core/
│   ├── src/
│   │   ├── billing/           # NEW: Cost tracking
│   │   ├── state_machine.rs   # NEW: Application state
│   │   ├── lifecycle.rs       # NEW: Init/shutdown
│   │   ├── error.rs           # NEW: Panic handling
│   │   └── lib.rs
│   └── migrations/            # Extended with gap fills
├── lazyjob-ralph/
│   ├── src/
│   │   ├── ipc/               # NEW: Protocol implementation
│   │   ├── prompts/           # NEW: Prompt versioning
│   │   └── lib.rs
├── lazyjob-tui/
│   └── src/
│       └── lib.rs             # Updated for IPC
├── lazyjob-cli/
│   └── src/
│       └── main.rs            # Updated for lifecycle
└── specs/
    └── XX-*.md                # 15 new gap specs
```

## Dependencies

### External Crates
- `tracing` + `tracing-subscriber` — Structured logging with spans
- `rusqlite_migration` — Database migration management
- `serde_json` — IPC message serialization
- `tokio` — Async runtime for lifecycle management
- `anyhow` + `thiserror` — Error handling

### Internal Dependencies
- All gap implementations depend on `lazyjob-core` being stable
- Ralph IPC depends on `06-ralph-loop-integration.md` protocol design
- Cost tracking depends on LLM provider trait (`02-llm-provider-abstraction.md`)

## Testing Strategy

### Unit Tests
- State machine: Valid transitions, invalid transitions rejected
- Cost calculation: Verify against known provider pricing
- Prompt diff: Visual diff matches expected output
- YAML validation: Valid/invalid YAML correctly identified

### Integration Tests
- TUI ↔ Ralph IPC: Full lifecycle (start → pause → resume → cancel)
- SQLite concurrency: TUI and Ralph both writing, verify consistency
- Budget enforcement: Verify calls blocked when budget exceeded

### Chaos Testing
- Ralph crash mid-operation → verify TUI recovers
- Database locked → verify busy_timeout behavior
- LLM rate limited → verify exponential backoff

## Open Questions

1. **Unix socket vs stdio pipes**: The gap analysis mentions both. Decision needed early.
2. **Structured output validation**: Should we use JSON mode or parse with LLM?
3. **Budget enforcement granularity**: Per-day? Per-week? Per-month? Per-loop-type?
4. **Undo/redo scope**: Full application undo or just state transitions?
5. **Cross-platform SQLite**: Test on Windows (limited WAL support)?

## Effort Estimate

**Phase 1 (Spec Creation)**: 3-5 days
- 15 new spec documents, averaging 2-4 hours each
- Some specs depend on others (must create in order)

**Phase 2 (Critical Implementation)**: 2-3 weeks
- Ralph IPC protocol: ~1 week (most complex)
- Prompt versioning: ~3-4 days
- Cost tracking: ~3-4 days

**Phase 3 (Important Implementation)**: 3-4 weeks
- State machine: ~1 week
- Lifecycle management: ~1 week
- Error handling & logging: ~1 week
- Function calling: ~1 week (can parallelize with others)

**Total**: 6-8 weeks for gap fills, concurrent with feature development
