# Argo Workflows installation for Insight

Argo Workflows is the engine for ingestion pipelines (Airbyte sync → dbt run → enrichment). It runs as its own Helm release, separate from the Insight umbrella; Insight services create `CronWorkflow` objects and the Argo controller executes them.

The curated values file at [`values.yaml`](./values.yaml) is the reference consumed by both deployment paths:

- **Local development** — `./dev-up.sh` invokes `deploy/scripts/install-argo.sh` against the local Kind cluster in the `insight` namespace.
- **Cluster deployment** — the private [`infra/insight-gitops`](../../docs/components/deployment/gitops/README.md) repository drives Argo from its `system/argo-workflows/values.yaml` overlay onto the `insight-infra` namespace as part of the L2 system layer.

`controller.instanceID` scopes workflows to the matching Insight install, so multiple installs on the same cluster do not interfere with each other.

## Pinned versions

| Path | Chart version | Source of truth |
|---|---|---|
| `dev-up.sh` | `0.45.x` | `deploy/scripts/install-argo.sh` |
| `infra/insight-gitops` | `1.0.13` | `system/argo-workflows/values.yaml` in the gitops repo |

## Cluster-wide CRDs

Argo Workflows ships cluster-scoped CRDs (`Workflow`, `WorkflowTemplate`, `CronWorkflow`, etc.). On a shared cluster with multiple Insight installs:

- The FIRST install creates the CRDs.
- Subsequent installs should disable CRD install (`--set crds.install=false`) to avoid conflicts. Alternatively, the platform operator installs the CRDs once out-of-band and every Insight release skips them.
- `controller.instanceID` in each release guarantees workflows do not leak between installs even though CRDs are shared.

## WorkflowTemplates

The WorkflowTemplates (`airbyte-sync`, `dbt-run`, `ingestion-pipeline`) are **content**, not infrastructure. They are shipped by the Insight umbrella chart under the `ingestion.templates.enabled: true` flag. After the umbrella is installed they appear in the same namespace as the umbrella and can be referenced from `CronWorkflow` objects.

In the gitops layered model the templates land in `insight` (L3) while Argo's controller runs in `insight-infra` (L2). The controller's `workflowNamespaces` value is set so it watches the `insight` namespace for template references.

## RBAC

The supplemental RBAC (`workflowtaskresults`, `pods`, `pods/log`) is shipped as a placeholder-templated [`rbac.yaml`](./rbac.yaml). `dev-up.sh` renders it via `sed`; the gitops repo renders it via `envsubst` from `bootstrap/argo-rbac.yaml.tmpl` (vendored copy at a known SHA).
