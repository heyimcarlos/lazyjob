use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ApplicationId, OfferId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Offer {
    pub id: OfferId,
    pub application_id: ApplicationId,
    pub salary: Option<i64>,
    pub equity: Option<String>,
    pub benefits: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub accepted: Option<bool>,
    pub notes: Option<String>,
}

impl Offer {
    pub fn new(application_id: ApplicationId) -> Self {
        Self {
            id: OfferId::new(),
            application_id,
            salary: None,
            equity: None,
            benefits: None,
            deadline: None,
            accepted: None,
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_construction() {
        let offer = Offer::new(ApplicationId::new());
        assert!(offer.salary.is_none());
        assert!(offer.accepted.is_none());
    }

    #[test]
    fn offer_serde_round_trip() {
        let mut offer = Offer::new(ApplicationId::new());
        offer.salary = Some(150_000);
        offer.equity = Some("0.1%".into());
        offer.accepted = Some(true);

        let json = serde_json::to_string(&offer).unwrap();
        let deserialized: Offer = serde_json::from_str(&json).unwrap();
        assert_eq!(offer, deserialized);
    }
}
