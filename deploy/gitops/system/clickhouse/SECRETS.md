# ClickHouse — Secrets

The Bitnami ClickHouse chart in `system/clickhouse/values.yaml`
references `auth.existingSecret: clickhouse-creds` with
`auth.existingSecretKey: admin-password`. That Secret must exist in
`insight-infra` before `make system-clickhouse` runs.

## Workflow

```bash
# 1. Stage the cleartext Secret manifest in your password manager keyed
#    by `insight-<env>-clickhouse-creds`. See "Cleartext Secret
#    manifest" below for the shape.

# 2. Seal:
make seal-secret ENV=<env> NAMESPACE=insight-infra NAME=clickhouse-creds

# 3. Commit:
git add environments/<env>/sealed-secrets/insight-infra/clickhouse-creds-sealedsecret.yaml
git commit -m "feat(<env>): seal clickhouse-creds"

# 4. Install:
make system-clickhouse ENV=<env>
```

## Required keys

| Key | Used for |
|---|---|
| `admin-password` | Password for the `auth.username` user (default `insight` per values.yaml). |

`auth.username` lives in values.yaml (not in this Secret) and defaults
to `insight`. Override in `environments/<env>/clickhouse-values.yaml`
if a particular env needs a different username.

## Cleartext Secret manifest

`kubeseal` accepts either YAML or JSON; pick whichever your password
manager stores cleanly. Replace `REPLACE_ADMIN_PASSWORD` with a strong
random password.

YAML:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: clickhouse-creds
  namespace: insight-infra
type: Opaque
stringData:
  admin-password: "REPLACE_ADMIN_PASSWORD"
```

Single-line JSON (for stores that strip newlines):

```json
{"apiVersion":"v1","kind":"Secret","metadata":{"name":"clickhouse-creds","namespace":"insight-infra"},"type":"Opaque","stringData":{"admin-password":"REPLACE_ADMIN_PASSWORD"}}
```

Generate the JSON without typing the password anywhere visible:

```bash
ADMIN_PW="$(openssl rand -base64 24 | tr -d /=+ | head -c 24)"
jq -cn --arg p "$ADMIN_PW" \
  '{apiVersion:"v1",kind:"Secret",
    metadata:{name:"clickhouse-creds",namespace:"insight-infra"},
    type:"Opaque",
    stringData:{"admin-password":$p}}'
```

## Rotation

Update the secret in your password manager → `make seal-secret …
NAME=clickhouse-creds` → commit → `make system-clickhouse ENV=<env>` →
`kubectl -n insight-infra rollout restart statefulset/clickhouse-shard0`
to pick up the new password from the updated Secret.

## Cross-namespace consumption

The umbrella app in the `insight` namespace reads its ClickHouse
password from its own Secret (`insight-db-creds` — see umbrella
`values.yaml`). The two Secrets must carry the SAME password (or the
app cannot authenticate). Stage a second resource
`insight-<env>-db-creds` in your password manager with the **same**
admin-password value and seal it into
`environments/<env>/sealed-secrets/insight/`.
