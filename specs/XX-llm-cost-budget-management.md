# LLM Cost Tracking and Budget Management

## Status
Researching

## Problem Statement

LazyJob uses LLM APIs (Anthropic, OpenAI, Ollama) for various autonomous tasks via ralph loops. These APIs charge per token. Ralph loops can make many LLM calls autonomously, leading to:
1. **Surprise billing**: Users don't realize how much compute LLM calls consume
2. **Budget overrun**: No limits means a runaway loop can generate thousands of dollars in charges
3. **No attribution**: Users can't tell which operations cost the most
4. **No optimization guidance**: No data on which prompts/models are most expensive

---

## Solution Overview

A cost tracking and budget management system that:
1. Tracks LLM usage per operation, per ralph loop, per day/week/month
2. Enforces configurable budget limits with graceful degradation
3. Provides real-time cost visibility in the TUI
4. Supports per-provider and per-model cost accounting
5. Estimates cost before making requests when possible

---

## Cost Model

### Provider Cost Tables

Costs are defined per model per provider (as of 2024, should be configurable):

```rust
// lazyjob-llm/src/cost.rs

#[derive(Clone)]
pub struct CostTable {
    costs: HashMap<(Provider, Model), ModelCost>,
}

#[derive(Clone, Debug)]
pub struct ModelCost {
    pub input_tokens: DollarCents,  // per 1M tokens
    pub output_tokens: DollarCents,
    pub batch_input_tokens: DollarCents,  // for async batch
    pub batch_output_tokens: DollarCents,
}

#[derive(Clone, Copy, Debug)]
pub struct DollarCents(i64);  // store as cents to avoid float issues

impl DollarCents {
    pub fn from_dollars(d: f64) -> Self {
        Self((d * 100.0).round() as i64)
    }

    pub fn from_per_million(per_million: f64, token_count: u32) -> Self {
        Self(((per_million / 1_000_000.0) * token_count as f64).round() as i64)
    }
}

// Default costs (as of 2024)
impl Default for CostTable {
    fn default() -> Self {
        let mut costs = HashMap::new();

        // Anthropic
        costs.insert(
            (Provider::Anthropic, "claude-3-5-sonnet-20241022".into()),
            ModelCost {
                input_tokens: DollarCents::from_dollars(3.0),   // $3/M input
                output_tokens: DollarCents::from_dollars(15.0),  // $15/M output
                batch_input_tokens: DollarCents::from_dollars(1.5),
                batch_output_tokens: DollarCents::from_dollars(7.5),
            },
        );
        costs.insert(
            (Provider::Anthropic, "claude-3-opus-20240229".into()),
            ModelCost {
                input_tokens: DollarCents::from_dollars(15.0),
                output_tokens: DollarCents::from_dollars(75.0),
                batch_input_tokens: DollarCents::from_dollars(7.5),
                batch_output_tokens: DollarCents::from_dollars(37.5),
            },
        );
        costs.insert(
            (Provider::Anthropic, "claude-3-haiku-20240307".into()),
            ModelCost {
                input_tokens: DollarCents::from_dollars(0.25),
                output_tokens: DollarCents::from_dollars(1.25),
                batch_input_tokens: DollarCents::from_dollars(0.125),
                batch_output_tokens: DollarCents::from_dollars(0.625),
            },
        );

        // OpenAI
        costs.insert(
            (Provider::OpenAI, "gpt-4o".into()),
            ModelCost {
                input_tokens: DollarCents::from_dollars(2.5),
                output_tokens: DollarCents::from_dollars(10.0),
                batch_input_tokens: DollarCents::from_dollars(1.0),
                batch_output_tokens: DollarCents::from_dollars(4.0),
            },
        );
        costs.insert(
            (Provider::OpenAI, "gpt-4o-mini".into()),
            ModelCost {
                input_tokens: DollarCents::from_dollars(0.15),
                output_tokens: DollarCents::from_dollars(0.6),
                batch_input_tokens: DollarCents::from_dollars(0.075),
                batch_output_tokens: DollarCents::from_dollars(0.3),
            },
        );

        Self { costs }
    }
}
```

---

## Usage Tracking

### Usage Record

```rust
// lazyjob-llm/src/cost.rs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub operation: OperationType,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_cents: i64,
    pub ralph_loop_id: Option<Uuid>,  // which ralph loop, if any
    pub trace_id: Option<String>,     // for distributed tracing
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum OperationType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailoring,
    CoverLetterGeneration,
    InterviewPrep,
    SalaryNegotiation,
    UserChat,         // direct user interaction
    BackgroundTask,    // other background tasks
}
```

### Usage Tracker

```rust
// lazyjob-llm/src/cost/tracker.rs

pub struct UsageTracker {
    pool: SqlitePool,
    cost_table: CostTable,
}

impl UsageTracker {
    pub async fn record(&self, record: UsageRecord) -> Result<()> {
        sqlx::query!(
            "INSERT INTO llm_usage (
                id, timestamp, provider, model, operation,
                prompt_tokens, completion_tokens, cost_cents,
                ralph_loop_id, trace_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            record.id.to_string(),
            record.timestamp,
            record.provider,
            record.model,
            serde_json::to_string(&record.operation)?,
            record.prompt_tokens,
            record.completion_tokens,
            record.cost_cents,
            record.ralph_loop_id.map(|id| id.to_string()),
            record.trace_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_current_period(
        &self,
        user_id: &str,
        period: BillingPeriod,
    ) -> Result<CostSummary> {
        let (start, end) = period.date_range();

        let row = sqlx::query!(
            r#"
            SELECT
                SUM(prompt_tokens) as total_prompt_tokens,
                SUM(completion_tokens) as total_completion_tokens,
                SUM(cost_cents) as total_cost_cents,
                COUNT(*) as request_count
            FROM llm_usage
            WHERE user_id = ? AND timestamp BETWEEN ? AND ?
            "#,
            user_id,
            start,
            end,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(CostSummary {
            period,
            prompt_tokens: row.total_prompt_tokens.unwrap_or(0) as u64,
            completion_tokens: row.total_completion_tokens.unwrap_or(0) as u64,
            cost_cents: row.total_cost_cents.unwrap_or(0),
            request_count: row.request_count as u64,
        })
    }

    pub async fn get_by_operation(
        &self,
        user_id: &str,
        period: BillingPeriod,
    ) -> Result<HashMap<OperationType, CostSummary>> {
        let (start, end) = period.date_range();

        let rows = sqlx::query!(
            r#"
            SELECT
                operation,
                SUM(prompt_tokens) as total_prompt_tokens,
                SUM(completion_tokens) as total_completion_tokens,
                SUM(cost_cents) as total_cost_cents,
                COUNT(*) as request_count
            FROM llm_usage
            WHERE user_id = ? AND timestamp BETWEEN ? AND ?
            GROUP BY operation
            "#,
            user_id,
            start,
            end,
        )
        .fetch_all(&self.pool)
        .await?;

        // ... map rows to CostSummary by operation
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BillingPeriod {
    CurrentDay,
    CurrentWeek,
    CurrentMonth,
    Last30Days,
    Custom { start: DateTime<Utc>, end: DateTime<Utc> },
}

impl BillingPeriod {
    pub fn date_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        match self {
            BillingPeriod::CurrentDay => {
                let start = now.date().and_hms(0, 0, 0);
                let end = start + Duration::days(1);
                (start, end)
            }
            // ... other periods
            BillingPeriod::Custom { start, end } => (*start, *end),
        }
    }
}
```

---

## Budget Configuration

```rust
// lazyjob-llm/src/cost/budget.rs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub enabled: bool,
    pub monthly_limit_cents: i64,
    pub alert_thresholds: Vec<AlertThreshold>,
    pub per_operation_limits: HashMap<OperationType, OperationLimit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlertThreshold {
    pub percentage: u8,  // e.g., 50, 75, 90, 100
    pub action: AlertAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AlertAction {
    Log,
    NotifyTUI,
    PauseRalphLoops,
    BlockNewRequests,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationLimit {
    pub per_day_cents: Option<i64>,
    pub per_week_cents: Option<i64>,
    pub per_month_cents: Option<i64>,
    pub max_requests_per_hour: Option<u32>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            enabled: false,  // off by default for MVP
            monthly_limit_cents: 10_000,  // $100 default
            alert_thresholds: vec![
                AlertThreshold { percentage: 50, action: AlertAction::Log },
                AlertThreshold { percentage: 75, action: AlertAction::NotifyTUI },
                AlertThreshold { percentage: 90, action: AlertAction::PauseRalphLoops },
                AlertThreshold { percentage: 100, action: AlertAction::BlockNewRequests },
            ],
            per_operation_limits: HashMap::new(),
        }
    }
}
```

### Budget Enforcer

```rust
// lazyjob-llm/src/cost/enforcer.rs

pub struct BudgetEnforcer {
    tracker: UsageTracker,
    config: BudgetConfig,
    notification_tx: broadcast::Sender<BudgetEvent>,
}

#[derive(Clone, Debug)]
pub enum BudgetEvent {
    ThresholdReached { threshold: u8, period: BillingPeriod },
    RequestBlocked { reason: String, estimated_cost: i64 },
    RalphLoopPaused { loop_id: Uuid, reason: String },
}

impl BudgetEnforcer {
    pub async fn check_and_record(
        &self,
        operation: OperationType,
        estimated_cost: i64,
        ralph_loop_id: Option<Uuid>,
    ) -> Result<CheckResult> {
        // 1. Check global monthly budget
        let monthly = self.tracker.get_current_period("default", BillingPeriod::CurrentMonth).await?;
        let projected = monthly.cost_cents + estimated_cost;

        if projected > self.config.monthly_limit_cents {
            // Check if over threshold
            let percentage = (projected * 100 / self.config.monthly_limit_cents) as u8;

            // Fire alert if needed
            for threshold in &self.config.alert_thresholds {
                if percentage >= threshold.percentage {
                    self.notification_tx.send(BudgetEvent::ThresholdReached {
                        threshold: threshold.percentage,
                        period: BillingPeriod::CurrentMonth,
                    })?;
                }
            }

            if percentage >= 100 {
                return Ok(CheckResult::Blocked {
                    reason: format!(
                        "Monthly budget of ${:.2} exceeded. Projected: ${:.2}",
                        self.config.monthly_limit_cents as f64 / 100.0,
                        projected as f64 / 100.0
                    ),
                });
            }
        }

        // 2. Check per-operation limits
        if let Some(limit) = self.config.per_operation_limits.get(&operation) {
            let daily = self.tracker.get_daily_cost(operation).await?;
            if let Some(daily_limit) = limit.per_day_cents {
                if daily + estimated_cost > daily_limit {
                    return Ok(CheckResult::Blocked {
                        reason: format!("Daily {} budget exceeded", operation),
                    });
                }
            }
        }

        // 3. Record the actual usage
        // ... record ...

        Ok(CheckResult::Allowed)
    }
}

pub enum CheckResult {
    Allowed,
    Blocked { reason: String },
    Throttled { wait_seconds: u64 },
}
```

---

## Cost Display in TUI

### Status Bar Widget

The TUI status bar shows current cost status:

```
[Cost: $3.42/$10.00 ████████░░░░░░░ 34%] [Ralph: Running 2 loops]
```

### Cost Dashboard View

A dedicated view shows detailed cost breakdown:

```
┌─────────────────────────────────────────────────────────────┐
│ LLM Usage & Budget                              [Settings] │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Current Month: $3.42 / $10.00 budget          ████░░░░░░░  │
│                                                             │
│  ┌─────────────────────┬────────────────────┐               │
│  │ Operation           │ Cost      │ Reqs    │               │
│  ├─────────────────────┼────────────────────┤               │
│  │ Job Discovery       │ $1.20  │   45     │               │
│  │ Company Research    │ $0.80  │   23     │               │
│  │ Resume Tailoring    │ $0.60  │   12     │               │
│  │ Cover Letter Gen    │ $0.42  │    8     │               │
│  │ Interview Prep      │ $0.30  │    6     │               │
│  │ Other               │ $0.10  │   15     │               │
│  └─────────────────────┴────────────────────┘               │
│                                                             │
│  Daily Usage (last 7 days)                                  │
│  ▁▂▃▄▃▂▁  (avg: $0.49/day)                                 │
│                                                             │
│  [Pause All Ralph]  [Adjust Budget]  [View Detailed Log]    │
└─────────────────────────────────────────────────────────────┘
```

---

## Pre-Request Cost Estimation

Estimate cost before making request:

```rust
// lazyjob-llm/src/cost.rs

impl CostTable {
    pub fn estimate(&self, provider: &str, model: &str, input_tokens: u32, output_tokens: u32) -> Option<i64> {
        let cost = self.costs.get(&(provider.into(), model.into()))?;

        let input_cost = cost.input_tokens.from_per_million_tokens(input_tokens);
        let output_cost = cost.output_tokens.from_per_million_tokens(output_tokens);

        Some(input_cost + output_cost)
    }

    /// Estimate cost from message list (rough estimate based on avg tokenization)
    pub fn estimate_from_messages(&self, messages: &[ChatMessage], model: &str, provider: &str) -> i64 {
        // Rough estimate: 4 chars per token for English
        let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let estimated_tokens = (total_chars / 4) as u32;
        let estimated_output = 500; // assume ~500 token response

        self.estimate(provider, model, estimated_tokens, estimated_output)
            .unwrap_or(0)
    }
}
```

---

## Database Schema Extension

```sql
-- Add to 04-sqlite-persistence.md schema

CREATE TABLE llm_usage (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    operation TEXT NOT NULL,  -- JSON serialized OperationType
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    cost_cents INTEGER NOT NULL,
    ralph_loop_id TEXT,
    trace_id TEXT,
    user_id TEXT NOT NULL DEFAULT 'default'
);

CREATE INDEX idx_llm_usage_timestamp ON llm_usage(timestamp);
CREATE INDEX idx_llm_usage_operation ON llm_usage(operation);
CREATE INDEX idx_llm_usage_ralph_loop ON llm_usage(ralph_loop_id) WHERE ralph_loop_id IS NOT NULL;

CREATE TABLE budget_config (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    user_id TEXT NOT NULL DEFAULT 'default',
    enabled INTEGER NOT NULL DEFAULT 0,
    monthly_limit_cents INTEGER NOT NULL DEFAULT 10000,
    alert_thresholds TEXT,  -- JSON
    per_operation_limits TEXT,  -- JSON
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

---

## Open Questions

1. **Currency**: Should support non-USD billing (EUR, GBP)?
2. **Refund handling**: API providers sometimes refund. How to track?
3. **Ollama cost**: Ollama is free but uses local compute. How to account for electricity/hardware?
4. **Batch vs synchronous**: Different pricing for batch APIs (OpenAI). How to leverage?
5. **Budget sync**: Should budget config sync across devices (future SaaS)?

---

## Related Specs

- `02-llm-provider-abstraction.md` - LLM Provider trait
- `XX-llm-prompt-versioning.md` - Prompt versioning system
- `XX-ralph-ipc-protocol.md` - Ralph loop IPC (for attribution)
