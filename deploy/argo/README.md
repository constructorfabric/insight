# Argo Workflows installation for Insight

Argo Workflows is the engine for ingestion pipelines (Airbyte sync → dbt run → enrichment). It is installed as a **standalone Helm release** in the **same namespace** as the rest of Insight (default: `insight`).

Insight services create `CronWorkflow` objects; the Argo controller executes them. `controller.instanceID` scopes workflows to this release, so multiple Insight installs on the same cluster do not interfere with each other.

## Pinned version

| Component | Version |
|-----------|---------|
| Chart     | 0.45.x (pinned in the install script) |

## Install (quickstart)

```bash
./deploy/scripts/install-argo.sh
```

Or manually:
```bash
helm repo add argo https://argoproj.github.io/argo-helm
helm upgrade --install argo-workflows argo/argo-workflows \
  --namespace insight --create-namespace \
  -f deploy/argo/values.yaml \
  --set controller.workflowNamespaces[0]=insight \
  --set controller.instanceID=argo-workflows-insight \
  --wait --timeout 5m

# Supplemental RBAC for the argo-workflow SA (workflowtaskresults, pods, pods/log).
# The rbac.yaml ships placeholders; render them to the target namespace.
sed 's|${NAMESPACE}|insight|g; s|${WORKFLOW_SA}|argo-workflow|g' \
  deploy/argo/rbac.yaml | kubectl apply -f -
```

Override the namespace with `INSIGHT_NAMESPACE=...`.

## Production overrides

On top of [`values.yaml`](./values.yaml), provide your own `values-prod.yaml`:
- HA: `controller.replicas: 2`, workflow archive in Postgres
- `server.sso` with an OIDC client
- Resource limits sized for your workflow volume
- Restrict `controller.parallelism` if the cluster is shared

```bash
EXTRA_VALUES_FILE=deploy/argo/values-prod.yaml \
  ./deploy/scripts/install-argo.sh
```

## Cluster-wide CRDs

Argo Workflows ships cluster-scoped CRDs (`Workflow`, `WorkflowTemplate`, `CronWorkflow`, etc.). On a shared cluster with multiple Insight installs:
- The FIRST install creates the CRDs.
- Subsequent installs should disable CRD install (`--set crds.install=false`) to avoid conflicts. Alternatively, the platform operator installs the CRDs once out-of-band and every Insight release skips them.
- `controller.instanceID` in each release guarantees workflows do not leak between installs even though CRDs are shared.

## WorkflowTemplates

The WorkflowTemplates (`airbyte-sync`, `dbt-run`, `ingestion-pipeline`) are **content**, not infrastructure. They are shipped by the Insight umbrella chart under the `ingestion.templates.enabled: true` flag. After the umbrella is installed they appear in the Insight namespace and can be referenced from `CronWorkflow` objects.

## Verify

```bash
kubectl -n insight get pods -l app.kubernetes.io/name=argo-workflows-server
kubectl -n insight port-forward svc/argo-workflows-server 2746:2746
# UI: http://localhost:2746

# Submit a test workflow
argo -n insight submit --from workflowtemplate/ingestion-pipeline -p connector=m365
```

## Uninstall

```bash
helm -n insight uninstall argo-workflows
sed 's|${NAMESPACE}|insight|g; s|${WORKFLOW_SA}|argo-workflow|g' \
  deploy/argo/rbac.yaml | kubectl delete -f -
# CRDs are cluster-scoped — remove only if no other product on the cluster uses them.
```
