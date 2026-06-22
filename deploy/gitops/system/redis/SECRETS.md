# Redis — Secrets

The Bitnami Redis chart in `system/redis/values.yaml` references
`auth.existingSecret: redis-creds` with
`auth.existingSecretPasswordKey: redis-password`. Required by
`make system-redis`.

## Workflow

```bash
# 1. Stage the cleartext Secret manifest in your password manager keyed
#    by `insight-<env>-redis-creds`. See "Cleartext Secret manifest"
#    below for the shape.

# 2. Seal:
make seal-secret ENV=<env> NAMESPACE=insight-infra NAME=redis-creds

# 3. Commit:
git add environments/<env>/sealed-secrets/insight-infra/redis-creds-sealedsecret.yaml
git commit -m "feat(<env>): seal redis-creds"

# 4. Install:
make system-redis ENV=<env>
```

## Required keys

| Key | Used for |
|---|---|
| `redis-password` | Auth password for the `default` Redis user. |

## Cleartext Secret manifest

YAML:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: redis-creds
  namespace: insight-infra
type: Opaque
stringData:
  redis-password: "REPLACE_REDIS_PASSWORD"
```

Single-line JSON (for stores that strip newlines):

```json
{"apiVersion":"v1","kind":"Secret","metadata":{"name":"redis-creds","namespace":"insight-infra"},"type":"Opaque","stringData":{"redis-password":"REPLACE_REDIS_PASSWORD"}}
```

Generate the JSON without typing the password anywhere visible:

```bash
REDIS_PW="$(openssl rand -base64 24 | tr -d /=+ | head -c 24)"
jq -cn --arg p "$REDIS_PW" \
  '{apiVersion:"v1",kind:"Secret",
    metadata:{name:"redis-creds",namespace:"insight-infra"},
    type:"Opaque",
    stringData:{"redis-password":$p}}'
```

## Rotation

Standard. After `make system-redis`, restart the master pod so the
chart picks up the new password from the updated Secret:

```bash
kubectl -n insight-infra rollout restart statefulset/redis-master
```

analytics-api reconnects on its next request — no app-side restart
needed if it uses connection pooling with retry.
