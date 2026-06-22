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
| `loki/` | `grafana/loki` | `loki` | not in baseline (single-tenant, no auth) |
| `alloy/` | `grafana/alloy` | `alloy` | not in baseline |
| `grafana/` | `grafana/grafana` | `grafana` | not in baseline (chart auto-gens admin pw; per-env overlay may seal `grafana-creds`) |

### Observability (loki / alloy / grafana)

These three are the bundled observability stack (LGTM, logs first). Two
independent decisions, mirroring the managed-vs-bundled choice for the data
stores above:

1. **Install the bundled stack?** — the `inventory.system.{loki,alloy,grafana}`
   toggles. On = self-host Loki/Alloy/Grafana in `insight-infra`. Off = don't
   (the cluster already runs observability, or stdout is enough).
2. **Where do services export?** — the umbrella's `observability.otlp.endpoint`
   (`environments/<env>/values.yaml`). Point it at this stack's Alloy when the
   toggles are on; at your own collector for an external one; leave it empty
   for stdout-only.

Services ALWAYS log structured JSON to stdout regardless — that is the
product contract; the endpoint only decides where (if anywhere) Insight also
exports OTLP.

**Dashboards.** `system/grafana/values.yaml` provisions two log-based
dashboards into the "Insight" folder: HTTP (request rate by status class,
latency percentiles per route — built on the api-gateway access log) and
Ingestion & deploys (reconcile / airbyte-sync / dbt pod logs, deploy hook
pods, shipped helm output). Deploy markers come from the
`insight-post-upgrade` hook pod; `make deploy-app` additionally ships its
helm log to Loki via `scripts/push-deploy-log.sh` (best-effort).

**Access (follow-up: auth).** The baseline Grafana ships with no ingress and
no SSO — reach it via port-forward:

```shell
kubectl -n insight-infra port-forward svc/grafana 3000:80
# admin password:
kubectl -n insight-infra get secret grafana -o jsonpath='{.data.admin-password}' | base64 -d
# Explore → Loki:
{namespace="insight"}            # service logs
{component="reconcile-loop"}     # reconcile ticks
```

Putting Grafana behind auth is a tracked follow-up: seal a `grafana-creds`
Secret (the optional-secrets helper applies it), then per-env ingress + OIDC
SSO via the existing `insight-oidc` app.

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
