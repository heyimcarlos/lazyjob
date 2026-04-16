use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{CompanyId, JobId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub title: String,
    pub company_id: Option<CompanyId>,
    pub company_name: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub match_score: Option<f64>,
    pub ghost_score: Option<f64>,
    pub discovered_at: DateTime<Utc>,
    pub notes: Option<String>,
}

impl Job {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: JobId::new(),
            title: title.into(),
            company_id: None,
            company_name: None,
            location: None,
            url: None,
            description: None,
            salary_min: None,
            salary_max: None,
            source: None,
            source_id: None,
            match_score: None,
            ghost_score: None,
            discovered_at: Utc::now(),
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_construction() {
        let job = Job::new("Senior Rust Engineer");
        assert_eq!(job.title, "Senior Rust Engineer");
        assert!(job.company_id.is_none());
    }

    #[test]
    fn job_serde_round_trip() {
        let mut job = Job::new("Backend Developer");
        job.company_name = Some("Acme Corp".into());
        job.salary_min = Some(120_000);
        job.salary_max = Some(180_000);
        job.location = Some("Remote".into());

        let json = serde_json::to_string(&job).unwrap();
        let deserialized: Job = serde_json::from_str(&json).unwrap();
        assert_eq!(job, deserialized);
    }
}
