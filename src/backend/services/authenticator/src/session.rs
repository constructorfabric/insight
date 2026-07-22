//! Session Manager — the single owner of session state in Redis (DESIGN §3.2,
//! §3.7). All keys carry the `asm:` prefix (authenticator session management).
//!
//! Multi-key writes go through `MULTI/EXEC` pipelines so a session and its
//! linked JWT, indexes, and refresh schedule stay consistent. The store fails
//! closed: a Redis error surfaces to the handler, which answers 401/503.
//!
//! Owns create / resolve / rotate (`/auth/refresh`) / exchange-reissue /
//! revoke and the login-state store. The sid-index consumer (back-channel
//! logout) and the refresh-due consumer (IdP refresher) key off the schema
//! written here.

use std::collections::HashMap;

use anyhow::Context as _;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

/// Transient per-login state, keyed by the OIDC `state` value (5 min TTL).
#[derive(Debug, Clone)]
pub struct LoginState {
    pub pkce_verifier: String,
    pub nonce: String,
    pub return_to: String,
}

impl LoginState {
    /// The HASH fields for `asm:login_state:{state}`.
    fn to_fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("pkce_verifier", self.pkce_verifier.clone()),
            ("nonce", self.nonce.clone()),
            ("return_to", self.return_to.clone()),
        ]
    }

    /// Parse a login-state HASH map (missing fields become empty strings).
    fn from_map(map: &HashMap<String, String>) -> Self {
        let get = |k: &str| map.get(k).cloned().unwrap_or_default();
        Self {
            pkce_verifier: get("pkce_verifier"),
            nonce: get("nonce"),
            return_to: get("return_to"),
        }
    }
}

/// A session record — the `asm:session:{session_id}` HASH (DESIGN §3.7).
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub person_id: String,
    /// The person's email (from the id_token at login) — surfaced to the SPA
    /// via `/auth/me` so the (email-keyed) frontend can self-locate.
    pub email: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub idp_iss: String,
    pub idp_sub: String,
    pub idp_sid: Option<String>,
    pub id_token: String,
    pub idp_refresh_token: Option<String>,
    pub idp_access_expires_at: Option<u64>,
    pub created_at: u64,
    pub expires_at: u64,
    pub absolute_expires_at: u64,
    pub user_agent: String,
    pub ip: String,
    pub csrf_token: String,
    /// The live cookie credential mapping to this session (deleted on revoke).
    pub current_token: String,
}

impl SessionRecord {
    /// The HASH fields for `asm:session:{session_id}` (DESIGN §3.7). `Vec`
    /// arrays serialize as JSON; optional fields store `""` when absent.
    fn to_fields(&self) -> Vec<(&'static str, String)> {
        let json = |v: &[String]| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_owned());
        vec![
            ("person_id", self.person_id.clone()),
            ("email", self.email.clone()),
            ("tenant_id", self.tenant_id.clone()),
            ("roles", json(&self.roles)),
            ("idp_iss", self.idp_iss.clone()),
            ("idp_sub", self.idp_sub.clone()),
            ("idp_sid", self.idp_sid.clone().unwrap_or_default()),
            ("id_token", self.id_token.clone()),
            (
                "idp_refresh_token",
                self.idp_refresh_token.clone().unwrap_or_default(),
            ),
            (
                "idp_access_expires_at",
                self.idp_access_expires_at
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            ("created_at", self.created_at.to_string()),
            ("expires_at", self.expires_at.to_string()),
            ("absolute_expires_at", self.absolute_expires_at.to_string()),
            ("user_agent", self.user_agent.clone()),
            ("ip", self.ip.clone()),
            ("csrf_token", self.csrf_token.clone()),
            ("current_token", self.current_token.clone()),
        ]
    }

    /// Parse a session HASH map; empty strings become `None` for optional
    /// fields, malformed numbers/JSON degrade to defaults.
    fn from_map(map: &HashMap<String, String>) -> Self {
        let get = |k: &str| map.get(k).cloned().unwrap_or_default();
        let opt = |k: &str| {
            let v = get(k);
            if v.is_empty() { None } else { Some(v) }
        };
        let parse_vec = |k: &str| serde_json::from_str::<Vec<String>>(&get(k)).unwrap_or_default();
        let parse_u64 = |k: &str| get(k).parse::<u64>().unwrap_or(0);

        Self {
            person_id: get("person_id"),
            email: get("email"),
            tenant_id: get("tenant_id"),
            roles: parse_vec("roles"),
            idp_iss: get("idp_iss"),
            idp_sub: get("idp_sub"),
            idp_sid: opt("idp_sid"),
            id_token: get("id_token"),
            idp_refresh_token: opt("idp_refresh_token"),
            idp_access_expires_at: opt("idp_access_expires_at").and_then(|v| v.parse().ok()),
            created_at: parse_u64("created_at"),
            expires_at: parse_u64("expires_at"),
            absolute_expires_at: parse_u64("absolute_expires_at"),
            user_agent: get("user_agent"),
            ip: get("ip"),
            csrf_token: get("csrf_token"),
            current_token: get("current_token"),
        }
    }
}

/// Everything needed to persist a brand-new session in one pipeline.
pub struct NewSession {
    pub session_id: String,
    pub token: String,
    pub record: SessionRecord,
    /// The linked JWT minted at login.
    pub jwt: String,
    /// TTL (seconds) for the stored JWT copy — `jwt_reissue_after_seconds`.
    /// Its expiry is what triggers reissue-ahead on the exchange path.
    pub jwt_reissue_after_seconds: u64,
    /// Refresh-schedule score (epoch seconds), when IdP refresh is scheduled.
    pub refresh_due_at: Option<u64>,
}

fn session_key(session_id: &str) -> String {
    format!("asm:session:{session_id}")
}
fn token_key(token: &str) -> String {
    format!("asm:token:{token}")
}
fn jwt_key(session_id: &str) -> String {
    format!("asm:jwt:{session_id}")
}
fn user_sessions_key(person_id: &str) -> String {
    format!("asm:user_sessions:{person_id}")
}
fn sid_index_key(iss: &str, idp_sid: &str) -> String {
    format!("asm:sid_index:{iss}:{idp_sid}")
}
fn sub_index_key(iss: &str, idp_sub: &str) -> String {
    format!("asm:sub_index:{iss}:{idp_sub}")
}
fn logout_jti_key(iss: &str, jti: &str) -> String {
    format!("asm:logout_jti:{iss}:{jti}")
}
fn login_state_key(state: &str) -> String {
    format!("asm:login_state:{state}")
}
/// Live login-state index (ZSET, score = expiry) backing the layer-2 cap:
/// counting `asm:login_state:*` cheaply requires an index, not SCAN-per-login.
const LOGIN_STATE_LIVE_KEY: &str = "asm:login_state_live";
fn service_jti_key(service: &str, jti: &str) -> String {
    format!("asm:svc_jti:{service}:{jti}")
}
const REFRESH_DUE_KEY: &str = "asm:idp_refresh_due";

/// The Session Manager. Cheap to clone (the connection manager is `Arc`-backed).
#[derive(Clone)]
pub struct SessionManager {
    conn: ConnectionManager,
}

impl SessionManager {
    /// Connect to Redis and establish a resilient connection manager.
    ///
    /// # Errors
    /// Fails when the URL is malformed or the initial connection can't be made.
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(!redis_url.is_empty(), "redis_url is required (fail closed)");
        let client = redis::Client::open(redis_url).context("open Redis client")?;
        let conn = client
            .get_connection_manager()
            .await
            .context("establish Redis connection manager")?;
        Ok(Self { conn })
    }

    /// Liveness check for readiness (Redis reachable). Fail closed if not.
    ///
    /// # Errors
    /// Fails when Redis does not answer `PING`.
    pub async fn ping(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .context("Redis PING")?;
        anyhow::ensure!(pong == "PONG", "unexpected PING reply: {pong}");
        Ok(())
    }

    // ── Login state ──────────────────────────────────────────────────────

    /// Store per-login state under `state`, expiring in `ttl_seconds`.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn put_login_state(
        &self,
        state: &str,
        ls: &LoginState,
        ttl_seconds: u64,
        now: u64,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let key = login_state_key(state);
        redis::pipe()
            .atomic()
            .hset_multiple(&key, &ls.to_fields())
            .ignore()
            .expire(&key, i64::try_from(ttl_seconds).unwrap_or(300))
            .ignore()
            // Live index (score = expiry) backing the layer-2 login cap.
            .zadd(
                LOGIN_STATE_LIVE_KEY,
                state,
                i64::try_from(now + ttl_seconds).unwrap_or(i64::MAX),
            )
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("store login state")?;
        Ok(())
    }

    /// Count live (unexpired) login states — the layer-2 cap input.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn live_login_states(&self, now: u64) -> anyhow::Result<u64> {
        let mut conn = self.conn.clone();
        conn.zcount(LOGIN_STATE_LIVE_KEY, format!("({now}"), "+inf")
            .await
            .context("count live login states")
    }

    /// Take one token from the `class`/`key` bucket (layer-2 rate limit).
    ///
    /// # Errors
    /// Fails on a Redis error — callers fail open (the coarse gateway layer
    /// still guards) rather than turning a Redis blip into a lockout.
    pub async fn rate_limit_take(
        &self,
        class: &str,
        key: &str,
        spec: crate::ratelimit::BucketSpec,
        now: u64,
    ) -> anyhow::Result<bool> {
        crate::ratelimit::take(&self.conn, class, key, spec, now).await
    }

    /// Atomically read and delete the login state for `state` (one-shot).
    ///
    /// # Errors
    /// Fails on a Redis error. Returns `Ok(None)` when the state is unknown or
    /// already consumed / expired.
    pub async fn take_login_state(&self, state: &str) -> anyhow::Result<Option<LoginState>> {
        let mut conn = self.conn.clone();
        let key = login_state_key(state);
        // HGETALL then DEL in one atomic transaction.
        let (map, _deleted, _unindexed): (HashMap<String, String>, i64, i64) = redis::pipe()
            .atomic()
            .hgetall(&key)
            .del(&key)
            .zrem(LOGIN_STATE_LIVE_KEY, state)
            .query_async(&mut conn)
            .await
            .context("take login state")?;
        if map.is_empty() {
            return Ok(None);
        }
        Ok(Some(LoginState::from_map(&map)))
    }

    // ── Service-token assertion replay guard ───────────────────────────────

    /// One-shot replay guard for an RFC 7523 client assertion `jti`
    /// (`asm:svc_jti:{service}:{jti}`), mirroring the back-channel `logout_jti`
    /// pattern: `SET NX EX`. Returns `true` when this `jti` was seen for the
    /// first time (the caller may proceed), `false` when it is a replay.
    ///
    /// # Errors
    /// Fails on a Redis error (the handler then fails closed).
    pub async fn guard_service_jti(
        &self,
        service: &str,
        jti: &str,
        ttl_seconds: u64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn.clone();
        let set: Option<String> = redis::cmd("SET")
            .arg(service_jti_key(service, jti))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds.max(1))
            .query_async(&mut conn)
            .await
            .context("guard service-token jti (NX EX)")?;
        Ok(set.is_some())
    }

    // ── Session lifecycle ──────────────────────────────────────────────────

    /// Persist a new session, its token mapping, linked JWT, and indexes in one
    /// `MULTI/EXEC` pipeline (DESIGN §3.2 "Create").
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn create_session(&self, s: &NewSession) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let r = &s.record;
        let skey = session_key(&s.session_id);
        let expires_at = i64::try_from(r.expires_at).unwrap_or(i64::MAX);

        let fields = r.to_fields();

        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.hset_multiple(&skey, &fields).ignore();
        pipe.expire_at(&skey, expires_at).ignore();
        // Cookie credential mapping -> session_id, same expiry as the session.
        pipe.set(token_key(&s.token), &s.session_id).ignore();
        pipe.expire_at(token_key(&s.token), expires_at).ignore();
        // Linked JWT — stored-copy TTL is the reissue-after window (its expiry
        // triggers reissue-ahead on the exchange path).
        pipe.set_options(
            jwt_key(&s.session_id),
            &s.jwt,
            redis::SetOptions::default()
                .with_expiration(redis::SetExpiry::EX(s.jwt_reissue_after_seconds)),
        )
        .ignore();
        // User-session index (score = expiry).
        pipe.zadd(user_sessions_key(&r.person_id), &s.session_id, expires_at)
            .ignore();
        // Back-channel logout indexes: by OIDC `sid` (when the IdP supplies
        // one) and by `(iss, sub)` — the sub-only fallback path.
        let absolute = i64::try_from(r.absolute_expires_at).unwrap_or(i64::MAX);
        if let Some(sid) = &r.idp_sid {
            let idx = sid_index_key(&r.idp_iss, sid);
            pipe.sadd(&idx, &s.session_id).ignore();
            pipe.expire_at(&idx, absolute).ignore();
        }
        if !r.idp_sub.is_empty() {
            let idx = sub_index_key(&r.idp_iss, &r.idp_sub);
            pipe.sadd(&idx, &s.session_id).ignore();
            pipe.expire_at(&idx, absolute).ignore();
        }
        // IdP refresh schedule (consumer lands in step 10).
        if let Some(due) = s.refresh_due_at {
            pipe.zadd(
                REFRESH_DUE_KEY,
                &s.session_id,
                i64::try_from(due).unwrap_or(i64::MAX),
            )
            .ignore();
        }

        pipe.query_async::<()>(&mut conn)
            .await
            .context("create session pipeline")?;
        Ok(())
    }

    /// Resolve a cookie token to its session record. `Ok(None)` when the token
    /// maps nowhere or the session record is gone (fail closed -> 401).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn resolve_by_token(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<(String, SessionRecord)>> {
        let mut conn = self.conn.clone();
        let session_id: Option<String> = conn
            .get(token_key(token))
            .await
            .context("resolve token mapping")?;
        let Some(session_id) = session_id else {
            return Ok(None);
        };
        match self.load_session(&session_id).await? {
            Some(record) => Ok(Some((session_id, record))),
            None => Ok(None),
        }
    }

    /// Load a session record by its stable id.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn load_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRecord>> {
        let mut conn = self.conn.clone();
        let map: HashMap<String, String> = conn
            .hgetall(session_key(session_id))
            .await
            .context("load session hash")?;
        if map.is_empty() {
            return Ok(None);
        }
        Ok(Some(SessionRecord::from_map(&map)))
    }

    /// The linked JWT for a session, if the stored copy is still fresh.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn get_linked_jwt(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        let mut conn = self.conn.clone();
        conn.get(jwt_key(session_id))
            .await
            .context("read linked JWT")
    }

    /// Store a reissued JWT with `SET ... NX EX` (DD-ROUTER-10 stampede safety).
    /// Returns `true` iff this caller won the race and its JWT is canonical.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn store_reissued_jwt(
        &self,
        session_id: &str,
        jwt: &str,
        ttl_seconds: u64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn.clone();
        let set: Option<String> = redis::cmd("SET")
            .arg(jwt_key(session_id))
            .arg(jwt)
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds)
            .query_async(&mut conn)
            .await
            .context("store reissued JWT (NX EX)")?;
        Ok(set.is_some())
    }

    /// Rotate the session credential (`POST /auth/refresh`, DESIGN §3.6
    /// "Session refresh — rotation without churn"): write the new token
    /// mapping, shorten the superseded mapping's TTL to the rotation grace,
    /// advance the session's `expires_at` (record field, key TTL, and per-user
    /// index score). The stable `session_id`, the linked JWT, and every other
    /// index stay untouched (G10).
    ///
    /// **Compare-and-swap on `current_token`** (atomic Lua): the whole
    /// rotation runs only if the session's stored `current_token` still equals
    /// the credential the caller presented. Two concurrent refreshes of the
    /// same cookie (multi-tab) therefore cannot both rotate — the loser gets
    /// `Ok(false)` and the handler answers the grace path with the winner's
    /// credential, so no orphan full-TTL token mapping is ever minted.
    ///
    /// # Errors
    /// Fails on a Redis error. `Ok(false)` = the presented token was no longer
    /// current (lost the race / already rotated); nothing was written.
    pub async fn rotate_session(
        &self,
        session_id: &str,
        record: &SessionRecord,
        presented_token: &str,
        new_token: &str,
        new_expires_at: u64,
        grace_ms: u64,
    ) -> anyhow::Result<bool> {
        // KEYS: session hash, new-token mapping, old-token mapping, user ZSET.
        // ARGV: expected current_token, session_id, expireAt (s), grace (ms).
        const ROTATE_LUA: &str = r"
            if redis.call('HGET', KEYS[1], 'current_token') ~= ARGV[1] then return 0 end
            redis.call('SET', KEYS[2], ARGV[2])
            redis.call('EXPIREAT', KEYS[2], ARGV[3])
            redis.call('PEXPIRE', KEYS[3], ARGV[4])
            redis.call('HSET', KEYS[1], 'expires_at', ARGV[3], 'current_token', ARGV[5])
            redis.call('EXPIREAT', KEYS[1], ARGV[3])
            redis.call('ZADD', KEYS[4], ARGV[3], ARGV[2])
            return 1
        ";
        let mut conn = self.conn.clone();
        let expires_at = i64::try_from(new_expires_at).unwrap_or(i64::MAX);
        let rotated: i64 = redis::Script::new(ROTATE_LUA)
            .key(session_key(session_id))
            .key(token_key(new_token))
            .key(token_key(&record.current_token))
            .key(user_sessions_key(&record.person_id))
            .arg(presented_token)
            .arg(session_id)
            .arg(expires_at)
            .arg(i64::try_from(grace_ms).unwrap_or(250))
            .arg(new_token)
            .invoke_async(&mut conn)
            .await
            .context("rotate session (CAS)")?;
        Ok(rotated == 1)
    }

    // ── Background workers (G5 refresher, janitor) ─────────────────────────

    /// Try to take (or renew) a leader lock. `SET key holder NX PX ttl` wins a
    /// free lock; an already-held lock renews only for the same `holder`
    /// (DD-BFF-09 — one leader per pass, Redis is already a hard dependency).
    ///
    /// # Errors
    /// Fails on a Redis error (the worker then skips this pass).
    pub async fn try_lead(&self, key: &str, holder: &str, ttl_ms: u64) -> anyhow::Result<bool> {
        // Atomic acquire-or-renew: SET NX wins a free lock; otherwise renew the
        // TTL **only if we still hold it** — compare-and-pexpire in one script
        // so the lock can't expire between a GET and a PEXPIRE and let a stale
        // holder extend the new leader's key (brief dual leadership).
        const LEAD_LUA: &str = r"
            if redis.call('SET', KEYS[1], ARGV[1], 'NX', 'PX', ARGV[2]) then return 1 end
            if redis.call('GET', KEYS[1]) == ARGV[1] then
                redis.call('PEXPIRE', KEYS[1], ARGV[2])
                return 1
            end
            return 0
        ";
        let mut conn = self.conn.clone();
        let led: i64 = redis::Script::new(LEAD_LUA)
            .key(key)
            .arg(holder)
            .arg(ttl_ms.max(1))
            .invoke_async(&mut conn)
            .await
            .context("acquire/renew leader lock")?;
        Ok(led == 1)
    }

    /// Sessions due for IdP refresh (`ZRANGEBYSCORE asm:idp_refresh_due 0 now`,
    /// bounded).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn due_refresh_sessions(
        &self,
        now: u64,
        limit: usize,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn.clone();
        conn.zrangebyscore_limit(
            REFRESH_DUE_KEY,
            0,
            i64::try_from(now).unwrap_or(i64::MAX),
            0,
            isize::try_from(limit).unwrap_or(isize::MAX),
        )
        .await
        .context("read refresh schedule")
    }

    /// Per-session refresh lock (`SET NX PX`): refresh-token rotation is
    /// one-time-use at most IdPs; two workers racing the same rotation would
    /// burn the grant and falsely kill the session. Returns `true` when this
    /// caller holds the lock.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn lock_session_refresh(
        &self,
        session_id: &str,
        ttl_ms: u64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn.clone();
        let set: Option<String> = redis::cmd("SET")
            .arg(format!("asm:refresh_lock:{session_id}"))
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(ttl_ms.max(1))
            .query_async(&mut conn)
            .await
            .context("acquire per-session refresh lock")?;
        Ok(set.is_some())
    }

    /// Release the per-session refresh lock.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn unlock_session_refresh(&self, session_id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = conn
            .del(format!("asm:refresh_lock:{session_id}"))
            .await
            .context("release per-session refresh lock")?;
        Ok(())
    }

    /// Persist a successful IdP refresh: rotated refresh token (when the IdP
    /// returned one), new access-token expiry, reset failure counter, and the
    /// next schedule entry — one atomic Lua step, **guarded on the session
    /// still existing**. Returns `false` when the session was revoked while the
    /// grant was in flight: without the guard, `HSET` on the deleted key would
    /// resurrect a TTL-less hash holding the freshly-rotated (live) IdP refresh
    /// token — a permanent, janitor-invisible secret for a logged-out user
    /// (review H2). On `false` the caller drops the schedule entry.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn store_idp_refresh(
        &self,
        session_id: &str,
        new_refresh_token: Option<&str>,
        access_expires_at: Option<u64>,
        next_due: u64,
    ) -> anyhow::Result<bool> {
        // KEYS: session hash, refresh-due ZSET.
        // ARGV: session_id, next_due, refresh_token|"", access_exp|"".
        const STORE_LUA: &str = r"
            if redis.call('EXISTS', KEYS[1]) == 0 then return 0 end
            if ARGV[3] ~= '' then redis.call('HSET', KEYS[1], 'idp_refresh_token', ARGV[3]) end
            if ARGV[4] ~= '' then redis.call('HSET', KEYS[1], 'idp_access_expires_at', ARGV[4]) end
            redis.call('HSET', KEYS[1], 'idp_refresh_failures', '0')
            redis.call('ZADD', KEYS[2], ARGV[2], ARGV[1])
            return 1
        ";
        let mut conn = self.conn.clone();
        let stored: i64 = redis::Script::new(STORE_LUA)
            .key(session_key(session_id))
            .key(REFRESH_DUE_KEY)
            .arg(session_id)
            .arg(i64::try_from(next_due).unwrap_or(i64::MAX))
            .arg(new_refresh_token.unwrap_or(""))
            .arg(access_expires_at.map(|e| e.to_string()).unwrap_or_default())
            .invoke_async(&mut conn)
            .await
            .context("store IdP refresh (guarded)")?;
        Ok(stored == 1)
    }

    /// Bump the per-session transient-failure counter; returns the new count
    /// (sizes the exponential backoff), or `None` if the session no longer
    /// exists (so a revoke mid-flight can't resurrect a counter-only zombie).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn bump_refresh_failures(&self, session_id: &str) -> anyhow::Result<Option<u64>> {
        const BUMP_LUA: &str = r"
            if redis.call('EXISTS', KEYS[1]) == 0 then return -1 end
            return redis.call('HINCRBY', KEYS[1], 'idp_refresh_failures', 1)
        ";
        let mut conn = self.conn.clone();
        let failures: i64 = redis::Script::new(BUMP_LUA)
            .key(session_key(session_id))
            .invoke_async(&mut conn)
            .await
            .context("bump refresh failures (guarded)")?;
        Ok((failures >= 0).then(|| u64::try_from(failures).unwrap_or(0)))
    }

    /// Re-schedule a session's next refresh attempt.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn reschedule_refresh(&self, session_id: &str, due_at: u64) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = conn
            .zadd(
                REFRESH_DUE_KEY,
                session_id,
                i64::try_from(due_at).unwrap_or(i64::MAX),
            )
            .await
            .context("reschedule refresh")?;
        Ok(())
    }

    /// Drop a session from the refresh schedule (dead session, or nothing to
    /// refresh).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn unschedule_refresh(&self, session_id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = conn
            .zrem(REFRESH_DUE_KEY, session_id)
            .await
            .context("unschedule refresh")?;
        Ok(())
    }

    /// One janitor pass (DESIGN §4.3): trim expired members from every
    /// `asm:user_sessions:*` ZSET (`ZREMRANGEBYSCORE 0 now` — per-key TTLs
    /// removed the records, the index members linger) and drop long-overdue
    /// orphans from the refresh schedule (live sessions are re-scheduled by
    /// the refresher; an entry still due after `orphan_grace` has no owner).
    /// Returns (removed members, overdue-backlog size before trimming).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn janitor_pass(&self, now: u64, orphan_grace: u64) -> anyhow::Result<(u64, u64)> {
        let mut conn = self.conn.clone();
        let now_i = i64::try_from(now).unwrap_or(i64::MAX);
        let mut removed = 0u64;
        let mut backlog = 0u64;

        // SCAN, never KEYS — bounded batches on a shared Redis.
        let mut cursor: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("asm:user_sessions:*")
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .context("scan user-session indexes")?;
            for key in keys {
                let expired: u64 = conn
                    .zcount(&key, 0, now_i)
                    .await
                    .context("count expired index members")?;
                if expired > 0 {
                    backlog += expired;
                    let n: u64 = conn
                        .zrembyscore(&key, 0, now_i)
                        .await
                        .context("trim expired index members")?;
                    removed += n;
                }
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        // Expired login-state index members (the HASH keys expired via TTL).
        let stale_states: u64 = conn
            .zrembyscore(LOGIN_STATE_LIVE_KEY, 0, now_i)
            .await
            .context("trim expired login-state index")?;
        removed += stale_states;

        // Refresh-schedule orphans: an entry overdue by more than the grace
        // window is trimmed **only if its session hash is actually gone**
        // (review M4). Blind ZREMRANGEBYSCORE would silently delete live-but-
        // behind entries — after a Redis restore from an old backup, or while
        // the refresher is disabled/wedged — permanently stopping IdP refresh
        // for those sessions with no signal (voiding the G5 guarantee).
        let orphan_cutoff = i64::try_from(now.saturating_sub(orphan_grace)).unwrap_or(0);
        let overdue: Vec<String> = conn
            .zrangebyscore(REFRESH_DUE_KEY, 0, now_i)
            .await
            .context("list overdue refresh entries")?;
        backlog += overdue.len() as u64;
        for sid in &overdue {
            // Only past the grace window, and only when the owner is gone.
            let score: Option<i64> = conn
                .zscore(REFRESH_DUE_KEY, sid)
                .await
                .context("read refresh-due score")?;
            if score.is_none_or(|s| s > orphan_cutoff) {
                continue;
            }
            let exists: bool = conn
                .exists(session_key(sid))
                .await
                .context("check session existence for orphan")?;
            if !exists {
                let n: u64 = conn
                    .zrem(REFRESH_DUE_KEY, sid)
                    .await
                    .context("trim refresh-schedule orphan")?;
                removed += n;
            }
        }

        Ok((removed, backlog))
    }

    // ── Back-channel logout (PRD 5.10) ─────────────────────────────────────

    /// One-shot replay guard for a back-channel `logout_token` `jti`
    /// (`asm:logout_jti:{iss}:{jti}`, `SET NX EX`). Returns `true` on first
    /// delivery; `false` when this `(iss, jti)` was already accepted.
    ///
    /// # Errors
    /// Fails on a Redis error (the handler then fails closed).
    pub async fn guard_logout_jti(
        &self,
        iss: &str,
        jti: &str,
        ttl_seconds: u64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn.clone();
        let set: Option<String> = redis::cmd("SET")
            .arg(logout_jti_key(iss, jti))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds.max(1))
            .query_async(&mut conn)
            .await
            .context("guard logout jti (NX EX)")?;
        Ok(set.is_some())
    }

    /// Release a back-channel `jti` guard (`DEL`). Called when the revoke that
    /// followed a first-delivery claim then failed — otherwise the IdP's retry
    /// of the same `logout_token` would hit the still-set guard and get an
    /// idempotent 200 without ever revoking (review M1). Revoke is idempotent,
    /// so re-processing on retry is safe.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn release_logout_jti(&self, iss: &str, jti: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let _: i64 = conn
            .del(logout_jti_key(iss, jti))
            .await
            .context("release logout jti")?;
        Ok(())
    }

    /// Sessions indexed under a back-channel `(iss, sid)` pair.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn sessions_by_idp_sid(
        &self,
        iss: &str,
        idp_sid: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn.clone();
        conn.smembers(sid_index_key(iss, idp_sid))
            .await
            .context("read sid index")
    }

    /// Sessions indexed under a back-channel `(iss, sub)` pair (the sub-only
    /// fallback).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn sessions_by_idp_sub(
        &self,
        iss: &str,
        idp_sub: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn.clone();
        conn.smembers(sub_index_key(iss, idp_sub))
            .await
            .context("read sub index")
    }

    /// List a person's live sessions from the per-user index (score > `now`),
    /// loading each record. Index members whose record has already expired are
    /// skipped (the janitor trims them).
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn list_user_sessions(
        &self,
        person_id: &str,
        now: u64,
    ) -> anyhow::Result<Vec<(String, SessionRecord)>> {
        let mut conn = self.conn.clone();
        let session_ids: Vec<String> = conn
            .zrangebyscore(user_sessions_key(person_id), format!("({now}"), "+inf")
            .await
            .context("list user sessions by score")?;
        let mut out = Vec::with_capacity(session_ids.len());
        for sid in session_ids {
            if let Some(record) = self.load_session(&sid).await? {
                out.push((sid, record));
            }
        }
        Ok(out)
    }

    /// Revoke one session — delete session, linked JWT, live token mapping,
    /// ZSET member, sid-index member, and refresh-due member in one pipeline
    /// (DESIGN §3.2 "Revoke"). Idempotent. Returns `true` if a session existed.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn revoke_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let Some(r) = self.load_session(session_id).await? else {
            return Ok(false);
        };
        let mut conn = self.conn.clone();
        let mut pipe = redis::pipe();
        pipe.atomic();
        pipe.del(session_key(session_id)).ignore();
        pipe.del(jwt_key(session_id)).ignore();
        pipe.del(token_key(&r.current_token)).ignore();
        pipe.zrem(user_sessions_key(&r.person_id), session_id)
            .ignore();
        if let Some(sid) = &r.idp_sid {
            pipe.srem(sid_index_key(&r.idp_iss, sid), session_id)
                .ignore();
        }
        if !r.idp_sub.is_empty() {
            pipe.srem(sub_index_key(&r.idp_iss, &r.idp_sub), session_id)
                .ignore();
        }
        pipe.zrem(REFRESH_DUE_KEY, session_id).ignore();
        pipe.query_async::<()>(&mut conn)
            .await
            .context("revoke session pipeline")?;
        Ok(true)
    }

    /// Revoke every live session for a person. Returns the count revoked.
    ///
    /// # Errors
    /// Fails on a Redis error.
    pub async fn revoke_user_sessions(&self, person_id: &str) -> anyhow::Result<u64> {
        let mut conn = self.conn.clone();
        let ukey = user_sessions_key(person_id);
        let session_ids: Vec<String> = conn
            .zrange(&ukey, 0, -1)
            .await
            .context("list user sessions")?;
        let mut revoked = 0u64;
        for sid in &session_ids {
            if self.revoke_session(sid).await? {
                revoked += 1;
            } else {
                // Session hash already expired (TTL) but its index member
                // lingers — trim it so the ZSET can't accumulate stale ids.
                let _: i64 = conn
                    .zrem(&ukey, sid)
                    .await
                    .context("trim stale session index")?;
            }
        }
        Ok(revoked)
    }
}
