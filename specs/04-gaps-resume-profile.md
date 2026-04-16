# Gap Analysis: Resume/Profile (07, profile-*, resume-optimization specs)

## Specs Reviewed
- `07-resume-tailoring-pipeline.md` - Resume Tailoring Pipeline
- `03-life-sheet-data-model.md` - Life Sheet Data Model
- `profile-life-sheet-data-model.md` - Profile Life Sheet Data Model
- `profile-resume-tailoring.md` - Resume Tailoring Pipeline Spec
- `profile-skills-gap-analysis.md` - Skills Gap Analysis
- `resume-optimization.md` - Resume Optimization & ATS Systems (research doc)

---

## What's Well-Covered

### resume-optimization.md (Research)
- ATS market landscape (Workday 39%, iCIMS, Greenhouse, Lever)
- ATS parsing technical realities (DOCX vs PDF, two-column layouts)
- Auto-rejection myth debunked (92% of ATS don't auto-reject)
- Tailoring conversion data (5.75-5.8% vs 2.68-3.73%)
- Competitor landscape (Jobscan, Teal, Resume Worded, Rezi, Huntr)
- Agentic opportunity analysis (Level 1-3 framework)
- Truthfulness guardrails concept
- Voice preservation concept

### profile-skills-gap-analysis.md
- Gap computation architecture (4 steps)
- Career transitioner transferable skill inference
- Skill normalization (lowercase, alias table, embedding similarity)
- GapSeverity classification (Critical, Significant, Minor)
- Prioritization algorithm (frequency * required_multiplier * career_relevance)
- Heat map display design
- Cache TTL strategy

### profile-life-sheet-data-model.md
- Dual-layer architecture (YAML + SQLite)
- ESCO/O*NET skill codes
- Deterministic ID proposal for stable re-imports
- profile_contacts vs application_contacts separation
- GitHub integration as future work
- Multi-variant LifeSheet deferred

### profile-resume-tailoring.md
- 6-stage pipeline design
- FabricationLevel enum (Safe, Acceptable, Risky, Forbidden)
- Voice preservation via few-shot examples
- Keyword density targeting (Tier 1-3)
- ResumeVersion tracking with FK to applications
- DOCX generation via docx-rs

### 07-resume-tailoring-pipeline.md
- JD parsing with LLM fallback to TF-IDF
- Gap analysis pure Rust computation
- Fabrication guardrails
- DOCX generation
- Version tracking

---

## Critical Gaps: What's Missing or Glossed Over

### GAP-39: ATS-Specific Resume Optimization (CRITICAL)

**Location**: All resume specs - generic ATS-safe format mentioned but no ATS-specific optimization

**What's missing**:
1. **Per-ATS optimization**: Workday parses differently than Greenhouse vs Lever. Specific optimizations for each?
2. **Greenhouse specifics**: Greenhouse uses keyword matching heavily, exact terminology matters
3. **Workday specifics**: Workday has strict character limits, different section parsing
4. **Lever specifics**: Lever favors certain formats, less strict than Workday
5. **ATS parse simulation**: Can we test how our resume parses before sending?
6. **Format detection**: How does LazyJob know which ATS a company uses?

**Why critical**: Different ATS systems parse resumes differently. A "generic ATS-safe" resume may not optimize for any specific system.

**What could go wrong**:
- User submits to Workday with overly long section headers, parsing fails
- Keywords in wrong section don't get indexed properly
- Company uses Greenhouse but resume optimized for Workday format

---

### GAP-40: Resume Version Management System (CRITICAL)

**Location**: `profile-resume-tailoring.md` - ResumeVersion defined but no management UI/system

**What's missing**:
1. **Version naming**: Can users name versions? (e.g., "Engineering Manager - Stripe v1")
2. **Version comparison**: Can user see diff between two versions? (e.g., what changed v1 → v2)
3. **Version branching**: Can user branch off an existing version for a different target?
4. **Version tagging**: Can user tag versions as "sent to X company"?
5. **Version cleanup**: How many versions to keep? Auto-prune old versions?
6. **Version templates**: Can user use a version as template for another job?

**Why critical**: Users generate multiple tailored resumes over time. Without management, versions proliferate and become unmanageable.

**What could go wrong**:
- User has 47 resume versions, no idea which was sent where
- Can't compare "what changed between v2 and v3"
- Sent wrong version to company, no way to track

---

### GAP-41: Multiple Resume Targets Per Job (IMPORTANT)

**Location**: All resume specs - one resume per job assumed

**What's missing**:
1. **Different angles for same job**: "Senior Engineer" vs "Tech Lead" versions of same job
2. **Role switching**: Same job, different focus (backend-heavy vs frontend-heavy)
3. **Version comparison**: A/B testing different approaches to same job
4. **Tracking which angle led to interview**: Feedback loop for targeting strategy
5. **Storage**: How to store multiple tailored versions per job application?

**Why important**: Same job can be targeted from different angles. User may want to experiment.

**What could go wrong**:
- User wants to apply to same job with different positioning, no way to manage
- Can't track which approach (technical vs leadership focus) worked better

---

### GAP-42: Automated Achievement Extraction from Work History (IMPORTANT)

**Location**: `03-life-sheet-data-model.md` - achievements defined but no extraction strategy

**What's missing**:
1. **LLM extraction from job descriptions**: Given a job description, extract quantifiable achievements
2. **Resume bullet generation**: Generate strong resume bullets from job descriptions + user input
3. **Metric normalization**: "Reduced latency by 40%" vs "Improved performance by 40%" - same metric?
4. **Achievement library**: Should achievements be stored separately, reusable across jobs?
5. **Evidence quality scoring**: Some achievements have strong metrics, others vague

**Why important**: Most users don't quantify their achievements. AI can help extract/identify quantifiable results from their work history.

**What could go wrong**:
- AI generates vague "improved performance" metrics with no actual numbers
- Achievements extracted incorrectly, user fabricates without realizing
- Users don't update achievement library as they complete new projects

---

### GAP-43: Master Resume Cold Start (IMPORTANT)

**Location**: `profile-life-sheet-data-model.md` - Open Question mentions this

**What's missing**:
1. **LinkedIn import**: Parse LinkedIn profile export (HTML/CSV) into LifeSheet
2. **Existing resume parsing**: Parse PDF/DOCX resume to extract LifeSheet structure
3. **Conversational bootstrap**: User describes career in chat, AI structures into LifeSheet
4. **GitHub integration**: Pull repos, contributions, from GitHub API
5. **Progressive enrichment**: Start with minimal data, enrich over time

**Why important**: Users without a structured LifeSheet can't use LazyJob. Cold start is critical for adoption.

**What could go wrong**:
- User has to manually enter everything, abandons product
- Import loses nuance/context from original documents
- AI extraction from unstructured text has errors user doesn't catch

---

### GAP-44: Resume Template System (MODERATE)

**Location**: All resume specs - mentions docx-rs but no template system

**What's missing**:
1. **Template selection**: Multiple resume templates (ATS minimal, clean modern, executive)
2. **Template customization**: Can user modify fonts, colors, spacing?
3. **Template versioning**: User's custom templates versioned alongside built-in ones
4. **Template preview**: WYSIWYG preview before generating DOCX
5. **Template marketplace**: Community-shared templates?
6. **Brand一致的**: Templates that match user's personal brand

**Why important**: Resume appearance matters for certain industries/roles.

**What could go wrong**:
- User stuck with one template they don't like
- Custom template breaks ATS parsing
- Template too visually distinct, hurts rather than helps

---

### GAP-45: Resume Feedback Loop Tracking (MODERATE)

**Location**: `resume-optimization.md` - mentions tracking which version led to interviews

**What's missing**:
1. **Application → Resume linkage**: When application created, link which ResumeVersion used
2. **Interview outcome feedback**: Did this resume lead to an interview? Offer?
3. **Version effectiveness scoring**: Which resume characteristics correlate with success?
4. **A/B testing infrastructure**: Can user run experiments with different resumes?
5. **Learning from outcomes**: Use outcomes to improve future tailoring

**Why important**: Without feedback, can't improve tailoring strategy over time.

**What could go wrong**:
- User doesn't know which resume version led to interview
- No way to learn "technical positioning works better than leadership for this company"
- Feedback loop never closed, product doesn't improve

---

### GAP-46: PDF Export Pipeline (MODERATE)

**Location**: `profile-resume-tailoring.md` - mentions DOCX only, PDF as future

**What's missing**:
1. **DOCX → PDF conversion**: LibreOffice headless? Cloud API? Pure Rust?
2. **PDF quality**: Does PDF conversion preserve formatting?
3. **PDF/A compatibility**: Long-term archive format?
4. **ATS compatibility of PDF**: Is PDF worse than DOCX for ATS?
5. **Hybrid approach**: Submit DOCX to ATS, PDF to hiring manager email

**Why important**: Some recruiters/hiring managers prefer PDF. Some situations require PDF.

**What could go wrong**:
- PDF conversion breaks formatting
- PDF doesn't parse well in ATS, user doesn't know
- Multiple formats to manage, confusing

---

### GAP-47: Voice and Writing Style Preservation (MODERATE)

**Location**: `profile-resume-tailoring.md` - mentions "few-shot examples" but implementation vague

**What's missing**:
1. **Writing style extraction**: Analyze user's existing resume/experience descriptions for style patterns
2. **Style fingerprint**: Metrics on formality, sentence length, vocabulary level
3. **Style consistency checking**: Does generated content match user's style?
4. **Voice template**: Store user's voice as template for generation
5. **Anti-uniformity detection**: Is generated content too generic/similar to other AI resumes?

**Why important**: AI-generated resumes all start sounding the same. Voice preservation differentiates.

**What could go wrong**:
- Generated resume sounds like every other AI resume
- User's unique voice not captured, loses personal brand
- Over-optimization removes authenticity

---

### GAP-48: Incremental LifeSheet Sync (MODERATE)

**Location**: `profile-life-sheet-data-model.md` - Open Question

**What's missing**:
1. **YAML diff detection**: What changed between current YAML and last import?
2. **Selective update**: Only update changed entities, not full truncate
3. **Entity ID stability**: Deterministic IDs based on (company + position + start_date)
4. **Application impact analysis**: If experience ID changes, what happens to linked applications?
5. **Conflict resolution**: What if YAML edited while DB being imported?

**Why important**: Full re-import is slow and invalidates pointers. Incremental is more efficient.

**What could go wrong**:
- Full re-import breaks application links to experience IDs
- Incremental diff misses subtle changes
- Entity merge conflicts not handled

---

## Cross-Spec Gaps

### Cross-Spec I: Resume Version ↔ LifeSheet Sync

When LifeSheet is re-imported:
- Should old ResumeVersions be invalidated?
- Can ResumeVersions still reference old experience IDs?
- Should user be warned that LifeSheet changed since version was created?

**Affected specs**: `profile-life-sheet-data-model.md`, `profile-resume-tailoring.md`

### Cross-Spec J: Fabrication Detection Integration

Fabrication detection needs to integrate across:
- Resume tailoring output
- Cover letter output
- Any future generated content

Should be a shared `fabrication.rs` module, not duplicated per feature.

**Affected specs**: `profile-resume-tailoring.md`, `08-cover-letter-generation.md`, (not yet read)

---

## Specs to Create

### Critical Priority

1. **XX-ats-specific-optimization.md** - Per-ATS (Greenhouse, Workday, Lever) optimization strategies, parse testing
2. **XX-resume-version-management.md** - Version naming, comparison, branching, tagging, cleanup

### Important Priority

3. **XX-resume-multi-target.md** - Multiple targeting strategies per job, A/B testing, tracking
4. **XX-achievement-extraction.md** - Automated extraction of quantifiable achievements from work history
5. **XX-life-sheet-cold-start.md** - LinkedIn import, resume parsing, conversational bootstrap

### Moderate Priority

6. **XX-resume-template-system.md** - Template selection, customization, preview, marketplace
7. **XX-resume-feedback-loop.md** - Application-resume linkage, outcome tracking, effectiveness scoring
8. **XX-pdf-export-pipeline.md** - DOCX to PDF conversion, quality preservation, ATS compatibility
9. **XX-resume-voice-preservation.md** - Writing style extraction, fingerprinting, anti-uniformity
10. **XX-lifesheet-incremental-sync.md** - Diff detection, selective update, conflict resolution

---

## Prioritization Summary

| Gap | Priority | Effort | Impact |
|-----|----------|--------|--------|
| GAP-39: ATS-Specific Optimization | Critical | High | Core value proposition |
| GAP-40: Resume Version Management | Critical | Medium | UX/efficiency |
| GAP-41: Multi-Target Per Job | Important | Medium | Flexibility |
| GAP-42: Achievement Extraction | Important | High | Content quality |
| GAP-43: Master Resume Cold Start | Important | High | User acquisition |
| GAP-44: Resume Template System | Moderate | Medium | User experience |
| GAP-45: Resume Feedback Loop | Moderate | Medium | Product improvement |
| GAP-46: PDF Export Pipeline | Moderate | Medium | Format flexibility |
| GAP-47: Voice Preservation | Moderate | Medium | Differentiation |
| GAP-48: Incremental LifeSheet Sync | Moderate | Low | Performance |
