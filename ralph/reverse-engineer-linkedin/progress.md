# Progress Log

Started: 2026-04-14
Objective: Reverse-engineer LinkedIn, job search platforms, and X.com professional features. Write research specs to ../specs/

---

## Iteration 1 — Profile System (2026-04-14)

**Task**: #1 profile-system
**Output**: `../../specs/profile-system.md`

### Key findings

1. **Profile structure**: 14+ sections organized in a fixed hierarchy. The Top Card (photo, headline, name, location) is the most critical — it appears everywhere. Headline (220 chars) is the single most important SEO field. About section is 2,600 chars. Up to 100 skills from a 41,000+ taxonomy.

2. **Profile strength meter**: 5 levels (Beginner → All-Star). Requires 7 core sections + 50 connections for All-Star. Meter disappears after completion. All-Star profiles are 27x more likely to appear in recruiter searches. Smart gamification that drives completion without annoying power users.

3. **Technical architecture**: Three major phases — monolith → microservices (750+) → view-centric component model. Current system uses recursive component schemas via Rest.li with union types. Single API returns ordered "cards" of components. 67% code reduction on clients. Infrastructure: Espresso (document store), Kafka (500B+ events/day), Voldemort (precomputed insights), GraphDB (connection graph).

4. **Profile analytics**: "Who Viewed Your Profile" is both an engagement driver (reciprocal curiosity loops) and Premium conversion mechanism (blurred details for free users). Three privacy browsing modes create a reciprocal system — go anonymous and you lose your own viewer data.

5. **Open to Work**: Two visibility modes (recruiter-only vs all members). Recruiter-only mode gives 40% uplift in outreach. Public green banner gives 14.5% positive response rate vs 4.6% without.

6. **Major weaknesses**: No employment verification (21M+ fake accounts removed per period), profiles are self-reported claims with no proof of work, one-size-fits-all structure doesn't serve different professions, engagement-farming culture degrades signal quality.

7. **Agent platform relevance**: Structured capability profiles transfer directly. Key advantage: agent capabilities CAN be verified (benchmarks, audit trails) unlike human claims. Key reimagining needed: profiles as live system status + capability manifests rather than static documents. Composability matters more than networking for agents.

### Notes for next iterations
- Skills and endorsements (task #2) will build directly on the skills taxonomy mentioned here
- The component architecture pattern is relevant to understanding how all LinkedIn features are built
- LinkedIn's transition to semantic entity mapping (Knowledge Graph) in 2026 is a cross-cutting theme that will be relevant to search, job matching, and feed tasks

---

## Iteration 2 — Skills & Endorsements (2026-04-14)

**Task**: #2 skills-endorsements
**Output**: `../../specs/skills-endorsements.md`

### Key findings

1. **Skills Taxonomy**: ~39,000 skills with 374,000+ aliases across 26 locales and 200,000+ edges. Polyhierarchical graph (skills can have multiple parents). Curated by human taxonomists + KGBert ML model (20%+ F1 improvement over prior models). Grown ~35% since Feb 2021. Accessible via Rest.li service and HDFS.

2. **Skills Graph**: Maps relationships between 39K skills, 875M people, 59M companies. Powers job matching, search, content recommendations, and learning suggestions. Four-stage ML extraction pipeline: segmentation → tagging (trie-based + two-tower Multilingual BERT) → expansion → multitask cross-domain scoring. 12-layer BERT compressed 80% via knowledge distillation for <100ms latency.

3. **Endorsements**: One-click social validation, aggressively prompted by LinkedIn. 150/day rate limit. Algorithm weighs endorser authority, temporal distribution, and profile coherence. Endorsed profiles 17x more likely to appear in recruiter searches. Major criticism: low-signal (anyone can endorse anyone), spammy UX, easily gamed. "Highly Skilled" designation requires high volume + quality endorsers.

4. **Skill Assessments (discontinued 2023)**: 15-question timed quizzes, top 30% got badge. Discontinued because (a) hirers preferred experience-based evidence, (b) answer keys were publicly available on GitHub. Left a verification vacuum.

5. **Skills Verification (2026 — new)**: Partner-based system (Descript, Lovable, Replit, etc.) verifying skills from actual tool usage patterns. Dynamic credentials that auto-update. Currently limited to a handful of AI/developer tools.

6. **Skills-first hiring**: LinkedIn's strategic bet — 88% of hirers filter out skilled candidates lacking traditional credentials. Skills-first increases non-degree candidate pools by 9% more, women in underrepresented roles by 24%. Companies using skills data are 60% more likely to find successful hires.

7. **Agent platform relevance**: THE key insight — agent capabilities can be objectively verified (benchmarks, audit trails, live testing), flipping LinkedIn's biggest weakness into an agent platform's biggest strength. Need rich proficiency metadata (accuracy %, latency, cost), real-time dynamic capabilities, and composability graphs (which agents work well together) rather than endorsement networks.

### Notes for next iterations
- The Skills Graph architecture is foundational context for task #5 (job-search-marketplace) and task #6 (search-and-discovery) — skills matching is core to both
- The skills-first hiring initiative is deeply connected to task #5 (job-search-marketplace)
- The skill extraction pipeline's Transformer architecture will be relevant for understanding the feed algorithm (task #4)
- The 2026 partner verification system is a significant new development worth tracking across multiple specs

---

## Iteration 3 — Network Graph & Connection System (2026-04-14)

**Task**: #3 network-graph
**Output**: `../../specs/network-graph.md`

### Key findings

1. **LIquid graph database**: LinkedIn's custom-built distributed graph DB storing 270B+ edges, handling 2M QPS. Triple-based edge model (subject, predicate, object) with log-structured in-memory inverted index. Single-writer, wait-free consistency using Datalog-based query language. When PYMK migrated from legacy GAIA to LIquid: QPS went from 120 to 18,000, latency from 1s+ to <50ms, CPU utilization dropped 3x.

2. **Connection degree system**: 1st/2nd/3rd degree tiers govern visibility, messaging access, search ranking, and content distribution. Average member has ~250 1st-degree connections, ~50,000+ 2nd-degree. Degree computation requires real-time graph traversal with pre-computation and caching for active members.

3. **PYMK (People You May Know)**: Responsible for 50%+ of all LinkedIn connections. Multi-stage pipeline: triangle closing candidate generation -> 100s of features (common connections, org overlap, temporal co-presence, geography) -> logistic regression scoring on 100s of millions of samples -> fairness re-ranking via LiFT toolkit. Impression discounting downranks ignored suggestions. Processes 100s of TBs daily.

4. **Follow vs. Connect**: Major 2024 shift — Follow became the default button (replacing Connect). Follow is unidirectional, zero-friction, unlimited. Connect is bidirectional, requires acceptance, limited to 100-200/week and 30,000 total. Creator Mode toggle was removed; features distributed to all users.

5. **Connection limits and anti-spam**: 100-200 requests/week (rolling), 500 pending cap, 30K total cap. Acceptance rate below ~30% triggers throttling. "I don't know this person" reports damage account health. Personalized notes boost acceptance from 15% to 30-45%.

6. **Fairness in PYMK**: LinkedIn's LiFT toolkit addresses "rich get richer" bias where frequent users were overrepresented. Post-processing re-ranking ensures infrequent members get fair representation — resulted in 5.44% more invites sent and 4.8% more connections to underrepresented members. Gender bias concerns persist as of 2025.

7. **LiGNN**: Production GNN framework operating on heterogeneous graph (members, jobs, posts, ads). Nearline inference pipeline via Kafka. 7x training speedup. Results: 0.1% WAU lift from people recommendations, 2% ads CTR lift. Temporal architectures with cold start solutions.

8. **Agent platform relevance**: Connection strength between agents is computable (API call frequency, success rates, latency) — flipping LinkedIn's biggest weakness. Agent relationships need TTLs, capability scoping, auto-deprecation. Key graph = composability graph ("who works well with whom" with performance metadata) not social graph. Degree-based access control maps to trust tiers for agent collaboration.

### Notes for next iterations
- LIquid database architecture is foundational context for search-and-discovery (task #6) and feed algorithm (task #4)
- The PYMK pipeline's fairness work is relevant to job-search-marketplace (task #5) — same LiFT toolkit applies
- Follow vs Connect dynamics are relevant to messaging/InMail (task #7) — follow doesn't grant messaging access
- The Economic Graph (connecting members, companies, skills, jobs, schools) is a cross-cutting concept relevant to nearly all remaining tasks
- LiGNN's cross-domain GNN approach is relevant to feed-algorithm (task #4) and job-search (task #5)

---

## Iteration 4 — Feed & Content Algorithm (2026-04-14)

**Task**: #4 feed-algorithm
**Output**: `../../specs/feed-algorithm.md`

### Key findings

1. **Three-generation architecture evolution**: LinkedIn's feed has gone through three major generations: (1) fragmented multi-source retrieval + DCNv2 ranker, (2) LiRank framework with enhanced DCNv2/attention/isotonic calibration, (3) current system — unified LLM-powered retrieval (fine-tuned LLaMA 3 dual encoder) + Feed Sequential Recommender (transformer-based sequential ranking). The 2026 system is a fundamental rebuild.

2. **Feed-SR (Sequential Recommender)**: Replaces pointwise DCNv2 scoring with a decoder-only transformer processing 1,000 recent impressions as an ordered sequence. Uses RoPE positional embeddings, late fusion for context features, and MMoE prediction head. Uses only ~20% of production features but outperforms the full-feature DCNv2. Key result: +2.10% time spent, +2.38% DAU. Custom SRMIS CUDA kernel achieves 80× speedup via shared context batching. Published as arxiv:2602.12354.

3. **LLM-powered retrieval**: Unified dual-encoder architecture using fine-tuned LLaMA 3 replaces multiple independent retrieval systems. Key innovation: semantic understanding (connecting "electrical engineering" to "small modular reactors" through world knowledge). Critical discovery: converting raw numerical features to percentile buckets improved correlation 30× and recall 15%. Sub-50ms k-nearest-neighbor search on GPU.

4. **Dwell time modeling**: Auto Normalized Long Dwell Model — binary classifier predicting whether time exceeds contextual percentiles, normalized by content type, creator type, and distribution method. Recalculated daily. Eliminates format bias (text vs video). Captures preferences of the 70% "ghost scrollers" who never explicitly engage.

5. **Content quality and spam**: Three-layer defense — at creation (~200ms SVM/DNN classification), during distribution (random forest/GBT virality prediction, 48% spam reduction), and reactive monitoring. 2025-2026 NLP classifiers detect engagement bait patterns. Graduated enforcement spectrum from demotion to suspension.

6. **Content format performance (2026)**: Carousels/PDFs lead at 7% engagement rate (3.5× text). Short video under 60s gets 53% more engagement. Polls need 7-day duration (70% penalty for 1-day). 54% of long-form posts estimated AI-generated, getting 45% less engagement. Creator Mode retired March 2024; features distributed to all users.

7. **Organic reach collapse**: Average post views down ~50%, engagement down ~25%, follower growth down ~59% under the new algorithm. LinkedIn frames this as "relevance-first" but creators are frustrated. 300K+ post analysis showed 50-65% organic reach decline.

8. **Competitive landscape**: X/Twitter open-sourced algorithm (shifted to Grok-based ranking 2025-2026, Premium gets 2-4× boost); Facebook/Meta uses RankNet-7 with micro-signals (cursor hover, scroll depth) and UTIS direct user surveys; TikTok is pure-recommendation (no social graph dependency, 100% algorithmic feed).

9. **Agent platform relevance**: THE key transferable concept is Feed-SR's sequential interaction modeling — an agent's task history as an ordered narrative powers capability discovery and task routing. The biggest structural advantage: agent quality is directly measurable (task success, latency, cost) unlike LinkedIn's noisy human behavioral proxies. The feed metaphor needs reimagining: not "what to read" but "what to do" or "who to work with."

### Notes for next iterations
- The LLM retrieval architecture (LLaMA 3 fine-tuning, dual encoders) is directly relevant to search-and-discovery (task #6)
- Feed-SR's multi-objective optimization framework is relevant to job-search-marketplace (task #5) — same team likely powers job ranking
- The notification/engagement loop (Concourse system) is relevant to messaging-inmail (task #7)
- Content quality classification approach applies to company-pages (task #8) — how company content is ranked in feed
- The organic reach collapse and creator frustration is relevant to premium-monetization (task #9) — it likely drives Premium conversion
- LiRank continues powering Jobs and Ads ranking surfaces — directly relevant to tasks #5 and #9

---

## Iteration 5 — Job Search & Marketplace (2026-04-14)

**Task**: #5 job-search-marketplace
**Output**: `../../specs/job-search-marketplace.md`

### Key findings

1. **Two-sided marketplace scale**: ~61M weekly job searchers, ~14,200 applications/minute (~20.4M daily), ~7 hires/minute (~3M annually). Talent Solutions is ~60% of LinkedIn's ~$17B annual revenue. Application volume surged 58% from 2024, partly driven by AI auto-apply tools (now 34% of submissions).

2. **Easy Apply paradox**: Reduces application friction to near-zero, but creates volume crisis. Recruiters spend 8.4 seconds screening each application. Callback rate: 1.2% standard vs 8.2% with strategic follow-up. Sponsored listings get 74 applications in 48 hours vs 19 for organic (3.9x). The feature optimizes for volume, not quality.

3. **JYMBII recommendation architecture**: Built on Galene (custom Lucene stack). Two parallel retrieval paths: term-based inverted index + embedding-based retrieval (two-tower neural network with IVFPQ serving via Zelda framework). Activity features (APPLY/SAVE/DISMISS over 28-day windows) evolved through 4 iterations from simple averaging to CNN sequence models, yielding >10% more applies and 5% more confirmed hires.

4. **Recruiter search architecture**: Multi-layer L1/L2 ranking. L1: distributed GBDT with pairwise learning-to-rank across Galene partitions. L2: centralized DNN refinement + GLMix entity-level personalization. Optimized for InMail Accept (positive candidate reply) not just click-through. LINE-based network embeddings enable semantic query expansion.

5. **AI Hiring Assistant (September 2025)**: LinkedIn's first agentic AI product. Plan-and-execute architecture with 7 specialized sub-agents (Intake, Sourcing, Evaluation, Outreach, Screening, Learning, Cognitive Memory). Each recruiter gets own agent instance. Operates in interactive and async ("source while you sleep") modes. Early results: 62% fewer profile reviews, 69% better InMail acceptance, 95% less manual searching.

6. **Job posting data model**: Full API schema documented at Microsoft Learn. Core required fields: title (200 chars), description (100-25K chars), location, poster email (corporate domain required since Oct 2025). Compensation schema supports range/exact with currency and period. Extension schemas for promoted jobs, Apply Connect, and RSC.

7. **Major weaknesses**: Ghosting epidemic (3-13% response rate, 70%+ seekers ghosted), ~27.4% ghost jobs on platform, job scam problem persists despite verification badges (April 2025), Easy Apply volume overwhelms recruiters, documented AI bias in matching (gender, career breaks), opaque pricing.

8. **Agent platform relevance**: Two-sided marketplace structure transfers directly. Key reimagining: agents don't "apply" — capability is verifiable (benchmarks, audit trails). The Easy Apply volume problem disappears when matching is based on objective capability verification. Real-time matching replaces "post and wait." The AI Hiring Assistant architecture (plan-and-execute with sub-agents) is actually a template for how the platform itself should work. Pricing can be transparent and outcome-based.

### Notes for next iterations
- The Galene search architecture is directly foundational for task #6 (search-and-discovery)
- Recruiter pricing and tiering connects to task #9 (premium-monetization) — Talent Solutions is 60% of revenue
- InMail as recruiter outreach tool is core context for task #7 (messaging-inmail)
- Job posting association with Company Pages is relevant to task #8 (company-pages)
- The AI Hiring Assistant's sub-agent architecture may be worth referencing in the agent platform design phase
- Ghost jobs and scam problems are relevant to trust/verification themes across all specs

---

## Iteration 6 — Search & Discovery (2026-04-14)

**Task**: #6 search-and-discovery
**Output**: `../../specs/search-and-discovery.md`

### Key findings

1. **Galene architecture**: LinkedIn's search-as-a-service platform built on Apache Lucene. Three-tier serving (Federator → Broker → Searcher) with three-segment indexing (Base Index weekly via Hadoop, Live Update Buffer in-memory, Snapshot Index on-disk every few hours). Field-granularity live updates replaced expensive entity-level operations. Static rank ordering enables early termination — skip scoring low-importance candidates.

2. **Scale**: ~700M searches per day. Typeahead consolidation reduced QPS from 16K to 8K with 3% P90 latency improvement. Dreamweaver system handles intent prediction for vertical selection (e.g., "software engineer" → Jobs primary).

3. **Semantic search evolution (Feb 2026 paper, arxiv:2602.07309)**: LLM-powered framework with three components: (a) SAGE — 8B parameter LLM relevance judge generating graded scores, tens of millions of evaluations/day; (b) LLM bi-encoder retrieval over 1.3B documents, top-1,000 candidates; (c) 0.6B SLM ranker via multi-teacher distillation, reranks top-250. OSSCAR pruning (600M→375M params) + MixLM achieves 75× throughput speedup (22K items/sec/GPU). Results: +7.73% NDCG@10 for Job Search, >10% for People Search, +1.2% DAU.

4. **Post embeddings (arxiv:2405.11344)**: 6-layer multilingual BERT, 89M params, producing 50-dimension embeddings — matching OpenAI ADA-002 (1536 dims) at 30× compression on LinkedIn benchmarks. Multi-task training on 104M pairs across Interest/Storyline/Hashtag/Search tasks. Nearline pipeline computes embeddings within 2 minutes of post creation. Impact: +10.46% video watch time, +0.42% revenue.

5. **Post search rebuilt 2022**: Three-stage pipeline — First Pass Ranker (GBDT, recall-optimized) → Second Pass Ranker (neural nets, precision-optimized) → Diversity Re-ranker. Multi-aspect modeling with independent ML models per aspect. Results: +6.2% CTR, +21% messages from search.

6. **AI People Search (Nov 2025)**: Premium-only (US), natural language queries like "someone who has scaled a startup." Replaces keyword matching with intent-driven discovery. Current limitations: badge confusion, phrasing sensitivity, US-only.

7. **Commercial use limits**: Free accounts capped at ~300 searches/month. Results capped at 1,000. Sales Navigator: unlimited + 2,500 results. Recruiter: full network access + 40+ filters. Search restriction is LinkedIn's clearest free-to-paid conversion lever.

8. **Agent platform relevance**: Federated multi-vertical search transfers directly. Static rank maps to pre-computed agent quality scores. Two-tower retrieval architecture ideal for semantic agent discovery. KEY differences: agent capabilities are verifiable (not self-reported), search should return live-available agents (not static profiles), and composability search (find agent pipelines) has no LinkedIn analog. Boolean search becomes unnecessary with properly structured capability data.

### Notes for next iterations
- The semantic search paper's SLM distillation approach is relevant context for premium-monetization (task #9) — this is what powers Premium AI search
- Galene's federation architecture is context for understanding company-pages (task #8) — company search is a vertical within federation
- The commercial use limit monetization model is directly relevant to premium-monetization (task #9)
- Post search pipeline improvements are relevant to projects-portfolio (task #10) — how project/portfolio content would be discovered
- The AI People Search feature is relevant to x-professional-features (task #12) — comparing discovery mechanisms

---

## Iteration 7 — Messaging & InMail (2026-04-14)

**Task**: #7 messaging-inmail
**Output**: `../../specs/messaging-inmail.md`

### Key findings

1. **Architecture evolution**: Three phases — email-like monolith with Oracle DB (2013) → sharded monolith with PDR (2016) → full microservices rebuild (2020). The 2020 rebuild took ~6 months with ~24 engineers, produced less than a dozen services, and normalized the data model from per-participant message copies to a single centralized copy. 60 separate converters handled legacy business logic. Zero-downtime migration of 17 years of messages via dual-write + Hadoop MapReduce + shadow verification.

2. **Plugin-based extensibility**: Core messaging is pure storage/delivery. Business logic lives in plugins with lifecycle callbacks (conversationPreCreate, messagePostDeliver, etc.). Platform stores but never inspects plugin metadata. Plugin failures don't cascade. This pattern is directly applicable to agent communication platforms.

3. **Real-time infrastructure**: Play Framework + Akka Actor Model with SSE (not WebSockets). One Actor per connection, hundreds of thousands of connections per machine. Presence platform: one Actor per online member, heartbeat-based detection with jitter guard (d + ε expiry), <200ms p99 end-to-end presence updates. Couchbase for subscription routing.

4. **InMail credit economy**: 5-150 credits/month depending on tier ($29.99-$835+/mo). Credit refund on response within 90 days — brilliant incentive alignment. Open Profile loophole allows free messaging to willing recipients (800/month cap). Additional credits ~$10 each. Response rates: 10-25% average, but declining (SaaS down to 4.77%).

5. **Air Traffic Controller (ATC)**: Samza-based notification system processing 1B+ requests/day. "5 Rights" framework (right message/member/channel/time/frequency). ML predicts click and disable rates per notification per member. RocksDB local state for millisecond lookups. Reduced complaints 50%, push latency from 12s to 1.5s.

6. **Sponsored messaging**: Message Ads ($0.26-0.50/send, 30-50% open rate) and Conversation Ads (branching logic, multiple CTAs). Frequency cap of ~1/member/30 days. Recipients can't reply to Message Ads — a fundamentally broken UX that erodes inbox trust.

7. **AI-assisted recruiting messages**: 69% higher response rates, 44% increase in accept rates vs. templates. Automated follow-ups yield 39% more accepts. "Conversation starters" (2026) suggest topics from candidate's recent activity.

8. **Major weaknesses**: No E2EE (LinkedIn can read all messages), spam epidemic from automation tools, no threading, primitive inbox management (no labels/folders/snooze), video calling outsourced to Teams/Zoom/BlueJeans, declining InMail response rates.

9. **Agent platform relevance**: Connection-degree access hierarchy maps to agent trust tiers. InMail credit refund model translates to collaboration credits (refunded on successful task completion). Key reimagining: agent communication is task-oriented (structured schemas, not free text), real-time performance data replaces social signals, and the spam problem disappears when matching is capability-verified.

### Notes for next iterations
- InMail credit tiers and pricing are core context for premium-monetization (task #9)
- Sponsored Messaging ad formats are relevant to premium-monetization (task #9)
- The Messenger SDK's cross-product unification pattern is relevant to company-pages (task #8) — messaging is embedded in recruiter/company workflows
- LinkedIn's lack of native video calling is a competitive gap relevant to x-professional-features (task #12) — X.com's Spaces fill this niche
- The automation tool ecosystem (Expandi, Waalaxy, etc.) is relevant to job-platforms-comparison (task #11) — these tools span multiple platforms
- The ATC notification system's "5 Rights" framework could be a foundational concept for agent platform notification design

---

## Iteration 8 — Company Pages & Employer Branding (2026-04-14)

**Task**: #8 company-pages
**Output**: `../../specs/company-pages.md`

### Key findings

1. **Page taxonomy**: Five page types — Company Page (free, main entity), Showcase Pages (free, up to 25, for sub-brands), Product Pages (free, up to 35, with Lead Gen Forms), Service Pages (free, up to 10), Career Pages (paid, $10K-70K/year via Talent Solutions). Showcase Pages have independent follower bases and limited features vs. main pages.

2. **Organization entity data model**: Full Rest.li schema documented via API. Public fields: id, name, vanityName, logoV2, locations, primaryOrganizationType (SCHOOL/BRAND/NONE). Admin-only fields: description, industries, specialties, staffCountRange, organizationType, organizationStatus, foundedOn, parentRelationship, pinnedPost. URN format: `urn:li:organization:{id}`. Showcase Pages use same URN format with `primaryOrganizationType: BRAND` and `parentRelationship` linking to parent.

3. **Organic reach collapse**: Company Page posts now reach only 1.6-5% of followers. 60-80% organic reach decline from 2024-2026. Company content accounts for just 1-2% of feed. Personal profiles generate 561% more reach with identical content. The 360Brew algorithm (150B-param model) explicitly prioritizes personal connections over brand content. Company pages get ~5% of feed allocation vs ~65% for personal profiles.

4. **Invitation credit gutting (March 2026)**: Monthly credits reduced from 250 to 50 per page — 80% cut. Credits refunded on acceptance. Premium Company Pages ($77-99/month) offer auto-invite for engaged users and followers of similar pages, partially mitigating the cut. Clear pay-to-play monetization move.

5. **Employee advocacy deprecated then needed more than ever**: My Company tab, Employee Advocacy analytics, and Curator admin role all removed November 2024. Yet employee advocacy is now the primary viable organic strategy: employee shares generate ~30% of total company engagement despite only 3% of employees sharing. Employee-sourced leads convert 7x more than paid channels.

6. **Verification trust chain**: Page verification → domain verification → employee workplace verification. Verified Pages get 2.4x more engagement. Companies with complete/verified pages receive 30% more weekly views. Domain verification enables employee email-based workplace verification.

7. **Premium Company Page**: $77-99/month. Features: custom CTA button (from picklist, not truly custom), visitor analytics (one visitor/day), auto-invite, competitor comparison (up to 9), AI content assistant, testimonials, dynamic banners, premium badge. ~80% subscriber growth QoQ — one of LinkedIn's fastest-growing products.

8. **Competitive landscape**: Glassdoor gets 3-20x more company profile traffic than LinkedIn due to anonymous reviews/salary data. X.com Verified Organizations ($200-1K/month) offer affiliated accounts with visual organizational badges — a verifiable employer association that LinkedIn lacks. Facebook dominates B2C with 3B users and Groups (2B active users) vs. LinkedIn's B2B focus.

9. **Agent platform relevance**: Company Page concept transforms from marketing surface to operational control plane. Key advantages: verified agent rosters (vs. self-reported employee counts), real-time observability dashboards (vs. retrospective analytics), structured changelogs (vs. social media posts), meritocratic discovery (vs. algorithm gaming). Employee count unreliability is LinkedIn's weakness; agent platforms can provide cryptographically verifiable organizational associations.

### Notes for next iterations
- Premium Company Page pricing and fastest-growing-product status is directly relevant to premium-monetization (task #9)
- The organic reach collapse and pay-to-play dynamics are relevant context for premium-monetization (task #9) — they drive Premium Company Page adoption
- Product Pages' Lead Gen Forms are relevant to projects-portfolio (task #10) — showcasing work with conversion mechanisms
- Glassdoor's anonymous review advantage is relevant to job-platforms-comparison (task #11)
- X.com Verified Organizations' affiliated accounts feature is relevant to x-professional-features (task #12)
- The employee count unreliability problem is a cross-cutting theme — it affects all specs dealing with company data

---

## Iteration 9 — Premium & Monetization (2026-04-14)

**Task**: #9 premium-monetization
**Output**: `../../specs/premium-monetization.md`

### Key findings

1. **Revenue structure**: LinkedIn generates ~$17.8B annually (FY2025) across four streams: Talent Solutions (~$7.8B, 44%), Marketing Solutions (~$6.2B, 35%), Premium Subscriptions (~$3.9B, 22%), and Sales Solutions (~$2.1B). Premium subscriptions are the fastest-growing segment at 23% YoY, crossing $2B annually in January 2025. 120M+ Premium subscribers represent 9.2% of the user base.

2. **Nine paid tiers**: Premium Career ($29.99/mo), Premium Business ($59.99/mo), Premium All-in-One ($89.99/mo — new 2026 plan bundling sales+marketing+hiring with $100/mo ad credits and $50/mo job credits), Sales Navigator Core ($119.99/mo), Sales Navigator Advanced ($159.99/mo), Sales Navigator Advanced Plus ($1,600/seat/year), Recruiter Lite ($169.99/mo), Recruiter Corporate ($900+/mo), and LinkedIn Learning ($39.99/mo standalone).

3. **Intent-based paywall timing**: LinkedIn's most powerful monetization insight — gate features at the exact moment of highest intent. Job seekers hit the wall when comparing themselves to applicants. Salespeople hit it when they've found a prospect but can't message them. The friction message "You've reached your limit" appears when conversion motivation is maximum.

4. **InMail credit-back model**: Credit refunded if no response within 90 days. Aligns incentives (senders write quality messages), creates measurable ROI, and generates engagement data for ML optimization. Declining response rates (SaaS now at 4.77%) threaten this model.

5. **Advertising platform**: Expensive but effective — $5-16 CPC (vs $1-3 Facebook), $30-60 CPM. But 75-85% of B2B social media leads come from LinkedIn, cost per lead 28% lower than Google Ads, 2x conversion rates. Thought Leader Ads are the standout format at $3.06 CPC vs $13.23 for single image ads (77% cost reduction). BrandLink (creator pre-roll ads) showing 130% higher video completion rates.

6. **Organic reach collapse as monetization driver**: The documented 50-65% decline in organic reach directly feeds Marketing Solutions revenue. Company Page posts reach only 1.6-5% of followers, pushing companies toward Sponsored Content and Premium Company Page ($77-99/mo, growing 80% QoQ).

7. **Competitive gaps**: Price opacity for enterprise products, no creator monetization at scale (BrandLink is invite-only vs X's democratic revenue sharing), LinkedIn Learning facing free alternatives, AI features increasingly available from third-party tools, Message Ads are one-directional (can't reply).

8. **Agent platform relevance**: Key transferable concepts — use-case-based tiering, intent-based paywall timing, credit-back incentive models. Key reimagining needed — transparent usage-based pricing (not opaque enterprise contracts), outcome-based monetization (charge for successful tasks, not access), creator economics from day one (revenue share for agent capability providers), and no artificial scarcity (charge for genuinely expensive capabilities, not degraded free experiences).

### Notes for next iterations
- The Premium All-in-One plan's bundled ad credits model is relevant to projects-portfolio (task #10) — how project visibility could be monetized
- BrandLink creator monetization is relevant to x-professional-features (task #12) — comparing creator economics across platforms
- Sales Navigator's competitive landscape (Apollo, Lusha, ZoomInfo) is relevant to job-platforms-comparison (task #11)
- The 312% ROI figure for Sales Navigator (Forrester study) is worth cross-referencing in future specs about marketplace economics
- LinkedIn's advertising CPM premium ($30-60 vs $7-15 Facebook) demonstrates the value of professional data — directly relevant to agent platform pricing strategy

---

## Iteration 10 — Projects & Portfolio Showcase (2026-04-14)

**Task**: #10 projects-portfolio
**Output**: `../../specs/projects-portfolio.md`

### Key findings

1. **Featured Section (launched Feb 2020)**: Horizontal card carousel supporting posts, articles, newsletters, links, images, and documents (up to 400 items, 100MB per file). Desktop shows only 2 items at once. Critical limitation: content is NOT indexed by LinkedIn search and NOT visible to non-logged-in visitors. No analytics on individual items. No custom thumbnails for links. Videos require publishing as posts first.

2. **Projects Section structural constraint**: Projects must be tied to an Experience or Education entry — no standalone projects allowed. The URL field was silently removed from the UI in mid-2023 (API still accepts it). The only workaround requires GraphQL interception via browser dev tools. Media attachments on projects are through a separate flow, not native to the Projects API schema.

3. **No public API for Featured section**: There is no documented public endpoint to read or write to the Featured section. The Profile Edit API requires `w_compliance` private partner permission. The `memberRichContents` field may reference Featured content but documentation is sparse.

4. **Collaborative Articles fully retired (Dec 2024)**: The only mechanism for earning expertise signals through content contribution was killed after widespread gaming. Community Top Voice gold badges expired. No replacement introduced.

5. **Competitive landscape reveals LinkedIn's fundamental gap**: No single platform combines discovery + portfolio depth + transactions + employment networking. Behance (visual case studies, Google SEO, Adobe integration), GitHub (83% of hiring managers trust over resumes, shows actual code), Contra (0% commission portfolio-to-pay pipeline), Substack (creator-owned audience), and Peerlist (GitHub-integrated developer profiles) each do a specific thing LinkedIn cannot. Read.cv was acquired by Perplexity AI (Jan 2025) — LinkedIn alternative space being explored as AI infrastructure.

6. **Cross-platform behavior**: Professionals maintain 3-5 platforms. LinkedIn serves as "employment credibility" layer but is rarely the primary portfolio for any specialized profession. Designers hub on Webflow/Behance, developers on GitHub, freelancers on Contra/Upwork, thought leaders on Substack.

7. **Agent platform relevance**: THE key insight — agent portfolios should be auto-generated from actual task execution (success rates, latency, cost, output samples), not manually curated like LinkedIn's Featured section. The Featured section equivalent fills itself. Portfolio = real-time monitoring dashboard, not static showcase. Discovery through demonstrated capability (not keywords) collapses the profile/search separation. Composition portfolios (pre-validated agent pipelines with performance data) have no LinkedIn analog.

### Notes for next iterations
- The cross-platform fragmentation analysis is relevant to job-platforms-comparison (task #11) — how job seekers use multiple platforms
- Behance's 2026 ATS layer roadmap and Contra for Companies are relevant to job-platforms-comparison (task #11)
- BrandLink creator monetization and Substack's creator economics are relevant to x-professional-features (task #12)
- GitHub Copilot's impact on portfolio perception (shifting from code quantity to architecture/review quality) is relevant context for agent capability evaluation
- The Read.cv acquisition by Perplexity AI signals that professional identity platforms are being absorbed into AI infrastructure — directly relevant to our agent platform thesis

---

## Iteration 11 — X.com Professional Features (2026-04-14)

**Task**: #12 x-professional-features
**Output**: `../../specs/x-professional-features.md`

### Key findings

1. **Public signal-based recruiting**: X hiring happens through observation (recruiters monitor tweet history) and DM outreach, not formal job applications. Hashtags like #Hiring, #BuildInPublic, #TechJobs function as informal job boards. No ATS, no structured applications — X is a sourcing and outreach layer.

2. **Build-in-public flywheel**: The #BuildInPublic culture creates a self-reinforcing cycle — share work publicly → build audience → hirers observe quality → inbound offers → more build → more audience. This continuous credential mechanism has no LinkedIn equivalent.

3. **Asymmetric following**: Unlike LinkedIn's connection-degree system, X follows are largely asymmetric. Any recruiter can DM any candidate without prior connection. Removes structural friction that makes LinkedIn outreach feel transactional.

4. **Verified Organizations**: $1,000/month domain-based verification for organizations. Distinct from individual Blue check ($8-12/month). Domain ownership authentication prevents impersonation. Includes 3 linked secondary accounts.

5. **No formal infrastructure**: X has no job posting system, no application tracking, no structured profiles, no compensation transparency. Hiring is entirely informal — works for senior tech roles, not scalable for high-volume recruiting.

6. **Grok-based algorithm (2025-2026)**: X open-sourced its algorithm in Dec 2023 (arxiv:2312.13217). Has since shifted to Grok-based ranking. Premium users get 2-4× distribution boost. Algorithm heavily weights engagement over stated interest relevance.

7. **Key agent platform insight**: The inbound discovery model — tasks flowing to capable agents based on observed performance, not agents applying to posted tasks — is the structural advantage. Both sides have objective, verifiable, real-time data that humans on X and LinkedIn don't have.

### Notes for next iterations
- Both tasks in tasks.json are now done
- The x-professional-features.md spec file is saved at /home/ren/repos/agentin/ralph/specs/x-professional-features.md
- All 12 research specs are now complete