use anyhow::Context as _;
use redis::AsyncCommands as _;
use redis::aio::ConnectionManager;

use super::types::{AuthorizationCodeGrant, PendingAuthorization, RefreshGrant, RegisteredClient};

const ROTATE_REFRESH_LUA: &str = r"
local current = redis.call('GET', KEYS[1])
if not current or current ~= ARGV[1] then
  return 0
end
redis.call('DEL', KEYS[1])
redis.call('SET', KEYS[2], ARGV[2], 'EX', ARGV[3])
return 1
";

const ACTIVATE_CLIENT_LUA: &str = r"
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', ARGV[1])
if redis.call('EXISTS', KEYS[1]) == 0 then
  return 0
end
local counted = redis.call('ZSCORE', KEYS[2], ARGV[4])
if not counted and redis.call('ZCARD', KEYS[2]) >= tonumber(ARGV[3]) then
  return 2
end
redis.call('EXPIRE', KEYS[1], ARGV[2])
redis.call('ZADD', KEYS[2], ARGV[5], ARGV[4])
return 1
";

const ACTIVE_CAPACITY_LUA: &str = r"
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', ARGV[1])
if redis.call('ZSCORE', KEYS[1], ARGV[2]) then
  return 1
end
if redis.call('ZCARD', KEYS[1]) < tonumber(ARGV[3]) then
  return 1
end
return 0
";

const CLIENT_INDEX_KEY: &str = "{mcp_oauth}:v2:clients";

#[derive(Debug, thiserror::Error)]
pub enum McpOAuthStoreError {
    #[error("MCP OAuth Redis operation failed: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("MCP OAuth record serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("MCP OAuth client registration quota exhausted")]
    ClientQuotaExceeded,
    #[error("MCP OAuth client registration expired")]
    ClientExpired,
}

#[derive(Clone)]
pub struct McpOAuthStore {
    conn: ConnectionManager,
}

impl McpOAuthStore {
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(redis_url).context("open MCP OAuth Redis client")?;
        let conn = client
            .get_connection_manager()
            .await
            .context("connect MCP OAuth Redis client")?;
        Ok(Self { conn })
    }

    pub async fn put_client(
        &self,
        client: &RegisteredClient,
        ttl_seconds: u64,
    ) -> Result<(), McpOAuthStoreError> {
        self.set_json_ex(client_key(&client.client_id), client, ttl_seconds)
            .await
    }

    pub async fn activate_client(
        &self,
        client_id: &str,
        ttl_seconds: u64,
        max_clients: u64,
        now: u64,
    ) -> Result<(), McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let expires_at = now.saturating_add(ttl_seconds);
        let outcome: i64 = redis::Script::new(ACTIVATE_CLIENT_LUA)
            .key(client_key(client_id))
            .key(CLIENT_INDEX_KEY)
            .arg(now)
            .arg(ttl_seconds)
            .arg(max_clients)
            .arg(client_id)
            .arg(expires_at)
            .invoke_async(&mut conn)
            .await?;
        match outcome {
            1 => Ok(()),
            0 => Err(McpOAuthStoreError::ClientExpired),
            2 => Err(McpOAuthStoreError::ClientQuotaExceeded),
            _ => Err(McpOAuthStoreError::Redis(redis::RedisError::from((
                redis::ErrorKind::UnexpectedReturnType,
                "unexpected ACTIVATE_CLIENT_LUA result",
            )))),
        }
    }

    pub async fn active_capacity_for(
        &self,
        client_id: &str,
        max_clients: u64,
        now: u64,
    ) -> Result<bool, McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let admitted: i64 = redis::Script::new(ACTIVE_CAPACITY_LUA)
            .key(CLIENT_INDEX_KEY)
            .arg(now)
            .arg(client_id)
            .arg(max_clients)
            .invoke_async(&mut conn)
            .await?;
        Ok(admitted == 1)
    }

    pub async fn client(
        &self,
        client_id: &str,
    ) -> Result<Option<RegisteredClient>, McpOAuthStoreError> {
        self.get_json(client_key(client_id)).await
    }

    pub async fn put_pending(
        &self,
        request_id: &str,
        pending: &PendingAuthorization,
        ttl_seconds: u64,
    ) -> Result<(), McpOAuthStoreError> {
        self.set_json_ex(pending_key(request_id), pending, ttl_seconds)
            .await
    }

    pub async fn pending(
        &self,
        request_id: &str,
    ) -> Result<Option<PendingAuthorization>, McpOAuthStoreError> {
        self.get_json(pending_key(request_id)).await
    }

    pub async fn take_pending(
        &self,
        request_id: &str,
    ) -> Result<Option<PendingAuthorization>, McpOAuthStoreError> {
        self.take_json(pending_key(request_id)).await
    }

    pub async fn put_code(
        &self,
        code: &str,
        grant: &AuthorizationCodeGrant,
        ttl_seconds: u64,
    ) -> Result<(), McpOAuthStoreError> {
        self.set_json_ex(code_key(code), grant, ttl_seconds).await
    }

    pub async fn take_code(
        &self,
        code: &str,
    ) -> Result<Option<AuthorizationCodeGrant>, McpOAuthStoreError> {
        self.take_json(code_key(code)).await
    }

    pub async fn put_refresh(
        &self,
        token_hash: &str,
        grant: &RefreshGrant,
        ttl_seconds: u64,
    ) -> Result<(), McpOAuthStoreError> {
        self.set_json_ex(refresh_key(token_hash), grant, ttl_seconds)
            .await
    }

    pub async fn refresh(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshGrant>, McpOAuthStoreError> {
        self.get_json(refresh_key(token_hash)).await
    }

    pub async fn delete_refresh(&self, token_hash: &str) -> Result<(), McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(refresh_key(token_hash)).await?;
        Ok(())
    }

    pub async fn rotate_refresh(
        &self,
        old_hash: &str,
        old_grant: &RefreshGrant,
        new_hash: &str,
        new_grant: &RefreshGrant,
        ttl_seconds: u64,
    ) -> Result<bool, McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let expected = serde_json::to_string(old_grant)?;
        let replacement = serde_json::to_string(new_grant)?;
        let rotated: i64 = redis::Script::new(ROTATE_REFRESH_LUA)
            .key(refresh_key(old_hash))
            .key(refresh_key(new_hash))
            .arg(expected)
            .arg(replacement)
            .arg(ttl_seconds)
            .invoke_async(&mut conn)
            .await?;
        Ok(rotated == 1)
    }

    async fn set_json_ex<T: serde::Serialize>(
        &self,
        key: String,
        value: &T,
        ttl_seconds: u64,
    ) -> Result<(), McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let value = serde_json::to_string(value)?;
        conn.set_ex::<_, _, ()>(key, value, ttl_seconds).await?;
        Ok(())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        key: String,
    ) -> Result<Option<T>, McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(key).await?;
        value
            .map(|value| serde_json::from_str(&value).map_err(McpOAuthStoreError::from))
            .transpose()
    }

    async fn take_json<T: serde::de::DeserializeOwned>(
        &self,
        key: String,
    ) -> Result<Option<T>, McpOAuthStoreError> {
        let mut conn = self.conn.clone();
        let value: Option<String> = redis::cmd("GETDEL").arg(key).query_async(&mut conn).await?;
        value
            .map(|value| serde_json::from_str(&value).map_err(McpOAuthStoreError::from))
            .transpose()
    }
}

fn client_key(client_id: &str) -> String {
    format!("{{mcp_oauth}}:client:{client_id}")
}

fn pending_key(request_id: &str) -> String {
    format!("{{mcp_oauth}}:pending:{request_id}")
}

fn code_key(code: &str) -> String {
    format!("{{mcp_oauth}}:v2:code:{code}")
}

fn refresh_key(token_hash: &str) -> String {
    format!("{{mcp_oauth}}:v2:refresh:{token_hash}")
}
