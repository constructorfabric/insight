//! Session store — the only writer/reader of `bff:*` keys.
//!
//! Phase 1 surface:
//!   * `create_session` — new login, atomic MULTI/EXEC, with session-
//!     fixation guard on an incoming cookie value.
//!   * `get_session` — HGETALL of a session record.
//!   * `revoke_session` — atomic delete + ZREM + SREM + cache invalidation.
//!
//! Phase 2 surface:
//!   * `refresh_session` — rolling cookie rotation per DD-BFF-10. Uses
//!     `SET bff:swap:{old_sid} NX PX grace_ms` as a stand-alone claim gate,
//!     then runs the rest of the rotation atomically. The DESIGN diagram
//!     shows everything inside one MULTI/EXEC, but MULTI/EXEC has no
//!     "abort on NX miss" semantic — queued commands always execute. The
//!     split here keeps loser racers out of the rotation while preserving
//!     the spec's "winner does the work, loser uses the swap" contract.
//!
//! Revoke-all + back-channel revocation land in Phase 3.

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

/// Inputs to `refresh_session`.
pub struct RefreshSessionRequest<'a> {
    /// Opaque SID from the incoming `__Host-sid` cookie.
    pub old_sid: &'a str,
    pub now: i64,
    pub session_ttl_seconds: u64,
    pub refresh_grace_ms: u64,
}

/// Successful refresh outcome — either a fresh rotation or a grace-path
/// resolution of a just-rotated SID.
pub struct RefreshSessionOutcome {
    pub new_sid: String,
    pub expires_at: i64,
    /// `user_id` for audit. Empty on the grace path when the resolved
    /// session record happens to have lost that field — treat as best-effort.
    pub user_id: String,
    /// `false` for the normal rotation path; `true` when the caller arrived
    /// with a just-rotated SID and we resolved it via `bff:swap:{old_sid}`.
    pub graced: bool,
}

/// Refresh result. `Gone` collapses every "401 + clear cookie" reason
/// (no session, no swap, or past absolute cap) so the handler has one
/// branch to render.
pub enum RefreshSessionResult {
    Ok(RefreshSessionOutcome),
    Gone,
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

    /// Rolling cookie rotation per DD-BFF-10. Three terminal states:
    ///
    ///   * `Ok(Outcome{graced=false})` — normal path: we owned the rotation,
    ///     wrote the swap key, RENAMEd the session, updated TTLs and indexes,
    ///     and invalidated the Router's JWT cache for the old SID.
    ///   * `Ok(Outcome{graced=true})` — grace path: caller arrived with a
    ///     just-rotated SID (or lost the race against a concurrent refresh).
    ///     We resolved `bff:swap:{old_sid}` to the current SID and did NOT
    ///     rotate again.
    ///   * `Gone` — no session, no swap, or past absolute cap. Caller must
    ///     respond 401 + clear cookie.
    pub async fn refresh_session(
        &self,
        req: RefreshSessionRequest<'_>,
    ) -> Result<RefreshSessionResult, BffError> {
        let mut conn = self.conn();
        let old_session_key = redis_keys::session(req.old_sid);

        // Read the four fields we need to rotate. HMGET with nil-tolerant
        // tuple deserialization — a missing key gives all-None.
        let (user_id_opt, idp_iss_opt, idp_sid_opt, abs_exp_opt): (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = redis::cmd("HMGET")
            .arg(&old_session_key)
            .arg("user_id")
            .arg("idp_iss")
            .arg("idp_sid")
            .arg("absolute_expires_at")
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        // No session under `old_sid` → maybe a just-rotated cookie within
        // the grace window.
        let Some(user_id) = user_id_opt else {
            return self.try_grace(req.old_sid).await;
        };

        let idp_iss = idp_iss_opt.unwrap_or_default();
        let idp_sid = idp_sid_opt.unwrap_or_default();
        let abs_exp: i64 = abs_exp_opt
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Past the absolute cap — refresh refuses to extend regardless of
        // remaining TTL. Spec §5.3: hard cap behaviour delegated to TTL,
        // but mirrored here so we never re-extend a session that has
        // already crossed `absolute_expires_at`.
        if abs_exp <= req.now {
            return Ok(RefreshSessionResult::Gone);
        }

        let proposed_exp = req
            .now
            .saturating_add(i64::try_from(req.session_ttl_seconds).unwrap_or(i64::MAX));
        let new_exp = proposed_exp.min(abs_exp);
        if new_exp <= req.now {
            // Cap so close that the next TTL would not move forward.
            return Ok(RefreshSessionResult::Gone);
        }

        // Claim the rotation slot. `SET ... PX <ms> NX` is atomic in
        // Redis; the racer who sees nil falls into the grace path below.
        let new_sid = new_session_id();
        let swap_key = redis_keys::swap(req.old_sid);
        let claim: Option<String> = redis::cmd("SET")
            .arg(&swap_key)
            .arg(&new_sid)
            .arg("PX")
            .arg(req.refresh_grace_ms)
            .arg("NX")
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        if claim.is_none() {
            // Someone else won the rotation. Resolve via the swap key
            // they already wrote.
            return self.try_grace(req.old_sid).await;
        }

        // We own the rotation. Run RENAME + index updates atomically.
        let new_session_key = redis_keys::session(&new_sid);
        let user_sessions_key = redis_keys::user_sessions(&user_id);

        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.cmd("RENAME")
            .arg(&old_session_key)
            .arg(&new_session_key)
            .ignore();
        pipe.hset(&new_session_key, "expires_at", new_exp).ignore();
        pipe.expire_at(&new_session_key, new_exp).ignore();
        pipe.zrem(&user_sessions_key, req.old_sid).ignore();
        pipe.zadd(&user_sessions_key, &new_sid, new_exp).ignore();
        if !idp_iss.is_empty() && !idp_sid.is_empty() {
            let sid_idx = redis_keys::sid_index(&idp_iss, &idp_sid);
            pipe.srem(&sid_idx, req.old_sid).ignore();
            pipe.sadd(&sid_idx, &new_sid).ignore();
        }
        pipe.del(redis_keys::router_jwt_cache(req.old_sid)).ignore();

        let pipe_result: Result<(), redis::RedisError> = pipe.query_async(&mut conn).await;
        if let Err(e) = pipe_result {
            // Most common cause: a concurrent /auth/logout (or future
            // /auth/sessions revoke) deleted the session between our
            // HMGET and this atomic block, so RENAME aborts with
            // "no such key". The swap key we already wrote points at a
            // SID we never materialised; the next request from the same
            // browser will resolve it via `try_grace` and get Gone.
            //
            // Surface that as 401 + clear cookie (Gone) rather than
            // bubbling a 503 — the user's session is genuinely gone, not
            // the store. A real Redis outage propagates from `try_grace`
            // below as `StoreUnavailable` and reaches the handler.
            tracing::warn!(
                error = %e,
                "refresh rotation pipeline failed after swap claim; falling back to grace",
            );
            return self.try_grace(req.old_sid).await;
        }

        Ok(RefreshSessionResult::Ok(RefreshSessionOutcome {
            new_sid,
            expires_at: new_exp,
            user_id,
            graced: false,
        }))
    }

    /// Resolve a swap key set by a concurrent refresh into the current SID.
    /// Returns `Gone` if no swap exists, or if the swap points at a session
    /// record that no longer exists (rotation failed mid-flight, or the
    /// session was revoked between the swap write and now).
    async fn try_grace(&self, old_sid: &str) -> Result<RefreshSessionResult, BffError> {
        let mut conn = self.conn();
        let new_sid: Option<String> = redis::cmd("GET")
            .arg(redis_keys::swap(old_sid))
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;
        let Some(new_sid) = new_sid else {
            return Ok(RefreshSessionResult::Gone);
        };

        let new_session_key = redis_keys::session(&new_sid);
        let (expires_at_opt, user_id_opt): (Option<String>, Option<String>) = redis::cmd("HMGET")
            .arg(&new_session_key)
            .arg("expires_at")
            .arg("user_id")
            .query_async(&mut conn)
            .await
            .map_err(|e| BffError::StoreUnavailable(e.to_string()))?;

        let Some(expires_at_s) = expires_at_opt else {
            return Ok(RefreshSessionResult::Gone);
        };
        let expires_at: i64 = expires_at_s.parse().unwrap_or(0);
        if expires_at <= 0 {
            return Ok(RefreshSessionResult::Gone);
        }

        Ok(RefreshSessionResult::Ok(RefreshSessionOutcome {
            new_sid,
            expires_at,
            user_id: user_id_opt.unwrap_or_default(),
            graced: true,
        }))
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

        // Unique IDs so parallel `cargo test` runs don't stomp on each
        // other's keys when sharing the same Redis DB.
        let s = test_suffix();
        let user_id = format!("test-user-{s}");
        let now = 4_070_908_800_i64;
        let req = CreateSessionRequest {
            user_id: &user_id,
            tenant_id: "test-tenant",
            idp_iss: "https://test-idp/",
            idp_sub: &format!("test-sub-{s}"),
            idp_sid: &format!("test-isid-{s}"),
            id_token: "irrelevant",
            email: "test@example.com",
            display_name: "Test",
            user_agent: "ua",
            ip: "127.0.0.1",
            now,
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
        assert_eq!(read.user_id, user_id);
        assert_eq!(read.email, "test@example.com");
        assert_eq!(read.expires_at, now + 60);

        store.revoke_session(&sid).await.expect("revoke");
        let after = store.get_session(&sid).await.expect("get");
        assert!(after.is_none(), "session should be gone after revoke");
    }

    // --- Phase 2: refresh + logout integration tests against real Redis ---
    //
    // Every test mints its own user/tenant/idp_sid suffix so parallel
    // `cargo test` runs don't collide on shared keys. Each test cleans up
    // its own keyspace at the end via `revoke_session` plus explicit
    // deletes for grace + jwt_cache keys.

    /// Open the shared Redis manager for #[ignore] tests, or `None` to
    /// skip when `BFF_TEST_REDIS_URL` isn't set.
    #[cfg(test)]
    async fn open_test_store() -> Option<SessionStore> {
        let url = std::env::var("BFF_TEST_REDIS_URL").ok()?;
        let client = redis::Client::open(url).ok()?;
        let manager = redis::aio::ConnectionManager::new(client).await.ok()?;
        let shared = std::sync::Arc::new(crate::redis_client::RedisShared::__test_from_manager(
            manager,
        ));
        Some(SessionStore::new(shared))
    }

    /// Test scaffold: returns (store, base_user_id, base_idp_sid).
    /// Each test gets a distinct suffix so parallel runs don't stomp.
    #[cfg(test)]
    fn test_suffix() -> String {
        // 6 chars of base64url from CSPRNG — plenty to disambiguate.
        crate::bff::secrets::new_session_id()
            .chars()
            .take(6)
            .collect()
    }

    /// Fixed "current" epoch for Redis tests: 2099-01-01T00:00:00Z. We need
    /// a real-future timestamp because `EXPIREAT` with a past value (or any
    /// value before Redis's wall clock) evicts the key immediately. Using a
    /// fixed value keeps every assertion deterministic.
    #[cfg(test)]
    const TEST_NOW: i64 = 4_070_908_800;

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_rotates_sid_atomically_against_real_redis() {
        let Some(store) = open_test_store().await else {
            eprintln!("BFF_TEST_REDIS_URL not set; skipping");
            return;
        };
        let s = test_suffix();
        let user_id = format!("u-{s}");
        let idp_sid = format!("isid-{s}");
        let now = TEST_NOW;

        let created = store
            .create_session(CreateSessionRequest {
                user_id: &user_id,
                tenant_id: "t-1",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &idp_sid,
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let old_sid = created.session_id.clone();

        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &old_sid,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        let outcome = match result {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("expected Ok, got Gone"),
        };
        assert!(!outcome.graced, "normal path must not be graced");
        assert_ne!(outcome.new_sid, old_sid, "SID must rotate");
        assert_eq!(outcome.user_id, user_id);
        assert_eq!(outcome.expires_at, now + 130);

        // Old session record is gone (RENAMEd away).
        assert!(store.get_session(&old_sid).await.expect("get").is_none());

        // New session record carries the rotated expiry.
        let new_rec = store
            .get_session(&outcome.new_sid)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(new_rec.expires_at, now + 130);
        assert_eq!(new_rec.user_id, user_id);

        store
            .revoke_session(&outcome.new_sid)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_grace_path_resolves_swap_without_rotating() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let now = TEST_NOW;
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &format!("u-{s}"),
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &format!("isid-{s}"),
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let real_sid = created.session_id.clone();

        // Simulate a just-rotated cookie: write swap pointing at the
        // live session. Suffix the stale sid so parallel test runs don't
        // step on each other's swap keys.
        let stale_sid = format!("phantom-old-sid-{s}");
        let mut conn = store.conn();
        let _: () = redis::cmd("SET")
            .arg(redis_keys::swap(&stale_sid))
            .arg(&real_sid)
            .arg("PX")
            .arg(5_000_u64)
            .query_async(&mut conn)
            .await
            .expect("seed swap");

        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &stale_sid,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        let outcome = match result {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("expected grace, got Gone"),
        };
        assert!(
            outcome.graced,
            "stale cookie within grace must take grace path"
        );
        assert_eq!(outcome.new_sid, real_sid, "grace must resolve to live SID");
        // Live session was created with new_exp = now + ttl(120).
        assert_eq!(outcome.expires_at, now + 120);

        // The swap key must NOT have been consumed (grace is read-only).
        let still_there: Option<String> = redis::cmd("GET")
            .arg(redis_keys::swap(&stale_sid))
            .query_async(&mut conn)
            .await
            .expect("get swap");
        assert_eq!(still_there.as_deref(), Some(real_sid.as_str()));

        // Cleanup.
        let _: () = redis::cmd("DEL")
            .arg(redis_keys::swap(&stale_sid))
            .query_async(&mut conn)
            .await
            .expect("cleanup swap");
        store.revoke_session(&real_sid).await.expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_returns_gone_when_no_session_and_no_swap() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let bogus = format!("bogus-{}", test_suffix());
        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &bogus,
                now: TEST_NOW,
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        assert!(matches!(result, RefreshSessionResult::Gone));
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_returns_gone_past_absolute_cap() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let now = TEST_NOW;
        // abs_lifetime = 3600 → abs_exp = now + 3600. Refresh at now + 5000
        // is past the cap → Gone.
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &format!("u-{s}"),
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &format!("isid-{s}"),
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 3_600,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let sid = created.session_id.clone();

        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &sid,
                now: now + 5_000, // > now + 3_600 (abs_exp)
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        assert!(matches!(result, RefreshSessionResult::Gone));

        // Session is still there (refresh refused; it didn't revoke).
        // We clean up so the test is hermetic.
        store.revoke_session(&sid).await.expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_caps_new_exp_at_absolute_cap() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let now = TEST_NOW;
        // abs_exp = now + 3_600. Refresh at now + 3_500 with ttl=200 would
        // propose now + 3_700; must clamp to abs_exp (now + 3_600).
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &format!("u-{s}"),
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &format!("isid-{s}"),
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 3_600,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &created.session_id,
                now: now + 3_500,
                session_ttl_seconds: 200,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        let outcome = match result {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("expected Ok"),
        };
        assert_eq!(
            outcome.expires_at,
            now + 3_600,
            "must clamp at absolute cap",
        );
        store
            .revoke_session(&outcome.new_sid)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_concurrent_races_into_one_winner_and_one_grace() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let user_id = format!("u-{s}");
        let now = TEST_NOW;
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &user_id,
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &format!("isid-{s}"),
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let old_sid = created.session_id.clone();

        // Fire two parallel refreshes on the same old_sid. Exactly one
        // must rotate; the other must take the grace path. Both must
        // succeed and converge on the same new SID.
        let s1 = store.clone();
        let old1 = old_sid.clone();
        let h1 = tokio::spawn(async move {
            s1.refresh_session(RefreshSessionRequest {
                old_sid: &old1,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 2_000, // generous grace so the loser hits it
            })
            .await
        });
        let s2 = store.clone();
        let old2 = old_sid.clone();
        let h2 = tokio::spawn(async move {
            s2.refresh_session(RefreshSessionRequest {
                old_sid: &old2,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 2_000,
            })
            .await
        });
        let r1 = h1.await.expect("join").expect("refresh1");
        let r2 = h2.await.expect("join").expect("refresh2");
        let o1 = match r1 {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("r1 gone"),
        };
        let o2 = match r2 {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("r2 gone"),
        };

        let graced_count = u8::from(o1.graced) + u8::from(o2.graced);
        assert_eq!(graced_count, 1, "exactly one of the two must be graced");
        assert_eq!(o1.new_sid, o2.new_sid, "both must converge on same new SID");

        // Only one entry in user_sessions ZSET (the new sid).
        let mut conn = store.conn();
        let members: Vec<String> = redis::cmd("ZRANGE")
            .arg(redis_keys::user_sessions(&user_id))
            .arg(0_i64)
            .arg(-1_i64)
            .query_async(&mut conn)
            .await
            .expect("zrange");
        assert_eq!(members, vec![o1.new_sid.clone()]);

        store.revoke_session(&o1.new_sid).await.expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_invalidates_router_jwt_cache_on_rotate() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let now = TEST_NOW;
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &format!("u-{s}"),
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &format!("isid-{s}"),
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let old_sid = created.session_id.clone();

        let mut conn = store.conn();
        let _: () = redis::cmd("SET")
            .arg(redis_keys::router_jwt_cache(&old_sid))
            .arg("dummy-jwt")
            .query_async(&mut conn)
            .await
            .expect("seed jwt cache");

        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &old_sid,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh");
        let outcome = match result {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("Gone"),
        };

        let cached: Option<String> = redis::cmd("GET")
            .arg(redis_keys::router_jwt_cache(&old_sid))
            .query_async(&mut conn)
            .await
            .expect("get cache");
        assert!(
            cached.is_none(),
            "stale jwt cache must be dropped on rotate"
        );

        store
            .revoke_session(&outcome.new_sid)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn refresh_session_skips_sid_index_when_idp_sid_empty() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let user_id = format!("u-{s}");
        let now = TEST_NOW;
        // No idp_sid → no sid_index entry on create, and refresh must
        // not error on the missing SREM/SADD pair.
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &user_id,
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: "",
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let result = store
            .refresh_session(RefreshSessionRequest {
                old_sid: &created.session_id,
                now: now + 10,
                session_ttl_seconds: 120,
                refresh_grace_ms: 250,
            })
            .await
            .expect("refresh must succeed with empty idp_sid");
        let outcome = match result {
            RefreshSessionResult::Ok(o) => o,
            RefreshSessionResult::Gone => panic!("Gone"),
        };
        assert!(!outcome.graced);
        store
            .revoke_session(&outcome.new_sid)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn logout_full_revoke_drops_session_and_indexes() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let s = test_suffix();
        let user_id = format!("u-{s}");
        let idp_sid = format!("isid-{s}");
        let created = store
            .create_session(CreateSessionRequest {
                user_id: &user_id,
                tenant_id: "t",
                idp_iss: "https://idp/",
                idp_sub: &format!("sub-{s}"),
                idp_sid: &idp_sid,
                id_token: "tok",
                email: "a@b",
                display_name: "A",
                user_agent: "ua",
                ip: "1.1.1.1",
                now: TEST_NOW,
                session_ttl_seconds: 120,
                absolute_lifetime_seconds: 28_800,
                incoming_sid: None,
            })
            .await
            .expect("create");
        let sid = created.session_id.clone();
        let mut conn = store.conn();

        // Seed a Router JWT cache entry so we can prove logout drops it.
        let _: () = redis::cmd("SET")
            .arg(redis_keys::router_jwt_cache(&sid))
            .arg("dummy-jwt")
            .query_async(&mut conn)
            .await
            .expect("seed cache");

        store.revoke_session(&sid).await.expect("logout revoke");

        // Session HASH gone.
        assert!(store.get_session(&sid).await.expect("get").is_none());
        // user_sessions ZSET no longer lists this sid.
        let members: Vec<String> = redis::cmd("ZRANGE")
            .arg(redis_keys::user_sessions(&user_id))
            .arg(0_i64)
            .arg(-1_i64)
            .query_async(&mut conn)
            .await
            .expect("zrange");
        assert!(!members.iter().any(|m| m == &sid));
        // sid_index SET no longer lists this sid.
        let in_set: bool = redis::cmd("SISMEMBER")
            .arg(redis_keys::sid_index("https://idp/", &idp_sid))
            .arg(&sid)
            .query_async(&mut conn)
            .await
            .expect("sismember");
        assert!(!in_set);
        // Router JWT cache dropped.
        let cached: Option<String> = redis::cmd("GET")
            .arg(redis_keys::router_jwt_cache(&sid))
            .query_async(&mut conn)
            .await
            .expect("get cache");
        assert!(cached.is_none());
    }

    #[tokio::test]
    #[ignore = "requires a running Redis; opt in via BFF_TEST_REDIS_URL"]
    async fn logout_is_idempotent_on_missing_session() {
        let Some(store) = open_test_store().await else {
            return;
        };
        let bogus = format!("bogus-{}", test_suffix());
        // Two calls — both succeed.
        store.revoke_session(&bogus).await.expect("first revoke");
        store.revoke_session(&bogus).await.expect("second revoke");
    }
}
