# Plan: Task 37 — Networking Contacts

## Files to Create
1. `crates/lazyjob-core/migrations/006_contacts_networking.sql` — add current_company, source columns
2. `crates/lazyjob-core/src/networking/mod.rs` — module root, re-exports
3. `crates/lazyjob-core/src/networking/types.rs` — WarmPath, ImportResult, ContactSource
4. `crates/lazyjob-core/src/networking/csv_import.rs` — LinkedIn CSV parser
5. `crates/lazyjob-core/src/networking/connection_mapper.rs` — warm path finder

## Files to Modify
1. `Cargo.toml` — add csv = "1" to workspace deps
2. `crates/lazyjob-core/Cargo.toml` — add csv = { workspace = true }
3. `crates/lazyjob-core/src/domain/contact.rs` — add current_company, source fields
4. `crates/lazyjob-core/src/repositories/contact.rs` — update queries for new fields, add find_by_company()
5. `crates/lazyjob-core/src/lib.rs` — add pub mod networking
6. `crates/lazyjob-tui/src/views/contacts.rs` — full implementation with table
7. `crates/lazyjob-tui/src/app.rs` — add load_contacts()
8. `crates/lazyjob-tui/src/event_loop.rs` — call load_contacts() on refresh
9. `crates/lazyjob-tui/src/lib.rs` — call load_contacts() on startup
10. `crates/lazyjob-tui/src/views/job_detail.rs` — add warm path section

## Types to Define
- `WarmPath { contact_name, contact_role, contact_email, score }` — warm connection to a company
- `ImportResult { imported, skipped, errors }` — CSV import stats
- `parse_linkedin_csv(reader) -> Result<Vec<Contact>>` — CSV parsing function
- `warm_paths_for_job(contacts, job) -> Vec<WarmPath>` — connection matching

## Tests
- Learning test: csv crate parsing proves Reader::from_reader works with headers
- Unit: parse_linkedin_csv with fixture CSV data
- Unit: import deduplicates by email
- Unit: warm_paths_for_job returns contacts at same company
- Unit: warm_paths_for_job case-insensitive matching
- Unit: ContactsView renders table with contacts
- Unit: ContactsView j/k navigation
- Unit: JobDetailView renders warm paths when available

## Migration
```sql
ALTER TABLE contacts ADD COLUMN IF NOT EXISTS current_company TEXT;
ALTER TABLE contacts ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';
```
