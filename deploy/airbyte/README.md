# Airbyte installation for Insight

Airbyte runs as its own Helm release, separate from the Insight umbrella. The umbrella chart only knows the Airbyte API URL and credentials (see the `airbyte:` block in [`charts/insight/values.yaml`](../../charts/insight/values.yaml)).

The curated values file at [`values.yaml`](./values.yaml) is the reference consumed by both deployment paths:

- **Local development** — `./dev-up.sh` invokes `deploy/scripts/install-airbyte.sh`, which applies `values.yaml` against the local Kind cluster in the `insight` namespace.
- **Cluster deployment** — the private [`infra/insight-gitops`](../../docs/components/deployment/gitops/README.md) repository drives Airbyte from its `system/airbyte/values.yaml` overlay onto the `insight-infra` namespace as part of the L2 system layer.

## Why a separate Helm release

- Airbyte is heavy (10+ pods) and its release cadence does not match Insight's.
- `helm upgrade` on the umbrella must not reinstall Airbyte every time.
- Compatibility: Insight `0.1.x` works against Airbyte chart `1.8.x` (dev-up.sh) and `1.9.x` (gitops L2 in production). The coupling is loose — ingestion templates talk to Airbyte over the stable `/api/v1/` surface, so minor-version drift is safe.

## Namespace model

Two flavours, both supported by the chart:

- **Single-namespace** (`dev-up.sh`): Airbyte, Argo Workflows, and the umbrella all live in `insight`. No cross-namespace service DNS, no Secret mirroring. Multiple Insight installs on a shared cluster simply use different namespaces.
- **Layered (gitops L0/L2/L3)**: Airbyte lives in `insight-infra` (the L2 shared-infra namespace), the umbrella runs in `insight` with `airbyte.deploy: false`. The umbrella's `airbyte.namespace` value (when set; see [#408](https://github.com/cyberfabric/insight/issues/408)) tells the ingestion templates to reach across namespaces.

`controller.instanceID` on Argo scopes workflows to the matching Insight install — even on a shared cluster two tenants never pick up each other's workflows.

## Pinned versions

| Path | Chart version | Source of truth |
|---|---|---|
| `dev-up.sh` | `1.8.5` | `deploy/scripts/install-airbyte.sh` |
| `infra/insight-gitops` | `1.9.2` (per env) | `system/airbyte/values.yaml` in the gitops repo |

Upgrades happen in a dedicated PR with regression tests over the ingestion workflows.

## Integration with Insight

Insight reaches Airbyte via in-namespace DNS (default release name `airbyte`, default namespace `insight`):

```
http://airbyte-airbyte-server-svc.insight.svc.cluster.local:8001
```

This URL is computed by the umbrella's `insight.airbyte.url` helper from `airbyte.releaseName` + `.Release.Namespace`, so changing the release name or namespace propagates automatically. It appears in:

- [`src/ingestion/airbyte-toolkit/lib/env.sh`](../../src/ingestion/airbyte-toolkit/lib/env.sh) → `AIRBYTE_API`
- [`charts/insight/files/ingestion/airbyte-sync.yaml`](../../charts/insight/files/ingestion/airbyte-sync.yaml) → default arg (via placeholder)
- [`charts/insight/values.yaml`](../../charts/insight/values.yaml) → `airbyte.apiUrl` (empty = compute from helpers)

**Auth**: the bearer token is a server-signed JWT signed with `AB_JWT_SIGNATURE_SECRET` from the `airbyte-server` pod. The Airbyte chart creates `airbyte-auth-secrets` in the release namespace; in single-namespace mode Insight shares that namespace so no cross-namespace mirror is needed. In the layered model the workflow templates read the Secret from `insight-infra`.
