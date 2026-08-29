//! Tiny shared module for User identity validation.
//!
//! Every User-scoped entry point in the Local Research domains validates the
//! User ID through this one helper so the rule lives in exactly one place.

pub(crate) fn validate_user(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 || user_id.contains(':') {
        Err("User ID is invalid".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::validate_user;

    #[test]
    fn rejects_key_delimiter_in_user_ids() {
        assert!(validate_user("alice:other").is_err());
        assert!(validate_user("alice").is_ok());
    }
}
