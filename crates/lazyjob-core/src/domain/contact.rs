use serde::{Deserialize, Serialize};

use super::ids::{CompanyId, ContactId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contact {
    pub id: ContactId,
    pub name: String,
    pub role: Option<String>,
    pub email: Option<String>,
    pub linkedin_url: Option<String>,
    pub company_id: Option<CompanyId>,
    pub relationship: Option<String>,
    pub notes: Option<String>,
}

impl Contact {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ContactId::new(),
            name: name.into(),
            role: None,
            email: None,
            linkedin_url: None,
            company_id: None,
            relationship: None,
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_construction() {
        let contact = Contact::new("Alice Smith");
        assert_eq!(contact.name, "Alice Smith");
        assert!(contact.company_id.is_none());
    }

    #[test]
    fn contact_serde_round_trip() {
        let mut contact = Contact::new("Bob Jones");
        contact.email = Some("bob@example.com".into());
        contact.company_id = Some(CompanyId::new());
        contact.relationship = Some("Former colleague".into());

        let json = serde_json::to_string(&contact).unwrap();
        let deserialized: Contact = serde_json::from_str(&json).unwrap();
        assert_eq!(contact, deserialized);
    }
}
