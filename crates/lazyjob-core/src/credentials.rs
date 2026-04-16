use std::collections::HashMap;
use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};

use crate::error::{CoreError, Result};

const SERVICE: &str = "lazyjob";

fn api_key_user(provider: &str) -> String {
    format!("api_key:{provider}")
}

pub trait CredentialStore: Send + Sync {
    fn set(&self, user: &str, password: &str) -> std::result::Result<(), String>;
    fn get(&self, user: &str) -> std::result::Result<Option<String>, String>;
    fn delete(&self, user: &str) -> std::result::Result<(), String>;
}

struct KeyringStore;

impl CredentialStore for KeyringStore {
    fn set(&self, user: &str, password: &str) -> std::result::Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, user).map_err(|e| e.to_string())?;
        entry.set_password(password).map_err(|e| e.to_string())
    }

    fn get(&self, user: &str) -> std::result::Result<Option<String>, String> {
        let entry = keyring::Entry::new(SERVICE, user).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn delete(&self, user: &str) -> std::result::Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, user).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub struct InMemoryStore {
    store: Mutex<HashMap<String, String>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for InMemoryStore {
    fn set(&self, user: &str, password: &str) -> std::result::Result<(), String> {
        self.store
            .lock()
            .unwrap()
            .insert(user.to_string(), password.to_string());
        Ok(())
    }

    fn get(&self, user: &str) -> std::result::Result<Option<String>, String> {
        Ok(self.store.lock().unwrap().get(user).cloned())
    }

    fn delete(&self, user: &str) -> std::result::Result<(), String> {
        self.store.lock().unwrap().remove(user);
        Ok(())
    }
}

pub struct CredentialManager {
    store: Box<dyn CredentialStore>,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            store: Box::new(KeyringStore),
        }
    }

    pub fn with_store(store: Box<dyn CredentialStore>) -> Self {
        Self { store }
    }

    pub fn set_api_key(&self, provider: &str, key: &SecretString) -> Result<()> {
        self.store
            .set(&api_key_user(provider), key.expose_secret())
            .map_err(CoreError::Credential)
    }

    pub fn get_api_key(&self, provider: &str) -> Result<Option<SecretString>> {
        self.store
            .get(&api_key_user(provider))
            .map(|opt| opt.map(SecretString::new))
            .map_err(CoreError::Credential)
    }

    pub fn delete_api_key(&self, provider: &str) -> Result<()> {
        self.store
            .delete(&api_key_user(provider))
            .map_err(CoreError::Credential)
    }
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn mock_cred() -> CredentialManager {
        CredentialManager::with_store(Box::new(InMemoryStore::new()))
    }

    // learning test: verifies keyring Entry API compiles and mock builder exists
    #[test]
    fn keyring_entry_api_compiles() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        let entry = keyring::Entry::new(SERVICE, "compile_check").unwrap();
        entry.set_password("val").unwrap();
        let got = entry.get_password().unwrap();
        assert_eq!(got, "val");
    }

    // learning test: verifies SecretString wraps and exposes correctly
    #[test]
    fn secrecy_expose_secret() {
        let secret = SecretString::new("my-api-key".to_string());
        assert_eq!(secret.expose_secret(), "my-api-key");
    }

    // learning test: verifies InMemoryStore shares state across calls
    #[test]
    fn in_memory_store_round_trip() {
        let store = InMemoryStore::new();
        store.set("key1", "value1").unwrap();
        assert_eq!(store.get("key1").unwrap(), Some("value1".to_string()));
        store.delete("key1").unwrap();
        assert_eq!(store.get("key1").unwrap(), None);
    }

    #[test]
    fn set_and_get_api_key() {
        let cred = mock_cred();
        let key = SecretString::new("sk-ant-test-123".to_string());
        cred.set_api_key("anthropic", &key).unwrap();
        let retrieved = cred.get_api_key("anthropic").unwrap().unwrap();
        assert_eq!(retrieved.expose_secret(), "sk-ant-test-123");
    }

    #[test]
    fn get_missing_key_returns_none() {
        let cred = mock_cred();
        let result = cred.get_api_key("nonexistent_provider").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_api_key() {
        let cred = mock_cred();
        let key = SecretString::new("sk-test".to_string());
        cred.set_api_key("openai", &key).unwrap();
        cred.delete_api_key("openai").unwrap();
        assert!(cred.get_api_key("openai").unwrap().is_none());
    }

    #[test]
    fn delete_missing_key_is_ok() {
        let cred = mock_cred();
        assert!(cred.delete_api_key("nonexistent").is_ok());
    }

    #[test]
    fn multiple_providers_independent() {
        let cred = mock_cred();
        let key1 = SecretString::new("key-anthropic".to_string());
        let key2 = SecretString::new("key-openai".to_string());
        cred.set_api_key("anthropic", &key1).unwrap();
        cred.set_api_key("openai", &key2).unwrap();
        assert_eq!(
            cred.get_api_key("anthropic")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "key-anthropic"
        );
        assert_eq!(
            cred.get_api_key("openai").unwrap().unwrap().expose_secret(),
            "key-openai"
        );
        cred.delete_api_key("anthropic").unwrap();
        assert!(cred.get_api_key("anthropic").unwrap().is_none());
        assert!(cred.get_api_key("openai").unwrap().is_some());
    }

    #[test]
    fn overwrite_existing_key() {
        let cred = mock_cred();
        let key1 = SecretString::new("old-key".to_string());
        let key2 = SecretString::new("new-key".to_string());
        cred.set_api_key("provider", &key1).unwrap();
        cred.set_api_key("provider", &key2).unwrap();
        assert_eq!(
            cred.get_api_key("provider")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "new-key"
        );
    }
}
