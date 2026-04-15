# Competitive Analysis

## Status
Researching

## Problem Statement

Understanding the competitive landscape helps position LazyJob correctly. This spec analyzes existing job search tools, their strengths, weaknesses, and opportunities for differentiation.

---

## Competitor Categories

### 1. Job Tracking Spreadsheets (Excel, Google Sheets)

**Pros**:
- Free
- Flexible
- Ubiquitous

**Cons**:
- Manual data entry
- No automation
- No AI assistance
- No structure

**Opportunity**: Much better UX + automation + AI

### 2. ATS Systems (Greenhouse, Lever, Workday)

**Pros**:
- Industry standard
- Rich data
- Integration with job boards

**Cons**:
- Enterprise-focused
- Designed for recruiters, not candidates
- No personal job search management
- Expensive

**Opportunity**: Personal ATS for individual job seekers

### 3. Job Search Apps (Huntr, JobHero)

**Pros**:
- Specifically for job seekers
- Mobile apps
- Basic tracking

**Cons**:
- Limited automation
- No AI assistance
- Basic analytics
- May lack depth

**Opportunity**: Deeper automation + AI-powered insights

### 4. Resume Builders (Teal, Kickresume, Resume.io)

**Pros**:
- Resume tailoring
- Template library
- Some AI features

**Cons**:
- Resume-focused only
- Limited job discovery
- Not comprehensive

**Opportunity**: Integration of job discovery + application tracking + resume

### 5. Career Coaches / Advisors

**Pros**:
- Human expertise
- Personalized guidance
- Accountability

**Cons**:
- Expensive
- Limited availability
- Not automated

**Opportunity**: AI-powered coaching at scale

---

## Key Competitor Profiles

### Huntr

**URL**: huntr.com
**Pricing**: Free + Pro ($15/mo)
**G2 Rating**: 4.6/5

**Features**:
- Job tracking kanban board
- Cloud sync
- Interview scheduling
- Offer comparison
- Chrome extension for LinkedIn

**Strengths**:
- Excellent UX
- Great mobile app
- Active development

**Weaknesses**:
- No AI assistance
- Limited automation
- US-focused

**LazyJob Differentiation**: AI-powered resume tailoring, company research, interview prep

### Teal

**URL**: tealhq.com
**Pricing**: Free + Pro ($15/mo)
**G2 Rating**: 4.7/5

**Features**:
- Job tracking
- Resume builder
- Company research
- Application tracking

**Strengths**:
- Clean interface
- Good company research
- Resume templates

**Weaknesses**:
- No AI features
- Limited ralph loop integration
- No local-first option

**LazyJob Differentiation**: Ralph autonomous agents, local-first, open source

### Lazygit (Inspiration)

**URL**: lazygit.dev
**Pricing**: Free, open source
**GitHub**: 35k stars

**Features**:
- Terminal UI
- Git management
- Custom commands
- Keybinding system

**Strengths**:
- Excellent UX
- Powerful yet simple
- Highly extensible

**Weaknesses**:
- Git only (not directly competitive)
- No AI features

**LazyJob Differentiation**: Job search focus, AI integration, not just git

### Notion (as a job tracker)

**URL**: notion.so
**Pricing**: Free + Pro ($10/mo)
**G2 Rating**: 4.5/5

**Features**:
- Flexible databases
- Templates
- Collaboration

**Strengths**:
- Very flexible
- Great for power users
- Active community

**Weaknesses**:
- Too flexible (no opinionated flow)
- No built-in job search features
- No AI

**LazyJob Differentiation**: Opinionated flow + AI automation

---

## Competitive Matrix

| Feature | Huntr | Teal | LazyJob | Spreadsheet |
|---------|-------|------|---------|------------|
| Job Discovery | Manual | Manual | Ralph auto | Manual |
| Resume Tailoring | ✗ | Basic | AI-powered | ✗ |
| Cover Letters | ✗ | Basic | AI-generated | ✗ |
| Company Research | Limited | Yes | Deep (Ralph) | ✗ |
| Interview Prep | Basic | Limited | AI-generated | ✗ |
| Salary Negotiation | ✗ | ✗ | AI strategy | ✗ |
| Local-first | ✗ | ✗ | ✓ | ✓ |
| Open Source | ✗ | ✗ | ✓ | ✓ |
| TUI Interface | ✗ | ✗ | ✓ | ✗ |
| AI Agents | ✗ | ✗ | ✓ | ✗ |

---

## Opportunities for LazyJob

1. **AI-First Architecture**: Ralph loops provide genuine automation, not just chatbots
2. **Local-First**: User owns their data, works offline
3. **TUI Innovation**: Developers love terminal tools; lazygit proved the market
4. **Open Source**: Community contributions, transparency, trust

---

## Threats

1. **Huntr/Teal adding AI**: Could quickly add AI features
2. **OpenAI releasing job search agent**: Vertical integration risk
3. **LinkedIn adding ATS features**: Platform risk

---

## Sources

- [Huntr Reviews - G2](https://www.g2.com/products/huntr/reviews)
- [Teal Reviews - G2](https://www.g2.com/products/teal-hq/reviews)
- [Lazygit GitHub](https://github.com/jesseduffield/lazygit)
- [Notion Job Tracker Templates](https://notion.so)
