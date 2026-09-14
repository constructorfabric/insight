# Raw-data ingest

`POST /api/core/v1/raw-data` stores one JSON record in the stream's own
ClickHouse table. It authenticates with an instance token in `X-Insight-Token`,
not a user session; the same token also reaches `PUT /api/core/v1/tables/{table}`,
which additionally requires the admin role and so is unusable with a token
alone.

## Enable

Deploy the service and give it the token:

```yaml
global:
  insightV3Core:
    deploy: true
insightV3Core:
  ingest:
    tokenSecret: insight-v3-core-token
    tokenKey: token
```

Create the operator-managed Secret `insight-v3-core-token` with key `token` in
the release namespace before the first deploy — without it the pod does not
start. The token must be 32 to 1024 bytes; generate at least 32 random bytes,
for example `openssl rand -hex 32`, and keep it in your secret manager. Never
commit plaintext; SealedSecrets carry it in GitOps, where
`environments/<env>/inventory.yaml` lists it for `make seal`.

Token strength is the operator's responsibility. Startup checks length, not
randomness.

Inherit gateway route defaults. A custom `gateway.gateway.routes` list replaces
the complete default list and must explicitly include `/api/core` with
`auth: instance_token`, upstream `http://{{ .Release.Name }}-v3-core:8086`,
`timeoutMs: 40000`, and `stripPrefix: true`.

For Docker Compose, set `INSIGHT_V3_INGEST_TOKEN` in your untracked
environment file.

## Ingest

```bash
curl --fail-with-body https://insight.example.com/api/core/v1/raw-data \
  -H "X-Insight-Token: ${INSIGHT_V3_INGEST_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data '{"table":"github_issues","raw_data":{"id":1,"state":"open"}}'
```

`204`, no body. A stream's first write creates the table it lands in, so a
connector needs nothing but the token. `table` must match
`^[A-Za-z0-9_]{1,128}$`; `raw_data` is stored whole and is not interpreted.

The request body is capped at 1 MiB and each pod admits 64 concurrent writes.
Responses use canonical problem JSON: 400 for an invalid table name or body,
401 for a missing or invalid token, 413 for an oversized body, 429 for write
capacity, 504 for a database timeout, and 500 for other backend failures.

## Rotate the token

1. Generate a replacement (`openssl rand -hex 32`).
2. Update the `token` key of `insight-v3-core-token` — in GitOps, reseal the
   Secret and commit; otherwise update it in place.
3. Roll the Deployment: `kubectl -n <namespace> rollout restart
   deploy/<release>-v3-core`. The token is env-injected and read only at
   startup, so a Secret change alone leaves running pods on the old value.
4. Hand the new token to every connector.

A release accepts one token. During the rollout old and new pods accept
different tokens, and once the last old pod is gone a connector still holding
the previous token gets 401 — so rotate and re-key connectors in the same
window.

## Access scope

The token grants instance-wide write access to raw-data ingest: any stream, no
user or tenant identity, no per-tenant row filter. It confers no read access —
definitions, metrics and dashboards are session-authenticated at `/api/v3`, and
MCP authorizes separately over OAuth. Distribute it only to connectors that may
write to this instance, and require HTTPS. Revoke by rotating the token or by
setting `global.insightV3Core.deploy: false`.
