# Spec: Multi-Offer Comparison UI

## Context

When a candidate receives multiple offers, they need to compare them side-by-side to make informed decisions. This spec addresses the comparison UI and underlying calculations.

## Motivation

- **High-stakes decision**: Wrong choice affects career and life
- **Time pressure**: Offers have expiration dates
- **Complexity**: Equity, benefits, salary don't compare trivially

## Design

### Offer Data Structure

```rust
pub struct OfferComparison {
    pub offers: Vec<ComparableOffer>,
    pub user_priorities: Vec<(PriorityFactor, f32)>,  // Factor -> weight
}

pub struct ComparableOffer {
    pub application_id: ApplicationId,
    pub company: String,
    pub role: String,
    pub base_salary: Money,
    pub signing_bonus: Money,
    pub annual_bonus: Money,         // Year 1 expected
    pub equity: EquityOffer,
    pub benefits: BenefitsValue,
    pub start_date: Date,
    pub expiry_date: Date,
    pub remote_policy: RemotePolicy,
    pub notes: String,
}

pub struct EquityOffer {
    pub grant_type: GrantType,       // RSU or Option
    pub shares: u64,
    pub strike_price: Option<Money>, // For options
    pub current_price: Option<Money>,
    pub vest_schedule: VestSchedule, // 4 year with 1 year cliff
    pub fmv_409a: Option<Money>,     // 409A valuation
}

pub struct BenefitsValue {
    pub health_insurance_annual: Money,     // Companypaid premium
    pub 401k_match_annual: Money,
    pub pto_days: u8,
    pub sick_days: u8,
    pub parental_leave_weeks: u8,
    // ... more as needed
}

pub struct Money {
    pub amount: f64,
    pub currency: Currency,
    pub annualization: Annualization,
}

pub enum Annualization {
    PerYear,
    Hourly { hours_per_week: f32 },
    OneTime,
}
```

### Total Compensation Calculation

```rust
impl ComparableOffer {
    pub fn total_comp_year_1(&self) -> Money {
        let base = self.base_salary;
        let bonus = self.annual_bonus;
        let signing = self.signing_bonus;
        let equity = self.equity.annual_value();  // See below
        let benefits = self.benefits.annual_value();
        
        Money {
            amount: base.amount + bonus.amount + signing.amount + equity + benefits,
            currency: base.currency,
            annualization: Annualization::PerYear,
        }
    }
    
    pub fn equity_annual_value(&self) -> f64 {
        match self.equity.grant_type {
            GrantType::RSU => {
                self.equity.shares as f64 * self.equity.current_price.unwrap_or(self.equity.fm_409a.unwrap()).amount
            }
            GrantType::Option => {
                // Black-Scholes for options (see XX-startup-equity-valuation.md)
                let intrinsic = (self.equity.current_price.unwrap().amount - self.equity.strike_price.unwrap().amount).max(0.0);
                let time_value = black_scholes_value(/* ... */);
                self.equity.shares as f64 * time_value
            }
        }
    }
}
```

### Comparison Table

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Offer Comparison                                           [Sort by: Total] │
├─────────────────────────────────────────────────────────────────────────────┤
│                              Company A      Company B      Company C        │
│                              Stripe         Meta           Anthropic        │
│                              Sr SWE          Sr SWE         Staff SWE       │
├─────────────────────────────────────────────────────────────────────────────┤
│  Base Salary               $185,000        $175,000        $195,000         │
│  Annual Bonus               $18,500          $8,750         $19,500         │
│  Signing Bonus              $25,000              -              -            │
│  Equity (annual value)     $45,000         $60,000         $35,000          │
│  ─────────────────────────────────────────────────────────────────────────  │
│  Total Cash Year 1        $273,500        $243,750        $249,500          │
│  Benefits Value            $25,000         $22,000         $28,000          │
│  ─────────────────────────────────────────────────────────────────────────  │
│  TOTAL COMP YEAR 1        $298,500        $265,750        $277,500          │
├─────────────────────────────────────────────────────────────────────────────┤
│  Equity Grant               5,000 RSUs     3,000 RSUs     10,000 options    │
│  Vest Schedule              4 yr / 1 yr    4 yr / 1 yr     4 yr / 1 yr       │
│  409A FMV                   $300            $200             N/A             │
├─────────────────────────────────────────────────────────────────────────────┤
│  Remote Policy              Hybrid          On-site         Remote OK      │
│  Start Date                Jun 1           Jul 15          Jun 15          │
│  Offer Expires             Apr 20          Apr 25          Apr 30          │
├─────────────────────────────────────────────────────────────────────────────┤
│  [Add to Negotiation]         ✓               -                -           │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Weighted Comparison

User ranks factors (salary, equity, remote, growth):

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Compare By My Priorities                                                  │
│                                                                             │
│  Priority Weights:                                                         │
│    Salary     [████████░░] 80%                                              │
│    Equity     [██░░░░░░░░] 20%                                              │
│    Remote     [░░░░░░░░░░] 0% (disabled)                                    │
│                                                                             │
│  Weighted Scores:                                                          │
│    Company A (Stripe):     $298,500 × 0.8 + $273,500 × 0.2 = $293,500      │
│    Company B (Meta):       $265,750 × 0.8 + $243,750 × 0.2 = $261,250      │
│    Company C (Anthropic):  $277,500 × 0.8 + $249,500 × 0.2 = $271,900      │
│                                                                             │
│  Recommended: Company A (Stripe) - highest weighted score                  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Negotiation Scenario Modeling

```rust
pub struct NegotiationScenario {
    pub base_offer: ComparableOffer,
    pub counter_amount: Money,
    pub counter_type: CounterType,  // Base, Signing, Equity, Bonus
    pub scenario: String,           // "If you get 10K more base from Stripe..."
}

impl NegotiationScenario {
    pub fn compare_with(&self, other: &ComparableOffer) -> ComparisonResult {
        // Calculate what the counter would make total comp
        // Compare with other offer
    }
}
```

### Expiration Tracking

Urgent warnings when offers expire soon:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ⚠️ Stripe offer expires in 3 days (Apr 20)                                  │
│                                                                             │
│  You have another offer from Meta expiring Apr 25.                          │
│                                                                             │
│  [Negotiate Extension]  [Decline & Move Forward]  [Compare Again]           │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Implementation Notes

- TUI view with keyboard navigation
- Data exportable as JSON/CSV for reference
- Offer expiry checks run on app startup
- Link to salary negotiation service for counter drafting

## Open Questions

1. **Multi-year comparison**: Should Year 2/3/4 total comp be shown?
2. **Equity risk adjustment**: Late-stage startup equity worth less
3. **Tax estimation**: Show pre-tax vs post-tax?

## Related Specs

- `salary-market-intelligence.md` - Salary comparison
- `salary-counter-offer-drafting.md` - Counter-offer drafting
- `application-workflow-actions.md` - Offer received workflow