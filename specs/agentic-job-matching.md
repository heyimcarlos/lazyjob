# Agentic Job Matching

## The reality today

### How job matching actually works in 2026

Job matching today is a broken two-sided search problem. Job seekers search by keywords and filters (title, location, salary range, remote). Employers post descriptions full of aspirational requirements. The match happens through keyword overlap — not understanding.

**The keyword matching trap:**
- A candidate who "built distributed systems at scale" won't match a JD requiring "microservices experience" — despite being the same skill
- "Full-stack developer" and "software engineer" are treated as different searches despite 80%+ role overlap
- Keywords reward resume-stuffers and punish people who describe their work differently than HR wrote the JD

**Current platform matching quality:**
- LinkedIn's job recommendations: algorithm factors in profile keywords, application history, and network signals. Quality is decent for obvious matches, poor for career transitions or non-obvious fits
- Indeed: primarily keyword + location matching with some "AI-enhanced" ranking. 20-25% response rate (vs LinkedIn's 3.3%) suggests better matching or less competition — probably both
- Google Jobs: aggregates from multiple sources, basic semantic matching via Knowledge Graph. 9.3% response rate
- Most ATS (Workday, iCIMS, Greenhouse): rank candidates by keyword overlap score. Despite marketing claims of "AI matching," 78% of initial screenings now use NLP but most still reduce to keyword frequency + location + title matching

**The ghost job epidemic destroys matching value:**
- 27-30% of online job listings are ghost jobs (ResumeUp.AI 2025 LinkedIn analysis; Fonzi AI 2026)
- 48% of tech listings never result in a hire (BLS JOLTS sector analysis)
- 93% of HR professionals admit to posting ghost jobs at least occasionally (LiveCareer, 918 HR professionals, March 2025)
- 36% of job seekers applied to at least one role that was never actually filled (Greenhouse 2025 Candidate Experience Survey)
- This means even "perfect" matching is wasted ~30% of the time on jobs that don't exist

**What job seekers actually do:**
- Set up alerts on 3-5 platforms → get flooded with irrelevant matches → develop alert fatigue → stop trusting recommendations → revert to manual search
- 32.4% feel exhausted, 26% feel stuck (Huntr Q1 2025)
- 55% of unemployed Americans report burnout from job searching
- Median 4 or fewer applications per week despite constant platform use — the bottleneck isn't finding jobs, it's evaluating fit and applying

### The semantic matching shift (real but early)

Enterprise ATS is genuinely moving beyond keywords:
- Semantic search finds 60% more relevant profiles than Boolean queries and reduces false positives by 62%
- AI-based skill matching now predicts job performance with 78% accuracy
- 78% of initial screenings use NLP for contextual skill evaluation (not just keyword frequency)
- Resume2Vec (2025 research) outperformed conventional ATS by up to 15.85% on match quality
- CareerBERT (March 2025, Expert Systems with Applications) achieved MAP@20 of 0.711 and MRR@20 of 0.861 matching resumes to ESCO job categories

**But this shift is employer-side, not job-seeker-side.** ATS gets smarter at ranking candidates. Job seekers still get dumb keyword alerts.

## What tools and products exist

### Job-seeker-side matching tools

| Tool | Approach | Pricing | Key Limitation |
|------|----------|---------|----------------|
| **Sorce** | Tinder-like swipe UI, AI auto-apply | Free (40 swipes/day), paid from $15/week | 1.5M jobs; matching quality unclear, submissions sometimes fail |
| **JobCopilot** | End-to-end automation, learns from interactions | Paid tiers | Black box matching, no transparency into fit scoring |
| **Jobright** | Filtered matches, company insights, real-time trends | Free + premium | Good UX but matching methodology not disclosed |
| **Teal** | Resume-to-JD matching + tracking | Free tier, paid for full features | More resume optimization than job discovery |
| **Jobscan** | ATS keyword matching score (target: 75%) | $50/month | Keyword-focused, not semantic. 6x interview rate claimed |
| **Flashfire Jobs** | Speed-focused matching | Various tiers | Newer entrant, limited track record |
| **SmartJobHunt** | AI matching guide approach | Various | 2026 launch, limited data |

**Critical gap:** None of these tools give users transparent, explainable match scores. They all say "AI matching" but none show WHY a job was recommended or let users calibrate the algorithm beyond basic filters.

### Employer-side matching (relevant for understanding the full picture)

| Platform | Approach | Notable |
|----------|----------|---------|
| **Gem** | AI sourcing + CRM, strong for startups/enterprise | G2 rating 4.8 |
| **Ashby** | Analytics-first ATS with AI matching | G2 rating 4.7 |
| **SeekOut** | Diversity-focused AI sourcing | Specialized matching |
| **HireEZ** | AI sourcing across 800M+ profiles | Enterprise |
| **Sense** | AI job matching for staffing agencies | Staffing-specific |

### Open-source and research tools

| Tool | What it does |
|------|-------------|
| **JobSpy** (Python) | Scrapes LinkedIn, Indeed, Glassdoor, Google, ZipRecruiter, Bayt, Naukri. Returns structured data (title, company, location, salary, description, job_type). Free, open source |
| **CareerBERT** | Research model. Fine-tuned SBERT mapping resumes to ESCO jobs in shared embedding space. Code on GitHub |
| **JobBERT v2** (TechWolf) | Hugging Face model for job title matching/similarity. Fine-tuned from all-mpnet-base-v2 |
| **Resume Matcher** | Open source, works with Ollama for local LLM matching |

### Skill ontologies and taxonomies

| Framework | Coverage | Access |
|-----------|----------|--------|
| **ESCO** (European Commission) | 3,000+ occupations, 13,890 skills, 27 languages | Free API, regularly updated |
| **O*NET** (US Dept of Labor) | 1,000+ occupations, detailed skill/ability/knowledge taxonomy | Free, downloadable |
| **ESCO-O*NET Crosswalk** | Maps between both standards using AI + human validation | Published 2024 |
| **LinkedIn Skills Graph** | 40,000+ skills with relationships | Not publicly accessible |

## The agentic opportunity

### What "agentic job matching" actually means

The core insight: **matching is not a search problem, it's an understanding problem.** The agent needs to deeply understand the candidate AND deeply understand the job — then reason about fit across multiple dimensions.

### Level 1: Intelligent job discovery (near-term, high-value)

**What the agent does:**
1. Ingests candidate's complete professional profile (resume, LinkedIn, portfolio, stated preferences, career goals)
2. Builds a multi-dimensional candidate model:
   - Explicit skills (listed on resume)
   - Inferred skills (e.g., "built a recommendation engine" → Python, ML, data pipelines, A/B testing)
   - Career trajectory and level (IC vs manager, growth direction)
   - Compensation expectations (from stated range + market data)
   - Work style preferences (remote, hybrid, company size, industry)
   - Dealbreakers (non-negotiables the agent should never recommend against)
3. Continuously scrapes job listings across platforms
4. Parses each JD to extract:
   - Required vs preferred skills
   - True seniority level (not just title — "Senior" means different things at different companies)
   - Compensation signals (posted range, market data for role/location/company)
   - Team/org signals (team size, reporting structure, tech stack)
   - Red flags (ghost job indicators, unrealistic requirements, high turnover signals)
5. Scores each job on multiple dimensions and presents a curated feed with EXPLANATIONS

**Inputs needed:**
- Resume/CV (structured or unstructured)
- LinkedIn profile data (with user permission, via profile export or manual input)
- Stated preferences (role type, location, salary range, company size, industry)
- Feedback on recommendations (thumbs up/down with optional reasoning)

**Actions it takes:**
- Scrapes job boards on a schedule (daily or more frequent)
- Parses and structures JDs using NLP
- Computes semantic similarity between candidate profile and job requirements
- Filters out ghost jobs using heuristics (posting age, company hiring patterns, salary transparency, specificity)
- Ranks remaining jobs by multi-dimensional fit score
- Generates natural language explanations for each recommendation

**APIs and data sources:**
- **Job listings:** JobSpy (open source, covers LinkedIn/Indeed/Glassdoor/Google/ZipRecruiter), Adzuna API (12 countries, free tier), unified ATS APIs (Merge.dev, unified.to) for direct employer listings
- **Skill ontologies:** ESCO API (free), O*NET (free, downloadable)
- **Compensation data:** levels.fyi API/scraping, Glassdoor salary data, BLS occupational data
- **Company intel:** Glassdoor reviews, LinkedIn company pages, Crunchbase (funding/growth signals)
- **Embeddings:** JobBERT v2, CareerBERT, or fine-tuned sentence-transformers on job/resume pairs

**What the human still needs to do:**
- Provide initial profile and preferences
- Give feedback on recommendations to calibrate the system
- Make the final "should I apply?" decision
- Evaluate culture/team fit signals the agent can't fully assess

**Ghost job detection heuristics:**
- Posting age > 60 days with no updates
- Reposted multiple times without changes
- Vague requirements with no specific tech stack or projects
- Company has posted the same role continuously for 6+ months
- No named hiring manager or team
- Salary range absent in states with transparency laws
- Company headcount declining while posting aggressively
- Cross-reference with layoff databases (layoffs.fyi)

### Level 2: Semantic skill inference engine (medium-term, differentiated)

**Beyond keyword matching — actually understanding what someone can do:**

Traditional: "Does resume contain 'Kubernetes'?" → yes/no
Semantic: "Candidate managed containerized deployments at scale across 3 cloud providers" → infers Kubernetes, Docker, ECS/EKS/GKE, IaC, CI/CD, monitoring, incident response

**Technical approach:**
1. Parse candidate's experience descriptions into structured claims (project, scale, technology, outcome)
2. Map claims to skill ontology (ESCO + custom tech skills taxonomy)
3. Infer adjacent/implied skills using skill graph relationships
4. Assign confidence levels to each skill (explicit mention > strong inference > weak inference)
5. Compare candidate skill graph to JD requirement graph
6. Score on coverage (what % of required skills are present), depth (how strong is each skill), and transferability (for skills not directly present, how learnable are they given adjacent skills)

**Key technical decisions:**
- Use embedding similarity (cosine distance) as a first-pass filter, then LLM reasoning for top candidates — combining speed with accuracy
- Maintain a living skill graph that updates as the candidate adds experience
- Weight recent experience higher than older experience
- Handle career transitions: "I was a PM for 5 years, now I want to be an engineer" requires different matching logic

### Level 3: Proactive career intelligence (longer-term, moat-building)

**The agent doesn't just match to current listings — it understands the market:**
- Tracks which skills are appearing more/less in JDs over time (skill demand trends)
- Identifies emerging roles before they have standardized titles
- Notices when a company the candidate is interested in starts hiring for a new team
- Alerts when a candidate's network connections join target companies (warm intro opportunities)
- Monitors compensation trends for the candidate's role/level/location
- Suggests upskilling paths based on gap between current profile and aspirational roles

**This creates a flywheel:** Better matching → more user engagement → more feedback data → better matching. The agent gets smarter about what THIS specific person considers a good job, not just what's statistically popular.

### Failure modes and risks

1. **Over-filtering:** Agent is too selective, candidate misses good opportunities that don't match on paper. Mitigation: always include a "stretch" category with explanation
2. **Stale preferences:** Candidate's goals change but agent keeps matching to old profile. Mitigation: periodic check-ins, detect drift in feedback patterns
3. **Ghost job false positives:** Agent incorrectly flags a real job as ghost. Mitigation: never silently filter — flag with confidence level and let user decide
4. **Semantic hallucination:** Agent infers skills the candidate doesn't have. Mitigation: always show inference chain, let candidate confirm/deny
5. **Echo chamber:** Agent only shows jobs similar to what candidate already has. Mitigation: explicitly include "career expansion" recommendations
6. **Gaming by employers:** Companies optimize JDs to match more candidates regardless of actual requirements. This is already happening and will accelerate
7. **Data freshness:** Scraped jobs go stale. Mitigation: re-verify before presenting, prioritize recent postings
8. **Volume collapse:** If matching is TOO good, candidates apply to very few jobs. In a world of ghost jobs and ghosting employers, you need SOME volume. The agent needs to calibrate between quality and quantity

## Technical considerations

### Job listing data access (ranked by viability)

**Tier 1: Clean API access**
- **Adzuna API:** Free developer tier, 12 countries, structured data, historical salary data. Best starting point for an MVP
- **Greenhouse Job Board API:** Public, returns structured job data for companies using Greenhouse. No auth needed
- **Lever Postings API:** Similar to Greenhouse, public job postings
- **Unified.to / Merge.dev:** Unified ATS APIs covering 60+ platforms. Real-time data, zero-storage architecture (unified.to). Likely the most efficient path to broad coverage

**Tier 2: Scraping (works but fragile)**
- **JobSpy:** Open source Python library. LinkedIn (rate-limited, needs proxies), Indeed (no rate limiting), Glassdoor, Google, ZipRecruiter. Capped at ~1,000 results per search. Returns title, company, location, salary, description, job_type
- **LinkedIn guest API:** Public job listings available at linkedin.com/jobs-guest/ without auth. HTML parsing required. LinkedIn actively fights this
- **Apify actors:** Cloud-based scrapers for LinkedIn jobs ($0.005-$0.01/result). Managed proxies handle rate limiting

**Tier 3: Restricted/unavailable**
- **LinkedIn Talent API:** Partners only. The most valuable data (who's hiring, who's looking, network graph) is locked behind partnership agreements
- **Indeed Publisher API:** Deprecated for job search use as of 2023. Only hiring-side APIs remain
- **Workday/iCIMS:** No public APIs for job search. Browser automation is the only path, and both actively detect it

### Legal and ToS landscape

- **LinkedIn:** hiQ Labs v. LinkedIn (2022 Supreme Court) established that scraping public data isn't necessarily a CFAA violation. BUT LinkedIn's ToS prohibits it, and they actively enforce via lawsuits (Jan 2025 case led to Proxycurl shutdown in July 2025). Legal gray area
- **Indeed:** ToS prohibits scraping. Less aggressive enforcement than LinkedIn
- **Greenhouse/Lever public APIs:** Explicitly designed for this use case. Clean
- **GDPR/CCPA:** Processing job listings is generally fine. Processing personal data (recruiter info, candidate data) requires consent frameworks
- **Safest approach:** Use official APIs where available, aggregate from public job board APIs, use scraping ONLY as a supplement with appropriate rate limiting and respect for robots.txt

### Embedding and matching architecture

**Recommended stack:**
1. **Job parsing:** LLM (Claude/GPT) to extract structured fields from raw JDs → required skills, preferred skills, level, comp range, location, team signals
2. **Skill mapping:** Map extracted skills to ESCO/O*NET ontology using JobBERT v2 or similar domain-specific embeddings
3. **Candidate encoding:** Encode candidate profile into same embedding space using CareerBERT approach (fine-tuned SBERT)
4. **Similarity scoring:** Cosine similarity for initial ranking, then LLM-based reasoning for top-N explanation generation
5. **Feedback loop:** Use candidate feedback (apply/skip/save) to fine-tune personal preference model via contrastive learning
6. **Ghost detection:** Rule-based heuristics + ML classifier trained on confirmed ghost vs real jobs

**Scale considerations:**
- New job listings: ~500K-1M per day across major US boards
- After deduplication and ghost filtering: ~300K-600K
- Per-candidate matching: can be done in near-real-time with pre-computed embeddings
- Storage: vector DB (Pinecone, Weaviate, pgvector) for job embeddings, refreshed daily
- Cost: embedding generation is cheap ($0.01-0.10 per 1K jobs with open models), LLM reasoning for top matches is the expensive part (~$0.01-0.05 per job explanation with Claude Haiku)

### Differentiation from existing tools

The gap in the market isn't "another AI job board." It's:
1. **Transparency:** Show WHY a job was recommended. No other tool does this well
2. **Ghost filtering:** Actively protect users from wasting time. No one markets this
3. **Skill inference:** Understand what candidates CAN do, not just what they SAY they can do
4. **Feedback-driven personalization:** Get dramatically better for each user over time
5. **Integration with the full workflow:** Matching is the START. Connected to resume tailoring, application tracking, interview prep, and negotiation (see other specs in this series)

## Open questions

1. **How good is ghost job detection in practice?** We have heuristics but no validated model. Need to build a labeled dataset of confirmed ghost vs real jobs and measure precision/recall
2. **How do career transitioners use this?** Someone switching from marketing to data science has a fundamentally different matching problem than someone looking for their next senior SWE role. How does the agent handle the gap between "what I've done" and "what I want to do"?
3. **What's the right match volume?** Too few recommendations → user feels stuck. Too many → alert fatigue returns. Need to A/B test daily recommendation volume (hypothesis: 3-7 per day is the sweet spot)
4. **Compensation data accuracy:** levels.fyi is good for Big Tech but sparse for startups/mid-market. Glassdoor data is noisy. How do we build reliable comp estimates for the long tail?
5. **How to handle the "hidden job market"?** 30-50% of jobs are never posted publicly. The agent can only match against what it can see. Networking/referral features (see networking-referrals-agentic spec) need to complement matching
6. **Candidate-side gaming:** If users know the matching algorithm, they'll optimize their profiles to match more jobs (like SEO for resumes). How do we keep the signal genuine?
7. **Two-sided marketplace dynamics:** If both sides use AI agents (candidates matching to jobs, employers matching to candidates), do the agents converge on the same matches? Or create new failure modes?
8. **Latency vs freshness tradeoff:** Job postings get most applications in the first 48 hours. How fast does our pipeline need to be from posting → recommendation? Sub-hour is probably necessary for competitive roles

## Sources

- [CareerBERT: Matching Resumes to ESCO Jobs (2025)](https://arxiv.org/abs/2503.02056) — Expert Systems with Applications, MAP@20 of 0.711
- [Resume2Vec: Intelligent Resume Embeddings (2025)](https://www.mdpi.com/2079-9292/14/4/794) — 15.85% improvement over conventional ATS
- [JobBERT v2 — TechWolf](https://huggingface.co/TechWolf/JobBERT-v2) — Domain-specific job title embeddings
- [ESCO-O*NET Crosswalk](https://esco.ec.europa.eu/en/about-esco/data-science-and-esco/crosswalk-between-esco-and-onet)
- [JobSpy — Open Source Job Scraper](https://github.com/speedyapply/JobSpy) — LinkedIn, Indeed, Glassdoor, Google, ZipRecruiter aggregation
- [Ghost Jobs 2026 — Fonzi AI](https://fonzi.ai/blog/ghost-jobs-meaning) — 30% of postings are ghost jobs
- [LiveCareer Ghost Jobs Survey (March 2025)](https://blog.theinterviewguys.com/ghost-jobs-exposed/) — 93% of HR professionals admit to posting ghost jobs
- [AI in Recruitment 2026 — InCruiter](https://incruiter.com/blog/ai-in-recruitment-2026-trends-stats-what-works/) — 78% of screenings use NLP
- [Semantic Search vs Boolean — SpotSaaS](https://www.spotsaas.com/blog/ai-matching-feature-in-ats-what-it-means/) — 60% more relevant profiles, 62% fewer false positives
- [Huntr Job Search Trends Q1 2025](https://huntr.co/) — Job seeker fatigue statistics
- [LinkedIn Scraping Landscape 2026 — Generect](https://generect.com/blog/linkedin-scraping/) — Proxycurl shutdown, legal landscape
- [Sorce — AI Job Search Reviews](https://www.funblocks.net/aitools/reviews/sorce) — Swipe-based matching UX
- [Merge.dev ATS Unified API](https://docs.merge.dev/ats/) — 60+ ATS platform integration
- [Unified.to ATS API](https://docs.unified.to/ats/overview) — Real-time, zero-storage ATS aggregation
- [Adzuna Developer API](https://developer.adzuna.com/) — 12-country job listing API
- [Greenhouse Job Board API](https://developers.greenhouse.io/job-board.html) — Public job listing access
- [AIHR Skills Ontology Guide](https://www.aihr.com/blog/skills-ontology/) — Skills ontology vs taxonomy explained
- [Nodes.inc AI Matching Platforms](https://nodes.inc/blogs/most-accurate-ai-candidate-matching-platforms-2025) — Comparative accuracy data
- [Contrastive ESCO Skill Extraction (2026)](https://arxiv.org/html/2601.09119) — BERT-based multi-label skill extraction
