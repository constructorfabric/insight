//! Session Manager — the single owner of session state in Redis (DESIGN §3.2,
//! §3.7). All keys carry the `asm:` prefix (authenticator session management).
//!
//! Multi-key writes go through `MULTI/EXEC` pipelines so a session and its
//! linked JWT, indexes, and refresh schedule stay consistent. The store fails
//! closed: a Redis error surfaces to the handler, which answers 401/503.
//!
//! Step 04 implements create / resolve / exchange-reissue / revoke and the
//! login-state store. Rotation (`/auth/refresh`), the sid-index consumer
//! (back-channel logout), and the refresh-due consumer (IdP refresher) are
//! wired into the schema here but their *consumers* land in later steps.

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
    pub tenants: Vec<String>,
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
            ("tenants", json(&self.tenants)),
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
            tenants: parse_vec("tenants"),
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
fn login_state_key(state: &str) -> String {
    format!("asm:login_state:{state}")
}
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
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let key = login_state_key(state);
        redis::pipe()
            .atomic()
            .hset_multiple(&key, &ls.to_fields())
            .ignore()
            .expire(&key, i64::try_from(ttl_seconds).unwrap_or(300))
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .context("store login state")?;
        Ok(())
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
        let (map, _deleted): (HashMap<String, String>, i64) = redis::pipe()
            .atomic()
            .hgetall(&key)
            .del(&key)
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
        // Back-channel logout index (only when the IdP supplies `sid`).
        if let Some(sid) = &r.idp_sid {
            let idx = sid_index_key(&r.idp_iss, sid);
            pipe.sadd(&idx, &s.session_id).ignore();
            pipe.expire_at(
                &idx,
                i64::try_from(r.absolute_expires_at).unwrap_or(i64::MAX),
            )
            .ignore();
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
