# Search & Discovery

## What it is

LinkedIn's search and discovery system is the unified platform that enables 700+ million daily searches across people, jobs, content, companies, groups, events, and more. Built on Galene — LinkedIn's custom search-as-a-service infrastructure layered atop Apache Lucene — the system has evolved from keyword-based inverted index retrieval into a hybrid architecture combining traditional term matching with LLM-powered semantic retrieval. In late 2025, LinkedIn launched AI-powered natural language people search for Premium users, signaling a fundamental shift from structured filter-based discovery to intent-driven matching. Search is both a core user utility and a critical monetization lever: free users hit commercial use limits at ~300 searches/month, driving conversion to Premium, Sales Navigator ($119.99/mo), and Recruiter ($170+/mo) tiers that unlock unlimited searches, expanded result sets, and 40+ advanced filters.

## How it works — User perspective

### Search entry point and typeahead

The search bar sits at the top of every LinkedIn page. As users type, a typeahead system (originally powered by Cleo, LinkedIn's open-source typeahead library) delivers instant suggestions across multiple entity types: people, companies, jobs, groups, skills, and site features. The typeahead considers likelihood of a successful search combined with popularity — so typing "j" surfaces "Java" before "Jobs" if your profile signals engineering interest.

### Search verticals

Results are organized into tabs/verticals:

- **People**: The primary search vertical. Free filters include: connection degree (1st/2nd/3rd+), location, current company, past company, industry, school, title, profile language, and service categories. Results are capped at 1,000 (100 pages × 10 results) for free users.
- **Jobs**: Keywords, date posted, experience level, company, job type, remote/on-site, industry, function, Easy Apply toggle, "Under 10 Applicants," and "In Your Network" filters. Job search is the least restricted vertical — LinkedIn wants high job search volume.
- **Posts/Content**: Date posted, content type (video/image/document), author company, author industry, author keywords, mentions. Post search was significantly rebuilt in 2022 with multi-stage ranking.
- **Companies**: Keywords, location, industry, company size, whether they have active job listings, and whether you have connections there.
- **Groups, Events, Schools, Products, Services, Courses**: Each has vertical-specific filters (e.g., courses have provider, level, time-to-complete).

### Vertical blending

When a user enters a query without selecting a specific tab, LinkedIn's federation layer performs intent prediction (via a system called Dreamweaver) to determine which verticals to fan out to and how to blend results. For "software engineer," the system predicts the user likely wants jobs and surfaces job results prominently alongside people results.

### Boolean search

Available across all verticals, Boolean operators enable power users (especially recruiters) to construct precise queries:
- **AND**: Results must contain all terms ("accountant AND CPA")
- **OR**: Results may contain any term ("developer OR programmer OR coder")
- **NOT**: Exclude terms ("VP NOT (assistant OR SVP)")
- **Quoted phrases**: Exact match ("product manager")
- **Parenthetical grouping**: Complex logic nesting

Boolean strings are capped at ~2,000 characters. This capability is disproportionately used by recruiters — the majority of regular users never use Boolean operators.

### AI-powered natural language search (November 2025)

LinkedIn launched an AI-powered people search for US Premium subscribers that replaces the standard "Search" prompt with "I'm looking for..." Users can type natural language queries like:
- "someone who has scaled a startup"
- "an expert in digital marketing"
- "people who co-founded a YC startup"

The system interprets intent rather than matching keywords literally, leveraging LinkedIn's 900M+ profiles and AI models trained on the full professional knowledge graph. Current limitations: the system sometimes conflates "LinkedIn Top Voice" badges with voice AI expertise, and results vary based on exact phrasing.

### Commercial use limits

Free accounts hit a Commercial Use Limit at approximately 250–350 people searches per month, resetting on the 1st of each month at midnight PT. This is LinkedIn's primary search monetization lever. Sales Navigator removes this limit entirely and expands results to 2,500 per search (100 pages × 25 results). Recruiter products offer full network visibility beyond 3rd-degree connections.

### Search tiers by product

| Feature | Free LinkedIn | Premium Career | Sales Navigator | Recruiter Lite | Recruiter |
|---------|--------------|----------------|-----------------|----------------|-----------|
| People search results | 1,000 | 1,000 | 2,500 | Full network | Full network |
| Monthly search limit | ~300 | Higher | Unlimited | Unlimited | Unlimited |
| Advanced filters | ~12 | ~12 | 30+ | 40+ | 40+ |
| Boolean search | Basic | Basic | Advanced | Advanced | Advanced |
| AI people search | No | Yes (US) | TBD | TBD | Yes |
| InMail from search | No | 5/mo | 50/mo | 30/mo | 100-150/mo |

## How it works — Technical perspective

### Galene: The search-as-a-service platform

LinkedIn's entire search stack runs on Galene, a unified search platform that replaced a fragmented collection of systems (Sensei, Zoie, Bobo, Cleo, Krati, Norbert) built on top of Lucene. Galene retains Lucene only for indexing primitives while moving all other functionality outside it.

**Three-tier serving architecture:**

1. **Federator**: Receives raw queries, invokes rewriter plugins (synonym expansion, spelling correction, graph-proximity personalization), distributes to search verticals, and blends results from multiple verticals using ML-based relevance algorithms. Includes Dreamweaver for intent prediction and vertical selection.

2. **Broker**: Operates within specific verticals (members, companies, jobs). Performs vertical-specific query rewriting before distributing to searchers, then merges shard-level responses.

3. **Searcher**: Operates on individual index shards. Retrieves matching entities, applies scoring via pluggable scorers using query details, entity metadata, and forward index data, then returns top results.

**Three-segment indexing architecture:**

- **Base Index**: Built offline via Hadoop MapReduce weekly. Never modified in-place; discarded when the next build completes.
- **Live Update Buffer**: In-memory segment accepting incremental field-level updates (not whole-entity updates — a key optimization). Maintains static rank ordering.
- **Snapshot Index**: Persistent on-disk segment created every few hours by merging the buffer with previous snapshots.

Live updates occur at field granularity rather than requiring expensive entity-level deletions and additions. The term-partitioned segment design means posting lists span multiple segments, traversed as a disjunction.

**Static rank and early termination**: Entities are pre-ordered by a query-independent importance score (static rank) computed offline. Because static rank correlates with final relevance scores, retrieval can terminate early once sufficient high-quality candidates are found, enabling more sophisticated scorers without latency penalties.

**Index distribution**: A BitTorrent-based framework distributes index segments across replica groups. Machines joining a group automatically receive associated data, with built-in versioning and rollback.

**Scale**: ~700 million searches per day across all verticals. Typeahead consolidation reduced QPS from 16K to 8K while improving P90 latency by 3%.

### Post search ranking pipeline

Rebuilt in 2022, post search uses a multi-stage architecture:

1. **First Pass Ranker (FPR)**: Scans large document volumes optimizing for recall. Uses lightweight gradient boosted decision trees. Models each ranking aspect independently (relevance, quality, personalization, engagement, freshness).

2. **Second Pass Ranker (SPR)**: Operates in the federation layer on top-k candidates from FPR. Supports complex neural network architectures. Focuses on precision over recall.

3. **Diversity Re-ranker**: Final layer that injects diverse content, surfaces potentially viral content for trending queries, and reduces duplication.

Results: +6.2% CTR, +5.4% engagement, +21% messages from post search, ~30-62ms latency improvement across platforms.

### Semantic search: Embedding-based retrieval (EBR)

LinkedIn's content search engine uses a two-tower model for embedding-based retrieval alongside the traditional token-based retriever:

- **Query tower**: Processes query text through multilingual-e5 embeddings, concatenates additional features via MLP, produces dense vectors at inference time.
- **Post tower**: Same architecture but pre-computed offline. Embeddings stored in Venice key-value database for low-latency lookup.
- **Retrieval**: Cosine similarity between query and post embeddings. Approximate nearest neighbor search with latency budgets.
- **Training data**: Historical search data labeled with on-topicness (does the post answer the query?) and long-dwell (engagement duration). Final score: α(on_topicness) + (1-α)(long_dwell).

Result: 10%+ improvement in on-topic rate and long-dwell metrics.

### LinkedIn Post Embeddings (LPE)

A foundational embedding model powering search, feed ranking, feed retrieval, and video recommendations (arxiv:2405.11344, CIKM 2025):

- **Base model**: 6-layer multilingual BERT pre-trained on LinkedIn data via masked language modeling. 89M parameters, 135K vocabulary.
- **Embedding dimension**: 50 — empirically optimal for latency/expressiveness tradeoff. Achieves 30× compression vs OpenAI ADA-002 (1536 dims) while matching performance on LinkedIn-specific benchmarks.
- **Multi-task training**: Siamese network trained simultaneously on 4 tasks (Interest/topic similarity, Storyline/editorial groupings, Hashtag co-occurrence, Search relevance) across 104M training pairs. Binary cross-entropy loss on cosine similarity.
- **Serving**: Samza nearline pipeline computes embeddings within 2 minutes of post creation, pushed to key-value store. Member embeddings derived via hierarchical clustering (Ward's method), updated daily.
- **Impact**: +0.42% revenue in feed ranking, +10.46% video watch time, +1.74% video DAU.

### Semantic Search at LinkedIn (arxiv:2602.07309, February 2026)

The most recent evolution — an LLM-powered semantic search framework powering AI Job Search and AI People Search:

**Architecture:**
1. **LLM Relevance Judge (SAGE)**: Proprietary 8B parameter model serving as the "policy-aligned judge." Generates graded relevance scores (0-4). Achieves linear kappa 0.77 alignment with human judgment. Supports tens of millions of evaluations per day.

2. **Retrieval Layer**: LLM-based bi-encoder with contrastive training. Retrieves top-1,000 candidates from corpora up to 1.3B documents using GPU retrieval-as-ranking (RAR) with personalization features. Uses InfoNCE loss combined with pairwise margin loss.

3. **SLM Ranker**: Decoder-only 0.6B parameter model trained via multi-teacher distillation (MTD). Jointly predicts relevance (binary) and five engagement tasks. Reranks top-250 results in production.

**Critical optimizations:**
- **OSSCAR pruning**: Removes 50% of MLP neurons and 8 transformer layers, reducing model from 600M to 375M parameters while matching baseline quality.
- **Context summarization**: RL-trained 1.7B model reduces document length by an order of magnitude.
- **MixLM**: Native text-embedding hybrid interaction enabling 22,000 items/sec/GPU throughput — a cumulative 75× speedup.
- **Distributed Couchbase cache**: Serves >50% of scoring requests from cache.

**Results:**
- Job Search: +7.73% NDCG@10, -46.88% poor match rate
- People Search: >10% NDCG@10
- Product impact: >+1.2% DAU lift
- Open-sourced scoring stack as part of SGLang

### Query understanding pipeline

The federation mid-tier runs modular rewriter plugins sequentially:

1. **Spelling correction**: Learned from search query logs, corrects typos before retrieval.
2. **Synonym expansion**: Data models (synonym maps, n-grams) built offline alongside search indices.
3. **Graph proximity personalization**: Incorporates connection graph signals to personalize result ordering.
4. **Query completion**: Powers typeahead with likelihood-of-successful-search × popularity scoring.
5. **Vertical intent classification**: Dreamweaver classifies queries into appropriate verticals (e.g., "software engineer" → Jobs primary, People secondary).

Data models are built offline with the search index and distributed to serving infrastructure via the BitTorrent framework.

### External search (Google SEO)

LinkedIn profiles are indexed by Google, creating a dual-optimization challenge:
- LinkedIn allows Google to crawl all public profile pages and the /pub/dir/ name directory.
- Keywords in headline (highest weight), About section (first ~40 words indexed by Google), job titles, and skills affect both LinkedIn internal and Google external ranking.
- LinkedIn articles are indexed by Google; regular posts are not.
- External links on LinkedIn carry "nofollow" — no SEO backlink value.
- Profile changes are indexed within 24-48 hours; ranking improvements take 2-4 weeks.

## What makes it successful

### 1. Unified search-as-a-service architecture
Galene powers every search product from the same platform — people, jobs, content, companies, typeahead, and internal tools. This unification enables shared infrastructure investment (query understanding, spelling correction, embedding models) to improve all verticals simultaneously.

### 2. Static rank enables performance without sacrificing quality
The insight that a query-independent importance score (computed from profile completeness, connection count, activity level, and engagement history) correlates strongly with final relevance allows early termination. This means LinkedIn can use more sophisticated rankers within the same latency budget — they do less work on low-quality candidates and more work on high-quality ones.

### 3. Progressive restriction as monetization
LinkedIn's search tier system is elegant: free users get enough search to experience value (and contribute data), but hit artificial limits at ~300 searches/month. Each paid tier unlocks progressively more: Premium gets AI search, Sales Navigator gets 2,500 results + 30 filters, Recruiter gets full network access + 40+ filters. Search is the single clearest conversion path from free to paid.

### 4. Hybrid retrieval (term + semantic)
Running token-based and embedding-based retrievers in parallel ensures that exact-match queries (company names, specific skills) work perfectly while semantic queries ("someone who's scaled a startup") also surface relevant results. The two approaches compensate for each other's weaknesses.

### 5. Multi-task post embeddings at 50 dimensions
Achieving OpenAI ADA-002 performance at 1/30th the embedding size is a remarkable engineering feat. The low dimensionality enables real-time retrieval at scale with manageable storage and compute costs. Multi-task training on interest/storyline/hashtag/search creates embeddings that generalize across applications.

### 6. Connection graph as a ranking signal
Every search result is personalized by the searcher's network position. Having a 1st-degree connection at a company, a shared alma mater, or 2nd-degree connections in common all boost relevance. This creates a virtuous cycle: the more connected you are, the better your search results, which makes you connect more.

### 7. Dual SEO surface
LinkedIn profiles rank on both LinkedIn's internal search and Google's external search, giving members two discovery channels from a single profile investment. This is unique among professional platforms and significantly extends LinkedIn's reach.

## Weaknesses and gaps

### 1. Commercial use limits are punitive and opaque
The ~300 search/month limit is invisible until you hit it, frustrating users who don't understand why search suddenly stops working. The boundary between "personal" and "commercial" use is undefined — LinkedIn decides unilaterally. This creates adversarial dynamics where users try workarounds (incognito browsing, Sales Navigator trials) rather than genuine engagement.

### 2. Filter-based search is fundamentally limited
Despite 12-40+ filters depending on tier, LinkedIn's faceted search requires users to know exactly what they want. The filters map to self-reported structured data (job titles, company names, locations) that is inconsistent, outdated, and gamed. You can't effectively search for "someone who has actually built a product from zero to one" — only "someone whose title contains 'founder' at a company they listed."

### 3. AI people search is still primitive
The November 2025 launch showed significant limitations: conflating LinkedIn badges with actual expertise, different results for semantically equivalent queries, and US Premium-only availability. Natural language search over self-reported profiles inherits all the noise and bias of the underlying data.

### 4. Search quality degrades with scale
As LinkedIn grows past 1B members, the signal-to-noise ratio in search results worsens. Fake profiles, abandoned profiles, misrepresented credentials, and SEO-gamed profiles all pollute results. LinkedIn's static rank partially addresses this by downranking inactive profiles, but the fundamental problem of unverified data persists.

### 5. No search transparency
Users have no visibility into why they rank where they do in search results. There's no equivalent of Google Search Console for LinkedIn — you can see "who viewed your profile" but not "what searches your profile appeared in." This makes optimization guesswork.

### 6. Content search lags behind people/job search
Post search was only significantly improved in 2022 and still lacks the sophistication of people and job search. There's no semantic post search for free users, limited content type filtering, and no ability to search within comments or nested threads.

### 7. Recruiter search creates information asymmetry
Recruiters using Recruiter Seat can see every member's full profile regardless of privacy settings, search the full 1B+ network, and access 40+ filters. Members have no visibility into recruiter searches and can't control their discoverability beyond coarse privacy toggles. This asymmetry is a feature for LinkedIn's revenue but a privacy concern for members.

## Competitive landscape

### Indeed
Indeed is a job metasearch engine indexing 45-50% of all online job postings worldwide (vs LinkedIn's ~15.7M active listings). Its search is keyword-and-filter only — no social graph, no connection-based personalization, no network effects. Indeed attracts 350M monthly visitors to LinkedIn's ~310M. Indeed's strength is volume and simplicity; its weakness is zero candidate differentiation beyond resume keywords. Google Jobs reports 11.29% response rate vs LinkedIn's 3.10%, suggesting Indeed/Google's simpler matching may outperform LinkedIn's over-optimized system for active job seekers.

### Google for Jobs
Google aggregates job listings from across the web (including LinkedIn) into Google Search results. Uses structured data (JobPosting schema) for parsing. Advantage: discovery at the point of search intent, no separate app needed. Disadvantage: no profiles, no networking, no candidate assessment — purely a listing aggregator.

### X/Twitter
No structured search for professional discovery. Relies on organic content surfacing (build-in-public, open-source contributions). Search is full-text across tweets with basic filters (date, media type, from:user). What X has that LinkedIn lacks: search surfaces actual work artifacts (code, writing, projects) rather than self-reported claims. Grok integration (2025-2026) adds AI-powered search but remains content-oriented, not profile-oriented.

### GitHub
GitHub's search indexes actual code, contributions, and project participation — the closest analog to "verifiable capability search." GitHub profiles show a contribution graph, repositories, pull requests, and stars. Recruiters increasingly use GitHub as a signal of engineering capability. Weakness: limited to developers, no explicit job marketplace, no structured profile beyond what code reveals.

### Wellfound (AngelList)
Focuses on startup hiring with search filters optimized for startup context: funding stage, equity range, team size, tech stack. More transparent than LinkedIn about compensation ranges. Smaller but higher-signal candidate pool for startup roles.

### Sales Navigator vs. dedicated sales tools
ZoomInfo, Apollo, Lusha, and Clearbit compete with Sales Navigator on B2B people search. These tools often scrape or aggregate LinkedIn data alongside other sources (email, phone, company data). They typically offer better data enrichment and integration with CRMs but depend on LinkedIn as a data source.

## Relevance to agent platforms

### What transfers directly

1. **Multi-vertical federated search**: An agent platform needs to search across agent capabilities, task types, pricing, performance history, and availability — directly analogous to LinkedIn's people/jobs/content/company verticals unified under one search system.

2. **Static rank for early termination**: Agents can have pre-computed quality scores (based on benchmark performance, task success rates, uptime) that enable the same early-termination optimization — skip low-quality agents early, spend compute on ranking the best ones.

3. **Embedding-based retrieval**: LinkedIn's two-tower model approach (pre-compute document/agent embeddings offline, compute query embeddings at request time) is directly applicable for semantic agent discovery ("find me an agent that can summarize legal documents and extract key clauses").

4. **Tiered access**: The progressive restriction model (free tier with limits → paid tiers with expanded access) translates to agent marketplace monetization.

### What needs reimagining

1. **Capability search replaces profile search**: Agents don't have "headlines" and "about sections" — they have capability manifests, benchmark scores, API schemas, and execution histories. Search should match task requirements to verified capabilities, not keywords to self-descriptions.

2. **Verifiable ranking signals**: LinkedIn's biggest search weakness (unverified self-reported data) is an agent platform's biggest opportunity. Agent quality is measurable: task success rates, latency percentiles, cost per token, error rates. Search ranking can be based on objective metrics, not social signals.

3. **Real-time availability**: LinkedIn search returns profiles of people who may or may not be reachable. Agent search should return only agents that are currently available, within capacity, and compatible with the requester's API version. This is a live system status query, not a document retrieval problem.

4. **Composability search**: LinkedIn doesn't help you find "teams" — just individuals. Agent search should support queries like "find a pipeline of agents that can ingest PDF, extract tables, validate data, and load to Postgres" — searching for composable agent chains, not individual agents.

5. **Query-by-example**: Instead of describing what you want in natural language, agent search could accept an example input/output pair and find agents that can reproduce similar transformations. This has no LinkedIn analog.

### What's irrelevant

1. **Connection-based personalization**: Agent discovery shouldn't be biased by who you've used before (though recommendation should be). Search should surface the objectively best agent for the task.

2. **Boolean search for power users**: Agent capabilities should be structured enough that Boolean search is unnecessary. If you need Boolean operators to find the right agent, the platform's data model is wrong.

3. **SEO gaming**: With verified capabilities, there's nothing to "optimize" — agents either can do the task or they can't. The entire SEO optimization industry around LinkedIn profiles is a symptom of unstructured, unverified data that an agent platform should not replicate.

## Sources

### LinkedIn Engineering Blog
- [Did you mean "Galene"?](https://engineering.linkedin.com/search/did-you-mean-galene) — Original Galene architecture announcement
- [Galene Articles](https://engineering.linkedin.com/blog/topic/galene) — Collection of Galene-related posts
- [Search Federation Architecture at LinkedIn](https://www.linkedin.com/blog/engineering/search/search-federation-architecture-at-linkedin) — Federation mid-tier redesign
- [Improving Post Search at LinkedIn](https://www.linkedin.com/blog/engineering/search/improving-post-search-at-linkedin) — 2022 post search rebuild
- [Introducing Semantic Capability in LinkedIn's Content Search Engine](https://www.linkedin.com/blog/engineering/search/introducing-semantic-capability-in-linkedins-content-search-engine) — Two-tower EBR integration
- [Building a more intuitive and streamlined search experience](https://www.linkedin.com/blog/engineering/search/building-a-more-intuitive-and-streamlined-search-experience) — UX improvements
- [Unifying the LinkedIn Search Experience](https://engineering.linkedin.com/search/unifying-linkedin-search-experience) — Vertical unification
- [Cleo: Open Source Typeahead](https://engineering.linkedin.com/open-source/cleo-open-source-technology-behind-linkedins-typeahead-search) — Typeahead library

### Research Papers
- [Semantic Search at LinkedIn (arxiv:2602.07309)](https://arxiv.org/abs/2602.07309) — Feb 2026, LLM-powered search framework with SAGE judge and SLM ranker
- [LinkedIn Post Embeddings (arxiv:2405.11344)](https://arxiv.org/abs/2405.11344) — May 2024 (CIKM 2025), 50-dim multi-task embeddings
- [LiRank: Industrial Large Scale Ranking Models (arxiv:2402.06859)](https://arxiv.org/abs/2402.06859) — Feb 2024, ranking framework

### Official LinkedIn
- [LinkedIn Help: Search for people](https://www.linkedin.com/help/linkedin/answer/a525054) — Official search documentation
- [LinkedIn Help: Boolean search](https://www.linkedin.com/help/linkedin/answer/a524335) — Boolean operator reference
- [LinkedIn Help: Boolean in Recruiter](https://www.linkedin.com/help/recruiter/answer/a415295) — Recruiter Boolean search
- [LinkedIn Introduces AI-Powered People Search](https://news.linkedin.com/2025/LinkedIn-Introduces-New-AI-Powered-People-Search-Experience-to-Premium-Subscribers-in-the-US) — Nov 2025 launch announcement

### Analysis and Comparison
- [LinkedIn Advanced Search Filters 2026 Guide](https://evaboot.com/blog/linkedin-advanced-search) — Comprehensive filter documentation
- [How to Bypass LinkedIn Search Limit](https://evaboot.com/blog/hack-to-bypass-linkedin-search-limit) — Commercial use limit details
- [LinkedIn vs Indeed comparison](https://www.linkedhelper.com/blog/indeed-vs-linkedin/) — Platform comparison
- [LinkedIn's Galene Architecture (Lucidworks)](https://lucidworks.com/blog/linkedins-galene-search-architecture-built-on-apache-lucene/) — Third-party analysis
- [Galene SlideShare presentation](https://www.slideshare.net/lucidworks/galene-linkedins-search-architecture-presented-by-diego-buthay-sriram-sankar-linkedin) — Original architecture presentation
