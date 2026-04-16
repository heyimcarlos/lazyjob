# Plan: Task 2 — Core Domain Types

## Files to create/modify

1. `lazyjob-core/src/error.rs` — CoreError enum + Result type alias
2. `lazyjob-core/src/domain/mod.rs` — re-exports all domain types
3. `lazyjob-core/src/domain/ids.rs` — all ID newtypes (JobId, ApplicationId, etc.)
4. `lazyjob-core/src/domain/job.rs` — Job struct
5. `lazyjob-core/src/domain/application.rs` — Application struct + ApplicationStage enum
6. `lazyjob-core/src/domain/company.rs` — Company struct
7. `lazyjob-core/src/domain/contact.rs` — Contact struct
8. `lazyjob-core/src/domain/interview.rs` — Interview struct
9. `lazyjob-core/src/domain/offer.rs` — Offer struct
10. `lazyjob-core/src/lib.rs` — add `pub mod domain;` and `pub mod error;`

## Types/functions to define

### error.rs
- `CoreError` enum (Db, Io, Parse, Validation, NotFound, Serialization)
- `type Result<T> = std::result::Result<T, CoreError>;`

### domain/ids.rs
- Macro `define_id!` to reduce boilerplate for 6 identical ID newtypes
- JobId, ApplicationId, CompanyId, ContactId, InterviewId, OfferId

### domain/job.rs
- `Job` struct

### domain/application.rs
- `ApplicationStage` enum (9 variants)
- `Application` struct

### domain/company.rs
- `Company` struct

### domain/contact.rs
- `Contact` struct

### domain/interview.rs
- `Interview` struct

### domain/offer.rs
- `Offer` struct

## Tests to write

### Unit tests (in each module)
- `ids::tests` — construct each ID, verify Display, verify serde round-trip
- `job::tests` — construct Job, serde round-trip
- `application::tests` — construct Application, serde round-trip, ApplicationStage serialization
- `company::tests` — construct Company, serde round-trip
- `contact::tests` — construct Contact, serde round-trip
- `interview::tests` — construct Interview, serde round-trip
- `offer::tests` — construct Offer, serde round-trip
- `error::tests` — verify CoreError Display output

No learning tests needed — uuid, chrono, serde are well-known crates already used in task 1.

## Migrations
None — this task is pure domain types, no persistence.
