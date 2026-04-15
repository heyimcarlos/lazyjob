# Skills & Endorsements

## What it is

LinkedIn's skills and endorsements system is the backbone of the platform's professional identity and matching infrastructure. It consists of three interconnected layers: (1) a **skills taxonomy** of ~39,000 standardized skills with 374,000+ aliases across 26 languages, organized in a polyhierarchical graph with 200,000+ edges; (2) an **endorsement system** that lets connections validate others' claimed skills with a single click; and (3) a **Skills Graph** that dynamically maps relationships between skills, 875M+ people, 59M companies, and millions of job postings to power job matching, search ranking, content recommendations, and learning suggestions. LinkedIn has been aggressively pivoting toward "skills-first hiring" — positioning skills as the atomic unit of professional identity, replacing degrees and job titles as the primary matching criterion.

## How it works — User perspective

### Adding skills to your profile

Members can add up to **100 skills** to their profile (though 20-30 is the recommended sweet spot). Skills are selected from LinkedIn's standardized taxonomy — you type a skill name and get autocomplete suggestions from the ~39,000 recognized skills. You can **pin 3 skills** to the top of your skills section (these appear most prominently and get the most endorsement traffic). The remaining skills can be reordered via drag-and-drop.

LinkedIn also **auto-suggests skills** based on your job title, industry, and profile content. When you add a new position, LinkedIn will suggest relevant skills. The system increasingly **infers skills** from your profile text even if you haven't explicitly listed them — your job descriptions, about section, and activity are all mined for skill signals.

Skills are categorized as either **hard skills** (technical, domain-specific: "Python," "Financial Modeling") or **soft skills** (interpersonal: "Leadership," "Communication"). Each skill has a unique identifier, definition, and set of aliases (abbreviations, translations, alternative phrasings).

### Receiving and giving endorsements

**Giving endorsements**: Visit someone's profile, scroll to their Skills section, and click the "+" button next to any skill. It's a one-click action — no text, no explanation, no proof required. LinkedIn limits endorsements to **150 per 24 hours** to prevent spam.

**Prompted endorsements**: LinkedIn aggressively prompts endorsements. When you visit a connection's profile, a popup often appears asking "Does [Name] have these skills?" with pre-selected skills and one-click endorse buttons. LinkedIn also prompts endorsements in the feed, notifications, and after accepting connection requests. This friction-free prompting is the primary driver of endorsement volume — most endorsements come from prompts, not intentional visits.

**Receiving endorsements**: You get a notification when someone endorses you. You can choose to show or hide individual endorsements. Endorsements accumulate as a count next to each skill (e.g., "Python · 99+"). You can see who endorsed you for each skill.

**Endorsement quality signals**: Not all endorsements are weighted equally by the algorithm. An endorsement from someone who themselves is highly endorsed for that skill (e.g., a senior data scientist endorsing you for "Machine Learning") carries more weight than an endorsement from someone with no demonstrated expertise in that area. Endorsements that accumulate gradually over time carry more weight than bursts of endorsements received in a short period (anti-gaming measure).

### The "Highly Skilled" designation

When you accumulate sufficient high-quality endorsements for a skill, LinkedIn may display a **"Highly Skilled"** badge. The exact threshold is undisclosed, but factors include: endorsement volume (dozens to hundreds depending on skill competitiveness), endorser authority in that skill area, overall profile strength, and consistency over time.

### Skill Assessments (now discontinued)

LinkedIn previously offered **Skill Assessments** — 15-question timed quizzes (1.5 minutes per question) covering technical, business, and design skills. Scoring in the **top 30%** earned a skill badge displayed on your profile. You could retake a failed assessment once every 6 months. Recruiters could see badges but not scores.

**LinkedIn discontinued Skill Assessments in late 2023**, removing all badges by early 2024. The stated reason: hirers told LinkedIn they valued seeing *how candidates applied skills* (through projects, credentials, experience) more than standardized test results. This was also likely driven by the widespread availability of answer keys on GitHub (the `linkedin-skill-assessments-quizzes` repository has thousands of stars with complete answer sets).

### Skills verification (2026 — new)

In January 2026, LinkedIn launched a **partner-based skills verification system**. Instead of quizzes, partner companies (Descript, Lovable, Relay.app, Replit, with GitHub, Gamma, Zapier coming soon) verify skills based on **actual usage patterns and product outcomes** within their tools. These credentials are **dynamic** — they automatically update as skills improve, reflecting current capabilities rather than historical achievement. This represents a fundamental shift from self-reported/peer-validated skills to tool-verified demonstrated proficiency.

### Tagging skills to experience

After discontinuing assessments, LinkedIn made it possible for members to **tag skills to specific credentials** — connecting skills to particular jobs, projects, or education entries on their profile. This provides context: instead of just "Project Management" as a floating skill, you can show it's connected to your role as VP of Operations at Company X. This contextual linking makes skills more credible and traceable.

## How it works — Technical perspective

### Skills Taxonomy architecture

The taxonomy is the vocabulary layer. Each skill is a **node** with:
- Unique identifier
- Canonical name
- Skill type (hard/soft)
- Definition
- Aliases (374,000+ across 26 locales — abbreviations, translations, alternative names)

Nodes are connected by **knowledge lineages** — directed parent-child edges that form a polyhierarchical graph. "Polyhierarchical" means a single skill can have multiple parents and children across domains. Example: "Offshore Construction" connects to both "Construction" (parent) and "Oil and Gas" (parent). This enables rich inference: knowing "Artificial Neural Networks" implies knowledge of both "Deep Learning" and "Machine Learning."

**Taxonomy curation** uses a human-ML hybrid approach:
- **Human taxonomists** manually assign relationships, investigate LinkedIn usage metadata, disambiguate terms (e.g., clarifying "Cadence" to "Cadence Software"), and enforce quality standards
- **KGBert model** (inspired by KG-BERT architecture) predicts parent-child relationships using BERT embeddings, achieving 20%+ F1 improvement over previous models. This enables scaling to thousands of new skill candidates per period while maintaining quality

Since February 2021, the taxonomy has grown ~35%.

### Skills Graph

The Skills Graph is the inference and application layer built on top of the taxonomy. It maps relationships between skills, people, companies, and jobs. Infrastructure:
- Taxonomy data is transformed into the graph via a **big data pipeline**
- Online access via **Rest.Li service** (LinkedIn's custom REST framework)
- Offline access via **HDFS (Hadoop Distributed File System)** dataset
- Powers both real-time (search, recommendations) and batch (analytics, model training) use cases

### Skill extraction pipeline

LinkedIn doesn't rely solely on self-reported skills. A four-stage ML pipeline extracts skills from all content:

**1. Skill Segmentation**: Parses unstructured content (job postings, profiles, resumes) into structural sections. Context matters — skills in a "qualifications" section get higher relevance than skills in a "company description."

**2. Skill Tagging** (dual approach):
- **Token-based matching**: Trie-based tagger encoding the full taxonomy for fast, scalable exact matching
- **Semantic matching**: Two-tower neural model using **Multilingual BERT** encoders for contextual understanding. Captures indirect mentions like "experience with design of iOS application" → "Mobile Development"

**3. Skill Expansion**: Queries the Skills Graph for related skills via parent-child relationships and skill groupings, broadening the candidate match set.

**4. Multitask Cross-Domain Scoring**: A Transformer-based architecture with:
- **Shared module**: Contextual Text Encoder + Contextual Entity Encoder
- **Domain-specific towers**: Separate model instances for job postings, member profiles, and feed content
- Classifies content-skill relationships as: "mention/valid," "required," or "core" (essential for job regardless of explicit mention)

**Production optimization**: The full 12-layer BERT model was compressed by **80%** via knowledge distillation to meet <100ms latency requirements. The system handles ~200 profile updates/second globally via nearline processing, with Spark-based offline scoring for full data reprocessing.

**Feedback loops**: Three product-integrated mechanisms refine models continuously:
1. Recruiter skill feedback (manual validation when posting jobs)
2. Job seeker skill feedback (reviewing/correcting skill matches)
3. Member profile feedback (assessment and profile interaction data)

**Measured impact from A/B testing**:
- Job Recommendation: +0.46% predicted confirmed hires
- Job Search: +0.76% PPC revenue
- Skill matching: +0.87% qualified applications, +0.24% predicted confirmed hires

### Endorsement algorithm

Endorsements function as weighted signals in LinkedIn's ranking systems:
- **Volume**: More endorsements → higher search ranking weight
- **Endorser authority**: Weighted by the endorser's own skill graph position (endorsement from someone highly skilled in that area > endorsement from someone with no demonstrated expertise)
- **Temporal distribution**: Gradual accumulation > sudden bursts (anti-gaming)
- **Profile coherence**: Endorsements that align with headline, about section, and work history boost authority more
- **Suspicious pattern detection**: Endorsement rings and coordinated activity are detected and down-weighted

Endorsed profiles are reportedly **17x more likely** to appear in recruiter searches than unendorsed profiles.

### Future direction

LinkedIn is investing in:
- **LLM-powered skill descriptions** for richer semantic understanding
- **Embedding-based skill representation** as the primary matching mechanism (replacing exact text matching)
- **Partner-verified dynamic credentials** (the 2026 verification initiative)

## What makes it successful

### 1. The taxonomy is the moat

LinkedIn's 39,000-skill taxonomy with 200,000+ edges is extraordinarily difficult to replicate. It took years of human curation + ML refinement, informed by the behavioral data of 875M+ members. No competitor has a comparable skills ontology built on comparable behavioral data. This taxonomy is the foundation that makes skills-first matching possible at scale.

### 2. Endorsements exploit zero-cost social reciprocity

The endorsement system is brilliant behavioral design despite its quality problems. By making endorsements one-click and aggressively prompting them, LinkedIn generated massive endorsement volume across the network. The reciprocity dynamic (you endorse me, I feel obligated to endorse you) creates a self-reinforcing cycle. The result: most active profiles have endorsement data, giving LinkedIn signal even when people don't fill out comprehensive skills sections.

### 3. Implicit skill extraction creates a complete picture

Most members only explicitly list a fraction of their skills. The ML extraction pipeline fills the gaps, inferring skills from job descriptions, profile text, posted content, and engagement patterns. This means LinkedIn has a skills picture even for passive members who never curate their profile — critical for recruiter search and job matching.

### 4. Skills-first hiring is a strategic bet that aligns incentives

By positioning skills over degrees, LinkedIn:
- Expands the addressable market (people without degrees are a massive underserved segment)
- Increases matching quality (skills are more predictive of job fit than credentials)
- Creates platform lock-in (your skills graph is built on LinkedIn's proprietary taxonomy)
- Drives Premium/Recruiter revenue (better matching = more willing-to-pay recruiters)

The data: skills-first hiring increases candidate pools by 9% more for non-degree workers and increases women in underrepresented roles by 24%.

### 5. The graph creates compounding value

Every new skill added, endorsement given, job posted, or content published enriches the Skills Graph. More data → better matching → more users → more data. Classic network effect, but on the skill/capability layer rather than just connections.

## Weaknesses and gaps

### 1. Endorsements are fundamentally low-signal

The core problem: **anyone can endorse anyone for anything with zero verification**. Your college friend who works in marketing can endorse you for "Kubernetes" with one click. LinkedIn's aggressive prompting actually makes this worse — people endorse reflexively when prompted, not because they've evaluated competence. Research shows most endorsements come from reciprocity or prompts rather than genuine expertise assessment.

The "Highly Skilled" designation attempts to solve this with quality weighting, but the underlying signal remains noisy. Recruiters largely discount endorsements: they're useful as a tiebreaker at best, never as primary evidence of competence.

### 2. Skill Assessments failed and left a verification vacuum

The discontinuation of Skill Assessments in 2023 removed the only first-party verification mechanism. The stated reason (hirers prefer experience-based evidence) was partially true, but the real driver was that answer keys were publicly available on GitHub, making the assessments trivially gameable. The 2026 partner verification initiative is promising but extremely limited in scope — it only covers a handful of AI/developer tools, leaving 99%+ of the 39,000 skills unverifiable.

### 3. Skills are still self-reported and uncontextualized

Despite ML extraction, the core skills on a profile are still self-reported claims. There's no mechanism to verify that someone who lists "Python" can actually write Python. Tagging skills to experience entries helps but doesn't solve the fundamental verification problem. The gap between claimed skills and actual competence remains enormous.

### 4. The taxonomy struggles with emerging skills

Despite the KGBert model and human curation, the taxonomy inherently lags behind rapidly evolving skill landscapes. New frameworks, tools, and methodologies emerge faster than the taxonomy can be updated. This is especially acute in fast-moving fields like AI/ML, where the relevant skill set changes quarterly.

### 5. Soft skills are essentially unverifiable

The taxonomy includes soft skills ("Leadership," "Communication," "Problem Solving") but these are completely unverifiable through any mechanism — endorsements, assessments, or extraction. They function as empty signals that everyone adds. LinkedIn has no way to differentiate genuine leadership ability from self-reported claims.

### 6. The endorsement UX is widely criticized as spammy

LinkedIn's aggressive endorsement prompting (popups on profile visits, notifications, feed cards) is a common complaint. It drives volume but degrades user experience and perception of endorsement quality. Many users view endorsements as a game rather than a meaningful professional validation.

### 7. No skill proficiency levels

LinkedIn skills are binary — you either have a skill or you don't. There's no standardized way to indicate proficiency level (beginner vs. expert). Endorsement count is a weak proxy, and the discontinued Skill Assessments only provided a pass/fail against the top 30%. Competing platforms like Pluralsight (Skill IQ with numerical proficiency scores) and HackerRank (difficulty-tiered challenges) handle this much better.

## Competitive landscape

### Workday Skills Cloud
Enterprise-focused skills ontology using AI to surface connections between skills within organizations. Strength: integrated with HCM workflows for internal mobility and upskilling. Weakness: limited to enterprise contexts (not a professional network), taxonomy customization is difficult, and it lacks the behavioral data that LinkedIn's 875M members provide.

### O*NET and ESCO (public taxonomies)
Government-maintained skills taxonomies. **O*NET** (US Department of Labor): ~20,000 skills and knowledge areas mapped to 900+ occupations. **ESCO** (European Commission): skills, competences, qualifications, and occupations framework used for workforce development. Strength: publicly available, standardized, government-backed. Weakness: slow to update, not connected to behavioral data, no social validation layer.

### HackerRank
Developer-focused skills verification through coding challenges. 26M+ developers, 3,000+ companies. Offers **skills certification** through practical coding challenges across difficulty tiers. Strength: skills are *demonstrated* not *claimed* — you prove competence by solving problems. Weakness: limited to programming/technical skills, no social/professional network context.

### Pluralsight Skill IQ
Technology skills assessment platform offering numerical Skill IQ scores measuring proficiency relative to peers. Strength: granular proficiency measurement (not just pass/fail), covers a wide range of tech skills, integrated learning paths to close gaps. Weakness: limited to tech, self-selecting assessment population, no professional network.

### GitHub
Launched formal certifications (GitHub Foundations, GitHub Actions, GitHub Advanced Security, etc.) verified through proctored exams. The profile itself serves as a portfolio of demonstrated work. Strength: proof-of-work (your commit history IS your skills evidence), certifications tied to practical platform competence. Weakness: only relevant for developers, no broader professional skills coverage.

### Credly/Acclaim (digital credentials)
Platform for issuing and displaying verified digital badges from authorized issuers (universities, training providers, companies). Strength: credentials are issued by verified authorities, not self-claimed. Weakness: no unified taxonomy, fragmented across issuers, no social validation layer.

### iMocha
Enterprise skills assessment platform with 5,000+ skills. Offers AI-powered skills intelligence, coding simulators, and certification-grade assessments. Strength: granular assessments across many skill domains, strong enterprise integration. Weakness: enterprise-only, no consumer/professional network.

### Key competitive insight

No single competitor matches LinkedIn's combination of (1) massive behavioral data from 875M+ members, (2) a comprehensive cross-domain taxonomy, (3) social validation (endorsements), and (4) direct connection to job matching. However, competitors consistently outperform LinkedIn on **verification** — proving skills rather than claiming them. HackerRank, GitHub, and Pluralsight all offer stronger evidence of actual competence than LinkedIn's endorsement-based system.

## Relevance to agent platforms

### What transfers directly

**Structured skill taxonomies**: Agents need capability descriptions that are machine-readable, standardized, and hierarchical. LinkedIn's taxonomy architecture — nodes with unique IDs, polyhierarchical edges, aliases — is directly applicable. An agent capability taxonomy would enumerate things like "text summarization," "code generation," "image classification," with parent-child relationships (e.g., "sentiment analysis" → "NLP" → "machine learning").

**The Skills Graph concept**: Mapping relationships between agent capabilities, tasks, organizations, and workflows is the agent equivalent of LinkedIn's Skills Graph. This powers the core matching problem: given a task, which agent(s) have the right capabilities?

**Contextual skill extraction**: LinkedIn's approach of inferring skills from unstructured content (job postings, profiles) translates to inferring agent capabilities from documentation, API specs, benchmark results, and usage logs.

### What needs fundamental reimagining

**Verification is the killer advantage**: Unlike human skills, agent capabilities can be **objectively verified**. This is the single biggest opportunity for an agent platform. Instead of endorsements (subjective, gameable), an agent platform can offer:
- **Benchmark results**: Standardized evaluations on published test suites
- **Audit trails**: Verifiable logs of tasks completed, accuracy achieved, resources consumed
- **Live capability testing**: On-demand verification (send a test prompt, get a measured result)
- **Usage-based proficiency**: Actual performance data from real-world deployments (akin to LinkedIn's 2026 partner verification, but universal)

This flips LinkedIn's biggest weakness (unverifiable self-reported claims) into the agent platform's biggest strength.

**Proficiency levels matter more for agents**: LinkedIn's binary skills (have/don't have) is inadequate for agents. An agent platform needs rich proficiency metadata: accuracy percentages, latency distributions, token costs, supported context lengths, fine-tuning specializations. A skill like "code generation" needs to specify: what languages, what complexity levels, what accuracy on what benchmarks.

**Dynamic capabilities replace static skills**: Human skills change slowly. Agent capabilities change with every model update, fine-tune, or tool integration. The skills system needs to be **real-time** — reflecting current capabilities, not historical claims. LinkedIn's 2026 dynamic credentials initiative points in this direction, but an agent platform should make this foundational.

**Endorsements become reviews/ratings**: The social validation layer transforms from "click to endorse" into structured reviews from organizations and users who have actually used the agent. Think app store ratings with structured feedback on specific capabilities, not one-click vanity metrics.

**Composability replaces endorsement networks**: For agents, the interesting graph isn't "who endorses whom" but "who works well with whom." Agent skill graphs should map **interoperability** — which agents compose well together, which have complementary capabilities, which have compatible interfaces.

### What's irrelevant

- **Self-reported skills**: Agents don't need to self-report. Their capabilities can be measured.
- **Soft skills**: Agents don't have "leadership" or "communication skills" in the human sense. Their interaction qualities are measurable properties (response quality, coherence, safety).
- **Endorsement reciprocity/social dynamics**: The behavioral psychology that drives LinkedIn endorsements (reciprocity, social obligation) doesn't apply to agents. Verification should be objective, not social.
- **Profile completion gamification**: Agents don't need to be nudged to fill out their profiles. Capability manifests can be auto-generated from benchmarks and usage data.

## Sources

- [Building and maintaining the skills taxonomy that powers LinkedIn's Skills Graph](https://www.linkedin.com/blog/engineering/data/building-maintaining-the-skills-taxonomy-that-powers-linkedins-skills-graph) — LinkedIn Engineering Blog, March 2023
- [Building LinkedIn's Skills Graph to Power a Skills-First World](https://www.linkedin.com/blog/engineering/skills-graph/building-linkedin-s-skills-graph-to-power-a-skills-first-world) — LinkedIn Engineering Blog
- [Extracting skills from content to fuel the LinkedIn Skills Graph](https://www.linkedin.com/blog/engineering/skills-graph/extracting-skills-from-content) — LinkedIn Engineering Blog, 2023
- [Skills-First: Reimagining the Labor Market and Breaking Down Barriers](https://economicgraph.linkedin.com/research/skills-first-report) — LinkedIn Economic Graph
- [LinkedIn AI Skills Verification](https://fortune.com/2026/01/28/linkedin-ai-skills-verification-profile-skills-mismatch/) — Fortune, January 2026
- [Skills-Based Hiring Report](https://economicgraph.linkedin.com/content/dam/me/economicgraph/en-us/PDF/skills-based-hiring-march-2025.pdf) — LinkedIn Economic Graph, March 2025
- [LinkedIn Skill Assessments Help](https://www.linkedin.com/help/linkedin/answer/a507663/linkedin-skill-assessments) — LinkedIn Help Center
- [Skill Assessments - no longer available](https://www.linkedin.com/help/linkedin/answer/a1690529) — LinkedIn Help Center
- [Skill Endorsements Overview](https://www.linkedin.com/help/linkedin/answer/a565106) — LinkedIn Help Center
- [How LinkedIn Endorsements Actually Work](https://www.ritnerdigital.com/blog/how-linkedin-endorsements-actually-work-and-how-to-get-the-highly-skilled-designation) — Ritner Digital
- [LinkedIn Skills on the Rise 2025](https://www.linkedin.com/pulse/linkedin-skills-rise-2025-15-fastest-growing-us-linkedin-news-hy0le) — LinkedIn News
- [The LinkedIn Endorsement Game: Why and How Professionals Attribute Skills to Others](https://www.researchgate.net/publication/310811088_The_LinkedIn_Endorsement_Game_Why_and_How_Professionals_Attribute_Skills_to_Others) — ResearchGate
- [HackerRank Skills Verification](https://www.hackerrank.com/skills-verification) — HackerRank
- [Pluralsight Skill IQ](https://www.pluralsight.com/product/skills-assessment) — Pluralsight
- [Workday Skills Cloud](https://www.workday.com/en-us/products/human-capital-management/skills-cloud.html) — Workday
- [GitHub Certifications](https://docs.github.com/en/get-started/showcase-your-expertise-with-github-certifications/about-github-certifications) — GitHub Docs
