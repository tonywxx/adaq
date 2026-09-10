//! Operating-system credential storage behind a small production/test seam.
//!
//! The production store writes through the `keyring` crate, which maps to
//! macOS Keychain, Windows Credential Manager, or the Linux Secret Service
//! depending on the platform. Tests use the in-memory store; both implement
//! the same `SecretStore` trait, so no test path ever touches the OS store.
//! The opt-in `local-env-credentials` build reads only the repository `.env`
//! for local OKX Demo tests and never falls back to the OS store.
//!
//! Keep the `apple-native`/`windows-native`/`sync-secret-service` features
//! in Cargo.toml: without a matching platform feature, keyring silently
//! falls back to an in-memory mock store and nothing reaches the OS.

#[cfg(all(feature = "local-env-credentials", not(debug_assertions)))]
compile_error!("local-env-credentials is restricted to Debug local tests");

#[cfg(any(test, feature = "local-env-credentials"))]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(feature = "local-env-credentials")]
use std::{env, fs, path::Path};

#[cfg(not(feature = "local-env-credentials"))]
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

#[cfg(feature = "local-env-credentials")]
pub(crate) struct LocalEnvSecretStore {
    credential_json: String,
}

#[cfg(feature = "local-env-credentials")]
impl LocalEnvSecretStore {
    pub(crate) fn load() -> Result<Self, String> {
        let values = load_local_env()?;
        Ok(Self {
            credential_json: credential_json_from_values(&values)?,
        })
    }
}

#[cfg(feature = "local-env-credentials")]
impl SecretStore for LocalEnvSecretStore {
    fn set(&self, _entry: &str, _value: &str) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable(
            "local .env credential store is read-only".into(),
        ))
    }

    fn get(&self, _entry: &str) -> Result<String, SecretStoreError> {
        Ok(self.credential_json.clone())
    }

    fn delete(&self, _entry: &str) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unavailable(
            "local .env credential store is read-only".into(),
        ))
    }
}

#[cfg(feature = "local-env-credentials")]
fn load_local_env() -> Result<HashMap<String, String>, String> {
    let mut locations = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        locations.push(current_dir);
    }
    if let Ok(executable) = env::current_exe() {
        locations.push(executable);
    }

    for location in locations {
        for path in location.ancestors() {
            if path.join(".git").exists() {
                return read_env_file(&path.join(".env"));
            }
        }
    }

    Err("Local OKX Demo test requires a repository root".to_owned())
}

#[cfg(feature = "local-env-credentials")]
fn read_env_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|_| "Local OKX Demo test requires a readable .env file".to_owned())?;
    Ok(parse_local_env(&content))
}

#[cfg(feature = "local-env-credentials")]
fn parse_local_env(content: &str) -> HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(value);
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(feature = "local-env-credentials")]
fn credential_json_from_values(values: &HashMap<String, String>) -> Result<String, String> {
    let required = |name: &str| {
        values
            .get(name)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| format!("Local OKX Demo test is missing {name}"))
    };
    serde_json::to_string(&serde_json::json!({
        "provider": "okx_demo",
        "api_key": required("OKX_DEMO_API_KEY")?,
        "secret_key": required("OKX_DEMO_API_SECRET")?,
        "passphrase": required("OKX_DEMO_API_PASSPHRASE")?,
    }))
    .map_err(|_| "Local OKX Demo credentials could not be prepared".to_owned())
}

/// macOS Keychain / Windows Credential Manager / Linux Secret Service.
#[cfg(not(feature = "local-env-credentials"))]
pub(crate) struct KeyringSecretStore;

#[cfg(not(feature = "local-env-credentials"))]
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
    use super::{InMemorySecretStore, SecretStore, SecretStoreError};

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
}

#[cfg(all(test, feature = "local-env-credentials"))]
mod local_env_tests {
    use super::{
        LocalEnvSecretStore, SecretStore, SecretStoreError, credential_json_from_values,
        parse_local_env,
    };
    use std::collections::HashMap;

    #[test]
    fn local_env_parser_ignores_comments_and_unquotes_values() {
        let values = parse_local_env(
            "# ignored\nexport OKX_DEMO_API_KEY='key'\nOKX_DEMO_API_SECRET=secret\n",
        );

        assert_eq!(values.get("OKX_DEMO_API_KEY"), Some(&"key".to_owned()));
        assert_eq!(
            values.get("OKX_DEMO_API_SECRET"),
            Some(&"secret".to_owned())
        );
    }

    #[test]
    fn local_env_credentials_fail_closed_when_required_value_is_missing() {
        let values = HashMap::from([
            ("OKX_DEMO_API_KEY".to_owned(), "key".to_owned()),
            ("OKX_DEMO_API_SECRET".to_owned(), "secret".to_owned()),
        ]);

        assert!(credential_json_from_values(&values).is_err());
    }

    #[test]
    fn repository_env_loads_okx_credentials_without_writing_or_printing_them() {
        let store = LocalEnvSecretStore::load().expect("repository .env should be available");
        let credentials: serde_json::Value =
            serde_json::from_str(&store.get("okx_demo").expect("OKX Demo credentials"))
                .expect("credential JSON");

        assert_eq!(credentials["provider"], "okx_demo");
        for name in ["api_key", "secret_key", "passphrase"] {
            assert!(
                credentials[name]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
        }
        assert!(matches!(
            store.set("okx_demo", "not-written"),
            Err(SecretStoreError::Unavailable(_))
        ));
    }
}
