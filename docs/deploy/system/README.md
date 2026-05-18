# L2 — System Layer

Shared infrastructure services that live in the `insight-infra`
namespace, **one Helm release per service**. Run manually via
`make system-<svc> ENV=<env>` — there is no top-level chain because
each cluster picks which services it self-hosts vs. swaps for managed
external endpoints (RDS, MSK, Confluent Cloud, S3, …) or another
team's infra.

See the top-level [`README.md`](../README.md) for the L0 / L2 / L3 layer
model and full workflow.

## Services

| Directory | Chart | Helm release in `insight-infra` | Needs a Secret? |
|-----------|-------|---------------------------------|-----------------|
| `mariadb/` | `oci://registry-1.docker.io/bitnamicharts/mariadb` | `mariadb` | yes → [SECRETS.md](mariadb/SECRETS.md) |
| `clickhouse/` | `oci://registry-1.docker.io/bitnamicharts/clickhouse` | `clickhouse` | yes → [SECRETS.md](clickhouse/SECRETS.md) |
| `redis/` | `oci://registry-1.docker.io/bitnamicharts/redis` | `redis` | yes → [SECRETS.md](redis/SECRETS.md) |
| `redpanda/` | `redpanda/redpanda` | `redpanda` | not in baseline (TLS/SASL off); per-env overlay may add |
| `redpanda-console/` | `redpanda/console` | `redpanda-console` | not in baseline |
| `airbyte/` | `airbyte/airbyte` | `airbyte` | not in baseline (uses embedded Postgres+MinIO); prod overlay needs S3 creds |
| `argo-workflows/` | `argo/argo-workflows` | `argo-workflows` | not in baseline |

## Values layout

```
system/<svc>/values.yaml                            # shared base — applied to every env
environments/<env>/<svc>-values.yaml                # per-env overlay — created only when an env diverges
```

Both are passed to `helm upgrade --install` in that order. Missing
overlay file = base values used as-is.

## Secret layout

```
environments/<env>/sealed-secrets/insight-infra/<svc>-creds-sealedsecret.yaml
```

Files are sealed against the cluster's sealed-secrets-controller public
cert (`environments/<env>/pub-cert.pem`). Source of truth for the
cleartext is your chosen password manager — `make seal-secret` shells
out to `scripts/secret-fetch.sh` with the resource name
`insight-<env>-<svc>-creds` and pipes the result to `kubeseal`. The
shipped stub reads from a local YAML file; replace it with your own
backend (Vault, 1Password, Bitwarden, AWS Secrets Manager, Passbolt, …).
See each service's `SECRETS.md` for the exact key shape and a paste-able
payload.

`make system-<svc>` enforces: if the Bitnami chart's
`auth.existingSecret` references a Secret that has no sealed manifest
in the repo, the target fails with the exact `make seal-secret …`
command to run and a pointer at this directory. No silent installs
against missing creds.

## Switching to a managed external endpoint

A cluster that uses a managed service (RDS for MariaDB, MSK for
Redpanda, Confluent Cloud, S3, …) simply does NOT run the corresponding
`make system-<svc>` target. Instead, the app layer (umbrella) values
point at the external host:

```yaml
# environments/<env>/values.yaml
mariadb:
  deploy: false
  host: <rds-endpoint>.<region>.rds.amazonaws.com
  port: 3306
  database: insight
  username: insight
  passwordSecret:
    name: insight-db-creds   # still a sealed-secret, in the `insight` namespace
    key:  mariadb-password
```

The umbrella's `mariadb.deploy: false` toggle skips the subchart; the
app reaches the managed endpoint at the host/port supplied.
