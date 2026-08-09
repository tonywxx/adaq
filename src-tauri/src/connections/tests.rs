//! Interface tests for the Connection domain: User isolation,
//! serialization exclusion, redaction, rotation atomicity, deletion
//! blocking, missing-reference failure, endpoint allowlisting,
//! environment mismatch, clock skew, permissions, and zero order requests.
//! All tests use the in-memory secret store and a scripted HTTP executor;
//! production OS-store behavior is covered by the ignored manual test in
//! secret_store.rs.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::{
    ALPACA_PAPER_TRADING_ENDPOINT, ConnectionError, ConnectionManager, OKX_DEMO_ENDPOINT,
    ProfileStatus, Provider, ProviderCredentials, RuntimeGuard, redact,
    secret_store::InMemorySecretStore,
    tester::{self, ALPACA_MARKET_DATA_ENDPOINT, HttpExecutor, HttpRequest, HttpResponse},
};

const NOW_MS: i64 = 1_752_000_000_000;
const ALPACA_KEY_ID: &str = "AK1234567890WXYZ";
const ALPACA_SECRET: &str = "alpaca-secret-value";
const OKX_API_KEY: &str = "OKX-API-KEY-1234";
const OKX_SECRET: &str = "okx-secret-value";
const OKX_PASSPHRASE: &str = "okx-passphrase-value";

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("timestamp in range")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Clone, Debug)]
enum MockResponse {
    Ok { status: u16, body: String },
    Network(String),
}

struct MockHttp {
    routes: Mutex<Vec<(String, MockResponse)>>,
    requests: Mutex<Vec<String>>,
}

impl MockHttp {
    fn new(routes: Vec<(String, MockResponse)>) -> Arc<Self> {
        Arc::new(Self {
            routes: Mutex::new(routes),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn set_routes(&self, routes: Vec<(String, MockResponse)>) {
        *self.routes.lock().expect("mock poisoned") = routes;
    }

    fn requested_paths(&self) -> Vec<String> {
        self.requests.lock().expect("mock poisoned").clone()
    }
}

impl HttpExecutor for MockHttp {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        self.requests
            .lock()
            .expect("mock poisoned")
            .push(request.url.clone());
        let routes = self.routes.lock().expect("mock poisoned");
        for (prefix, response) in routes.iter() {
            if request.url.contains(prefix.as_str()) {
                return match response {
                    MockResponse::Ok { status, body } => Ok(HttpResponse {
                        status: *status,
                        body: body.clone(),
                    }),
                    MockResponse::Network(message) => Err(message.clone()),
                };
            }
        }
        Err(format!("no mock route for {}", request.url))
    }
}

fn alpaca_ok_routes() -> Vec<(String, MockResponse)> {
    vec![
        (
            "/v2/account".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"account_number":"ALPACA-12345","status":"ACTIVE","currency":"USD","trading_blocked":false,"account_blocked":false}"#.to_owned(),
            },
        ),
        (
            "/v2/clock".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"timestamp":"{}","is_open":true}}"#, iso(NOW_MS + 5_000)),
            },
        ),
    ]
}

fn okx_ok_routes() -> Vec<(String, MockResponse)> {
    vec![
        (
            "/api/v5/public/time".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#, NOW_MS + 5_000),
            },
        ),
        (
            "/api/v5/account/config".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uid":"OKX-UID-999","permTrade":"1","permWit":"0"}]}"#.to_owned(),
            },
        ),
        (
            "/api/v5/account/balance".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"details":[{"ccy":"USDT","eq":"1000.5"}]}]}"#.to_owned(),
            },
        ),
    ]
}

/// Routes for both providers combined, so one harness can serve Alpaca and
/// OKX saves and re-tests in a single test.
fn combined_ok_routes() -> Vec<(String, MockResponse)> {
    alpaca_ok_routes()
        .into_iter()
        .chain(okx_ok_routes())
        .collect()
}

struct Harness {
    manager: ConnectionManager,
    secrets: Arc<InMemorySecretStore>,
    http: Arc<MockHttp>,
    guard: Arc<TestRuntimeGuard>,
}

#[derive(Default)]
struct TestRuntimeGuard(Mutex<usize>);

impl RuntimeGuard for TestRuntimeGuard {
    fn active_dependent_count(&self, _user_id: &str, _provider: Provider) -> usize {
        *self.0.lock().expect("guard poisoned")
    }
}

fn harness(routes: Vec<(String, MockResponse)>) -> Harness {
    let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let secrets = Arc::new(InMemorySecretStore::default());
    let http = MockHttp::new(routes);
    let guard = Arc::new(TestRuntimeGuard::default());
    let manager =
        ConnectionManager::open(database, secrets.clone(), http.clone(), guard.clone()).unwrap();
    Harness {
        manager,
        secrets,
        http,
        guard,
    }
}

fn alpaca_credentials() -> ProviderCredentials {
    ProviderCredentials::AlpacaPaper {
        key_id: ALPACA_KEY_ID.to_owned(),
        secret_key: ALPACA_SECRET.to_owned(),
    }
}

fn okx_credentials() -> ProviderCredentials {
    ProviderCredentials::OkxDemo {
        api_key: OKX_API_KEY.to_owned(),
        secret_key: OKX_SECRET.to_owned(),
        passphrase: OKX_PASSPHRASE.to_owned(),
    }
}

fn error_code<T>(result: Result<T, ConnectionError>) -> String {
    result.err().expect("expected an error").code
}

#[test]
fn save_creates_usable_profile_with_redacted_view_and_stored_secret() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();

    assert_eq!(profile.provider, Provider::AlpacaPaper);
    assert_eq!(profile.environment, "alpaca_paper");
    assert_eq!(profile.status, ProfileStatus::Usable);
    assert_eq!(profile.masked_key_suffix, "WXYZ");
    assert_eq!(profile.account_id.as_deref(), Some("ALPACA-12345"));
    assert_eq!(profile.currency.as_deref(), Some("USD"));
    let evidence = profile.last_test_evidence.unwrap();
    assert_eq!(evidence.outcome, super::tester::TestOutcome::Success);
    assert!(evidence.capabilities.contains(&"trade".to_owned()));

    let listed = harness.manager.list("user-a").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].profile_id, profile.profile_id);

    // The OS store holds exactly one entry, keyed by user + random
    // reference, and its value is the full credential JSON.
    let entries = harness.secrets.entries();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].0.starts_with("user-a:"));
    assert!(entries[0].1.contains(ALPACA_SECRET));
    assert!(entries[0].1.contains(ALPACA_KEY_ID));
}

#[test]
fn serialization_exclusion_keeps_secrets_out_of_sqlite() {
    let harness = harness(alpaca_ok_routes());
    let _ = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();

    let database = harness.manager.database();
    let database = database.lock().unwrap();
    let columns: Vec<String> = database
        .prepare("PRAGMA table_info(connection_profiles)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // No column may carry a credential value. The opaque secret_reference
    // and masked_key_suffix columns are explicitly allowed by ADR 0055.
    for forbidden in [
        "secret_key",
        "passphrase",
        "api_key",
        "key_id",
        "credential",
    ] {
        assert!(
            !columns.iter().any(|column| column.contains(forbidden)),
            "column {forbidden} must not exist, got {columns:?}"
        );
    }

    let row_text: String = database
        .query_row(
            "SELECT profile_id || environment || secret_reference || masked_key_suffix ||
                COALESCE(account_id, '') || COALESCE(currency, '') || status ||
                COALESCE(last_test_evidence_json, '')
             FROM connection_profiles",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!row_text.contains(ALPACA_SECRET));
    assert!(!row_text.contains(ALPACA_KEY_ID));
    assert!(!row_text.contains(OKX_PASSPHRASE));
}

#[test]
fn another_user_cannot_resolve_or_use_the_profile() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();

    assert!(harness.manager.list("user-b").unwrap().is_empty());
    assert_eq!(
        error_code(harness.manager.test("user-b", &profile.profile_id, NOW_MS)),
        "invalid_profile"
    );
    assert_eq!(
        error_code(harness.manager.delete("user-b", &profile.profile_id)),
        "invalid_profile"
    );

    // The same provider under a second User creates a separate Profile
    // rather than sharing or replacing the first one.
    let _ = harness
        .manager
        .save("user-b", alpaca_credentials(), NOW_MS)
        .unwrap();
    assert_eq!(harness.manager.list("user-a").unwrap().len(), 1);
    assert_eq!(harness.manager.list("user-b").unwrap().len(), 1);
    assert_eq!(harness.secrets.entries().len(), 2);
}

#[test]
fn failed_save_leaves_no_profile_and_removes_the_new_secret() {
    let harness = harness(alpaca_ok_routes());
    harness.http.set_routes(vec![(
        "/v2/account".to_owned(),
        MockResponse::Ok {
            status: 401,
            body: "unauthorized".to_owned(),
        },
    )]);
    assert_eq!(
        error_code(harness.manager.save("user-a", alpaca_credentials(), NOW_MS)),
        "auth_failed"
    );
    assert!(harness.manager.list("user-a").unwrap().is_empty());
    assert!(harness.secrets.entries().is_empty());
}

#[test]
fn rotation_keeps_the_prior_profile_on_failed_replacement() {
    let harness = harness(alpaca_ok_routes());
    let first = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    let first_reference = harness.secrets.entries()[0].0.clone();

    harness.http.set_routes(vec![(
        "/v2/account".to_owned(),
        MockResponse::Ok {
            status: 403,
            body: "forbidden".to_owned(),
        },
    )]);
    assert_eq!(
        error_code(harness.manager.save("user-a", alpaca_credentials(), NOW_MS)),
        "auth_failed"
    );

    let profile = &harness.manager.list("user-a").unwrap()[0];
    assert_eq!(profile.profile_id, first.profile_id);
    assert_eq!(profile.status, ProfileStatus::Usable);
    assert_eq!(profile.masked_key_suffix, "WXYZ");
    assert_eq!(harness.secrets.entries().len(), 1);
    assert_eq!(harness.secrets.entries()[0].0, first_reference);
}

#[test]
fn rotation_atomically_switches_and_retires_the_previous_secret() {
    let harness = harness(alpaca_ok_routes());
    let first = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    assert_eq!(harness.secrets.entries().len(), 1);

    let rotated = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    assert_eq!(rotated.profile_id, first.profile_id);
    // The old secret entry was retired; only the new one remains.
    assert_eq!(harness.secrets.entries().len(), 1);

    // The stored credential still resolves for a later re-test.
    let _ = harness
        .manager
        .test("user-a", &rotated.profile_id, NOW_MS)
        .unwrap();
}

#[test]
fn retest_uses_the_stored_credential_and_records_evidence() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();

    harness.http.set_routes(vec![
        (
            "/v2/account".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"account_number":"ALPACA-12345","status":"ACTIVE","currency":"USD","trading_blocked":true,"account_blocked":false}"#
                    .to_owned(),
            },
        ),
        (
            "/v2/clock".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"timestamp":"{}","is_open":true}}"#, iso(NOW_MS + 5_000)),
            },
        ),
    ]);
    let tested = harness
        .manager
        .test("user-a", &profile.profile_id, NOW_MS)
        .unwrap();
    assert_eq!(tested.status, ProfileStatus::Usable);
    let evidence = tested.last_test_evidence.unwrap();
    assert_eq!(evidence.account_id.as_deref(), Some("ALPACA-12345"));
    // trading_blocked removes the trade capability from the evidence.
    assert_eq!(evidence.capabilities, ["read"]);
}

#[test]
fn missing_reference_marks_the_profile_unusable() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    harness.secrets.clear();

    assert_eq!(
        error_code(harness.manager.test("user-a", &profile.profile_id, NOW_MS)),
        "missing_reference"
    );
    let profile = &harness.manager.list("user-a").unwrap()[0];
    assert_eq!(profile.status, ProfileStatus::Unusable);
    assert_eq!(
        profile
            .last_test_evidence
            .as_ref()
            .unwrap()
            .error_code
            .as_deref(),
        Some("missing_reference")
    );
}

#[test]
fn deletion_is_blocked_while_a_dependent_runtime_is_active() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    *harness.guard.0.lock().unwrap() = 1;

    assert_eq!(
        error_code(harness.manager.delete("user-a", &profile.profile_id)),
        "blocked_active_runtime"
    );
    assert_eq!(harness.manager.list("user-a").unwrap().len(), 1);
    assert_eq!(harness.secrets.entries().len(), 1);
}

#[test]
fn deletion_removes_secret_and_invalidates_the_profile() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    harness
        .manager
        .delete("user-a", &profile.profile_id)
        .unwrap();

    assert!(harness.manager.list("user-a").unwrap().is_empty());
    assert!(harness.secrets.entries().is_empty());
    assert_eq!(
        error_code(harness.manager.test("user-a", &profile.profile_id, NOW_MS)),
        "invalid_profile"
    );
}

#[test]
fn okx_save_checks_permissions_and_currency() {
    let harness = harness(okx_ok_routes());
    let profile = harness
        .manager
        .save("user-a", okx_credentials(), NOW_MS)
        .unwrap();
    assert_eq!(profile.environment, "okx_demo");
    assert_eq!(profile.masked_key_suffix, "1234");
    assert_eq!(profile.account_id.as_deref(), Some("OKX-UID-999"));
    assert_eq!(profile.currency.as_deref(), Some("USDT"));
    let evidence = profile.last_test_evidence.unwrap();
    assert!(evidence.capabilities.contains(&"no_withdraw".to_owned()));
    assert!(evidence.capabilities.contains(&"simulated".to_owned()));
}

#[test]
fn okx_withdrawal_capability_is_rejected() {
    let harness = harness(okx_ok_routes());
    harness.http.set_routes(vec![
        (
            "/api/v5/public/time".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#, NOW_MS + 5_000),
            },
        ),
        (
            "/api/v5/account/config".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uid":"OKX-UID-999","permTrade":"1","permWit":"1"}]}"#.to_owned(),
            },
        ),
        (
            "/api/v5/account/balance".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"details":[{"ccy":"USDT"}]}]}"#.to_owned(),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.save("user-a", okx_credentials(), NOW_MS)),
        "withdrawal_capability"
    );
}

#[test]
fn okx_missing_trade_permission_fails_closed() {
    let harness = harness(okx_ok_routes());
    harness.http.set_routes(vec![
        (
            "/api/v5/public/time".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#, NOW_MS + 5_000),
            },
        ),
        (
            "/api/v5/account/config".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uid":"OKX-UID-999","permTrade":"0","permWit":"0"}]}"#.to_owned(),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.save("user-a", okx_credentials(), NOW_MS)),
        "missing_permission"
    );
}

#[test]
fn okx_real_environment_key_fails_closed() {
    let harness = harness(okx_ok_routes());
    harness.http.set_routes(vec![
        (
            "/api/v5/public/time".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(
                    r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#,
                    NOW_MS + 5_000
                ),
            },
        ),
        (
            "/api/v5/account/config".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"50113","msg":"demo trading is not supported","data":[]}"#
                    .to_owned(),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.save("user-a", okx_credentials(), NOW_MS)),
        "environment_mismatch"
    );
}

#[test]
fn okx_currency_mismatch_fails_closed() {
    let harness = harness(okx_ok_routes());
    harness.http.set_routes(vec![
        (
            "/api/v5/public/time".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"code":"0","msg":"","data":[{{"ts":"{}"}}]}}"#, NOW_MS + 5_000),
            },
        ),
        (
            "/api/v5/account/config".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uid":"OKX-UID-999","permTrade":"1","permWit":"0"}]}"#.to_owned(),
            },
        ),
        (
            "/api/v5/account/balance".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"details":[{"ccy":"BTC","eq":"1"}]}]}"#.to_owned(),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.save("user-a", okx_credentials(), NOW_MS)),
        "currency_mismatch"
    );
}

#[test]
fn alpaca_clock_skew_fails_closed() {
    let harness = harness(alpaca_ok_routes());
    harness.http.set_routes(vec![
        (
            "/v2/account".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"account_number":"ALPACA-12345","status":"ACTIVE","currency":"USD","trading_blocked":false,"account_blocked":false}"#.to_owned(),
            },
        ),
        (
            "/v2/clock".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"timestamp":"{}","is_open":true}}"#, iso(NOW_MS + 3_600_000)),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.save("user-a", alpaca_credentials(), NOW_MS)),
        "clock_skew"
    );
}

#[test]
fn endpoint_allowlist_is_fixed_and_never_custom() {
    assert_eq!(
        ALPACA_PAPER_TRADING_ENDPOINT,
        "https://paper-api.alpaca.markets"
    );
    assert_eq!(ALPACA_MARKET_DATA_ENDPOINT, "https://data.alpaca.markets");
    assert_eq!(OKX_DEMO_ENDPOINT, "https://www.okx.com");
    assert_eq!(Provider::AlpacaPaper.environment(), "alpaca_paper");
    assert_eq!(Provider::OkxDemo.environment(), "okx_demo");
    // There is no API surface that accepts a custom or Live endpoint; the
    // environment is fixed per provider by construction.
    assert_eq!(
        Provider::AlpacaPaper.trading_endpoint(),
        ALPACA_PAPER_TRADING_ENDPOINT
    );
    assert_eq!(Provider::OkxDemo.trading_endpoint(), OKX_DEMO_ENDPOINT);
}

#[test]
fn connection_test_never_requests_an_order_endpoint() {
    let harness = harness(combined_ok_routes());
    let alpaca = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();
    let _ = harness
        .manager
        .test("user-a", &alpaca.profile_id, NOW_MS)
        .unwrap();
    let okx = harness
        .manager
        .save("user-a", okx_credentials(), NOW_MS)
        .unwrap();
    let _ = harness
        .manager
        .test("user-a", &okx.profile_id, NOW_MS)
        .unwrap();

    let paths = harness.http.requested_paths();
    for path in &paths {
        assert!(
            !path.contains("/orders") && !path.contains("/trade/") && !path.contains("order"),
            "connection test must never call an order endpoint, hit {path}"
        );
    }
    assert_eq!(
        paths,
        [
            "https://paper-api.alpaca.markets/v2/account",
            "https://paper-api.alpaca.markets/v2/clock",
            "https://paper-api.alpaca.markets/v2/account",
            "https://paper-api.alpaca.markets/v2/clock",
            "https://www.okx.com/api/v5/public/time",
            "https://www.okx.com/api/v5/account/config",
            "https://www.okx.com/api/v5/account/balance",
            "https://www.okx.com/api/v5/public/time",
            "https://www.okx.com/api/v5/account/config",
            "https://www.okx.com/api/v5/account/balance",
        ]
    );
}

#[test]
fn retest_with_changed_account_identity_fails_closed() {
    let harness = harness(alpaca_ok_routes());
    let profile = harness
        .manager
        .save("user-a", alpaca_credentials(), NOW_MS)
        .unwrap();

    harness.http.set_routes(vec![
        (
            "/v2/account".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: r#"{"account_number":"DIFFERENT-ACCOUNT","status":"ACTIVE","currency":"USD","trading_blocked":false,"account_blocked":false}"#
                    .to_owned(),
            },
        ),
        (
            "/v2/clock".to_owned(),
            MockResponse::Ok {
                status: 200,
                body: format!(r#"{{"timestamp":"{}","is_open":true}}"#, iso(NOW_MS + 5_000)),
            },
        ),
    ]);
    assert_eq!(
        error_code(harness.manager.test("user-a", &profile.profile_id, NOW_MS)),
        "account_mismatch"
    );
    let profile = &harness.manager.list("user-a").unwrap()[0];
    assert_eq!(profile.status, ProfileStatus::Unusable);
    assert_eq!(
        profile
            .last_test_evidence
            .as_ref()
            .unwrap()
            .error_code
            .as_deref(),
        Some("account_mismatch")
    );
}

#[test]
fn redaction_removes_every_sensitive_value() {
    let redacted = redact(
        "key AK1234567890WXYZ with secret alpaca-secret-value and passphrase okx-passphrase-value",
        &[ALPACA_KEY_ID, ALPACA_SECRET, OKX_PASSPHRASE],
    );
    assert!(!redacted.contains(ALPACA_KEY_ID));
    assert!(!redacted.contains(ALPACA_SECRET));
    assert!(!redacted.contains(OKX_PASSPHRASE));
    assert!(redacted.contains("[redacted]"));
}

#[test]
fn invalid_inputs_are_rejected() {
    let harness = harness(alpaca_ok_routes());
    assert_eq!(
        error_code(harness.manager.save(
            "user-a",
            ProviderCredentials::AlpacaPaper {
                key_id: String::new(),
                secret_key: ALPACA_SECRET.to_owned(),
            },
            NOW_MS
        )),
        "invalid_input"
    );
    assert_eq!(
        error_code(harness.manager.save("", alpaca_credentials(), NOW_MS)),
        "invalid_input"
    );
}
