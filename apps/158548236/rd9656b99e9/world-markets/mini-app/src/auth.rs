use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MAX_AUTH_AGE_SECS: i64 = 3600;

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    MissingHash,
    MissingAuthDate,
    Expired,
    BadUser,
    HashMismatch,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingHash => write!(f, "missing hash"),
            Self::MissingAuthDate => write!(f, "missing auth_date"),
            Self::Expired => write!(f, "auth_date expired"),
            Self::BadUser => write!(f, "missing user id"),
            Self::HashMismatch => write!(f, "hash mismatch"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TelegramUser {
    pub id: u64,
    pub first_name: Option<String>,
}

pub fn verify_init_data(init_data: &str, bot_token: &str) -> Result<TelegramUser, AuthError> {
    verify_init_data_at(init_data, bot_token, now_unix())
}

fn verify_init_data_at(
    init_data: &str,
    bot_token: &str,
    now: i64,
) -> Result<TelegramUser, AuthError> {
    let mut fields = BTreeMap::new();
    for pair in init_data.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = urlencoding::decode(key)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| key.to_string());
        let value = urlencoding::decode(value)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| value.to_string());
        fields.insert(key, value);
    }

    let provided = fields.remove("hash").ok_or(AuthError::MissingHash)?;
    let auth_date: i64 = fields
        .get("auth_date")
        .ok_or(AuthError::MissingAuthDate)?
        .parse()
        .map_err(|_| AuthError::MissingAuthDate)?;
    if (now - auth_date).abs() > MAX_AUTH_AGE_SECS {
        return Err(AuthError::Expired);
    }

    let data_check = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut secret = HmacSha256::new_from_slice(b"WebAppData").expect("HMAC key");
    secret.update(bot_token.as_bytes());
    let secret_key = secret.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&secret_key).expect("HMAC key");
    mac.update(data_check.as_bytes());
    let computed = hex::encode(mac.finalize().into_bytes());
    if !eq_hex(&computed, &provided) {
        return Err(AuthError::HashMismatch);
    }

    let user_raw = fields.get("user").ok_or(AuthError::BadUser)?;
    let user: serde_json::Value = serde_json::from_str(user_raw).map_err(|_| AuthError::BadUser)?;
    let id = user
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or(AuthError::BadUser)?;
    let first_name = user
        .get("first_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(TelegramUser { id, first_name })
}

fn eq_hex(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
fn sign_init_data_for_tests(bot_token: &str, auth_date: i64, user_json: &str) -> String {
    let user_enc = urlencoding::encode(user_json);
    let mut fields = BTreeMap::new();
    fields.insert("auth_date".to_string(), auth_date.to_string());
    fields.insert("user".to_string(), user_json.to_string());
    fields.insert("query_id".to_string(), "AAE".to_string());
    let data_check = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut secret = HmacSha256::new_from_slice(b"WebAppData").expect("HMAC key");
    secret.update(bot_token.as_bytes());
    let secret_key = secret.finalize().into_bytes();
    let mut mac = HmacSha256::new_from_slice(&secret_key).expect("HMAC key");
    mac.update(data_check.as_bytes());
    let hash = hex::encode(mac.finalize().into_bytes());
    format!("auth_date={auth_date}&hash={hash}&query_id=AAE&user={user_enc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "123456:TEST_TOKEN";

    #[test]
    fn valid_init_data_roundtrip() {
        let now = 1_700_000_000;
        let signed = sign_init_data_for_tests(TOKEN, now, r#"{"id":42,"first_name":"A"}"#);
        assert_eq!(
            verify_init_data_at(&signed, TOKEN, now),
            Ok(TelegramUser {
                id: 42,
                first_name: Some("A".into()),
            })
        );
    }

    #[test]
    fn tampered_hash_is_rejected() {
        let now = 1_700_000_000;
        let signed = sign_init_data_for_tests(TOKEN, now, r#"{"id":42}"#);
        let tampered = signed.replace("hash=", "hash=00");
        assert_eq!(
            verify_init_data_at(&tampered, TOKEN, now),
            Err(AuthError::HashMismatch)
        );
    }

    #[test]
    fn expired_auth_date_is_rejected() {
        let signed = sign_init_data_for_tests(TOKEN, 1_000, r#"{"id":1}"#);
        assert_eq!(
            verify_init_data_at(&signed, TOKEN, 1_000 + 3601),
            Err(AuthError::Expired)
        );
    }

    #[test]
    fn wrong_bot_token_is_rejected() {
        let now = 1_700_000_000;
        let signed = sign_init_data_for_tests(TOKEN, now, r#"{"id":7}"#);
        assert_eq!(
            verify_init_data_at(&signed, "other:token", now),
            Err(AuthError::HashMismatch)
        );
    }
}
