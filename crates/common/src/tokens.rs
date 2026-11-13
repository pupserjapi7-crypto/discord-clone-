use std::time::{Duration, Instant};

use dashmap::DashMap;
use rand::{RngCore, rngs::OsRng};

/// A live access token. Kept only in process memory: revocation is immediate and
/// there is no signature to verify on the hot path.
#[derive(Clone, Debug)]
pub struct AccessToken {
    pub user_id: i64,
    pub session_id: String,
    pub expires_at: Instant,
}

#[derive(Debug)]
pub struct TokenStore {
    tokens: DashMap<String, AccessToken>,
    ttl: Duration,
}

impl TokenStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            tokens: DashMap::new(),
            ttl,
        }
    }

    /// Issue a new opaque token for an authenticated session.
    pub fn issue(&self, user_id: i64, session_id: &str) -> String {
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        let token = hex::encode(raw);
        self.tokens.insert(
            token.clone(),
            AccessToken {
                user_id,
                session_id: session_id.to_owned(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        token
    }

    pub fn validate(&self, token: &str) -> Option<AccessToken> {
        let entry = self.tokens.get(token)?;
        if entry.expires_at <= Instant::now() {
            drop(entry);
            self.tokens.remove(token);
            return None;
        }
        Some(entry.clone())
    }

    /// Revoke every token that belongs to a login session.
    pub fn revoke_session(&self, session_id: &str) {
        self.tokens
            .retain(|_, token| token.session_id != session_id);
    }

    pub fn sweep(&self) {
        let now = Instant::now();
        self.tokens.retain(|_, token| token.expires_at > now);
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_validate() {
        let store = TokenStore::new(Duration::from_secs(60));
        let token = store.issue(42, "s1");
        let validated = store.validate(&token).expect("token must validate");
        assert_eq!(validated.user_id, 42);
        assert_eq!(validated.session_id, "s1");
        assert!(store.validate("nope").is_none());
    }

    #[test]
    fn revoking_a_session_invalidates_its_tokens() {
        let store = TokenStore::new(Duration::from_secs(60));
        let a = store.issue(1, "s1");
        let b = store.issue(1, "s2");
        store.revoke_session("s1");
        assert!(store.validate(&a).is_none());
        assert!(store.validate(&b).is_some());
    }
}
