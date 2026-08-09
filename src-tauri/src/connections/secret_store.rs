//! Operating-system credential storage behind a small production/test seam.
//!
//! The production store writes through the `keyring` crate, which maps to
//! macOS Keychain, Windows Credential Manager, or the Linux Secret Service
//! depending on the platform. Tests use the in-memory store; both implement
//! the same `SecretStore` trait, so no test path ever touches the OS store.
//!
//! Keep the `apple-native`/`windows-native`/`sync-secret-service` features
//! in Cargo.toml: without a matching platform feature, keyring silently
//! falls back to an in-memory mock store and nothing reaches the OS.

#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

const KEYRING_SERVICE: &str = "adaq";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SecretStoreError {
    /// The referenced entry does not exist.
    Missing,
    /// The OS store could not be reached or rejected the operation.
    Unavailable(String),
}

pub(crate) trait SecretStore: Send + Sync {
    fn set(&self, entry: &str, value: &str) -> Result<(), SecretStoreError>;
    fn get(&self, entry: &str) -> Result<String, SecretStoreError>;
    fn delete(&self, entry: &str) -> Result<(), SecretStoreError>;
}

/// macOS Keychain / Windows Credential Manager / Linux Secret Service.
pub(crate) struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn set(&self, entry: &str, value: &str) -> Result<(), SecretStoreError> {
        let credential = keyring::Entry::new(KEYRING_SERVICE, entry)
            .map_err(|error| SecretStoreError::Unavailable(error.to_string()))?;
        credential
            .set_password(value)
            .map_err(|error| SecretStoreError::Unavailable(error.to_string()))
    }

    fn get(&self, entry: &str) -> Result<String, SecretStoreError> {
        let credential = keyring::Entry::new(KEYRING_SERVICE, entry)
            .map_err(|error| SecretStoreError::Unavailable(error.to_string()))?;
        credential.get_password().map_err(|error| match error {
            keyring::Error::NoEntry => SecretStoreError::Missing,
            other => SecretStoreError::Unavailable(other.to_string()),
        })
    }

    fn delete(&self, entry: &str) -> Result<(), SecretStoreError> {
        let credential = keyring::Entry::new(KEYRING_SERVICE, entry)
            .map_err(|error| SecretStoreError::Unavailable(error.to_string()))?;
        credential.delete_credential().map_err(|error| match error {
            keyring::Error::NoEntry => SecretStoreError::Missing,
            other => SecretStoreError::Unavailable(other.to_string()),
        })
    }
}

/// Test-only store; never used by production code paths.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct InMemorySecretStore(Mutex<HashMap<String, String>>);

#[cfg(test)]
impl InMemorySecretStore {
    pub(crate) fn entries(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = self
            .0
            .lock()
            .expect("in-memory secret store poisoned")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        entries.sort();
        entries
    }

    pub(crate) fn clear(&self) {
        self.0
            .lock()
            .expect("in-memory secret store poisoned")
            .clear();
    }
}

#[cfg(test)]
impl SecretStore for InMemorySecretStore {
    fn set(&self, entry: &str, value: &str) -> Result<(), SecretStoreError> {
        let mut entries = self.0.lock().expect("in-memory secret store poisoned");
        entries.insert(entry.to_owned(), value.to_owned());
        Ok(())
    }

    fn get(&self, entry: &str) -> Result<String, SecretStoreError> {
        let entries = self.0.lock().expect("in-memory secret store poisoned");
        entries.get(entry).cloned().ok_or(SecretStoreError::Missing)
    }

    fn delete(&self, entry: &str) -> Result<(), SecretStoreError> {
        let mut entries = self.0.lock().expect("in-memory secret store poisoned");
        entries
            .remove(entry)
            .map(|_| ())
            .ok_or(SecretStoreError::Missing)
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemorySecretStore, KeyringSecretStore, SecretStore, SecretStoreError};

    #[test]
    fn in_memory_store_round_trips_and_misses() {
        let store = InMemorySecretStore::default();
        assert_eq!(store.get("a"), Err(SecretStoreError::Missing));
        store.set("a", "value").unwrap();
        assert_eq!(store.get("a").unwrap(), "value");
        store.delete("a").unwrap();
        assert_eq!(store.get("a"), Err(SecretStoreError::Missing));
        assert_eq!(store.delete("a"), Err(SecretStoreError::Missing));
    }

    /// Exercises the real OS store on the current platform without exposing
    /// the value. Ignored by default so CI never touches the Keychain;
    /// run locally as `cargo test --bin adaq keyring_store_manual -- --ignored`.
    #[test]
    #[ignore]
    fn keyring_store_manual_round_trip() {
        let store = KeyringSecretStore;
        let entry = format!("manual-test-{}", std::process::id());
        store.set(&entry, "not-a-real-credential").unwrap();
        assert_eq!(store.get(&entry).unwrap(), "not-a-real-credential");
        store.delete(&entry).unwrap();
        assert_eq!(store.get(&entry), Err(SecretStoreError::Missing));
    }
}
