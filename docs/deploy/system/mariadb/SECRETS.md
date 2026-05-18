# MariaDB — Secrets

The Bitnami MariaDB chart in `system/mariadb/values.yaml` references
`auth.existingSecret: mariadb-creds`. That Kubernetes Secret must exist
in the `insight-infra` namespace **before** `make system-mariadb` runs.
The Makefile target enforces this and refuses to install without it.

## Workflow

```bash
# 1. Stage the cleartext Secret manifest in your password manager keyed
#    by the resource name `insight-<env>-mariadb-creds`. The sample
#    `scripts/secret-fetch.sh` stub reads from a local YAML file —
#    swap it for your own integration. See "Cleartext Secret manifest"
#    below for the shape.

# 2. Seal it into the repo (cleartext is streamed via secret-fetch.sh
#    straight into kubeseal — never touches disk):
make seal-secret ENV=<env> NAMESPACE=insight-infra NAME=mariadb-creds

# 3. Commit:
git add environments/<env>/sealed-secrets/insight-infra/mariadb-creds-sealedsecret.yaml
git commit -m "feat(<env>): seal mariadb-creds"

# 4. Install MariaDB. The target applies the sealed manifest and then
#    runs `helm upgrade --install`:
make system-mariadb ENV=<env>
```

## Required keys

The Bitnami chart reads these keys from `mariadb-creds`:

| Key | Used for |
|---|---|
| `mariadb-root-password` | DBA password for the `root` account. |
| `mariadb-password` | Password for the application user (`auth.username` in values.yaml, default `insight`). |
| `mariadb-replication-password` | Required **only** if a per-env overlay sets `architecture: replication`. Omit otherwise. |

If `mariadb-password` is missing, MariaDB starts but the application user
cannot connect — pods reach Ready, then app deploys fail at first query.

## Cleartext Secret manifest

`scripts/secret-fetch.sh` prints whatever your backend returns to
stdout, which is then piped into `kubeseal`. `kubeseal` accepts the
Kubernetes Secret manifest in either YAML or JSON. Single-line JSON is
convenient for password managers whose secret fields strip newlines
(Passbolt, …); plain YAML works fine for stores that preserve them.

YAML (good for vaults / files):

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: mariadb-creds
  namespace: insight-infra
type: Opaque
stringData:
  mariadb-root-password: "REPLACE_ROOT_PASSWORD"
  mariadb-password: "REPLACE_USER_PASSWORD"
```

Single-line JSON (good for single-line password fields):

```json
{"apiVersion":"v1","kind":"Secret","metadata":{"name":"mariadb-creds","namespace":"insight-infra"},"type":"Opaque","stringData":{"mariadb-root-password":"REPLACE_ROOT_PASSWORD","mariadb-password":"REPLACE_USER_PASSWORD"}}
```

Generate the JSON without typing passwords into shell history:

```bash
ROOT_PW="$(openssl rand -base64 24 | tr -d /=+ | head -c 24)"
USER_PW="$(openssl rand -base64 24 | tr -d /=+ | head -c 24)"
jq -cn --arg r "$ROOT_PW" --arg u "$USER_PW" \
  '{apiVersion:"v1",kind:"Secret",
    metadata:{name:"mariadb-creds",namespace:"insight-infra"},
    type:"Opaque",
    stringData:{"mariadb-root-password":$r,"mariadb-password":$u}}'
```

## Rotation

Update the secret in your password manager → `make seal-secret …
NAME=mariadb-creds` → commit → `make system-mariadb ENV=<env>`
(re-applies sealed manifest) → `kubectl -n insight-infra rollout
restart statefulset/mariadb` (the chart doesn't auto-restart on
Secret change).
