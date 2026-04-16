# Spec: Resume Version Management System

## Context

Users generate multiple tailored resumes over time. Without management, versions proliferate and become unmanageable. This spec addresses resume version tracking, comparison, and lifecycle management.

## Motivation

- **Organization**: 20+ resume versions without management is chaos
- **Tracking**: Which version was sent to which company?
- **Comparison**: What changed between versions?
- **Branching**: Branch a version for different targeting strategies

## Design

### ResumeVersion Model

```rust
pub struct ResumeVersion {
    pub id: ResumeVersionId,
    pub name: String,                    // User-provided name, e.g., "Stripe - Engineering Manager v1"
    pub resume_id: ResumeId,             // Parent resume
    pub version_number: u32,             // Auto-increment per resume
    pub content: ResumeContent,          // The actual resume data
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: VersionSource,       // "user", "ralph_tailoring", "import"
    pub status: VersionStatus,
    pub sent_applications: Vec<ApplicationId>,  // Which apps used this version
}

pub enum VersionStatus {
    Draft,           // Not yet used
    Sent,            // Used in at least one application
    Archived,        // User archived this version
}

pub enum VersionSource {
    UserEdited,
    RalphTailoring { loop_id: Uuid },
    Import,
}
```

### Version Operations

#### Create New Version

```rust
pub struct ResumeVersionService {
    db: Database,
}

impl ResumeVersionService {
    /// Create new version from existing (branching)
    pub async fn create_from(
        &self,
        source_version: ResumeVersionId,
        name: String,
    ) -> Result<ResumeVersionId> {
        let source = self.get_version(source_version).await?;

        let new_version = ResumeVersion {
            id: ResumeVersionId::new(),
            name,
            resume_id: source.resume_id,
            version_number: source.version_number + 1,
            content: source.content.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::UserEdited,
            status: VersionStatus::Draft,
            sent_applications: vec![],
        };

        self.db.insert_version(&new_version).await?;
        Ok(new_version.id)
    }

    /// Create version from Ralph tailoring
    pub async fn create_from_tailoring(
        &self,
        resume_id: ResumeId,
        tailoring_result: TailoredResume,
        job_id: JobId,
    ) -> Result<ResumeVersionId> {
        let base_version = self.get_latest_version(resume_id).await?;

        let new_version = ResumeVersion {
            id: ResumeVersionId::new(),
            name: format!("Tailored for {}", job_id),
            resume_id,
            version_number: base_version.version_number + 1,
            content: tailoring_result.content,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::RalphTailoring { loop_id: tailoring_result.loop_id },
            status: VersionStatus::Draft,
            sent_applications: vec![],
        };

        self.db.insert_version(&new_version).await?;
        Ok(new_version.id)
    }
}
```

### Version Naming

Default naming convention with user customization:

```rust
pub fn default_version_name(company: &str, role: &str, version: u32) -> String {
    format!("{} - {} v{}", company, role, version)
}

// User can override with custom names
let version = ResumeVersion {
    name: "Google - Tech Lead - Senior Interview Prep".to_string(),
    // ...
};
```

### Version Comparison (Diff)

```rust
pub struct VersionDiff {
    pub from_version: ResumeVersionId,
    pub to_version: ResumeVersionId,
    pub sections_added: Vec<String>,
    pub sections_removed: Vec<String>,
    pub sections_modified: Vec<SectionDiff>,
    pub keyword_changes: KeywordChanges,
}

pub struct SectionDiff {
    pub section_name: String,
    pub before: String,
    pub after: String,
}

pub enum KeywordChanges {
    KeywordsAdded { keywords: Vec<String> },
    KeywordsRemoved { keywords: Vec<String> },
    KeywordsReordered,
}
```

**Diff algorithm**:
1. Parse both versions into structured sections
2. Compare section-by-section
3. Detect keyword additions/removals (for ATS optimization)
4. Show line-by-line diff for resume content

**TUI visualization**:
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Resume Version Diff: Stripe - SWE v1 → v2                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  - 5 years of experience                                                    │
│  + 6 years of experience                                                    │
│                                                                             │
│  - Built scalable APIs supporting 10M+ daily users                          │
│  + Built scalable APIs supporting 50M+ daily users                          │
│                                                                             │
│  SKILLS section:                                                            │
│  + [Kubernetes]                                                            │
│  - [Docker]                                                                 │
│                                                                             │
│  [Accept Changes]  [Reject Changes]  [View Full Diff]                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Version Tagging

```rust
pub struct VersionTag {
    pub id: TagId,
    pub name: String,           // e.g., "sent-to-stripe", "interview-prep"
    pub color: Color,          // For visual distinction
}

pub struct VersionTagging {
    pub version_id: ResumeVersionId,
    pub tags: Vec<TagId>,
}

impl ResumeVersionService {
    pub async fn tag_version(&self, version_id: ResumeVersionId, tag: &str) -> Result<()> {
        let tag_id = self.get_or_create_tag(tag).await?;
        self.db.add_tag_to_version(version_id, tag_id).await?;
        Ok(())
    }
}
```

### Version Cleanup

```rust
pub enum CleanupPolicy {
    KeepAll,                    // User manually deletes
    KeepLastN { count: u32 },   // Keep last N versions
    KeepSentVersions,          // Auto-archive unsent after 30 days
    AutoPruneAfterHiring,      // After offer accepted, keep only final version
}

impl ResumeVersionService {
    pub async fn apply_cleanup(&self, policy: &CleanupPolicy) -> Result<u32> {
        match policy {
            CleanupPolicy::KeepAll => Ok(0),  // Nothing to do
            CleanupPolicy::KeepLastN { count } => {
                self.prune_to_count(*count).await
            }
            CleanupPolicy::KeepSentVersions => {
                self.archive_unsent_older_than_days(30).await
            }
            CleanupPolicy::AutoPruneAfterHiring => {
                // Only for versions linked to accepted offers
                self.prune_accepted_offer_versions().await
            }
        }
    }
}
```

### Version → Application Linkage

```rust
pub struct ApplicationVersionLink {
    pub application_id: ApplicationId,
    pub resume_version_id: ResumeVersionId,
    pub sent_at: DateTime<Utc>,
    pub sent_via: SubmissionChannel,  // Email, Portal, LinkedIn
}

pub enum SubmissionChannel {
    Email,
    CompanyPortal,
    LinkedIn,
    Greenhouse,
    Lever,
}
```

## Implementation Notes

- Versions stored as JSON in SQLite (not separate files)
- Diff computed on-demand (not stored)
- Archive hides from default views but doesn't delete
- Can always unarchive

## Open Questions

1. **Version branching**: Should versions form a DAG or just a linear list?
2. **Template system**: Can a version be promoted to template?
3. **ATS-optimized variants**: Auto-generate ATS version alongside main version?

## Related Specs

- `profile-resume-tailoring.md` - Resume tailoring pipeline
- `XX-ats-specific-optimization.md` - ATS-specific optimization
- `application-workflow-actions.md` - Application submission