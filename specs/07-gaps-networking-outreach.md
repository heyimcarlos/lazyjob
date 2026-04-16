# Gap Analysis: Networking & Outreach (networking-*, messaging-inmail specs)

## Specs Reviewed
- `networking-referrals-agentic.md` - Research on networking landscape and agentic opportunities
- `networking-connection-mapping.md` - Connection mapping to warm paths
- `networking-outreach-drafting.md` - Outreach message drafting pipeline
- `networking-referral-management.md` - Relationship stage machine and reminder poller
- `messaging-inmail.md` - Deep research on LinkedIn messaging/InMail architecture

---

## What's Well-Covered

### networking-connection-mapping.md
- ConnectionMapper with company name normalization matching
- Warmth tiers (FirstDegreeCurrentEmployee → Cold) with scoring
- SuggestedApproach per tier (RequestReferral, InformationalInterview, ReconnectFirst, ColdOutreach)
- LinkedIn CSV export import (no scraping - hard constraint)
- Second-degree heuristic approximation (flagged as low confidence)
- `contact_source` column tracking import origin

### networking-outreach-drafting.md
- Three-phase pipeline: context assembly → LLM drafting → validation
- SharedContext computed via pure structural comparison (no LLM) - excellent anti-fabrication
- Anti-fabrication prompt rules (hedged language, no invented facts)
- Medium-specific length enforcement (LinkedIn ≤300 chars, Email 100-300 words)
- Tone calibration mapped to SuggestedApproach
- No automation guarantee (hard product constraint: LazyJob never sends)

### networking-referral-management.md
- RelationshipStage state machine (Identified → Contacted → Replied → Warmed → ReferralAsked → ReferralResolved)
- ReferralReadinessChecker with 5 readiness criteria
- NetworkingReminderPoller tokio task with configurable interval
- Anti-spam guardrails (2 reminders per contact per 30 days, max 5 new contacts/week)
- Referral outcome integration (Succeeded, Declined, NoResponse, NotApplicable)
- Per-(contact, job) referral tracking via referral_asks table

### networking-referrals-agentic.md
- Comprehensive research on referral paradox (7-18x hire likelihood)
- Cold outreach problem analysis (3-5% response rates)
- Agentic opportunities identified: Network Mapping, Outreach Drafting, Relationship Maintenance, Referral Identification, Informational Interview Prep
- Product design principle: "agent suggests, human approves, human sends"
- Legal/ToS constraints clearly documented (CAN-SPAM, GDPR, LinkedIn ToS)

### messaging-inmail.md
- Deep technical architecture of LinkedIn messaging (microservices, SSE, presence platform)
- InMail credit system analysis (refund mechanism is smart design)
- Anti-spam mechanisms and "LinkedIn Jail" consequences
- Competitive landscape (email, X DMs, Slack, WhatsApp)
- Agent platform applicability analysis

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-69: Multi-Source Contact Import (CRITICAL)

**Location**: `networking-connection-mapping.md` - only covers LinkedIn CSV import and manual entry

**What's missing**:
1. **Email contact import**: Gmail contacts API? Apple Contacts? Users have contacts scattered across email providers
2. **Phone contact import**: Mobile contacts sync - address book integration
3. **Business card scanning**: Text extraction from business card photos (Gemalto?手动?)
4. **vCard import**: Standard .vcf format for contact exchange
5. **CSV import formats**: Beyond LinkedIn CSV - what about other platforms' export formats?
6. **Incremental import**: Import new contacts from ongoing exports without duplicating existing ones
7. **Contact merge on import**: If same person imported from multiple sources (LinkedIn + email), merge automatically

**Why critical**: Users have contacts in many places. LinkedIn CSV alone misses people the user knows from email, phone, conferences, etc.

**What could go wrong**:
- User has 500 LinkedIn contacts but 200 more from email not imported
- Same person imported twice from LinkedIn and email, treated as separate contacts
- Import process loses data (only imports name, misses company/title/email)

---

### GAP-70: Relationship Decay Tracking and Visualization (IMPORTANT)

**Location**: `networking-referral-management.md` - days_since_last_interaction tracked but decay model not specified

**What's missing**:
1. **Relationship decay model**: Is relationship decay linear? Exponential? What factors accelerate/decay?
2. **Decay visualization**: Can user see a "relationship health" score or decay timeline for each contact?
3. **Staleness detection**: Beyond "contacted > 7 days", what's the full decay curve?
4. **Decay reversal**: Does one interaction reset decay completely or partially?
5. **Inactive contact detection**: When is a contact considered "dormant"? Should dormant contacts be archived or flagged differently?
6. **Decay影响因素**: Does company tenure affect decay? (Employee who left 2 years ago - relationship decays faster?)

**Why important**: The ReferralReadinessChecker uses a simple 7/14/21-day threshold but doesn't model relationship decay dynamics.

**What could go wrong**:
- User hasn't contacted a warm lead in 6 months, LazyJob still treats them as viable referral path
- Relationship decay isn't uniform - some contacts go stale faster than others
- No visual representation of relationship health, users can't prioritize

---

### GAP-71: LinkedIn Connection Automation within ToS (IMPORTANT)

**Location**: `networking-referrals-agentic.md` - clearly states "no LinkedIn API access, no scraping"; `networking-outreach-drafting.md` - no automation guarantee; but user need is real

**What's missing**:
1. **Clarify what's actually allowed**: LinkedIn's ToS prohibits scraping and automated access. But what about using LinkedIn's official "Apply with LinkedIn" OAuth flow? (This is a legitimate, ToS-compliant integration)
2. **"Apply with LinkedIn" OAuth**: Users could import their LinkedIn profile + connections via this official flow
3. **Official LinkedIn Login**: OAuth for signing into LazyJob with LinkedIn credentials
4. **What automation is safe**: Manual copy-paste from LazyJob draft to LinkedIn is the current path. Is there a safer middle ground?
5. **Risk disclosure**: What are the actual risks of LinkedIn integration? (Account ban? IP ban? Legal action?)
6. **Alternative channels**: If LinkedIn is too risky, should LazyJob prioritize email outreach as the primary channel?

**Why important**: The current "no automation" stance is safe but leaves a significant UX gap. Users want more than copy-paste.

**What could go wrong**:
- User risks their LinkedIn account for a small productivity gain
- LazyJob provides no guidance on what's safe vs. risky on LinkedIn
- Email becomes the fallback but users prefer LinkedIn for professional networking

---

### GAP-72: Warm Path Expansion Suggestions (IMPORTANT)

**Location**: `networking-connection-mapping.md` - maps existing contacts to companies; `networking-referral-management.md` - referral readiness for existing contacts

**What's missing**:
1. **How to build new warm paths**: User has no connections at target company. What do they do?
2. **Event/conference discovery**: Find industry events where they could meet people at target companies
3. **Shared community identification**: Find Slack groups, Discord servers, Twitter communities where target company employees are active
4. **Alumni network tools**: Find people from same school who work at target company
5. **Mutual connection suggestion**: "You know Sarah who works at X - ask her about her colleagues"
6. **Content-based warming**: "This person posted about topic X. Engaging with their content is a low-stakes way to start a relationship"
7. **Second-degree path creation**: Suggest who to reach out to to create a new warm path

**Why important**: Users with weak networks at target companies need a strategy to build new connections, not just map existing ones.

**What could go wrong**- User has zero warm paths at target company, gives up on networking
- User spends time on low-value networking activities that don't produce warm paths
- "Build your network" advice is too abstract, users need concrete next steps

---

### GAP-73: Networking Activity Analytics and Attribution (MODERATE)

**Location**: `networking-referral-management.md` - tracks referral outcomes but not outreach activity correlation

**What's missing**:
1. **Outreach funnel metrics**: How many outreach messages → how many responses → how many warm relationships → how many referrals asked → how many succeeded?
2. **Channel effectiveness**: Which medium (LinkedIn, email, InMail) produces best response rates?
3. **Template effectiveness**: Do personalized messages outperform generic ones by how much?
4. **Time-to-response tracking**: What's the typical response time by channel?
5. **Referral attribution**: When user gets a job, can they attribute it to a specific referral? Can LazyJob learn from this?
6. **Networking ROI**: Did the networking effort actually help? Correlation between networking intensity and outcomes?

**Why important**: Without tracking, users can't improve their networking strategy over time.

**What could go wrong**:
- User sends 100 messages, gets 3 responses, no idea what went wrong
- All channels treated equally, user invests in low-return channels
- Can't learn from successful networking stories

---

### GAP-74: Contact Deduplication and Identity Resolution (MODERATE)

**Location**: `networking-referral-management.md` - Open Question #4 mentions duplicate contact across import sources

**What's missing**:
1. **Fuzzy matching algorithm**: How to detect same person imported from LinkedIn CSV and email? (Name + email? Name + company?)
2. **Merge UI**: When duplicate detected, how does user confirm and merge?
3. **Conflict resolution**: If same field has different values from different sources (e.g., different phone numbers), which wins?
4. **Source priority**: If LinkedIn says "Engineer" and email contacts says "Manager", which is more recent/trusted?
5. **Identity graph**: Build a graph of "these are all the same person" across multiple import sources
6. **Deduplication on import**: Should duplicates be detected and merged automatically, or shown to user for confirmation?

**Why important**: Without deduplication, users see the same person multiple times with different data.

**What could go wrong**:
- User sees "John Smith (LinkedIn)" and "John Smith (email)" as separate contacts
- Contact data becomes inconsistent after merge if not done carefully
- Important contact information lost during merge

---

### GAP-75: Non-Outreach Interaction Logging (MODERATE)

**Location**: `networking-outreach-drafting.md` - outreach_status tracks messages; `networking-referral-management.md` - interaction_count incremented when?

**What's missing**:
1. **What counts as an interaction**: Phone call, video chat, conference meeting, coffee, DM reply, email reply
2. **Interaction logging UX**: How to log an interaction quickly? ("I just had a great call with Sarah" - log with one tap?)
3. **Interaction quality scoring**: Was the interaction substantive? (ReferralReadinessChecker only cares about existence, not quality)
4. **Interaction notes**: Should every logged interaction have a note field?
5. **Conference/event tracking**: Attended same conference as a contact. How to log this and credit it as interaction?
6. **Mutual content engagement**: Commented on each other's LinkedIn posts - does this count as interaction?

**Why important**: The system only tracks outreach messages, not the full relationship-building process.

**What could go wrong**:
- User had a great coffee chat with a contact but doesn't log it, system doesn't know relationship is warm
- All interactions treated equally regardless of quality
- Conference connections don't count as "interaction" unless follow-up email happens

---

### GAP-76: Networking Touchpoint Cadence Recommendations (MODERATE)

**Location**: `networking-referral-management.md` - NetworkingReminderPoller uses fixed thresholds (7, 14, 21 days)

**What's missing**:
1. **Cadence recommendations by tier**: FirstDegreeCurrentEmployee might need quarterly check-ins, alumni might need monthly
2. **Seasonal awareness**: Don't reach out during holidays, company's busy season (Q4 for retail)
3. **Event-triggered outreach**: After company raises funding, is that a good or bad time to reach out? (Good: hiring surge; Bad: too busy)
4. **Personalized cadence**: Some contacts need more frequent touchpoints than others
5. **Cadence learning**: Can the system learn from which cadences produced responses?
6. **Quiet periods**: Respect when user is in "active job search" mode vs. "passive networking" mode

**Why important**: Fixed thresholds are a blunt instrument. Personalized cadences are more effective.

**What could go wrong**:
- User pings contact every 7 days, gets annoyed, relationship damaged
- Seasonal outreach at wrong time (holiday email = ignored)
- All contacts treated the same regardless of relationship strength

---

### GAP-77: Outreach Quality Scoring (MODERATE)

**Location**: `networking-outreach-drafting.md` - fabrication_warnings but no quality/personalization score

**What's missing**:
1. **Personalization depth scoring**: Rate how personalized the message is (mentions shared context? References specific post? Uses contact's name appropriately?)
2. **Message quality heuristics**: Length appropriate? Tone matches approach? No red-flag phrases?
3. **AI-quality detection**: Can we detect if a message sounds too "AI-generated"? (Overused phrases, perfect grammar, etc.)
4. **A/B testing support**: Can user generate two versions and compare?
5. **Response rate prediction**: Based on message characteristics, predict likely response rate
6. **Improvement suggestions**: What would make this message more personalized?

**Why important**: Users have no feedback on whether their outreach is good or generic.

**What could go wrong**:
- Generated message sounds great to user but is obviously AI-generated to recipients
- No way to improve over time without feedback
- "Personalized" messages all score the same, no differentiation

---

## Cross-Spec Gaps

### Cross-Spec P: Contact Data ↔ LifeSheet Overlap

`networking-connection-mapping.md` mentions `profile_contacts` is distinct from `application_contacts`, but there's potential overlap with `LifeSheet`:
- Should the user's own work history in LifeSheet be used to find shared employers with contacts?
- Should contacts from the user's past (who are in LifeSheet) be automatically promoted to profile_contacts?
- There's no spec for how these data structures should reference shared entities

**Affected specs**: `profile-life-sheet-data-model.md`, `networking-connection-mapping.md`

### Cross-Spec Q: Outreach ↔ Application State

When a contact at a company refers the user and an application is created, how should this be tracked?
- Application knows it came from referral (via application_contacts?)
- Referral outcome (ReferralSucceeded) linked to application
- But there's no explicit spec linking networking outreach activity to application outcomes

**Affected specs**: `networking-referral-management.md`, `application-workflow-actions.md`

---

## Specs to Create

### Critical Priority

1. **XX-contact-multi-source-import.md** - Email/phone/vCard/business card import, incremental import, contact merge on import

### Important Priority

2. **XX-relationship-decay-modeling.md** - Decay math, visualization, staleness detection, dormant contact handling
3. **XX-linkedin-automation-policy.md** - What automation is ToS-compliant, Apply with LinkedIn OAuth, risk disclosure
4. **XX-warm-path-expansion.md** - Event discovery, community identification, content-based warming, second-degree path creation

### Moderate Priority

5. **XX-networking-activity-analytics.md** - Outreach funnel metrics, channel effectiveness, attribution, networking ROI
6. **XX-contact-identity-resolution.md** - Fuzzy matching, merge UI, conflict resolution, identity graph
7. **XX-non-outreach-interaction-logging.md** - Interaction types, quick logging UX, quality scoring, conference tracking
8. **XX-networking-touchpoint-cadence.md** - Tier-based cadences, seasonal awareness, event-triggered outreach
9. **XX-outreach-quality-scoring.md** - Personalization depth scoring, AI-quality detection, A/B testing

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-69: Multi-Source Contact Import | Critical | High | Network completeness |
| GAP-70: Relationship Decay Modeling | Important | Medium | Referral accuracy |
| GAP-71: LinkedIn Automation Policy | Important | Medium | UX expectations |
| GAP-72: Warm Path Expansion | Important | High | Network-poor users |
| GAP-73: Networking Activity Analytics | Moderate | Medium | Strategy improvement |
| GAP-74: Contact Identity Resolution | Moderate | Medium | Data quality |
| GAP-75: Non-Outreach Interaction Logging | Moderate | Low | Relationship tracking |
| GAP-76: Touchpoint Cadence Recommendations | Moderate | Low | Relationship maintenance |
| GAP-77: Outreach Quality Scoring | Moderate | Low | Message effectiveness |
