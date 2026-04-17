pub mod config;
pub mod cover_letter;
pub mod credentials;
pub mod db;
pub mod discovery;
pub mod domain;
pub mod error;
pub mod life_sheet;
pub mod networking;
pub mod repositories;
pub mod resume;
pub mod stats;
#[cfg(any(test, feature = "integration"))]
pub mod test_db;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_returns_crate_version() {
        assert_eq!(version(), "0.1.0");
    }
}
