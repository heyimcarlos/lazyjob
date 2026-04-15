# Progress Log

Started: 2026-04-14
Objective: Research the job-seeking workflow and agentic automation opportunities

Note: A parallel ralph loop is running at ../reverse-engineer-linkedin/ producing platform-level specs. Task #10 in this loop will critique those specs from a job-seeker perspective.

---
## Task 1: job-search-workflow-today (COMPLETE)

**Spec written to:** `../../specs/job-search-workflow-today.md`

### Key findings:

**The workflow has 6 phases:** Preparation → Search & Discovery → Application → Waiting/Follow-up → Interview → Offer/Negotiation

**Critical data points (from Huntr Q2 2025 — 461K tracked applications):**
- LinkedIn dominates with 77-80% of job saves, but has only 3.3% response rate
- Indeed growing fast with 20-25% response rate; Google Jobs highest at 9.3%
- Tailored resumes convert at 5.75% vs 2.68% for generic (115% improvement)
- Median time to offer: 68.5 days and growing (+22%)
- Median weekly applications: 4 or fewer; most common path to offer: 10-20 apps
- 75% of applications receive zero response (37.5M/month ghosted)
- Ghosting rate hit 3-year peak in 2025 (48% of applicants ignored)

**The referral paradox:**
- Referrals = 7% of applicants but 72% of interviews. 7-18x more likely to be hired
- 30-50% of all hires come through referrals
- The most effective channel is the hardest to scale/automate

**The spray-and-pray feedback loop:**
- 48% apply broadly; 45% say ATS encourages this
- 76% would apply more selectively if employers gave feedback
- Mass application is a rational response to employer silence — NOT carelessness

**AI tool landscape:**
- 93% of seekers use AI for resume/cover letter help
- Auto-apply tools (LazyApply, Sonara, Jobright) are growing but risk accelerating the spam cycle
- Tracking tools (Huntr, Teal, Careerflow) are more clearly valuable

**Biggest agentic opportunities:**
1. Intelligent job discovery & ghost job filtering
2. Automated resume tailoring (the 115% conversion lift makes this a no-brainer)
3. Application tracking & follow-up automation
4. Referral network mining (high value, high risk of inauthenticity)

**Key open question:** The volume paradox — AI auto-apply increases volume, which increases ghosting, which drives more auto-apply. How do we break the cycle instead of accelerating it?

---

## Task 2: resume-optimization (COMPLETE)

**Spec written to:** `../../specs/resume-optimization.md`

### Key findings:

**The "75% ATS rejection" myth is BUSTED:**
- A 2025 Enhancv study (25 U.S. recruiters) found 92% confirm ATS does NOT auto-reject resumes
- The 75% stat came from a 2012 sales pitch by Preptel (went bankrupt 2013, no methodology published)
- Real filtering is human + volume: when 400-2000 people apply, recruiters look at top 20 ranked candidates

**ATS market landscape:**
- Workday dominates Fortune 500 (39%); iCIMS leads overall market (10.7%); Greenhouse gaining fast in tech
- $2.65B market (2026), growing 7.36% CAGR
- Most ATS use keyword matching + ranking, NOT auto-rejection

**Resume tailoring is the #1 automation target:**
- Tailored resumes: 5.75% conversion vs 2.68% generic (115% improvement) — Huntr data, 1.7M applications
- Aligning resume title with job title alone = 3.5x interview rate increase
- 54% of candidates DON'T tailor because it's tedious — textbook automation opportunity

**The "6-second scan" is also a myth:**
- Actual initial scan: ~11.2 seconds average
- Median total review if interested: 1 minute 34 seconds
- F-pattern eye tracking: name, current title/company, previous title/company, dates, education get 80% of attention

**Resume format realities:**
- DOCX generally safer than PDF for ATS parsing (XML-based structure)
- Single-column safest; two-column works IF built with native columns (no tables/text boxes)
- Leading parsers reach ~99% accuracy on well-formatted resumes in 2025
- White-text keyword stuffing is dead — modern ATS detect it, recruiters catch it, companies blacklist

**AI detection of resumes:**
- ATS don't distinguish AI-written from human-written
- Recruiters detect AI by uniformity and vagueness, not detection software
- The real risk: all AI resumes sounding the same, not "being caught"

**Tool landscape:**
- Scanning/matching: Jobscan ($50/mo), Teal (free tier), Resume Worded, SkillSyncer
- AI builders: Rezi ($29/mo), Enhancv ($25/mo), Kickresume ($9/week)
- Tailoring-specific: Huntr, TailoredCV, Reztune (60+ LLM prompts pipeline)
- Open source: Resume Matcher (GitHub, works with Ollama)
- APIs: UseResume AI (REST API for programmatic generation)

**Agentic opportunity — three levels:**
1. Smart Resume Audit (table stakes, already commoditized)
2. Automated Resume Tailoring (the real opportunity — master resume → tailored per JD)
3. Continuous Resume Intelligence (differentiated — living skill graph, market monitoring, feedback loops)

**Critical design decisions for our product:**
- Maintain master resume as ground truth; generate targeted versions
- Truthfulness guardrails: NEVER fabricate skills/experience
- Voice preservation: learn and maintain candidate's writing style
- Feedback loop: track which versions lead to interviews
- Integration: tailoring is commodity alone; value is in connecting to job discovery + tracking + interview prep

---

## Task 3: cover-letters-applications (COMPLETE)

**Spec written to:** `../../specs/cover-letters-applications.md`

### Key findings:

**Cover letters still matter — the data is overwhelming:**
- 94% of hiring managers say cover letters influence interview decisions
- 83% read them even when not required; 45% read them BEFORE the resume
- 72% expect them even when listed as "optional"
- 49% say a strong cover letter can convince them to interview a weak candidate
- The Problem-Solution format outperforms all other formats across studies

**Industry segmentation is critical:**
- Tech giants (FAANG) rarely even have cover letter fields — they're irrelevant there
- Startups care more — smaller applicant pools, culture fit signals
- Finance, legal, consulting — table stakes, communication quality is core
- The agent needs to know WHEN to generate a cover letter, not just HOW

**AI detection reality (2026):**
- 67% of hiring managers claim they can spot AI-generated letters (TopResume, 800+ managers)
- 54% view AI content negatively, 19.6% would reject
- BUT: they can't detect AI that's been humanized with personal details and authentic voice
- The real risk isn't "being caught" — it's the AI monoculture where all letters converge on the same patterns
- 29.3% of job seekers now use AI for applications (up from 17.3% in 2024)

**Auto-apply tools are a trap:**
- LazyApply, Sonara, etc. optimize volume (up to 1,500/day)
- But: 70-85% LinkedIn ban rate within 30 days for automation tool users
- Mass applications yield 2-4% conversion vs 20-30% for targeted
- Applying to 81+ positions DECREASES offer rate (20.36%) vs 21-80 applications (30.89%)
- Auto-apply optimizes the wrong metric entirely

**The real agentic opportunity is three levels:**
1. **Smart cover letter generation** — Problem-Solution format, candidate voice, company-specific research, 30-second review vs 30-minute writing (near-term, table stakes)
2. **Full application package management** — resume + cover letter + screening questions + submission across ATS platforms, with human approval gate (medium-term, high value)
3. **Adaptive application strategy** — feedback loops on what's working, channel allocation, market intelligence (longer-term, differentiated)

**Technical integration landscape:**
- Greenhouse and Lever have public APIs that support application submission — clean path
- Workday, LinkedIn, Indeed have NO application APIs — require browser automation (risky)
- Unified ATS APIs (unified.to, Merge.dev) normalize across 60+ platforms
- Browser automation faces increasing detection: fingerprinting, behavioral biometrics, CAPTCHAs

**Key insight for our product:**
Cover letter generation alone is commoditized (free tiers everywhere). The value is in the WORKFLOW: intelligent job assessment → tailored application package → smart submission → tracking → feedback loop. The cover letter is one artifact in a larger system.

---

## Task 4: agentic-job-matching (COMPLETE)

**Spec written to:** `../../specs/agentic-job-matching.md`

### Key findings:

**Semantic matching is real but employer-side only:**
- Semantic search finds 60% more relevant profiles than Boolean, reduces false positives by 62%
- AI skill matching predicts job performance with 78% accuracy
- CareerBERT (2025) achieved MAP@20 of 0.711 matching resumes to ESCO jobs — state of the art
- Resume2Vec outperformed conventional ATS by 15.85%
- BUT: this technology lives in employer ATS systems. Job seekers still get dumb keyword alerts

**Ghost jobs destroy matching value:**
- 27-30% of online listings are ghost jobs (multiple 2025-2026 studies)
- 48% of tech listings never result in a hire (BLS JOLTS)
- 93% of HR professionals admit to posting ghost jobs (LiveCareer, 918 respondents)
- Ghost detection is a massive untapped opportunity — no product markets this

**Job listing data access landscape:**
- Clean APIs: Adzuna (free tier, 12 countries), Greenhouse Job Board API (public), Lever Postings API, unified ATS APIs (Merge.dev, unified.to covering 60+ ATS platforms)
- Scraping: JobSpy (open source, covers LinkedIn/Indeed/Glassdoor/Google/ZipRecruiter), Apify actors ($0.005/result)
- Restricted: LinkedIn Talent API (partners only), Indeed Publisher API (deprecated for search), Workday/iCIMS (no public APIs)
- Legal: hiQ v. LinkedIn protects public scraping somewhat, but LinkedIn actively sues (Proxycurl shut down July 2025)

**Skill ontologies are mature and free:**
- ESCO: 3,000+ occupations, 13,890 skills, 27 languages, free API
- O*NET: 1,000+ occupations, detailed taxonomies, free/downloadable
- ESCO-O*NET crosswalk published 2024 (AI + human validated)
- JobBERT v2 (TechWolf, Hugging Face) for domain-specific job title embeddings

**Existing tools have a critical gap:**
- Sorce (swipe UI, 1.5M jobs, $15/week), JobCopilot (end-to-end), Jobright (filtered matches) — none provide transparent, explainable match scores
- No tool tells users WHY a job was recommended
- No tool actively filters ghost jobs
- No tool does genuine skill inference (understanding what you CAN do vs what you LISTED)

**Three-level agentic opportunity:**
1. **Intelligent job discovery** (near-term): Multi-dimensional candidate model + continuous scraping + ghost filtering + explainable recommendations. The 3-7 daily curated matches hypothesis
2. **Semantic skill inference engine** (medium-term): Parse experience into claims, map to skill ontology, infer adjacent skills, compare skill graphs. Career transitioner support
3. **Proactive career intelligence** (longer-term): Skill demand trends, emerging role detection, network monitoring, compensation tracking, upskilling suggestions. Creates the personalization flywheel

**Key technical architecture:**
- LLM for JD parsing → ESCO/O*NET skill mapping → CareerBERT/SBERT for embeddings → cosine similarity for ranking → LLM reasoning for top-N explanations → feedback loop for personalization
- Cost: ~$0.01-0.10 per 1K jobs for embeddings, ~$0.01-0.05 per explanation
- Scale: 500K-1M new listings/day, pre-computed embeddings in vector DB, near-real-time matching

**Critical open question:** Two-sided AI dynamics — if both candidates and employers use AI agents, do they converge on the same matches or create new failure modes?

---

## Task 5: agent-interfaces-job-platforms (COMPLETE)

**Spec written to:** `../../specs/agent-interfaces-job-platforms.md`

### Key findings:

**The platform API landscape breaks into 4 tiers:**

1. **Open APIs (can submit applications):** Greenhouse Job Board API (cleanest path — public REST API, multipart form POST, no server-side validation), Lever Postings API (rate-limited, requires retry logic), iCIMS REST API (gated, enterprise-oriented)
2. **Job listing APIs only (read, no submit):** Indeed ($3/call, advertisers only), LinkedIn (employer-side only, no consumer API, all automation banned), Workday (no public API, 39% of Fortune 500)
3. **Unified/aggregation APIs:** Merge.dev and Unified.to normalize 60-73+ ATS platforms but require employer authorization. Jobo/Fantastic Jobs is the most interesting — 2M+ jobs from 80K+ career sites, claims programmatic application submission across 25+ ATS platforms
4. **Scraping only:** JobSpy (open source, scrapes Indeed/LinkedIn/Glassdoor/ZipRecruiter concurrently), Apify actors (~$0.005/result)

**Auto-apply tools: volume vs quality is settled:**
- Pure AI auto-apply (LazyApply, Sonara): 1-3% callback rate, 70-85% LinkedIn ban rate within 30 days
- Human+AI hybrid (Scale.jobs): 25-47% callback rate at ~30 targeted apps/day for $199/250 apps
- The data overwhelmingly favors quality over quantity — 8-47x better callback rates

**Major platform enforcement actions (2025):**
- Proxycurl: Shut down July 2025 after LinkedIn lawsuit (fake accounts, billions of bot requests)
- Apollo.io + Seamless.ai: LinkedIn pages banned March 6, 2025 for browser extension data extraction
- LinkedIn detection rates increased 340% from 2023-2025; 23% account restriction rate within 90 days of automation use

**Browser automation is losing the arms race:**
- Modern anti-bot (Cloudflare, DataDome, Akamai) detect Chrome DevTools Protocol usage itself
- Stealth plugins only solve fingerprint-level detection — TLS fingerprinting, behavioral biometrics, IP reputation remain unsolved
- `rebrowser` project (rebuilds outside CDP) is the cutting edge but fragile
- Python playwright-stealth actively maintained; Node.js stealth stack stagnating

**Legal landscape:**
- hiQ v. LinkedIn (2022): scraping public data likely NOT a CFAA violation
- BUT Proxycurl (2025): LinkedIn won on contract/ToS claims despite hiQ precedent
- Safe harbor: ATS APIs (Greenhouse, Lever) are explicitly sanctioned; aggregator APIs (Adzuna, Jobo) clean; LinkedIn anything is high legal risk

**The recommended architecture:**
- Discovery layer: multi-source (APIs + scraping + aggregators) → deduplicate → ghost filter
- Matching layer: semantic skill matching → explainable scores → human approval gate
- Application layer: route by platform — API for Greenhouse/Lever, aggregator for others, browser automation as last resort, NEVER automate LinkedIn
- Tracking layer: status monitoring → feedback loop → strategy adaptation

**Critical insight:** The right model may be "agent does everything except final submission" — Scale.jobs' 25-47% callback rate with human VAs doing the clicking suggests that human submission is worth the cost. The agent's value is in discovery, matching, material preparation, and tracking — not in clicking "Submit."

**Key open questions:**
- Jobo/Fantastic Jobs claims programmatic submission across 25+ ATS platforms — what are the actual mechanics and reliability?
- Is there a LinkedIn partnership path for read-only job data access?
- Will Greenhouse/Lever restrict their APIs if agent-submitted applications flood them?
- Is MCP (Model Context Protocol) the right abstraction layer for agent-platform interfaces?

---

## Task 6: recruiter-workflow (COMPLETE)

**Spec written to:** `../../specs/recruiter-workflow.md`

### Key findings:

**The three recruiter types have different incentives:**
- Internal/corporate recruiters: manage 20-50+ open reqs, measured on time-to-hire and fill rate
- Agency recruiters: commission-based (20-25% of first-year salary), speed vs. quality tension
- Retained/executive search: paid retainer upfront, 90-120 day cycles, C-suite focus

**Time allocation reveals the automation opportunity:**
- Sourcing/discovery: 23-35% of time
- Screening/assessment: 15-25%
- Administrative/coordinating: 25-35% (THE BIG TIME SINK)
- Actual judgment work: <20%

**The tool landscape breaks into 5 tiers:**
1. ATS as system of record (Greenhouse, Lever, Workday, iCIMS, Ashby)
2. Sourcing/CRM tools (Gem, HireEZ, Seekout, Paradox)
3. LinkedIn Recruiter (dominant but $12-16K/year/seat, all automation banned)
4. Assessment/scheduling platforms (HireVue, Calendly)
5. The fragmentation problem: most orgs run 3-10 tools simultaneously

**AI adoption is accelerating but early:**
- Gartner: 80% of recruiting vendors will embed AI by 2027
- But: 38% of candidates may decline offers if process too automated
- Paradox, HireEZ, Seekout, Lever all have meaningful AI features
- Greenhouse, Ashby embedding AI throughout workflows

**7 concrete agentic opportunities identified:**
1. **Candidate discovery** — parallel multi-source querying, biggest time sink
2. **Personalized outreach at scale** — generate first drafts, human approves
3. **Resume screening & ranking** — semantic vs. keyword matching
4. **Interview scheduling automation** — logistics, already commoditized
5. **Interview prep briefs** — synthesize candidate info for interviewers
6. **Market intelligence** — compensation data, competitor activity, talent pool visibility
7. **Pipeline maintenance & follow-up** — CRM hygiene at scale

**Key insight for our product:**
The agent should serve as "recruiter co-pilot" — handling the 80% of time spent on administrative/sourcing work while the recruiter focuses on relationship management, judgment calls, and closing. The value is NOT replacing the recruiter; it's making the recruiter 5-10x more effective by handling the logistics and research that currently consume most of their time.

**Critical legal/ethical constraints:**
- EEO compliance requires audit trails
- Ban-the-box, salary history bans restrict early-stage questions
- LinkedIn ToS bans all automation (hiQ v. LinkedIn provides some public data scraping cover)
- Resume fraud growing (51% of resumes contain inaccuracies) — agents can help detect

**Key open questions:**
- Who controls the agent (recruiter vs. company vs. ATS vendor) creates different trust models
- Does AI screening help or hurt diversity? Evidence is mixed
- What is the recruiter's new role when agents handle sourcing/screening/scheduling?
- Platform risk if ATS/sourcing tools change pricing or restrict API access

---

## Task 7: networking-referrals-agentic (COMPLETE)

**Spec written to:** `../../specs/networking-referrals-agentic.md`

### Key findings:

**The referral paradox (biggest opportunity + hardest to automate):**
- 30-50% of hires come from referrals, but only 7% of applicants are referrals
- Referral candidates = 7-18x more likely to get hired, 72% of interviews
- The most effective channel is the LEAST accessible to those who need it most (career changers, new grads, laid off workers)
- The referral effectiveness comes from trust multiplication: referrer stakes social capital, company trusts referrer's judgment

**Cold outreach reality:**
- LinkedIn InMail response rate: 3-5%
- Email cold outreach: 1-5%
- Most cold outreach fails because: generic templates, no mutual connection, wrong timing, no compelling reason to respond

**Networking tools landscape:**
- Clay (GTM data enrichment + AI outreach): $200+/month, sales-focused
- Dex (relationship management): limited enrichment
- Apollo.io (B2B email/phone outreach): detected as sales tool
- HubSpot Free CRM: too generic for professional networking
- Professional networking CRM tools are almost entirely sales-focused — no real "networking for job seeking" tools exist

**6 concrete agentic opportunities:**
1. **Network mapping & warm path finding** — map 1st/2nd/3rd degree connections to target companies, identify introduction paths humans wouldn't think to look for
2. **Personalized outreach drafting at scale** — generate personalized messages referencing specific shared context; 10 carefully personalized > 100 generic
3. **Relationship maintenance & follow-up** — track relationship strength, remind to follow up, suggest relevant content to share
4. **Referral request identification** — identify which contacts at target company might refer, assess relationship strength, suggest timing
5. **Informational interview prep** — generate conversation topics, questions, company background for 30-min info interviews
6. **Company research for networking** — deep research on people you're reaching out to, recent posts, company news

**Critical design principle: agent suggests, human approves, human sends**
- Agent drafts, human edits, human sends → networking
- Agent sends automatically without human review → spam
- The product must enforce human-in-the-loop for all outreach

**Key technical constraints:**
- LinkedIn: No public API for messaging/connections, all automation banned by ToS
- Browser automation: increasingly detected by Cloudflare/DataDome
- Clean paths: Email outreach via Apollo/Hunter.io, data enrichment via Clearbit/RocketReach

**Biggest open questions:**
- Can agents authentically represent humans in professional networking without crossing into spam?
- What about network-poor job seekers (career changers, new grads) who need networking most but have least to work with?
- If everyone uses agents for personalized outreach, does it just become higher-quality spam arms race?
- Should agent help candidate get referred INTO company, or help companies refer employees OUT? (LinkedIn Recruiter model is outbound, not inbound)

---

## Task 8: interview-prep-agentic (COMPLETE)

**Spec written to:** `../../specs/interview-prep-agentic.md`

### Key findings:

**The fragmentation problem is the core pain:**
- Job seekers juggle 6-8 separate tools: LeetCode, Exponent, Glassdoor, Blind, Reddit, Pramp, YouTube
- No unified place to prep — most serious candidates target FAANG with 4-12 weeks of preparation
- Typical time split: 60-70% technical (algorithms/system design), 20-30% behavioral
- Behavioral prep consistently underprepared — often compressed to last 1-2 weeks

**Mock interview landscape:**
- Pramp: peer-to-peer, free, availability-constrained, peer quality varies
- Interviewing.io: real FAANG engineers, anonymous, free tier, limited availability
- LeetCode mock: company-tagged problems, AI feedback on code but not communication
- Refactored AI: video + emotion/facial analysis, unproven at scale
- Gap: No tool provides behavioral + technical + system design in one session with STAR evaluation

**The behavioral prep gap:**
- STAR method well-known but practiced haphazardly
- Candidates report: "hard to evaluate own responses", "stories don't fit different companies"
- No AI tool effectively evaluates STAR responses with nuanced feedback
- Most behavioral prep is YouTube/blog self-directed

**Company research is manual and takes 2-4 hours per company:**
- Sources: Glassdoor (outdated), Blind (anonymous), Reddit (fragmented), LeetCode Discuss (scattered)
- Aggregating this manually is the biggest time sink
- Data freshness is a real problem — processes change

**5 concrete agentic opportunities identified:**
1. **Interview Prep Plan Generation** — parse job posting + candidate background → week-by-week study plan. Most differentiated (no tool does this).
2. **Company Research Agent** — auto-aggregate Glassdoor/Blind/Reddit/LeetCode Discuss → company-specific cheat sheet. High value.
3. **AI Mock Interview Simulation** — conduct full mock with real-time feedback on communication, STAR adherence, code quality.
4. **STAR Method Coach** — evaluate stories on structure/depth/results/relevance, suggest improvements, match to likely questions.
5. **Progress Tracking Dashboard** — track prep by topic, show trends, identify gaps.

**Key design principle:** Agent as "prep co-pilot" — plans, researches, and tracks. Human does the actual practice and makes strategic decisions.

**Technical challenges:**
- Real-time speech/text feedback for mock interviews (voice vs. text interface tradeoffs)
- Company data freshness from scraping Glassdoor/Blind (ToS risk)
- System design evaluation is subjective — hard to build an objective rubric
- Integration strategy: build own question DB (hard) vs. wrap LeetCode (easier, less differentiated)

**Biggest open questions:**
- Voice vs. text interface for mock interviews?
- B2C (individual job seeker) vs. B2B (company/bootcamp) — which drives the model?
- How to handle system design evaluation rubric?
- ROI measurement — how to credit the agent when candidate gets an offer?

---

## Task 9: salary-negotiation-offers (COMPLETE)

**Spec written to:** `../../specs/salary-negotiation-offers.md`

### Key findings:

**The Negotiation Gap:**
- 40-50% of candidates who negotiate receive a better offer
- Negotiation typically yields 5-15% base salary increase
- Not negotiating can cost hundreds of thousands over a career
- Top performers who negotiate strategically see 10-30% total comp increases
- Women are less likely to initiate negotiations (Babcock et al. research shows 18-20% increases from negotiation training)

**The Total Comp Blind Spot:**
- Most candidates focus on BASE SALONE — ignoring 20-40% of actual compensation
- Proper negotiating unit is total comp (base + equity + bonus + benefits)
- Equity often 10-20% of total comp at senior tech levels
- Most candidates cannot accurately evaluate multi-year offers with vesting schedules

**Compensation Data Landscape:**
- Levels.fyi: Dominant in tech (~3M monthly users), crowdsourced verified data, no rigorous academic validation
- Glassdoor: Self-reported, smaller samples for tech roles
- Blind: Anonymous, smaller dataset but more detailed
- Payscale: Survey-based millions of respondents, better for non-tech
- CRITICAL GAP: No tool handles multi-year, multi-component offer comparison with risk adjustment

**AI Tool Gap — No True Negotiation Coaching Exists:**
- All "negotiation tools" are data lookup (levels.fyi, Glassdoor, Salary.com)
- NO AI chatbot drafts counter-offer letters
- NO real-time negotiation coaching during actual negotiations
- NO tool evaluates private company equity accurately
- Human coaches exist but are expensive and not AI

**Four-Level Agentic Opportunity:**
1. **Market Intelligence Agent** (near-term): Monitors comp data, alerts on band changes, personalized market rate. Table stakes.
2. **Offer Evaluation Engine** (near-term, high value): Calculates annualized + risk-adjusted value of full offers. Handles the math humans can't do.
3. **Negotiation Strategy + Counter-Offer Drafting** (medium-term): Generates counter-offer letter + phone script. Human reviews, edits, sends. Critical: agent drafts, human owns.
4. **End-to-End Negotiation Coach** (longer-term): Real-time coaching during calls, monitors email/calendar, coordinates multi-offer timelines.

**Pay Transparency Laws (2025-2026):**
- CA, NY, CO, WA, CT, HI, IL, MD, MA, NV, RI require salary ranges on job postings
- Reduces information asymmetry, strengthens candidate leverage
- But ranges often anchor low — candidates need to research above-range expectations

---

## Task 10: gap-analysis-and-critique (COMPLETE)

**Spec written to:** `../../specs/gap-analysis-and-critique.md`

### Key findings:

**Critical Finding: LinkedIn Specs Missing**
The LinkedIn specs described in the objective (from ../reverse-engineer-linkedin/) do not exist in the filesystem. Only x-professional-features.md (about X.com) exists from parallel research.

**What's Missing Altogether:**
1. **Ghost Job Detection** — 27-30% of listings are fake. No spec addresses this. Highest-impact missing topic.
2. **Offer Rejection and Decline Etiquette** — the post-interview, pre-offer phase is underexplored
3. **Career Transition Guidance** — hardest case (career changers), least covered
4. **The "Just Need a Job" Baseline User** — entry-level, laid-off workers, gig economy
5. **Geographic/Remote Considerations** — visa, tax, cost-of-living
6. **Long-term Career Planning** — beyond single job

**What's Over-Researched vs. Under-Researched:**
- Over-researched: Agent-platform technical interfaces, basic cover letter data
- Under-researched: Ghost job detection, offer evaluation tools, career transition, referral network mapping

**Assumptions in x-professional-features.md That Need Challenging:**
- "X is a viable hiring platform for most tech roles" — false; X is niche (~5% of roles)
- "Real-time information advantage is worth the noise" — signal-to-noise ratio is low
- "Build in public" advice is career-risky for many industries

**Recommendations:**
1. Add ghost job detection as a research priority
2. Interview job seekers who recently negotiated — what tools did they use?
3. Research career changer workflows specifically
4. Primary sources still needed: r/cscareerquestions, Hacker News, Blind app discussions

---

## ALL TASKS COMPLETE

All 10 research tasks are now complete. The specs are written to:
- `/home/ren/repos/agentin/specs/job-search-workflow-today.md`
- `/home/ren/repos/agentin/specs/resume-optimization.md`
- `/home/ren/repos/agentin/specs/cover-letters-applications.md`
- `/home/ren/repos/agentin/specs/agentic-job-matching.md`
- `/home/ren/repos/agentin/specs/agent-interfaces-job-platforms.md`
- `/home/ren/repos/agentin/specs/recruiter-workflow.md`
- `/home/ren/repos/agentin/specs/networking-referrals-agentic.md`
- `/home/ren/repos/agentin/specs/interview-prep-agentic.md`
- `/home/ren/repos/agentin/specs/salary-negotiation-offers.md`
- `/home/ren/repos/agentin/specs/gap-analysis-and-critique.md`

The most important gaps identified for future work:
1. Ghost job detection (27-30% of listings are fake)
2. Career transition support (hardest case, most underserved)
3. Entry-level and laid-off worker workflows
4. The LinkedIn specs from the parallel research loop (if they exist elsewhere)
