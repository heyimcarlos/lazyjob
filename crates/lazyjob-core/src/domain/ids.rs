use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
        #[serde(transparent)]
        #[sqlx(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<Uuid> for $name {
            fn from(id: Uuid) -> Self {
                Self(id)
            }
        }
    };
}

define_id!(JobId);
define_id!(ApplicationId);
define_id!(CompanyId);
define_id!(ContactId);
define_id!(InterviewId);
define_id!(OfferId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_display_matches_inner_uuid() {
        let uuid = Uuid::new_v4();
        let id = JobId::from_uuid(uuid);
        assert_eq!(id.to_string(), uuid.to_string());
    }

    #[test]
    fn id_serde_round_trip() {
        let id = ApplicationId::new();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ApplicationId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn different_id_types_not_interchangeable() {
        let uuid = Uuid::new_v4();
        let job_id = JobId::from_uuid(uuid);
        let company_id = CompanyId::from_uuid(uuid);
        assert_eq!(job_id.as_uuid(), company_id.as_uuid());
        // But they are different types — can't accidentally pass one as the other
    }

    #[test]
    fn id_default_generates_unique() {
        let a = ContactId::default();
        let b = ContactId::default();
        assert_ne!(a, b);
    }
}
