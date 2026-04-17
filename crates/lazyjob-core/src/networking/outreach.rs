use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::discovery::Completer;
use crate::domain::{Contact, ContactId, Job, JobId};
use crate::error::{CoreError, Result};
use crate::life_sheet::LifeSheet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutreachTone {
    Warm,
    Professional,
    Casual,
    ReferralAsk,
}

impl OutreachTone {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutreachTone::Warm => "warm",
            OutreachTone::Professional => "professional",
            OutreachTone::Casual => "casual",
            OutreachTone::ReferralAsk => "referral_ask",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "warm" => OutreachTone::Warm,
            "casual" => OutreachTone::Casual,
            "referral_ask" | "referral-ask" | "referral" => OutreachTone::ReferralAsk,
            _ => OutreachTone::Professional,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            OutreachTone::Warm => "Warm & Friendly",
            OutreachTone::Professional => "Professional",
            OutreachTone::Casual => "Casual Reconnect",
            OutreachTone::ReferralAsk => "Referral Request",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutreachDraft {
    pub id: Uuid,
    pub contact_id: ContactId,
    pub job_id: Option<JobId>,
    pub tone: OutreachTone,
    pub subject: Option<String>,
    pub body: String,
    pub fabrication_warnings: Vec<String>,
    pub char_count: i32,
    pub word_count: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OutreachRecord {
    pub id: Uuid,
    pub contact_id: Uuid,
    pub job_id: Option<Uuid>,
    pub tone: String,
    pub subject: Option<String>,
    pub body: String,
    pub fabrication_warnings: serde_json::Value,
    pub char_count: i32,
    pub word_count: i32,
    pub created_at: DateTime<Utc>,
}

pub struct OutreachRepository {
    pool: PgPool,
}

impl OutreachRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn save_draft(&self, draft: &OutreachDraft) -> Result<()> {
        let warnings_json = serde_json::to_value(&draft.fabrication_warnings)?;
        let job_id_uuid: Option<Uuid> = draft.job_id.map(|j| *j.as_uuid());
        sqlx::query(
            "INSERT INTO outreach_drafts (id, contact_id, job_id, tone, subject, body, fabrication_warnings, char_count, word_count, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(draft.id)
        .bind(draft.contact_id)
        .bind(job_id_uuid)
        .bind(draft.tone.as_str())
        .bind(&draft.subject)
        .bind(&draft.body)
        .bind(&warnings_json)
        .bind(draft.char_count)
        .bind(draft.word_count)
        .bind(draft.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_latest_for_contact(
        &self,
        contact_id: &ContactId,
    ) -> Result<Option<OutreachRecord>> {
        let row = sqlx::query_as::<_, OutreachRow>(
            "SELECT id, contact_id, job_id, tone, subject, body, fabrication_warnings, char_count, word_count, created_at
             FROM outreach_drafts WHERE contact_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(contact_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn list_for_contact(&self, contact_id: &ContactId) -> Result<Vec<OutreachRecord>> {
        let rows = sqlx::query_as::<_, OutreachRow>(
            "SELECT id, contact_id, job_id, tone, subject, body, fabrication_warnings, char_count, word_count, created_at
             FROM outreach_drafts WHERE contact_id = $1 ORDER BY created_at DESC",
        )
        .bind(contact_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM outreach_drafts WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct OutreachRow {
    id: Uuid,
    contact_id: Uuid,
    job_id: Option<Uuid>,
    tone: String,
    subject: Option<String>,
    body: String,
    fabrication_warnings: serde_json::Value,
    char_count: i32,
    word_count: i32,
    created_at: DateTime<Utc>,
}

impl From<OutreachRow> for OutreachRecord {
    fn from(row: OutreachRow) -> Self {
        Self {
            id: row.id,
            contact_id: row.contact_id,
            job_id: row.job_id,
            tone: row.tone,
            subject: row.subject,
            body: row.body,
            fabrication_warnings: row.fabrication_warnings,
            char_count: row.char_count,
            word_count: row.word_count,
            created_at: row.created_at,
        }
    }
}

pub struct OutreachDraftingService {
    completer: Arc<dyn Completer>,
    pool: PgPool,
}

impl OutreachDraftingService {
    pub fn new(completer: Arc<dyn Completer>, pool: PgPool) -> Self {
        Self { completer, pool }
    }

    pub fn repo(&self) -> OutreachRepository {
        OutreachRepository::new(self.pool.clone())
    }

    pub async fn draft(
        &self,
        contact: &Contact,
        job: Option<&Job>,
        tone: OutreachTone,
        life_sheet: &LifeSheet,
    ) -> Result<OutreachDraft> {
        let contact_role = contact
            .role
            .clone()
            .unwrap_or_else(|| "professional".into());
        let contact_company = contact
            .current_company
            .clone()
            .unwrap_or_else(|| "their company".into());

        let (job_title, company_name, job_id) = match job {
            Some(j) => (
                j.title.clone(),
                j.company_name
                    .clone()
                    .unwrap_or_else(|| contact_company.clone()),
                Some(j.id),
            ),
            None => ("the role".into(), contact_company.clone(), None),
        };

        let user_background = build_user_background(life_sheet);

        let system = "You are a professional networking assistant. Generate a single, personalized outreach message.\n\n\
            RULES:\n\
            1. Every factual claim must be grounded in the user's actual background.\n\
            2. Never invent shared history, mutual connections, or experiences.\n\
            3. Never reference salary, personal details, or internal company information.\n\
            4. For casual tone: do NOT include a job ask. Focus on relationship re-warming.\n\
            5. For referral-ask tone: make the referral ask specific to the role.\n\
            6. Keep the message concise (3-5 sentences).\n\
            7. Do NOT use cliché phrases like 'passionate about', 'synergy', 'leverage my'.\n\n\
            OUTPUT: Return only the message body text. No meta-commentary.";

        let user_prompt = format!(
            "Draft a {} outreach message to {} ({} at {}).\n\n\
             Target role: {} at {}\n\n\
             My background:\n{}\n\n\
             Write a concise, personalized message.",
            tone.label(),
            contact.name,
            contact_role,
            contact_company,
            job_title,
            company_name,
            user_background,
        );

        let raw_body = self.completer.complete(system, &user_prompt).await?;
        let body = raw_body.trim().to_string();

        if body.is_empty() {
            return Err(CoreError::Validation(
                "LLM returned empty outreach draft".into(),
            ));
        }

        let char_count = body.chars().count() as i32;
        let word_count = body.split_whitespace().count() as i32;

        let fabrication_warnings = check_fabrication(&body, life_sheet);

        let draft = OutreachDraft {
            id: Uuid::new_v4(),
            contact_id: contact.id,
            job_id,
            tone,
            subject: None,
            body,
            fabrication_warnings,
            char_count,
            word_count,
            created_at: Utc::now(),
        };

        let repo = self.repo();
        repo.save_draft(&draft).await?;

        Ok(draft)
    }
}

fn build_user_background(life_sheet: &LifeSheet) -> String {
    let mut parts = Vec::new();

    for exp in life_sheet.work_experience.iter().take(3) {
        let current = if exp.is_current { " (current)" } else { "" };
        parts.push(format!("{} at {}{}", exp.position, exp.company, current));
    }

    if !life_sheet.skills.is_empty() {
        let skills: Vec<&str> = life_sheet
            .skills
            .iter()
            .flat_map(|cat| cat.skills.iter().map(|s| s.name.as_str()))
            .take(10)
            .collect();
        if !skills.is_empty() {
            parts.push(format!("Skills: {}", skills.join(", ")));
        }
    }

    if parts.is_empty() {
        "Professional background".into()
    } else {
        parts.join("\n")
    }
}

fn check_fabrication(body: &str, life_sheet: &LifeSheet) -> Vec<String> {
    let mut warnings = Vec::new();

    let prohibited = &[
        "passionate about",
        "synergy",
        "leverage my",
        "team player",
        "proven track record",
        "think outside the box",
        "go-getter",
        "self-starter",
        "dynamic individual",
        "results-driven",
        "hit the ground running",
    ];

    let lower = body.to_lowercase();
    for phrase in prohibited {
        if lower.contains(phrase) {
            warnings.push(format!("Contains cliché phrase: '{phrase}'"));
        }
    }

    let companies: Vec<&str> = life_sheet
        .work_experience
        .iter()
        .map(|e| e.company.as_str())
        .collect();

    let company_claim_patterns = [
        "worked at",
        "worked for",
        "time at",
        "experience at",
        "years at",
    ];

    for sentence in body.split('.') {
        let sentence_lower = sentence.to_lowercase();
        for pattern in &company_claim_patterns {
            if sentence_lower.contains(pattern) {
                let has_known_company = companies
                    .iter()
                    .any(|c| sentence_lower.contains(&c.to_lowercase()));
                if !has_known_company && !sentence_lower.contains("your") {
                    warnings.push(format!(
                        "Possible ungrounded company claim: '{}'",
                        sentence.trim()
                    ));
                }
            }
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Job;
    use crate::life_sheet::{Basics, Skill, SkillCategory, WorkExperience};
    use crate::repositories::{ContactRepository, JobRepository};

    #[test]
    fn outreach_tone_as_str() {
        assert_eq!(OutreachTone::Warm.as_str(), "warm");
        assert_eq!(OutreachTone::Professional.as_str(), "professional");
        assert_eq!(OutreachTone::Casual.as_str(), "casual");
        assert_eq!(OutreachTone::ReferralAsk.as_str(), "referral_ask");
    }

    #[test]
    fn outreach_tone_from_str_loose() {
        assert_eq!(OutreachTone::from_str_loose("warm"), OutreachTone::Warm);
        assert_eq!(
            OutreachTone::from_str_loose("referral-ask"),
            OutreachTone::ReferralAsk
        );
        assert_eq!(
            OutreachTone::from_str_loose("referral"),
            OutreachTone::ReferralAsk
        );
        assert_eq!(
            OutreachTone::from_str_loose("unknown"),
            OutreachTone::Professional
        );
    }

    #[test]
    fn outreach_tone_serde_round_trip() {
        let tone = OutreachTone::Warm;
        let json = serde_json::to_string(&tone).unwrap();
        let deserialized: OutreachTone = serde_json::from_str(&json).unwrap();
        assert_eq!(tone, deserialized);
    }

    #[test]
    fn outreach_tone_label() {
        assert_eq!(OutreachTone::Warm.label(), "Warm & Friendly");
        assert_eq!(OutreachTone::ReferralAsk.label(), "Referral Request");
    }

    fn mock_life_sheet() -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: "Alice Smith".into(),
                label: None,
                email: Some("alice@example.com".into()),
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![WorkExperience {
                company: "TechCorp".into(),
                position: "Senior Engineer".into(),
                location: None,
                url: None,
                start_date: "2020".into(),
                end_date: None,
                is_current: true,
                summary: None,
                achievements: vec![],
                team_size: None,
                industry: None,
                tech_stack: vec!["Rust".into()],
            }],
            education: vec![],
            skills: vec![SkillCategory {
                name: "Languages".into(),
                level: None,
                skills: vec![Skill {
                    name: "Rust".into(),
                    years_experience: Some(4),
                    proficiency: None,
                }],
            }],
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        }
    }

    #[test]
    fn build_user_background_includes_experience() {
        let sheet = mock_life_sheet();
        let bg = build_user_background(&sheet);
        assert!(bg.contains("Senior Engineer at TechCorp"));
        assert!(bg.contains("(current)"));
        assert!(bg.contains("Rust"));
    }

    #[test]
    fn build_user_background_empty_sheet() {
        let sheet = LifeSheet {
            basics: Basics {
                name: "Test".into(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: None,
                location: None,
            },
            work_experience: vec![],
            education: vec![],
            skills: vec![],
            certifications: vec![],
            languages: vec![],
            projects: vec![],
            preferences: None,
            goals: None,
        };
        let bg = build_user_background(&sheet);
        assert_eq!(bg, "Professional background");
    }

    #[test]
    fn check_fabrication_detects_cliches() {
        let sheet = mock_life_sheet();
        let body = "I am passionate about this opportunity and am a self-starter.";
        let warnings = check_fabrication(body, &sheet);
        assert!(warnings.iter().any(|w| w.contains("passionate about")));
        assert!(warnings.iter().any(|w| w.contains("self-starter")));
    }

    #[test]
    fn check_fabrication_clean_text_no_warnings() {
        let sheet = mock_life_sheet();
        let body = "Hi Alice, I noticed your work at Acme Corp. As a Senior Engineer at TechCorp, I would love to connect.";
        let warnings = check_fabrication(body, &sheet);
        assert!(
            warnings.is_empty(),
            "Expected no warnings, got: {warnings:?}"
        );
    }

    #[test]
    fn check_fabrication_flags_unknown_company_claim() {
        let sheet = mock_life_sheet();
        let body = "During my time at FakeCompany I learned a lot.";
        let warnings = check_fabrication(body, &sheet);
        assert!(
            warnings.iter().any(|w| w.contains("ungrounded")),
            "Expected ungrounded warning, got: {warnings:?}"
        );
    }

    #[test]
    fn outreach_draft_construction() {
        let draft = OutreachDraft {
            id: Uuid::new_v4(),
            contact_id: ContactId::new(),
            job_id: None,
            tone: OutreachTone::Professional,
            subject: None,
            body: "Hello, I would like to connect.".into(),
            fabrication_warnings: vec![],
            char_count: 31,
            word_count: 6,
            created_at: Utc::now(),
        };
        assert_eq!(draft.tone, OutreachTone::Professional);
        assert!(draft.fabrication_warnings.is_empty());
    }

    #[tokio::test]
    async fn draft_with_mock_completer() {
        use crate::test_db::TestDb;

        struct MockCompleter;

        #[async_trait::async_trait]
        impl Completer for MockCompleter {
            async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
                Ok("Hi Alice, I came across your profile and was impressed by your work at Acme Corp. \
                    As a Senior Engineer at TechCorp specializing in Rust, I believe we share similar interests \
                    in systems engineering. Would you be open to a brief chat about your experience?".into())
            }
        }

        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = OutreachDraftingService::new(completer, db.pool().clone());

        let mut contact = Contact::new("Alice Smith");
        contact.role = Some("Staff Engineer".into());
        contact.current_company = Some("Acme Corp".into());
        contact.email = Some("alice@acme.com".into());

        let contact_repo = ContactRepository::new(db.pool().clone());
        contact_repo.insert(&contact).await.unwrap();

        let sheet = mock_life_sheet();

        let draft = svc
            .draft(&contact, None, OutreachTone::Professional, &sheet)
            .await
            .unwrap();

        assert!(!draft.body.is_empty());
        assert!(draft.char_count > 0);
        assert!(draft.word_count > 0);
        assert!(draft.fabrication_warnings.is_empty());
    }

    #[tokio::test]
    async fn draft_with_job_context() {
        use crate::test_db::TestDb;

        struct MockCompleter;

        #[async_trait::async_trait]
        impl Completer for MockCompleter {
            async fn complete(&self, _system: &str, user: &str) -> Result<String> {
                assert!(user.contains("Backend Engineer"));
                assert!(user.contains("Acme Corp"));
                Ok(
                    "Hi Alice, I am interested in the Backend Engineer role at Acme Corp. \
                    My experience at TechCorp in Rust systems would be a great fit."
                        .into(),
                )
            }
        }

        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = OutreachDraftingService::new(completer, db.pool().clone());

        let mut contact = Contact::new("Alice Smith");
        contact.role = Some("Recruiter".into());
        contact.current_company = Some("Acme Corp".into());
        contact.email = Some("alice@acme.com".into());

        let contact_repo = ContactRepository::new(db.pool().clone());
        contact_repo.insert(&contact).await.unwrap();

        let mut job = Job::new("Backend Engineer");
        job.company_name = Some("Acme Corp".into());
        let job_repo = JobRepository::new(db.pool().clone());
        job_repo.insert(&job).await.unwrap();

        let sheet = mock_life_sheet();
        let draft = svc
            .draft(&contact, Some(&job), OutreachTone::ReferralAsk, &sheet)
            .await
            .unwrap();

        assert!(!draft.body.is_empty());
        assert!(draft.job_id.is_some());
    }

    #[tokio::test]
    async fn save_and_retrieve_outreach_draft() {
        use crate::test_db::TestDb;

        let db = TestDb::spawn().await;
        let repo = OutreachRepository::new(db.pool().clone());

        let mut contact = Contact::new("Bob Jones");
        contact.email = Some("bob@example.com".into());
        let contact_repo = ContactRepository::new(db.pool().clone());
        contact_repo.insert(&contact).await.unwrap();

        let draft = OutreachDraft {
            id: Uuid::new_v4(),
            contact_id: contact.id,
            job_id: None,
            tone: OutreachTone::Warm,
            subject: None,
            body: "Hey Bob, would love to reconnect!".into(),
            fabrication_warnings: vec![],
            char_count: 34,
            word_count: 6,
            created_at: Utc::now(),
        };

        repo.save_draft(&draft).await.unwrap();

        let retrieved = repo.get_latest_for_contact(&contact.id).await.unwrap();
        assert!(retrieved.is_some());
        let record = retrieved.unwrap();
        assert_eq!(record.body, "Hey Bob, would love to reconnect!");
        assert_eq!(record.tone, "warm");
    }

    #[tokio::test]
    async fn list_drafts_for_contact() {
        use crate::test_db::TestDb;

        let db = TestDb::spawn().await;
        let repo = OutreachRepository::new(db.pool().clone());

        let mut contact = Contact::new("Charlie");
        contact.email = Some("charlie@example.com".into());
        let contact_repo = ContactRepository::new(db.pool().clone());
        contact_repo.insert(&contact).await.unwrap();

        for i in 0..3 {
            let draft = OutreachDraft {
                id: Uuid::new_v4(),
                contact_id: contact.id,
                job_id: None,
                tone: OutreachTone::Professional,
                subject: None,
                body: format!("Draft number {i}"),
                fabrication_warnings: vec![],
                char_count: 15,
                word_count: 3,
                created_at: Utc::now(),
            };
            repo.save_draft(&draft).await.unwrap();
        }

        let drafts = repo.list_for_contact(&contact.id).await.unwrap();
        assert_eq!(drafts.len(), 3);
    }
}
