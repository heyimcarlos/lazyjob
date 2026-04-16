# Spec: Cover Letter Version Tracking and Sent-State Management

## Context

Cover letters are revised multiple times. Without tracking which version was sent where, users lose visibility. This spec addresses cover letter version lifecycle and sent-state management.

## Motivation

- **Organization**: 10+ cover letter versions without management is chaos
- **Mistake prevention**: Sending wrong version ("excited about Google" in Meta letter)
- **Follow-up reference**: Need to reference what was sent in follow-up emails

## Design

### CoverLetterVersion Model

```rust
pub struct CoverLetterVersion {
    pub id: CoverLetterVersionId,
    pub name: String,                    // User-provided or auto-generated
    pub application_id: Option<ApplicationId>,
    pub content: CoverLetterContent,     // Structured content
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: VersionSource,
    pub status: VersionStatus,
    pub sent_via: Option<SentChannel>,
    pub sent_at: Option<DateTime<Utc>>,
}

pub enum VersionStatus {
    Draft,
    Sent,
    Archived,
}

pub enum VersionSource {
    UserEdited,
    RalphGeneration { loop_id: Uuid },
    Import,
}

pub enum SentChannel {
    Email { to: String, subject: String },
    CompanyPortal,
    LinkedIn,
    Greenhouse,
    Lever,
}
```

### Sent State Tracking

```rust
pub struct CoverLetterService {
    db: Database,
}

impl CoverLetterService {
    pub async fn mark_sent(
        &self,
        version_id: CoverLetterVersionId,
        channel: SentChannel,
    ) -> Result<()> {
        let now = Utc::now();

        self.db.update_version_sent(version_id, now, channel).await?;

        // If linked to application, update application status
        if let Some(app_id) = self.get_version(version_id).await?.application_id {
            self.db.update_application_contact(
                app_id,
                OutreachAction::CoverLetterSent { version_id, channel: channel.clone() }
            ).await?;
        }

        Ok(())
    }
}
```

### Version Comparison (Diff)

```rust
pub struct VersionDiff {
    pub from_version: CoverLetterVersionId,
    pub to_version: CoverLetterVersionId,
    pub paragraph_changes: Vec<ParagraphDiff>,
}

pub struct ParagraphDiff {
    pub index: usize,
    pub change_type: ChangeType,
    pub before: String,
    pub after: String,
}

pub enum ChangeType {
    Added,
    Removed,
    Modified,
}
```

**TUI visualization**:
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Cover Letter Version Diff: Stripe v1 → v2                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Paragraph 1:                                                               │
│  - I am excited to apply for the Senior Software Engineer role at Stripe.   │
│  + I am thrilled to apply for the Staff Software Engineer position at      │
│    Stripe, where I can contribute to [specific team].                      │
│                                                                             │
│  Paragraph 3 (Added):                                                      │
│  + My experience at Figma building collaborative design tools aligns with  │
│  + Stripe's mission to empower everyone to build.                           │
│                                                                             │
│  [Accept New Version]  [Keep v1]  [View Full Version]                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Version Branching

```rust
impl CoverLetterService {
    /// Create new version from existing
    pub async fn create_from(
        &self,
        source_version: CoverLetterVersionId,
        name: String,
    ) -> Result<CoverLetterVersionId> {
        let source = self.get_version(source_version).await?;

        let new_version = CoverLetterVersion {
            id: CoverLetterVersionId::new(),
            name,
            application_id: None,  // Must be set when linked
            content: source.content.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: VersionSource::UserEdited,
            status: VersionStatus::Draft,
            sent_via: None,
            sent_at: None,
        };

        self.db.insert_version(&new_version).await?;
        Ok(new_version.id)
    }

    /// Duplicate for new application (same base, new targeting)
    pub async fn duplicate_for_application(
        &self,
        source_version: CoverLetterVersionId,
        application_id: ApplicationId,
    ) -> Result<CoverLetterVersionId> {
        let mut new_version = self.create_from(source_version, format!("Application {}", application_id)).await?;
        new_version.application_id = Some(application_id);
        self.db.update_version(&new_version).await?;
        Ok(new_version.id)
    }
}
```

### Submission Channel Tracking

```rust
impl CoverLetterService {
    pub async fn record_submission(
        &self,
        version_id: CoverLetterVersionId,
        channel: &str,
        metadata: SubmissionMetadata,
    ) -> Result<()> {
        let sent_record = SentCoverLetter {
            version_id,
            channel: channel.to_string(),
            recipient: metadata.recipient.clone(),
            subject: metadata.subject.clone(),
            sent_at: Utc::now(),
            confirmation_id: metadata.confirmation_id.clone(),
        };

        self.db.insert_sent_record(&sent_record).await?;

        // Update version status
        self.db.update_version_status(version_id, VersionStatus::Sent).await?;

        Ok(())
    }
}

pub struct SubmissionMetadata {
    pub recipient: String,
    pub subject: String,
    pub confirmation_id: Option<String>,
    pub error_message: Option<String>,
}
```

### Confirmation Tracking

After sending, track confirmation (when available):

```rust
pub struct SentCoverLetter {
    pub id: SentCoverLetterId,
    pub version_id: CoverLetterVersionId,
    pub channel: String,
    pub recipient: String,
    pub subject: String,
    pub sent_at: DateTime<Utc>,
    pub confirmation_id: Option<String>,
    pub tracking_status: TrackingStatus,
}

pub enum TrackingStatus {
    Pending,           // Not yet confirmed
    ConfirmedSent,     // Confirmed sent
    Delivered,         // Read receipt (if available)
    Bounced,           // Failed
    OpenedRead,        // Opened (email)
}
```

### Version Cleanup

```rust
pub enum CleanupPolicy {
    KeepAll,
    KeepLastN { count: u32 },
    KeepSentVersions,
    AutoPruneAfterHiring,
}

impl CoverLetterService {
    pub async fn apply_cleanup(&self, policy: &CleanupPolicy) -> Result<u32> {
        match policy {
            CleanupPolicy::KeepAll => Ok(0),
            CleanupPolicy::KeepLastN { count } => self.prune_to_count(count).await,
            CleanupPolicy::KeepSentVersions => self.archive_unsent_older_than_days(30).await,
            CleanupPolicy::AutoPruneAfterHiring => self.prune_accepted_offer_versions().await,
        }
    }
}
```

### Version → Application Linkage

```rust
pub struct ApplicationCoverLetterLink {
    pub application_id: ApplicationId,
    pub cover_letter_version_id: CoverLetterVersionId,
    pub linked_at: DateTime<Utc>,
}

impl CoverLetterService {
    pub async fn link_to_application(
        &self,
        version_id: CoverLetterVersionId,
        application_id: ApplicationId,
    ) -> Result<()> {
        // Verify application exists
        let app = self.db.get_application(application_id).await?;

        // Update version with application
        let mut version = self.get_version(version_id).await?;
        version.application_id = Some(application_id);
        self.db.update_version(&version).await?;

        // Create link record
        let link = ApplicationCoverLetterLink {
            application_id,
            cover_letter_version_id: version_id,
            linked_at: Utc::now(),
        };
        self.db.insert_link(&link).await?;

        Ok(())
    }
}
```

## Implementation Notes

- Versions stored as structured JSON in SQLite
- Sent records for tracking delivery confirmation
- Archive hides from default views, doesn't delete
- Can link one cover letter to multiple applications (e.g., "sent to multiple contacts at same company")

## Open Questions

1. **Template system**: Can a version be promoted to reusable template?
2. **ATS-specific variants**: Auto-generate plain-text version for ATS portals?
3. **Email read receipts**: Integrate with email tracking?

## Related Specs

- `profile-cover-letter-generation.md` - Cover letter generation
- `application-workflow-actions.md` - Application submission
- `XX-resume-version-management.md` - Resume version system (similar pattern)