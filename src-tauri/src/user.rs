//! Tiny shared module for User identity validation.
//!
//! Every User-scoped entry point in the Local Research domains validates the
//! User ID through this one helper so the rule lives in exactly one place.

pub(crate) fn validate_user(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("User ID is invalid".into())
    } else {
        Ok(())
    }
}
