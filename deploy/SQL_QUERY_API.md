# SQL query API

`POST /api/sql/query` executes SQL without saving a query. It uses the same
read-only ClickHouse account, grants, SQL validation, and per-pod concurrency
budget as MCP. Saved-query endpoints and MCP OAuth are unchanged.

## Enable

Create two operator-managed Kubernetes Secrets in the release namespace:

- `insight-mcp-creds`, key `clickhouse-password`: the existing SQL explorer
  ClickHouse password. Reuse this Secret if MCP is already configured.
- `insight-sql-api-creds`, key `token`: a separate, cryptographically random
  instance token. Generate at least 32 random bytes, for example with
  `openssl rand -hex 32`, and store it in your secret manager. Never commit
  plaintext credentials; SealedSecrets can carry them in GitOps.

Enable the endpoint in Helm values:

```yaml
global:
  sqlApi:
    enabled: true
analytics:
  sqlApi:
    tokenSecret: insight-sql-api-creds
    tokenKey: token
```

MCP does not need to be enabled. The existing `analytics.mcp` values configure
the shared listener, ClickHouse credentials, and concurrency budget. When either
feature is enabled, database provisioning creates the read-only explorer access.
Leave `global.mcp.enabled` and `global.mcp.publicUrl` unchanged.

Inherit gateway route defaults. A custom `gateway.gateway.routes` list replaces
the complete default list and must explicitly include `/api/sql/query` with
`auth: instance_token`, upstream `http://{{ .Release.Name }}-analytics:8086`,
`timeoutMs: 40000`, and `sqlApiOnly: true`.

For Docker Compose, set `SQL_API_ENABLED=true`, `SQL_API_TOKEN`, and
`CLICKHOUSE_MCP_PASSWORD` in your untracked environment file. Restart analytics
after changing the token. In Kubernetes, update the token Secret and roll the
analytics Deployment: environment-injected secrets are read only at startup.
During a rolling rotation, old and new pods may temporarily accept different tokens.

## Query

With `INSIGHT_SQL_TOKEN` supplied by your secret manager:

```bash
curl --fail-with-body https://insight.example.com/api/sql/query \
  -H "Authorization: Bearer ${INSIGHT_SQL_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data '{"sql":"SELECT 1 AS answer"}'
```

```json
{
  "columns": [{"name": "answer", "type": "UInt8"}],
  "rows": [{"answer": 1}],
  "row_count": 1,
  "truncated": false
}
```

Discover available relations through `system.databases`, `system.tables`, and
`system.columns`. The same grants cover bronze, staging, silver, the configured
gold database, identity, and config. No new grants are introduced by this API.

Only one SELECT/WITH statement is accepted. Multiple CTEs, subqueries, joins,
and unions are allowed. Query SETTINGS/FORMAT and external table functions are
rejected. SQL is capped at 64 KiB and the request body at 128 KiB. The shared
executor caps ClickHouse response bytes at 5 MiB; database row, memory, scan,
and execution-time limits remain in force. Exceeding a limit fails the query;
results are not silently truncated.

Responses use canonical problem JSON for errors: 400 for invalid requests/SQL,
401 for missing or invalid tokens, 429 for capacity or size limits, 504 for query
timeouts, and 500 for backend failures. Responses are marked `Cache-Control:
no-store`. Database details and the token are not returned in errors.

## Access scope

The instance token grants all SQL explorer access, not access on behalf of an
individual user. There are no personal role checks, per-user revocation, or
automatic tenant-row filters on this endpoint. Distribute it only to trusted
operators authorized to access the entire allowed dataset, and require HTTPS.
Revoke access by rotating the token or disabling `global.sqlApi.enabled`.
