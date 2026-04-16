pub mod config;
pub mod credentials;
pub mod db;
pub mod discovery;
pub mod domain;
pub mod error;
pub mod life_sheet;
pub mod repositories;

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
