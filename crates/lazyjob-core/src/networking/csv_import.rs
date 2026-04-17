use std::io::Read;

use crate::domain::Contact;
use crate::error::{CoreError, Result};

pub fn parse_linkedin_csv<R: Read>(reader: R) -> Result<Vec<Contact>> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(reader);

    let headers = csv_reader
        .headers()
        .map_err(|e| CoreError::Parse(format!("Failed to read CSV headers: {e}")))?
        .clone();

    let first_name_idx = find_header(&headers, &["First Name", "first_name", "FirstName"]);
    let last_name_idx = find_header(&headers, &["Last Name", "last_name", "LastName"]);
    let email_idx = find_header(&headers, &["Email Address", "email", "Email"]);
    let company_idx = find_header(&headers, &["Company", "company", "Organization"]);
    let position_idx = find_header(&headers, &["Position", "position", "Title", "Job Title"]);
    let url_idx = find_header(&headers, &["URL", "Profile URL", "linkedin_url"]);

    if first_name_idx.is_none() && last_name_idx.is_none() {
        return Err(CoreError::Parse(
            "CSV must have First Name or Last Name columns".into(),
        ));
    }

    let mut contacts = Vec::new();

    for result in csv_reader.records() {
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Skipping malformed CSV row: {e}");
                continue;
            }
        };

        let first = first_name_idx
            .and_then(|i| record.get(i))
            .unwrap_or("")
            .trim();
        let last = last_name_idx
            .and_then(|i| record.get(i))
            .unwrap_or("")
            .trim();

        let name = match (first.is_empty(), last.is_empty()) {
            (false, false) => format!("{first} {last}"),
            (false, true) => first.to_string(),
            (true, false) => last.to_string(),
            (true, true) => continue,
        };

        let mut contact = Contact::new(name);
        contact.email = email_idx
            .and_then(|i| record.get(i))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());
        contact.current_company = company_idx
            .and_then(|i| record.get(i))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());
        contact.role = position_idx
            .and_then(|i| record.get(i))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());
        contact.linkedin_url = url_idx
            .and_then(|i| record.get(i))
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());
        contact.source = Some("linkedin_csv".into());

        contacts.push(contact);
    }

    Ok(contacts)
}

fn find_header(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    for candidate in candidates {
        if let Some(idx) = headers.iter().position(|h| h.trim() == *candidate) {
            return Some(idx);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: proves csv crate Reader::from_reader parses headers and records
    #[test]
    fn csv_crate_parses_headers_and_records() {
        let data = "Name,Age\nAlice,30\nBob,25\n";
        let mut rdr = csv::Reader::from_reader(data.as_bytes());
        let headers = rdr.headers().unwrap();
        assert_eq!(headers.get(0), Some("Name"));
        assert_eq!(headers.get(1), Some("Age"));

        let records: Vec<_> = rdr.records().filter_map(|r| r.ok()).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get(0), Some("Alice"));
        assert_eq!(records[1].get(1), Some("25"));
    }

    // learning test: proves csv crate handles flexible column counts
    #[test]
    fn csv_crate_flexible_mode() {
        let data = "A,B,C\n1,2\n1,2,3,4\n";
        let mut rdr = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(data.as_bytes());
        let records: Vec<_> = rdr.records().filter_map(|r| r.ok()).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].len(), 2);
        assert_eq!(records[1].len(), 4);
    }

    #[test]
    fn parse_standard_linkedin_csv() {
        let csv_data = "First Name,Last Name,Email Address,Company,Position\n\
                        Alice,Smith,alice@example.com,Acme Corp,Software Engineer\n\
                        Bob,Jones,bob@example.com,Widget Inc,Product Manager\n";

        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(contacts.len(), 2);

        assert_eq!(contacts[0].name, "Alice Smith");
        assert_eq!(contacts[0].email.as_deref(), Some("alice@example.com"));
        assert_eq!(contacts[0].current_company.as_deref(), Some("Acme Corp"));
        assert_eq!(contacts[0].role.as_deref(), Some("Software Engineer"));
        assert_eq!(contacts[0].source.as_deref(), Some("linkedin_csv"));

        assert_eq!(contacts[1].name, "Bob Jones");
        assert_eq!(contacts[1].current_company.as_deref(), Some("Widget Inc"));
    }

    #[test]
    fn parse_csv_with_missing_columns() {
        let csv_data = "First Name,Last Name,Email Address\n\
                        Alice,Smith,alice@example.com\n";

        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].current_company.is_none());
        assert!(contacts[0].role.is_none());
    }

    #[test]
    fn parse_csv_skips_empty_name_rows() {
        let csv_data = "First Name,Last Name,Email Address\n\
                        ,,empty@example.com\n\
                        Alice,Smith,alice@example.com\n";

        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].name, "Alice Smith");
    }

    #[test]
    fn parse_csv_with_only_first_name() {
        let csv_data = "First Name,Last Name\nAlice,\n";
        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].name, "Alice");
    }

    #[test]
    fn parse_csv_trims_whitespace() {
        let csv_data = "First Name,Last Name,Email Address,Company\n\
                        Alice , Smith , alice@example.com , Acme Corp \n";
        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(contacts[0].name, "Alice Smith");
        assert_eq!(contacts[0].email.as_deref(), Some("alice@example.com"));
        assert_eq!(contacts[0].current_company.as_deref(), Some("Acme Corp"));
    }

    #[test]
    fn parse_csv_with_url_column() {
        let csv_data = "First Name,Last Name,URL\n\
                        Alice,Smith,https://linkedin.com/in/alice\n";
        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert_eq!(
            contacts[0].linkedin_url.as_deref(),
            Some("https://linkedin.com/in/alice")
        );
    }

    #[test]
    fn parse_csv_rejects_missing_name_headers() {
        let csv_data = "Email,Company\nalice@ex.com,Acme\n";
        let result = parse_linkedin_csv(csv_data.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn parse_csv_empty_file() {
        let csv_data = "First Name,Last Name\n";
        let contacts = parse_linkedin_csv(csv_data.as_bytes()).unwrap();
        assert!(contacts.is_empty());
    }
}
