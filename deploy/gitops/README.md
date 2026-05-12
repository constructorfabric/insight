# GitOps deployment (ArgoCD)

For enterprise customers whose stack already runs on ArgoCD. Four `Application` manifests deploy the entire stack into **one namespace** (`insight` by default), with sync-wave ordering so Airbyte + Argo are Healthy before Insight starts rolling out.

**Model**: Git is the source of truth; ArgoCD watches the repo and reconciles. Upgrading a version = a commit to that repo.

## Single-namespace model

All Insight components live in one namespace (default `insight`):
- Multiple Insight installs on a shared cluster → different namespaces, each fully self-contained.
- `controller.instanceID` on Argo scopes workflows to the matching install, so tenants don't pick up each other's workflows.
- No cross-namespace DNS, no secret mirroring.

## Prerequisites

- ArgoCD **2.6+** (multi-source support is required for the `$values` pattern used below)
- ArgoCD has access to:
  - `https://airbytehq.github.io/helm-charts` (Airbyte chart repo)
  - `https://argoproj.github.io/argo-helm` (Argo Workflows chart repo)
  - `oci://ghcr.io/cyberfabric/charts` (Insight OCI registry) — or to a Git repo with the chart
  - `https://github.com/cyberfabric/insight.git` (or your fork) for values files and supplemental manifests
- An `AppProject` is created (or use `default`)
- **`insight-db-creds` Secret pre-created** in the release namespace BEFORE the first sync. See "GitOps + auto-gen" below.

## GitOps + auto-generated credentials — IMPORTANT

The umbrella's `credentials.autoGenerate: true` (chart default) uses Helm's `lookup` function to keep `insight-db-creds` stable across upgrades. **ArgoCD renders charts via `helm template`, where `lookup` always returns nil** — so under a GitOps reconcile loop every sync would regenerate `randAlphaNum 24`, overwriting the Secret and rotating passwords on every Argo sync. This breaks every DB client.

For GitOps deployments: the shipped [`insight-values.example.yaml`](./insight-values.example.yaml) sets `credentials.autoGenerate: false`. The operator MUST pre-create `insight-db-creds` with all four required keys:
- `clickhouse-password`
- `mariadb-password`
- `mariadb-root-password`
- `redis-password`

Use the secret-management tool of your choice — ExternalSecrets Operator + Vault, sealed-secrets, SOPS — to keep the values out of Git. The umbrella's BYO validator refuses to install when any required key is missing, empty, or contains URL-reserved characters (`@ : / ? # %`) that would silently corrupt embedded DSNs.

The same caveat applies to OIDC: pre-create `insight-oidc` (referenced as `apiGateway.oidc.existingSecret`) with `issuer`, `audience`, `clientId`, `redirectUri`. The chart fails fast if any field is missing.

## Files

| File | Purpose |
|------|---------|
| [`airbyte-application.yaml`](./airbyte-application.yaml) | Airbyte chart. Sync wave 0. Destination namespace `insight`. |
| [`argo-application.yaml`](./argo-application.yaml) | Argo Workflows chart. Sync wave 0. Destination namespace `insight`. Sets `controller.workflowNamespaces=[insight]` and `controller.instanceID` via Helm parameters. |
| [`argo-rbac-application.yaml`](./argo-rbac-application.yaml) | Supplemental Argo RBAC (`argo-workflow-executor` Role/Binding). Sync wave 0. Uses the pre-rendered [`rbac-insight.yaml`](../argo/rbac-insight.yaml). |
| [`insight-application.yaml`](./insight-application.yaml) | Insight umbrella. Sync wave 1. Destination namespace `insight`. |
| [`insight-values.example.yaml`](./insight-values.example.yaml) | EXAMPLE GitOps overlay — `credentials.autoGenerate: false`, image tags pinned, OIDC via `existingSecret`, ingress hosts as `*.example.com` placeholders. **Copy to `insight-values.yaml`, replace placeholders, do NOT apply as-is.** |
| [`root-app.yaml`](./root-app.yaml) | App-of-Apps: one entry point that manages the four above. |

## Single source of truth

Each infra component has exactly one values file:
- Airbyte: [`deploy/airbyte/values.yaml`](../airbyte/values.yaml)
- Argo Workflows: [`deploy/argo/values.yaml`](../argo/values.yaml)
- Insight: the chart's own [`values.yaml`](../../charts/insight/values.yaml), plus a customer-edited copy of [`insight-values.example.yaml`](./insight-values.example.yaml) for ArgoCD-specific overrides (image tags, OIDC secret, ingress hosts, TLS, `credentials.autoGenerate: false`)

All four Applications reference these files via multi-source (`$values` pattern), so imperative (`deploy/scripts/install-*.sh`) and declarative (ArgoCD) deploys render **identical** manifests.

## Canonical path: App-of-Apps

Use `root-app.yaml` to get the sync-wave ordering between infra and the
umbrella:

```bash
kubectl apply -f deploy/gitops/root-app.yaml
```

`root-app.yaml` makes Airbyte/Argo/Argo-RBAC/Insight **child Applications** of a parent Application. **Only then do ArgoCD sync-wave annotations enforce ordering across them**: Insight (wave 1) waits for Airbyte + Argo + RBAC (wave 0) to become Healthy.

Benefit: the customer applies ONE manifest and everything else is reconciled from Git.

## Alternative: apply the four Applications directly (without ordering)

If you cannot use App-of-Apps, you can apply the four Applications
independently:

```bash
kubectl apply -f deploy/gitops/airbyte-application.yaml
kubectl apply -f deploy/gitops/argo-application.yaml
kubectl apply -f deploy/gitops/argo-rbac-application.yaml
kubectl apply -f deploy/gitops/insight-application.yaml
```

**Caveat:** when Applications are managed directly by the root ArgoCD (not nested under a parent Application), sync-wave annotations **do not order them**. Insight may start syncing before Argo Workflows is Healthy and fail with `no matches for kind "WorkflowTemplate"`. Mitigations:
- Apply infra first, wait for `argocd app get argo-workflows` to report Healthy, then apply `insight-application.yaml`.
- Or set `helm.skipTests: true` + `syncPolicy.syncOptions.SkipDryRunOnMissingResource=true` and rely on ArgoCD's automatic retries.
- Or flip `ingestion.templates.enabled: false` on the Insight Application until Argo CRDs are present, then enable.

The App-of-Apps path avoids all of this.

## Different target namespace

The shipped manifests hardcode `insight` for simplicity. To target a different namespace (for multi-tenant deployments):

1. Fork this repo.
2. Search-and-replace `namespace: insight` (and the `controller.workflowNamespaces[0]=insight`, `controller.instanceID=...` parameters in `argo-application.yaml`) to your chosen namespace.
3. Render a matching `rbac-<ns>.yaml` from `deploy/argo/rbac.yaml` and update `argo-rbac-application.yaml` to reference it.
4. Point each Application's `sources[].repoURL` at your fork.

## Upgrade flow

```bash
# 1. In your fork — bump the chart version
# Portable across BSD/GNU sed: write the result to a tempfile, then
# overwrite. (`sed -i ''` is BSD-only; `sed -i` without an arg is
# GNU-only — there is no portable in-place flag.)
sed 's/targetRevision: 0.1.0/targetRevision: 0.2.0/' insight-application.yaml > insight-application.yaml.tmp \
  && mv insight-application.yaml.tmp insight-application.yaml

# 2. PR → merge → ArgoCD syncs automatically
# (or manual sync via UI / argocd CLI)
```

## Rollback

```bash
# Via the ArgoCD CLI
argocd app rollback insight <REVISION>

# Or git revert
git revert <commit>; git push
```

## Health checks

```bash
argocd app list
argocd app get insight
argocd app get airbyte
argocd app get argo-workflows
argocd app get argo-workflows-rbac
```
