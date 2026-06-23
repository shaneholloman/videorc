use crate::protocol::{AccountStatus, VideorcAccountSnapshot};

// The desktop's Videorc PRODUCT account. Real web auth + token storage are not
// wired yet, so with no stored session the backend reports signed-out. The
// renderer reads this via the `account.get` command (mirrors entitlements.get);
// token storage will populate the signed-in fields here in a later slice.
pub fn current_account() -> VideorcAccountSnapshot {
    signed_out_account()
}

pub fn signed_out_account() -> VideorcAccountSnapshot {
    VideorcAccountSnapshot {
        status: AccountStatus::SignedOut,
        username: None,
        display_name: None,
        email: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_account_is_signed_out_without_a_stored_session() {
        let account = current_account();
        assert_eq!(account.status, AccountStatus::SignedOut);
        assert!(account.username.is_none());
        assert!(account.email.is_none());
    }

    #[test]
    fn signed_out_account_omits_optional_fields_and_round_trips() {
        let json = serde_json::to_string(&signed_out_account()).unwrap();
        assert_eq!(json, r#"{"status":"signed-out"}"#);
        let restored: VideorcAccountSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, signed_out_account());
    }

    #[test]
    fn signed_in_account_serializes_identity_fields_in_camel_case() {
        let account = VideorcAccountSnapshot {
            status: AccountStatus::SignedIn,
            username: Some("orc_dev".to_string()),
            display_name: Some("Orc Dev".to_string()),
            email: Some("orc@videorc.com".to_string()),
        };
        let json = serde_json::to_string(&account).unwrap();
        assert!(json.contains("\"status\":\"signed-in\""));
        assert!(json.contains("\"displayName\":\"Orc Dev\""));
        let restored: VideorcAccountSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, account);
    }
}
