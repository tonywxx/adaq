use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::user::validate_user;

const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;

/// The non-secret identity established after Host-side session verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedSession {
    pub(crate) user_id: String,
}

trait AuthVerifier: Send + Sync {
    fn verify(&self, access_token: &str) -> Result<VerifiedSession, String>;
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Context fields are consumed by the next User-scoped command migration slice"
    )
)]
#[derive(Clone)]
struct AuthenticatedUserContext {
    user_id: String,
    access_token: String,
    verified_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthContextView {
    pub(crate) user_id: String,
    pub(crate) verified_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct AuthState {
    verifier: Arc<dyn AuthVerifier>,
    bindings: Arc<RwLock<HashMap<String, AuthenticatedUserContext>>>,
}

impl AuthState {
    pub(crate) fn from_environment() -> Self {
        Self::new(Arc::new(SupabaseUserVerifier::from_environment()))
    }

    fn new(verifier: Arc<dyn AuthVerifier>) -> Self {
        Self {
            verifier,
            bindings: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub(crate) fn bind(
        &self,
        window_label: &str,
        access_token: &str,
        verified_at_ms: i64,
    ) -> Result<AuthContextView, String> {
        let result = self.bind_verified(window_label, access_token, verified_at_ms);
        if result.is_err() {
            self.clear(window_label);
        }
        result
    }

    pub(crate) fn clear(&self, window_label: &str) {
        if let Ok(mut bindings) = self.bindings.write() {
            bindings.remove(window_label);
        }
    }

    pub(crate) fn user_id_for_window(&self, window_label: &str) -> Result<String, String> {
        let bindings = self
            .bindings
            .read()
            .map_err(|_| "Authentication state is unavailable".to_owned())?;
        bindings
            .get(window_label)
            .map(|context| context.user_id.clone())
            .ok_or_else(|| "Authenticated User Context is unavailable".to_owned())
    }

    fn bind_verified(
        &self,
        window_label: &str,
        access_token: &str,
        verified_at_ms: i64,
    ) -> Result<AuthContextView, String> {
        if access_token.trim().is_empty() {
            return Err("Access token is required".into());
        }
        if access_token.len() > MAX_ACCESS_TOKEN_BYTES {
            return Err("Access token is too large".into());
        }

        let verified = self.verifier.verify(access_token)?;
        validate_user(&verified.user_id)?;
        let view = AuthContextView {
            user_id: verified.user_id.clone(),
            verified_at_ms,
        };
        let context = AuthenticatedUserContext {
            user_id: verified.user_id,
            access_token: access_token.to_owned(),
            verified_at_ms,
        };
        if let Ok(mut bindings) = self.bindings.write() {
            bindings.insert(window_label.to_owned(), context);
            return Ok(view);
        }
        Err("Authentication state is unavailable".into())
    }

    #[cfg(test)]
    fn context_for(&self, window_label: &str) -> Option<(String, String, i64)> {
        self.bindings.read().ok().and_then(|bindings| {
            bindings.get(window_label).map(|context| {
                (
                    context.user_id.clone(),
                    context.access_token.clone(),
                    context.verified_at_ms,
                )
            })
        })
    }
}

#[derive(Clone)]
struct SupabaseUserVerifier {
    project_url: Option<String>,
    anon_key: Option<String>,
}

impl SupabaseUserVerifier {
    fn from_environment() -> Self {
        Self {
            project_url: option_env!("ADAQ_SUPABASE_URL").map(str::to_owned),
            anon_key: option_env!("ADAQ_SUPABASE_ANON_KEY").map(str::to_owned),
        }
    }
}

#[derive(Deserialize)]
struct SupabaseUser {
    id: String,
}

impl AuthVerifier for SupabaseUserVerifier {
    fn verify(&self, access_token: &str) -> Result<VerifiedSession, String> {
        let (Some(project_url), Some(anon_key)) = (&self.project_url, &self.anon_key) else {
            return Err("Host authentication is not configured".into());
        };
        let endpoint = format!("{}/auth/v1/user", project_url.trim_end_matches('/'));
        let response = reqwest::blocking::Client::new()
            .get(endpoint)
            .header("apikey", anon_key)
            .bearer_auth(access_token)
            .send()
            .map_err(|_| "Authentication verification is unavailable".to_owned())?;
        if !response.status().is_success() {
            return Err("Authentication rejected".into());
        }
        let user: SupabaseUser = response
            .json()
            .map_err(|_| "Authentication response is invalid".to_owned())?;
        validate_user(&user.id)?;
        Ok(VerifiedSession { user_id: user.id })
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct StaticVerifier {
        sessions: HashMap<String, String>,
    }

    impl AuthVerifier for StaticVerifier {
        fn verify(&self, access_token: &str) -> Result<VerifiedSession, String> {
            self.sessions
                .get(access_token)
                .cloned()
                .map(|user_id| VerifiedSession { user_id })
                .ok_or_else(|| "Authentication rejected".into())
        }
    }

    fn state() -> AuthState {
        AuthState::new(Arc::new(StaticVerifier {
            sessions: HashMap::from([(String::from("valid"), String::from("alice"))]),
        }))
    }

    #[test]
    fn bind_derives_user_from_verified_session() {
        let state = state();

        let view = state.bind("main", "valid", 42).expect("valid session");

        assert_eq!(view.user_id, "alice");
        assert_eq!(view.verified_at_ms, 42);
        assert_eq!(
            state.context_for("main"),
            Some((String::from("alice"), String::from("valid"), 42))
        );
    }

    #[test]
    fn unbound_windows_are_rejected() {
        assert_eq!(
            state().user_id_for_window("missing").unwrap_err(),
            "Authenticated User Context is unavailable"
        );
    }

    #[test]
    fn failed_rebind_clears_previous_window_context() {
        let state = state();
        state.bind("main", "valid", 42).expect("valid session");

        assert!(state.bind("main", "forged", 43).is_err());
        assert_eq!(state.context_for("main"), None);
    }

    #[test]
    fn clearing_one_window_does_not_clear_another_window() {
        let state = state();
        state.bind("main", "valid", 42).expect("valid session");
        state.bind("secondary", "valid", 43).expect("valid session");

        state.clear("main");

        assert_eq!(state.context_for("main"), None);
        assert_eq!(
            state.context_for("secondary"),
            Some((String::from("alice"), String::from("valid"), 43))
        );
    }

    #[test]
    fn empty_access_token_is_rejected_before_verification() {
        let state = state();

        assert_eq!(
            state.bind("main", " ", 42).unwrap_err(),
            "Access token is required"
        );
        assert_eq!(state.context_for("main"), None);
    }
}
