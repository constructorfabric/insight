//! Session store — the only writer/reader of `bff:*` keys.
//!
//! Phase 1 surface:
//!   * `create_session` — new login, atomic MULTI/EXEC, with session-
//!     fixation guard on an incoming cookie value.
//!   * `get_session` — HGETALL of a session record.
//!   * `revoke_session` — atomic delete + ZREM + SREM + cache invalidation.
//!
//! Refresh + revoke-all + back-channel revocation land in Phase 2/3.

use std::sync::Arc;

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tracing::warn;

use crate::bff::errors::BffError;
use crate::bff::redis_keys;
use crate::bff::secrets::{new_csrf_token, new_session_id};
use crate::bff::session::SessionRecord;
use crate::redis_client::RedisShared;

/// Inputs to `create_session`.
pub struct CreateSessionRequest<'a> {
    pub user_id: &'a str,
    pub tenant_id: &'a str,
    pub idp_iss: &'a str,
    pub idp_sub: &'a str,
    /// Empty string means "IdP did not supply a sid claim". Empty values
    /// skip the sid_index write.
    pub idp_sid: &'a str,
    pub id_token: &'a str,
    pub email: &'a str,
    pub display_name: &'a str,
    pub user_agent: &'a str,
    pub ip: &'a str,
    pub now: i64,
    pub session_ttl_seconds: u64,
    pub absolute_lifetime_seconds: u64,
    /// Cookie value present on the callback request. We never reuse it;
    /// if it maps to a live session we revoke it before creating the new
    /// one (DESIGN §3.6 fixation guard).
    pub incoming_sid: Option<&'a str>,
}

pub struct CreateSessionOutcome {
    pub session_id: String,
    /// The full record we just persisted. Phase 1 callers only need
    /// `session_id`, but Phase 2 (refresh/sessions endpoints) will read
    /// from this without touching Redis again.
    #[allow(dead_code)]
    pub record: SessionRecord,
}

/// Concrete session-store implementation backed by Redis.
#[derive(Clone)]
pub struct SessionStore {
    redis: Arc<RedisShared>,
}

impl SessionStore {
    #[must_use]
    pub fn new(redis: Arc<RedisShared>) -> Self {
        Self { redis }
    }

    fn conn(&self) -> ConnectionManager {
        self.redis.manager()
    }

    /// Look up a live session record by SID. Returns `None` if the key
    /// does not exist (expired or never existed). Surfaces a
    /// `StoreUnavailable` error when Redis cannot be reached.
    pub async fn get_session(&self, sid: &str) -> Result<Option<SessionRecord>, BffError> {
        let mut conn = self.conn();
        let key = redis_keys::session(sid);
        let pairs: Vec<(String, String)> = conn
            .hgetall(&key)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;
        if pairs.is_empty() {
            return Ok(None);
        }
        decode_session(&pairs)
    }

    /// Create a fresh session, optionally revoking an incoming attacker-
    /// planted SID first. Returns the new opaque `session_id` and the
    /// stored record.
    pub async fn create_session(
        &self,
        req: CreateSessionRequest<'_>,
    ) -> Result<CreateSessionOutcome, BffError> {
        let mut conn = self.conn();

        // 1. Fixation guard: revoke incoming SID if it resolved to a live
        //    session. We do NOT propagate any cookie state into the new
        //    session. A revoke failure on a stale/unknown SID isn't fatal —
        //    log and continue. A real Redis outage would also fail step 2
        //    below and bail there.
        if let Some(incoming) = req.incoming_sid
            && !incoming.is_empty()
            && let Err(e) = self.revoke_session(incoming).await
        {
            warn!(
                error = %e,
                "failed to revoke incoming SID during fixation guard; continuing",
            );
        }

        // 2. Mint fresh session_id + CSRF token (server-side CSPRNG).
        let session_id = new_session_id();
        let csrf_token = new_csrf_token();

        let expires_at = req
            .now
            .saturating_add(i64::try_from(req.session_ttl_seconds).unwrap_or(i64::MAX));
        let absolute_expires_at = req
            .now
            .saturating_add(i64::try_from(req.absolute_lifetime_seconds).unwrap_or(i64::MAX));

        let record = SessionRecord {
            user_id: req.user_id.to_owned(),
            tenant_id: req.tenant_id.to_owned(),
            idp_iss: req.idp_iss.to_owned(),
            idp_sub: req.idp_sub.to_owned(),
            idp_sid: req.idp_sid.to_owned(),
            id_token: req.id_token.to_owned(),
            email: req.email.to_owned(),
            display_name: req.display_name.to_owned(),
            created_at: req.now,
            expires_at,
            absolute_expires_at,
            user_agent: req.user_agent.to_owned(),
            ip: req.ip.to_owned(),
            csrf_token,
        };

        // 3. Atomic write: HSET + EXPIREAT + ZADD (+ SADD when sid).
        let pairs = record.to_redis_pairs();
        let session_key = redis_keys::session(&session_id);
        let user_key = redis_keys::user_sessions(&record.user_id);

        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hset_multiple(&session_key, &pairs).ignore();
        pipe.expire_at(&session_key, expires_at).ignore();
        pipe.zadd(&user_key, &session_id, expires_at).ignore();
        if !record.idp_sid.is_empty() {
            let sid_idx = redis_keys::sid_index(&record.idp_iss, &record.idp_sid);
            pipe.sadd(&sid_idx, &session_id).ignore();
        }

        let _: () = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        Ok(CreateSessionOutcome { session_id, record })
    }

    /// Revoke a session by SID. Idempotent: revoking a missing session is
    /// a successful no-op.
    pub async fn revoke_session(&self, sid: &str) -> Result<(), BffError> {
        let mut conn = self.conn();

        // Read the three index pointers we need to drop. Explicit HMGET
        // with a positional tuple, so we never have to second-guess
        // whether redis-rs flattened nils into a shorter Vec.
        let session_key = redis_keys::session(sid);
        let (user_id_opt, idp_iss_opt, idp_sid_opt): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = redis::cmd("HMGET")
            .arg(&session_key)
            .arg("user_id")
            .arg("idp_iss")
            .arg("idp_sid")
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        // Missing session → all three nils → no-op.
        if user_id_opt.is_none() && idp_iss_opt.is_none() && idp_sid_opt.is_none() {
            return Ok(());
        }

        let user_id = user_id_opt.unwrap_or_default();
        let idp_iss = idp_iss_opt.unwrap_or_default();
        let idp_sid = idp_sid_opt.unwrap_or_default();

        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.del(&session_key).ignore();
        if !user_id.is_empty() {
            pipe.zrem(redis_keys::user_sessions(&user_id), sid).ignore();
        }
        if !idp_iss.is_empty() && !idp_sid.is_empty() {
            pipe.srem(redis_keys::sid_index(&idp_iss, &idp_sid), sid)
                .ignore();
        }
        pipe.del(redis_keys::router_jwt_cache(sid)).ignore();

        let _: () = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        Ok(())
    }
}

/// PKCE login state stored at `bff:login_state:{state}` for 5 minutes.
///
/// Phase 1: this is read once on `/auth/callback` and deleted. Step-up
/// to a typed Redis HASH read happens via `HGETALL`; the spec only requires
/// us to keep PKCE verifier + nonce + return URL.
pub mod login_state {
    use std::sync::Arc;

    use redis::AsyncCommands;
    use serde::{Deserialize, Serialize};

    use crate::bff::errors::BffError;
    use crate::bff::redis_keys;
    use crate::redis_client::RedisShared;

    pub const TTL_SECONDS: u64 = 300; // 5 min, per DESIGN §3.7

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LoginState {
        pub pkce_verifier: String,
        pub nonce: String,
        pub return_to: String,
    }

    pub async fn store(
        redis: &Arc<RedisShared>,
        state: &str,
        ls: &LoginState,
    ) -> Result<(), BffError> {
        let mut conn = redis.manager();
        let key = redis_keys::login_state(state);
        let pairs: [(&'static str, String); 3] = [
            ("pkce_verifier", ls.pkce_verifier.clone()),
            ("nonce", ls.nonce.clone()),
            ("return_to", ls.return_to.clone()),
        ];
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hset_multiple(&key, &pairs).ignore();
        pipe.expire(&key, i64::try_from(TTL_SECONDS).unwrap_or(i64::MAX))
            .ignore();
        let _: () = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;
        Ok(())
    }

    /// Read and delete the login-state record in one round-trip. Returns
    /// `None` if the state has expired, was already consumed, or never
    /// existed (state mismatch).
    pub async fn take(
        redis: &Arc<RedisShared>,
        state: &str,
    ) -> Result<Option<LoginState>, BffError> {
        let mut conn = redis.manager();
        let key = redis_keys::login_state(state);
        // Pipeline: HGETALL + DEL — atomic enough for a single-shot consumption.
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hgetall(&key);
        pipe.del(&key).ignore();
        let pairs: Vec<(String, String)> = pipe
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;
        if pairs.is_empty() {
            return Ok(None);
        }
        let mut verifier = String::new();
        let mut nonce = String::new();
        let mut return_to = String::new();
        for (k, v) in pairs {
            match k.as_str() {
                "pkce_verifier" => verifier = v,
                "nonce" => nonce = v,
                "return_to" => return_to = v,
                _ => {}
            }
        }
        if verifier.is_empty() || nonce.is_empty() {
            return Ok(None);
        }
        Ok(Some(LoginState {
            pkce_verifier: verifier,
            nonce,
            return_to,
        }))
    }

    /// Increment the per-pod login-state cap counter and return the new
    /// value. The caller compares against `auth_login_state_max` and
    /// rejects with 429 if exceeded.
    ///
    /// Phase 1 stub — we increment but the cap check lives in Phase 3
    /// (`cpt-insightspec-nfr-bff-rate-limit-auth`). Plumbed in now so the
    /// counter exists before the rate-limit middleware lands.
    pub async fn touch(redis: &Arc<RedisShared>) -> Result<i64, BffError> {
        let mut conn = redis.manager();
        let v: i64 = conn
            .incr("bff:rl:login_state_count", 1)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;
        Ok(v)
    }
}

fn decode_session(pairs: &[(String, String)]) -> Result<Option<SessionRecord>, BffError> {
    let get = |name: &str| -> String {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let parse_i64 = |name: &str| -> Result<i64, BffError> {
        let raw = get(name);
        if raw.is_empty() {
            return Ok(0);
        }
        raw.parse::<i64>().map_err(|_| {
            BffError::Internal(anyhow::anyhow!("session field {name} is not i64: {raw}"))
        })
    };

    Ok(Some(SessionRecord {
        user_id: get("user_id"),
        tenant_id: get("tenant_id"),
        idp_iss: get("idp_iss"),
        idp_sub: get("idp_sub"),
        idp_sid: get("idp_sid"),
        id_token: get("id_token"),
        email: get("email"),
        display_name: get("display_name"),
        created_at: parse_i64("created_at")?,
        expires_at: parse_i64("expires_at")?,
        absolute_expires_at: parse_i64("absolute_expires_at")?,
        user_agent: get("user_agent"),
        ip: get("ip"),
        csrf_token: get("csrf_token"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_session_round_trips_through_pairs() {
        let original = SessionRecord {
            user_id: "u-1".into(),
            tenant_id: "t-1".into(),
            idp_iss: "iss".into(),
            idp_sub: "sub".into(),
            idp_sid: "isid".into(),
            id_token: "jwt".into(),
            email: "alice@example.com".into(),
            display_name: "Alice".into(),
            created_at: 100,
            expires_at: 220,
            absolute_expires_at: 28_900,
            user_agent: "ua".into(),
            ip: "1.2.3.4".into(),
            csrf_token: "csrf".into(),
        };
        let pairs: Vec<(String, String)> = original
            .to_redis_pairs()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let decoded = decode_session(&pairs).expect("ok").expect("present");
        assert_eq!(decoded.user_id, original.user_id);
        assert_eq!(decoded.expires_at, original.expires_at);
        assert_eq!(decoded.absolute_expires_at, original.absolute_expires_at);
        assert_eq!(decoded.csrf_token, original.csrf_token);
        assert_eq!(decoded.email, original.email);
    }

    #[test]
    fn decode_session_handles_missing_optional_fields() {
        let pairs = vec![
            ("user_id".to_owned(), "u-1".to_owned()),
            ("expires_at".to_owned(), "220".to_owned()),
        ];
        let decoded = decode_session(&pairs).expect("ok").expect("present");
        assert_eq!(decoded.user_id, "u-1");
        assert_eq!(decoded.expires_at, 220);
        assert_eq!(decoded.email, "");
        assert_eq!(decoded.created_at, 0);
    }

    #[test]
    fn decode_session_rejects_non_numeric_int_fields() {
        let pairs = vec![
            ("user_id".to_owned(), "u-1".to_owned()),
            ("expires_at".to_owned(), "not-a-number".to_owned()),
        ];
        assert!(decode_session(&pairs).is_err());
    }

    /// End-to-end SessionStore round-trip against a real Redis. Skipped
    /// unless `BFF_TEST_REDIS_URL` is set, so CI / local `cargo test`
    /// without Redis stays green. Run with:
    ///
    /// ```ignore
    /// BFF_TEST_REDIS_URL=redis://localhost:6379/15 cargo test \
    ///     -p insight-api-gateway --bin insight-api-gateway \
    ///     -- --ignored session_store_round_trip
    /// ```
    ///
    /// Use a dedicated DB (e.g. `/15`) — the test wipes its own keys
    /// but does not flush the DB, and a stray collision could break a
    /// shared dev instance.
    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn session_store_round_trip_against_real_redis() {
        let Ok(url) = std::env::var("BFF_TEST_REDIS_URL") else {
            eprintln!("BFF_TEST_REDIS_URL not set; skipping");
            return;
        };
        let client = redis::Client::open(url).expect("open client");
        let manager = redis::aio::ConnectionManager::new(client)
            .await
            .expect("connect");
        let shared = std::sync::Arc::new(crate::redis_client::RedisShared::__test_from_manager(
            manager,
        ));
        let store = SessionStore::new(shared);

        let req = CreateSessionRequest {
            user_id: "test-user-zzz",
            tenant_id: "test-tenant",
            idp_iss: "https://test-idp/",
            idp_sub: "test-sub-zzz",
            idp_sid: "test-isid-zzz",
            id_token: "irrelevant",
            email: "test@example.com",
            display_name: "Test",
            user_agent: "ua",
            ip: "127.0.0.1",
            now: 1,
            session_ttl_seconds: 60,
            absolute_lifetime_seconds: 3600,
            incoming_sid: None,
        };
        let outcome = store.create_session(req).await.expect("create");
        let sid = outcome.session_id.clone();

        let read = store
            .get_session(&sid)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(read.user_id, "test-user-zzz");
        assert_eq!(read.email, "test@example.com");
        assert_eq!(read.expires_at, 61);

        store.revoke_session(&sid).await.expect("revoke");
        let after = store.get_session(&sid).await.expect("get");
        assert!(after.is_none(), "session should be gone after revoke");
    }
}
