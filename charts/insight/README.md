# Insight umbrella chart

Single canonical unit of delivery for the Insight platform.

- **Chart**: `insight`
- **Version**: see `Chart.yaml` → `version`
- **App version**: see `Chart.yaml` → `appVersion` (matches image tags)

## What it contains

The umbrella bundles ONLY the first-party application services. Each is a
local `file://` subchart:

| Component            | Source                                              | Toggle                       | Default |
|----------------------|-----------------------------------------------------|------------------------------|---------|
| Gateway (edge)       | `src/backend/services/gateway/helm`                 | mandatory (no flag)          | on      |
| Authenticator        | `src/backend/services/authenticator/helm`           | mandatory (no flag)          | on      |
| Analytics            | `src/backend/services/analytics/helm`               | mandatory (no flag)          | on      |
| Identity Resolution  | `src/backend/services/identity-resolution/helm`     | `identityResolution.deploy`  | on — the validator refuses `false` |
| Keycloak (broker)    | `src/backend/services/keycloak/helm`                | `keycloak.deploy`            | off     |
| Previews             | `src/backend/services/previews/helm`                | `previews.deploy`            | on      |
| Git CLI proxy        | `src/backend/services/git-cli-proxy/helm`           | `gitCliProxy.deploy`         | on      |
| Frontend (SPA)       | `src/frontend/helm`                                 | `frontend.deploy`            | on      |

> Identity Resolution is required: the authenticator's login-bootstrap resolve
> exists only there, so `insight.validate` fails the render when
> `identityResolution.deploy` is false. It is not an OIDC provider.

## What it does NOT contain

| Component                          | Why separate                                          | How to install                              |
|------------------------------------|-------------------------------------------------------|---------------------------------------------|
| ClickHouse / MariaDB / Redis / Redpanda (L2 infra) | Operated independently; shared lifecycle / managed services | Separate releases in `insight-infra` (gitops `make system-*`); the umbrella dials them via `<dep>.host` |
| Airbyte                            | Heavy (10+ pods), its own release cadence             | Separate helm release                       |
| Argo Workflows                     | Cluster-scoped infra, often shared across products    | Separate helm release                       |
| Plugins                            | Runtime-managed via UI (not Helm — see architecture)  | Through platform API                        |

See [`deploy/HELM_DEPLOY.md`](../../deploy/HELM_DEPLOY.md) for the full
external-consumer runbook and [`deploy/gitops/README.md`](../../deploy/gitops/README.md)
for the Makefile-driven deployment pipeline.

## Release name convention

**This chart assumes release name = `insight`.**

Internal DNS references between app services (e.g. `http://insight-analytics:8081`, `http://insight-identity-resolution:8082`) are templated with the `insight-` prefix. Helm subcharts use `{{ .Release.Name }}-{chart-suffix}` for service naming, which produces these exact names when the release is `insight`. (External L2 infra is reached via the explicit `<dep>.host` wiring, not the release-name convention.)

If you install under a different name, override all cross-service URLs in your own values.yaml. Prefer sticking to the convention.

## Install

The published artifact is the normal install path:

```bash
helm upgrade --install insight oci://ghcr.io/constructorfabric/charts/insight \
  --version <x.y.z> \
  --namespace insight --create-namespace \
  -f my-values.yaml \
  --wait --timeout 10m
```

Omit `--version` for the latest published release. The full prerequisite list
(Gateway API, OIDC IdP, external L2 systems, required Secrets) is in
[`deploy/HELM_DEPLOY.md`](../../deploy/HELM_DEPLOY.md).

To iterate on the chart from a working copy:

```bash
helm dependency update charts/insight
helm template insight charts/insight --namespace insight   # dry-run render
helm upgrade --install insight charts/insight \
  --namespace insight --create-namespace -f my-values.yaml
```

## Install (production checklist)

Before going to prod:

- [ ] Decide on credentials strategy:
  - **Auto-gen (default):** `credentials.autoGenerate: true` — the umbrella creates `insight-db-creds` with random 24-char passwords on first install and reuses them via `lookup` on every upgrade.
  - **BYO / Constructor Platform:** pre-create `insight-db-creds` with all required keys (`clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password`) before the first `helm install`. The umbrella picks them up. Missing/empty keys fail fast.
    - Works regardless of `credentials.autoGenerate`: the chart auto-detects BYO via absence of the `app.kubernetes.io/managed-by=Helm` label on the existing Secret and skips its own Secret-template emission, so Helm never tries to take ownership of the customer-managed Secret. No manual labeling required.
    - **Dry-run note**: `helm install --dry-run` (default, client-side) skips the `lookup` function, so the BYO preview will incorrectly show the chart emitting `insight-db-creds`. Use `helm install --dry-run=server` (Helm ≥3.13) for an accurate BYO sanity-check — it exercises `lookup` against the real cluster.
  - Rendering under a GitOps controller (`helm template`): set `deploymentMode: gitops` and `autoGenerate: false`, and supply the config Secrets out-of-band — the validator refuses `gitops` + `autoGenerate: true`.
- [ ] Configure OIDC under `authenticator.oidc.*`: `issuerUrl`, `clientId`, `redirectUri`, and the client secret via a Secret (never inline in a committed values file).
- [ ] Provide `insight-authenticator-signing-keys` (ES256 `current.pem`) — not auto-generated.
- [ ] Attach routes to the shared Gateway: `gateway.route`, `frontend.route` (TLS terminates at the Gateway listener)
- [ ] Bump resources where needed (default `requests` are conservative)
- [ ] Provision the L2 infra (ClickHouse / MariaDB / Redis / Redpanda) out-of-chart and fill `<dep>.host` / `.port` / `.passwordSecret`. App-service URLs follow automatically (resolved by helpers).
- [ ] Set `global.imagePullSecrets` if pulling from a private registry

## Infra wiring

L2 infra (ClickHouse, MariaDB, Redis, Redpanda) is always **external** — deployed out-of-chart as separate releases in `insight-infra` (gitops `make system-*`), or pointed at managed services. The umbrella only carries the wiring it needs to dial them:

- `<dep>.host` is required (the validator / helpers fail fast otherwise). Redpanda uses `<dep>.brokers`.
- `<dep>.port` / `.database` / `.username` as applicable.
- `<dep>.passwordSecret` points at a Secret in the namespace (e.g. `insight-db-creds`) — auto-generated by the umbrella, mirrored by a platform operator, or pre-created (BYO).
- App-service URLs are computed by helpers from `<dep>.host` / `.port`, so no extra overrides are needed.

The umbrella validator (`templates/_helpers.tpl` → `insight.validate`) fails fast on the typical misconfigurations: missing `<dep>.host` / `.brokers`, missing `passwordSecret.{name,key}`, `identityResolution.deploy: false`, incomplete `authenticator.oidc` (e.g. `resolveBy: email` without `identityResolution.rosterSourceType`), and the `gitops` + `autoGenerate: true` combination.

## Values reference

See comments in [`values.yaml`](./values.yaml) — every block is documented inline.

Key groups:

- `credentials.deploymentMode` / `credentials.autoGenerate` — who owns the generated Secrets (`helm` with lookup-based reuse, or `gitops` with out-of-band Secrets)
- `global.*` — cluster-wide defaults (pull secrets, storage class, `tenantDefaultId`, `observability.logs.{level,format}`, `observability.otlp.endpoint`)
- `<dep>.host` / `<dep>.port` / `<dep>.passwordSecret` (Redpanda: `<dep>.brokers`) — external-infra wiring for ClickHouse, MariaDB, Redis, Redpanda
- `gateway` / `authenticator` / `analytics` — **mandatory** app services (no deploy flag; the gateway is the single entrance and the product is one unit)
- `authenticator.oidc.*` — OIDC upstream and login-resolution mode (`resolveBy: external_id | email`)
- `identityResolution.*` — identity-resolution service (must stay deployed; `rosterSourceType` for email-mode logins)
- `keycloak.deploy` + `keycloakConfig.*` — the in-stack identity broker and its realms-as-code hook
- `previews.*`, `gitCliProxy.*`, `frontend.*` — optional services, on by default
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

## Publishing

Publishing is automated: the `publish-chart` job in
[`.github/workflows/build-images.yml`](../../.github/workflows/build-images.yml)
packages the umbrella and pushes it to
`oci://ghcr.io/constructorfabric/charts/insight` on every push to `main` and
`release-*`, then commits the `version`/`appVersion` bump back to the branch.
Do not push chart packages by hand.
