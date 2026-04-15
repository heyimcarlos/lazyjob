# Life Sheet Data Model

## Status
Researching

## Problem Statement

The "Life Sheet" is LazyJob's name for a comprehensive job seeker profile. Unlike a traditional resume which is a static document, the Life Sheet is a structured data repository that:
1. Contains all career information (experience, education, skills)
2. Is machine-readable for AI processing
3. Is human-editable (YAML format for users)
4. Supports rich metadata (context, outcomes, relationships to job applications)
5. Can be mapped to standard formats (JSON Resume, PDF resumes)

This spec defines both the user-facing YAML schema and the SQLite-backed data model.

---

## Research Findings

### JSON Resume Schema

The JSON Resume standard (`resume-schema`) is a community-driven open source initiative creating a JSON-based resume standard. It provides a good starting point.

**Basics Section**
```json
{
  "name": "John Doe",
  "label": "Software Engineer",
  "image": "https://...",
  "email": "john@example.com",
  "phone": "(555) 123-4567",
  "url": "https://johndoe.com",
  "summary": "Experienced software engineer...",
  "location": {
    "address": "2712 Broadway St",
    "postalCode": "CA 94115",
    "city": "San Francisco",
    "countryCode": "US",
    "region": "California"
  },
  "profiles": [
    { "network": "GitHub", "username": "johndoe", "url": "https://github.com/johndoe" }
  ]
}
```

**Work Section**
```json
{
  "name": "Company",
  "position": "Software Engineer",
  "location": "San Francisco, CA",
  "url": "https://company.com",
  "startDate": "2020-01-01",
  "endDate": "2023-06-30",
  "summary": "Built...",
  "highlights": [
    "Improved performance by 50%",
    "Led team of 5 engineers"
  ]
}
```

**Education Section**
```json
{
  "institution": "MIT",
  "url": "https://mit.edu",
  "area": "Computer Science",
  "studyType": "Bachelor",
  "startDate": "2016-09-01",
  "endDate": "2020-05-01",
  "score": "3.8",
  "courses": ["CS101", "CS201"]
}
```

**Skills Section**
```json
{
  "name": "Programming Languages",
  "level": "Advanced",
  "keywords": ["Rust", "Python", "TypeScript"]
}
```

**Limitations of JSON Resume**:
- Flat `highlights` array - no structure for achievement types
- No support for skills taxonomy mapping (ESCO, O*NET codes)
- No support for application context (which jobs each experience is relevant to)
- No support for ongoing learning/certifications
- No rich metadata on work experience quality/outcomes

### ESCO (European Skills, Competences, Qualifications and Occupations)

ESCO is the EU's multilingual classification of skills, competencies, qualifications, and occupations. It provides:
- **Skills**: Transversal and occupation-specific skills with preferred labels in 27 EU languages
- **Occupations**: 294 occupation groups, 2,966 occupations mapped to ISCO-08
- **Qualifications**: References to national qualifications frameworks

**Key Features**:
- URI-based identification (e.g., `http://data.europa.eu/esco/skill/abc123`)
- Hierarchical structure (skill → skill cluster)
- Linked to occupations (which skills are relevant to which jobs)
- Free, open API available

**Data Format**:
```json
{
  "uri": "http://data.europa.eu/esco/skill/abc123",
  "title": "JavaScript",
  "description": "...",
  "type": "technical",
  "group": "programming-languages",
  "preferredLabel": "JavaScript",
  "alternativeLabels": ["JS", "ECMAScript"]
}
```

### O*NET (Occupational Information Network)

O*NET is the US Department of Labor's occupational database. Key components:

**Content Model Categories**:
- **Abilities**: Enduring attributes of the individual that influence performance
- **Skills**: Developed capacities that facilitate learning
- **Knowledge**: Organized sets of facts/principles
- **Work Activities**: Generalized work activities
- **Work Context**: Physical and social work environment
- **Tasks**: Specific work activities
- **Tools & Technology**: Equipment used

**Scale Types**:
- Level (1-7) for abilities, skills
- Importance (1-5) for knowledge, tasks
- Frequency for work activities

**Example Skill**:
```json
{
  "title": "Programming",
  "category": "Skill",
  "description": "Writing computer programs...",
  "importance": 89,
  "level": 67
}
```

**O*NET-SOC Taxonomy**:
- 1,016 occupational titles
- 923 data-level occupations
- Updated quarterly

### LinkedIn Data Export

LinkedIn exports profile data as:
- **Profile**: Name, headline, summary, location, connections count
- **Positions**: Company, title, dates, description (markdown)
- **Education**: School, degree, field, dates
- **Skills**: Name, endorsements count
- **Recommendations**: Given/received
- **Languages**: Proficiency level
- **Certifications**: Name, authority, dates

**Format**: HTML with some CSV attachments

---

## Design Options

### Option A: Flat JSON Resume Extension

**Description**: Extend JSON Resume schema with minimal additions for LazyJob-specific fields.

**Pros**:
- Familiar, well-documented schema
- Easy export to existing resume templates
- Good tooling ecosystem (validators, themes)

**Cons**:
- Limited expressiveness for complex profiles
- No taxonomy mapping capability
- Hard to extend without breaking compatibility

**Best for**: Quick implementation, compatibility with existing tools

### Option B: Rich Domain Model (Recommended)

**Description**: Comprehensive domain model with:
- Rich entities (Experience with Context, Achievement with Metrics)
- Taxonomy mapping (ESCO/O*NET skill codes)
- Application linking (which jobs each item is relevant to)
- YAML for human editing, SQLite for programmatic access

**Pros**:
- Full expressiveness for job search scenarios
- Taxonomy mapping enables smart matching
- Clear separation of concerns
- Supports both human editing and programmatic access

**Cons**:
- More complex than JSON Resume
- Custom schema means no off-the-shelf tooling

**Best for**: Production LazyJob with AI-powered features

### Option C: Graph-based Model

**Description**: Use a property graph model (like Neo4j or PostgreSQL graph) to model career history as interconnected nodes.

**Pros**:
- Natural representation of career progression
- Rich relationship queries
- Can model complex career transitions

**Cons**:
- Overkill for single-user local tool
- Graph DB adds complexity
- Harder to serialize to YAML

**Best for**: Large-scale job market analysis platforms

---

## Recommended Approach

**Option B: Rich Domain Model** is recommended.

Rationale:
- LazyJob needs taxonomy mapping for smart job matching
- Application linking is core to the product
- YAML for human editing improves user experience
- SQLite for programmatic access enables AI processing

---

## YAML Schema (User-Editable Format)

```yaml
# LazyJob Life Sheet
# Human-editable YAML format
# Saved at ~/.lazyjob/life-sheet.yaml

meta:
  version: "1.0"
  created_at: "2024-01-15"
  updated_at: "2024-06-20"

basics:
  name: "Jane Smith"
  label: "Senior Software Engineer"
  email: "jane@example.com"
  phone: "+1 (555) 123-4567"
  url: "https://janesmith.dev"
  location:
    city: "San Francisco"
    region: "California"
    country: "US"
    remote: true  # LazyJob extension
  summary: |
    Senior software engineer with 8+ years building distributed systems.
    Previously at Stripe and Airbnb. Passionate about developer tools
    and programming language design.
  profiles:
    - network: "GitHub"
      username: "janesmith"
      url: "https://github.com/janesmith"
    - network: "LinkedIn"
      username: "janesmith"
      url: "https://linkedin.com/in/janesmith"
    - network: "Twitter"
      username: "@janesmith"
      url: "https://twitter.com/janesmith"

experience:
  - company: "Stripe"
    position: "Senior Software Engineer"
    location: "San Francisco, CA"
    url: "https://stripe.com"
    start_date: "2021-03"
    end_date: "2024-01"  # null if current
    current: false
    summary: |
      Led the API Gateway team, responsible for routing and rate limiting
      for all Stripe API requests. Managed a team of 6 engineers.
    context:
      team_size: 6
      org_size: 5000
      industry: "FinTech"
      tech_stack: ["Go", "Ruby", "PostgreSQL", "Redis"]
    achievements:
      - description: "Reduced API latency by 40%"
        metrics:
          improvement_percent: 40
          timeframe_months: 6
        evidence: "Internal metrics dashboard"
      - description: "Designed new rate limiting system"
        metrics:
          requests_per_second: "1M+"
          uptime_percent: 99.99
      - description: "Onboarded 4 new engineers to the team"
        metrics:
          onboarding_time_reduction_months: 2
    skills:
      esco_codes: ["escoskill:programming", "escoskill:team-leadership"]
      onet_codes: ["15-1132.00"]  # Software Developers, Applications
    relevance_tags: ["backend", "distributed-systems", "api-design"]
    job_applications: []  # Will be populated by LazyJob

  - company: "Airbnb"
    position: "Software Engineer"
    location: "San Francisco, CA"
    url: "https://airbnb.com"
    start_date: "2018-06"
    end_date: "2021-02"
    current: false
    summary: |
      Full-stack engineer on the Payments team. Built features for
      guest refund processing and host payouts.
    context:
      team_size: 4
      org_size: 10000
      industry: "Marketplace"
      tech_stack: ["Ruby on Rails", "React", "MySQL", "Kafka"]
    achievements:
      - description: "Built guest refund processing system"
        metrics:
          transactions_per_day: "100K+"
          error_rate_percent: 0.01
    skills:
      ets_codes: []
    relevance_tags: ["payments", "fullstack", "marketplace"]

education:
  - institution: "Stanford University"
    url: "https://stanford.edu"
    degree: "Master of Science"
    field: "Computer Science"
    area: "Artificial Intelligence"
    start_date: "2016-09"
    end_date: "2018-06"
    score: "3.8"
    thesis: "Optimizing Neural Architecture Search"
    courses:
      - "CS224n: Natural Language Processing with Deep Learning"
      - "CS229: Machine Learning"
      - "CS231n: Convolutional Neural Networks"
    relevant_tags: ["ai-ml", "research"]

  - institution: "UC Berkeley"
    url: "https://berkeley.edu"
    degree: "Bachelor of Science"
    field: "Electrical Engineering and Computer Science"
    start_date: "2012-09"
    end_date: "2016-05"
    score: "3.7"
    relevant_tags: ["systems", "programming-languages"]

skills:
  - name: "Programming Languages"
    level: "Advanced"
   ESCO_codes: ["escoskill:programming"]
    keywords:
      - name: "Python"
        years: 8
        proficiency: "expert"
      - name: "Go"
        years: 4
        proficiency: "advanced"
      - name: "Rust"
        years: 2
        proficiency: "intermediate"
      - name: "TypeScript"
        years: 5
        proficiency: "advanced"

  - name: "Frameworks & Tools"
    level: "Advanced"
    keywords:
      - name: "React"
        years: 5
      - name: "Django"
        years: 4
      - name: "PostgreSQL"
        years: 6
      - name: "Redis"
        years: 4
      - name: "Kubernetes"
        years: 3

  - name: "Machine Learning"
    level: "Intermediate"
    keywords:
      - name: "PyTorch"
        years: 3
      - name: "TensorFlow"
        years: 2
      - name: "Hugging Face Transformers"
        years: 2

certifications:
  - name: "AWS Solutions Architect Professional"
    authority: "Amazon Web Services"
    date: "2023-06"
    credential_id: "ABC123"
    url: "https://aws.amazon.com/verification/ABC123"
    expires: "2026-06"

  - name: "Google Cloud Professional Data Engineer"
    authority: "Google"
    date: "2022-09"
    credential_id: "DEF456"

languages:
  - name: "English"
    proficiency: "native"
  - name: "Mandarin Chinese"
    proficiency: "conversational"
  - name: "Spanish"
    proficiency: "intermediate"

projects:
  - name: "Open Source CLI Tool"
    description: |
      A command-line tool for managing dotfiles with plugin support.
      2,000+ GitHub stars.
    url: "https://github.com/janesmith/dotfiles"
    start_date: "2020-01"
    end_date: "2020-06"
    highlights:
      - "2,000+ GitHub stars"
      - "Featured in Hacker News"
    skills: ["Rust", "CLI", "Open Source"]

preferences:
  job_types: ["full-time", "contract"]
  locations:
    - city: "San Francisco"
      region: "California"
      country: "US"
      remote_ok: true
    - city: "New York"
      region: "New York"
      country: "US"
      remote_ok: true
  industries:
    - "FinTech"
    - "Developer Tools"
    - "AI/ML"
  salary:
    currency: "USD"
    min: 200000
    max: 350000
    base_or_total: "base"
  notice_period_weeks: 4
  visa_sponsorship: false

goals:
  short_term: "Transition into AI/ML engineering role at a well-funded startup"
  long_term: "Start a developer tools company"
  timeline: "12-18 months"

contact_network:
  - name: "Sarah Johnson"
    relationship: "former-manager"
    company: "Stripe"
    email: "sarah@example.com"
    linkedin: "https://linkedin.com/in/sarahjohnson"
    notes: "Great mentor, still in touch"
```

---

## SQLite Data Model

```sql
-- Life Sheet Tables (extends basic Job/Application tables)

CREATE TABLE life_sheet_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE personal_info (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    label TEXT,
    email TEXT,
    phone TEXT,
    url TEXT,
    summary TEXT,
    city TEXT,
    region TEXT,
    country TEXT,
    remote_preference INTEGER DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE work_experience (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    company_name TEXT NOT NULL,
    position TEXT NOT NULL,
    location TEXT,
    company_url TEXT,
    start_date TEXT,
    end_date TEXT,
    is_current INTEGER DEFAULT 0,
    summary TEXT,
    team_size INTEGER,
    org_size INTEGER,
    industry TEXT,
    tech_stack TEXT,  -- JSON array
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE achievement (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    experience_id TEXT NOT NULL,
    description TEXT NOT NULL,
    metric_type TEXT,  -- 'percent', 'absolute', 'currency', 'duration'
    metric_value REAL,
    metric_unit TEXT,
    evidence TEXT,
    FOREIGN KEY (experience_id) REFERENCES work_experience(id) ON DELETE CASCADE
);

CREATE TABLE education (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    institution TEXT NOT NULL,
    institution_url TEXT,
    degree TEXT,
    field TEXT,
    area TEXT,
    start_date TEXT,
    end_date TEXT,
    score TEXT,
    thesis TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE course (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    education_id TEXT NOT NULL,
    name TEXT NOT NULL,
    FOREIGN KEY (education_id) REFERENCES education(id) ON DELETE CASCADE
);

CREATE TABLE skill_category (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    level TEXT,  -- 'Beginner', 'Intermediate', 'Advanced', 'Expert'
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE skill (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    category_id TEXT NOT NULL,
    name TEXT NOT NULL,
    years_experience INTEGER,
    proficiency TEXT,  -- 'beginner', 'intermediate', 'advanced', 'expert'
    esco_code TEXT,  -- ESCO skill URI
    onet_code TEXT,  -- O*NET code
    FOREIGN KEY (category_id) REFERENCES skill_category(id) ON DELETE CASCADE
);

CREATE TABLE certification (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    authority TEXT,
    issue_date TEXT,
    expiry_date TEXT,
    credential_id TEXT,
    url TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE language (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    proficiency TEXT NOT NULL,  -- 'native', 'fluent', 'professional', 'conversational', 'basic'
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE project (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    description TEXT,
    url TEXT,
    start_date TEXT,
    end_date TEXT,
    highlights TEXT,  -- JSON array
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE project_skill (
    project_id TEXT NOT NULL,
    skill_name TEXT NOT NULL,
    PRIMARY KEY (project_id, skill_name),
    FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);

CREATE TABLE profile (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    personal_info_id TEXT NOT NULL,
    network TEXT NOT NULL,  -- 'GitHub', 'LinkedIn', 'Twitter', etc.
    username TEXT,
    url TEXT,
    FOREIGN KEY (personal_info_id) REFERENCES personal_info(id) ON DELETE CASCADE
);

CREATE TABLE job_preferences (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    job_types TEXT NOT NULL,  -- JSON array: ['full-time', 'contract']
    locations TEXT,  -- JSON array of location objects
    industries TEXT,  -- JSON array
    salary_currency TEXT DEFAULT 'USD',
    salary_min INTEGER,
    salary_max INTEGER,
    base_or_total TEXT DEFAULT 'base',
    notice_period_weeks INTEGER DEFAULT 2,
    visa_sponsorship INTEGER DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE career_goal (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    short_term TEXT,
    long_term TEXT,
    timeline TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE contact (
    id TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    name TEXT NOT NULL,
    relationship TEXT,  -- 'former-manager', 'colleague', 'mentor', 'recruiter', etc.
    company TEXT,
    email TEXT,
    linkedin_url TEXT,
    twitter_handle TEXT,
    notes TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes for common queries
CREATE INDEX idx_experience_current ON work_experience(is_current) WHERE is_current = 1;
CREATE INDEX idx_skill_esco ON skill(esco_code) WHERE esco_code IS NOT NULL;
CREATE INDEX idx_skill_onet ON skill(onet_code) WHERE onet_code IS NOT NULL;
CREATE INDEX idx_contact_relationship ON contact(relationship);
```

---

## Conversion Between Formats

### YAML to SQLite

```rust
fn import_life_sheet(yaml: &LifeSheetYaml) -> Result<()> {
    // Insert personal_info
    let personal_id = insert_personal_info(&yaml.basics)?;

    // Insert work experiences
    for exp in &yaml.experience {
        let exp_id = insert_work_experience(personal_id, exp)?;
        for achievement in &exp.achievements {
            insert_achievement(exp_id, achievement)?;
        }
    }

    // Insert skills with taxonomy codes
    for category in &yaml.skills {
        let cat_id = insert_skill_category(personal_id, category)?;
        for skill in &category.keywords {
            insert_skill(cat_id, skill)?;
        }
    }

    // ... similar for education, certifications, etc.
}
```

### SQLite to JSON Resume

```rust
fn export_json_resume(personal_id: &str) -> Result<JsonResume> {
    let basics = get_personal_info(personal_id)?;
    let work = get_work_experiences(personal_id)?;
    let education = get_education(personal_id)?;
    let skills = get_skills(personal_id)?;

    Ok(JsonResume {
        basics: Basics {
            name: basics.name,
            label: basics.label,
            email: basics.email,
            phone: basics.phone,
            url: basics.url,
            summary: basics.summary,
            location: Location {
                city: basics.city,
                region: basics.region,
                countryCode: basics.country,
                ..Default::default()
            },
            profiles: get_profiles(personal_id)?,
        },
        work: work.into_iter().map(|e| WorkEntry {
            name: e.company_name,
            position: e.position,
            location: e.location,
            startDate: e.start_date,
            endDate: e.end_date,
            summary: e.summary,
            highlights: get_achievement_descriptions(&e.id)?,
            ..Default::default()
        }).collect(),
        education: education.into_iter().map(|e| EducationEntry {
            institution: e.institution,
            area: e.field,
            studyType: e.degree,
            courses: get_courses(&e.id)?,
            ..Default::default()
        }).collect(),
        skills: build_skill_arrays(skills),
        ..Default::default()
    })
}
```

---

## Open Questions

1. **YAML Validation**: Should we validate ESCO/O*NET codes against official taxonomies? This requires API calls.
2. **Resume Versioning**: Should users be able to maintain multiple resume variants (e.g., "Engineering" vs "Management" focus)?
3. **LinkedIn Import**: Should we offer direct LinkedIn profile import? (Technically possible but against ToS for scraping)
4. **GitHub Integration**: GitHub has an API - should we pull repositories and contributions automatically?
5. **Partial Updates**: If user edits YAML, should we do a full re-import or incremental updates?

---

## Dependencies

- **serde_yaml** or **yaml-rust2** for YAML parsing
- **rusqlite** with JSON extension for SQLite
- **reqwest** for ESCO/O*NET API lookups (optional)
- **chrono** for date handling

---

## Sources

- [JSON Resume Schema](https://docs.jsonresume.org/schema)
- [JSON Resume GitHub](https://github.com/jsonresume/resume-schema)
- [ESCO Portal](https://esco.ec.europa.eu/)
- [O*NET Content Model](https://www.onetcenter.org/overview.html)
- [O*NET Database](https://www.onetcenter.org/db.html)
