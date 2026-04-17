use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use crate::{
    domain::{Job, JobId},
    error::Result,
    life_sheet::LifeSheet,
};

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub struct GhostScore {
    pub score: u8,
}

impl GhostScore {
    pub fn is_likely_ghost(&self) -> bool {
        self.score >= 5
    }
}

static GENERIC_TITLES: &[&str] = &[
    "software engineer",
    "software developer",
    "senior software engineer",
    "senior software developer",
    "junior software engineer",
    "junior software developer",
    "full stack developer",
    "full stack engineer",
    "backend developer",
    "backend engineer",
    "frontend developer",
    "frontend engineer",
    "web developer",
    "web engineer",
    "programmer",
    "data scientist",
    "machine learning engineer",
    "product manager",
    "ux designer",
    "ui designer",
    "analyst",
    "engineer",
    "developer",
];

#[derive(Default)]
pub struct GhostDetector {
    pub duplicate_description: bool,
    pub high_application_count: bool,
}

impl GhostDetector {
    pub fn with_duplicate_description(mut self) -> Self {
        self.duplicate_description = true;
        self
    }

    pub fn with_high_application_count(mut self) -> Self {
        self.high_application_count = true;
        self
    }

    pub fn score(&self, job: &Job) -> GhostScore {
        let mut score: u8 = 0;

        let age_days = (Utc::now() - job.discovered_at).num_days();
        if age_days > 60 {
            score = score.saturating_add(3);
        }

        let title_lower = job.title.to_lowercase();
        if GENERIC_TITLES.iter().any(|&t| t == title_lower) {
            score = score.saturating_add(2);
        }

        if job.company_name.is_none() {
            score = score.saturating_add(2);
        }

        if job.salary_min.is_none() && job.salary_max.is_none() {
            score = score.saturating_add(1);
        }

        if job.url.is_none() {
            score = score.saturating_add(1);
        }

        if self.duplicate_description {
            score = score.saturating_add(1);
        }

        if self.high_application_count {
            score = score.saturating_add(1);
        }

        GhostScore { score }
    }
}

pub fn life_sheet_to_text(sheet: &LifeSheet) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ref summary) = sheet.basics.summary {
        parts.push(summary.clone());
    }

    for exp in &sheet.work_experience {
        parts.push(exp.position.clone());
        parts.push(exp.company.clone());
        if !exp.tech_stack.is_empty() {
            parts.push(exp.tech_stack.join(", "));
        }
        for achievement in &exp.achievements {
            parts.push(achievement.description.clone());
        }
    }

    for category in &sheet.skills {
        for skill in &category.skills {
            parts.push(skill.name.clone());
        }
    }

    for cert in &sheet.certifications {
        parts.push(cert.name.clone());
    }

    for project in &sheet.projects {
        parts.push(project.name.clone());
        if let Some(ref desc) = project.description {
            parts.push(desc.clone());
        }
        if !project.highlights.is_empty() {
            parts.push(project.highlights.join(". "));
        }
    }

    parts.join(" ")
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

pub struct MatchScorer {
    embedder: Arc<dyn Embedder>,
}

impl MatchScorer {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self { embedder }
    }

    pub async fn embed_life_sheet(&self, sheet: &LifeSheet) -> Result<Vec<f32>> {
        let text = life_sheet_to_text(sheet);
        self.embedder.embed(&text).await
    }

    pub async fn score_job(&self, job: &Job, profile_embedding: &[f32]) -> Result<f64> {
        let description = job.description.as_deref().unwrap_or(&job.title);
        let job_embedding = self.embedder.embed(description).await?;
        let similarity = cosine_similarity(profile_embedding, &job_embedding);
        Ok(f64::from(similarity).clamp(0.0, 1.0))
    }

    pub async fn score_all(&self, jobs: &mut [Job], sheet: &LifeSheet) -> Result<()> {
        let profile_embedding = self.embed_life_sheet(sheet).await?;
        for job in jobs.iter_mut() {
            let score = self.score_job(job, &profile_embedding).await?;
            job.match_score = Some(score);
        }
        Ok(())
    }

    pub async fn store_embedding(
        &self,
        pool: &PgPool,
        job_id: &JobId,
        embedding: &[f32],
    ) -> Result<()> {
        let bytes = embedding_to_bytes(embedding);
        sqlx::query(
            "INSERT INTO job_embeddings (job_id, embedding)
             VALUES ($1, $2)
             ON CONFLICT (job_id) DO UPDATE
             SET embedding = EXCLUDED.embedding, embedded_at = now()",
        )
        .bind(job_id.as_uuid())
        .bind(&bytes)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn load_embedding(&self, pool: &PgPool, job_id: &JobId) -> Result<Option<Vec<f32>>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT embedding FROM job_embeddings WHERE job_id = $1")
                .bind(job_id.as_uuid())
                .fetch_optional(pool)
                .await?;
        Ok(row.map(|(bytes,)| bytes_to_embedding(&bytes)))
    }
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::Job;
    use crate::life_sheet::{Basics, LifeSheet, Skill, SkillCategory};

    struct MockEmbedder {
        response: Vec<f32>,
    }

    #[async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, _text: &str) -> crate::error::Result<Vec<f32>> {
            Ok(self.response.clone())
        }
    }

    fn make_mock_scorer(response: Vec<f32>) -> MatchScorer {
        MatchScorer::new(Arc::new(MockEmbedder { response }))
    }

    fn clean_job() -> Job {
        let mut job = Job::new("Rust Backend Engineer at Acme Corp");
        job.company_name = Some("Acme Corp".to_string());
        job.salary_min = Some(120_000);
        job.salary_max = Some(180_000);
        job.url = Some("https://acme.com/careers/rust-engineer".to_string());
        job
    }

    fn old_generic_job() -> Job {
        let mut job = Job::new("Software Engineer");
        job.discovered_at = Utc::now() - Duration::days(65);
        job
    }

    // learning test: verifies library behavior
    #[test]
    fn cosine_similarity_orthogonal_vectors_is_zero() {
        let a = [1.0f32, 0.0, 0.0];
        let b = [0.0f32, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 0.0).abs() < 1e-6,
            "orthogonal vectors should have similarity 0"
        );
    }

    // learning test: verifies library behavior
    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let a = [0.3f32, 0.4, 0.5];
        let sim = cosine_similarity(&a, &a);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "identical vectors should have similarity 1"
        );
    }

    // learning test: verifies library behavior
    #[test]
    fn cosine_similarity_known_pair() {
        let a = [1.0f32, 0.0];
        let b = [0.0f32, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);

        let a2 = [3.0f32, 4.0];
        let b2 = [3.0f32, 4.0];
        assert!((cosine_similarity(&a2, &b2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector_returns_zero() {
        let a = [0.0f32, 0.0, 0.0];
        let b = [1.0f32, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
        assert_eq!(cosine_similarity(&a, &a), 0.0);
    }

    #[test]
    fn cosine_similarity_different_lengths_returns_zero() {
        let a = [1.0f32, 2.0];
        let b = [1.0f32, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn ghost_detector_old_job_adds_three() {
        let job = old_generic_job();
        let detector = GhostDetector::default();
        assert!(detector.score(&job).score >= 3);
    }

    #[test]
    fn ghost_detector_generic_title_adds_two() {
        let mut job = clean_job();
        job.title = "Software Engineer".to_string();
        let detector = GhostDetector::default();
        assert!(detector.score(&job).score >= 2);
    }

    #[test]
    fn ghost_detector_no_company_name_adds_two() {
        let mut job = clean_job();
        job.company_name = None;
        let detector = GhostDetector::default();
        assert!(detector.score(&job).score >= 2);
    }

    #[test]
    fn ghost_detector_missing_salary_adds_one() {
        let mut job = clean_job();
        job.salary_min = None;
        job.salary_max = None;
        let detector = GhostDetector::default();
        let base = GhostDetector::default().score(&clean_job()).score;
        assert!(detector.score(&job).score >= base + 1);
    }

    #[test]
    fn ghost_detector_no_url_adds_one() {
        let mut job = clean_job();
        job.url = None;
        let detector = GhostDetector::default();
        let base = GhostDetector::default().score(&clean_job()).score;
        assert!(detector.score(&job).score >= base + 1);
    }

    #[test]
    fn ghost_detector_duplicate_description_adds_one() {
        let job = clean_job();
        let base = GhostDetector::default().score(&job).score;
        let detector = GhostDetector::default().with_duplicate_description();
        assert_eq!(detector.score(&job).score, base + 1);
    }

    #[test]
    fn ghost_detector_combined_likely_ghost() {
        let job = old_generic_job();
        let detector = GhostDetector::default().with_duplicate_description();
        let score = detector.score(&job);
        assert!(
            score.is_likely_ghost(),
            "old job with no company/salary/url should be a ghost (score={})",
            score.score
        );
    }

    #[test]
    fn ghost_detector_clean_job_not_ghost() {
        let job = clean_job();
        let detector = GhostDetector::default();
        assert!(!detector.score(&job).is_likely_ghost());
    }

    #[test]
    fn ghost_is_likely_ghost_threshold() {
        assert!(GhostScore { score: 5 }.is_likely_ghost());
        assert!(!GhostScore { score: 4 }.is_likely_ghost());
    }

    fn make_life_sheet(name: &str, summary: Option<&str>) -> LifeSheet {
        LifeSheet {
            basics: Basics {
                name: name.to_string(),
                label: None,
                email: None,
                phone: None,
                url: None,
                summary: summary.map(|s| s.to_string()),
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
        }
    }

    #[test]
    fn life_sheet_to_text_includes_skill_names() {
        let mut sheet = make_life_sheet("Alice", Some("Rust developer"));
        sheet.skills = vec![SkillCategory {
            name: "Languages".to_string(),
            level: None,
            skills: vec![
                Skill {
                    name: "Rust".to_string(),
                    years_experience: None,
                    proficiency: None,
                },
                Skill {
                    name: "Python".to_string(),
                    years_experience: None,
                    proficiency: None,
                },
            ],
        }];

        let text = life_sheet_to_text(&sheet);
        assert!(text.contains("Rust"));
        assert!(text.contains("Python"));
        assert!(text.contains("Rust developer"));
    }

    #[test]
    fn embedding_round_trip() {
        let original = vec![0.1f32, 0.2, 0.3, -0.5, 1.0];
        let bytes = embedding_to_bytes(&original);
        let recovered = bytes_to_embedding(&bytes);
        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!(
                (a - b).abs() < 1e-7,
                "embedding round-trip mismatch: {a} vs {b}"
            );
        }
    }

    #[tokio::test]
    async fn score_job_returns_value_in_range() {
        let scorer = make_mock_scorer(vec![1.0f32, 0.0, 0.0]);
        let job = clean_job();
        let score = scorer.score_job(&job, &[1.0f32, 0.0, 0.0]).await.unwrap();
        assert!((0.0..=1.0).contains(&score));
    }

    #[tokio::test]
    async fn score_all_sets_match_score_on_jobs() {
        let scorer = make_mock_scorer(vec![1.0f32, 0.0, 0.0]);
        let mut jobs = vec![clean_job(), old_generic_job()];
        let sheet = make_life_sheet("Bob", None);
        scorer.score_all(&mut jobs, &sheet).await.unwrap();
        for job in &jobs {
            assert!(job.match_score.is_some());
            let s = job.match_score.unwrap();
            assert!((0.0..=1.0).contains(&s));
        }
    }

    #[tokio::test]
    async fn store_and_load_embedding() {
        let db = crate::test_db::TestDb::spawn().await;

        let scorer = make_mock_scorer(vec![0.1, 0.2, 0.3]);
        let job = clean_job();

        let repo = crate::repositories::JobRepository::new(db.pool().clone());
        repo.insert(&job).await.unwrap();

        let embedding = vec![0.1f32, 0.2, 0.3, 0.4, 0.5];
        scorer
            .store_embedding(db.pool(), &job.id, &embedding)
            .await
            .unwrap();
        let loaded = scorer
            .load_embedding(db.pool(), &job.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(embedding.len(), loaded.len());
        for (a, b) in embedding.iter().zip(loaded.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }
}
