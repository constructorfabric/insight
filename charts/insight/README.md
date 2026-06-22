# Insight umbrella chart

Single canonical unit of delivery for the Insight platform.

- **Chart**: `insight`
- **Version**: see `Chart.yaml` → `version`
- **App version**: see `Chart.yaml` → `appVersion` (matches image tags)

## What it contains

The umbrella bundles ONLY the first-party application services. Each is a
local `file://` subchart:

| Component             | Kind                 | Source                                       | Toggle                          |
|-----------------------|----------------------|----------------------------------------------|---------------------------------|
| API Gateway           | app service (req'd)  | `src/backend/services/api-gateway/helm`      | mandatory (no flag)             |
| Analytics API         | app service (req'd)  | `src/backend/services/analytics-api/helm`    | mandatory (no flag)             |
| Frontend (SPA)        | app service (req'd)  | `src/frontend/helm`                          | mandatory (no flag)             |
| Identity (.NET 9)     | app service (opt)    | `src/backend/services/identity/helm`         | `identity.deploy`               |

> Identity requires a populated `persons` table (seeded by `src/backend/services/identity/seed/seed-persons.sh`). Not an OIDC provider. Off by default.

## What it does NOT contain

| Component                          | Why separate                                          | How to install                              |
|------------------------------------|-------------------------------------------------------|---------------------------------------------|
| ClickHouse / MariaDB / Redis / Redpanda (L2 infra) | Operated independently; shared lifecycle / managed services | Separate releases in `insight-infra` (gitops `make system-*`); the umbrella dials them via `<dep>.host` |
| Airbyte                            | Heavy (10+ pods), its own release cadence             | Separate helm release                       |
| Argo Workflows                     | Cluster-scoped infra, often shared across products    | Separate helm release                       |
| Plugins                            | Runtime-managed via UI (not Helm — see architecture)  | Through platform API                        |

See [`docs/distribution/README.md`](../../docs/distribution/README.md) for the full distribution model.

## Release name convention

**This chart assumes release name = `insight`.**

Internal DNS references between app services (e.g. `http://insight-analytics-api:8081`, `http://insight-identity:8082`) are templated with the `insight-` prefix. Helm subcharts use `{{ .Release.Name }}-{chart-suffix}` for service naming, which produces these exact names when the release is `insight`. (External L2 infra is reached via the explicit `<dep>.host` wiring, not the release-name convention.)

If you install under a different name, override all cross-service URLs in your own values.yaml. Prefer sticking to the convention.

## Install (quickstart)

```bash
# 1. Pull & resolve subcharts into charts/insight/charts/
helm dependency update charts/insight

# 2. Dry-run — check that values compose cleanly
helm template insight charts/insight --namespace insight

# 3. Install
helm upgrade --install insight charts/insight \
  --namespace insight --create-namespace \
  -f my-values.yaml \
  --wait --timeout 10m
```

## Install (production checklist)

Before going to prod:

- [ ] Decide on credentials strategy:
  - **Auto-gen (default):** `credentials.autoGenerate: true` — the umbrella creates `insight-db-creds` with random 24-char passwords on first install and reuses them via `lookup` on every upgrade.
  - **BYO / Constructor Platform:** pre-create `insight-db-creds` with all required keys (`clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password`) before the first `helm install`. The umbrella picks them up. Missing/empty keys fail fast.
    - Works regardless of `credentials.autoGenerate`: the chart auto-detects BYO via absence of the `app.kubernetes.io/managed-by=Helm` label on the existing Secret and skips its own Secret-template emission, so Helm never tries to take ownership of the customer-managed Secret. No manual labeling required.
    - **Dry-run note**: `helm install --dry-run` (default, client-side) skips the `lookup` function, so the BYO preview will incorrectly show the chart emitting `insight-db-creds`. Use `helm install --dry-run=server` (Helm ≥3.13) for an accurate BYO sanity-check — it exercises `lookup` against the real cluster.
- [ ] Set OIDC via `apiGateway.oidc.existingSecret` (preferred) or all three of `issuer` + `clientId` + `redirectUri` together. Never inline secrets.
- [ ] Enable ingress + TLS: `apiGateway.ingress`, `frontend.ingress`
- [ ] Bump resources where needed (default `requests` are conservative)
- [ ] Provision the L2 infra (ClickHouse / MariaDB / Redis / Redpanda) out-of-chart and fill `<dep>.host` / `.port` / `.passwordSecret`. App-service URLs follow automatically (resolved by helpers).
- [ ] Set `global.imagePullSecrets` if pulling from a private registry

## Infra wiring

L2 infra (ClickHouse, MariaDB, Redis, Redpanda) is always **external** — deployed out-of-chart as separate releases in `insight-infra` (gitops `make system-*`), or pointed at managed services. The umbrella only carries the wiring it needs to dial them:

- `<dep>.host` is required (the validator / helpers fail fast otherwise). Redpanda uses `<dep>.brokers`.
- `<dep>.port` / `.database` / `.username` as applicable.
- `<dep>.passwordSecret` points at a Secret in the namespace (e.g. `insight-db-creds`) — auto-generated by the umbrella, mirrored by a platform operator, or pre-created (BYO).
- App-service URLs are computed by helpers from `<dep>.host` / `.port`, so no extra overrides are needed.

The umbrella validator (`templates/_helpers.tpl` → `insight.validate`) fails fast on the typical typos: missing `<dep>.host` / `.brokers`, OIDC enabled without `existingSecret` or all inline fields, missing `passwordSecret.{name,key}`.

## Values reference

See comments in [`values.yaml`](./values.yaml) — every block is documented inline.

Key groups:

- `credentials.autoGenerate` — toggle umbrella-managed `insight-db-creds`
- `global.*` — cluster-wide defaults (pull secrets, storage class)
- `<dep>.host` / `<dep>.port` / `<dep>.passwordSecret` (Redpanda: `<dep>.brokers`) — external-infra wiring for ClickHouse, MariaDB, Redis, Redpanda
- `apiGateway` / `analyticsApi` / `frontend` — **mandatory** app services (no deploy-flag; the gateway is the single entrance and the product is one unit)
- `identity.deploy` — **optional** .NET identity service (off by default; not an OIDC provider)
- `apiGateway.oidc` — OIDC configuration (prefer `existingSecret`; inline requires `issuer` + `clientId` + `redirectUri` together)
- `apiGateway.proxy.routes` — reverse-proxy config to downstream services
- `ingestion.templates.enabled` — whether to ship Argo WorkflowTemplates; requires Argo CRDs to be present in the cluster

## Operations

```bash
# Status
helm -n insight status insight
kubectl -n insight get pods -l app.kubernetes.io/part-of=insight

# Upgrade (new appVersion → update image tags via -f values.yaml)
helm upgrade insight charts/insight -n insight -f my-values.yaml

# Rollback
helm -n insight rollback insight <REVISION>

# Uninstall (does NOT delete PVCs for stateful components — cleanup manually)
helm -n insight uninstall insight
kubectl -n insight delete pvc -l app.kubernetes.io/part-of=insight
```

## Publishing (release workflow — not wired up yet)

```bash
# 1. Package
helm package charts/insight -d dist/

# 2. Push to OCI registry (ghcr.io example)
helm push dist/insight-0.1.0.tgz oci://ghcr.io/constructorfabric/charts

# 3. Customer install:
helm upgrade --install insight oci://ghcr.io/constructorfabric/charts/insight \
  --version 0.1.0 \
  --namespace insight --create-namespace \
  -f customer-values.yaml
```

Wire this up in GitHub Actions on tag `v*`. TODO separately.
