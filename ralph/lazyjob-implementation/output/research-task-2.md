# Research: Task 2 — Core Domain Types

## What we're building
All domain model types for lazyjob-core in a `domain/` module directory.

## Types to define

### ID Newtypes (parse-don't-validate pattern from rust-patterns.md)
- `JobId(Uuid)` — wraps uuid::Uuid
- `ApplicationId(Uuid)`
- `CompanyId(Uuid)`
- `ContactId(Uuid)`
- `InterviewId(Uuid)`
- `OfferId(Uuid)`

All need: Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Display.
Constructor: `new()` generates v4 UUID. No validation needed beyond UUID format.

### Enums
- `ApplicationStage` — 9 variants per task 5 spec: Interested, Applied, PhoneScreen, Technical, Onsite, Offer, Accepted, Rejected, Withdrawn

### Domain structs
- `Job` — id, title, company_id (optional), location, url, description, salary_min/max, source, source_id, match_score, ghost_score, discovered_at, notes
- `Application` — id, job_id, stage, submitted_at, updated_at, resume_version, cover_letter_version, notes
- `Company` — id, name, website, industry, size, tech_stack, culture_keywords, notes
- `Contact` — id, name, role, email, linkedin_url, company_id (optional), relationship, notes
- `Interview` — id, application_id, interview_type, scheduled_at, location, notes, completed
- `Offer` — id, application_id, salary, equity, benefits, deadline, accepted, notes

### Error type
- `CoreError` with thiserror — variants: Db, Io, Parse, Validation, NotFound, Serialization
- `type Result<T> = std::result::Result<T, CoreError>;`

## Dependencies needed
Already in lazyjob-core/Cargo.toml: uuid, chrono, thiserror, anyhow, serde, serde_json, tokio.
No new dependencies needed.

## Key decisions
- Use chrono::DateTime<Utc> for timestamps (spec uses chrono)
- Use Option<f64> for match_score/ghost_score (not yet computed)
- Use String for most text fields (no newtypes for email, url, etc. — YAGNI for MVP)
- ApplicationStage gets serde rename_all = "snake_case" for clean serialization
- ID newtypes use Deref to Uuid for convenience but maintain type safety
