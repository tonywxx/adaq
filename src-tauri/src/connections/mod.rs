//! The Host-owned Paper Connection domain.
//!
//! Owns Profile metadata, opaque Secret Reference identity, the legal
//! lifecycle (save-with-validation, read-only test, atomic rotation,
//! guarded deletion), typed redacted diagnostics, and the fixed provider
//! environments. Credential values live only in the operating-system
//! secret store; SQLite rows in this domain never contain a secret, and
//! every row is scoped to the current ADAQ User and device.

pub(crate) mod secret_store;
pub(crate) mod tester;
#[cfg(test)]
mod tests;

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use adaq_data_core::alpaca::{AlpacaClient, AlpacaCredentials};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use secret_store::{KeyringSecretStore, SecretStore};
use tester::{
    ALPACA_MARKET_DATA_ENDPOINT, ConnectionEvidence, ConnectionTester, HttpExecutor,
    ReqwestExecutor, TestCredential, TestFailure,
};

pub(crate) use tester::{ALPACA_PAPER_TRADING_ENDPOINT, OKX_DEMO_ENDPOINT};

const ALPACA_PAPER_ENVIRONMENT: &str = "alpaca_paper";
const OKX_DEMO_ENVIRONMENT: &str = "okx_demo";
const MAX_CREDENTIAL_LENGTH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provider {
    AlpacaPaper,
    OkxDemo,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AlpacaPaper => "alpaca_paper",
            Self::OkxDemo => "okx_demo",
        }
    }

    /// The fixed Paper/Demo environment for the provider. V1 never accepts
    /// a Live environment or an arbitrary custom endpoint.
    pub fn environment(self) -> &'static str {
        match self {
            Self::AlpacaPaper => ALPACA_PAPER_ENVIRONMENT,
            Self::OkxDemo => OKX_DEMO_ENVIRONMENT,
        }
    }

    /// The fixed trading endpoint the Connection Test authenticates
    /// against. Market-data endpoints are allowlisted separately and are
    /// never reachable with credentials from the test.
    pub fn trading_endpoint(self) -> &'static str {
        match self {
            Self::AlpacaPaper => ALPACA_PAPER_TRADING_ENDPOINT,
            Self::OkxDemo => OKX_DEMO_ENDPOINT,
        }
    }

    /// The complete fixed endpoint allowlist for the provider. The tester
    /// refuses any request outside this list, so a Live or arbitrary
    /// custom endpoint can never be reached.
    pub fn fixed_endpoints(self) -> &'static [&'static str] {
        match self {
            Self::AlpacaPaper => &[ALPACA_PAPER_TRADING_ENDPOINT, ALPACA_MARKET_DATA_ENDPOINT],
            Self::OkxDemo => &[OKX_DEMO_ENDPOINT],
        }
    }
}

/// Credential values as entered by the User. Deserialized at the Tauri
/// boundary, validated here, and written to the OS store; the values never
/// enter SQLite, logs, or frontend state after this struct is consumed.
#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub(crate) enum ProviderCredentials {
    AlpacaPaper {
        #[serde(rename = "keyId")]
        key_id: String,
        #[serde(rename = "secretKey")]
        secret_key: String,
    },
    OkxDemo {
        #[serde(rename = "apiKey")]
        api_key: String,
        #[serde(rename = "secretKey")]
        secret_key: String,
        passphrase: String,
    },
}

impl ProviderCredentials {
    pub fn provider(&self) -> Provider {
        match self {
            Self::AlpacaPaper { .. } => Provider::AlpacaPaper,
            Self::OkxDemo { .. } => Provider::OkxDemo,
        }
    }

    /// Opaque credential value for OS-store write and in-Host test.
    fn into_test_credential(self) -> TestCredential {
        match self {
            Self::AlpacaPaper { key_id, secret_key } => {
                TestCredential::AlpacaPaper { key_id, secret_key }
            }
            Self::OkxDemo {
                api_key,
                secret_key,
                passphrase,
            } => TestCredential::OkxDemo {
                api_key,
                secret_key,
                passphrase,
            },
        }
    }

    fn validate(&self) -> Result<(), ConnectionError> {
        let values: Vec<&str> = match self {
            Self::AlpacaPaper { key_id, secret_key } => vec![key_id, secret_key],
            Self::OkxDemo {
                api_key,
                secret_key,
                passphrase,
            } => vec![api_key, secret_key, passphrase],
        };
        if values
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > MAX_CREDENTIAL_LENGTH)
        {
            return Err(ConnectionError::new(
                "invalid_input",
                "Credential values must be non-empty and at most 512 characters.",
            ));
        }
        Ok(())
    }
}

/// A random, unguessable identity for one credential entry in the OS store.
/// Only the Host resolves it; the GUI, Workers, and other Users never see
/// or use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecretReference(String);

impl SecretReference {
    fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileStatus {
    Usable,
    Unusable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileView {
    pub profile_id: String,
    pub provider: Provider,
    pub environment: String,
    pub masked_key_suffix: String,
    pub account_id: Option<String>,
    pub currency: Option<String>,
    pub status: ProfileStatus,
    pub last_test_at_ms: Option<i64>,
    pub last_test_evidence: Option<ConnectionEvidence>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Typed, redacted connection error surfaced to the GUI as a JSON string.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionError {
    pub code: String,
    pub message: String,
}

impl ConnectionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

/// Tells the Connection domain whether a Profile still has an active
/// dependent runtime (for example a running Bot) that must not be cut off
/// mid-operation.
pub(crate) trait RuntimeGuard: Send + Sync {
    fn active_dependent_count(&self, user_id: &str, provider: Provider) -> usize;
}

/// No Bot runtime exists yet in V1, so nothing can depend on a Profile.
pub(crate) struct EmptyRuntimeGuard;

impl RuntimeGuard for EmptyRuntimeGuard {
    fn active_dependent_count(&self, _user_id: &str, _provider: Provider) -> usize {
        0
    }
}

#[derive(Debug, Clone)]
struct ProfileRow {
    profile_id: String,
    user_id: String,
    provider: Provider,
    environment: String,
    secret_reference: String,
    masked_key_suffix: String,
    account_id: Option<String>,
    currency: Option<String>,
    status: ProfileStatus,
    last_test_at_ms: Option<i64>,
    last_test_evidence_json: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl ProfileRow {
    fn view(&self) -> Result<ProfileView, String> {
        let last_test_evidence = match &self.last_test_evidence_json {
            Some(json) => Some(serde_json::from_str(json).map_err(|error| error.to_string())?),
            None => None,
        };
        Ok(ProfileView {
            profile_id: self.profile_id.clone(),
            provider: self.provider,
            environment: self.environment.clone(),
            masked_key_suffix: self.masked_key_suffix.clone(),
            account_id: self.account_id.clone(),
            currency: self.currency.clone(),
            status: self.status,
            last_test_at_ms: self.last_test_at_ms,
            last_test_evidence,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }

    fn os_store_entry(&self) -> String {
        // The User ID is part of the OS-store key, so even a leaked
        // reference cannot resolve under another User's scope.
        os_entry(
            &self.user_id,
            &SecretReference(self.secret_reference.clone()),
        )
    }
}

pub(crate) struct ConnectionManager {
    database: Arc<Mutex<Connection>>,
    secrets: Arc<dyn SecretStore>,
    tester: ConnectionTester,
    runtime_guard: Arc<dyn RuntimeGuard>,
    device_id: String,
    alpaca_rate_gate: Arc<Mutex<Instant>>,
}

impl ConnectionManager {
    /// Opens the Connection domain with production dependencies: the OS
    /// secret store, the real HTTP executor, and the (empty) runtime guard.
    pub(crate) fn open_production(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        Self::open(
            database,
            Arc::new(KeyringSecretStore),
            Arc::new(ReqwestExecutor::new()),
            Arc::new(EmptyRuntimeGuard),
        )
    }

    /// Opens the Connection domain behind the production/test seam: schema
    /// and device scope live here, the secret store and HTTP executor are
    /// injected.
    pub(crate) fn open(
        database: Arc<Mutex<Connection>>,
        secrets: Arc<dyn SecretStore>,
        http: Arc<dyn HttpExecutor>,
        runtime_guard: Arc<dyn RuntimeGuard>,
    ) -> Result<Self, String> {
        let device_id = {
            let connection = database.lock().map_err(|error| error.to_string())?;
            connection
                .execute_batch(
                    "PRAGMA foreign_keys = ON;
                     CREATE TABLE IF NOT EXISTS connection_device (
                        device_id TEXT PRIMARY KEY
                     );
                     CREATE TABLE IF NOT EXISTS connection_profiles (
                        profile_id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        device_id TEXT NOT NULL,
                        provider TEXT NOT NULL,
                        environment TEXT NOT NULL,
                        secret_reference TEXT NOT NULL,
                        masked_key_suffix TEXT NOT NULL,
                        account_id TEXT,
                        currency TEXT,
                        status TEXT NOT NULL,
                        last_test_at_ms INTEGER,
                        last_test_evidence_json TEXT,
                        created_at_ms INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL,
                        UNIQUE(user_id, provider)
                     );",
                )
                .map_err(|error| error.to_string())?;
            connection
                .query_row(
                    "SELECT device_id FROM connection_device LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .unwrap_or_else(|| {
                    let mut bytes = [0u8; 16];
                    rand::thread_rng().fill_bytes(&mut bytes);
                    let id = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                    let _ = connection.execute(
                        "INSERT INTO connection_device(device_id) VALUES (?1)",
                        [&id],
                    );
                    id
                })
        };
        Ok(Self {
            database,
            secrets,
            tester: ConnectionTester::new(http),
            runtime_guard,
            device_id,
            alpaca_rate_gate: Arc::new(Mutex::new(Instant::now())),
        })
    }

    #[cfg(test)]
    pub(crate) fn database(&self) -> Arc<Mutex<Connection>> {
        self.database.clone()
    }

    /// Lists the redacted Profiles of one User on this device.
    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<ProfileView>, String> {
        validate_user_id(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let rows = query_profiles(&database, user_id, &self.device_id)?;
        rows.iter().map(ProfileRow::view).collect()
    }

    /// Resolves one saved Alpaca Paper Profile inside the Host and keeps the
    /// credential inside the caller's Host-side operation.
    pub(crate) fn with_alpaca_client<T>(
        &self,
        user_id: &str,
        operation: impl FnOnce(AlpacaClient) -> T,
    ) -> Result<T, String> {
        validate_user_id(user_id)?;
        let row = {
            let database = self.database.lock().map_err(|error| error.to_string())?;
            query_profile(&database, user_id, &self.device_id, Provider::AlpacaPaper)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "No Alpaca Paper connection is configured.".to_owned())?
        };
        if row.status != ProfileStatus::Usable {
            return Err("The Alpaca Paper connection is unusable; test or re-save it in Settings > Connections.".into());
        }
        let stored = self
            .secrets
            .get(&row.os_store_entry())
            .map_err(|error| match error {
                secret_store::SecretStoreError::Missing => {
                    "The stored Alpaca credential is missing; re-save the connection.".to_owned()
                }
                secret_store::SecretStoreError::Unavailable(_) => {
                    "The operating-system secret store is unavailable.".to_owned()
                }
            })?;
        let credential: TestCredential = serde_json::from_str(&stored).map_err(|_| {
            "The stored Alpaca credential is invalid; re-save the connection.".to_owned()
        })?;
        let TestCredential::AlpacaPaper { key_id, secret_key } = credential else {
            return Err("The saved credential does not match the Alpaca Paper provider.".into());
        };
        Ok(operation(AlpacaClient::with_rate_gate(
            AlpacaCredentials::new(key_id, secret_key),
            self.alpaca_rate_gate.clone(),
        )))
    }

    /// Saves (or rotates) a Profile: writes the secret to the OS store
    /// under a fresh random reference, runs the read-only Connection Test,
    /// and only then atomically switches the Profile row and retires the
    /// previous secret. A failed test leaves any prior Profile usable and
    /// removes the unused new secret.
    pub(crate) fn save(
        &self,
        user_id: &str,
        credentials: ProviderCredentials,
        now_ms: i64,
    ) -> Result<ProfileView, ConnectionError> {
        validate_user_id(user_id)
            .map_err(|message| ConnectionError::new("invalid_input", message))?;
        credentials.validate()?;
        let provider = credentials.provider();
        let credential = credentials.into_test_credential();
        let stored_json = serde_json::to_string(&credential)
            .map_err(|error| ConnectionError::new("internal", error.to_string()))?;

        let reference = SecretReference::generate();
        self.secrets
            .set(&os_entry(user_id, &reference), &stored_json)
            .map_err(|error| {
                ConnectionError::new("secret_store_unavailable", describe_secret_error(error))
            })?;
        // Every failure after this point must remove the fresh secret so a
        // failed save or rotation never orphans a credential entry.
        let cleanup = |secrets: &dyn SecretStore, user_id: &str, reference: &SecretReference| {
            let _ = secrets.delete(&os_entry(user_id, reference));
        };

        let tested = match self.tester.test(&credential, now_ms) {
            Ok(evidence) => evidence,
            Err(failure) => {
                cleanup(self.secrets.as_ref(), user_id, &reference);
                return Err(ConnectionError {
                    code: failure.code.to_owned(),
                    message: failure.redacted_message,
                });
            }
        };
        let account_id = tested.account_id.clone();
        let currency = tested.currency.clone();

        let database = self
            .database
            .lock()
            .map_err(|error| ConnectionError::new("internal", format!("database lock: {error}")))?;
        let previous = query_profile(&database, user_id, &self.device_id, provider)?;
        let previous_reference = previous.as_ref().map(|row| row.secret_reference.clone());
        let profile_id = match previous {
            Some(row) => row.profile_id,
            None => format!("prof-{}", random_hex(16)),
        };
        let evidence_json = match serde_json::to_string(&tested) {
            Ok(json) => json,
            Err(error) => {
                cleanup(self.secrets.as_ref(), user_id, &reference);
                return Err(ConnectionError::new("internal", error.to_string()));
            }
        };
        match database.execute(
            "INSERT INTO connection_profiles(
                profile_id, user_id, device_id, provider, environment, secret_reference,
                masked_key_suffix, account_id, currency, status, last_test_at_ms,
                last_test_evidence_json, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
             ON CONFLICT(user_id, provider) DO UPDATE SET
                device_id = excluded.device_id,
                environment = excluded.environment,
                secret_reference = excluded.secret_reference,
                masked_key_suffix = excluded.masked_key_suffix,
                account_id = excluded.account_id,
                currency = excluded.currency,
                status = excluded.status,
                last_test_at_ms = excluded.last_test_at_ms,
                last_test_evidence_json = excluded.last_test_evidence_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                profile_id,
                user_id,
                self.device_id,
                provider.as_str(),
                provider.environment(),
                reference.as_str(),
                credential.masked_key_suffix(),
                account_id,
                currency,
                ProfileStatus::Usable.as_str(),
                tested.checked_at_ms,
                evidence_json,
                now_ms,
            ],
        ) {
            Err(error) => {
                cleanup(self.secrets.as_ref(), user_id, &reference);
                return Err(ConnectionError::new("internal", error.to_string()));
            }
            Ok(_) => {}
        }

        // The row now points at the new reference; the retired secret is
        // orphaned and safe to remove. Failure here must not fail the save,
        // because the switch already succeeded.
        if let Some(retired) = previous_reference {
            let _ = self
                .secrets
                .delete(&os_entry(user_id, &SecretReference(retired)));
        }
        ProfileRow {
            profile_id,
            user_id: user_id.to_owned(),
            provider,
            environment: provider.environment().to_owned(),
            secret_reference: reference.as_str().to_owned(),
            masked_key_suffix: credential.masked_key_suffix(),
            account_id,
            currency,
            status: ProfileStatus::Usable,
            last_test_at_ms: Some(tested.checked_at_ms),
            last_test_evidence_json: Some(evidence_json),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
        .view()
        .map_err(|message| ConnectionError::new("internal", message))
    }

    /// Re-runs the read-only Connection Test for a saved Profile and
    /// records the typed redacted evidence.
    pub(crate) fn test(
        &self,
        user_id: &str,
        profile_id: &str,
        now_ms: i64,
    ) -> Result<ProfileView, ConnectionError> {
        validate_user_id(user_id)
            .map_err(|message| ConnectionError::new("invalid_input", message))?;

        // Read the row and resolve the credential before any network call
        // so the database lock is never held across provider HTTP traffic.
        let row: ProfileRow = {
            let database = self.database.lock().map_err(|error| {
                ConnectionError::new("internal", format!("database lock: {error}"))
            })?;
            query_profile_by_id(&database, user_id, &self.device_id, profile_id)?
                .ok_or_else(|| ConnectionError::new("invalid_profile", "Profile not found."))?
        };

        let stored = self.secrets.get(&row.os_store_entry()).map_err(|error| {
            let missing = matches!(error, secret_store::SecretStoreError::Missing);
            let (code, message) = if missing {
                (
                    "missing_reference",
                    "The stored credential is missing; re-save the connection.",
                )
            } else {
                (
                    "secret_store_unavailable",
                    "The operating-system secret store is unavailable.",
                )
            };
            if missing {
                let evidence = ConnectionEvidence {
                    outcome: tester::TestOutcome::Failure,
                    error_code: Some(code.to_owned()),
                    redacted_error: Some(message.to_owned()),
                    account_id: None,
                    currency: None,
                    account_status: None,
                    server_time_ms: None,
                    clock_skew_seconds: None,
                    capabilities: Vec::new(),
                    requested_paths: Vec::new(),
                    checked_at_ms: now_ms,
                };
                if let Ok(mut database) = self.database.lock() {
                    let _ = update_profile_evidence(
                        &mut database,
                        &row,
                        ProfileStatus::Unusable,
                        &evidence,
                        now_ms,
                    );
                }
            }
            ConnectionError::new(code, message)
        })?;
        let credential: TestCredential = serde_json::from_str(&stored)
            .map_err(|error| ConnectionError::new("internal", error.to_string()))?;
        drop(stored);

        let tested = self.tester.test(&credential, now_ms);
        let (evidence, failure) = match tested {
            Ok(evidence) => {
                // The Profile is bound to the account identity and currency
                // confirmed at save time; a later test that reports a
                // different account or currency fails closed instead of
                // silently rebinding the Profile.
                let mismatch = row
                    .account_id
                    .as_deref()
                    .zip(evidence.account_id.as_deref())
                    .is_some_and(|(bound, fresh)| bound != fresh)
                    || row
                        .currency
                        .as_deref()
                        .zip(evidence.currency.as_deref())
                        .is_some_and(|(bound, fresh)| bound != fresh);
                if mismatch {
                    let failure = TestFailure::new(
                        "account_mismatch",
                        "The provider account identity or currency changed since the confirmed binding."
                            .to_owned(),
                    );
                    (failure.evidence(now_ms), Some(failure))
                } else {
                    (evidence, None)
                }
            }
            Err(failure) => (failure.evidence(now_ms), Some(failure)),
        };
        {
            let mut database = self.database.lock().map_err(|error| {
                ConnectionError::new("internal", format!("database lock: {error}"))
            })?;
            let status = if failure.is_none() {
                ProfileStatus::Usable
            } else {
                ProfileStatus::Unusable
            };
            let _ = update_profile_evidence(&mut database, &row, status, &evidence, now_ms);
            let view = query_profile_by_id(&database, user_id, &self.device_id, profile_id)?
                .ok_or_else(|| ConnectionError::new("invalid_profile", "Profile not found."))?
                .view()
                .map_err(|message| ConnectionError::new("internal", message))?;
            drop(database);
            if let Some(failure) = failure {
                return Err(ConnectionError {
                    code: failure.code.to_owned(),
                    message: failure.redacted_message,
                });
            }
            Ok(view)
        }
    }

    /// Explicitly removes a Profile: blocked while an active dependent
    /// runtime exists, then deletes the OS secret and invalidates the row.
    pub(crate) fn delete(&self, user_id: &str, profile_id: &str) -> Result<(), ConnectionError> {
        validate_user_id(user_id)
            .map_err(|message| ConnectionError::new("invalid_input", message))?;
        let database = self
            .database
            .lock()
            .map_err(|error| ConnectionError::new("internal", format!("database lock: {error}")))?;
        let row: ProfileRow = query_profile_by_id(&database, user_id, &self.device_id, profile_id)?
            .ok_or_else(|| ConnectionError::new("invalid_profile", "Profile not found."))?;

        let dependents = self
            .runtime_guard
            .active_dependent_count(user_id, row.provider);
        if dependents > 0 {
            return Err(ConnectionError::new(
                "blocked_active_runtime",
                format!(
                    "Deletion is blocked while {dependents} active runtime(s) depend on this Profile."
                ),
            ));
        }

        // The OS secret is removed before the row so a crashed deletion can
        // never leave a live credential behind a stale row.
        let _ = self.secrets.delete(&row.os_store_entry());
        database
            .execute(
                "DELETE FROM connection_profiles WHERE profile_id = ?1",
                [&row.profile_id],
            )
            .map_err(|error| ConnectionError::new("internal", error.to_string()))?;
        Ok(())
    }
}

impl ProfileStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Unusable => "unusable",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "usable" => Some(Self::Usable),
            "unusable" => Some(Self::Unusable),
            _ => None,
        }
    }
}

fn validate_user_id(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("User ID is invalid".into())
    } else {
        Ok(())
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn random_hex(byte_count: usize) -> String {
    let mut bytes = vec![0u8; byte_count];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn os_entry(user_id: &str, reference: &SecretReference) -> String {
    format!("{user_id}:{}", reference.as_str())
}

fn describe_secret_error(error: secret_store::SecretStoreError) -> String {
    match error {
        secret_store::SecretStoreError::Missing => {
            "The operating-system secret store has no such entry.".to_owned()
        }
        secret_store::SecretStoreError::Unavailable(message) => {
            format!("The operating-system secret store is unavailable: {message}")
        }
    }
}

const PROFILE_COLUMNS: &str =
    "profile_id, user_id, device_id, provider, environment, secret_reference,
    masked_key_suffix, account_id, currency, status, last_test_at_ms,
    last_test_evidence_json, created_at_ms, updated_at_ms";

fn query_profiles(
    database: &Connection,
    user_id: &str,
    device_id: &str,
) -> Result<Vec<ProfileRow>, String> {
    let mut statement = database
        .prepare(&format!(
            "SELECT {PROFILE_COLUMNS}
             FROM connection_profiles
             WHERE user_id = ?1 AND device_id = ?2
             ORDER BY provider"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![user_id, device_id], row_to_profile)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn query_profile(
    database: &Connection,
    user_id: &str,
    device_id: &str,
    provider: Provider,
) -> Result<Option<ProfileRow>, ConnectionError> {
    database
        .query_row(
            &format!(
                "SELECT {PROFILE_COLUMNS}
                 FROM connection_profiles
                 WHERE user_id = ?1 AND device_id = ?2 AND provider = ?3"
            ),
            params![user_id, device_id, provider.as_str()],
            row_to_profile,
        )
        .optional()
        .map_err(|error| ConnectionError::new("internal", error.to_string()))
}

fn query_profile_by_id(
    database: &Connection,
    user_id: &str,
    device_id: &str,
    profile_id: &str,
) -> Result<Option<ProfileRow>, ConnectionError> {
    database
        .query_row(
            &format!(
                "SELECT {PROFILE_COLUMNS}
                 FROM connection_profiles
                 WHERE profile_id = ?1 AND user_id = ?2 AND device_id = ?3"
            ),
            params![profile_id, user_id, device_id],
            row_to_profile,
        )
        .optional()
        .map_err(|error| ConnectionError::new("internal", error.to_string()))
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileRow> {
    let provider: String = row.get(3)?;
    let status: String = row.get(9)?;
    // Column 2 is device_id; the row is already device-scoped by the query.
    Ok(ProfileRow {
        profile_id: row.get(0)?,
        user_id: row.get(1)?,
        provider: parse_provider(&provider).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("unknown provider {provider:?}").into(),
            )
        })?,
        environment: row.get(4)?,
        secret_reference: row.get(5)?,
        masked_key_suffix: row.get(6)?,
        account_id: row.get(7)?,
        currency: row.get(8)?,
        status: ProfileStatus::parse(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                format!("unknown status {status:?}").into(),
            )
        })?,
        last_test_at_ms: row.get(10)?,
        last_test_evidence_json: row.get(11)?,
        created_at_ms: row.get(12)?,
        updated_at_ms: row.get(13)?,
    })
}

fn parse_provider(value: &str) -> Option<Provider> {
    match value {
        "alpaca_paper" => Some(Provider::AlpacaPaper),
        "okx_demo" => Some(Provider::OkxDemo),
        _ => None,
    }
}

fn update_profile_evidence(
    database: &mut Connection,
    row: &ProfileRow,
    status: ProfileStatus,
    evidence: &ConnectionEvidence,
    now_ms: i64,
) -> Result<(), String> {
    let evidence_json = serde_json::to_string(evidence).map_err(|error| error.to_string())?;
    database
        .execute(
            "UPDATE connection_profiles
             SET status = ?1, last_test_at_ms = ?2, last_test_evidence_json = ?3, updated_at_ms = ?4
             WHERE profile_id = ?5",
            params![
                status.as_str(),
                evidence.checked_at_ms,
                evidence_json,
                now_ms,
                row.profile_id
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Central redaction: replaces every occurrence of the sensitive values
/// with a fixed marker so provider diagnostics never retain credentials.
pub(crate) fn redact(message: &str, sensitive: &[&str]) -> String {
    let mut out = message.to_owned();
    for value in sensitive {
        if !value.is_empty() {
            out = out.replace(value, "[redacted]");
        }
    }
    out
}
