mod application;
mod company;
mod contact;
mod job;

pub use application::ApplicationRepository;
pub use company::CompanyRepository;
pub use contact::ContactRepository;
pub use job::JobRepository;

#[derive(Debug, Clone)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::domain::*;

    #[test]
    fn pagination_default() {
        let p = Pagination::default();
        assert_eq!(p.limit, 50);
        assert_eq!(p.offset, 0);
    }

    async fn setup_db() -> Option<Database> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("Skipping integration test: DATABASE_URL not set");
                return None;
            }
        };
        Some(Database::connect(&url).await.unwrap())
    }

    #[tokio::test]
    async fn job_crud() {
        let Some(db) = setup_db().await else {
            return;
        };
        let repo = JobRepository::new(db.pool().clone());

        let mut job = Job::new("Rust Developer");
        job.company_name = Some("TestCorp".into());
        job.salary_min = Some(100_000);
        job.salary_max = Some(150_000);
        job.location = Some("Remote".into());

        repo.insert(&job).await.unwrap();

        let found = repo.find_by_id(&job.id).await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.title, "Rust Developer");
        assert_eq!(found.company_name.as_deref(), Some("TestCorp"));
        assert_eq!(found.salary_min, Some(100_000));

        let mut updated = found;
        updated.title = "Senior Rust Developer".into();
        repo.update(&updated).await.unwrap();

        let found = repo.find_by_id(&job.id).await.unwrap().unwrap();
        assert_eq!(found.title, "Senior Rust Developer");

        let list = repo.list(&Pagination::default()).await.unwrap();
        assert!(list.iter().any(|j| j.id == job.id));

        repo.delete(&job.id).await.unwrap();
        let found = repo.find_by_id(&job.id).await.unwrap();
        assert!(found.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn application_crud() {
        let Some(db) = setup_db().await else {
            return;
        };
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job = Job::new("App Test Job");
        job_repo.insert(&job).await.unwrap();

        let mut app = Application::new(job.id);
        app.stage = ApplicationStage::Applied;
        app.resume_version = Some("v1".into());
        app_repo.insert(&app).await.unwrap();

        let found = app_repo.find_by_id(&app.id).await.unwrap().unwrap();
        assert_eq!(found.stage, ApplicationStage::Applied);
        assert_eq!(found.resume_version.as_deref(), Some("v1"));

        let mut updated = found;
        updated.stage = ApplicationStage::Technical;
        app_repo.update(&updated).await.unwrap();

        let found = app_repo.find_by_id(&app.id).await.unwrap().unwrap();
        assert_eq!(found.stage, ApplicationStage::Technical);

        let list = app_repo.list(&Pagination::default()).await.unwrap();
        assert!(list.iter().any(|a| a.id == app.id));

        app_repo.delete(&app.id).await.unwrap();
        job_repo.delete(&job.id).await.unwrap();

        db.close().await;
    }

    #[tokio::test]
    async fn company_crud_with_arrays() {
        let Some(db) = setup_db().await else {
            return;
        };
        let repo = CompanyRepository::new(db.pool().clone());

        let mut company = Company::new("ArrayTestCo");
        company.tech_stack = vec!["Rust".into(), "PostgreSQL".into()];
        company.culture_keywords = vec!["remote-first".into()];
        company.industry = Some("Tech".into());

        repo.insert(&company).await.unwrap();

        let found = repo.find_by_id(&company.id).await.unwrap().unwrap();
        assert_eq!(found.name, "ArrayTestCo");
        assert_eq!(found.tech_stack, vec!["Rust", "PostgreSQL"]);
        assert_eq!(found.culture_keywords, vec!["remote-first"]);

        let mut updated = found;
        updated.tech_stack.push("Tokio".into());
        repo.update(&updated).await.unwrap();

        let found = repo.find_by_id(&company.id).await.unwrap().unwrap();
        assert_eq!(found.tech_stack.len(), 3);

        let list = repo.list(&Pagination::default()).await.unwrap();
        assert!(list.iter().any(|c| c.id == company.id));

        repo.delete(&company.id).await.unwrap();

        db.close().await;
    }

    #[tokio::test]
    async fn contact_crud_with_company_fk() {
        let Some(db) = setup_db().await else {
            return;
        };
        let company_repo = CompanyRepository::new(db.pool().clone());
        let contact_repo = ContactRepository::new(db.pool().clone());

        let company = Company::new("ContactTestCo");
        company_repo.insert(&company).await.unwrap();

        let mut contact = Contact::new("Alice Test");
        contact.company_id = Some(company.id);
        contact.email = Some("alice@test.com".into());
        contact.role = Some("Engineer".into());
        contact_repo.insert(&contact).await.unwrap();

        let found = contact_repo.find_by_id(&contact.id).await.unwrap().unwrap();
        assert_eq!(found.name, "Alice Test");
        assert_eq!(found.company_id, Some(company.id));

        let mut updated = found;
        updated.role = Some("Senior Engineer".into());
        contact_repo.update(&updated).await.unwrap();

        let found = contact_repo.find_by_id(&contact.id).await.unwrap().unwrap();
        assert_eq!(found.role.as_deref(), Some("Senior Engineer"));

        let list = contact_repo.list(&Pagination::default()).await.unwrap();
        assert!(list.iter().any(|c| c.id == contact.id));

        contact_repo.delete(&contact.id).await.unwrap();
        company_repo.delete(&company.id).await.unwrap();

        db.close().await;
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_missing() {
        let Some(db) = setup_db().await else {
            return;
        };
        let repo = JobRepository::new(db.pool().clone());
        let result = repo.find_by_id(&JobId::new()).await.unwrap();
        assert!(result.is_none());
        db.close().await;
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let Some(db) = setup_db().await else {
            return;
        };
        let repo = JobRepository::new(db.pool().clone());
        repo.delete(&JobId::new()).await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn update_missing_returns_not_found() {
        let Some(db) = setup_db().await else {
            return;
        };
        let repo = JobRepository::new(db.pool().clone());
        let job = Job::new("Ghost Job");
        let result = repo.update(&job).await;
        assert!(result.is_err());
        db.close().await;
    }

    #[tokio::test]
    async fn transition_stage_succeeds() {
        let Some(db) = setup_db().await else {
            return;
        };
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job = Job::new("Transition Test Job");
        job_repo.insert(&job).await.unwrap();

        let app = Application::new(job.id);
        app_repo.insert(&app).await.unwrap();

        let transition = app_repo
            .transition_stage(&app.id, ApplicationStage::Applied, Some("submitted resume"))
            .await
            .unwrap();

        assert_eq!(transition.from_stage, ApplicationStage::Interested);
        assert_eq!(transition.to_stage, ApplicationStage::Applied);
        assert_eq!(transition.notes.as_deref(), Some("submitted resume"));

        let updated = app_repo.find_by_id(&app.id).await.unwrap().unwrap();
        assert_eq!(updated.stage, ApplicationStage::Applied);

        app_repo.delete(&app.id).await.unwrap();
        job_repo.delete(&job.id).await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn transition_stage_invalid_rejects() {
        let Some(db) = setup_db().await else {
            return;
        };
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job = Job::new("Invalid Transition Job");
        job_repo.insert(&job).await.unwrap();

        let app = Application::new(job.id);
        app_repo.insert(&app).await.unwrap();

        let result = app_repo
            .transition_stage(&app.id, ApplicationStage::Technical, None)
            .await;
        assert!(result.is_err());

        let unchanged = app_repo.find_by_id(&app.id).await.unwrap().unwrap();
        assert_eq!(unchanged.stage, ApplicationStage::Interested);

        app_repo.delete(&app.id).await.unwrap();
        job_repo.delete(&job.id).await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn transition_history_returns_ordered() {
        let Some(db) = setup_db().await else {
            return;
        };
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job = Job::new("History Test Job");
        job_repo.insert(&job).await.unwrap();

        let app = Application::new(job.id);
        app_repo.insert(&app).await.unwrap();

        app_repo
            .transition_stage(&app.id, ApplicationStage::Applied, Some("step 1"))
            .await
            .unwrap();
        app_repo
            .transition_stage(&app.id, ApplicationStage::PhoneScreen, Some("step 2"))
            .await
            .unwrap();
        app_repo
            .transition_stage(&app.id, ApplicationStage::Technical, Some("step 3"))
            .await
            .unwrap();

        let history = app_repo.transition_history(&app.id).await.unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].from_stage, ApplicationStage::Interested);
        assert_eq!(history[0].to_stage, ApplicationStage::Applied);
        assert_eq!(history[1].from_stage, ApplicationStage::Applied);
        assert_eq!(history[1].to_stage, ApplicationStage::PhoneScreen);
        assert_eq!(history[2].from_stage, ApplicationStage::PhoneScreen);
        assert_eq!(history[2].to_stage, ApplicationStage::Technical);
        assert!(history[0].transitioned_at <= history[1].transitioned_at);
        assert!(history[1].transitioned_at <= history[2].transitioned_at);

        app_repo.delete(&app.id).await.unwrap();
        job_repo.delete(&job.id).await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn transition_stage_not_found() {
        let Some(db) = setup_db().await else {
            return;
        };
        let app_repo = ApplicationRepository::new(db.pool().clone());
        let fake_id = ApplicationId::new();
        let result = app_repo
            .transition_stage(&fake_id, ApplicationStage::Applied, None)
            .await;
        assert!(result.is_err());
        db.close().await;
    }
}
