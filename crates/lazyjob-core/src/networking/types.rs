use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WarmPath {
    pub contact_name: String,
    pub contact_role: Option<String>,
    pub contact_email: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_path_serde_round_trip() {
        let wp = WarmPath {
            contact_name: "Alice".into(),
            contact_role: Some("Engineer".into()),
            contact_email: Some("alice@example.com".into()),
        };
        let json = serde_json::to_string(&wp).unwrap();
        let deserialized: WarmPath = serde_json::from_str(&json).unwrap();
        assert_eq!(wp, deserialized);
    }

    #[test]
    fn import_result_defaults() {
        let result = ImportResult::default();
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 0);
        assert!(result.errors.is_empty());
    }
}
