pub mod docx;
pub mod generator;
pub mod repository;
pub mod types;

pub use docx::CoverLetterDocxGenerator;
pub use generator::CoverLetterGenerator;
pub use repository::CoverLetterRepository;
pub use types::*;

use std::sync::Arc;

use similar::TextDiff;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::discovery::Completer;
use crate::domain::Job;
use crate::error::Result;
use crate::life_sheet::LifeSheet;

const PROHIBITED_PHRASES: &[&str] = &[
    "i am writing to express my interest",
    "passionate about",
    "synergy",
    "leverage my",
    "team player",
    "detail-oriented",
    "proven track record",
    "think outside the box",
    "go-getter",
    "self-starter",
    "dynamic individual",
    "results-driven",
    "hit the ground running",
    "wear many hats",
    "move the needle",
    "circle back",
    "low-hanging fruit",
    "paradigm shift",
    "deep dive",
    "value-add",
];

pub struct CoverLetterService {
    generator: CoverLetterGenerator,
    repo: CoverLetterRepository,
}

impl CoverLetterService {
    pub fn new(completer: Arc<dyn Completer>, pool: sqlx::PgPool) -> Self {
        Self {
            generator: CoverLetterGenerator::new(completer),
            repo: CoverLetterRepository::new(pool),
        }
    }

    pub fn repo(&self) -> &CoverLetterRepository {
        &self.repo
    }

    pub async fn generate(
        &self,
        job: &Job,
        life_sheet: &LifeSheet,
        options: CoverLetterOptions,
        progress_tx: Option<mpsc::Sender<ProgressEvent>>,
    ) -> Result<CoverLetterVersion> {
        send_progress(&progress_tx, ProgressEvent::Generating { pct: 10 }).await;

        let content = self
            .generator
            .generate(
                job,
                life_sheet,
                options.template,
                options.tone,
                options.length,
                options.custom_intro.as_deref(),
            )
            .await?;

        send_progress(&progress_tx, ProgressEvent::CheckingFabrication { pct: 60 }).await;

        let plain_text = CoverLetterGenerator::to_plain_text(&content);
        let key_points = CoverLetterGenerator::extract_key_points(&content);

        let found_phrases = detect_prohibited_phrases(&content);
        if !found_phrases.is_empty() {
            tracing::warn!(
                "Cover letter contains {} prohibited phrase(s): {:?}",
                found_phrases.len(),
                found_phrases
            );
        }

        send_progress(&progress_tx, ProgressEvent::Persisting { pct: 80 }).await;

        let previous = self.repo.latest_for_job(job.id.as_uuid()).await?;
        let version_number = previous.as_ref().map(|v| v.version + 1).unwrap_or(1);

        let diff = previous.as_ref().map(|prev| {
            let diff = TextDiff::from_lines(&prev.content, &content);
            diff.unified_diff()
                .context_radius(3)
                .header("previous", "current")
                .to_string()
        });

        let version = CoverLetterVersion {
            id: CoverLetterId::new(),
            job_id: *job.id.as_uuid(),
            application_id: None,
            version: version_number,
            template: options.template,
            content,
            plain_text,
            key_points,
            tone: options.tone,
            length: options.length,
            options: options.clone(),
            diff_from_previous: diff,
            is_submitted: false,
            label: None,
            created_at: chrono::Utc::now(),
        };

        self.repo.save(&version).await?;

        send_progress(
            &progress_tx,
            ProgressEvent::Done {
                version: version_number,
            },
        )
        .await;

        Ok(version)
    }

    pub async fn list_versions(&self, job_id: &Uuid) -> Result<Vec<CoverLetterVersionSummary>> {
        self.repo.list_for_job(job_id).await
    }
}

pub fn detect_prohibited_phrases(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    PROHIBITED_PHRASES
        .iter()
        .filter(|phrase| lower.contains(**phrase))
        .map(|phrase| (*phrase).to_string())
        .collect()
}

async fn send_progress(tx: &Option<mpsc::Sender<ProgressEvent>>, event: ProgressEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Job;
    use crate::life_sheet::{Achievement, Basics, Skill, SkillCategory, WorkExperience};
    use crate::test_db::TestDb;

    struct MockCompleter;

    #[async_trait::async_trait]
    impl Completer for MockCompleter {
        async fn complete(&self, _system: &str, _user: &str) -> crate::error::Result<String> {
            Ok("When I discovered Acme's engineering blog on memory safety, I knew this was the team I wanted to join.\n\n\
                At TechCorp, I reduced API latency by 40% while handling 10M daily requests. \
                This experience directly maps to your need for a backend engineer who can scale services.\n\n\
                I would welcome the opportunity to discuss how my background aligns with your team's goals."
                .into())
        }
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
                achievements: vec![Achievement {
                    description: "Reduced API latency by 40% serving 10M requests/day".into(),
                    metric_type: None,
                    metric_value: None,
                    metric_unit: None,
                }],
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

    fn mock_job() -> Job {
        let mut job = Job::new("Backend Engineer");
        job.company_name = Some("Acme Inc".into());
        job.description = Some("We need a Rust backend engineer with API experience".into());
        job
    }

    // learning test: verifies similar crate's TextDiff produces unified diff output
    #[test]
    fn similar_text_diff_produces_unified_diff() {
        let old = "line one\nline two\nline three\n";
        let new = "line one\nline TWO\nline three\n";
        let diff = TextDiff::from_lines(old, new);
        let unified = diff
            .unified_diff()
            .context_radius(1)
            .header("old", "new")
            .to_string();
        assert!(unified.contains("--- old"));
        assert!(unified.contains("+++ new"));
        assert!(unified.contains("-line two"));
        assert!(unified.contains("+line TWO"));
    }

    #[test]
    fn detect_prohibited_phrases_finds_matches() {
        let text = "I am passionate about this role and have a proven track record of success.";
        let found = detect_prohibited_phrases(text);
        assert!(found.contains(&"passionate about".to_string()));
        assert!(found.contains(&"proven track record".to_string()));
    }

    #[test]
    fn detect_prohibited_phrases_empty_on_clean_text() {
        let text = "My experience at TechCorp reduced API latency by 40%.";
        let found = detect_prohibited_phrases(text);
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn generate_creates_version_1() {
        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = CoverLetterService::new(completer, db.pool().clone());

        let job = mock_job();
        let job_id = *job.id.as_uuid();
        sqlx::query("INSERT INTO jobs (id, title, discovered_at) VALUES ($1, $2, now())")
            .bind(job_id)
            .bind(&job.title)
            .execute(db.pool())
            .await
            .unwrap();

        let sheet = mock_life_sheet();
        let opts = CoverLetterOptions::default();

        let version = svc.generate(&job, &sheet, opts, None).await.unwrap();
        assert_eq!(version.version, 1);
        assert!(version.diff_from_previous.is_none());
        assert!(!version.content.is_empty());
        assert!(!version.key_points.is_empty());
    }

    #[tokio::test]
    async fn generate_second_version_has_diff() {
        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = CoverLetterService::new(completer, db.pool().clone());

        let job = mock_job();
        let job_id = *job.id.as_uuid();
        sqlx::query("INSERT INTO jobs (id, title, discovered_at) VALUES ($1, $2, now())")
            .bind(job_id)
            .bind(&job.title)
            .execute(db.pool())
            .await
            .unwrap();

        let sheet = mock_life_sheet();

        let v1 = svc
            .generate(&job, &sheet, CoverLetterOptions::default(), None)
            .await
            .unwrap();
        assert_eq!(v1.version, 1);

        let v2 = svc
            .generate(&job, &sheet, CoverLetterOptions::default(), None)
            .await
            .unwrap();
        assert_eq!(v2.version, 2);
        assert!(v2.diff_from_previous.is_some());
    }

    #[tokio::test]
    async fn generate_sends_progress_events() {
        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = CoverLetterService::new(completer, db.pool().clone());

        let job = mock_job();
        let job_id = *job.id.as_uuid();
        sqlx::query("INSERT INTO jobs (id, title, discovered_at) VALUES ($1, $2, now())")
            .bind(job_id)
            .bind(&job.title)
            .execute(db.pool())
            .await
            .unwrap();

        let sheet = mock_life_sheet();
        let (tx, mut rx) = mpsc::channel(16);

        svc.generate(&job, &sheet, CoverLetterOptions::default(), Some(tx))
            .await
            .unwrap();

        let mut events = vec![];
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(events.len() >= 4);
        assert!(matches!(events[0], ProgressEvent::Generating { .. }));
        assert!(matches!(
            events[1],
            ProgressEvent::CheckingFabrication { .. }
        ));
        assert!(matches!(events[2], ProgressEvent::Persisting { .. }));
        assert!(matches!(events[3], ProgressEvent::Done { .. }));
    }

    #[tokio::test]
    async fn list_versions_returns_saved() {
        let db = TestDb::spawn().await;
        let completer: Arc<dyn Completer> = Arc::new(MockCompleter);
        let svc = CoverLetterService::new(completer, db.pool().clone());

        let job = mock_job();
        let job_id = *job.id.as_uuid();
        sqlx::query("INSERT INTO jobs (id, title, discovered_at) VALUES ($1, $2, now())")
            .bind(job_id)
            .bind(&job.title)
            .execute(db.pool())
            .await
            .unwrap();

        let sheet = mock_life_sheet();
        svc.generate(&job, &sheet, CoverLetterOptions::default(), None)
            .await
            .unwrap();

        let versions = svc.list_versions(&job_id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
    }
}
