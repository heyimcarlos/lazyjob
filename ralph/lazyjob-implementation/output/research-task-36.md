# Research: Task 36 — Resume Tailor Ralph Loop

## Objective
Wire ResumeTailor and CoverLetterService into the Ralph subprocess system. Implement CLI worker subcommands, trigger from TUI, show progress in Ralph panel.

## Existing Infrastructure

### Ralph Process Manager (`lazyjob-ralph/src/process_manager.rs`)
- `RalphProcessManager::spawn(&mut self, loop_type: &str, params: Value) -> Result<RunId>`
- Spawns `{binary_path} worker` with piped stdin/stdout
- Sends `WorkerCommand::Start { loop_type, params }` on stdin
- Background task reads stdout lines, decodes as `WorkerEvent`, broadcasts as `(RunId, WorkerEvent)`
- `cancel(&mut self, run_id)` — sends Cancel command, waits 3s, then SIGKILL
- `subscribe()` — returns `broadcast::Receiver<(RunId, WorkerEvent)>`

### Protocol (`lazyjob-ralph/src/protocol.rs`)
- `WorkerCommand::Start { loop_type: String, params: Value }` / `WorkerCommand::Cancel`
- `WorkerEvent::Status { phase, progress: f32, message }` / `Results { data }` / `Error { code, message }` / `Done { success }`
- `NdjsonCodec::encode(cmd) -> String`, `decode(line) -> Result<WorkerEvent>`

### CLI (`lazyjob-cli/src/main.rs`)
- No `Worker` subcommand exists — needs to be added
- `LlmProviderCompleter` struct already bridges `LlmProvider` → `Completer` trait (line 263)
- LLM setup pattern: `Config::load()` + `CredentialManager::new()` + `LlmBuilder::from_config()` (line 322-324)

### TUI State
- `RalphUpdate` enum: Progress, LogLine, Completed, Failed — no `Started` variant
- `_ralph_tx` dropped immediately in `lib.rs:26` — channel has no producer
- `App.handle_action`: TailorResume, GenerateCoverLetter, CancelRalphLoop are all no-ops
- `RalphPanelView.on_update()` creates ActiveEntry with `loop_type: "unknown"` for new Progress IDs
- Event loop polls `ralph_rx` already — just needs a sender

### ResumeTailor (`lazyjob-core/src/resume/mod.rs`)
- `ResumeTailor::new(completer: Arc<dyn Completer>)`
- `tailor(&self, job, life_sheet, options, progress_tx: Option<mpsc::Sender<ProgressEvent>>)`
- Returns `(ResumeContent, GapReport, FabricationReport)`
- ProgressEvent: ParsingJd, GapAnalysis, FabricationPreCheck, RewritingBullets, Assembling, Done

### CoverLetterService (`lazyjob-core/src/cover_letter/mod.rs`)
- `CoverLetterService::new(completer: Arc<dyn Completer>, pool: PgPool)`
- `generate(&self, job, life_sheet, options, progress_tx: Option<mpsc::Sender<ProgressEvent>>)`
- Returns `CoverLetterVersion`
- ProgressEvent: Generating, CheckingFabrication, Persisting, Done

### Completer Trait (`lazyjob-core/src/discovery/company.rs:16-18`)
- `async fn complete(&self, system: &str, user: &str) -> Result<String>`

## Architecture Decisions

1. **Worker subprocess**: Add hidden `Worker` subcommand to lazyjob-cli. The binary is `lazyjob`, process manager runs `lazyjob worker`.
2. **Event bridging**: App stores `ralph_tx: broadcast::Sender<RalphUpdate>`. Event loop creates RalphProcessManager, subscribes to it, maps `(RunId, WorkerEvent)` → `RalphUpdate`.
3. **Spawn from sync context**: App uses `mpsc::UnboundedSender<RalphCommand>` to send spawn/cancel requests. Event loop drains the channel and calls process_manager.spawn()/cancel().
4. **RalphUpdate::Started**: New variant to set loop_type on ActiveEntry properly.
5. **Params**: TUI passes `{ "job_id": "uuid-str", "database_url": "..." }` in params JSON.
