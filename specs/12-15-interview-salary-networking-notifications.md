# Interview Prep, Salary Negotiation, Networking & Notifications

## Status
Researching

## Problem Statement

Beyond job discovery and applications, LazyJob provides AI-powered assistance for:
1. **Interview Preparation**: Practice questions, research, feedback
2. **Salary Negotiation**: Market data, offer evaluation, strategy
3. **Networking**: Finding contacts, warm introductions, outreach
4. **Notifications**: Staying on top of follow-ups, interview schedules

This spec covers these four interrelated product features.

---

## Part 1: Interview Preparation

### Research Findings

**Types of Interviews**:
1. **Phone Screen**: Recruiter chat, 30 min, basic fit assessment
2. **Technical Screen**: Live coding or take-home, 60-90 min
3. **Behavioral**: Culture fit, STAR method responses
4. **On-site**: Multi-round, technical + behavioral + system design
5. **Final/Executive**: Often informal, bar raiser style

**AI Interview Prep Approaches**:
1. **Question Generation**: Generate STAR-method behavioral questions based on JD
2. **Mock Interviews**: AI acts as interviewer, provides feedback
3. **Company Research**: Synthesize Glassdoor, Blind, interviewer LinkedIn
4. **Flashcards**: Key concepts, system design patterns, trivia

### Interview Prep Service

```rust
pub struct InterviewPrepService {
    llm: Arc<dyn LLMProvider>,
    company_researcher: Arc<CompanyResearcher>,
}

pub struct InterviewPrepRequest {
    pub application_id: Uuid,
    pub interview_type: InterviewType,
    pub focus_areas: Vec<String>,  // "system design", "coding", "behavioral"
}

pub struct InterviewPrep {
    pub questions: Vec<InterviewQuestion>,
    pub company_insights: CompanyInsights,
    pub talking_points: Vec<String>,
    pub resources: Vec<Resource>,
}

pub struct InterviewQuestion {
    pub question: String,
    pub question_type: QuestionType,
    pub ideal_answer: Option<String>,
    pub tips: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum QuestionType {
    Behavioral,    // STAR format
    Technical,
    SystemDesign,
    Coding,
    CultureFit,
    Situational,
}
```

### Question Generation

```rust
impl InterviewPrepService {
    pub async fn generate_questions(
        &self,
        job: &Job,
        interview_type: InterviewType,
    ) -> Result<Vec<InterviewQuestion>> {
        let jd_keywords = self.extract_keywords(&job.description)?;
        let company_info = self.company_researcher.research(&job.company_name).await?;

        let prompt = format!(
            r#"Generate {} interview questions for a {} position at {}.

Job keywords: {}
Company info: {}

Generate questions in these categories:
- 2 behavioral (STAR method)
- 2 technical based on keywords
- 1 culture fit
- 1 closing question (for candidate to ask interviewer)

For each question provide:
- The question text
- What the interviewer is looking for
- Tips for answering

Return as JSON array."#,
            6,
            job.title,
            job.company_name,
            jd_keywords.join(", "),
            format!("{:?}", company_info),
        );

        let response = self.llm.complete(&prompt).await?;
        let questions: Vec<InterviewQuestion> = serde_json::from_str(&response)?;

        Ok(questions)
    }
}
```

### Mock Interview Loop (Ralph)

```rust
// Ralph interview-prep subcommand

pub struct MockInterviewLoop {
    // Simulates an interview, user types responses
    // LLM provides feedback after each answer
}

impl MockInterviewLoop {
    pub async fn run(&self, interview_type: InterviewType) -> Result<()> {
        // 1. Generate questions
        let questions = self.generate_questions(interview_type).await?;

        // 2. For each question:
        for question in &questions {
            self.send_question(question).await?;

            // 3. Get user response (reads from stdin or TUI input)
            let response = self.wait_for_response().await?;

            // 4. Get LLM feedback
            let feedback = self.get_feedback(question, &response).await?;

            // 5. Send feedback
            self.send_feedback(feedback).await?;
        }

        // 6. Overall summary
        let summary = self.generate_summary(questions).await?;
        self.send_summary(summary).await?;
        Ok(())
    }
}
```

---

## Part 2: Salary Negotiation

### Research Findings

**Salary Components**:
1. Base Salary
2. Annual Bonus (% or fixed)
3. Equity (RSUs, stock options)
4. Signing Bonus
5. Benefits (health, 401k match, etc.)

**Market Data Sources**:
- Levels.fyi (tech salaries)
- Glassdoor
- Payscale
- Blind
- H1B LCAs (public data)
- Built.in (startups)

**Negotiation Strategy**:
1. Never disclose current salary
2. Always negotiate total comp, not just base
3. Get offer in writing first
4. Have BATNA (Best Alternative to Negotiated Agreement)
5. Don't negotiate against yourself

### Salary Data Service

```rust
pub struct SalaryService {
    http_client: reqwest::Client,
    levels_api_key: Option<String>,
}

pub struct SalaryData {
    pub role: String,
    pub company: String,
    pub location: String,
    pub levels: SalaryLevels,
    pub currency: String,
}

pub struct SalaryLevels {
    pub low: i64,
    pub median: i64,
    pub high: i64,
    pub total_comp: Option<TotalComp>,
}

pub struct TotalComp {
    pub base: i64,
    pub bonus_percent: f32,
    pub equity_annual: i64,
    pub signing_bonus: Option<i64>,
}

pub struct OfferEvaluation {
    pub your_offer: OfferedComp,
    pub market_data: SalaryData,
    pub gap_analysis: GapAnalysis,
    pub negotiation_leverage: Vec<NegotiationPoint>,
    pub recommendation: String,
}

pub struct NegotiatedComp {
    pub original: OfferedComp,
    pub negotiated: OfferedComp,
    pub delta: CompDelta,
}
```

### Offer Evaluation

```rust
impl SalaryService {
    pub async fn evaluate_offer(
        &self,
        offer: &OfferedComp,
        job: &Job,
    ) -> Result<OfferEvaluation> {
        // 1. Get market data
        let market = self.get_market_data(&job.title, &job.company_name, &job.location).await?;

        // 2. Calculate gaps
        let gap_analysis = self.calculate_gaps(offer, &market)?;

        // 3. Identify negotiation leverage
        let leverage = self.identify_leverage(&gap_analysis)?;

        // 4. Generate recommendation
        let recommendation = self.generate_recommendation(&gap_analysis, &leverage).await?;

        Ok(OfferEvaluation {
            your_offer: offer.clone(),
            market_data: market,
            gap_analysis,
            negotiation_leverage: leverage,
            recommendation,
        })
    }

    async fn get_market_data(
        &self,
        title: &str,
        company: &str,
        location: &str,
    ) -> Result<SalaryData> {
        // Try multiple sources and aggregate

        // 1. Levels.fyi API (if available)
        let levels_data = self.fetch_levels_fyi(title, company).await.ok();

        // 2. Glassdoor estimates
        let glassdoor_data = self.fetch_glassdoor(title, company, location).await.ok();

        // 3. Blind data points
        let blind_data = self.fetch_blind(title, company).await.ok();

        // Aggregate and return median estimates
        self.aggregate_salary_data(levels_data, glassdoor_data, blind_data)
    }
}
```

### Negotiation Strategy Generator

```rust
impl SalaryService {
    pub async fn generate_negotiation_strategy(
        &self,
        offer: &Offer,
        evaluation: &OfferEvaluation,
    ) -> Result<NegotiationStrategy> {
        let prompt = format!(
            r#"A {} sent me an offer for {} at {} in {}.

Current offer breakdown:
- Base: ${}
- Bonus: {}% (${})
- Equity: {} over {} years
- Signing: ${}
- Total: ${}

Market data:
- Median for this role/company/location: ${}
- Range: ${}-${}

Gap analysis:
{}

Generate a negotiation strategy with:
1. Opening line (never reveal current salary)
2. Key points to negotiate
3. Priority order (base vs bonus vs equity)
4. Walk-away points (if any)
5. Expected outcome

Be specific and practical."#,
            offer.source,
            offer.job_title,
            offer.company_name,
            offer.location,
            offer.base,
            offer.bonus_percent,
            offer.bonus_value(),
            offer.equity_value(),
            offer.equity_years,
            offer.signing_bonus.unwrap_or(0),
            offer.total_comp(),
            evaluation.market_data.levels.median,
            evaluation.market_data.levels.low,
            evaluation.market_data.levels.high,
            format!("{:?}", evaluation.gap_analysis),
        );

        let response = self.llm.complete(&prompt).await?;

        Ok(NegotiationStrategy {
            original_offer: offer.clone(),
            strategy: response,
            suggested_counter: self.suggest_counter(offer, &evaluation.gap_analysis)?,
        })
    }
}
```

---

## Part 3: Networking & Referrals

### Research Findings

**Referral Impact**:
- Referrals are 4x more likely to get hired
- Referrals skip first recruiter screen 40% of time
- Warm introductions 10x more effective than cold outreach

**Networking Sources**:
1. LinkedIn (1st, 2nd, 3rd degree connections)
2. Alumni networks
3. Industry events
4. Twitter/X, Hacker News
5. Company employees on GitHub

**Warm Introduction Framework**:
1. Find shared connection
2. Ask for introduction (not job directly)
3. Offer value before asking

### Networking Service

```rust
pub struct NetworkingService {
    http_client: reqwest::Client,
    linkedin_client: Option<LinkedInClient>,
}

pub struct ContactFinder {
    pub target_company: String,
    pub target_role: Option<String>,
    pub mutual_connections: Vec<MutualConnection>,
    pub outreach_templates: Vec<OutreachTemplate>,
}

pub struct MutualConnection {
    pub name: String,
    pub role: String,
    pub company: String,
    pub linkedin_url: String,
    pub connection_degree: ConnectionDegree,
    pub relationship_strength: f32,  // 0-1
}

#[derive(Debug, Clone, Copy)]
pub enum ConnectionDegree {
    First,   // Direct connection
    Second,  // Connection of connection
    Third,   // Can be reached via company groups
}

pub struct OutreachTemplate {
    pub template: String,
    pub best_for: ConnectionDegree,
    pub tone: OutreachTone,
}
```

### Contact Finding

```rust
impl NetworkingService {
    pub async fn find_contacts_for_company(
        &self,
        company_name: &str,
    ) -> Result<Vec<Contact>> {
        // 1. Search LinkedIn for employees
        let linkedin_contacts = self.search_linkedin_employees(company_name).await?;

        // 2. Search by role/team
        let by_role = self.search_by_role(company_name).await?;

        // 3. Find mutual connections
        let mutuals = self.find_mutual_connections(&linkedin_contacts).await?;

        // 4. Rank by relevance
        let ranked = self.rank_contacts(mutuals, by_role)?;

        Ok(ranked)
    }

    pub async fn generate_outreach(
        &self,
        contact: &Contact,
        introduction_context: &str,
    ) -> Result<String> {
        let template = match contact.connection_degree {
            ConnectionDegree::First => OutreachTemplate::Casual,
            ConnectionDegree::Second => OutreachTemplate::Warm,
            ConnectionDegree::Third => OutreachTemplate::Formal,
        };

        self.render_template(&template, contact, introduction_context).await
    }
}
```

### Outreach Templates

```rust
// Warm introduction (2nd degree)
"Hi {connection_name},

I noticed {target_name} works on {team} at {company}.
We're connected through {common_connection}, and I wanted to see if
a warm introduction might make sense.

I'm exploring opportunities in {field} and {company} is someone I'd
love to learn more about. Would you be comfortable making an intro?

I know this is a favor, and I'm happy to return the kindness
however I can.

Thanks for considering!

{your_name}"

// 2nd degree cold outreach
"Hi {target_name},

I came across your work on {project/team} at {company} and was
really impressed by {specific thing}.

I'm currently focused on {your_focus} and would love to connect
about opportunities in {field}. I noticed we share {common connection},
which is what prompted me to reach out.

Would you have 20 minutes for a quick chat? I'm happy to work
around your schedule.

{your_name}"
```

---

## Part 4: Morning Brief & Notifications

### Research Findings

**Morning Brief Benefits**:
1. Sets daily priorities
2. Surfaces forgotten follow-ups
3. Keeps job search top-of-mind
4. Provides momentum

**Notification Types**:
1. Application status changes
2. Interview reminders
3. Follow-up reminders
4. New jobs matching profile
5. Offer deadlines
6. Weekly/Monthly summaries

### Notification Service

```rust
pub struct NotificationService {
    db: Database,
    scheduler: Scheduler,
    email_client: Option<EmailClient>,
}

pub enum NotificationType {
    MorningBrief,
    InterviewReminder,
    FollowUpReminder,
    ApplicationUpdate,
    NewJobMatch,
    OfferDeadline,
    WeeklySummary,
}

pub struct Notification {
    pub id: Uuid,
    pub notification_type: NotificationType,
    pub title: String,
    pub body: String,
    pub action_url: Option<String>,
    pub priority: Priority,
    pub created_at: DateTime<Utc>,
    pub scheduled_for: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}
```

### Morning Brief Generator

```rust
impl NotificationService {
    pub async fn generate_morning_brief(&self, user_id: &Uuid) -> Result<MorningBrief> {
        let (applications, upcoming_interviews, follow_ups, new_matches) =
            self.gather_data(user_id).await?;

        let brief = MorningBrief {
            date: Utc::now().date(),
            summary: self.generate_summary_line(&applications).await?,
            action_items: self.generate_action_items(
                &upcoming_interviews,
                &follow_ups,
                &new_matches,
            ).await?,
            stats: PipelineStats::calculate(&applications),
            new_opportunities: new_matches.take(5).collect(),
        };

        Ok(brief)
    }

    async fn generate_action_items(
        &self,
        interviews: &[Interview],
        follow_ups: &[FollowUp],
        new_jobs: &[Job],
    ) -> Result<Vec<ActionItem>> {
        let mut items = Vec::new();

        // Overdue follow-ups
        for fu in follow_ups.iter().filter(|f| f.is_overdue()) {
            items.push(ActionItem {
                priority: Priority::High,
                title: format!("Follow up: {} at {}", fu.company_name, fu.job_title),
                deadline: fu.due_at,
                action: format!("Send follow-up email to {}", fu.contact_name),
            });
        }

        // Tomorrow's interviews
        for int in interviews.iter().filter(|i| i.is_tomorrow()) {
            items.push(ActionItem {
                priority: Priority::High,
                title: format!("Interview: {} at {}", int.job_title, int.company_name),
                deadline: int.scheduled_at,
                action: "Prepare and review company research".to_string(),
            });
        }

        // New matching jobs
        for job in new_jobs.iter().take(3) {
            items.push(ActionItem {
                priority: Priority::Medium,
                title: format!("New match: {} at {}", job.title, job.company_name),
                deadline: None,
                action: format!("{:.0}% match - review?", job.match_score * 100.0),
            });
        }

        Ok(items)
    }
}
```

### Scheduling

```rust
impl NotificationService {
    pub fn schedule_morning_brief(&self, user_id: &Uuid, time: chrono::NaiveTime) -> Result<()> {
        let schedule = format!("0 {} * * *", time.format("%M %H"));  // cron format

        self.scheduler.add_job(
            &format!("morning_brief_{}", user_id),
            "MorningBrief",
            &schedule,
            |ctx| async move {
                let brief = self.generate_morning_brief(user_id).await?;
                self.deliver_brief(&brief, user_id).await?;
                Ok(())
            },
        )?;

        Ok(())
    }
}
```

---

## Open Questions

1. **Interview Recording**: Should we allow recording/mock interviews for self-review?
2. **Salary Data Accuracy**: How to handle limited data for less common roles?
3. **LinkedIn Outreach**: How to integrate without violating ToS?
4. **Notification Channels**: Email, push, in-app, or all?

---

## Dependencies

```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

---

## Sources

- [Levels.fyi Salary Data](https://www.levels.fyi/)
- [Glassdoor Salary Guide](https://www.glassdoor.com/Salaries/index.htm)
- [Hacker News "Salary Negotiation" Threads](https://news.ycombinator.com/)
