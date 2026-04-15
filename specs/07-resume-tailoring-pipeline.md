# Resume Tailoring Pipeline

## Status
Researching

## Problem Statement

A key value proposition of LazyJob is tailoring resumes to specific job descriptions. This requires:
1. **Parsing resumes**: Extracting structured data from existing resume documents
2. **Job description analysis**: Identifying key requirements, keywords, and skills
3. **Gap analysis**: Comparing user's profile against job requirements
4. **Tailoring**: Generating a customized resume that highlights relevant experience
5. **Fabrication guardrails**: Ensuring generated content is based on real data, not hallucinated

This spec covers the complete resume tailoring pipeline.

---

## Research Findings

### docx-rs (Word Document Generation)

The `docx-rs` crate (v0.4) enables programmatic Word document creation:

**Document Structure**
```rust
use docx_rs::*;

Docx::new()
    .add_paragraph(Paragraph::new()
        .add_run(Run::new()
            .add_text("John Doe")
            .bold()
            .size(32)))
    .add_paragraph(Paragraph::new()
        .add_run(Run::new()
            .add_text("Software Engineer")))
    .build()
    .pack(file)?;
```

**Text Formatting**
- `Run::new().bold()` - Bold text
- `Run::new().italic()` - Italic text
- `Run::new().underline("single")` - Underline
- `Run::new().size(28)` - Font size (half-points)
- `Run::new().color("FF0000")` - Text color
- `Run::new().highlight("yellow")` - Text highlight

**Paragraph Formatting**
- `Paragraph::new().align(AlignmentType::Center)`
- `Paragraph::new().indent(100)` - Indentation
- Numbered/bulleted lists supported

**Document Elements**
- Paragraphs with text runs
- Tables
- Headers/footers
- Images (base64 embedded)
- Sections

### ATS Resume Parsing Approaches

**How ATS Systems Work**:

1. **Document Parsing**: Extract text from uploaded resume (DOC, DOCX, PDF, TXT)
2. **Section Identification**: Identify headers (Experience, Education, Skills) via NLP/rule-based detection
3. **Entity Extraction**: Pull out names, emails, phone numbers, company names, dates, job titles
4. **Keyword Analysis**: Identify important terms (skills, technologies, certifications)
5. **Semantic Analysis**: Understand context and relationships
6. **Scoring**: Compare against job description keywords

**Key Factors in ATS Scoring**:
- **Keyword Density**: Frequency of job-relevant keywords
- **Exact Matches**: "Python" vs "python programming"
- **Synonym Recognition**: "JS" vs "JavaScript"
- **Section Headers**: Proper headers increase parse accuracy
- **Contact Info**: Must be present and properly formatted
- **Dates**: Employment dates help with chronology
- **File Format**: DOCX generally best, PDF acceptable, plain text risky

**Common ATS Platforms**:
- Greenhouse, Lever, Workday, iCIMS, Taleo, BambooHR
- Each has slightly different parsing rules

### Keyword Extraction Methods

**Method 1: TF-IDF (Term Frequency-Inverse Document Frequency)**
```rust
fn extract_keywords_tfidf(text: &str, job_descriptions: &[&str], top_k: usize) -> Vec<(String, f32)> {
    // Count term frequency in target JD
    // Weight by inverse frequency across corpus
    // Return top-k terms
}
```

**Method 2: Named Entity Recognition**
- Extract: Technologies (Python, React), Certifications (AWS), Job Titles (Software Engineer)
- Use simple keyword lists or NLP library

**Method 3: Rule-Based Extraction**
- Look for: "Required:", "Must have:", "Experience with:", "Proficiency in:"
- Extract bullet points following these phrases

### Resume Sections

**Standard Resume Sections**:
1. **Header**: Name, contact info, LinkedIn, GitHub
2. **Summary/Objective**: 2-3 sentence professional summary
3. **Experience**: Company, Title, Dates, Bullet points (achievements)
4. **Education**: Degree, Institution, Graduation date, GPA (if good)
5. **Skills**: Technical skills, Languages, Tools
6. **Projects**: Notable projects with descriptions
7. **Certifications**: Professional certifications

---

## Design

### Pipeline Overview

```
┌─────────────────────────────────────────────────────────────┐
│ Input: Life Sheet (YAML) + Job Description                    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 1: Parse Job Description                                  │
│   - Extract structured requirements                           │
│   - Identify key skills, qualifications                       │
│   - Extract "must-have" vs "nice-to-have"                     │
│   - Score job description using LLM                          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 2: Analyze Life Sheet                                    │
│   - Extract relevant experience for this job                 │
│   - Identify matching skills                                  │
│   - Find gaps (missing skills/experience)                    │
│   - Prioritize achievements relevant to this role             │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 3: Gap Analysis                                          │
│   - Compare job requirements to user profile                 │
│   - Identify keywords to add                                  │
│   - Identify experiences to emphasize                        │
│   - Note fabricatable additions (certifications, skills)     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 4: Draft Resume Content                                   │
│   - Select relevant experience entries                       │
│   - Rewrite bullet points with JD keywords                   │
│   - Order skills by relevance                                │
│   - Add targeted summary statement                           │
│   - Fabricate minimally (with clear flags)                  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 5: Generate Document                                     │
│   - Create DOCX using docx-rs                               │
│   - Apply formatting (bold, headers, spacing)               │
│   - Add contact info                                        │
│   - Output to user-specified location                        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 6: Validation & Review                                   │
│   - Check keyword presence                                    │
│   - Review formatting                                        │
│   - Present for human approval                               │
│   - Store version for tracking                               │
└─────────────────────────────────────────────────────────────┘
```

### Component Design

```rust
// lazyjob-core/src/resume/mod.rs

pub struct ResumeTailor {
    llm: Arc<dyn LLMProvider>,
    life_sheet_repo: LifeSheetRepository,
}

impl ResumeTailor {
    pub async fn tailor(
        &self,
        life_sheet: &LifeSheet,
        job: &Job,
        options: TailoringOptions,
    ) -> Result<TailoredResume> {
        // Step 1: Parse job description
        let jd_analysis = self.analyze_job_description(&job.description).await?;

        // Step 2: Analyze life sheet
        let profile_analysis = self.analyze_life_sheet(life_sheet, &jd_analysis)?;

        // Step 3: Gap analysis
        let gaps = self.analyze_gaps(&jd_analysis, &profile_analysis)?;

        // Step 4: Draft content
        let content = self.draft_content(life_sheet, &jd_analysis, &profile_analysis, &gaps, &options).await?;

        // Step 5: Generate document
        let docx = self.generate_docx(&content, life_sheet)?;

        Ok(TailoredResume { content, docx, gaps })
    }
}
```

### Job Description Analysis

```rust
// lazyjob-core/src/resume/jd_parser.rs

pub struct JobDescriptionAnalysis {
    pub raw_text: String,
    pub required_skills: Vec<Skill>,
    pub nice_to_have_skills: Vec<Skill>,
    pub required_experience: Vec<ExperienceRequirement>,
    pub responsibilities: Vec<String>,
    pub qualifications: Vec<String>,
    pub keywords: Vec<String>,  // All important terms
    pub soft_skills: Vec<String>,
    pub culture_signals: Vec<String>,
}

impl JobDescriptionAnalysis {
    pub async fn parse(jd_text: &str, llm: &Arc<dyn LLMProvider>) -> Result<Self> {
        let prompt = format!(
            r#"Analyze this job description and extract structured information.

Job Description:
{}

Return a JSON object with:
- required_skills: List of required technical skills (exact names)
- nice_to_have_skills: Skills that are preferred but not required
- required_experience: Years and type of experience required
- responsibilities: Key job responsibilities (3-5 bullet points)
- qualifications: Educational and experience qualifications
- keywords: All important terms (skills, tools, technologies)
- soft_skills: Required soft skills (leadership, communication, etc.)
- culture_signals: Words indicating company culture (startup, fast-paced, etc.)

Return ONLY valid JSON, no markdown formatting."#,
            jd_text
        );

        let response = llm.complete(&prompt).await?;
        let parsed: JDParserOutput = serde_json::from_str(&response)
            .context("Failed to parse LLM response as JSON")?;

        Ok(Self {
            raw_text: jd_text.to_string(),
            required_skills: parsed.required_skills,
            nice_to_have_skills: parsed.nice_to_have_skills,
            required_experience: parsed.required_experience,
            responsibilities: parsed.responsibilities,
            qualifications: parsed.qualifications,
            keywords: parsed.keywords,
            soft_skills: parsed.soft_skills,
            culture_signals: parsed.culture_signals,
        })
    }
}
```

### Gap Analysis

```rust
// lazyjob-core/src/resume/gap_analysis.rs

pub struct GapAnalysis {
    pub matched_skills: Vec<MatchedSkill>,
    pub missing_skills: Vec<MissingSkill>,
    pub emphasized_experiences: Vec<ExperienceEntry>,
    pub relevant_achievements: Vec<Achievement>,
    pub fabrication_flags: Vec<FabricationFlag>,
}

pub struct MatchedSkill {
    pub skill_name: String,
    pub evidence: String,  // Where this skill appears in life sheet
    pub strength: f32,     // 0-1, how strongly it's demonstrated
}

pub struct MissingSkill {
    pub skill_name: String,
    pub is_required: bool,
    pub fabrication_safe: bool,  // Can we safely add this?
    pub fabrication_method: Option<FabricationMethod>,
}

pub struct FabricationFlag {
    pub description: String,
    pub severity: Severity,  // Warning, Error
    pub recommendation: String,
}

impl GapAnalysis {
    pub fn analyze(
        &self,
        jd: &JobDescriptionAnalysis,
        profile: &ProfileAnalysis,
    ) -> Self {
        let mut matched_skills = Vec::new();
        let mut missing_skills = Vec::new();

        // Check required skills
        for req_skill in &jd.required_skills {
            if let Some(match_) = profile.find_skill(&req_skill.name) {
                matched_skills.push(MatchedSkill {
                    skill_name: req_skill.name.clone(),
                    evidence: match_.source.clone(),
                    strength: match_.relevance_score,
                });
            } else {
                missing_skills.push(MissingSkill {
                    skill_name: req_skill.name.clone(),
                    is_required: true,
                    fabrication_safe: Self::is_fabricatable(&req_skill.name),
                    fabrication_method: Self::suggest_fabrication(&req_skill.name),
                });
            }
        }

        // ... similar for nice-to-have

        Self {
            matched_skills,
            missing_skills,
            emphasized_experiences: profile.select_relevant_experiences(&jd),
            relevant_achievements: profile.select_relevant_achievements(&jd),
            fabrication_flags: Self::check_fabrication_flags(&missing_skills),
        }
    }
}
```

### Content Drafting

```rust
// lazyjob-core/src/resume/drafting.rs

pub struct ResumeContent {
    pub summary: String,
    pub experience: Vec<ExperienceSection>,
    pub skills: SkillsSection,
    pub education: Vec<EducationEntry>,
    pub projects: Vec<ProjectEntry>,
}

impl ResumeContent {
    pub async fn draft(
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapAnalysis,
        options: &TailoringOptions,
        llm: &Arc<dyn LLMProvider>,
    ) -> Result<Self> {
        // Generate targeted summary
        let summary = Self::draft_summary(life_sheet, jd, gaps, llm).await?;

        // Rewrite experience bullets with keywords
        let experience = Self::draft_experience(life_sheet, jd, gaps, llm).await?;

        // Order skills by relevance
        let skills = Self::draft_skills(life_sheet, jd, gaps)?;

        // Select relevant education
        let education = Self::select_education(life_sheet, jd)?;

        // Select relevant projects
        let projects = Self::select_projects(life_sheet, jd)?;

        Ok(Self {
            summary,
            experience,
            skills,
            education,
            projects,
        })
    }

    async fn draft_summary(
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapAnalysis,
        llm: &Arc<dyn LLMProvider>,
    ) -> Result<String> {
        let matched = gaps.matched_skills.iter()
            .take(3)
            .map(|s| s.skill_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let years = Self::calculate_experience_years(life_sheet);

        let prompt = format!(
            r#"Write a 2-3 sentence professional summary for a resume.

Target Job: {}
Matched Skills: {}
Experience: {} years in relevant field

Write ONLY the summary, no formatting. Be specific and compelling. Avoid clichés like "hard-working" or "team player"."#,
            jd.responsibilities.join(" "),
            matched,
            years
        );

        llm.complete(&prompt).await.context("Failed to generate summary")
    }

    async fn draft_experience(
        life_sheet: &LifeSheet,
        jd: &JobDescriptionAnalysis,
        gaps: &GapAnalysis,
        llm: &Arc<dyn LLMProvider>,
    ) -> Result<Vec<ExperienceSection>> {
        let mut sections = Vec::new();

        for exp in &gaps.emphasized_experiences {
            let bullets = Self::rewrite_bullets(&exp.bullets, &jd.keywords, llm).await?;

            sections.push(ExperienceSection {
                company: exp.company.clone(),
                title: exp.title.clone(),
                dates: exp.date_range.clone(),
                bullets,
            });
        }

        Ok(sections)
    }

    async fn rewrite_bullets(
        original: &[String],
        keywords: &[String],
        llm: &Arc<dyn LLMProvider>,
    ) -> Result<Vec<String>> {
        let keyword_str = keywords.iter()
            .take(10)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let prompt = format!(
            r#"Rewrite these resume bullet points to incorporate relevant keywords.

Original bullets:
{}

Target keywords to naturally incorporate: {}

Rules:
- Keep the achievements based on real accomplishments
- Use action verbs (Built, Led, Designed, Implemented, Reduced)
- Quantify results when possible
- Do NOT make up achievements
- Write 1-2 bullets max

Return ONLY the bullets, one per line, no numbering."#,
            original.join("\n"),
            keyword_str
        );

        let response = llm.complete(&prompt).await?;
        let bullets = response.lines().map(|s| s.to_string()).collect();

        Ok(bullets)
    }
}
```

### Document Generation

```rust
// lazyjob-core/src/resume/docx_generator.rs

use docx_rs::*;

pub fn generate_resume_docx(
    content: &ResumeContent,
    personal: &PersonalInfo,
) -> Result<Vec<u8>> {
    let mut doc = Docx::new();

    // Header
    doc = doc.add_paragraph(Paragraph::new()
        .add_run(Run::new()
            .add_text(&personal.name)
            .bold()
            .size(48))
        .align(AlignmentType::Center));

    // Contact info
    let contact = format!(
        "{} | {} | {} | {}",
        personal.email,
        personal.phone,
        personal.location,
        personal.profiles.iter().map(|p| p.url.clone()).collect::<Vec<_>>().join(" | ")
    );
    doc = doc.add_paragraph(Paragraph::new()
        .add_run(Run::new()
            .add_text(&contact)
            .size(20))
        .align(AlignmentType::Center));

    doc = doc.add_paragraph(Paragraph::new()); // Spacer

    // Summary
    doc = doc.add_paragraph(Block::new()
        .title("Professional Summary")
        .add_run(Run::new()
            .add_text(&content.summary)
            .size(24)));

    // Experience
    for exp in &content.experience {
        doc = doc.add_paragraph(Paragraph::new()
            .add_run(Run::new()
                .add_text(&format!("{} | {}", exp.title, exp.company))
                .bold()
                .size(28)));

        doc = doc.add_paragraph(Paragraph::new()
            .add_run(Run::new()
                .add_text(&exp.dates)
                .size(22))
            .add_run(Run::new()
                .add_text(&exp.bullets.join("\n"))
                .size(22)));
    }

    // Skills
    doc = doc.add_paragraph(Block::new()
        .title("Skills")
        .add_run(Run::new()
            .add_text(&content.skills.to_string())
            .size(24)));

    // Education
    for edu in &content.education {
        doc = doc.add_paragraph(Paragraph::new()
            .add_run(Run::new()
                .add_text(&format!("{} | {}", edu.degree, edu.institution))
                .bold()
                .size(26)));
    }

    // Pack to bytes
    let mut buffer = Vec::new();
    doc.build().pack(&mut buffer)?;

    Ok(buffer)
}
```

### Fabrication Guardrails

```rust
// lazyjob-core/src/resume/fabrication_guardrails.rs

#[derive(Debug, Clone)]
pub enum FabricationLevel {
    Safe,       // Based on real data, just reworded
    Acceptable, // Can be claimed (e.g., "familiar with X")
    Risky,     // Cannot be claimed without evidence
    Forbidden,  // Never fabricate (licenses, certifications)
}

pub fn assess_fabrication(
    item: &str,
    life_sheet: &LifeSheet,
) -> FabricationLevel {
    match item {
        // Always safe - rephrasing
        s if life_sheet.contains_skill(s) => FabricationLevel::Safe,

        // Acceptable - familiarity claim
        s if is_adjacent_skill(s, life_sheet) => FabricationLevel::Acceptable,

        // Risky - no evidence
        _ => FabricationLevel::Risky,
    }
}

pub struct FabricationReport {
    pub items: Vec<FabricationItem>,
    pub overall_score: f32,  // 0-100
    pub warnings: Vec<String>,
    pub is_submittable: bool,
}

impl FabricationReport {
    pub fn generate(
        resume: &ResumeContent,
        life_sheet: &LifeSheet,
    ) -> Self {
        let mut items = Vec::new();
        let mut warnings = Vec::new();

        // Check skills
        for skill in &resume.skills.items {
            let level = assess_fabrication(skill, life_sheet);
            items.push(FabricationItem {
                item: skill.clone(),
                level,
                source: life_sheet.find_skill_source(skill),
            });

            if matches!(level, FabricationLevel::Risky | FabricationLevel::Forbidden) {
                warnings.push(format!(
                    "'{}' cannot be claimed without evidence",
                    skill
                ));
            }
        }

        // Check experience claims
        for exp in &resume.experience {
            for bullet in &exp.bullets {
                if contains_fabricated_claim(bullet, life_sheet) {
                    warnings.push(format!(
                        "Bullet point contains claim without evidence: {}",
                        bullet
                    ));
                }
            }
        }

        let is_submittable = warnings.is_empty()
            || warnings.iter().all(|w| !w.contains("Forbidden"));

        Self {
            items,
            overall_score: calculate_score(&items),
            warnings,
            is_submittable,
        }
    }
}
```

---

## API Surface

```rust
// lazyjob-core/src/resume/mod.rs

pub struct ResumeService {
    tailor: ResumeTailor,
    repository: ResumeVersionRepository,
}

impl ResumeService {
    /// Tailor a resume for a specific job
    pub async fn tailor_for_job(
        &self,
        job_id: &Uuid,
        options: TailoringOptions,
    ) -> Result<TailoredResume> {
        let job = self.job_repo.get(job_id).await?;
        let life_sheet = self.life_sheet_repo.get().await?;

        self.tailor.tailor(&life_sheet, &job, options).await
    }

    /// Save a tailored resume version
    pub async fn save_version(
        &self,
        tailored: &TailoredResume,
        job_id: &Uuid,
    ) -> Result<ResumeVersion> {
        let version = ResumeVersion {
            id: Uuid::new_v4(),
            job_id: *job_id,
            content: tailored.content.clone(),
            docx_data: tailored.docx.clone(),
            fabrication_report: tailored.fabrication_report.clone(),
            created_at: Utc::now(),
        };

        self.repository.save(&version).await
    }

    /// Export to file
    pub async fn export(&self, version_id: &Uuid, path: &Path) -> Result<()> {
        let version = self.repository.get(version_id).await?;
        tokio::fs::write(path, &version.docx_data).await?;
        Ok(())
    }
}
```

---

## Failure Modes

1. **LLM Hallucination**: Guardrails prevent fabricating achievements; all content must map to real life sheet data
2. **JD Parse Failure**: If LLM fails to parse, fall back to keyword extraction (TF-IDF)
3. **No Matched Skills**: If user has no relevant experience for job, warn user and suggest upskilling
4. **Document Generation Error**: docx-rs errors; catch and provide plain text fallback
5. **Fabrication Flags**: Block submission of resumes with forbidden-level fabrication

---

## Open Questions

1. **PDF Support**: Should we support reading existing resumes (PDF/DOCX) and parsing them? This adds significant complexity.
2. **Version Tracking**: How many resume versions to keep? Should we track which version was used for each application?
3. **Custom Templates**: Should users be able to customize resume formatting templates?
4. **Cover Letter Integration**: Should cover letter be generated as part of the same tailoring process?

---

## Dependencies

```toml
# lazyjob-core/Cargo.toml
[dependencies]
docx-rs = "0.4"              # Word document generation
regex = "1"                   # Text processing
thiserror = "2"
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# Optional PDF reading
# pdf-extract = "0.7"         # PDF text extraction
# docx2txt = "0.4"             # DOCX text extraction
```

---

## Sources

- [docx-rs GitHub](https://github.com/bokuweb/docx-rs)
- [docx-rs Documentation](https://docs.rs/docx-rs/latest/docx_rs/)
- [ATS Resume Parsing Guide - HireRight](https://hireright.com/blog/resume-tips/ats-applicant-tracking-system-resume-scanning)
- [Jobscan Resume Parsing Guide](https://www.jobscan.co/blog/resume-parsing/)
