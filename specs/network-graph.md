# Network Graph & Connection System

## What it is

LinkedIn's network graph is the foundational social infrastructure underlying the entire platform. It maps relationships between 930+ million members through explicit connections (bidirectional) and follows (unidirectional), organizing them into degree-based tiers (1st, 2nd, 3rd) that govern visibility, messaging access, content distribution, and search ranking. The network graph is not just a feature — it IS the platform's core asset. LinkedIn's "People You May Know" (PYMK) recommendation system, which runs on top of this graph, is responsible for building more than 50% of LinkedIn's professional connections. The graph extends beyond people to encompass the broader "Economic Graph" — 270 billion edges connecting members, companies, skills, jobs, and schools.

## How it works — User perspective

### Connection degrees

LinkedIn organizes every member's relationship to every other member into three tiers based on graph distance:

**1st-degree connections**: People you are directly connected to via a mutually accepted connection request. You can message them for free, see their full profile, see their contact information (if shared), and their content appears in your feed. This is a symmetric, bidirectional relationship — both parties must consent.

**2nd-degree connections**: People connected to your 1st-degree connections but not to you directly. You can see their full name and profile, send them a connection request (with optional note), and see mutual connections. You cannot message them for free (requires InMail or a shared group). Their content may appear in your feed if a 1st-degree connection engages with it.

**3rd-degree connections**: People connected to your 2nd-degree connections. You can see their first name and last initial (unless in a shared group). You can sometimes send connection requests. Very limited visibility.

**Out of network**: Beyond 3rd degree. Minimal visibility. Name may be partially hidden. Cannot send connection requests directly. Requires InMail (Premium) to reach.

### Connection request flow

1. **Initiating**: Click "Connect" on a profile. Optionally add a personalized note (up to 300 characters). LinkedIn sometimes removes the note option for connections without shared context (to discourage spam).
2. **Receiving**: Target gets a notification with the request. They can Accept, Ignore, or Report ("I don't know this person").
3. **Acceptance**: Both parties become 1st-degree connections and automatically follow each other. The connector's entire network expands — they gain new 2nd and 3rd-degree paths.
4. **Ignoring/Withdrawal**: Ignored requests stay pending. Sender can withdraw after sending. Must wait ~3 weeks before re-requesting the same person. Pending requests count against the 500 pending invitation cap.
5. **Reporting**: "I don't know this person" reports damage the sender's account health and may trigger restrictions.

**Personalization impact**: Personalized connection requests see 30-45% acceptance rates vs. 15% for generic ones. Best practices: under 300 characters, sent Tuesday-Thursday 8AM-2PM.

### Follow vs. Connect

LinkedIn has two distinct relationship types:

**Connect** (bidirectional): Both parties must agree. Creates a 1st-degree connection. Both see each other's content. Both can message for free. Counts toward the 30,000 connection cap. Limited to ~100-200 requests/week.

**Follow** (unidirectional): No permission needed. Follower sees the followed person's public posts. The followed person does NOT see the follower's content. No messaging access granted. No limit on number of follows. No cap on followers received.

**Default button behavior**: As of 2024-2025, LinkedIn removed the separate "Creator Mode" toggle and made the Follow/Connect button customizable for all users. The default primary action button is now "Follow" rather than "Connect" — a significant shift that encourages audience-building over network-building. Users can change this in Settings > Visibility > Followers > "Make follow primary."

**Automatic follow**: Connecting automatically creates a follow relationship in both directions. As of September 2024, removing a connection also automatically unfollows them (previously, unfollowing was independent of disconnecting).

### Connection limits and anti-spam

- **Weekly request limit**: 100-200 connection requests per week (rolling 7-day window)
- **Free accounts**: Recommended to stay under 80/week for safety
- **Sales Navigator**: May allow 150-200/week
- **Pending cap**: Maximum 500 pending (unaccepted) invitations at any time
- **Network cap**: Maximum 30,000 1st-degree connections total
- **Follower cap**: No limit
- **Acceptance rate threshold**: If acceptance rate drops below ~30%, LinkedIn throttles your sending limits, assuming spam behavior
- **"I don't know this person" reports**: Accumulating these triggers account restrictions or temporary bans
- **Withdrawal cooldown**: ~3 weeks before re-requesting after withdrawing an invitation

### Managing connections

- **Remove connection**: Silent — the other person is not notified. As of 2024, removing also unfollows.
- **Unfollow**: Stop seeing someone's content without disconnecting. They are not notified.
- **Mute**: Hide notifications from a connection but keep feed visibility.
- **Block**: Complete bilateral invisibility — neither party can see the other's profile, posts, or appear in search results. The blocked person is not explicitly notified but may notice if they try to find you.

### People You May Know (PYMK)

PYMK appears on the homepage, in notifications, in the "My Network" tab, and in email digests. It shows profile cards of suggested connections with:
- Mutual connection count
- Shared organizations (company, school, group)
- A "Connect" button for one-click action

PYMK uses **impression discounting** — suggestions you see but don't act on get downranked over time. The system refreshes daily (sometimes hourly for active users).

### Network-adjacent features

**Alumni Tool**: A dashboard filtering all members who listed a specific school — filterable by graduation year, location, employer, job function, major, skills, and connection degree. Higher response rates than cold outreach due to shared institutional bonds.

**Social Selling Index (SSI)**: A 0-100 score measuring networking effectiveness across four pillars (25 points each):
1. Establishing professional brand (profile completeness, content)
2. Finding the right people (searches, profile views)
3. Engaging with insights (content interaction, InMail responses)
4. Building relationships (connection nurturing, messaging)

SSI updates daily. LinkedIn reports 45% more opportunities for high-SSI sellers, but the metric is being de-emphasized in favor of AI tools.

**Groups**: Shared group membership creates a pseudo-connection — members of the same group can message each other for free and appear more prominently in each other's searches and PYMK.

**Collaborative Articles**: LinkedIn's highest-engagement content format (12.3% engagement rate in Q1 2026) that creates loose community ties around topic expertise.

## How it works — Technical perspective

### LIquid: The graph database

LinkedIn's network graph runs on **LIquid**, a purpose-built distributed graph database that replaced the legacy GAIA system. Key architectural decisions:

**Data model**: Triple-based edge model using (subject, predicate, object) strings. Edges represent connections, follows, subscriptions, group memberships, employment relationships, skill associations, and more. Compounds of edges represent n-ary relationships (e.g., "Person A worked at Company B from Date C to Date D with Title E").

**Storage**: Log-structured, in-memory inverted index. Each edge is appended to a graph log. Offsets serve as virtual timestamps for consistency. Each identity maintains a mini-log of edges referencing it, enabling efficient bidirectional lookups.

**Consistency model**: Single-writer, wait-free. One writer appends edges sequentially. Readers establish consistency by selecting a log offset (point-in-time query). Deletes use tombstone edges. No lock contention or deadlocks possible.

**Query language**: Declarative, based on Datalog. Queries are expressed as "little graphs of edge constraints." Dynamic query planning chooses optimal access paths — e.g., when finding relationships between two entities, it scans the index of the entity with fewer edges first.

**Scale**:
- 270 billion edges (and growing)
- 2 million queries per second (expected to double in 18 months)
- 99.99% availability target
- Entire graph loaded into working memory (at least 1 TB RAM per server)
- Sub-50ms average latency (down from 1s+ on GAIA)

**Migration impact**: When PYMK moved from GAIA to LIquid:
- QPS: 120 -> 18,000 (150x improvement)
- Latency: >1s -> <50ms average
- CPU utilization: >3x reduction
- Schema update time: months -> ~2 weeks

### Degree computation

Computing 1st/2nd/3rd degree requires real-time graph traversal. For 2nd-degree computation:
- Average member has ~250 1st-degree connections
- 2nd-degree network: ~50,000+ entities (250 x 200+ per connection, with deduplication)
- LinkedIn pre-computes and caches these for active members rather than computing on every request
- Write amplification is a concern: each new connection creates ~250x the base write rate for 2nd-degree updates

**Triangle closing queries**: LIquid's primary pattern for PYMK candidate generation — finding entities that would "close a triangle" (A knows B, B knows C, suggest A-C). This is the core graph traversal operation.

### People You May Know (PYMK) — ML pipeline

PYMK is LinkedIn's most important growth engine. The technical pipeline:

**1. Candidate generation** (graph traversal):
- Triangle closing: friends-of-friends analysis
- Organizational co-membership: shared company, school, group
- Geographic proximity
- Generates candidates from 100s of billions of potential pairs

**2. Feature extraction** (100s of features):
- Common connection count and quality
- Organizational overlap (company, school, group)
- Temporal co-presence (overlapping time at same org, weighted by org size and tenure duration)
- Geographic distance
- Age/demographic similarity
- Profile similarity signals
- Historical interaction patterns

**3. Scoring model**:
- **Logistic regression** for binary classification (will they connect?)
- Trained on 100s of millions of samples using LinkedIn's open-sourced ML-Ease library
- Processes 100s of terabytes of data daily
- Features weighted by signal strength

**4. Re-ranking and fairness**:
- **Impression discounting**: Results seen but not acted on are downranked
- **Fairness re-ranking**: Using LinkedIn's LiFT (LinkedIn Fairness Toolkit) to prevent "rich get richer" bias
- Problem: Frequently active members were overrepresented in training data, causing PYMK to optimize for frequent users at the expense of infrequent members
- Solution: Post-processing re-ranking that decrements scores for recipients with many unanswered invitations, ensuring equality of opportunity between frequent and infrequent members
- Result: 5.44% increase in invites sent, 4.8% increase in connections made to previously underrepresented infrequent members

**5. Serving**:
- Primarily batch-processed (daily refresh) using Voldemort (LinkedIn's distributed key-value store), Azkaban (workflow scheduler), Kafka (event streaming), and Cubert
- Near real-time augmentation: new connections are indexed within seconds and can affect subsequent PYMK results
- Integration with Venice (ML features) and Pinot (analytics) for ranking

### LiGNN: Graph Neural Networks

LinkedIn deployed **LiGNN**, a production GNN framework, to enhance recommendations across the platform. Key technical details:

- Operates on a heterogeneous graph integrating members, jobs, posts, ads, and other entities
- Temporal graph architectures with long-term losses
- Cold start solutions via graph densification and ID embeddings
- Multi-hop neighbor sampling for learning node representations
- 7x training speedup through adaptive neighbor sampling, batch grouping/slicing, shared-memory queues, and local gradient optimization
- Nearline inference pipeline: Item creation events arrive via Kafka -> feature collection -> GNN inference -> embeddings stored in Venice or published to Kafka
- Results: 0.1% weekly active user lift from people recommendations, 0.5% feed engaged DAU lift, 1% job application hearing-back rate improvement, 2% ads CTR lift

### The Economic Graph

The network graph is a subset of LinkedIn's broader **Economic Graph** — a knowledge graph connecting:
- 930M+ members
- 59M+ companies
- 39K+ standardized skills (with 374K+ aliases)
- Millions of open jobs
- 90K+ schools

All entities and relationships are stored in LIquid. The Economic Graph powers cross-domain recommendations: "hiring in your network" (network topology + job data), skills-based suggestions (network + skills graph), and alumni features (network + education entities).

## What makes it successful

### 1. The connection request as a trust gate

The bidirectional connection request creates a meaningful social contract. Unlike Twitter/X follows (zero friction, zero commitment), a LinkedIn connection implies mutual professional recognition. This makes the network higher-signal for recruiters and sellers — a 1st-degree connection means something. The friction is calibrated: high enough to prevent spam networks, low enough to not discourage growth.

### 2. Degree-based access control creates upgrade incentives

The degree system is LinkedIn's most elegant monetization lever. 2nd-degree profiles are visible but not messageable (for free). 3rd-degree profiles are partially hidden. This creates natural demand for InMail credits and Premium subscriptions. Recruiters NEED expanded network access — Recruiter Lite only searches up to 3rd degree, while full Recruiter searches the entire network. The degree system simultaneously provides value (organizing relationships) and creates scarcity (limiting access).

### 3. PYMK as a growth flywheel

PYMK building 50%+ of all connections means LinkedIn's growth is self-reinforcing. Each new connection generates new PYMK candidates for both parties AND their networks. The triangle-closing algorithm ensures suggestions feel relevant (you actually know these people), maintaining high acceptance rates that keep the flywheel spinning. Impression discounting prevents stale suggestions from degrading the experience.

### 4. Network effects with density, not just size

LinkedIn's moat isn't just 930M members — it's the density of professional relationships mapped. A competitor can't replicate the graph even if they attract the same users, because users would need to rebuild all their connections. The more connections exist, the more useful search, PYMK, and content distribution become (Metcalfe's Law), but LinkedIn has specifically optimized for connection density over raw size.

### 5. Follow + Connect dual model

The 2024 shift to making Follow the default button was strategic: it lets LinkedIn serve both networking (connect) and content consumption (follow) use cases. Creators can build large audiences without hitting the 30,000 connection cap. Consumers can curate their feed without committing to bidirectional relationships. This lets LinkedIn compete with X/Twitter on content while maintaining its unique connection-based professional network.

### 6. Real-time graph infrastructure

LIquid's sub-50ms latency on 270B edges at 2M QPS is a genuine technical achievement. New connections are reflected in PYMK within seconds. This makes the platform feel alive — connect with someone and immediately see new suggestions from their network. The Datalog-based query language enables complex graph patterns (multi-hop traversals, triangle closing) that would be impractical on traditional databases.

### 7. Fairness as a growth lever

LinkedIn's fairness work on PYMK (ensuring infrequent members get fair representation) isn't just ethical — it's good business. By preventing the "rich get richer" bias, they ensure that new and less-active members also receive connection invitations, improving retention for the long tail of users who might otherwise churn.

## Weaknesses and gaps

### 1. The 30,000 connection cap is artificial

Power users — recruiters, sales professionals, thought leaders — routinely hit the 30,000 connection limit. This forces them to curate connections (removing old ones to add new) or rely on the follow relationship, which provides less signal and no messaging access. The cap was likely designed to keep the graph manageable and prevent pure-numbers gaming, but it penalizes the platform's most valuable users.

### 2. Connection quality decay

LinkedIn has no mechanism to naturally age or depreciate connections. A connection made 15 years ago at a company you briefly worked at has the same graph weight as a current close colleague. This inflates 2nd-degree networks with irrelevant paths and degrades PYMK quality. There's no concept of "connection strength" visible to users (though LinkedIn likely models it internally).

### 3. PYMK can feel creepy

PYMK's accuracy — surfacing people from your phone contacts, email, or inferred from location data — sometimes crosses into uncomfortable territory. LinkedIn has faced criticism for importing phone contacts aggressively and for suggestions that seem to reveal private information (e.g., suggesting a therapist's other patients, or people you met at sensitive events).

### 4. The "open networker" (LION) problem

A significant subset of users (LinkedIn Open Networkers) accept all connection requests to maximize network size. This degrades the signal quality of the entire graph — 1st-degree connections stop meaning "we know each other" and instead mean "we're both on LinkedIn." LinkedIn has tried to discourage this with the 30K cap and acceptance-rate monitoring, but hasn't eliminated it.

### 5. Follow-button default confuses users

The 2024 switch to Follow as the default button confused many users who expected Connect. The two-action model (follow vs. connect) adds cognitive load. Users don't always understand why their "Connect" button disappeared or how to switch it back. The distinction between followers and connections is not intuitive.

### 6. No verified connections

LinkedIn can't verify that two people actually know each other professionally. The connection request says "I know this person" but there's no proof. This enables spam connections, fake relationship claims, and social engineering attacks. Competitors like Wellfound at least tie connections to verified co-founder/co-worker relationships.

### 7. Groups are vestigial

LinkedIn Groups were once a primary networking feature but have been neglected for years. They suffer from spam, low engagement, and poor moderation tools. The "free messaging within groups" perk creates perverse incentives (join groups just to spam message members). LinkedIn has invested minimally in groups while competitors like Slack, Discord, and Circle have built much better community infrastructure.

### 8. Gender and demographic bias in PYMK

Despite LinkedIn's LiFT toolkit, PYMK still shows demographic biases. Because the algorithm optimizes for acceptance probability, and because professional networks are often homophilous (people know people like themselves), PYMK can reinforce existing network segregation by gender, race, and socioeconomic status. In 2025, women reported noticeably lower content visibility after algorithm changes, suggesting structural bias persists.

### 9. Network size as vanity metric

The SSI score and various LinkedIn features encourage users to grow their network for its own sake. But a network of 10,000 weak ties is often less useful than 500 strong relationships. LinkedIn provides no tools for network health analysis, connection pruning recommendations, or relationship strength scoring.

## Competitive landscape

### X (formerly Twitter)
- **Follow-only model**: Purely unidirectional. No connection requests. Zero friction to follow anyone. This makes X faster for content consumption but weaker for professional identity verification.
- **DM access**: Configurable — users can open DMs to everyone, followers only, or verified accounts only. Simpler than LinkedIn's degree-gated messaging.
- **Professional identity**: X profiles are identity-light (no structured work history, skills, etc.). Professional credibility is earned through content quality and follower count, not connection density.
- **Network value**: X's network is content-centric (who produces interesting content) vs. LinkedIn's professional-centric (who do you work with). Neither captures relationship strength well.

### Facebook / Meta
- **Bidirectional friend model**: Similar to LinkedIn's connect, but for personal relationships. 5,000 friend limit (vs. LinkedIn's 30,000).
- **Follow for pages/public figures**: Unidirectional follow for public entities. Instagram's follow model is more like X.
- **Professional features**: Facebook has largely abandoned professional networking. Workplace (Meta's enterprise tool) was shut down in 2025.
- **Graph advantage**: Facebook's social graph is denser for personal relationships but nearly useless for professional ones.

### Wellfound (AngelList Talent)
- **Verified connections**: Connections tied to actual startup co-working relationships (co-founders, team members). Higher signal than LinkedIn's self-reported connections.
- **Smaller graph**: Much smaller network (~10M) but higher density within the startup ecosystem.
- **No degree system**: Flat access model — anyone can view anyone. Messaging requires mutual interest (like a dating app model for jobs).

### Professional platforms (Slack, Discord)
- **Community-first networking**: Connections form organically through shared community membership and conversation. Higher intent signal than LinkedIn's one-click connect.
- **No persistent graph**: Relationships exist within communities, not as a persistent global graph. You lose connections when you leave a community.
- **Real-time interaction**: Networking happens through actual conversation, not profile-browsing and connection-requesting.

### Polywork / specialized platforms
- **Richer relationship modeling**: Some platforms model relationship type (collaborator, mentor, investor) rather than just "connected."
- **Activity-based connections**: Connections formed through shared projects or contributions rather than requests.
- **Smaller scale**: None have achieved LinkedIn's network density.

## Relevance to agent platforms

### What transfers directly

**Degree-based access control**: Agents will need trust tiers for interacting with other agents. An agent you've deployed and configured (1st-degree) should have different access than an agent recommended by your agent (2nd-degree) or a completely unknown agent (out of network). This maps naturally to authorization and capability delegation.

**The connection request as a trust handshake**: When Agent A wants to collaborate with Agent B, there should be a bidirectional verification process — both parties (or their operators) agree to the relationship, establishing what capabilities are shared and what data access is granted. This is analogous to API key exchange or OAuth scoping but embedded in the social graph.

**Triangle closing for discovery**: "Agents used by teams that work with your team" is a powerful discovery signal. If Agent A integrates well with Agent B, and Agent B integrates well with Agent C, suggesting the A-C integration is valuable. This maps to composability discovery.

### What needs reimagining

**Connection strength should be computable**: Unlike human relationships, agent interactions produce exact data — API call frequency, data volume exchanged, success rates, latency. Connection strength between agents can be objectively measured, not inferred. This flips LinkedIn's biggest graph weakness into a strength.

**Dynamic, not static relationships**: Agent relationships should have TTLs, capability scoping, and automatic deprecation. An agent integration that hasn't been used in 6 months should decay in the graph, unlike LinkedIn connections that persist forever. Relationships should be contextual — Agent A trusts Agent B for translation tasks but not for code execution.

**Composability graph > social graph**: The key graph question for agents isn't "who knows whom" but "who works well with whom." The edges should carry performance metadata: latency, error rates, throughput, cost. PYMK becomes "Agents That Compose Well" — recommending integrations based on actual compatibility data, not just organizational proximity.

**Operator-mediated vs. autonomous connections**: Some agent relationships will be established by human operators (like deploying an agent pipeline). Others could be autonomous — Agent A discovering and connecting to Agent B based on capability matching. The platform needs to support both, with appropriate trust levels for each.

**No vanity metrics**: Unlike LinkedIn, where connection count is a status symbol, agent networks should optimize for utility. A tightly integrated network of 5 agents that reliably handle your workflows is more valuable than loose connections to 500 agents you never use.

### What's irrelevant

**Degree-based content distribution**: Agents don't consume feeds. Content distribution mechanics from LinkedIn's network graph don't apply.

**Human social dynamics**: Reciprocity bias ("I should connect back"), social pressure to accept, the awkwardness of removing connections — none of these apply to agent networks. This simplifies the design significantly.

**Demographic fairness in recommendations**: Agent recommendations should be purely capability-based. There are no demographic groups to protect. Fairness concerns shift to preventing monopolistic lock-in by dominant agent providers.

## Sources

### LinkedIn Engineering Blog
- [LIquid: The Soul of a New Graph Database, Part 1](https://www.linkedin.com/blog/engineering/graph-systems/liquid-the-soul-of-a-new-graph-database-part-1)
- [How LIquid Connects Everything So Our Members Can Do Anything](https://www.linkedin.com/blog/engineering/graph-systems/how-liquid-connects-everything-so-our-members-can-do-anything)
- [Building the Activity Graph, Part I](https://engineering.linkedin.com/blog/2017/06/building-the-activity-graph--part-i)
- [A Brief History of Scaling LinkedIn](https://engineering.linkedin.com/architecture/brief-history-scaling-linkedin)
- [Graph Infrastructure Team](https://engineering.linkedin.com/teams/data/data-infrastructure/graph)
- [Building a Heterogeneous Social Network Recommendation System](https://www.linkedin.com/blog/engineering/optimization/building-a-heterogeneous-social-network-recommendation-system)
- [Using the LinkedIn Fairness Toolkit in Large-Scale AI Systems](https://www.linkedin.com/blog/engineering/fairness/using-the-linkedin-fairness-toolkit-large-scale-ai)
- [Reinventing People You May Know at LinkedIn](https://www.linkedin.com/pulse/reinventing-people-you-may-know-linkedin-mitul-tiwari) — Mitul Tiwari

### Academic Papers
- [LiGNN: Graph Neural Networks at LinkedIn](https://arxiv.org/abs/2402.11139) — KDD 2024
- [How LinkedIn Economic Graph Bonds Information and Product](https://dl.acm.org/doi/10.1145/3219819.3219921) — KDD 2018
- [Talent Search and Recommendation Systems at LinkedIn](https://arxiv.org/pdf/1809.06481)
- [LinkSAGE: Optimizing Job Matching Using Graph Neural Networks](https://arxiv.org/html/2402.13430v1) — KDD 2025

### Analysis and Reporting
- [LinkedIn's Real-Time Graph Database Is LIquid](https://thenewstack.io/linkedins-real-time-graph-database-is-liquid/) — The New Stack
- [LinkedIn's LIquid Graph Database: Scaling Real-Time Data Access for 930+ Million Members](https://www.infoq.com/news/2023/06/linkedin-liquid-graph-database/) — InfoQ
- [The Scaling Journey of LinkedIn](https://blog.bytebytego.com/p/the-scaling-journey-of-linkedin) — ByteByteGo
- [Why LinkedIn Has No Competitors](https://growthcasestudies.com/p/linkedin-case-study) — Growth Case Studies
- [LinkedIn Says It Reduced Bias in Its Connection Suggestion Algorithm](https://venturebeat.com/business/linkedin-says-it-reduced-bias-in-its-connection-suggestion-algorithm) — VentureBeat
- [How LinkedIn's PYMK Algorithm Rewrites Itself Daily](https://techpreneurr.medium.com/how-linkedins-people-you-may-know-algorithm-rewrites-itself-daily-878efc500c98) — Medium

### LinkedIn Help Documentation
- [Follow and Connect on LinkedIn](https://www.linkedin.com/help/linkedin/answer/a702683)
- [Remove a Connection on LinkedIn](https://www.linkedin.com/help/linkedin/answer/a541617)
- [Block or Unblock a Member](https://www.linkedin.com/help/linkedin/answer/a1338373)

### Connection Limits and Mechanics
- [LinkedIn Connection Request Limit in 2026](https://www.joinvalley.co/blog/linkedin-invitation-limit-in-2025-weekly-limits-more)
- [LinkedIn Limits in 2026 (Complete Breakdown)](https://www.leadloft.com/blog/linkedin-limits)
- [LinkedIn Pending Connections Guide](https://www.linkedhelper.com/blog/linkedin-pending-connections/)
- [LinkedIn Connection Request Benchmarks 2025](https://www.alsona.com/blog/linkedin-connection-request-benchmarks-healthy-acceptance-rate-in-2025)
- [LinkedIn Alumni Tool Guide](https://www.linkedhelper.com/blog/linkedin-alumni-tool/)
- [What Happened to LinkedIn Creator Mode? (2026)](https://www.salesrobot.co/blogs/linkedin-creator-mode)
