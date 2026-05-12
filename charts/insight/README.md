# Insight umbrella chart

Single canonical unit of delivery for the Insight platform.

- **Chart**: `insight`
- **Version**: see `Chart.yaml` → `version`
- **App version**: see `Chart.yaml` → `appVersion` (matches image tags)

## What it contains

| Component             | Kind                 | Source                                       | Toggle                          |
|-----------------------|----------------------|----------------------------------------------|---------------------------------|
| ClickHouse            | infra                | `helmfile/charts/clickhouse` (local wrapper) | `clickhouse.deploy`             |
| MariaDB               | infra                | bitnami/mariadb ~20                          | `mariadb.deploy`                |
| Redis                 | infra                | bitnami/redis ~21                            | `redis.deploy`                  |
| Redpanda              | infra                | redpanda/redpanda ~5                         | `redpanda.deploy`               |
| API Gateway           | app service (req'd)  | `src/backend/services/api-gateway/helm`      | mandatory (no flag)             |
| Analytics API         | app service (req'd)  | `src/backend/services/analytics-api/helm`    | mandatory (no flag)             |
| Frontend (SPA)        | app service (req'd)  | `src/frontend/helm`                          | mandatory (no flag)             |
| Identity Resolution   | app service (opt)    | `src/backend/services/identity/helm`         | `identityResolution.deploy`     |

> Identity Resolution is a C# stub that requires populated bronze data; it is **not** an OIDC provider. Off by default.

## What it does NOT contain

| Component        | Why separate                                          | How to install                 |
|------------------|-------------------------------------------------------|--------------------------------|
| Airbyte          | Heavy (10+ pods), its own release cadence             | Separate helm release          |
| Argo Workflows   | Cluster-scoped infra, often shared across products    | Separate helm release          |
| Plugins          | Runtime-managed via UI (not Helm — see architecture)  | Through platform API           |

See [`docs/distribution/README.md`](../../docs/distribution/README.md) for the full distribution model.

## Release name convention

**This chart assumes release name = `insight`.**

Internal DNS references (e.g. `http://insight-analytics-api:8081`, `http://insight-clickhouse:8123`) are hardcoded in `values.yaml` with the `insight-` prefix. Helm subcharts use `{{ .Release.Name }}-{chart-suffix}` for service naming, which produces these exact names when the release is `insight`.

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
- [ ] `redpanda.tls.enabled: true`, `redpanda.auth.sasl.enabled: true`
- [ ] Point MariaDB/ClickHouse/Redis/Redpanda to external managed services if running inside Constructor Platform — set `<dep>.deploy: false` and fill `<dep>.host` / `.port` / `.passwordSecret`. App-service URLs follow automatically (resolved by helpers).
- [ ] Set `global.imagePullSecrets` if pulling from a private registry

## Integration modes

The chart uses ONE unified shape per infra dependency (ClickHouse, MariaDB, Redis, Redpanda). The `deploy` flag toggles whether the umbrella runs the subchart; everything else (host, port, credentials) is the same data the consumers read in either case.

**Standalone** (eval, on-prem single-tenant, dev):
- `<dep>.deploy: true` — the umbrella runs the subchart.
- `<dep>.host: ""` — defaults to `{release}-<dep>` (internal in-cluster service).
- `<dep>.passwordSecret` points at `insight-db-creds`, which the umbrella auto-generates on first install (or you pre-create for BYO).

**Constructor Platform component** (Insight ships inside the platform):
- `<dep>.deploy: false` — the umbrella does NOT run the subchart.
- `<dep>.host` is required (validator fails fast otherwise).
- `<dep>.passwordSecret` points at a Secret the platform created in the namespace.
- App-service URLs are computed by helpers from the same `<dep>.host` / `.port`, so no extra overrides are needed.

The umbrella validator (`templates/_helpers.tpl` → `insight.validate`) fails fast on the typical typos: `deploy: false` without `host`, OIDC enabled without `existingSecret` or all inline fields, missing `passwordSecret.{name,key}`.

## Values reference

See comments in [`values.yaml`](./values.yaml) — every block is documented inline.

Key groups:

- `credentials.autoGenerate` — toggle umbrella-managed `insight-db-creds`
- `global.*` — cluster-wide defaults (pull secrets, storage class, bitnami image policy)
- `<dep>.deploy` / `<dep>.host` / `<dep>.port` / `<dep>.passwordSecret` — unified shape for ClickHouse, MariaDB, Redis, Redpanda
- `apiGateway` / `analyticsApi` / `frontend` — **mandatory** app services (no deploy-flag; the gateway is the single entrance and the product is one unit)
- `identityResolution.deploy` — **optional** identity-resolution service (off by default; not an OIDC provider)
- `apiGateway.oidc` — OIDC configuration (prefer `existingSecret`; inline requires `issuer` + `clientId` + `redirectUri` together)
- `apiGateway.proxy.routes` — reverse-proxy config to downstream services
- `ingestion.templates.enabled` — whether to ship Argo WorkflowTemplates; requires Argo CRDs to be present in the cluster

## Bitnami Legacy images — maintenance model

In late 2025 Bitnami removed free image distribution from `docker.io/bitnami/*` ([bitnami/charts#30850](https://github.com/bitnami/charts/issues/30850)) and moved unsupported tags to a `docker.io/bitnamilegacy/*` namespace. The umbrella points the bundled MariaDB and Redis subcharts at `bitnamilegacy` so the eval / on-prem path keeps working without a paid Bitnami subscription. This is documented inline in `values.yaml` (`mariadb.image.repository`, `redis.image.repository`).

**Ownership and cadence.** Insight maintainers own the upgrade cadence for these images. The chart `~20.0.0` / `~21.0.0` constraints allow patch-level (CVE) bumps, but **minor releases require an explicit chart edit** — minors can carry breaking changes that need verification.

- **CVE-driven bumps**: tracked via Renovate against `bitnamilegacy/mariadb` and `bitnamilegacy/redis` tags; a critical CVE in either image opens a PR within 24h.
- **Routine bumps**: scheduled monthly review of the `~MAJOR.0.0` constraint window. Minor-version bumps (e.g. `mariadb 20.x → 21.x`) ship in a dedicated PR with regression tests.
- **Upstream deprecation risk**: if Bitnami deprecates `bitnamilegacy/*` (no announced timeline as of 2026-04), Insight will mirror the last-good tags into a self-hosted registry and update `image.repository` / `image.registry` in `values.yaml`.

**Enterprise customers** with a Bitnami subscription or an internal mirror should override the registry once in their values overlay:

```yaml
mariadb:
  image:
    registry: registry.internal.example.com
    repository: my-mirror/mariadb
redis:
  image:
    registry: registry.internal.example.com
    repository: my-mirror/redis
```

…and unset `global.security.allowInsecureImages` (the Bitnami chart's `secure-images` allowlist will accept your registry once images come from a non-`bitnamilegacy` path).

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

## Subchart prerequisites (done as part of this change)

To make the umbrella compose cleanly, two subcharts were patched:

- `helmfile/charts/clickhouse` — added `clickhouse.fullname` helper so the Service is named `<release>-clickhouse`, not just `<release>`.
- `src/frontend/helm` — changed `insight-frontend.fullname` to append `-frontend`, so it doesn't collide with other resources that use bare `{release}`.

Both charts remain compatible with the existing Helmfile (`helmfile sync`) — they add a suffix that wasn't there before, so service names under Helmfile become `clickhouse-clickhouse` / `frontend-frontend`. If Helmfile references old names anywhere, update those too.

## Relationship to `helmfile.yaml.gotmpl`

| Concern           | `helmfile` (dev)                   | umbrella (distribution)            |
|-------------------|------------------------------------|------------------------------------|
| Audience          | Developers                          | Customers / GitOps                 |
| Invocation        | `helmfile -e local sync`            | `helm install insight charts/insight` |
| Templating        | gotmpl DSL                          | Pure Helm + YAML                   |
| Secret injection  | `.env.local` → helmfile vars        | `existingSecret` references        |
| Publishing        | Not published                       | OCI registry (`helm push`)          |

They coexist. Devs keep using Helmfile locally for fast iteration; distribution goes through the umbrella.

## Publishing (release workflow — not wired up yet)

```bash
# 1. Package
helm package charts/insight -d dist/

# 2. Push to OCI registry (ghcr.io example)
helm push dist/insight-0.1.0.tgz oci://ghcr.io/cyberfabric/charts

# 3. Customer install:
helm upgrade --install insight oci://ghcr.io/cyberfabric/charts/insight \
  --version 0.1.0 \
  --namespace insight --create-namespace \
  -f customer-values.yaml
```

Wire this up in GitHub Actions on tag `v*`. TODO separately.
