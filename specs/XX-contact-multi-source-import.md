# Spec: Multi-Source Contact Import

## Context

Users have contacts scattered across email (Gmail, Apple Contacts), phone (mobile contacts), LinkedIn, business cards, and vCard files. Currently only LinkedIn CSV import is supported. This spec addresses comprehensive contact import.

## Motivation

- **Network completeness**: User may know 200 people from email, only 50 on LinkedIn
- **Data quality**: Richer contact data improves networking suggestions
- **User friction**: Manual entry is painful, users won't do it

## Design

### Contact Source Types

```rust
pub enum ContactSource {
    LinkedInCsv { imported_at: DateTime<Utc> },
    GmailContacts { email: String },
    AppleContacts,
    VCardFile { path: PathBuf },
    BusinessCardPhoto { photo_path: PathBuf },
    ManualEntry,
}

pub struct ImportedContact {
    pub id: ContactId,
    pub source: ContactSource,
    pub names: Vec<ContactName>,
    pub emails: Vec<Email>,
    pub phone_numbers: Vec<PhoneNumber>,
    pub companies: Vec<Company>,
    pub job_titles: Vec<String>,
    pub notes: Option<String>,
    pub photo_url: Option<String>,
    pub relationship: Option<String>,
}
```

### Import Pipeline

```rust
pub struct ContactImportService {
    parsers: HashMap<ContactSource, Box<dyn ContactParser>>,
    dedup_service: ContactDedupService,
}

pub trait ContactParser {
    async fn parse(&self, source: &ContactSource) -> Result<Vec<ImportedContact>>;
}

pub struct LinkedInCsvParser { /* existing implementation */ }

pub struct VCardParser {
    impl VCardParser {
        pub async fn parse(&self, path: &Path) -> Result<Vec<ImportedContact>> {
            let content = std::fs::read_to_string(path)?;
            let vcards = self.parse_vcard_format(&content)?;

            vcards.into_iter().map(|v| {
                Ok(ImportedContact {
                    id: ContactId::new(),
                    source: ContactSource::VCardFile { path: path.to_path_buf() },
                    names: vec![ContactName {
                        given_name: v.name.given.clone(),
                        family_name: v.name.family.clone(),
                        full_name: v.name.full.clone(),
                    }],
                    emails: v.emails.into_iter().map(|e| Email { address: e }).collect(),
                    phone_numbers: v.phones.into_iter().map(|p| PhoneNumber { number: p }).collect(),
                    // ...
                })
            }).collect()
        }
    }
}
```

### Gmail Contacts Import

```rust
pub struct GmailContactsParser {
    oauth_client: OAuthClient,
}

impl GmailContactsParser {
    pub async fn parse(&self, access_token: &str) -> Result<Vec<ImportedContact>> {
        // Use Google Contacts API
        let url = "https://people.googleapis.com/v1/people/me/connections";
        let response = self.oauth_client.get(url, access_token).await?;

        let connections: ConnectionsResponse = serde_json::from_str(&response)?;

        connections.connections.into_iter().map(|person| {
            Ok(ImportedContact {
                id: ContactId::new(),
                source: ContactSource::GmailContacts { email: "user@gmail.com".to_string() },
                names: person.names.into_iter().map(|n| ContactName { ... }).collect(),
                emails: person.emails.into_iter().map(|e| Email { address: e }).collect(),
                // ...
            })
        }).collect()
    }
}
```

### Apple Contacts (macOS)

```rust
#[cfg(target_os = "macos")]
pub struct AppleContactsParser {
    // Use Objective-C Contact framework via FFI
}

#[cfg(target_os = "macos")]
impl AppleContactsParser {
    pub async fn parse(&self) -> Result<Vec<ImportedContact>> {
        // Query Address Book framework for all contacts
        // Iterate and convert to ImportedContact format
    }
}
```

### Business Card Scanning

```rust
pub struct BusinessCardScanner {
    ocr_service: OcrService,  // Use device OCR or cloud OCR
}

impl BusinessCardScanner {
    pub async fn scan_photo(&self, photo_path: &Path) -> Result<ImportedContact> {
        // 1. OCR the image
        let text = self.ocr_service.extract_text(photo_path).await?;

        // 2. Parse using LLM
        let contact = self.llm.parse_business_card(&text).await?;

        Ok(ImportedContact {
            id: ContactId::new(),
            source: ContactSource::BusinessCardPhoto { photo_path: photo_path.to_path_buf() },
            names: vec![ContactName { full_name: contact.name, .. }],
            companies: vec![Company { name: contact.company, .. }],
            job_titles: vec![contact.title],
            emails: contact.email.map(|e| Email { address: e }).into_iter().collect(),
            phone_numbers: contact.phone.map(|p| PhoneNumber { number: p }).into_iter().collect(),
            // ...
        })
    }
}
```

### Incremental Import

```rust
pub struct IncrementalImport {
    last_import: HashMap<ContactSource, DateTime<Utc>>,
}

impl IncrementalImport {
    pub async fn import_new_contacts(&self, source: &ContactSource) -> Result<Vec<ImportedContact>> {
        let last = self.last_import.get(source).unwrap_or(&DateTime::MIN);

        let all_contacts = self.parser.parse(source).await?;

        // Filter to only contacts modified since last import
        let new = all_contacts
            .into_iter()
            .filter(|c| c.modified_at > *last)
            .collect();

        // Update last import timestamp
        self.last_import.insert(source.clone(), Utc::now());

        Ok(new)
    }
}
```

### Contact Deduplication on Import

```rust
pub struct ImportDedupService {
    similarity_threshold: f32 = 0.85,
}

impl ImportDedupService {
    pub async fn merge_duplicates(
        &self,
        existing: &[Contact],
        imported: &[ImportedContact],
    ) -> Result<Vec<ContactMerge>> {
        let mut merges = vec![];

        for imp in imported {
            if let Some(existing_match) = self.find_match(existing, imp) {
                merges.push(ContactMerge {
                    existing_id: existing_match.id,
                    imported_data: imp.clone(),
                    confidence: self.calculate_similarity(existing_match, imp),
                });
            }
        }

        Ok(merges)
    }

    fn calculate_similarity(&self, existing: &Contact, imported: &ImportedContact) -> f32 {
        let mut score = 0.0;
        let mut weights = 0.0;

        // Name match (high weight)
        if let Some(name_match) = self.name_similarity(&existing.name, &imported.names) {
            score += name_match * 0.4;
            weights += 0.4;
        }

        // Email match (very high weight)
        if let Some(email_match) = self.email_similarity(&existing.emails, &imported.emails) {
            score += email_match * 0.5;
            weights += 0.5;
        }

        // Company match (medium weight)
        if let Some(company_match) = self.company_similarity(&existing.company, &imported.companies) {
            score += company_match * 0.1;
            weights += 0.1;
        }

        if weights > 0.0 {
            score / weights
        } else {
            0.0
        }
    }
}
```

### Import UI

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Import Contacts                                                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐             │
│  │ 📇 LinkedIn CSV │  │ 📧 Gmail       │  │ 📱 Apple        │             │
│  │     Import     │  │   Contacts     │  │   Contacts      │             │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘             │
│                                                                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐             │
│  │ 📇 vCard File   │  │ 📷 Business     │  │ ✏️ Manual       │             │
│  │     Import      │  │     Card Scan   │  │     Entry       │             │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘             │
│                                                                             │
│  Import Summary:                                                           │
│  ┌──────────────────────────────────────────────────────────────┐           │
│  │  LinkedIn CSV: 234 contacts found                           │           │
│  │  Gmail: 456 contacts found                                   │           │
│  │  12 duplicates detected - will be merged                     │           │
│  └──────────────────────────────────────────────────────────────┘           │
│                                                                             │
│  [Select All Sources]  [Review Duplicates]  [Start Import]                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Implementation Notes

- Use platform-native libraries where available (AddressBook on macOS)
- OAuth integration for Gmail
- OCR via device capabilities or cloud API
- Batch imports processed in background to not block UI

## Open Questions

1. **Permission handling**: How to request contacts permission on each platform?
2. **Data minimization**: Only import fields we need?
3. **Sync vs one-time**: Should contacts stay synced or one-time import?

## Related Specs

- `networking-connection-mapping.md` - Connection mapping
- `networking-referral-management.md` - Relationship tracking
- `XX-contact-identity-resolution.md` - Dedup after import