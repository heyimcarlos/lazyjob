# Projects & Portfolio Showcase

## What it is
LinkedIn's portfolio and project showcase capabilities are a collection of profile features — Featured section, Projects section, media attachments, Articles/Newsletters, and Services pages — that allow professionals to display evidence of their work beyond the standard resume-format Experience section. Unlike dedicated portfolio platforms (Behance, Dribbble, GitHub), LinkedIn treats portfolio as a secondary layer on top of employment history rather than a first-class citizen. The result is a set of features that are functional but shallow, forcing most professionals to link out to dedicated platforms for substantive work showcase.

## How it works — User perspective

### Featured Section
The Featured section appears on personal profiles below the About section (repositioned lower in the 2024 layout restructure). It's hidden until a user manually adds at least one item.

**Adding content:** Click the "+" icon in the Featured section header. Choose from:
- LinkedIn posts you authored or reshared
- LinkedIn articles you published
- LinkedIn newsletters you publish
- External links (personal sites, portfolio URLs, GitHub repos, case studies)
- Uploaded images (JPEG, PNG, WEBP, HEIF/HEIC — GIFs accepted but only first frame renders)
- Uploaded documents (PDF, PPT/PPTX, DOC/DOCX, ODS, ODT, PPSX)
- Videos cannot be uploaded directly — must be published as a feed post first, then featured

**Display behavior:** Items appear as a horizontally scrollable card row. Desktop shows 2 items at once; mobile shows 1 (item 3 is only partially visible, the "2.5 Rule"). Each card shows thumbnail/preview, title, and description. Newest-featured items appear first by default; manual reordering via drag-and-drop requires at least 2 items.

**Limits:** Up to 400 items displayed. File size cap: 100 MB. Documents: 300 pages max, 1M words max. Images: 36 megapixels (one help article says 120MP — official docs are inconsistent).

**Visibility constraints:** Featured content is NOT visible on the fully public profile (non-logged-in view). Viewers must be signed into LinkedIn. Featured content is NOT indexed by LinkedIn search or external search engines. This fundamentally limits its discoverability.

**Removal:** Two options — "Remove from Featured" (content stays in Activity) vs. "Delete" (permanent, cannot be restored). No version history, drafts, or undo.

### Projects Section
Accessed via "Add profile section" → "Additional" → "Projects." This is a structured section predating Featured by several years.

**Fields per project:**
- Title (required)
- Description (up to 2,000 characters, rich text)
- Start/end dates (month + year precision only)
- URL (external link — **removed from UI in mid-2023**, API still accepts it)
- Team members (must be LinkedIn connections to appear in dropdown; must include self)
- Occupation link (ties project to an Experience position)

**Critical constraint:** Projects must be attached to an existing Experience or Education entry. You cannot add standalone projects — a frequently criticized limitation affecting freelancers, open-source contributors, and academics.

**The URL removal regression (mid-2023):** LinkedIn removed the Project URL field from the creation/editing UI without announcement. The field still exists in the underlying API/GraphQL schema. The only workaround is browser dev tools to inject the URL field via GraphQL mutation interception. LinkedIn's suggested alternative — "Add media, then Add a link" — produces a media card instead of a clean clickable URL. This disproportionately affects developers, designers, and researchers who link to GitHub repos, live demos, or papers.

### Media Attachments on Profile Sections
Sections supporting media: Experience, Education, Projects, Licenses & Certifications, Volunteer Experience. The About section previously supported media; this content was migrated to Featured when it launched in February 2020.

Supported types: images (JPEG, PNG, GIF static, HEIF, WEBP), documents (PDF, PPT, DOC, ODS, ODT, PPSX), and external URL links (rendered as OG preview cards). **Documents cannot be uploaded from mobile devices** — desktop only.

Document attachments render as scrollable inline viewers with page counters. Viewers can download attachments. Link attachments pull OG metadata for preview cards — users cannot override thumbnails without modifying OG tags on the external site.

### Services Section (Freelancers/Consultants)
A request/proposal marketplace at `linkedin.com/services`. Service providers create a Service Page listing services offered, work location, remote availability, optional hourly rate, and description. Clients submit project requests; providers respond with proposals.

**Free tier:** Basic Service Page, searchable on LinkedIn and Google, free messaging from prospective clients, basic description preview.

**Premium tier (Premium Business/Sales Navigator/Recruiter Lite):** Full "Services Showcase" with carousel media display, "Request services" button on profile/feed/search, customer ratings (4-5 star reviews), media uploads (documents, photos, videos, websites). Media uploads for Services Showcase are Premium-only.

**Permanent choice:** The decision of whether to manage Services through a Company Page vs. personal profile cannot be changed once set.

### Articles & Newsletters
**Articles:** Long-form content on LinkedIn's publishing platform. Full rich text formatting, inline images, video embeds via URL, code blocks (basic), member/org tagging. Custom cover image; video cover image added June 2025. Articles generate canonical URLs (`linkedin.com/pulse/...`) and are indexed by Google. Can be added to Featured section.

**Newsletters:** Serialized article collections. Opened to all members August 2025 (previously required Creator Mode or follower threshold). Subscribers receive push/in-app/email notifications per edition. Analytics added February 2025: email sends, open rates, impressions, reactions, comments.

**Critical limitations for both:**
- No subscriber email list export — audience belongs to LinkedIn, not the creator
- No A/B testing, scheduling, or subscriber segmentation
- No paywall or gating functionality
- No content export feature
- Organic reach declined ~65% from peak by Q3 2025
- Algorithm de-prioritizes articles vs. native short posts

### Collaborative Articles (Retired)
AI-generated topic prompts where invited experts contributed commentary. Community Top Voice badges (gold) recognized top 5% contributors per skill (max 2,500 per skill). **Fully retired by December 2024** — badges expired, no new contributions permitted, existing content is read-only. Retired due to gaming (non-experts earning badges) and AI-generated low-quality contributions. The separate blue Top Voice badge (editorial invitation only) continues.

## How it works — Technical perspective

### Profile Component Architecture
LinkedIn rebuilt profiles in 2022 using a configurable, server-driven component model. Components are defined as Rest.li union types where each alias represents a component type (Header, Prompt, Text, Entity, Tab-structured layouts). A "card" wraps an array of components. Most attributes are optional for configuration flexibility. Rendering logic is centralized server-side, enabling consistent cross-platform behavior. Results: 67% client-side code reduction, 4x cheaper to build new component experiences.

### Projects API
Endpoint: `POST/PATCH/DELETE https://api.linkedin.com/v2/people/id={personId}/projects`

Schema fields: `id` (auto-generated), `title` (MultiLocaleString, required), `description` (MultiLocaleRichText), `url` (string — still in API, removed from UI), `startMonthYear`/`endMonthYear` ({month, year}), `singleDate` (boolean), `members` (array of {memberId: URN, name: localized string}, required, must include self), `occupation` (position URN: `urn:li:position:(urn:li:person:{id},{positionId})`).

**No native media attachment field in the Projects schema.** Media linking is through the `url` field only (now hidden in UI) or via the separate media attachment flow.

Ordering controlled via `projectsOrder` field accepting arrays of item IDs. All Profile Edit API access requires `w_compliance` private permission — restricted to select LinkedIn partners, not available to general developers.

### Media/Digital Assets
URN format: `urn:li:digitalmediaAsset:{id}`. Metadata includes:
- `mediaTypeFamily`: STILLIMAGE, VIDEO, SOUND, PAGINATEDDOCUMENT, SLIDESHOWDOCUMENT, BINARYDATA, ARCHIVE
- `status`: ALLOWED, BLOCKED, ABANDONED
- Three-step upload: register upload (POST `?action=registerUpload`) → binary upload to returned URL → reference asset URN

Playable streams accessed via decoration: `~digitalmediaAsset:playableStreams` (public URLs) or `~digitalmediaAsset:privatePlayableStreams` (authenticated URLs).

### Featured Section — No Public API
There is no documented public endpoint to read or write the Featured section. The `memberRichContents` field in the Profile Edit API may reference Featured content but documentation is sparse and access requires private partner permissions. Featured section management is exclusively through the LinkedIn UI.

### Patent: Profile Personalization (US9817905B2)
Filed 2015, published 2017. A highlight module receives profile view requests, accesses viewer/member data, determines attributes most relevant to the specific viewer, and calculates scores using viewer-member alignment coefficients. Three attribute categories: Entity (shared employers, education, skills), Network (mutual connections, industry %), and Event (relevant posts, articles, opportunities). High-scoring attributes are presented as profile highlights. The profile appears differently to different viewers based on relevance scoring.

### Overall Patent Portfolio
LinkedIn holds 955 patents globally (894 unique families). ~1,085 active US patents. 791 patents acquired from IBM in 2015 doubled the portfolio. No patents found specifically covering Featured section implementation, media showcase, or work verification — this functionality is likely implemented as trade secret.

## What makes it successful

### The Featured section works as a "proof layer"
Despite its limitations, the Featured section serves an important function: it transforms the profile from a claims document into a hybrid of claims + evidence. A headline says "marketing strategist"; a Featured case study PDF shows it. This is LinkedIn's closest analog to proof-of-work.

### Project team tagging creates social proof
The Projects section's team member linking creates verifiable social proof — if four connected professionals all list the same project, it's implicitly validated. This is a lightweight verification mechanism that costs LinkedIn nothing to maintain.

### Services section solves a real pain point
For freelancers and consultants, the Services section provides a zero-cost way to be discoverable and receive project requests through a platform with 1B+ members. The "Request services" flow is simple and solves the "how do I find a freelancer I can trust" problem using LinkedIn's existing trust infrastructure (connections, endorsements, shared employers).

### Articles/Newsletters leverage existing distribution
Creating a newsletter on LinkedIn gives immediate access to a notification channel reaching all your connections/followers — something that takes months or years to build on Substack or Medium. The distribution subsidy (LinkedIn pushes your newsletter to followers via email) is the core value proposition.

### Revolving profile banner slideshow (December 2024)
Allows multiple images in a rotating banner at the profile header level — a lightweight way to showcase projects/achievements at the most visible profile real estate without requiring the viewer to scroll to the Featured section.

## Weaknesses and gaps

### Structural deficiencies

**No standalone projects.** Projects must be tied to Experience or Education entries. Independent consultants, open-source contributors, side project builders, and academics whose work doesn't map to a single employer cannot cleanly represent their work.

**Featured section is invisible to search.** Content in Featured is not indexed by LinkedIn's search engine or external search engines. A recruiter searching for candidates with specific portfolio work cannot find them through Featured content. This makes the feature a display case, not a discovery mechanism.

**Featured section hidden from public view.** Non-logged-in visitors cannot see Featured content. Rich media work samples are explicitly excluded from the public profile version. This limits the section's value for professionals sharing their LinkedIn profile as a portfolio URL.

**No layout customization.** All Featured items display as identical horizontal scrolling cards. No ability to group items (separate "writing" from "design" from "code"), customize card sizes, or create visual hierarchy. Choice paralysis reported with 10+ items creating cluttered, unfocused profiles.

### Regressions

**Project URL field removed (mid-2023).** A functional field linking projects to live URLs was silently removed from the UI. The only workaround requires browser dev tools and GraphQL interception. LinkedIn's suggested alternative (media attachment flow) produces inferior results.

**Collaborative Articles badge system killed (December 2024).** The only mechanism for earning expertise signals through content contribution was retired after widespread gaming. No replacement was introduced.

**Audio Events discontinued (December 2024).** A native broadcasting tool removed with no native replacement.

**About section moved above Featured (2024).** The Featured section was pushed further down the profile, reducing its visibility to casual viewers.

### Missing capabilities

**No analytics on Featured items.** Cannot track who clicked, viewed, or engaged with specific Featured content. Only general profile impression counts are available (Premium only).

**No native video in Featured.** Videos must be published as feed posts first, then featured — no direct upload path. GIFs render as static first-frame images.

**No inline document preview.** PDFs force download rather than inline preview. Documents display as thumbnails, not readable content.

**No custom thumbnails.** External URL link previews pull OG metadata automatically. Users cannot override thumbnails without modifying OG tags on the external site. Link preview images frequently fail to render or appear in mismatched aspect ratios.

**Mobile document upload blocked.** Documents cannot be uploaded from mobile devices — desktop only. Significant gap given LinkedIn's mobile usage growth.

**No content ownership.** Newsletter subscriber lists cannot be exported. Articles have no native export. Content created on LinkedIn stays on LinkedIn.

**No portfolio builder.** Users must piece together Featured + Experience + Projects + About sections manually. There is no guided portfolio creation flow or template system.

**No API access for portfolio aggregation.** The Profile Edit API (including projects) requires LinkedIn partner approval with `w_compliance` permission. No self-serve pathway for developers to build third-party portfolio aggregation tools. No public API for the Featured section at all.

## Competitive landscape

### Behance (Adobe)
50M+ users. Project-first: multi-page narrative case studies with full-bleed images, embedded video, and process documentation. Human-curated discovery (not purely algorithmic). ~90% of views from external traffic (Google SEO). Adobe Creative Cloud integration enables direct publishing from design tools. Pro tier ($9.99/month) adds analytics, password-protected projects, and custom domains. **2025 additions:** LinkedIn integration (verified credentials), Freelancer Dashboard with payment integration (Stripe/PayPal), AI-assisted hiring, and structured job posts. **2026 roadmap:** Full ATS layer for companies to find/evaluate/hire creative talent.

**What Behance does that LinkedIn cannot:** Full-bleed visual presentation, Google-indexed portfolio pages, human curation, integrated payment for freelance transactions.

### Dribbble
Shot-based format (single polished visual frame) vs. Behance's case study approach. "Instagram for designers." Pro ($8/month) includes marketplace (3.5% commission, waived for Pro), video pitch introductions, and priority in client recommendations. **2024 additions:** Free direct messaging, designer-client marketplace with transaction capability.

**What Dribbble does that LinkedIn cannot:** Authentic design critique culture, visual-first discovery, integrated freelance transactions.

### GitHub
100M+ developers. Profile README (fully customizable Markdown/HTML), pinned repos (up to 6), contribution graph (365-day activity heatmap), GitHub Pages (free hosting), GitHub Sponsors. 83% of technical hiring managers trust GitHub profiles more than traditional resumes (Beamery 2025). Recruiters search by language, star count, activity recency, and organization membership.

**GitHub Copilot's impact:** Writes ~46% of average developer code (up to 61% in Java). Shifting portfolio evaluation from "quantity of code" toward architecture decisions, code review quality, PR descriptions, and system design. GitHub portfolio's explanatory elements (READMEs, comments, reviews) now more important than raw commit count.

**What GitHub does that LinkedIn cannot:** Shows actual code quality, architecture decisions, collaboration patterns, open source contributions, and live deployments.

### Contra
Commission-free freelance platform. Profiles combine portfolio, services, and payment in one flow. "Portfolio Magic" (AI-generated portfolio presentations). "Indy AI" browser extension surfaces hidden freelance opportunities from LinkedIn/X networks. 0% commission on client payments (revenue from Pro subscription at $29/month). **August 2025:** Contra for Companies (enterprise hiring tools). Users rate 4.9/5 on G2.

**What Contra does that LinkedIn cannot:** Portfolio-to-hire-to-pay pipeline in a single flow with zero commission.

### Toptal
Only ~3% of applicants accepted through 5-week vetting. Portfolio functions as vetting evidence, not discovery mechanism. Profile Editors and Talent Coaches help accepted talent present work. Selectivity IS the product — clients pay premium for verified top talent.

### Upwork
Most utilitarian portfolio model. Freelancers with portfolios hired 9x more often. Job Success Score (derived from reviews, repeat hiring, contract completion) is the primary credibility signal, more than portfolio content itself. AI collaboration skills are fastest-growing category in 2025.

### Read.cv (Defunct)
Design-forward professional networking platform. Minimalist, craft-conscious UI. Personal website publishing via "Sites" feature with custom domains. **Acquired by Perplexity AI in January 2025; shut down May 2025.** The acquisition by an AI company (not a professional networking player) signals that LinkedIn alternative space is being explored as infrastructure for AI-native professional tools.

### Polywork
Focused on non-linear career paths and "doing multiple things." Profiles capture full-time job, side projects, consulting, open source, talks, publications simultaneously. AI-powered LinkedIn profile → personal website conversion. $44.5M in funding. Key insight: LinkedIn's Experience section forces work into rigid employment history that doesn't reflect modern professional reality.

### Peerlist
Tech-professional-specific portfolio-first network. GitHub integration (repos appear natively), Dribbble/Product Hunt integrations, verified workplace/education credentials, custom domain hosting, Project Launchpad for side project launches. Particularly strong for developers and builders who want integrated GitHub contribution graphs and side project launches.

### Medium vs. Substack
**Medium:** Algorithmic discovery, platform-owned audience, declining creator earnings. Good for establishing presence via algorithm reach. **Substack:** Creator-owned subscriber lists, direct monetization (creator sets prices, 10% platform fee), Notes (short-form microblogging), livestreaming (January 2025), recommendations (cross-promotion network). 32M new subscribers from in-app discovery in Q3-Q4 2025. A Substack with 5,000+ subscribers is a more compelling expertise signal than a LinkedIn article.

### Cross-platform behavior patterns
No single platform combines community discovery, portfolio depth, transaction capability, and employment networking. Professionals maintain 3-5 platforms simultaneously:

| Role | Hub | Discovery | Depth | Transaction |
|---|---|---|---|---|
| Designers | Personal site (Webflow) | Dribbble | Behance | Contra |
| Developers | GitHub profile + Pages | GitHub search | GitHub repos | Upwork/Toptal |
| Freelancers | Contra or personal site | LinkedIn inbound | Behance/GitHub | Contra/Upwork |
| Thought leaders | Substack | LinkedIn amplification | Substack archive | Direct consulting |

LinkedIn serves as the "employment credibility" and "recruiter discovery" layer across all roles but is rarely the primary portfolio for any specialized profession.

### Portfolio site builders
Webflow (visual builder generating real HTML/CSS/JS, signals design-to-implementation competency), Squarespace (16+ portfolio templates, mobile-first, popular with photographers/artists), Wix (most accessible, AI website builder from prompts), Notion (template ecosystem for text-heavy structured portfolios, services like Super.so convert to styled websites). Key insight: portfolio visual layer is commoditized — differentiators have shifted to content quality, narrative, and external social proof.

## Relevance to agent platforms

### What transfers directly

**Featured section as capability showcase.** The concept of a prominent showcase section displaying evidence of capabilities transfers directly. For agents, this becomes: sample task outputs, benchmark results, integration demos, and live capability previews. Unlike LinkedIn's static display, agent showcases can be interactive — "try this agent on a sample task."

**Project team tagging → Agent composition evidence.** The Projects section's team member linking maps to agent composition evidence — which agents worked together on which tasks, with what results. For agents, this data is automatically generated rather than manually entered.

**Services section → Agent service marketplace.** The Services request/proposal flow translates to task request/capability matching. But for agents, matching can be automated and verified in real-time rather than requiring manual proposals.

### What needs reimagining

**Proof-of-work is verifiable, not claimed.** LinkedIn's biggest portfolio weakness — self-reported claims with no verification — is an agent platform's biggest advantage. Agent portfolios should be auto-generated from actual task execution: success rates, latency percentiles, cost per task, error patterns, and real output samples with client consent. The Featured section equivalent fills itself.

**Portfolio as live system, not static showcase.** LinkedIn's Featured section is a curated display case of past work. Agent portfolios should be real-time dashboards: current availability, recent task performance, active capabilities vs. deprecated ones, version history. The portfolio is the monitoring interface.

**Discovery through capability, not keywords.** LinkedIn's Featured content isn't searchable. Agent portfolio content should be the primary search surface — find agents by demonstrated capability (not claimed skills), by output quality metrics, by composition compatibility. The separation between "profile" and "search" collapses.

**Composition portfolios.** LinkedIn has no concept of "these five people work well together." Agent platforms need first-class composition portfolios: pre-validated agent pipelines with performance data, showing which agents combine effectively for complex tasks. This is the agent-world equivalent of a team project page, but with verifiable performance metrics.

**Content ownership is irrelevant.** LinkedIn's newsletter subscriber ownership problem doesn't exist for agents. The relevant ownership question for agents is: who owns the fine-tuned model weights, the task execution data, and the performance history? These are the agent-world equivalents of "subscriber lists."

### What's irrelevant

**Visual portfolio presentation.** Behance-style full-bleed visual showcases are irrelevant for agents. Agent capability evidence is structured data (metrics, logs, output samples), not visual art. The presentation layer matters less than the data layer.

**Self-reported career narrative.** LinkedIn's Experience → Projects → Featured hierarchy tells a human career story. Agents don't have careers — they have capability manifests and performance histories. The narrative is the data.

**Endorsement-based social proof.** LinkedIn's endorsement and recommendation systems are irrelevant for agents. Agent social proof is objective: did the task succeed, at what cost, in what time? No endorsement needed.

## Sources

### LinkedIn Official Documentation
- [Featured Section FAQs — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a552452)
- [Manage Featured Samples — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a550399)
- [Add Profile Content to Featured — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a1513395)
- [Add Sections to Your Profile — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a540837)
- [Media File Types Supported — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a564109)
- [Media File Types on Profile — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a1516731)
- [LinkedIn Newsletters — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a522525)
- [Creator Mode Updates — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a5999182)
- [Offer Services on LinkedIn — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a569554)
- [Service Pages FAQs — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a569534)
- [Collaborative Articles FAQ — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a1443723)
- [Sections Missing from Public Profile — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a524256)
- [Community Top Voices — LinkedIn Help](https://www.linkedin.com/help/linkedin/answer/a6245087)

### LinkedIn API / Microsoft Learn
- [Profile API](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/profile-api)
- [Profile Edit API](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/profile-edit-api)
- [Profile Edit API — Projects](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/profile-edit-api/projects)
- [Profile Edit API — Patents](https://learn.microsoft.com/en-us/linkedin/shared/integrations/people/profile-edit-api/patents)
- [Documents API](https://learn.microsoft.com/en-us/linkedin/marketing/community-management/shares/documents-api?view=li-lms-2025-11)
- [Digital Media Asset Schema](https://learn.microsoft.com/en-us/linkedin/shared/references/v2/digital-media-asset)
- [URNs and IDs](https://learn.microsoft.com/en-us/linkedin/shared/api-guide/concepts/urns)

### LinkedIn Engineering Blog
- [Leveraging Configurable Components to Scale Profile Experience (2022)](https://www.linkedin.com/blog/engineering/profile/leveraging-configurable-components-to-scale-linkedin-s-profile-e)
- [Render Models at LinkedIn (2022)](https://www.linkedin.com/blog/engineering/product-design/render-models-at-linkedin)

### Patents
- [US9817905B2 — Profile Personalization Based on Viewer (Google Patents)](https://patents.google.com/patent/US9817905)
- [LinkedIn Patent Portfolio — TechInsights](https://www.techinsights.com/blog/linkedins-patent-portfolio-looking-hidden-gems)
- [LinkedIn Patents Stats — GreyB](https://insights.greyb.com/linkedin-patents/)

### Competitive Platforms
- [Behance MAX 2025 Updates](https://www.behance.net/blog/max-2025-behance-updates)
- [Behance MAX 2024 Updates](https://www.behance.net/blog/max-2024-behance-updates)
- [Behance Pro](https://www.behance.net/pro)
- [Behance-Adobe Portfolio Integration](https://help.myportfolio.com/hc/en-us/articles/360036128674)
- [Dribbble Hiring Updates May 2024](https://dribbble.com/stories/2024/05/31/big-news-from-dribbble)
- [Dribbble vs Behance 2024 — Giant Creates](https://giantcreates.com/design/dribbble-vs-behance-for-designers-in-2024/)
- [GitHub Docs: Profile README](https://docs.github.com/en/account-and-profile/how-tos/profile-customization/managing-your-profile-readme)
- [GitHub Docs: Pinned Items](https://docs.github.com/en/account-and-profile/how-tos/profile-customization/pinning-items-to-your-profile)
- [GitHub Recruiting — Fonzi.ai](https://fonzi.ai/blog/github-recruiting)
- [GitHub Copilot Statistics 2025 — Second Talent](https://www.secondtalent.com/resources/github-copilot-statistics/)
- [Perplexity Acquires Read.cv — TechCrunch](https://techcrunch.com/2025/01/17/perplexity-acquires-read-cv-a-social-media-platform-for-professionals/)
- [Peerlist Platform](https://peerlist.io/)
- [Polywork — Product Hunt](https://www.producthunt.com/products/polywork)
- [Contra 2025 — JoWorks](https://joworks.studio/blog/contra-the-commission-free-freelance-platform-changing-the-game-in-2025/)
- [Contra vs Dribbble vs Behance — Ruul](https://ruul.io/blog/contra-vs-dribbble-vs-behance)
- [Upwork Portfolio Guide](https://www.upwork.com/resources/portfolio-guide)
- [Toptal Review — Pi.tech](https://pi.tech/blog/toptal-review)
- [Substack vs Medium — Nick Wolny](https://nickwolny.com/substack-vs-medium/)
- [Substack Features 2025 — Women in Publishing](https://womeninpublishingsummit.com/substack-features/)
- [Notion Portfolio Templates](https://www.notion.com/templates/category/portfolio)

### Analysis & Commentary
- [LinkedIn Portfolio Guide 2026 — LinkedHelper](https://www.linkedhelper.com/blog/linkedin-portfolio/)
- [Featured Section Strategy — RankLN](https://rankln.com/blog/linkedin-featured-section-high-ticket-consulting)
- [Featured Section Deal — Narrativio](https://narrativio.com/en/so-whats-the-deal-with-the-featured-section-on-linkedin/)
- [LinkedIn Feature Releases 2024-2025 — Rafal Szymanski](https://rafalszymanski.pl/en/blog/linkedin-feature-releases-2024-2025/)
- [LinkedIn New Features 2026 — LinkedFusion](https://www.linkedfusion.io/blogs/linkedin-new-features-and-updates/)
- [LinkedIn Updates 2026 — SocialBee](https://socialbee.com/blog/linkedin-updates/)
- [LinkedIn Updates 2026 — HeyOrca](https://www.heyorca.com/blog/linkedin-social-news)
- [Bringing Back Project URL — Yicheng Xia](https://yichengxia.github.io/articles/202312/bringing-back-project-url-on-linkedin.html)
- [LinkedIn Removes Top Voice Badges — Social Media Today](https://www.socialmediatoday.com/news/linkedins-removing-top-voice-badges-collaborative-articles/728247/)
- [LinkedIn Phases Out Audio Events — Social Media Today](https://www.socialmediatoday.com/news/linkedins-phasing-dedicated-live-audio-events/733669/)
- [LinkedIn Video Covers for Articles — Social Media Today](https://www.socialmediatoday.com/news/linkedin-adds-video-header-images-articles-newsletters/751813/)
- [AI Impact on Front-End Hiring 2025](https://geekvibesnation.com/how-ai-tools-like-github-copilot-are-reshaping-front-end-hiring-in-2025/)
- [LinkedIn is Broken — Duperrin](https://www.duperrin.com/english/2025/03/07/linkedin-broken/)
- [Collaborative Articles Critique — Fortune](https://fortune.com/2024/04/18/linkedin-microsoft-collaborative-articles-generative-ai-feedback-loop-user-backlash/)
- [LinkedIn Limits 2026 — Wandify](https://wandify.io/blog/sourcing/linkedin-limits-in-2026-complete-guide/)
- [LinkedIn Alternatives 2026 — MagicPost](https://magicpost.in/blog/linkedin-alternatives)
