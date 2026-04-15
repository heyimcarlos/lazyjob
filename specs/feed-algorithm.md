# Feed & Content Algorithm

## What it is

LinkedIn's feed is the central content consumption surface for 1.3 billion professionals, responsible for surfacing a personalized stream of posts, articles, videos, and documents from a member's network and beyond. The feed algorithm determines which content each member sees, in what order, and how widely each piece of content is distributed. It is LinkedIn's primary engagement driver and the surface through which most platform value is delivered — connecting professionals with relevant knowledge, opportunities, and people. As of 2026, the system has been fundamentally rebuilt around LLM-powered retrieval and transformer-based sequential ranking, replacing the legacy multi-source retrieval and DCNv2-based ranking that served the platform for years.

## How it works — User perspective

### The Feed Experience

When a member opens LinkedIn, they see an infinite-scrolling feed of content cards. Each card contains:
- **Author info**: Name, headline, mutual connections, follow/connect button
- **Content**: Text (up to 3,000 characters), images, videos, documents/carousels, polls, or link previews
- **Engagement bar**: Like (with reaction variants), Comment, Repost, Send
- **Social proof**: "[Name] and 47 others liked this", "[Connection] commented on this"
- **"See more" truncation**: Posts are truncated after ~3 lines on mobile, requiring a tap to expand

The feed mixes several content streams:
1. **Network posts**: Content from 1st-degree connections and followed accounts
2. **Suggested posts**: Content from outside the network, recommended based on interests
3. **Sponsored content**: Paid posts from advertisers, marked with "Promoted" label
4. **Notifications-driven**: "X commented on Y's post" social context entries

### Content Types and Their Performance (2026 data)

| Format | Avg Engagement Rate | Reach Multiplier | Key Metric |
|--------|-------------------|-------------------|------------|
| Carousels/PDFs | 7.00% | 3.5x vs text | Highest engagement format |
| Multi-image | High likes | Strong for reactions | Best for like generation |
| Short video (<60s) | 53% more than long | 1.4x vs other formats | Native video 69% better than links |
| Polls (7-day) | Strong reach | 1.64x | 70% penalty for 1-day polls |
| Text-only | Growing (+12% YoY) | Baseline | Benefits from dwell time |
| Newsletters | Moderate | Bi-weekly sweet spot | Weekly gets less reach |
| Articles | Lower reach | Limited | Repurposed content works better |

### The "Golden Hour" Lifecycle

Content goes through a staged evaluation:

1. **Quality check (0-60 min)**: The algorithm classifies the post as spam, low-quality, or clear. If clear, it's shown to a small initial audience (typically a fraction of 1st-degree connections). SVM classifiers and DNNs make this determination in ~200ms.

2. **Engagement testing (1-2 hours)**: Initial engagement is measured — not just volume, but quality. Meaningful comments from relevant professionals carry far more weight than likes. Author responses within 30 minutes boost total comments by 64% and views by 2.3×.

3. **Distribution decision (2-8 hours)**: If the post passes engagement thresholds, it expands to 2nd and 3rd-degree connections. The consumption rate (how much of the post people actually read) matters more than raw engagement counts.

4. **Long tail (8-24+ hours)**: High-performing content continues to circulate. LinkedIn's 2026 algorithm can resurface older content if it remains relevant, though a mid-2025 experiment showing too many old posts was rolled back after user complaints.

### Creator Features

**Creator Mode** was retired in March 2024. Instead of separating creators from regular users, LinkedIn distributed creator features to all members:
- Follow as default CTA (replacing Connect)
- Newsletter creation and publishing
- LinkedIn Live and Audio Events access
- Enhanced analytics dashboard
- Profile hashtag topics (removed February 2024)
- "Top Voice" badges for recognized contributors

**Collaborative Articles** were launched in 2023 as AI-generated article outlines where experts contributed insights. They were retired for new contributions in October 2025 after mixed reception — while they drove some engagement, many viewed them as low-quality AI slop. The AI-generated frameworks often felt generic.

**AI-Generated Content Impact**: As of October 2024, an estimated 54% of long-form LinkedIn posts are AI-generated. However, likely-AI posts receive 45% less engagement than original posts, suggesting the algorithm and users both discount them.

## How it works — Technical perspective

### Architecture Overview: Three-Generation Evolution

**Generation 1 (Pre-2023)**: Fragmented retrieval from multiple independent sources (chronological index, trending content, collaborative filtering, embedding-based systems) → DCNv2-based pointwise ranking → rule-based blending.

**Generation 2 (2023-2025)**: LiRank framework — enhanced DCNv2 with attention mechanisms, isotonic calibration, multi-task learning. Still pointwise scoring (each impression scored independently).

**Generation 3 (2025-2026)**: Unified LLM-powered retrieval + Feed Sequential Recommender (Feed-SR) using transformer-based sequential ranking. This is the current production system.

### Stage 1: Retrieval — LLM-Powered Unified Pipeline

The retrieval system replaces the previous heterogeneous multi-source approach with a single dual-encoder architecture powered by fine-tuned LLMs.

**Dual Encoder Architecture**:
- **Member encoder**: Converts user profile data, engagement history, and behavioral signals into a text prompt, then generates a dense vector embedding using a fine-tuned LLaMA 3 model
- **Content encoder**: Posts are encoded in the same embedding space using the same model family
- **Matching**: GPU-accelerated exhaustive k-nearest-neighbor search across millions of posts in sub-50ms

**Key Innovation — Semantic Understanding**: The LLM embeddings capture conceptual relationships beyond keywords. A professional interested in "electrical engineering" can discover posts about "small modular reactors" through the model's world knowledge, not keyword overlap.

**Numerical Feature Handling**: A critical discovery — raw numerical features (e.g., "views:12345") performed poorly in the text prompt format. Converting to percentile buckets wrapped in special tokens (e.g., `<view_percentile>71</view_percentile>`) improved retrieval correlation by **30×** and recall by **15%**.

**Training**:
- InfoNCE loss with negative sampling
- Easy negatives: randomly sampled unshown posts
- Hard negatives: posts shown but not engaged with — adding just 2 hard negatives per member improved recall by +3.6%
- Filtering training data to only positively-engaged posts: 37% memory reduction, 40% more sequences per batch, 2.6× faster training

**Online Serving — Three Nearline Pipelines**:
1. Prompt generation pipeline (minutes latency)
2. Embedding generation via GPU clusters
3. GPU-accelerated indexing for sub-50ms nearest-neighbor search

**Member Profile Embeddings**: Qwen3 0.6B parameter model generates dense representations, achieving +2% AUC gains for cold-start members with fewer than 10 historical actions.

### Stage 2: Ranking — Feed Sequential Recommender (Feed-SR)

Feed-SR replaces the DCNv2-based ranker with a transformer-based sequential recommendation model. Published as an academic paper in February 2026 (arxiv:2602.12354).

**Core Architecture**:
- **Decoder-only transformer** with Pre-LayerNorm, RoPE positional embeddings, and scaled residual connections
- **Sequence input**: Interleaved post and action embeddings from 1,000 recent impressions per member: `X_in = [X₁, A₁, X₂, A₂, ..., X_T, A_T]`
- **Causal attention masking**: The model processes interaction history as an ordered sequence, capturing temporal patterns (e.g., Monday ML engagement → Tuesday distributed systems → Wednesday content discovery)
- **Late fusion**: Context features (device type, profile embeddings, count/affinity signals) are appended after the transformer layers
- **MMoE prediction head**: Multi-gate Mixture-of-Experts for multi-task prediction (likes, comments, shares, dwell time, etc.)

**Key Technical Properties**:
- Uses only ~20% of production features but outperforms the previous full-feature DCNv2 ranker
- ~50-dimensional content embeddings + learned ID embeddings for actors
- Scaling law: For every 10× increase in training FLOPS, Long Dwell AUC improves by ~0.0093
- 1-year history windows yield +0.21% Long Dwell AUC over 6-month windows

**Why Not LLM-as-Ranker?**: LinkedIn evaluated using LLMs directly for ranking but found: inefficient token usage (hundreds per post, tens of thousands for histories), poor numeric feature encoding, and failure on network-based recommendations. Feed-SR's explicit ID embeddings better capture relationship strength.

### Multi-Task Objective Function

The final ranking score is a **linear combination of multiple predicted objectives**:
- **Positive engagement**: P(like), P(comment), P(share), P(repost)
- **Dwell time**: Binary classification predicting whether time-on-content exceeds contextual percentiles (e.g., 90th percentile for that content type and position)
- **Negative signals**: P(skip), P(hide), P(report) — subtracted from the score

**Dwell Time Modeling**: Rather than predicting raw seconds (which varies wildly by content type), LinkedIn uses an "Auto Normalized Long Dwell Model":
- Binary classifier predicting whether a user will spend more time than x% of comparable posts
- Percentile thresholds recalculated daily
- Normalized by content type, creator type, and distribution method
- This eliminates bias toward longer-format content

**Task Grouping**: Tasks are grouped by statistical similarity rather than arbitrary assignment. "Like" and "Contribution" share a tower (high positive rates), while "Comment" and "Share" are separated.

### Serving Infrastructure

**Disaggregated Inference**:
- CPU service handles feature fetching, tracking, and transformations
- Python-based PyTorch GPU server with high-performance gRPC interface
- Zero-copy Arrow buffer conversion eliminates serialization overhead

**Multi-Item Scoring Optimization (SRMIS)**:
- All ~512 candidates share the same member history context
- Custom Flash Attention variant computes history once, scores candidates in parallel
- **80× speedup** on transformer forward pass vs. naive per-candidate scoring
- **2× speedup** over standard masked SDPA through compute-skipping

**CPU-Side Optimizations**:
- Member history parsing: 450ms → 2ms (225× speedup via NumPy strided arrays)
- Sparse-to-dense conversion: 254ms → 5ms (50× speedup)
- Overall: 66% fewer CPU cycles, 71% fewer instructions, 90% fewer cache misses

**Latency**: Sub-second end-to-end serving despite transformer computational demands.

**Energy Efficiency**: Compared to the previous CPU-served ranker, the new GPU-served model uses 0.7× the inference energy (GPU efficiency gains offset the increased compute), though training costs 3.6× more.

### LiRank — The Previous-Generation Framework (Still Powers Other Surfaces)

LiRank continues to power Jobs, Ads, and other ranking surfaces. Key components:

- **Residual DCN**: Enhanced DCNv2 with self-attention between low-rank transformations (query/key/value matrices from duplicated low-rank mappings)
- **Isotonic Calibration Layer**: Trainable calibration integrated into the neural network (not post-hoc), bucketizing predicted logits with per-bucket learnable weights
- **Dense Gating**: MLP layers widened to 3,500 dimensions across 4 layers
- **QR Hashing**: Quotient-remainder vocabulary compression — 4B IDs compressed 1000× to ~4M rows, eliminating vocabulary maintenance
- **8-bit Quantization**: Row-wise middle-max quantization reduces embedding tables by 70%+

**Training Scalability**:
- 4D Model Parallelism: Training time from 70 to 20 hours
- Custom Avro Tensor Dataset Loader: 160× faster than stock TensorFlow reader
- Total: 50% reduction in end-to-end training time

### Content Quality and Spam Detection

**Three-Layer Defense**:

1. **At creation (synchronous, ~200ms)**: SVM classifiers and DNNs label posts as spam/low-quality/clear. Separate classifiers for text, images, long-form content.

2. **During distribution (continuous monitoring)**: Random forest classifiers and gradient boosted trees predict viral spam potential based on:
   - Network features (follower counts, industry diversity, geographic spread of engagers)
   - Temporal velocity signals (engagement acceleration patterns)
   - Content features (polarity scoring, spam indicators from user reports)
   - Result: 48% reduction in spam/low-quality content impressions

3. **Reactive defense**: Monitors engagement patterns post-publication to catch content that evaded proactive filters. Combined proactive + reactive models reduced overall spam views by 7.3% and policy-violating content views by 12%.

**Engagement Bait Detection (2025-2026)**: Advanced NLP classifiers actively scan for manipulative prompts like "Comment YES if you agree!" or "Like this to get the PDF." These trigger immediate algorithmic penalties. The system also detects:
- Mismatched video-text combinations
- Recycled content without novel insights
- Engagement pod patterns (coordinated commenting groups)
- Generic AI-generated template content

**Graduated Enforcement**: Rather than binary allow/block, LinkedIn applies a spectrum: feed demotion → geographic restrictions → profile-only visibility → sitewide suppression → account suspension.

### Notification-Driven Engagement Loop

LinkedIn's **Concourse** system generates personalized content notifications in near-real-time, surfacing social signals ("X commented on Y's post") that drive members back to the feed. The notification system uses:
- Response prediction models to estimate engagement likelihood
- Utility optimization: high-utility notifications (likely to please) vs. low-utility (likely to irritate)
- Two categories: Unfiltered (peer-to-peer messages, connection invites) and Filter-Eligible (content engagement notifications)
- Notifications serve as a key re-engagement mechanism, completing the loop: post → notify connections → drive feed visits → generate more engagement

### Infrastructure Stack

- **Kafka**: 2.1 trillion messages/day, peaks of 4.5M messages/sec per cluster. All activity data, operational metrics, service call traces flow through Kafka feeds.
- **Samza**: Real-time stream processing framework built on YARN. Powers nearline feature computation and notification generation.
- **Espresso**: Document store for content and profile data
- **Voldemort**: Pre-computed feature stores for low-latency serving
- **GPU Clusters**: For embedding generation (retrieval) and model inference (ranking)
- **Pro-ML Platform**: Unified model training and deployment for TensorFlow and PyTorch models

## What makes it successful

### 1. Sequential Understanding of Professional Interests

The Generative Recommender model's core insight is treating engagement history as a narrative rather than a bag of signals. By processing 1,000+ interactions as an ordered sequence with causal attention, the model captures evolving professional interests — noticing that someone is shifting from "data engineering" to "MLOps" before they've explicitly updated their profile. This temporal awareness produces **+2.10% time spent** over the previous pointwise ranker, with the strongest gains among the most active members.

### 2. Semantic Retrieval via LLM Embeddings

The shift from keyword-based to semantic retrieval dramatically expanded content discovery. The LLM understands that "small modular reactors" is relevant to "electrical engineering" professionals through world knowledge, not keyword overlap. This addresses the fundamental cold-start and topic discovery problems that plagued the previous fragmented retrieval system.

### 3. Professional Context as Moat

Unlike general-purpose social feeds, LinkedIn's algorithm deeply integrates professional signals:
- Author expertise matters — the algorithm rewards consistent topical authority
- Comment quality from relevant professionals carries more weight than volume
- Network context (who else engaged, from what industry) influences distribution
- This makes the feed harder to game through generic viral tactics

### 4. Dwell Time as Quality Signal

The innovation of normalized dwell time — predicting whether engagement exceeds contextual percentiles rather than raw seconds — elegantly solves the format bias problem. A 30-second read of a text post can score as highly as a 3-minute video watch if both exceed their format-specific norms. This gives genuine quality signals from the 70% of users who are "ghost scrollers" (consume without explicit engagement).

### 5. Multi-Objective Optimization

The linear combination of positive and negative objectives (engagement + dwell - skip - hide) prevents the algorithm from optimizing solely for engagement bait. The explicit modeling of negative signals creates a feedback loop where low-quality content is penalized even when it generates some engagement.

### 6. The Golden Hour Mechanic

The staged distribution model creates a powerful incentive structure: authors are motivated to engage with early commenters (64% more comments, 2.3× views), which creates genuine conversation rather than passive broadcasting. This turns the feed into a conversation platform rather than a content dump.

### 7. Engagement Notification Loop

The Concourse notification system completes the flywheel: content → engagement → notifications to connections → feed visits → more engagement. Social proof ("X and 47 others liked this") leverages FOMO and professional curiosity to drive re-engagement.

## Weaknesses and gaps

### 1. Organic Reach Collapse

The 2026 LLM-powered algorithm caused a dramatic decline in organic reach: average post views down ~50%, engagement down ~25%, follower growth down ~59%. While LinkedIn frames this as "relevance-first" design, it creates frustration for creators and professionals who invested in building audiences. The platform risks losing content creators to platforms with more transparent and rewarding distribution.

### 2. "Facebookification" of Professional Content

Despite LinkedIn's efforts to maintain professional relevance, the feed increasingly resembles general social media:
- Personal stories outperform professional analysis
- Emotional narratives get more engagement than technical depth
- "Humblebrags" and manufactured vulnerability posts thrive
- The algorithm rewards posting frequency and format tricks over substance

### 3. Algorithmic Opacity

Unlike X/Twitter (which open-sourced its algorithm), LinkedIn provides zero transparency into ranking decisions. Creators cannot understand why a post performed well or poorly. The engagement signals are complex and the sequential model is essentially a black box. This breeds conspiracy theories and gaming behavior.

### 4. AI Content Flood

With 54% of long-form posts estimated to be AI-generated, the feed faces a quality crisis. While AI posts get 45% less engagement, they still consume feed slots. LinkedIn's NLP classifiers for detecting AI content are in a constant arms race with improving language models. The retired Collaborative Articles experiment showed the risks of AI-generated content.

### 5. Ghost Scroller Problem

70% of LinkedIn users are passive consumers who never engage. The dwell time signal helps capture their preferences, but the platform struggles to convert them into active participants. This creates a creator-consumer imbalance where a small minority of users produce all content.

### 6. Cold Start and New Member Experience

While LLM embeddings help with cold start (+2% AUC for members with <10 actions), new members still face a largely irrelevant feed until they build interaction history. The sequential model explicitly showed "not statistically significant" improvements for new members in A/B tests.

### 7. Content Format Arms Race

The constant shifting of format preferences (carousels were king, then video, then polls, now carousels again) creates a treadmill for creators. The algorithm's format biases change faster than creators can adapt, leading to frustration and low-quality content produced solely for algorithmic favorability.

### 8. Engagement Pod and Coordination Resistance

Despite crackdowns, coordinated engagement remains a challenge. Professional communities naturally form engagement groups, making it hard to distinguish genuine industry interest from artificial boosting. The sequential model should help (it can detect artificial temporal patterns), but the problem persists.

## Competitive landscape

### X (Twitter/X)

**Architecture**: Originally open-sourced in 2023, revealing the 50/50 in-network/out-of-network split. Migrated to Grok-based ranking in 2025-2026 (Rust codebase with components Home Mixer, Thunder, Phoenix).

**Key Differences**:
- **Transparency**: X open-sourced its algorithm (though the Grok migration reduced this transparency). LinkedIn has never shared algorithm details publicly.
- **Scoring weights**: Simplified engagement weights (Likes ×1, Retweets ×20, Replies ×13.5, Profile Clicks ×12). Much more transparent than LinkedIn's multi-objective optimization.
- **Premium boost**: X Premium subscribers get 2-4× reach boost — pay-to-play model LinkedIn hasn't adopted for organic content.
- **Sentiment analysis**: Grok's sentiment analysis boosts positive/constructive content and reduces negative/combative tones, even if they generate high engagement. LinkedIn doesn't explicitly model sentiment this way.
- **Real-time focus**: X optimizes for recency and real-time conversation. LinkedIn tolerates older content if it's relevant.

**Strengths over LinkedIn**: Real-time discourse, open algorithm, better for breaking news and public conversation, no corporate veneer requirement.

**Weaknesses**: Less professional context, more noise, toxicity challenges, less sophisticated ML infrastructure.

### Facebook/Meta

**Architecture**: Four-step Inventory → Signals → Predictions → Relevance Score pipeline. RankNet-7 model (2026) processing micro-signals like cursor hover duration, partial scroll depth, and re-read frequency.

**Key Differences**:
- **Recommended content**: Up to 50% of Facebook feeds are now from accounts you don't follow — far more aggressive than LinkedIn's suggested content ratio.
- **Video dominance**: All videos reclassified as Reels (mid-2025). Reels account for 38.4% of time spent. LinkedIn has not made this video-first pivot.
- **UTIS model**: User True Interest Survey — Facebook directly surveys users in-feed about content relevance. LinkedIn relies entirely on behavioral signals.
- **Micro-signals**: RankNet-7 processes cursor hover duration, partial scroll depth, re-read frequency — more granular than LinkedIn's dwell time model.
- **"Genuine care" signals**: Prioritizes substantive replies and back-and-forth conversations, similar to LinkedIn's comment quality emphasis.

**Strengths over LinkedIn**: More sophisticated behavioral micro-signals, direct user feedback integration, better video infrastructure, larger scale (3B+ users vs 1.3B).

**Weaknesses**: No professional context model, general-purpose content dilution, privacy backlash limits signal collection.

### TikTok

**Key Differences**:
- **Pure recommendation**: Feed is 100% algorithmic — no social graph dependency. LinkedIn's feed still heavily weights network connections.
- **Watch time optimization**: Completion rate is the primary signal. LinkedIn's dwell time model is similar in spirit but less central.
- **Content-first, not creator-first**: TikTok can make any video go viral regardless of follower count. LinkedIn still heavily favors established networks.
- **A/B testing content**: TikTok reportedly tests thumbnails and initial segments with small audiences. LinkedIn's golden hour mechanic is similar but less sophisticated.

**Relevance**: TikTok's success with pure-recommendation feeds (no social graph needed) is a key lesson for agent platforms — agents don't have social networks, so a recommendation-first approach may be more appropriate.

### Substack / Medium

**Key Differences**:
- **Subscription model**: Content distribution driven by subscriber choices, not algorithmic ranking.
- **Long-form focus**: Designed for essays and newsletters, not feed scrolling.
- **Creator economics**: Direct reader-to-writer payments. LinkedIn monetizes through ads and premium subscriptions, not content creator payments.

**Relevance**: LinkedIn's newsletter feature competes directly but with inferior economics for creators (no direct monetization).

## Relevance to agent platforms

### What Transfers Directly

1. **Sequential interaction modeling**: The Feed-SR approach of treating interaction history as an ordered sequence directly applies to agent platforms. An agent's task history, tool usage patterns, and collaboration sequences tell a story about evolving capabilities and specializations. A sequential recommender could surface relevant agents by understanding these trajectories.

2. **Multi-objective optimization**: Agent feeds would need to balance multiple objectives — task relevance, capability match, cost efficiency, latency requirements, reliability track record. LinkedIn's linear combination of predicted objectives with negative signal subtraction provides a proven framework.

3. **Dwell time → execution quality**: LinkedIn's normalized dwell time concept translates to execution quality metrics for agents. Rather than raw task completion time, measure whether an agent's performance exceeds contextual percentiles (adjusted for task type, complexity, domain).

4. **Content quality classification**: The three-layer spam/quality detection system (at creation, during distribution, reactive) maps to agent output quality assurance — classify agent outputs at generation, monitor during task execution, and reactively evaluate based on downstream outcomes.

### What Needs Reimagining

1. **Feed as task marketplace, not content stream**: For agents, the "feed" isn't content to consume — it's tasks to bid on, capabilities to discover, and collaboration opportunities to evaluate. The feed metaphor needs fundamental rethinking. Instead of "what's interesting to read," it's "what's relevant to do" or "who's relevant to work with."

2. **Engagement signals → performance signals**: Likes, comments, and shares are meaningless for agents. The equivalent signals are: task completion rate, output quality scores, response latency, cost efficiency, error rates, and downstream task success (did the work product enable the next step?). These are all objectively measurable, unlike human engagement which is inherently subjective.

3. **Golden hour → capability validation**: Instead of testing content with a small audience, new agent capabilities should be validated through staged rollout — small task samples → monitored production → full deployment. The staged evaluation concept transfers, but the signals are performance metrics, not engagement.

4. **Network-based distribution → capability-based routing**: LinkedIn distributes content through social network connections. Agent platforms should route based on capability graphs — which agents can handle this task type, with what reliability, at what cost? The social graph becomes a capability/compatibility graph.

5. **Creator features → provider features**: Agent "creators" (developers who build and deploy agents) need dashboards showing not engagement metrics but: uptime, task success rates, cost per task, error categorization, capability utilization, and competitive benchmarking against similar agents.

### What's Irrelevant

1. **Golden hour engagement gaming**: Agents don't need to post at optimal times or respond to comments within 30 minutes. Performance is measured objectively, not through social signals.

2. **Content format optimization**: Carousels vs. video vs. text is irrelevant. Agent outputs are structured data, API responses, and task completions — not content for human consumption feeds.

3. **Engagement bait detection**: Agents don't write "Comment YES if you agree!" The equivalent concern is capability inflation (agents claiming skills they don't have), which is solved through verified benchmarks, not NLP classifiers.

4. **Ghost scroller problem**: In an agent platform, every participant is an active actor. There are no passive consumers — every agent either provides or consumes services. The engagement asymmetry problem doesn't exist.

### Key Insight for Agent Platforms

LinkedIn's biggest algorithmic challenge is measuring content quality through noisy, gameable human behavioral proxies (likes, dwell time, comments). An agent platform has a massive structural advantage: **quality is directly measurable**. Task success/failure is binary. Output quality can be evaluated by downstream agents. Latency is a number. Cost is a number. This eliminates the entire engagement-signal-as-proxy problem that consumes most of LinkedIn's ML engineering effort.

The Feed-SR sequential model's approach — treating interaction history as an ordered narrative — is the most transferable concept. An agent's trajectory of tasks, tool calls, and collaborations tells a rich story about specialization and capability evolution. A sequential recommender trained on these trajectories could power both task routing ("which agent should handle this?") and capability discovery ("what agent do I need for this new task type?").

## Sources

### LinkedIn Engineering Blog
- [Engineering the next generation of LinkedIn's Feed](https://www.linkedin.com/blog/engineering/feed/engineering-the-next-generation-of-linkedins-feed) — March 2026 deep dive on LLM-powered retrieval and Generative Recommender
- [Leveraging Dwell Time to Improve Member Experiences](https://www.linkedin.com/blog/engineering/feed/leveraging-dwell-time-to-improve-member-experiences-on-the-linkedin-feed) — Technical details on dwell time modeling
- [Strategies for Keeping the LinkedIn Feed Relevant](https://www.linkedin.com/blog/engineering/feed/strategies-for-keeping-the-linkedin-feed-relevant) — Content quality and spam detection approach
- [Viral Spam Content Detection at LinkedIn](https://www.linkedin.com/blog/engineering/trust-and-safety/viral-spam-content-detection-at-linkedin) — ML-based spam detection system
- [Feed AI Team Page](https://engineering.linkedin.com/teams/data/artificial-intelligence/feed)
- [Concourse: Generating Personalized Content Notifications](https://engineering.linkedin.com/blog/2018/05/concourse--generating-personalized-content-notifications-in-near)

### Academic Papers
- [LiRank: Industrial Large Scale Ranking Models at LinkedIn](https://arxiv.org/html/2402.06859v1) — February 2024, comprehensive LiRank framework paper
- [An Industrial-Scale Sequential Recommender for LinkedIn Feed Ranking](https://arxiv.org/html/2602.12354v1) — February 2026, Feed-SR paper with full architecture and A/B test results
- [Large Scale Retrieval for the LinkedIn Feed using Causal Language Models](https://arxiv.org/abs/2510.14223v1) — October 2025, LLM-based retrieval paper

### Infrastructure
- [Running Kafka At Scale](https://engineering.linkedin.com/kafka/running-kafka-scale)
- [Apache Samza: LinkedIn's Real-time Stream Processing Framework](https://engineering.linkedin.com/data-streams/apache-samza-linkedins-real-time-stream-processing-framework)
- [Kafka Ecosystem at LinkedIn](https://www.linkedin.com/blog/engineering/open-source/kafka-ecosystem-at-linkedin)

### Industry Analysis
- [LinkedIn Feed Algorithm Update 2026: How LLM-Powered Ranking Works](https://almcorp.com/blog/linkedin-feed-algorithm-update-llm-2026/)
- [How the LinkedIn Algorithm Works in 2025](https://blog.hootsuite.com/linkedin-algorithm/)
- [LinkedIn Benchmarks 2026](https://www.socialinsider.io/social-media-benchmarks/linkedin)
- [The Unofficial LinkedIn Algorithm Guide, Q1 2026 Edition](https://www.trustinsights.ai/wp-content/uploads/2025/09/the_unofficial_linkedin_algorithm_guide_fall_2025_edition.pdf)
- [Over ½ of Long Posts on LinkedIn are Likely AI-Generated](https://originality.ai/blog/ai-content-published-linkedin)
- [LinkedIn's Removing Its Creator Mode Option](https://www.socialmediatoday.com/news/linkedins-removing-creator-mode-option/706585/)
- ['Cesspool of AI crap' — LinkedIn Collaborative Articles](https://fortune.com/2024/04/18/linkedin-microsoft-collaborative-articles-generative-ai-feedback-loop-user-backlash/)

### Competitive References
- [X/Twitter Open Source Algorithm](https://github.com/twitter/the-algorithm)
- [X's Recommendation Algorithm Blog Post](https://blog.x.com/engineering/en_us/topics/open-source/2023/twitter-recommendation-algorithm)
- [Feed Ranking Systems: A Deep Dive](https://www.shaped.ai/blog/feed-ranking-systems) — Cross-platform comparison
