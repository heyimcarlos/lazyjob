use crate::domain::{Contact, Job};

use super::types::WarmPath;

pub fn warm_paths_for_job(contacts: &[Contact], job: &Job) -> Vec<WarmPath> {
    let company = match &job.company_name {
        Some(name) if !name.is_empty() => name,
        _ => return Vec::new(),
    };

    let company_lower = company.to_lowercase();

    contacts
        .iter()
        .filter(|c| {
            c.current_company
                .as_ref()
                .is_some_and(|cc| cc.to_lowercase() == company_lower)
        })
        .map(|c| WarmPath {
            contact_name: c.name.clone(),
            contact_role: c.role.clone(),
            contact_email: c.email.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_contact(name: &str, company: &str, role: &str) -> Contact {
        let mut c = Contact::new(name);
        c.current_company = Some(company.into());
        c.role = Some(role.into());
        c.email = Some(format!(
            "{}@example.com",
            name.to_lowercase().replace(' ', ".")
        ));
        c
    }

    fn make_job(title: &str, company: &str) -> Job {
        let mut j = Job::new(title);
        j.company_name = Some(company.into());
        j
    }

    #[test]
    fn finds_contacts_at_same_company() {
        let contacts = vec![
            make_contact("Alice Smith", "Acme Corp", "Engineer"),
            make_contact("Bob Jones", "Widget Inc", "Manager"),
            make_contact("Charlie Brown", "Acme Corp", "Designer"),
        ];
        let job = make_job("Senior Engineer", "Acme Corp");

        let paths = warm_paths_for_job(&contacts, &job);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].contact_name, "Alice Smith");
        assert_eq!(paths[1].contact_name, "Charlie Brown");
    }

    #[test]
    fn case_insensitive_matching() {
        let contacts = vec![make_contact("Alice", "ACME CORP", "Engineer")];
        let job = make_job("Engineer", "acme corp");

        let paths = warm_paths_for_job(&contacts, &job);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].contact_name, "Alice");
    }

    #[test]
    fn no_matches_returns_empty() {
        let contacts = vec![make_contact("Alice", "Other Co", "Engineer")];
        let job = make_job("Engineer", "Acme Corp");

        let paths = warm_paths_for_job(&contacts, &job);
        assert!(paths.is_empty());
    }

    #[test]
    fn job_without_company_returns_empty() {
        let contacts = vec![make_contact("Alice", "Acme Corp", "Engineer")];
        let job = Job::new("Engineer");

        let paths = warm_paths_for_job(&contacts, &job);
        assert!(paths.is_empty());
    }

    #[test]
    fn contacts_without_company_are_skipped() {
        let contacts = vec![Contact::new("Alice")];
        let job = make_job("Engineer", "Acme Corp");

        let paths = warm_paths_for_job(&contacts, &job);
        assert!(paths.is_empty());
    }

    #[test]
    fn warm_path_includes_role_and_email() {
        let contacts = vec![make_contact("Alice Smith", "Acme Corp", "Staff Engineer")];
        let job = make_job("Engineer", "Acme Corp");

        let paths = warm_paths_for_job(&contacts, &job);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].contact_role.as_deref(), Some("Staff Engineer"));
        assert_eq!(
            paths[0].contact_email.as_deref(),
            Some("alice.smith@example.com")
        );
    }
}
