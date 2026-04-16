# Gap Analysis: SaaS / MVP (18-saas-migration-path, 19-competitor-analysis, 20-openapi-mvp, premium-monetization, AUDIENCE_JTBD, spec-inventory specs)

## Specs Reviewed
- `18-saas-migration-path.md` - 3-phase migration (Local → Sync → SaaS), Repository trait, AuthProvider, SyncOperation, multi-tenancy
- `19-competitor-analysis.md` - Huntr, Teal, Notion, Lazygit comparison, opportunities and threats
- `20-openapi-mvp.md` - 12-week single-engineer build plan, 6 phases, all specs cross-referenced
- `premium-monetization.md` - LinkedIn's $17.8B revenue model, InMail credit-back mechanism, tier pricing analysis
- `AUDIENCE_JTBD.md` - 4 audiences, 13 JTBDs, cross-cutting constraints (privacy, offline-first, human in loop, anti-fabrication, ghost job filtering)
- `spec-inventory.md` - 38 source specs → 33 output specs, implementation order recommendations

---

## What's Well-Covered

### 18-saas-migration-path.md
- 3-phase migration: Local-first → Add Sync Option → Full SaaS
- Repository trait for storage abstraction
- AuthProvider enum (Google, GitHub, EmailMagicLink, EnterpriseSSO)
- Plan enum (Free, Pro, Team, Enterprise)
- SyncOperation enum (Upsert, Delete, Merge) with conflict resolution
- Multi-tenancy design (tenant_id on all tables)
- EncryptedDatabase optional layer for local-first

### 20-openapi-mvp.md
- 12-week single-engineer build plan with clear phases
- P0/P1/P2 priority matrix (Ralph, Resume, Job Discovery = P0)
- Success criteria per phase with milestone definitions
- All specs cross-referenced, no spec left behind
- Specific tool recommendations (Apollo for data, Mailchimp for nurture)
- OpenAPI spec planned for Week 9

### premium-monetization.md
- LinkedIn's revenue model analysis ($17.8B, 72% from Talent Solutions)
- InMail credit-back mechanism (monthly rollover, refund on application rejection)
- Tier pricing analysis (Entry/Tier1/Tier2/Premium)
- Manufactured scarcity (limited InMail credits, premium features)
- Credit system mechanics (how credits accumulate and are consumed)

### AUDIENCE_JTBD.md
- 4 distinct audiences (Active Seeker, Passive Explorer, Career Changer, Network Dependent)
- 13 JTBDs with priority scores
- Cross-cutting constraints clearly identified (privacy, offline-first, human in loop, anti-fabrication, ghost job filtering)
- Audience-specific implementation considerations

### spec-inventory.md
- 38 source specs → 33 output specs (5 specs not needed)
- Redundancy analysis (LLM Provider Abstraction → Agentic LLM Provider Abstraction)
- Implementation order recommendations
- Clear spec relationship mapping

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-99: Freemium Model Specifics (CRITICAL)

**Location**: `18-saas-migration-path.md` - Plan enum (Free, Pro, Team, Enterprise) exists but no feature breakdown per tier

**What's missing**:
1. **Feature matrix per tier**: What's included in Free vs Pro vs Team vs Enterprise?
2. **Usage limits on Free**: How many jobs per month? How many applications? How many Ralph runs? Storage limit?
3. **Soft limits vs hard limits**: When free tier hits limit, what happens? (Warn then block? Immediate block? Upsell prompt?)
4. **Free tier quota tracking**: Where is quota tracked? How to display usage to user?
5. **Freemium conversion triggers**: When does upsell happen? (After 10 jobs created? After first Ralph run?)
6. **Trial period for Pro**: Is there a trial? (14-day free trial of Pro features?)
7. **Feature gating vs usage gating**: Some features gated by tier (Team workspaces), others by usage (extra job slots)

**Why critical**: Freemium is the acquisition funnel. Without clear limits, users get confused and either churn (hit hard limit unexpectedly) or never convert (don't see value).

**What could go wrong**:
- User creates 15 jobs, hits "limit", doesn't know what to do
- Free tier so limited it's useless, user abandons before experiencing value
- No upsell path visible, user never converts
- Enterprise features needed by Pro users, must upgrade to Team unnecessarily

---

### GAP-100: Data Portability and Exit Migration (CRITICAL)

**Location**: `18-saas-migration-path.md` - mentions "user owns their data" but no export spec; `spec-inventory.md` - no data portability spec

**What's missing**:
1. **Complete data export**: All user data in standard format (JSON/CSV). Jobs, applications, contacts, resumes, cover letters, Ralph conversation history
2. **Export granularity**: Can user export just jobs? Just contacts? Everything?
3. **Export format standard**: JSON schema with documented structure. CSV for tabular data. ZIP for multi-file export
4. **Scheduled automatic export**: Can user configure daily/weekly auto-export to their own storage?
5. **Exit migration path**: If user cancels, how do they export everything before deletion?
6. **Data retention after cancellation**: How long is data retained after subscription ends? (GDPR: must be exportable before deletion)
7. **Third-party import compatibility**: Is export format compatible with competitor import? (Huntr, Teal)
8. **Large export handling**: For users with years of data, how is large export handled? (Streaming? Chunked?)

**Why critical**: Data portability is a fundamental trust requirement. Users must be able to leave with their data. Without it, LazyJob is a walled garden.

**What could go wrong**:
- User data is trapped, can't leave even if they want
- Export fails for large datasets
- Export format undocumented, third parties can't build import tools
- User cancels, data deleted before they exported

---

### GAP-101: Team Shared Workspaces (IMPORTANT)

**Location**: `18-saas-migration-path.md` - Plan enum has Team tier; `spec-inventory.md` - no collaborative features spec

**What's missing**:
1. **Team formation flow**: How do users create a team? Invite members by email? Share link?
2. **Role-based access control**: Admin (full control), Member (create/edit own), Viewer (read-only). Custom roles?
3. **Shared job board**: Can multiple team members see the same jobs? Can one person add jobs visible to all?
4. **Shared application pipeline**: Can team members see each other's applications?
5. **Team analytics**: Aggregate success rates across team members. Who's getting more interviews? What's working?
6. **Admin controls**: Team settings, member management, billing for team seat
7. **Team templates**: Shared cover letter templates, shared reachout templates
8. **Individual vs team data**: Clear distinction between personal data and team-shared data
9. **Removing team members**: What happens to their data when removed? (Personal data stays, team data transferred)

**Why important**: Teams (job search clubs, professional networks, peer groups) need shared views. Solo users might eventually want to share with a partner or spouse.

**What could go wrong**:
- Team tier exists but no way to actually form a team
- No RBAC, all team members can delete everything
- Team analytics blend individual and shared data confusingly
- Member leaves, their application data lost or inaccessible

---

### GAP-102: Mobile Companion App Strategy (IMPORTANT)

**Location**: None of the specs address mobile

**What's missing**:
1. **Mobile app scope**: Is this a full-featured app or read-only companion? (MVP likely read-only: view jobs, check status, get notifications)
2. **iOS vs Android priority**: Which first? React Native or native?
3. **Data sync with TUI**: How does mobile stay in sync with desktop TUI? (Same SQLite file via cloud sync?)
4. **Feature parity roadmap**: What mobile features come after MVP? (Full apply on mobile? Mobile-only features?)
5. **Offline mobile experience**: Can user view jobs without internet? (Essential for commute/interview)
6. **Mobile-specific interactions**: Swipe to dismiss jobs? Touch to call contact? Push notification on status change?
7. **Mobile auth flow**: Biometric login on mobile? OAuth via mobile browser?
8. **App store distribution**: How to distribute? (TestFlight for iOS, internal beta for Android first?)

**Why important**: Mobile is where users spend most of their time. A read-only companion app extends LazyJob's presence beyond the desktop TUI.

**What could go wrong**:
- Mobile app announced but not specced, never ships
- Mobile app tries to be full TUI on phone, UX terrible
- Mobile and desktop get out of sync, user confused about actual status
- Push notifications too noisy, user disables them

---

### GAP-103: Collaborative Features - Shared Drafts (IMPORTANT)

**Location**: `networking-outreach-drafting.md` - human-in-loop is clear; `spec-inventory.md` - no collaboration spec

**What's missing**:
1. **Draft sharing**: Can user share a cover letter or outreach draft with a friend/mentor for review?
2. **Comment/feedback on shared drafts**: Can reviewer add comments without editing?
3. **Shared templates**: Team admin creates shared templates, members use them
4. **Collaborative filtering**: "Users who applied to X also applied to Y" - is this team-wide or global?
5. **Sharing permissions**: Can share with anyone (public link) or only authenticated users?
6. **Real-time collaboration**: Can two people edit same draft simultaneously?
7. **Version history for shared items**: Track who changed what on shared drafts

**Why important**: Job search is often supported by mentors, friends, spouses. They need to be able to review and comment on materials without having a full LazyJob account.

**What could go wrong**:
- User wants spouse to review cover letter, no way to share
- Shared draft link is public, sensitive personal data exposed
- Real-time collaboration attempted but conflicts are messy

---

### GAP-104: Usage-Based vs Seat-Based Billing Clarity (MODERATE)

**Location**: `18-saas-migration-path.md` - Plan enum exists; `premium-monetization.md` - subscription model but not clearly usage vs seat

**What's missing**:
1. **Seat-based or usage-based?**: Pro tier is per-user seat or per-month flat? Team tier is per-seat or flat?
2. **How seats are counted**: Can one user have multiple devices? (Desktop TUI + mobile app = one seat or two?)
3. **Overage pricing**: If team has 5 seats but 8 users, what happens? (Block extra users? Overage charge?)
4. **Annual vs monthly**: Is there a discount for annual billing? (Standard: 20% off annual)
5. **Usage-based add-ons**: Beyond seat limit, can extra usage be purchased? (Extra job slots, extra LLM calls)
6. **Billing metrics**: What exactly counts as "usage"? (Jobs created? Applications sent? Ralph runs?)

**Why important**: Configuous billing leads to churn and support tickets. Users must understand what they're paying for.

**What could go wrong**:
- User buys Pro thinking it's unlimited, hits "credits" limit, angry
- Team has 3 members, one uses most resources, billing confusion
- Annual billing doesn't clearly explain what's included

---

### GAP-105: Onboarding and First-Time Experience (MODERATE)

**Location**: `AUDIENCE_JTBD.md` - audience-specific needs identified; `20-openapi-mvp.md` - onboarding not specced as distinct phase

**What's missing**:
1. **First-run wizard**: What does first launch look like? (Connect LLM provider? Import LinkedIn? Create first job?)
2. **Import from competitors**: Can user import data from Huntr or Teal? (Critical for switching)
3. **Sample data for exploration**: Does first launch include sample job/application for user to explore?
4. **Product tour**: Do we show users the key features with guided tooltips?
5. **Onboarding completion metrics**: What counts as "activated" user? (Created first job? Sent first application?)
6. **Re-engagement for inactive users**: If user hasn't opened app in 7 days, trigger re-engagement email?
7. **Onboarding by audience**: Active Seeker gets different onboarding than Passive Explorer

**Why important**: Onboarding determines activation rate. Users who don't experience "aha moment" within first session churn.

**What could go wrong**:
- First launch is blank slate, user doesn't know what to do
- No import from competitors, switching cost is high
- Product tour is too long, user skips it and misses key features

---

### GAP-106: Enterprise SSO and Security Compliance (MODERATE)

**Location**: `18-saas-migration-path.md` - AuthProvider includes EnterpriseSSO; `19-competitor-analysis.md` - enterprise market mentioned

**What's missing**:
1. **SAML/OIDC integration**: Which SSO providers are supported? (Okta, Azure AD, Google Workspace, OneLogin?)
2. **SCIM provisioning**: Automated user provisioning/deprovisioning via SCIM?
3. **Audit log**: Enterprise admin needs to see who accessed what and when
4. **Data retention by policy**: Can enterprise set their own retention policy?
5. **IP allowlisting**: Can enterprise restrict access to specific IP ranges?
6. **SOC 2 compliance**: Type I or Type II? What's the compliance timeline?
7. **Security questionnaire process**: Enterprise procurement usually requires security questionnaire

**Why important**: Enterprise sales require security compliance. Without it, only small teams can use LazyJob.

**What could go wrong**:
- Enterprise prospect wants SSO, LazyJob doesn't support it, deal lost
- Security questionnaire can't be answered because policies don't exist
- Audit log missing, enterprise can't satisfy compliance requirements

---

### GAP-107: Webhook and API Extension Ecosystem (MODERATE)

**Location**: `20-openapi-mvp.md` - OpenAPI spec planned for Week 9, but webhook ecosystem not specced

**What's missing**:
1. **Webhook events**: What events can third-parties subscribe to? (Application stage change, new interview scheduled, offer received)
2. **Webhook delivery reliability**: How are webhooks delivered? (At-least-once? Retries? Dead letter queue?)
3. **API rate limits by tier**: Free: X calls/min, Pro: Y calls/min, Enterprise: Z calls/min
4. **Official integrations**: Zapier, Make, n8n integration templates
5. **Developer documentation**: OpenAPI spec, SDKs (Python, JS, Rust), sample code
6. **API versioning**: How to evolve API without breaking existing integrations?
7. **Self-hosted webhook receiver**: Can users run their own endpoint for webhooks?

**Why important**: API and webhooks enable integrations and extensibility. Third-party developers can build on LazyJob.

**What could go wrong**:
- Webhook fires, delivery fails, third-party integration misses important event
- Rate limits not communicated, integrations break silently
- API changes without versioning, breaks existing integrations

---

### GAP-108: SLA and Uptime Commitment (MODERATE)

**Location**: `18-saas-migration-path.md` - mentions "Full SaaS" but no SLA spec

**What's missing**:
1. **SLA tiers by plan**: Free: 99%? Pro: 99.5%? Team: 99.9%? Enterprise: 99.99%?
2. **Downtime definition**: Is "downtime" when API is unreachable? When TUI can't sync?
3. **Maintenance windows**: Scheduled downtime announcements? (Weekly? Monthly?)
4. **Incident response SLA**: How fast does team respond to P0/P1/P2 incidents?
5. **Compensation for SLA breach**: Service credits? Refund?
6. **Status page**: Is there a public status page (status.lazyjob.com)?
7. **Historical uptime**: Can users see past uptime metrics?

**Why important**: Enterprise customers require SLA. Free users expect reliability too.

**What could go wrong**:
- User builds critical workflow on LazyJob API, API goes down, no SLA to reference
- Status page doesn't exist, users don't know if issue is on their end or LazyJob's
- Maintenance window during business hours, angry users

---

### GAP-109: Infrastructure Scaling Strategy (MODERATE)

**Location**: `20-openapi-mvp.md` - mentions "cloud-native" but no infrastructure spec

**What's missing**:
1. **Database scaling**: PostgreSQL or SQLite? If PostgreSQL, sharding strategy? (Multi-tenant per schema or database-per-tenant?)
2. **Read replica strategy**: For global users, read replicas in multiple regions
3. **Cache layer**: Redis for session data? For LLM response caching?
4. **CDN for static assets**: User-uploaded resumes stored where? (S3? Cloudflare R2?)
5. **LLM request proxy**: Rate limiting and failover between Anthropic/OpenAI/Ollama
6. **Multi-region deployment**: If EU user, is their data stored in EU? (GDPR compliance)
7. **Cost optimization**: Reserved instances? Spot instances for non-critical workers?

**Why important**: Infrastructure decisions made early are hard to change. SaaS phase must plan for scale.

**What could go wrong**:
- Single-region deployment, EU users have high latency
- No cache strategy, LLM costs spike
- Database doesn't scale, performance degrades as users increase

---

## Cross-Spec Gaps

### Cross-Spec V: Billing System ↔ Plan Limits

No spec addresses how plan limits (job slots, LLM calls, storage) are tracked and enforced. The usage tracking system must integrate with the billing system.

**Affected specs**: `18-saas-migration-path.md`, (new billing spec needed)

### Cross-Spec W: Data Portability ↔ Encryption

If LazyJob encrypts local database with age, how does the SaaS sync handle encrypted blobs vs. server-side encryption? What's the export format for encrypted data?

**Affected specs**: `16-privacy-security.md`, `18-saas-migration-path.md`

---

## Specs to Create

### Critical Priority

1. **XX-freemium-model-specifics.md** - Feature matrix, usage limits, soft/hard limits, quota tracking, conversion triggers, trial periods
2. **XX-data-portability-export.md** - Complete data export, export formats, exit migration, retention after cancellation, large export handling

### Important Priority

3. **XX-team-shared-workspaces.md** - Team formation, RBAC, shared job board, team analytics, admin controls
4. **XX-mobile-companion-app.md** - Mobile scope, iOS/Android priority, TUI sync, offline experience, feature roadmap
5. **XX-collaborative-shared-drafts.md** - Draft sharing, comments, shared templates, collaboration permissions

### Moderate Priority

6. **XX-billing-pricing-model.md** - Seat-based vs usage-based, overage pricing, annual discount, billing metrics
7. **XX-onboarding-activation.md** - First-run wizard, competitor import, sample data, product tour, activation metrics
8. **XX-enterprise-security-compliance.md** - SSO/SAML, SCIM, audit log, IP allowlisting, SOC 2, security questionnaire
9. **XX-webhooks-api-ecosystem.md** - Webhook events, delivery reliability, rate limits, official integrations, API versioning
10. **XX-sla-uptime-commitment.md** - SLA tiers, downtime definition, maintenance windows, incident response, status page
11. **XX-infrastructure-scaling.md** - Database scaling, read replicas, cache layer, CDN, multi-region, cost optimization

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-99: Freemium Model Specifics | Critical | Medium | User acquisition |
| GAP-100: Data Portability/Exit | Critical | Medium | User trust |
| GAP-101: Team Shared Workspaces | Important | High | Team adoption |
| GAP-102: Mobile Companion App | Important | High | Always-on access |
| GAP-103: Collaborative Shared Drafts | Important | Medium | Mentor involvement |
| GAP-104: Billing Clarity | Moderate | Low | Churn reduction |
| GAP-105: Onboarding | Moderate | Medium | Activation rate |
| GAP-106: Enterprise Security | Moderate | High | Enterprise sales |
| GAP-107: Webhook/API Ecosystem | Moderate | Medium | Extensibility |
| GAP-108: SLA/Uptime | Moderate | Low | Trust |
| GAP-109: Infrastructure Scaling | Moderate | High | Performance at scale |