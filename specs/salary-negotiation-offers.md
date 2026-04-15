# Salary Negotiation and Offer Evaluation

## The Reality Today

### The Negotiation Gap
Research from Payscale (31,000 respondents) and career negotiation studies establishes:
- **40-50% of candidates who negotiate receive a better offer**
- Negotiation typically yields **5-15% increase in base salary**
- Not negotiating can cost **hundreds of thousands of dollars over a career**
- Top performers who negotiate strategically see **10-30% total comp increases**

However, significant gaps exist:
- **Women are less likely to initiate negotiations** — and when they do, often achieve lower outcomes
- **Research by Babcock et al.** found negotiation training for women resulted in 18-20% average increases
- Social backlash against women who negotiate aggressively is well-documented

### The Total Comp Blind Spot
Most candidates focus on **base salary alone** — ignoring 20-40% of their actual compensation:
- **RSU/Equity**: Often 10-20% of total comp at senior levels in tech
- **Signing Bonus**: One-time payments, often negotiable
- **Annual Bonus**: Typically 10-20% of base for tech roles
- **Benefits**: Health, 401k matching, PTO, etc.

**The proper negotiating unit is total compensation, not base salary.**

### How Offers Actually Break Down (Tech)

**Public Companies (FAANG, established tech):**
- Base + RSU (4-year vest with 1-year cliff) + signing + annual bonus
- RSUs valued at current stock price — liquid upon vesting
- Typically 10-20% of total comp at senior levels

**Pre-IPO Companies:**
- RSUs at 409A fair market value
- Illiquid until IPO or acquisition — higher risk
- Often includes significant overhang from existing equity

**Startups:**
- Options with strike price vs. fair market value
- Liquidation preferences determine actual value
- 10-year expiration on options — clock starts at grant

### The Offer Evaluation Problem
Evaluating multi-year offers requires:
1. Annualizing equity (total grant / vest years)
2. Understanding cliff periods and acceleration provisions
3. Risk-adjusting for company stage (public = liquid, private = illiquid)
4. Factoring in refresh grants vs. one-time awards
5. Comparing benefits packages meaningfully

**Most candidates cannot do this math accurately** — they default to comparing base salary only.

### Competing Offers: The Ultimate Lever
- Using competing offers as leverage is **standard practice in tech**
- You do NOT need to show documentation — verbal confirmation is typically sufficient
- Present competing offers as context for your market value
- The key is communicating alternatives, not revealing details
- **Multiple offers dramatically increase leverage** — companies accelerate when they know they have competition

### The Negotiation Process (Big Tech)
1. Receive initial offer (often lowballed 10-15% expecting negotiation)
2. Request time to review (always — never accept on the spot)
3. Research market rates via levels.fyi, Blind, Glassdoor
4. Identify your walkaway point and target
5. Make counter-offer focused on total comp
6. Typically 1-2 rounds of back-and-forth
7. Final answer within 24-48 hours of final offer

**Failure modes:**
- Accepting immediately (leaving money on table)
- Asking without market data justification
- Lying about competing offers (companies verify)
- Being inflexible about what matters
- 3+ counter-offer rounds (becomes problematic)
- Not knowing your minimum

---

## What Tools and Products Exist

### Compensation Data Platforms

**Levels.fyi** (Dominant in tech)
- ~3M monthly users, crowdsourced verified salary data
- Company-specific breakdowns by level (L3-L7, IC1-IC5, etc.)
- Includes base, equity, signing, bonus for FAANG, Microsoft, Amazon, Apple, startups
- Claims: "We never accept payment to adjust leveling or salary numbers"
- Limitations: Anonymous unverified submissions weighted differently; no rigorous academic validation of methodology
- Cost: Free for job seekers

**Glassdoor**
- Self-reported salary data from employees
- Smaller sample sizes for niche tech roles
- Includes reviews, interview insights, company culture
- Less accurate for specialized tech roles

**Blind**
- Anonymous employee reports from tech workers
- Smaller dataset but often more detailed
- Good for recent offers and negotiation tactics
- Fully anonymous — less validated

**Payscale**
- Survey-based data from millions of respondents
- More applicable to non-tech roles
- Salary comparison and negotiation guides

**Salary.com**
- Job pricing tools and market baselines
- More enterprise/HR-facing

**Key Gap**: No comprehensive tool that takes a full offer letter (all components) and calculates risk-adjusted total comp value across time.

### Negotiation Coaching

**What's available:**
- Levels.fyi blog has negotiation guides (human-written)
- Human coaches via同城, Exponent, career coaches
- Resume review services (human)
- No AI-powered counter-offer drafting tools exist

**What's missing:**
- No AI chatbot that drafts counter-offer letters
- No real-time negotiation coaching during actual negotiations
- No tool that evaluates equity valuation (startup options especially)
- No comprehensive offer comparison tool

### Offer Evaluation Tools

**levels.fyi calculator**: Calculates percentiles for base+equity+bonus
**Hired.com**: AI-powered job matching with salary insights
**Beamery, Textio**: Enterprise talent platforms with compensation insights (B2B)

**Critical Gap**: No tool handles multi-year, multi-component offer comparison with risk adjustment for private company equity.

---

## The Agentic Opportunity

### Level 1: Market Intelligence Agent (Near-term, Table Stakes)

**What it does:**
- Continuously monitors compensation data across levels.fyi, Blind, Glassdoor for target companies/roles
- Alerts when bands change, when new data appears, when your target role's comp shifts
- Maintains personalized market rate for your profile (level, yoe, skills, location)

**Inputs needed:**
- Target companies list
- Current role/level and desired role/level
- Location preferences
- Skills/tech stack

**Outputs:**
- Real-time market rate intelligence
- Alert when your current offer is below market
- Alert when comp bands increase for roles you're targeting

**Failure modes:**
- Data is only as good as crowdsourced inputs (outdated, biased samples)
- Can't account for your specific experience vs. market average
- Privacy concerns with sharing exact comp

### Level 2: Offer Evaluation Engine (Near-term, High Value)

**What it does:**
- Takes full offer letter JSON (base, equity grant, strike price, vest schedule, signing, bonus, benefits)
- Calculates annualized value of each component
- Risk-adjusts for company stage (public = 100% value, private = discount for illiquidity)
- Compares across competing offers
- Generates valuation report

**Inputs:**
- Full offer details for all competing offers
- Company stage/risk assessment
- Your personal risk tolerance

**Outputs:**
- "Offer A is worth $X/year vs Offer B at $Y/year"
- Breakdown by component
- Risk-adjusted comparison
- Gap analysis vs. market rate

**Example calculation:**
```
Offer A: $200K base + $100K RSU (4yr) + $30K signing + $20K bonus
Annualized: $200K + $25K + $30K (signing amortized) + $20K = $275K

Offer B: $180K base + $200K RSUs (4yr, front-loaded) + $50K signing + $15K bonus
Annualized: $180K + $50K + $12.5K + $15K = $257.5K

But Offer B is pre-IPO with 409A at $10/share, current FMV $25, options expire in 10y...
```

**Failure modes:**
- Requires complete information from candidate (often don't have full details)
- Private company valuation is inherently speculative
- Doesn't capture non-monetary factors (growth, culture, WLB)

### Level 3: Negotiation Strategy and Counter-Offer Drafting (Medium-term, Differentiated)

**What it does:**
- Generates counter-offer letter based on: target total comp, market data, competing offers, company's known constraints
- Provides talking points for phone negotiation
- Suggests what's negotiable vs. not (equity refresh vs. base vs. signing)
- Coaches on phrasing ("I appreciate the offer, I was expecting closer to X based on...")
- Reminds of best practices (don't accept on spot, get everything in writing)

**Inputs:**
- Initial offer details
- Target total comp
- Competing offers (if any)
- Your priorities (base vs. equity vs. signing vs. start date)
- Company's known compensation philosophy

**Outputs:**
- Draft counter-offer email
- Phone script with talking points
- FAQ responses for common objections
- "What to say next" guidance

**Critical constraint**: Human must review, edit, and send. Agent drafts, human owns.

**Failure modes:**
- Counter-offer that's too aggressive damages relationship
- Fabricating competing offers (companies verify)
- Draft sounds AI-generated (detectable)
- Agent doesn't understand non-obvious constraints (budget, band, internal equity)

### Level 4: End-to-End Negotiation Coach (Longer-term)

**What it does:**
- Monitors email/calendar for offer discussions
- Provides real-time coaching during phone negotiations (voice interface)
- Tracks negotiation history and outcomes
- Learns what works for your profile/companies
- Coordinates across multiple offers in parallel

**Inputs:**
- Email access (for written negotiations)
- Calendar access (for scheduling)
- Voice interface during calls
- Full context on all active negotiations

**Outputs:**
- Real-time suggestions during calls
- Follow-up reminders
- Multi-offer timeline coordination
- "You have leverage here" signals

**Failure modes:**
- Privacy implications of email/calendar access
- Real-time voice coaching is technically hard (latency, context)
- Candidates may become over-reliant on agent
- Companies may detect coaching if responses are too polished

---

## Technical Considerations

### Data Access
- **levels.fyi**: No public API; scraping is technically possible but ToS risk
- **Blind, Glassdoor**: No APIs; scraping difficult
- **LinkedIn Salary**: Limited, only visible to job seekers on specific job pages
- **Candidate-provided**: Most reliable data is what the candidate enters themselves

### API Landscape
- No major ATS (Greenhouse, Lever, Workday) exposes candidate-facing salary negotiation APIs
- Beamery/Textio are enterprise B2B platforms
- **No clean API path for compensation intelligence**

### Privacy
- Sharing exact offers with any third party (including our agent) has privacy implications
- Offer details are often confidential — sharing with an AI agent may violate offer terms
- Data minimization: agent should forget offer details after session unless explicitly stored

### Legal Constraints
- Pay transparency laws vary by state (CA, NY, CO, WA require salary ranges on postings)
- Employers cannot retaliate against candidates for discussing compensation
- Candidates can legally discuss their offers with others
- No laws prevent using AI to help evaluate/negotiate

### Equity Valuation Complexity
- RSU valuation is straightforward for public companies
- Options at private companies require 409A valuation + black-scholes for expected value
- Startup equity is highly speculative — requires understanding liquidation preferences, preferred vs. common, dilution
- No tool does this accurately for early-stage startups

---

## Open Questions

1. **Data accuracy**: How reliable is crowdsourced salary data? What's the actual sample size per role/company?

2. **Counter-offer drafting**: Can AI draft genuinely compelling counter-offers that don't sound AI-generated? What's the detection risk?

3. **Private company equity**: Can we build a reasonable expected value calculator for startup equity? What discount rate for illiquidity is appropriate?

4. **Multi-offer coordination**: How should an agent handle 3-4 simultaneous offers with different timelines? What's the optimal sequencing?

5. **Gender dynamics**: How should the agent account for research on gender and negotiation? Should it coach differently based on candidate demographics?

6. **When to walk away**: How does the agent help a candidate recognize when an offer is genuinely below market and not worth negotiating vs. worth fighting for?

7. **Offer acceptance**: When is the right time to accept and stop negotiating? How does the agent help with the psychology of "good enough"?

8. **Post-offer negotiation**: Can the agent help negotiate AFTER acceptance (counter-offer on counter-offer, reneging ethically)?

9. **Retention counter-offers**: When a candidate has an existing offer rescinded or gets a competing offer, how does the agent help with the current employer counter-offer dynamic?

10. **Ethical boundaries**: Should the agent help candidates who are "salary negotiable" by being too aggressive or misrepresenting their situation?

---

## Sources

- [Levels.fyi Blog - Negotiation](https://www.levels.fyi/blog)
- [Levels.fyi About](https://www.levels.fyi/about)
- [Payscale Salary Negotiation Guide](https://www.payscale.com/salary-negotiation-guide/)
- [Harvard PON - Women and Negotiation](https://www.pon.harvard.edu/)
- [NLRB - Employee Rights to Discuss Compensation](https://www.nlrb.gov/about-nlrb/rights-we-protect/your-rights/employee-rights)
- [Hired.com State of Salary Negotiation](https://www.lhh.com/us/en/hired/)
- [Wikipedia - Pay Transparency](https://en.wikipedia.org/wiki/Pay_transparency)
- Babcock, Linda, et al. "Women Don't Ask: Negotiation and the Gender Divide" - research on negotiation training outcomes
