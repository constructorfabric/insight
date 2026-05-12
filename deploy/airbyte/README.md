# Airbyte installation for Insight

Airbyte is installed as a **standalone Helm release** in the **same namespace** as the rest of Insight (default: `insight`). Multiple Insight instances on the same cluster live in different namespaces — each is fully self-contained.

The umbrella chart only knows the Airbyte API URL and credentials. See the `airbyte:` block in [`charts/insight/values.yaml`](../../charts/insight/values.yaml).

## Why separate Helm release

See the architecture notes for the full discussion. In short:
- Airbyte is heavy (10+ pods) and its release cadence does not match Insight's.
- `helm upgrade` on the umbrella must not reinstall Airbyte every time.
- Compatibility matrix: Insight 0.1.x supports Airbyte 1.8.x (pinned to 1.8.5 in the installer). The coupling is loose — ingestion templates talk to Airbyte over the stable `/api/v1/` surface, so minor-version drift is safe. Chart 1.9.x is currently skipped because it ships app 2.0.x-alpha.

## Single-namespace model

All Insight components (Airbyte, Argo Workflows, the umbrella) live in one namespace (default `insight`). Benefits:
- No cross-namespace service DNS, no secret mirroring.
- Multiple Insight installs on a shared cluster simply use different namespaces.
- `controller.instanceID` on Argo scopes workflows to the matching Insight install, so two tenants on the same cluster never pick up each other's workflows.

## Pinned version

| Component   | Version | Status |
|-------------|---------|--------|
| Chart       | 1.8.5   | supported |
| Application | 1.8.5   | matches chart appVersion |

Upgrades happen in a dedicated PR with regression tests over the ingestion workflows.

## Install (quickstart / eval)

```bash
./deploy/scripts/install-airbyte.sh
```

Or manually:
```bash
helm repo add airbyte https://airbytehq.github.io/helm-charts
helm repo update
helm upgrade --install airbyte airbyte/airbyte \
  --namespace insight --create-namespace \
  --version 1.8.5 \
  -f deploy/airbyte/values.yaml \
  --wait --timeout 15m
```

Override the namespace with `INSIGHT_NAMESPACE=...` (for example, when running multiple tenants on the same cluster).

## Install (production)

1. Provision external resources:
   - managed Postgres (RDS / CloudSQL / on-prem) for Airbyte state
   - S3-compatible bucket for logs + state
2. Create Secrets in the Insight namespace:
   ```bash
   kubectl create namespace insight
   kubectl -n insight create secret generic airbyte-db-secret \
     --from-literal=password='...'
   kubectl -n insight create secret generic airbyte-s3-creds \
     --from-literal=AWS_ACCESS_KEY_ID='...' \
     --from-literal=AWS_SECRET_ACCESS_KEY='...'
   ```
3. Create an overrides file (see the commented blocks in [`values.yaml`](./values.yaml)) and save as `values-prod.yaml`.
4. Install:
   ```bash
   helm upgrade --install airbyte airbyte/airbyte \
     --namespace insight --create-namespace \
     --version 1.8.5 \
     -f deploy/airbyte/values.yaml \
     -f deploy/airbyte/values-prod.yaml \
     --wait --timeout 15m
   ```

## Verify

```bash
# Wait for all pods to be ready
kubectl -n insight get pods -l app.kubernetes.io/name=airbyte-server -w

# UI via port-forward
kubectl -n insight port-forward svc/airbyte-airbyte-webapp-svc 8080:80
# → http://localhost:8080

# API reachable
kubectl -n insight port-forward svc/airbyte-airbyte-server-svc 8001:8001
curl http://localhost:8001/api/v1/health
```

## Integration with Insight

Insight reaches Airbyte via in-namespace DNS (default release name `airbyte`, default namespace `insight`):
```
http://airbyte-airbyte-server-svc.insight.svc.cluster.local:8001
```

This URL is computed by the umbrella's `insight.airbyte.url` helper from `airbyte.releaseName` + `.Release.Namespace`, so changing the release name or namespace propagates automatically. It appears in:
- [`src/ingestion/airbyte-toolkit/lib/env.sh`](../../src/ingestion/airbyte-toolkit/lib/env.sh) → `AIRBYTE_API`
- [`charts/insight/files/ingestion/airbyte-sync.yaml`](../../charts/insight/files/ingestion/airbyte-sync.yaml) → default arg (via placeholder)
- [`charts/insight/values.yaml`](../../charts/insight/values.yaml) → `airbyte.apiUrl` (empty = compute from helpers)

**Auth**: the bearer token is a server-signed JWT signed with `AB_JWT_SIGNATURE_SECRET` from the `airbyte-server` pod. The Airbyte chart creates `airbyte-auth-secrets` in the release namespace; because Insight shares that namespace, no cross-namespace mirror is needed — the workflow templates reference the secret directly.

## Uninstall

```bash
helm -n insight uninstall airbyte
# Then remove any Airbyte PVCs (each release leaves its own)
kubectl -n insight get pvc -l app.kubernetes.io/part-of=airbyte -o name | xargs -r kubectl -n insight delete
```
