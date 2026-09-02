//! Read-only Paper Connection Tests.
//!
//! A test retrieves the credential inside the Host, authenticates against
//! the fixed provider environment, checks account identity/status, native
//! currency, permissions, provider time/clock skew, and capabilities, and
//! records typed redacted evidence. It never submits, amends, cancels, or
//! queries a synthetic test order: the only endpoints reached are the
//! read-only account/clock endpoints below, and the requested paths are
//! themselves part of the recorded evidence.

use std::sync::Arc;

use base64::Engine;
use chrono::DateTime;
use hmac::{Hmac, KeyInit, Mac};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::{Provider, redact};

pub(crate) const ALPACA_PAPER_TRADING_ENDPOINT: &str = "https://paper-api.alpaca.markets";
pub(crate) const ALPACA_MARKET_DATA_ENDPOINT: &str = "https://data.alpaca.markets";
pub(crate) const OKX_DEMO_ENDPOINT: &str = "https://www.okx.com";

const ALPACA_ACCOUNT_PATH: &str = "/v2/account";
const ALPACA_CLOCK_PATH: &str = "/v2/clock";
const OKX_PUBLIC_TIME_PATH: &str = "/api/v5/public/time";
const OKX_ACCOUNT_CONFIG_PATH: &str = "/api/v5/account/config";
const OKX_ACCOUNT_BALANCE_PATH: &str = "/api/v5/account/balance";
const OKX_OPEN_ORDERS_PATH: &str = "/api/v5/trade/orders-pending?instType=SPOT";

/// Maximum tolerated absolute difference between provider time and the
/// local clock. OKX already rejects signed requests more than 30 seconds
/// off, so this bound is looser than the provider's own check.
const MAX_CLOCK_SKEW_SECONDS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionEvidence {
    pub outcome: TestOutcome,
    pub error_code: Option<String>,
    pub redacted_error: Option<String>,
    pub account_id: Option<String>,
    pub currency: Option<String>,
    pub account_status: Option<String>,
    pub server_time_ms: Option<i64>,
    pub clock_skew_seconds: Option<i64>,
    pub capabilities: Vec<String>,
    pub requested_paths: Vec<String>,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TestOutcome {
    Success,
    Failure,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TestFailure {
    pub code: &'static str,
    pub redacted_message: String,
}

impl TestFailure {
    pub(crate) fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            redacted_message: redact(&message, &[]),
        }
    }

    pub(crate) fn evidence(&self, checked_at_ms: i64) -> ConnectionEvidence {
        ConnectionEvidence {
            outcome: TestOutcome::Failure,
            error_code: Some(self.code.to_owned()),
            redacted_error: Some(self.redacted_message.clone()),
            account_id: None,
            currency: None,
            account_status: None,
            server_time_ms: None,
            clock_skew_seconds: None,
            capabilities: Vec::new(),
            requested_paths: Vec::new(),
            checked_at_ms,
        }
    }
}

/// The credential values the Host holds only for the duration of one test.
/// Serialized as a single JSON value inside the OS store so every field the
/// provider requires survives a later re-test; the JSON never leaves the
/// Host or enters SQLite.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub(crate) enum TestCredential {
    AlpacaPaper {
        key_id: String,
        secret_key: String,
    },
    OkxDemo {
        api_key: String,
        secret_key: String,
        passphrase: String,
    },
}

impl TestCredential {
    /// The last four characters of the public key identifier, used as the
    /// masked display suffix. Never includes the secret value.
    pub fn masked_key_suffix(&self) -> String {
        let public_key = match self {
            Self::AlpacaPaper { key_id, .. } => key_id,
            Self::OkxDemo { api_key, .. } => api_key,
        };
        public_key
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub(crate) trait HttpExecutor: Send + Sync {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, String>;
}

/// Production executor. Never logs requests; the request path is recorded
/// in evidence by the tester, never the headers or bodies.
pub(crate) struct ReqwestExecutor {
    client: reqwest::blocking::Client,
}

impl ReqwestExecutor {
    pub(crate) fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest blocking client builds");
        Self { client }
    }
}

impl Default for ReqwestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpExecutor for ReqwestExecutor {
    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = self.client.request(
            reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|error| error.to_string())?,
            &request.url,
        );
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let response = builder.send().map_err(|error| error.to_string())?;
        Ok(HttpResponse {
            status: response.status().as_u16(),
            body: response.text().map_err(|error| error.to_string())?,
        })
    }
}

pub(crate) struct ConnectionTester {
    http: Arc<dyn HttpExecutor>,
}

impl ConnectionTester {
    pub(crate) fn new(http: Arc<dyn HttpExecutor>) -> Self {
        Self { http }
    }

    /// Runs the read-only connection test for the credential. `now_ms` is
    /// injected so clock-skew behavior is testable.
    pub(crate) fn test(
        &self,
        credential: &TestCredential,
        now_ms: i64,
    ) -> Result<ConnectionEvidence, TestFailure> {
        match credential {
            TestCredential::AlpacaPaper { key_id, secret_key } => {
                self.test_alpaca_paper(key_id, secret_key, now_ms)
            }
            TestCredential::OkxDemo {
                api_key,
                secret_key,
                passphrase,
            } => self.test_okx_demo(api_key, secret_key, passphrase, now_ms),
        }
    }

    /// Reads the OKX Demo pending-order list for reconciliation on the Host
    /// HTTP boundary, keeping the credential out of Workers and Components.
    pub(crate) fn fetch_okx_demo_open_orders(
        &self,
        credential: &TestCredential,
        now_ms: i64,
    ) -> Result<Vec<serde_json::Value>, TestFailure> {
        Ok(self
            .fetch_okx_demo_private(credential, now_ms, OKX_OPEN_ORDERS_PATH)?
            .data)
    }

    pub(crate) fn fetch_okx_demo_balance(
        &self,
        credential: &TestCredential,
        now_ms: i64,
    ) -> Result<adaq_trading_crypto::Balances, TestFailure> {
        let parsed: OkxResponse<OkxBalance> =
            self.fetch_okx_demo_private(credential, now_ms, OKX_ACCOUNT_BALANCE_PATH)?;
        let balance = parsed.first().ok_or_else(|| {
            TestFailure::new(
                "request_failed",
                "OKX returned no account balance.".to_owned(),
            )
        })?;
        let mut balances = adaq_trading_crypto::Balances::default();
        for detail in &balance.details {
            balances.accounts.insert(
                detail.ccy.clone(),
                adaq_trading_crypto::Balance {
                    free: parse_okx_decimal(detail.avail_bal.as_deref())?,
                    total: parse_okx_decimal(detail.cash_bal.as_deref())?,
                    ..Default::default()
                },
            );
        }
        Ok(balances)
    }

    pub(crate) fn create_okx_demo_order(
        &self,
        credential: &TestCredential,
        instrument: &str,
        order_type: &str,
        side: &str,
        amount: &str,
        price: Option<&str>,
        now_ms: i64,
    ) -> Result<serde_json::Value, TestFailure> {
        if !matches!(order_type, "limit" | "market") || !matches!(side, "buy" | "sell") {
            return Err(TestFailure::new(
                "request_failed",
                "Unsupported OKX Demo order type or side.".to_owned(),
            ));
        }
        let mut body = serde_json::json!({
            "instId": instrument,
            "tdMode": "cash",
            "side": side,
            "ordType": order_type,
            "sz": amount,
        });
        if let Some(price) = price {
            body["px"] = serde_json::Value::String(price.to_owned());
        }
        self.request_okx_demo_private(credential, now_ms, "POST", "/api/v5/trade/order", &body)
    }

    pub(crate) fn cancel_okx_demo_order(
        &self,
        credential: &TestCredential,
        instrument: &str,
        provider_order_id: &str,
        now_ms: i64,
    ) -> Result<serde_json::Value, TestFailure> {
        self.request_okx_demo_private(
            credential,
            now_ms,
            "POST",
            "/api/v5/trade/cancel-order",
            &serde_json::json!({"instId": instrument, "ordId": provider_order_id}),
        )
    }

    pub(crate) fn fetch_okx_demo_order(
        &self,
        credential: &TestCredential,
        instrument: &str,
        provider_order_id: &str,
        now_ms: i64,
    ) -> Result<serde_json::Value, TestFailure> {
        let path = format!("/api/v5/trade/order?instId={instrument}&ordId={provider_order_id}");
        self.request_okx_demo_private(credential, now_ms, "GET", &path, &serde_json::Value::Null)
    }

    fn fetch_okx_demo_private<T: serde::de::DeserializeOwned>(
        &self,
        credential: &TestCredential,
        now_ms: i64,
        path: &str,
    ) -> Result<OkxResponse<T>, TestFailure> {
        self.request_okx_demo_private_response(
            credential,
            now_ms,
            "GET",
            path,
            &serde_json::Value::Null,
        )
    }

    fn request_okx_demo_private(
        &self,
        credential: &TestCredential,
        now_ms: i64,
        method: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, TestFailure> {
        let parsed: OkxResponse<serde_json::Value> =
            self.request_okx_demo_private_response(credential, now_ms, method, path, body)?;
        parsed.first().cloned().ok_or_else(|| {
            TestFailure::new(
                "request_failed",
                "OKX returned no order evidence.".to_owned(),
            )
        })
    }

    fn request_okx_demo_private_response<T: serde::de::DeserializeOwned>(
        &self,
        credential: &TestCredential,
        now_ms: i64,
        method: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<OkxResponse<T>, TestFailure> {
        let TestCredential::OkxDemo {
            api_key,
            secret_key,
            passphrase,
        } = credential
        else {
            return Err(TestFailure::new(
                "environment_mismatch",
                "The saved credential does not match the OKX Demo provider.".to_owned(),
            ));
        };
        let sensitive = [api_key.as_str(), secret_key.as_str(), passphrase.as_str()];
        let time_response = self
            .get(format!("{OKX_DEMO_ENDPOINT}{OKX_PUBLIC_TIME_PATH}"), vec![])
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        let timestamp_ms = if time_response.status == 200 {
            parse_okx_time(&time_response.body).map_err(|message| {
                TestFailure::new("request_failed", redact(&message, &sensitive))
            })?
        } else {
            now_ms
        };
        let timestamp = format_rfc3339_ms(timestamp_ms);
        let body = if method == "GET" {
            String::new()
        } else {
            serde_json::to_string(body).map_err(|error| {
                TestFailure::new("request_failed", redact(&error.to_string(), &sensitive))
            })?
        };
        let response = self
            .http
            .execute(&HttpRequest {
                method: method.to_owned(),
                url: format!("{OKX_DEMO_ENDPOINT}{path}"),
                headers: okx_headers(
                    api_key, secret_key, passphrase, &timestamp, method, path, &body,
                ),
                body,
            })
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        let parsed = parse_okx_response(&response.body, &sensitive)?;
        if let Some(error) = okx_error(&response, &parsed, &sensitive) {
            return Err(error);
        }
        Ok(parsed)
    }

    fn test_alpaca_paper(
        &self,
        key_id: &str,
        secret_key: &str,
        now_ms: i64,
    ) -> Result<ConnectionEvidence, TestFailure> {
        let sensitive = [key_id, secret_key];
        let mut paths = Vec::new();
        let endpoint = Provider::AlpacaPaper.trading_endpoint();
        self.ensure_allowlisted(endpoint, Provider::AlpacaPaper)?;

        let basic =
            base64::engine::general_purpose::STANDARD.encode(format!("{key_id}:{secret_key}"));
        let account_response = self
            .get(
                format!("{endpoint}{ALPACA_ACCOUNT_PATH}"),
                vec![("Authorization".to_owned(), format!("Basic {basic}"))],
            )
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        paths.push(ALPACA_ACCOUNT_PATH.to_owned());

        if account_response.status == 401 || account_response.status == 403 {
            return Err(TestFailure::new(
                "auth_failed",
                redact(
                    &format!(
                        "Alpaca rejected the Paper credentials (HTTP {}).",
                        account_response.status
                    ),
                    &sensitive,
                ),
            ));
        }
        if account_response.status != 200 {
            return Err(TestFailure::new(
                "request_failed",
                redact(
                    &format!(
                        "Alpaca account request failed with HTTP {}.",
                        account_response.status
                    ),
                    &sensitive,
                ),
            ));
        }

        let account: AlpacaAccount =
            serde_json::from_str(&account_response.body).map_err(|error| {
                TestFailure::new("request_failed", redact(&error.to_string(), &sensitive))
            })?;

        if account.status != "ACTIVE" {
            return Err(TestFailure::new(
                "inactive_account",
                redact(
                    &format!(
                        "Alpaca account is not ACTIVE (reported status {:?}).",
                        account.status
                    ),
                    &sensitive,
                ),
            ));
        }
        if account.currency != "USD" {
            return Err(TestFailure::new(
                "currency_mismatch",
                redact(
                    &format!(
                        "Alpaca account currency is {:?}, expected USD.",
                        account.currency
                    ),
                    &sensitive,
                ),
            ));
        }

        let clock_response = self
            .get(
                format!("{endpoint}{ALPACA_CLOCK_PATH}"),
                vec![("Authorization".to_owned(), format!("Basic {basic}"))],
            )
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        paths.push(ALPACA_CLOCK_PATH.to_owned());
        if clock_response.status != 200 {
            return Err(TestFailure::new(
                "request_failed",
                redact(
                    &format!(
                        "Alpaca clock request failed with HTTP {}.",
                        clock_response.status
                    ),
                    &sensitive,
                ),
            ));
        }
        let clock: AlpacaClock = serde_json::from_str(&clock_response.body).map_err(|error| {
            TestFailure::new("request_failed", redact(&error.to_string(), &sensitive))
        })?;
        let server_time_ms = parse_rfc3339_ms(&clock.timestamp)?;
        let skew_seconds = (server_time_ms - now_ms) / 1000;
        if skew_seconds.abs() > MAX_CLOCK_SKEW_SECONDS {
            return Err(TestFailure::new(
                "clock_skew",
                redact(
                    &format!("Local clock is {skew_seconds}s away from Alpaca server time."),
                    &sensitive,
                ),
            ));
        }

        let mut capabilities = vec!["read".to_owned()];
        if !account.trading_blocked && !account.account_blocked {
            capabilities.push("trade".to_owned());
        }

        Ok(ConnectionEvidence {
            outcome: TestOutcome::Success,
            error_code: None,
            redacted_error: None,
            account_id: Some(account.account_number),
            currency: Some(account.currency),
            account_status: Some(account.status),
            server_time_ms: Some(server_time_ms),
            clock_skew_seconds: Some(skew_seconds),
            capabilities,
            requested_paths: paths,
            checked_at_ms: now_ms,
        })
    }

    fn test_okx_demo(
        &self,
        api_key: &str,
        secret_key: &str,
        passphrase: &str,
        now_ms: i64,
    ) -> Result<ConnectionEvidence, TestFailure> {
        let sensitive = [api_key, secret_key, passphrase];
        let mut paths = Vec::new();
        let endpoint = Provider::OkxDemo.trading_endpoint();
        self.ensure_allowlisted(endpoint, Provider::OkxDemo)?;

        let time_response = self
            .get(format!("{endpoint}{OKX_PUBLIC_TIME_PATH}"), vec![])
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        paths.push(OKX_PUBLIC_TIME_PATH.to_owned());
        let server_time_ms = if time_response.status == 200 {
            Some(parse_okx_time(&time_response.body).map_err(|message| {
                TestFailure::new("request_failed", redact(&message, &sensitive))
            })?)
        } else {
            // Provider time is evidence, not a requirement: the signed
            // requests below already fail closed on real skew through OKX's
            // 30-second timestamp window. Record no clock evidence rather
            // than fabricating zero skew.
            None
        };
        let skew_seconds = server_time_ms.map(|server| (server - now_ms) / 1000);
        if let Some(skew) = skew_seconds {
            if skew.abs() > MAX_CLOCK_SKEW_SECONDS {
                return Err(TestFailure::new(
                    "clock_skew",
                    redact(
                        &format!("Local clock is {skew}s away from OKX server time."),
                        &sensitive,
                    ),
                ));
            }
        }
        // Sign with the provider's own time when available so the request is
        // never rejected by OKX's stricter 30-second timestamp window.
        let timestamp = format_rfc3339_ms(server_time_ms.unwrap_or(now_ms));

        let config_response = self
            .get(
                format!("{endpoint}{OKX_ACCOUNT_CONFIG_PATH}"),
                okx_headers(
                    api_key,
                    secret_key,
                    passphrase,
                    &timestamp,
                    "GET",
                    OKX_ACCOUNT_CONFIG_PATH,
                    "",
                ),
            )
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        paths.push(OKX_ACCOUNT_CONFIG_PATH.to_owned());
        let config: OkxResponse<OkxAccountConfig> =
            parse_okx_response(&config_response.body, &sensitive)?;
        if let Some(error) = okx_error(&config_response, &config, &sensitive) {
            return Err(error);
        }
        let config = config.first().ok_or_else(|| {
            TestFailure::new(
                "request_failed",
                "OKX returned no account configuration.".to_owned(),
            )
        })?;

        // An account that cannot trade fails closed; a key that can withdraw
        // is rejected because V1 only accepts the least-privilege boundary.
        if config.perm_wit.as_deref() == Some("1") {
            return Err(TestFailure::new(
                "withdrawal_capability",
                "OKX key reports withdrawal capability; V1 requires Read/Trade only.".to_owned(),
            ));
        }
        if config.perm_trade.as_deref() == Some("0") {
            return Err(TestFailure::new(
                "missing_permission",
                "OKX key lacks trade permission.".to_owned(),
            ));
        }

        let balance_response = self
            .get(
                format!("{endpoint}{OKX_ACCOUNT_BALANCE_PATH}"),
                okx_headers(
                    api_key,
                    secret_key,
                    passphrase,
                    &timestamp,
                    "GET",
                    OKX_ACCOUNT_BALANCE_PATH,
                    "",
                ),
            )
            .map_err(|message| TestFailure::new("request_failed", redact(&message, &sensitive)))?;
        paths.push(OKX_ACCOUNT_BALANCE_PATH.to_owned());
        let balance: OkxResponse<OkxBalance> =
            parse_okx_response(&balance_response.body, &sensitive)?;
        if let Some(error) = okx_error(&balance_response, &balance, &sensitive) {
            return Err(error);
        }
        let balance = balance.first().ok_or_else(|| {
            TestFailure::new(
                "request_failed",
                "OKX returned no account balance.".to_owned(),
            )
        })?;

        let currencies: Vec<&str> = balance
            .details
            .iter()
            .map(|detail| detail.ccy.as_str())
            .collect();
        // A reported currency that is not USDT fails closed; a zero-balance
        // demo account reports no currency at all and is recorded as such.
        if !currencies.is_empty() && !currencies.contains(&"USDT") {
            return Err(TestFailure::new(
                "currency_mismatch",
                format!("OKX account reports currencies {currencies:?}, expected USDT."),
            ));
        }

        let mut capabilities = vec![
            "read".to_owned(),
            "trade".to_owned(),
            "simulated".to_owned(),
        ];
        if config.perm_wit.as_deref() == Some("0") {
            capabilities.push("no_withdraw".to_owned());
        }
        let currency = if currencies.contains(&"USDT") {
            Some("USDT".to_owned())
        } else {
            None
        };

        Ok(ConnectionEvidence {
            outcome: TestOutcome::Success,
            error_code: None,
            redacted_error: None,
            account_id: Some(config.uid.clone()),
            currency,
            account_status: Some("demo".to_owned()),
            server_time_ms,
            clock_skew_seconds: skew_seconds,
            capabilities,
            requested_paths: paths,
            checked_at_ms: now_ms,
        })
    }

    /// Fail-closed guard: a request may only leave the Host for one of the
    /// fixed Paper/Demo endpoints of the provider. There is no configuration
    /// path for Live or arbitrary custom endpoints, so a mismatch here can
    /// only mean a programming error; the test still refuses to proceed.
    fn ensure_allowlisted(&self, url: &str, provider: Provider) -> Result<(), TestFailure> {
        if provider
            .fixed_endpoints()
            .iter()
            .any(|endpoint| url.starts_with(endpoint))
        {
            Ok(())
        } else {
            Err(TestFailure::new(
                "environment_mismatch",
                format!("refusing to contact endpoint {url:?} outside the fixed environment"),
            ))
        }
    }

    fn get(&self, url: String, headers: Vec<(String, String)>) -> Result<HttpResponse, String> {
        self.http.execute(&HttpRequest {
            method: "GET".to_owned(),
            url,
            headers,
            body: String::new(),
        })
    }
}

/// Every private OKX request carries the signature headers and the
/// simulated-trading header; the demo environment is therefore enforced per
/// request, never configured once.
fn okx_headers(
    api_key: &str,
    secret_key: &str,
    passphrase: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Vec<(String, String)> {
    let signature = okx_signature(secret_key, timestamp, method, path, body);
    vec![
        ("OK-ACCESS-KEY".to_owned(), api_key.to_owned()),
        ("OK-ACCESS-SIGN".to_owned(), signature),
        ("OK-ACCESS-TIMESTAMP".to_owned(), timestamp.to_owned()),
        ("OK-ACCESS-PASSPHRASE".to_owned(), passphrase.to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("x-simulated-trading".to_owned(), "1".to_owned()),
    ]
}

fn okx_signature(secret: &str, timestamp: &str, method: &str, path: &str, body: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}{method}{path}{body}").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn parse_okx_response<T>(body: &str, sensitive: &[&str]) -> Result<OkxResponse<T>, TestFailure>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(body)
        .map_err(|error| TestFailure::new("request_failed", redact(&error.to_string(), sensitive)))
}

fn okx_error<T>(
    response: &HttpResponse,
    parsed: &OkxResponse<T>,
    sensitive: &[&str],
) -> Option<TestFailure> {
    if response.status != 200 {
        return Some(TestFailure::new(
            "request_failed",
            redact(
                &format!("OKX request failed with HTTP {}.", response.status),
                sensitive,
            ),
        ));
    }
    if parsed.code == "0" {
        return None;
    }
    let message = format!("OKX error {}: {}.", parsed.code, parsed.message);
    // Error 50113 identifies a real-environment key used on Demo; the
    // simulated header stays fixed, so this fails closed instead of
    // switching to Live mode.
    let code = if parsed.code == "50113" {
        "environment_mismatch"
    } else if matches!(parsed.code.as_str(), "50102" | "50111" | "50112") {
        "auth_failed"
    } else {
        "request_failed"
    };
    Some(TestFailure::new(code, redact(&message, sensitive)))
}

fn parse_rfc3339_ms(value: &str) -> Result<i64, TestFailure> {
    DateTime::parse_from_rfc3339(value)
        .map(|datetime| datetime.timestamp_millis())
        .map_err(|error| {
            TestFailure::new(
                "request_failed",
                format!("Provider returned an unparsable timestamp: {error}."),
            )
        })
}

fn format_rfc3339_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("provider timestamp in range")
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn parse_okx_time(body: &str) -> Result<i64, String> {
    #[derive(Deserialize)]
    struct OkxTime {
        data: Vec<TimeData>,
    }
    #[derive(Deserialize)]
    struct TimeData {
        ts: String,
    }
    let parsed: OkxTime = serde_json::from_str(body).map_err(|error| error.to_string())?;
    parsed
        .data
        .first()
        .and_then(|data| data.ts.parse::<i64>().ok())
        .ok_or_else(|| "OKX time response contained no timestamp".to_owned())
}

#[derive(Deserialize)]
struct AlpacaAccount {
    account_number: String,
    status: String,
    currency: String,
    #[serde(default)]
    trading_blocked: bool,
    #[serde(default)]
    account_blocked: bool,
}

#[derive(Deserialize)]
struct AlpacaClock {
    timestamp: String,
}

#[derive(Deserialize)]
struct OkxResponse<T> {
    code: String,
    #[serde(rename = "msg")]
    message: String,
    #[serde(default = "empty_vec")]
    data: Vec<T>,
}

fn empty_vec<T>() -> Vec<T> {
    Vec::new()
}

impl<T> OkxResponse<T> {
    fn first(&self) -> Option<&T> {
        self.data.first()
    }
}

#[derive(Deserialize)]
struct OkxAccountConfig {
    uid: String,
    #[serde(rename = "permTrade")]
    perm_trade: Option<String>,
    #[serde(rename = "permWit")]
    perm_wit: Option<String>,
}

#[derive(Deserialize)]
struct OkxBalance {
    details: Vec<OkxBalanceDetail>,
}

#[derive(Deserialize)]
struct OkxBalanceDetail {
    ccy: String,
    #[serde(rename = "cashBal")]
    cash_bal: Option<String>,
    #[serde(rename = "availBal")]
    avail_bal: Option<String>,
}

fn parse_okx_decimal(value: Option<&str>) -> Result<Option<Decimal>, TestFailure> {
    value
        .map(|value| {
            value.parse().map_err(|error| {
                TestFailure::new(
                    "request_failed",
                    format!("OKX returned an invalid balance: {error}."),
                )
            })
        })
        .transpose()
}
