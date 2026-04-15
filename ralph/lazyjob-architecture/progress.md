# Progress Log

Started: 2026-04-15
Objective: Deep architecture research for LazyJob — a lazygit-style job search TUI built in Rust

This ralph will produce 20 specs covering all aspects of LazyJob. The user will be away for 12 hours — go deep on everything.

IMPORTANT: Mark tasks done in tasks.json AND write the spec file to ../../specs/[spec-filename] BEFORE marking done. Both must happen.

---

## Task 1: Architecture Overview - COMPLETED

**Spec written**: `specs/01-architecture.md`

### Key Findings

**Ratatui Architecture (v0.30.0+)**:
- Modular workspace: ratatui-core, ratatui-widgets, ratatui-crossterm, ratatui-macros
- Core traits: `Widget` (stateless) and `StatefulWidget` (with external state)
- Layout system: `Layout::vertical()` / `Layout::horizontal()` with `Constraint` types (Length, Percentage, Fill, Min, Max)
- NO built-in event handling - apps must use crossterm::event directly
- Application pattern: `ratatui::run(|terminal| { loop { terminal.draw(...); handle_events(); } })`
- Styling: Builder pattern with `Style::new().fg(Color::Blue).bold()`
- Stateful widgets: List (ListState), Table (TableState), Scrollbar (ScrollbarState)

**Lazygit Patterns (Go, using gocui)**:
- View/window abstraction with context-aware keybindings
- Central app state struct passed through command chain
- Command pattern with undo support
- Selective keybinding system - different bindings per panel
- Vim-inspired key philosophy: space=primary action, enter=detail, hjkl=navigation

**Recommended Crate Structure (Option C - Three-Tier)**:
```
lazyjob/
├── lazyjob-core/      # Domain models, persistence, state machine
├── lazyjob-llm/       # LLM provider abstraction, prompts
├── lazyjob-ralph/     # Ralph loop IPC, subprocess management
├── lazyjob-tui/       # Terminal UI layer
├── lazyjob-cli/       # Binary entry point
└── lazyjob-macros/    # Procedural macros
```

**Dependency Graph**:
- lazyjob-cli → lazyjob-tui → lazyjob-ralph + lazyjob-core
- lazyjob-ralph → lazyjob-llm → lazyjob-core

### Critical Decision
Chose Option C (Three-Tier) over Options A (monolithic) and D (loom-style 30+ crates). This provides:
- Clean separation between UI and business logic
- Ability to run core headlessly in future
- Path to multi-UI (TUI, web, API)
- Manageable complexity for small team

---

## Task 2: LLM Provider Abstraction - COMPLETED

**Spec written**: `specs/02-llm-provider-abstraction.md`

### Key Findings

**async-openai (OpenAI SDK)**:
- Client configured via `OpenAIConfig` with API key, org, base URL
- Chat completions via `client.chat().create(request)`
- Streaming via `client.chat().create_stream(request)` - returns SSE stream
- Embeddings via `client.embeddings().create(request)`
- Does NOT support Anthropic - separate implementation needed

**ollama-rs (Local Ollama)**:
- `Ollama::default()` for localhost:11434, or custom endpoint
- `send_chat_messages_with_history()` for chat
- `generate_stream()` for streaming
- `generate_embeddings()` for embeddings
- Supports function calling via `Coordinator` and `#[ollama_rs::function]` macro

**Anthropic API differences**:
- Uses `/v1/messages` endpoint (not `/chat/completions`)
- Requires `anthropic-version` header
- Different SSE event types: `message_start`, `content_block_delta`, `message_stop`
- Does NOT have embeddings API (as of 2024)
- Generally slower response times

**Trait Design (Option B - Recommended)**:
```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError>;
    async fn chat_stream(&self, messages: Vec<ChatMessage>) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, LLMError>> + Send>>, LLMError>;
    async fn complete(&self, prompt: &str) -> Result<String, LLMError> { ... }
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LLMError>;
    fn context_length(&self) -> u32;
}
```

**Provider Registry Pattern**:
- HashMap of provider name to `Arc<dyn LLMProvider>`
- Builder pattern for setup from config
- Fallback support when providers fail

### Open Questions (Task 2)
1. Embedding provider - Anthropic doesn't offer embeddings. Should we always use OpenAI or use Ollama locally?
2. Cost tracking - normalize across providers?
3. Caching strategy?
4. Global rate limiter?

---

## Task 3: Life Sheet Data Model - COMPLETED

**Spec written**: `specs/03-life-sheet-data-model.md`

### Key Findings

**JSON Resume Schema** (well-documented standard):
- Basics: name, label, email, phone, location, summary, profiles[]
- Work: company, position, dates, summary, highlights[]
- Education: institution, degree, field, dates, courses[]
- Skills: name, level, keywords[] (flat, no taxonomy codes)
- Languages: name, fluency

**ESCO** (EU Skills Taxonomy):
- URI-based identification for skills/occupations
- 27 EU languages supported
- Free REST API available
- Skills linked to occupations
- Structure: title, description, type, group, preferredLabel, alternativeLabels

**O*NET** (US Labor Dept):
- Content Model: Abilities, Skills, Knowledge, Work Activities, Tasks, Tools
- 1,016 occupational titles
- Skills have importance (1-5) and level (1-7) scales
- Free database downloads and API
- Quarterly updates

**Recommended Schema Design (Option B)**:
- Rich YAML format with: context (team_size, org_size, industry), achievements (with metrics), taxonomy codes (ESCO/O*NET)
- Separate SQLite for programmatic access
- Application linking (which jobs each experience is relevant to)
- Preferences section for job search criteria
- Goals section for career planning

---

## Task 4: SQLite Persistence - COMPLETED

**Spec written**: `specs/04-sqlite-persistence.md`

### Key Findings

**rusqlite (Synchronous)**:
- Connection::open() for file-based DB
- Prepared statements with params! macro
- Transaction support via conn.transaction()
- Savepoints for nested transactions
- busy_timeout for lock contention
- WAL mode for concurrent reads
- Backup API for online backups

**sqlx (Async, Recommended)**:
- SqlitePool with connection pooling
- Compile-time query checking via macros
- Migrator for schema migrations
- SqlitePoolOptions for configuration
- `runtime-tokio` feature for async

**WAL Mode**:
- `PRAGMA journal_mode=WAL`
- Concurrent reads during writes
- Checkpoint modes: PASSIVE, FULL, RESTART, TRUNCATE
- WAL hooks available in rusqlite

**Migration Patterns**:
- rusqlite_migration: uses user_version PRAGMA
- sqlx migrate: `Migrator::new(Path::new("./migrations")).await?`
- Migration files: `{version}_{name}.sql`

**Backup Strategy**:
- rusqlite backup::Backup for online backup
- Periodic VACUUM for compaction
- WAL file existence check on startup (dirty shutdown indicator)
- Auto-backup on startup if needed

### Open Questions (Task 4)
1. Query complexity - SQL vs in-memory filtering?
2. Full-Text Search with SQLite FTS5?
3. Ralph connection pooling strategy?
4. Backup retention policy?

### Notes for Future Iterations
- **Task 5 (Job Discovery)**: COMPLETED - see below
- **Task 6 (Ralph Loop Integration)**: Critical path - novel research on TUI + agent subprocess IPC. No prior art to reference locally.
- LLM trait design needs more depth (Task 2)
- The TUI view hierarchy is sketched but not fully detailed (Task 9 covers TUI design)

---

## Task 5: Job Discovery Layer - COMPLETED

**Spec written**: `specs/05-job-discovery-layer.md`

### Key Findings

**Greenhouse API** (Public, no auth):
- Endpoint: `GET /v1/boards/{board_token}/jobs`
- `content=true` includes full job description
- Board token is company identifier in URL
- High quality, structured data

**Lever API** (Public, no auth):
- Endpoint: `GET /v0/postings/{company}?mode=json`
- Structured categories (commitment, team, department)
- Plain text alternatives to HTML

**JobSpy**: Python scraping library for LinkedIn/Indeed/Glassdoor
- NOT recommended for production
- LinkedIn scraping is against ToS
- Fragile, unreliable

**Embedding for Job Matching**:
- text-embedding-ada-002 (1536 dim) or nomic-embed-text (768 dim, local)
- Cosine similarity for matching
- For LazyJob's scale (100s-1000s jobs), in-memory is sufficient
- No need for dedicated vector DB like Chroma/Qdrant

**Recommended Approach (Phase 1 MVP)**:
- API aggregation only (Greenhouse + Lever)
- Manual job entry for other companies
- No scraping
- In-memory/SQLite cosine similarity for matching

---

## Task 6: Ralph Loop Integration - COMPLETED

**Spec written**: `specs/06-ralph-loop-integration.md`

### Key Findings

**Tokio Process Management**:
- `tokio::process::Command` with `Stdio::piped()` for async process spawning
- `BufReader::lines()` for reading stdout as async line stream
- `child.kill().await` for process termination
- `child.wait()` for waiting on exit

**Unix Domain Sockets** (alternative IPC):
- `tokio::net::UnixListener` + `UnixStream` for socket-based IPC
- Better for long-lived connections, survives TUI restart
- More complex (socket file management)

**Recommended Approach (Option A - Stdio JSON Protocol)**:
- Ralph spawned as subprocess, communicates via newline-delimited JSON
- TUI → Ralph: Commands on stdin
- Ralph → TUI: Events on stdout
- Shared SQLite for state persistence
- Simple, language-agnostic, easy to debug

**Protocol Messages**:
```json
// Start
{"type": "start", "loop": "job_discovery", "params": {...}}
// Status
{"type": "status", "phase": "searching", "progress": 0.5}
// Results
{"type": "results", "data": {...}}
// Done
{"type": "done", "success": true}
```

**Loop Types**: JobDiscovery, CompanyResearch, ResumeTailor, CoverLetterGeneration, InterviewPrep, SalaryNegotiation, Networking

### Open Questions (Task 6)
1. Ralph as separate crate or completely separate binary?
2. How does Ralph get LLM API keys?
3. Progress persistence for crash recovery?
4. Limit concurrent loops per type?
5. Ralph logging strategy?

### Notes for Future Iterations

---

## Task 7: Resume Tailoring Pipeline - COMPLETED

**Spec written**: `specs/07-resume-tailoring-pipeline.md`

### Key Findings

**docx-rs** (Word Document Generation):
- Builder pattern: `Docx::new().add_paragraph(...).build().pack(file)`
- Run = styled text: `Run::new().bold().size(28).color("FF0000")`
- Paragraph, Block (for borders/titles), Table elements
- Supports headers/footers, images, sections

**ATS Resume Parsing**:
- Extract text from DOC/PDF, identify sections via NLP/rule-based
- Score based on keyword density, exact matches, synonym recognition
- Key factors: keyword frequency, section headers, proper dates, file format

**Pipeline Steps**:
1. Parse JD with LLM → structured requirements
2. Analyze life sheet → relevant experiences/skills
3. Gap analysis → missing skills, emphasized experiences
4. Draft content → rewrite bullets with JD keywords
5. Generate DOCX with docx-rs
6. Validation with fabrication guardrails

**Fabrication Guardrails**:
- Safe: Based on real data, just reworded
- Acceptable: "familiar with X" claims
- Risky: Cannot claim without evidence
- Forbidden: Never fabricate (licenses, certs)

### Open Questions (Task 7)
1. PDF/DOCX resume parsing for importing existing resumes?
2. Version tracking strategy?
3. Custom formatting templates?
4. Cover letter integration?

---

## Task 8: Cover Letter Generation - COMPLETED

**Spec written**: `specs/08-cover-letter-generation.md`

### Key Findings

**Cover Letter Structure** (Problem-Solution Format):
1. Opening hook: specific role + where found
2. Company research paragraph: show you've done homework
3. Value proposition: connect background to their needs
4. Specific achievements: 2-3 bullets with quantified results
5. Closing: call to action

**Company Research for Personalization**:
- Sources: website, LinkedIn, Crunchbase, Glassdoor, TechCrunch
- LLM synthesis: extract mission, values, culture signals, tech hints
- Personalization hooks: 2-3 specific things to mention

**Templates**:
- Standard Professional: Traditional format
- Problem-Solution (Recommended): Lead with company's problem
- Career Changer: Address non-traditional background

### Open Questions (Task 8)
1. Quick draft mode (skip research)?
2. Detect "no cover letter needed" from job posting?
3. A/B testing with multiple variants?
4. ATS compatibility for cover letters?

### Notes for Future Iterations

---
