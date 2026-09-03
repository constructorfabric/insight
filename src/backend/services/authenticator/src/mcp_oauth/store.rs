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

    pub async fn put_client(&self, client: &RegisteredClient) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let value = serde_json::to_string(client)?;
        conn.set::<_, _, ()>(client_key(&client.client_id), value)
            .await
            .context("store MCP OAuth client")
    }

    pub async fn client(&self, client_id: &str) -> anyhow::Result<Option<RegisteredClient>> {
        self.get_json(client_key(client_id), "load MCP OAuth client")
            .await
    }

    pub async fn put_pending(
        &self,
        request_id: &str,
        pending: &PendingAuthorization,
        ttl_seconds: u64,
    ) -> anyhow::Result<()> {
        self.set_json_ex(
            pending_key(request_id),
            pending,
            ttl_seconds,
            "store MCP authorization request",
        )
        .await
    }

    pub async fn pending(&self, request_id: &str) -> anyhow::Result<Option<PendingAuthorization>> {
        self.get_json(pending_key(request_id), "load MCP authorization request")
            .await
    }

    pub async fn take_pending(
        &self,
        request_id: &str,
    ) -> anyhow::Result<Option<PendingAuthorization>> {
        self.take_json(pending_key(request_id), "consume MCP authorization request")
            .await
    }

    pub async fn put_code(
        &self,
        code: &str,
        grant: &AuthorizationCodeGrant,
        ttl_seconds: u64,
    ) -> anyhow::Result<()> {
        self.set_json_ex(
            code_key(code),
            grant,
            ttl_seconds,
            "store MCP authorization code",
        )
        .await
    }

    pub async fn take_code(&self, code: &str) -> anyhow::Result<Option<AuthorizationCodeGrant>> {
        self.take_json(code_key(code), "consume MCP authorization code")
            .await
    }

    pub async fn put_refresh(
        &self,
        token_hash: &str,
        grant: &RefreshGrant,
        ttl_seconds: u64,
    ) -> anyhow::Result<()> {
        self.set_json_ex(
            refresh_key(token_hash),
            grant,
            ttl_seconds,
            "store MCP refresh token",
        )
        .await
    }

    pub async fn refresh(&self, token_hash: &str) -> anyhow::Result<Option<RefreshGrant>> {
        self.get_json(refresh_key(token_hash), "load MCP refresh token")
            .await
    }

    pub async fn delete_refresh(&self, token_hash: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(refresh_key(token_hash))
            .await
            .context("revoke MCP refresh token")
    }

    pub async fn rotate_refresh(
        &self,
        old_hash: &str,
        old_grant: &RefreshGrant,
        new_hash: &str,
        new_grant: &RefreshGrant,
        ttl_seconds: u64,
    ) -> anyhow::Result<bool> {
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
            .await
            .context("rotate MCP refresh token")?;
        Ok(rotated == 1)
    }

    async fn set_json_ex<T: serde::Serialize>(
        &self,
        key: String,
        value: &T,
        ttl_seconds: u64,
        context: &'static str,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let value = serde_json::to_string(value)?;
        conn.set_ex::<_, _, ()>(key, value, ttl_seconds)
            .await
            .context(context)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        key: String,
        context: &'static str,
    ) -> anyhow::Result<Option<T>> {
        let mut conn = self.conn.clone();
        let value: Option<String> = conn.get(key).await.context(context)?;
        value
            .map(|value| serde_json::from_str(&value).context(context))
            .transpose()
    }

    async fn take_json<T: serde::de::DeserializeOwned>(
        &self,
        key: String,
        context: &'static str,
    ) -> anyhow::Result<Option<T>> {
        let mut conn = self.conn.clone();
        let value: Option<String> = redis::cmd("GETDEL")
            .arg(key)
            .query_async(&mut conn)
            .await
            .context(context)?;
        value
            .map(|value| serde_json::from_str(&value).context(context))
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
    format!("{{mcp_oauth}}:code:{code}")
}

fn refresh_key(token_hash: &str) -> String {
    format!("{{mcp_oauth}}:refresh:{token_hash}")
}
