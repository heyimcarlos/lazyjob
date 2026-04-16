# Gap Analysis: Platform / Privacy (11, agent-interfaces-*, 16, job-platforms-comparison specs)

## Specs Reviewed
- `11-platform-api-integrations.md` - Platform API integrations (Greenhouse, Lever)
- `agent-interfaces-job-platforms.md` - Deep research on job platform APIs, scraping, auto-apply
- `16-privacy-security.md` - Privacy and security design
- `job-platforms-comparison.md` - Job platform competitive landscape

---

## What's Well-Covered

### agent-interfaces-job-platforms.md
- Tier 1-4 platform classification
- Greenhouse and Lever API specs (clean, documented)
- Auto-apply tool landscape (LazyApply, Sonara, Scale.jobs, Simplify)
- Browser automation detection landscape (CDP detection, stealth plugins, rebrowser)
- Legal/ToS analysis (hiQ v LinkedIn, Proxycurl precedent)
- Data access architecture (safe/gray/risky tiers)
- Cost estimates for infrastructure
- Clear safe harbor recommendations

### 11-platform-api-integrations.md
- PlatformClient trait design
- GreenhouseClient and LeverClient implementations
- RateLimiter with request-per-minute control
- Data quality matrix

### 16-privacy-security.md
- Encryption approaches (SQLite SEE, SQLCipher, age)
- Keyring integration (libsecret, macOS Keychain, Windows Credential Manager)
- CredentialManager with get/set/delete API keys
- EncryptedDatabase design with optional encryption
- Database export (always decrypted - user owns data)
- PrivacyMode enum (Full, Minimal, Stealth)
- SecureStorage with error handling
- Failure modes and fallbacks

### job-platforms-comparison.md
- Indeed (aggregation, 350M visitors), Glassdoor (reviews, salary), ZipRecruiter (SMB, Phil AI), Wellfound (startups, transparency), Handshake (early-career), Hired (reverse marketplace)
- Cross-platform usage patterns
- Ghost job problem (18-27% across platforms)
- Competitive dynamics and consolidation trends

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-89: Encrypted Backup and Export (CRITICAL)

**Location**: `16-privacy-security.md` - backup method mentioned but encrypted backup not specced

**What's missing**:
1. **Encrypted backup format**: If database is encrypted with age, should backups also be encrypted?
2. **Backup key management**: Who holds the backup encryption key? How is it derived from user password?
3. **Encrypted export**: If user exports data to JSON, is it decrypted or encrypted?
4. **Secure deletion**: When user deletes data, are temp files securely wiped?
5. **Backup restoration**: How does restore work with encryption? Master password required?
6. **Cloud backup integration**: If user backs up to Google Drive/Dropbox, how is the backup encrypted?

**Why critical**: If database is encrypted but backups aren't, the encryption is pointless.

**What could go wrong**:
- Backup file is stored unencrypted on disk
- Temp files with sensitive data left behind after export
- User loses encryption key, can't restore backup
- Backup uploaded to cloud unencrypted

---

### GAP-90: Master Password for Application Unlock (CRITICAL)

**Location**: `16-privacy-security.md` - Open Question #2: "Should we require a master password for the app?" - no resolution

**What's missing**:
1. **Password-derived encryption key**: Master password → derive encryption key (argon2, scrypt)
2. **App unlock flow**: On startup, prompt for password before decrypting database
3. **Session timeout**: Lock app after N minutes of inactivity
4. **Password fallback**: If password forgotten, what's the recovery path? (Local-only data = no recovery)
5. **Password strength requirements**: Minimum password requirements?
6. **Biometric unlock**: On macOS, can use Touch ID instead of password?
7. **Password change**: How to change master password?

**Why critical**: If LazyJob stores sensitive data (resumes, job applications, contacts), it should require authentication to access.

**What could go wrong**:
- Sensitive data accessible to anyone with file system access
- No authentication on app launch, data visible if laptop stolen
- User forgets password, all data lost permanently

---

### GAP-91: Multi-Device Sync for Encrypted Data (IMPORTANT)

**Location**: `16-privacy-security.md` - Open Question #1: "Cloud Sync" - no resolution

**What's missing**:
1. **Sync architecture**: If LazyJob is local-only, how do users sync across machines?
2. **Encrypted sync**: Can encrypted database be synced via iCloud/Dropbox/NextCloud?
3. **Sync conflicts**: If same database modified on two machines, how to resolve?
4. **Key distribution**: How does second machine get the encryption key?
5. **Selective sync**: Can users choose which data to sync?
6. **SaaS relay option**: For users who want sync, is there an optional cloud relay?

**Why important**: Users work on multiple machines (desktop + laptop). Local-only data is a limitation.

**What could go wrong**:
- User manually copies database between machines, risks data loss
- Sync service stores unencrypted data on cloud
- Conflict resolution is unclear, data lost or duplicated

---

### GAP-92: LinkedIn OAuth "Apply with LinkedIn" (IMPORTANT)

**Location**: `agent-interfaces-job-platforms.md` - LinkedIn automation is ToS-violating, but official OAuth isn't addressed

**What's missing**:
1. **OAuth 2.0 flow**: LinkedIn's "Apply with LinkedIn" uses OAuth. Is this available to third-party apps?
2. **Profile data access**: Via OAuth, what profile data can LazyJob legitimately access?
3. **Application submission**: Can OAuth be used to submit applications to LinkedIn Easy Apply?
4. **Connection data**: Can OAuth access user's LinkedIn connections (for networking)?
5. **API rate limits**: What's the OAuth rate limit vs. scraping?
6. **User consent**: What's the OAuth consent flow for LazyJob users?

**Why important**: LinkedIn is where most job seekers have their professional identity. OAuth provides a legitimate path.

**What could go wrong**:
- OAuth scope not available for job seeker apps, only recruiter products
- User doesn't understand what data LazyJob accesses via OAuth
- OAuth token expires, discovery fails silently

---

### GAP-93: Browser Fingerprinting and Evasion (IMPORTANT)

**Location**: `agent-interfaces-job-platforms.md` - mentions stealth plugins and rebrowser but no LazyJob-specific strategy

**What's missing**:
1. **Detection assessment**: How likely is LazyJob to be detected if using Playwright?
2. **Stealth strategy**: What level of stealth is appropriate? (fingerprint blocking? residential proxies?)
3. **Proxy infrastructure**: If proxies needed, which provider? How managed?
4. **Session management**: How to handle session isolation and rotation?
5. **Failure handling**: When detection occurs, what's the recovery?
6. **Workday-specific**: Workday uses which anti-bot systems?

**Why important**: If browser automation is used for Workday/custom ATS, detection is a real risk.

**What could go wrong**:
- User's IP banned from Workday
- LazyJob flagged as bot, all sessions from that IP blocked
- Stealth implementation breaks legitimate browsing features

---

### GAP-94: Data Retention and Deletion Policy (MODERATE)

**Location**: `16-privacy-security.md` - no explicit retention/deletion policy

**What's missing**:
1. **Retention periods**: How long is each data type kept? (Job listings, applications, contacts)
2. **Deletion cascade**: When user deletes a job, what happens to linked applications, resumes, cover letters?
3. **Secure deletion**: Does deletion actually remove data or just mark as deleted?
4. **GDPR compliance**: Right to deletion, data portability
5. **Legal hold**: If data is under legal investigation, can it be deleted?
6. **Backup retention**: How long are backups kept?

**Why important**: Without a retention policy, data accumulates indefinitely and deletion is ambiguous.

**What could go wrong**:
- Deleted job reappears because linked data wasn't deleted
- User thinks data is deleted but it's still in backups
- GDPR request can't be fulfilled because retention policy is unclear

---

### GAP-95: Third-Party LLM Provider Data Handling (MODERATE)

**Location**: `16-privacy-security.md` - no spec for what LLM providers do with data sent to them

**What's missing**:
1. **Data sent to LLM**: What data is sent to Anthropic/OpenAI/Ollama? (Resumes, job descriptions, personal info)
2. **Provider data policies**: What do these providers do with input data? (Training? Logging?)
3. **PII handling**: Is personal information handled safely by LLM providers?
4. **Ollama as privacy option**: Local Ollama means data never leaves machine - this should be highlighted
5. **Provider selection UI**: When users configure LLM, show privacy implications of each choice
6. **Corporate/proxy concerns**: If user uses LLM behind corporate proxy, what data is logged?

**Why important**: Sending personal data to LLM providers has privacy implications.

**What could go wrong**:
- User sends resume to OpenAI, doesn't realize it may be used for training
- Corporate LLM usage logs all prompts centrally
- Personal data sent to third-party providers without user's informed consent

---

### GAP-96: Crash Reports and Telemetry (MODERATE)

**Location**: `16-privacy-security.md` - not mentioned at all

**What's missing**:
1. **Crash reporting**: If LazyJob crashes, is a report sent? What data does it contain?
2. **Opt-in telemetry**: Is there any usage telemetry? What does it collect?
3. **Error anonymization**: Are error reports anonymized before sending?
4. **Self-hosted crash collection**: Can users run their own crash collection server?
5. **Sensitive data in crashes**: Can stack traces contain sensitive data? How sanitized?
6. **GDPR for telemetry**: If EU user, how does telemetry comply?

**Why important**: Crash reports are common in software but need privacy controls.

**What could go wrong**:
- Crash report contains user's API keys or personal data
- Telemetry enabled by default, privacy-violating
- User doesn't know crash reports are being sent

---

### GAP-97: Workday Integration Strategy (MODERATE)

**Location**: `11-platform-api-integrations.md` - Workday mentioned as challenge; `agent-interfaces-job-platforms.md` - browser automation as fallback

**What's missing**:
1. **Workday detection**: How does LazyJob know a company uses Workday? (URL pattern?)
2. **Workday scraping feasibility**: Workday is heavily JavaScript-rendered. Is headless browser sufficient?
3. **Credential management**: For Workday, user needs to provide credentials. How stored?
4. **Form completion automation**: Workday forms vary by company. What's the automation approach?
5. **Error handling**: Workday sessions expire, CAPTCHAs appear. How handled?
6. **Alternatives to scraping**: Is there any official Workday API for job seekers?

**Why important**: Workday powers 39% of Fortune 500 job listings. Can't be ignored.

**What could go wrong**:
- Workday blocks all automation attempts
- User provides credentials but they don't work
- Workday blocks IP after first automation attempt

---

### GAP-98: Job Aggregator Cost-Benefit Analysis (MODERATE)

**Location**: `agent-interfaces-job-platforms.md` - mentions Adzuna (free tier), Jobo/Fantastic Jobs (enterprise pricing)

**What's missing**:
1. **Actual pricing**: What do Jobo and Fantastic Jobs cost? Is there a per-job or subscription model?
2. **Free alternatives**: Are there other free job aggregator APIs beyond Adzuna?
3. **Scraping as fallback**: If paid aggregators are too expensive, is open scraping (JobSpy) viable at scale?
4. **Cost per job calculation**: If using Apify at $0.005/result, what's the monthly cost for discovering 500 jobs?
5. **Tier strategy**: Which sources are worth paying for vs. free scraping?

**Why important**: Job discovery costs can become significant at scale.

**What could go wrong**:
- Job discovery budget spirals without clear cost control
- Free tier hits rate limits, discovery stops
- Enterprise pricing for aggregators is out of reach for individual users

---

## Cross-Spec Gaps

### Cross-Spec T: Encryption Key Management

The encryption design in `16-privacy-security.md` mentions encryption but doesn't specify:
- How is the encryption key derived?
- Is there a keyfile or password-derived key?
- How does the key survive app restarts but not compromise security?

**Affected specs**: `16-privacy-security.md`, (database specs)

### Cross-Spec U: API Key Storage Across Providers

LLM providers (Anthropic, OpenAI, Ollama) and platform APIs (Greenhouse, Lever) all need credentials. There's no unified credential storage spec.

**Affected specs**: `16-privacy-security.md`, `02-llm-provider-abstraction.md`

---

## Specs to Create

### Critical Priority

1. **XX-encrypted-backup-export.md** - Encrypted backup format, key management, secure deletion, cloud backup
2. **XX-master-password-app-unlock.md** - Password-derived encryption, app unlock flow, session timeout, biometric unlock

### Important Priority

3. **XX-multi-device-sync-encrypted.md** - Encrypted sync architecture, conflict resolution, key distribution
4. **XX-linkedin-oauth-integration.md** - Apply with LinkedIn OAuth, profile data access, user consent
5. **XX-browser-fingerprinting-evasion.md** - Detection assessment, stealth strategy, proxy management

### Moderate Priority

6. **XX-data-retention-deletion.md** - Retention periods, deletion cascade, GDPR compliance, secure deletion
7. **XX-llm-provider-privacy.md** - Data sent to providers, provider policies, Ollama as privacy option
8. **XX-crash-reports-telemetry.md** - Crash reporting setup, anonymization, opt-in telemetry
9. **XX-workday-integration-strategy.md** - Workday detection, scraping feasibility, credential management
10. **XX-job-aggregator-cost-analysis.md** - Pricing comparison, free tiers, cost per job calculation

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-89: Encrypted Backup/Export | Critical | Medium | Data security |
| GAP-90: Master Password Unlock | Critical | Medium | App security |
| GAP-91: Multi-Device Sync | Important | High | UX/multi-machine |
| GAP-92: LinkedIn OAuth | Important | Medium | Platform coverage |
| GAP-93: Browser Fingerprinting | Important | High | Automation viability |
| GAP-94: Data Retention/Deletion | Moderate | Low | Legal compliance |
| GAP-95: LLM Provider Privacy | Moderate | Low | Privacy awareness |
| GAP-96: Crash Reports/Telemetry | Moderate | Low | Debugging/privacy |
| GAP-97: Workday Integration | Moderate | High | Platform coverage |
| GAP-98: Aggregator Costs | Moderate | Low | Cost planning |
