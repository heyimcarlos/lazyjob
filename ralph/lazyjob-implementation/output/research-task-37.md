# Research: Task 37 — Networking Contacts

## What Exists

### Domain Layer
- `Contact` type in `domain/contact.rs` with fields: id, name, role, email, linkedin_url, company_id, relationship, notes
- `ContactId` defined via `define_id!` macro in `domain/ids.rs`
- Both re-exported from `domain/mod.rs`

### Database
- `contacts` table exists in migration 001 with: id (UUID PK), name, role, email, linkedin_url, company_id (FK to companies), relationship, notes, created_at, updated_at
- Index on `company_id`

### Repository
- `ContactRepository` in `repositories/contact.rs` with full CRUD: insert, find_by_id, list, update, delete
- Uses `ContactRow` (sqlx::FromRow) with From<ContactRow> for Contact conversion
- Uses `Pagination` struct (limit/offset)

### TUI
- `ContactsView` is a stub in `views/contacts.rs` — unit struct, renders placeholder text, no key handling
- Registered in `Views` struct and `App::active_view_mut()` dispatch
- No `load_contacts()` in App, no contact-related `Action` variants

### Missing
- No `networking/` module in lazyjob-core
- No `csv` crate in workspace
- No `current_company` text field on contacts (only company_id FK)
- No warm path / connection mapping logic
- No CSV import capability

## Design Decisions

1. **Extend Contact type** — add `current_company: Option<String>` and `source: Option<String>` fields for CSV import support and warm path matching by company name
2. **Migration 006** — ALTER TABLE contacts ADD COLUMN current_company TEXT; ADD COLUMN source TEXT
3. **csv crate** — add to workspace deps for LinkedIn CSV parsing
4. **Warm path matching** — match contacts by `current_company` text against `job.company_name` (case-insensitive)
5. **ConnectionMapper** — pure function, no LLM needed; returns Vec<WarmPath> with contact name + role
6. **ContactsView** — scrollable table with name, company, role, email columns; j/k navigation
7. **JobDetailView** — add warm paths section below existing metadata when contacts exist at same company
