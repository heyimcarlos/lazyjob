# Spec: SaaS — Pricing Strategy

**JTBD**: Access premium AI features without complex setup; Understand the product's business model
**Topic**: Define the LazyJob subscription tier structure, free tier limits, pricing rationale, and feature gates
**Domain**: saas

---

## What

LazyJob's SaaS pricing follows three tiers (Free, Pro, Team) with transparent, usage-based limits. The pricing model is inspired by LinkedIn's intent-based paywall timing and agent platform best practices: gate premium features at moments of demonstrated need, not at the point of entry. The free tier is a functional, useful product — not a trial with an expiration date. Paid tiers unlock convenience, scale, and team features.

## Why

The competitive landscape (Huntr, Teal, LazyApply) offers either free-limited or subscription models with no clear value differentiation. LazyJob's pricing should:
- **Be transparent**: Publish prices upfront (unlike LinkedIn's opaque enterprise pricing)
- **Align with value**: Users pay for outcomes (successful applications, interview prep quality), not just access
- **Respect the local-first brand**: Free tier should feel complete, not crippled
- **Avoid artificial scarcity**: Don't manufacture limits to force upgrades — charge for genuinely expensive operations

LinkedIn's InMail credit-back model (refund on response) maps well to LazyJob: charge for premium AI features, refund if the feature fails or returns below quality threshold.

## How

### Subscription Tiers

| Feature | Free | Pro | Team |
|---------|------|-----|------|
| **Local SQLite** | Yes | Yes | Yes |
| **Cloud Sync** | - | Yes | Yes |
| **Job Applications** | 20/month | Unlimited | Unlimited |
| **Ralph Loops** | 10/day | Unlimited | Unlimited |
| **Resume Tailoring** | 3/month | Unlimited | Unlimited |
| **Cover Letter Generation** | - | Unlimited | Unlimited |
| **Interview Prep** | 3/month | Unlimited | Unlimited |
| **Salary Negotiation** | - | Unlimited | Unlimited |
| **Team Collaboration** | - | - | Yes |
| **API Access** | - | - | Yes |
| **SSO** | - | - | Yes |
| **SLA** | - | Best effort | 99.9% |

### Free Tier Limits

The free tier is genuinely useful — not a time-limited trial. Limits are set to:
- Cover a full job search sprint (20 applications = ~1 week of active applying)
- Enable evaluation of premium features (3 resume tailoring requests = enough to see the value)
- Prevent abuse (10 Ralph loops/day = ~3 full discovery runs, prevents scraper-style usage)

**Free tier mechanics:**
- Counter resets monthly (calendar month, not rolling 30 days)
- User receives in-app notification when approaching limit: "You've used 15/20 applications this month"
- At limit: "Upgrade to Pro to continue applying" — upgrade prompt is non-intrusive, appears once
- No hard block on viewing, exploring, or organizing — only on actions that consume LLM compute

### Pro Tier ($15/month or $120/year)

Pro unlocks unlimited AI-powered features. Priced at $15/month (same as Huntr/Teal Pro) to:
- Signal quality at the same price point as proven competitors
- Leave room for annual discount (~$10/month)
- Be affordable for job seekers who land a $80K+ role with 2 weeks saved

**Value justification:**
- Average job seeker spends 15-30 min/application on tailoring. Unlimited tailoring saves 5-15 hours/month at Pro tier.
- Interview prep AI generates questions + feedback — saves 3-5 hours of self-preparation per interview
- Net time savings alone justify $15/month at any salary above $50K

### Team Tier ($49/month per seat)

Team is for career coaches, job search groups, and small recruiting teams. Priced at ~3x Pro to:
- Cover the multi-user infrastructure cost
- Leave room for group discounts at 10+ seats
- Be cheaper than individual subscriptions for 5+ person groups

**Team features:**
- Shared job search workspace (team members see each other's applications)
- Collaborative application review
- Team analytics (which roles are getting traction, response rate trends)
- Admin dashboard for team leads

### Enterprise (Custom pricing, not in MVP)

Enterprise is for recruiting teams at companies. Custom pricing based on:
- Number of seats
- API call volume
- SSO + compliance requirements

Not in MVP scope — phase 2 product.

### Free Tier Revenue Recovery

Free tier costs LazyJob money (LLM calls are not free). Revenue recovery comes from:
1. **Conversion rate**: ~5-10% of free users convert to Pro within 60 days of active job search
2. **Network effects**: Team members on Pro invite colleagues on Free → virality
3. **Data value**: Aggregated, anonymized job search trend data (e.g., "Engineers in SF see 40% more response in April") has value for compensation research — not sold, but informs product decisions

### Feature Gating Implementation

```rust
// lazyjob-core/src/billing/tier.rs

pub enum SubscriptionTier {
    Free,
    Pro,
    Team,
    Enterprise,
}

pub struct FeatureGate {
    tier: SubscriptionTier,
    monthly_counters: MonthlyCounters,
}

pub struct MonthlyCounters {
    pub applications_used: usize,
    pub applications_limit: usize,
    pub resume_tailoring_used: usize,
    pub resume_tailoring_limit: usize,
    pub ralph_loops_used: usize,
    pub ralph_loops_limit: usize,
}

impl SubscriptionTier {
    pub fn limits(&self) -> MonthlyCounters {
        match self {
            SubscriptionTier::Free => MonthlyCounters {
                applications_limit: 20,
                resume_tailoring_limit: 3,
                ralph_loops_limit: 10 * 30, // daily, shown as monthly
                ..Default::default()
            },
            SubscriptionTier::Pro => MonthlyCounters {
                applications_limit: usize::MAX,
                resume_tailoring_limit: usize::MAX,
                ralph_loops_limit: usize::MAX,
                ..Default::default()
            },
            SubscriptionTier::Team => MonthlyCounters {
                applications_limit: usize::MAX,
                resume_tailoring_limit: usize::MAX,
                ralph_loops_limit: usize::MAX,
                ..Default::default()
            },
            SubscriptionTier::Enterprise => MonthlyCounters {
                applications_limit: usize::MAX,
                resume_tailoring_limit: usize::MAX,
                ralph_loops_limit: usize::MAX,
                ..Default::default()
            },
        }
    }
}

impl FeatureGate {
    pub fn check(&self, feature: Feature) -> Result<()> {
        match feature {
            Feature::SubmitApplication => {
                if self.monthly_counters.applications_used >= self.monthly_counters.applications_limit {
                    return Err(Error::FeatureLimitExceeded { feature, limit: self.monthly_counters.applications_limit });
                }
            }
            Feature::ResumeTailoring => {
                if self.monthly_counters.resume_tailoring_used >= self.monthly_counters.resume_tailoring_limit {
                    return Err(Error::FeatureLimitExceeded { feature, limit: self.monthly_counters.resume_tailoring_limit });
                }
            }
            Feature::CoverLetterGeneration => {
                if self.tier == SubscriptionTier::Free {
                    return Err(Error::TierRequired { required: SubscriptionTier::Pro });
                }
            }
            // ...
        }
        Ok(())
    }

    pub fn record_usage(&mut self, feature: Feature) {
        match feature {
            Feature::SubmitApplication => self.monthly_counters.applications_used += 1,
            Feature::ResumeTailoring => self.monthly_counters.resume_tailoring_used += 1,
            // ...
        }
    }
}
```

### Intent-Based Paywall Timing

Inspired by LinkedIn Premium's insight that paywalls hit at moments of highest intent:

| Moment | Feature | Gating |
|--------|---------|--------|
| User clicks "Apply" for the 21st time | Application submission | "Upgrade to Pro for unlimited applications" |
| User requests the 4th resume tailoring | Resume tailoring | "3/3 free tailoring used. Unlock unlimited for $15/mo" |
| User completes 3 mock interviews | Mock interview | "Upgrade to continue with AI interview prep" |
| User tries to generate a cover letter | Cover letter | "Cover letters are a Pro feature. Start your free trial" |
| User sets up team workspace | Team features | "Team collaboration is a Team feature" |

**Key difference from LinkedIn**: Instead of blocking immediately, show the feature with a watermark ("Pro") and an upgrade prompt. The user can still see the output preview, but cannot export or use it without upgrading. This demonstrates value before demanding payment.

## Open Questions

- **Annual discount pricing**: LinkedIn offers ~33% annual discount ($19.99/mo annual vs $29.99/mo monthly). Should LazyJob match this? The spec suggests $120/year ($10/month equivalent). This is consistent with Teal/Huntr annual pricing.
- **Per-feature credit-back model**: Should LazyJob refund LLM costs for features that fail or return hallucinated content? The spec suggests yes for counter-offer drafts (where fabrication is the highest risk). Implement as `CreditBack` events triggered by user-reported failures.

## Implementation Tasks

- [ ] Define `SubscriptionTier` enum and `FeatureGate` struct in `lazyjob-core/src/billing/tier.rs`
- [ ] Implement `MonthlyCounters` with reset logic (check `created_at` vs current month, reset if new month)
- [ ] Implement `FeatureGate::check()` and `FeatureGate::record_usage()` for all gated features
- [ ] Add `[billing]` section to `lazyjob.toml`: `tier = "free" | "pro" | "team"`, subscription key for SaaS mode
- [ ] Implement `FeatureLimitExceeded` error variant in `lazyjob-core/src/error.rs` with upgrade URL
- [ ] Wire `FeatureGate::check()` calls in all ralph loop entry points (discovery, tailoring, cover letter, interview prep)
- [ ] Add in-app upgrade prompt component in TUI — appears once when user hits 80% of free tier limit
- [ ] Implement `lazyjob billing usage` CLI command showing current month usage and limits
- [ ] Add `credit_back` events to usage tracking for per-feature refund tracking
