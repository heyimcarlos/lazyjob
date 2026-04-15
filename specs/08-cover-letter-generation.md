# Cover Letter Generation

## Status
Researching

## Problem Statement

A cover letter complements the tailored resume by:
1. Telling the story that connects the candidate's background to the specific role
2. Demonstrating genuine interest in the company (researched)
3. Addressing potential concerns (career gaps, job changes, etc.)
4. Showcasing communication skills that resumes can't convey

This spec covers cover letter content generation, company research for personalization, and formatting.

---

## Research Findings

### Cover Letter Best Practices

**Structure (Problem-Solution Format)**:

1. **Opening Hook**: Capture attention immediately. Mention the specific role and where you found it, or lead with a compelling achievement.

2. **Company Research Paragraph**: Show you've done homework. Mention specific products, recent news, company culture, or mission alignment.

3. **Value Proposition**: Connect your background to their needs. "Your job description mentions X, and in my career I've..."

4. **Specific Achievements**: 2-3 bullet-style paragraphs with quantified results. Show, don't tell.

5. **Closing**: Call to action. "I look forward to discussing how I can contribute to [Team/Company]."

**Length**: 250-400 words (one page max)

**Tone**: Professional but authentic. Avoid stiff corporate-speak.

### Company Research Agent

For effective personalization, Ralph needs to research the company:

**Research Areas**:
1. **Company Mission/Values**: From website, LinkedIn, Glassdoor
2. **Recent News**: Press releases, TechCrunch, company blog
3. **Products/Services**: Understanding what they actually build
4. **Team/Technology**: Engineering blog, job description hints
5. **Culture Signals**: Fast-paced startup? Enterprise? Remote-first?

**Data Sources**:
- Company website (About, Careers pages)
- LinkedIn Company Page
- Crunchbase/Funded.ai (funding, size, stage)
- Glassdoor (reviews, culture)
- TechCrunch (recent news)
- The company's own blog/engineering posts

### Problem-Solution Format

The most effective cover letter structure:

```
[Company Name],

I was excited to see the [Role Title] position on [Where You Found It].
With [X years] of experience in [Relevant Skill/Industry], I've
consistently delivered results that align with your mission to
[Company Mission/Goal].

At [Previous Company], I [Specific Achievement with Metrics].
This experience taught me [Transferable Lesson] — exactly what
you need for [Aspect of the Role].

I'm particularly drawn to [Company] because [Specific Reason:
Product/Mission/Culture]. Your recent [News/Development] demonstrated
[Something Admirable].

I would love to bring my expertise in [Key Skills] to [Team/Company]
and contribute to [Specific Goal]. I'm happy to discuss how my
background aligns with your needs.

Best regards,
[Name]
```

---

## Design

### Cover Letter Service

```rust
// lazyjob-core/src/cover_letter/mod.rs

pub struct CoverLetterService {
    llm: Arc<dyn LLMProvider>,
    company_researcher: CompanyResearcher,
}

pub struct CoverLetterRequest {
    pub job: Job,
    pub life_sheet: LifeSheet,
    pub tailored_resume: Option<ResumeContent>,
    pub company_research: Option<CompanyResearch>,
    pub tone: CoverLetterTone,
    pub length: CoverLetterLength,
}

pub enum CoverLetterTone {
    Professional,
    Casual,
    Creative,
}

pub enum CoverLetterLength {
    Short,      // ~200 words
    Standard,   // ~300 words
    Detailed,   // ~400 words
}

pub struct CoverLetter {
    pub content: String,
    pub plain_text: String,
    pub research_used: CompanyResearch,
    pub key_points: Vec<String>,
}
```

### Company Researcher

```rust
// lazyjob-core/src/cover_letter/company_researcher.rs

pub struct CompanyResearch {
    pub name: String,
    pub mission: Option<String>,
    pub values: Vec<String>,
    pub products: Vec<String>,
    pub recent_news: Vec<NewsItem>,
    pub culture_signals: Vec<String>,
    pub team_size: Option<CompanySize>,
    pub funding: Option<String>,
    pub tech_stack: Vec<String>,
    pub linkedin_follower_count: Option<i64>,
    pub glassdoor_rating: Option<f32>,
}

pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub date: DateTime<Utc>,
    pub url: Option<String>,
    pub summary: String,
}

pub struct CompanyResearcher {
    http_client: reqwest::Client,
    llm: Arc<dyn LLMProvider>,
}

impl CompanyResearcher {
    pub async fn research(&self, company_name: &str) -> Result<CompanyResearch> {
        // 1. Gather raw data from multiple sources
        let (website_text, careers_text) = self.scrape_company_pages(company_name).await?;
        let news = self.fetch_news(company_name).await?;
        let linkedin = self.fetch_linkedin_info(company_name).await.ok();
        let glassdoor = self.fetch_glassdoor_info(company_name).await.ok();

        // 2. Synthesize with LLM
        let synthesis = self.synthesize(website_text, careers_text, &news, &linkedin, &glassdoor).await?;

        Ok(synthesis)
    }

    async fn synthesize(
        &self,
        website: String,
        careers: String,
        news: &[NewsItem],
        linkedin: Option<&LinkedInData>,
        glassdoor: Option<&GlassdoorData>,
    ) -> Result<CompanyResearch> {
        let prompt = format!(
            r#"Analyze the following company information and extract key insights for personalizing a cover letter.

Company Website:
{}

Careers Page:
{}

Recent News:
{}

LinkedIn: {}
Glassdoor: {}

Return JSON with:
- mission: Company's stated mission (if found)
- values: 3-5 company values or culture signals
- products: 2-3 key products or services
- culture_signals: Words/phrases describing culture (startup, fast-paced, innovative, etc.)
- technology_hints: Any hints about tech stack from the content
- personalization_hooks: 2-3 specific things to mention in a cover letter

Return ONLY valid JSON."#,
            website.chars().take(3000).collect::<String>(),
            careers.chars().take(2000).collect::<String>(),
            news.iter().take(5).map(|n| format!("{}: {}", n.title, n.summary)).collect::<Vec<_>>().join("\n"),
            linkedin.map(|l| format!("Followers: {}, Industry: {}", l.follower_count, l.industry)).unwrap_or_default(),
            glassdoor.map(|g| format!("Rating: {}, Culture: {}", g.rating, g.culture)).unwrap_or_default(),
        );

        let response = self.llm.complete(&prompt).await?;
        let parsed: CompanyResearchOutput = serde_json::from_str(&response)
            .context("Failed to parse LLM response")?;

        Ok(CompanyResearch {
            name: "TODO".to_string(),
            mission: parsed.mission,
            values: parsed.values,
            products: parsed.products,
            recent_news: news.to_vec(),
            culture_signals: parsed.culture_signals,
            team_size: None,
            funding: None,
            tech_stack: parsed.technology_hints,
            linkedin_follower_count: None,
            glassdoor_rating: None,
        })
    }
}
```

### Cover Letter Generation

```rust
// lazyjob-core/src/cover_letter/generator.rs

impl CoverLetterService {
    pub async fn generate(&self, request: &CoverLetterRequest) -> Result<CoverLetter> {
        // 1. Research company if not provided
        let research = match &request.company_research {
            Some(r) => r.clone(),
            None => self.company_researcher.research(&request.job.company_name).await?,
        };

        // 2. Extract relevant experience from life sheet
        let relevant_experience = self.extract_relevant_experience(
            &request.life_sheet,
            &request.job,
        ).await?;

        // 3. Generate cover letter content
        let content = self.generate_content(
            &request.job,
            &research,
            &relevant_experience,
            &request.tone,
            &request.length,
        ).await?;

        // 4. Extract key points for reference
        let key_points = self.extract_key_points(&content)?;

        Ok(CoverLetter {
            content,
            plain_text: self.to_plain_text(&content)?,
            research_used: research,
            key_points,
        })
    }

    async fn generate_content(
        &self,
        job: &Job,
        research: &CompanyResearch,
        experience: &[RelevantExperience],
        tone: &CoverLetterTone,
        length: &CoverLetterLength,
    ) -> Result<String> {
        let word_target = match length {
            CoverLetterTone::Short => 200,
            CoverLetterTone::Standard => 300,
            CoverLetterTone::Detailed => 400,
        };

        let experience_summary = experience.iter()
            .map(|e| format!("- {} at {}: {}", e.title, e.company, e.highlights[0]))
            .collect::<Vec<_>>()
            .join("\n");

        let personalization = research.culture_signals.iter()
            .take(2)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let product_hook = research.products.get(0)
            .map(|p| format!("your {}", p))
            .unwrap_or_else(|| "your team".to_string());

        let prompt = format!(
            r#"Write a cover letter for the following job application.

Company: {} ({})
Job Title: {}
Job Description Summary: {}

Company Research:
- Mission: {}
- Culture: {}
- Products: {}
- Recent News: {}

Relevant Experience:
{}

Tone: {}
Length target: ~{} words

Requirements:
- Start with the specific role and where you found it
- Include a paragraph about why you're excited by this company specifically
- Highlight 1-2 specific achievements relevant to the role
- Use concrete numbers/metrics where available
- End with a call to action
- Do NOT use clichés like "I am writing to express my interest"
- Be authentic and specific

Return ONLY the cover letter text, no formatting or labels."#,
            job.company_name,
            personalization,
            job.title,
            job.description.chars().take(500).collect::<String>(),
            research.mission.as_deref().unwrap_or("Not available"),
            personalization,
            research.products.join(", "),
            research.recent_news.first().map(|n| n.summary.as_str()).unwrap_or("None available"),
            experience_summary,
            format!("{:?}", tone).to_lowercase(),
            word_target,
        );

        self.llm.complete(&prompt).await.context("Failed to generate cover letter")
    }

    fn extract_key_points(&self, content: &str) -> Result<Vec<String>> {
        // Use simple NLP or LLM to extract main points
        // For now, return first sentence of each paragraph
        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        let key_points = paragraphs.iter()
            .take(4)  // First 4 paragraphs
            .filter_map(|p| p.lines().next())  // First line of each
            .map(|s| s.to_string())
            .collect();

        Ok(key_points)
    }
}
```

### Output Formats

```rust
// lazyjob-core/src/cover_letter/mod.rs

pub struct CoverLetterOutput {
    pub markdown: String,
    pub plain_text: String,
    pub docx: Vec<u8>,  // Optional Word doc
}

impl CoverLetterOutput {
    pub fn to_docx(&self) -> Result<Vec<u8>> {
        use docx_rs::*;

        let doc = Docx::new()
            .add_paragraph(Paragraph::new()
                .add_run(Run::new()
                    .add_text(&format!("Cover Letter - {}", Utc::now().format("%Y-%m-%d")))
                    .bold()
                    .size(24)))
            .add_paragraph(Paragraph::new()) // Spacer
            .add_paragraph(Paragraph::new()
                .add_run(Run::new()
                    .add_text(&self.plain_text)
                    .size(24)))
            .build();

        let mut buffer = Vec::new();
        doc.pack(&mut buffer)?;
        Ok(buffer)
    }

    pub fn to_plain_text(&self) -> String {
        // Convert markdown to plain text
        self.markdown
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
```

### Ralph Loop Integration

```rust
// Ralph cover-letter subcommand

#[derive(Parser)]
enum Commands {
    /// Generate a cover letter for a job
    Generate {
        /// Job ID from the database
        #[arg(long)]
        job_id: Uuid,

        /// Life sheet path
        #[arg(long)]
        life_sheet: PathBuf,

        /// Output path
        #[arg(long)]
        output: PathBuf,

        /// Tone (professional, casual, creative)
        #[arg(long, default_value = "professional")]
        tone: String,

        /// Length (short, standard, detailed)
        #[arg(long, default_value = "standard")]
        length: String,
    },
}

impl CoverLetterLoop {
    pub async fn run(&self, request: GenerateRequest) -> Result<()> {
        // Send status updates to TUI
        self.send_status("researching", 0.1, "Researching company...").await?;
        let research = self.company_researcher.research(&request.company_name).await?;

        self.send_status("generating", 0.5, "Writing cover letter...").await?;
        let cover_letter = self.generate(&request).await?;

        self.send_status("writing", 0.9, "Saving to file...").await?;
        tokio::fs::write(&request.output, &cover_letter.plain_text).await?;

        self.send_results(serde_json::json!({
            "job_id": request.job_id,
            "output_path": request.output,
            "key_points": cover_letter.key_points,
        })).await?;

        self.send_done(true).await
    }
}
```

---

## Cover Letter Templates

### Template 1: Standard Professional

```
[Your Name]
[Your Email] | [Your Phone] | [Your Location]

[Date]

Dear [Hiring Manager Name],

I was excited to discover the [Job Title] position at [Company Name]
through [Where You Found It]. With my [X years] of experience in
[Relevant Field], I am confident I can contribute to your team's
continued success.

In my current role at [Current/Previous Company], I have achieved
[Specific Achievement with Metrics]. This experience has prepared me
to handle [Relevant Responsibility from Job Description] — a key
focus of your open position.

What draws me to [Company Name] is your commitment to [Something
Specific about Company Mission, Product, or Culture]. Your recent
[News/Milestone] particularly impressed me because [Personal Connection].

I am excited about the opportunity to bring my expertise in [Key
Skills] to [Team/Department] and contribute to [Specific Goal].
I would welcome the opportunity to discuss how my background aligns
with your needs.

Thank you for considering my application.

Best regards,
[Your Name]
```

### Template 2: Problem-Solution (Recommended)

```
I couldn't help but notice the [Job Title] opening at [Company] —
the timing feels serendipitous because I've been following [Company's]
growth in [Industry/Technology] and recently [Specific Achievement
or News].

Your job description emphasizes [Key Requirement]. At [Previous Company],
I faced a similar challenge: [Problem You Solved]. I approached it by
[What You Did], resulting in [Quantified Impact]. I see a direct
parallel to [Aspect of Company's Problem], and I'm eager to apply
what I learned.

Beyond the technical fit, what excites me is [Company's Mission or
Culture Aspect]. Your team's approach to [Something Specific] resonates
with how I like to work: [Your Working Style].

I'm reaching out because I believe my background in [Your Expertise]
could help [Company] achieve [Their Goal]. Would you have 20 minutes
to explore this further?

[Your Name]
```

### Template 3: Career Changer

```
When I saw the [Job Title] role at [Company], it stopped me mid-scroll.
After [X years] in [Previous Industry/Role], I've been deliberately
building toward this moment — and your company is exactly where I want
that journey to lead.

Let me explain the through-line: My work in [Previous Field] taught
me [Transferable Skill], which I see as foundational to [New Field].
For example, when I [Specific Achievement], I developed the exact
[Skill Mentioned in Job Description] that your role requires.

I'm particularly drawn to [Company] because [Specific Reason]. Your
focus on [Something] aligns with my conviction that [Related Belief].

I know I don't have the traditional background for this path. But I
do have [Relevant Skills], a track record of [Transferable Achievements],
and the intellectual curiosity to keep learning. I hope you'll consider
what I could bring to [Team].

[Your Name]
```

---

## API Surface

```rust
// lazyjob-core/src/cover_letter/mod.rs

#[cfg_attr(async_trait::async_trait, async_trait)]
pub trait CoverLetterGenerator {
    async fn generate(
        &self,
        job_id: Uuid,
        life_sheet: &LifeSheet,
        options: CoverLetterOptions,
    ) -> Result<CoverLetter>;

    async fn research_company(&self, company_name: &str) -> Result<CompanyResearch>;
}

pub struct CoverLetterOptions {
    pub tone: CoverLetterTone,
    pub length: CoverLetterLength,
    pub include_research: bool,
    pub custom_intro: Option<String>,
}

impl CoverLetterService {
    /// Generate cover letter for a job
    pub async fn generate_for_job(
        &self,
        job_id: &Uuid,
        options: CoverLetterOptions,
    ) -> Result<CoverLetter> {
        let job = self.job_repository.get(job_id).await?;
        let life_sheet = self.life_sheet_repository.get().await?;

        self.generate(&CoverLetterRequest {
            job,
            life_sheet,
            tailored_resume: None,
            company_research: None,
            tone: options.tone,
            length: options.length,
        }).await
    }

    /// Save generated cover letter
    pub async fn save_version(
        &self,
        cover_letter: &CoverLetter,
        job_id: &Uuid,
    ) -> Result<CoverLetterVersion> {
        let version = CoverLetterVersion {
            id: Uuid::new_v4(),
            job_id: *job_id,
            content: cover_letter.content.clone(),
            plain_text: cover_letter.plain_text.clone(),
            research_summary: serde_json::to_string(&cover_letter.research_used)?,
            key_points: cover_letter.key_points.clone(),
            created_at: Utc::now(),
        };

        self.version_repository.save(&version).await
    }
}
```

---

## Failure Modes

1. **Company Research Fails**: Fall back to job description-only cover letter, warn user about lack of personalization
2. **LLM Generates Clichés**: Prompt includes explicit anti-cliché instructions; regenerate if detected
3. **No Relevant Experience**: Warn user that their background may not fit, suggest resume tailoring first
4. **News/Research Stale**: Note in output when research was conducted and that company info may have changed

---

## Open Questions

1. **Personalization vs. Speed**: Company research takes time. Should we have a "quick draft" mode that skips deep research?
2. **Cover Letter Length**: Some companies explicitly say "no cover letter needed." Should we detect this and skip generation?
3. **A/B Testing**: Should we generate multiple variants for user to choose from?
4. **ATS Compatibility**: Should cover letters be plain text to ensure ATS parsing?

---

## Dependencies

```toml
# lazyjob-core/Cargo.toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }
scraper = "0.20"              # HTML parsing
docx-rs = "0.4"               # Word doc generation
thiserror = "2"
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
tokio = { version = "1", features = ["full"] }
futures = "0.3"
regex = "1"

# HTML extraction
 ammonia = "4"                # HTML sanitization
```

---

## Sources

- [Jobscan Cover Letter Guide](https://www.jobscan.co/blog/cover-letter/)
- [The Muse - Cover Letter Templates](https://www.themuse.com/cover-letter-templates)
- [Harvard Business Review - Cover Letter Advice](https://hbr.org/2022/01/what-a-good-cover-letter-looks-like-in-2022)
