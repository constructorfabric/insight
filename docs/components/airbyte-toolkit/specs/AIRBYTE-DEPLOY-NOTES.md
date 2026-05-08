# Airbyte deploy notes (PR #281)

Notes for the operator / platform team on what to change in the Airbyte deployment when upgrading to a chart with this PR. The reconcile loop assumes Airbyte 1.7+ and the chart conventions described below.

## Required Airbyte ConfigMap fixes

These were uncovered while debugging dev-vhc and apply to any cluster whose Airbyte chart was installed before 1.7+ migration.

### `WEBAPP_URL`

Old value: `http://airbyte-airbyte-webapp-svc:80`. The `webapp-svc` Service was removed from Airbyte 1.8 — the webapp is now baked into `airbyte-server-svc`. The stale value is used by the server as the JWT `iss` claim and by some admin UIs as a self-link.

Set in ConfigMap `airbyte-airbyte-env`:

```yaml
data:
  WEBAPP_URL: "http://airbyte-airbyte-server-svc:8001"
```

After the change: `kubectl rollout restart deploy/airbyte-server`.

This should land in the Helm values that drive the bundled Airbyte subchart (or in the operator-supplied Airbyte values file). Without it, OAuth tokens still mint correctly but the JWT iss is misleading and any tooling that follows `WEBAPP_URL` ends up at a non-existent service.

### `API_AUTHORIZATION_ENABLED`

Default `true` on community-edition Airbyte. **Leave it as-is.** Reconcile and the rebuilt Argo workflow templates work with it on. (We briefly flipped it to `false` for diagnostics; restore to `true` before shipping.)

## Required Ingress changes (Airbyte 1.7+)

If the cluster exposes Airbyte through Ingress (rather than only port-forwards), the connector-builder UI requires a separate ingress rule for `/api/v1/connector_builder/*` paths to route to `airbyte-airbyte-connector-builder-server-svc:80`. All other paths continue to go to `airbyte-airbyte-server-svc:8001`.

Reference: <https://docs.airbyte.com/platform/1.7/deploying-airbyte/integrations/ingress-1-7>.

Symptom if missing: the Connector Builder UI shows `Could not validate your connector — Forbidden` and `connector_builder/health` returns 403 from any non-builder-server endpoint. The Insight reconcile loop itself does **not** depend on builder-paths being routed (it talks to server-svc directly inside the cluster), so this only matters for human UI users.

## Required RBAC

The chart now creates `<release>-reconcile` ServiceAccount + Role + RoleBinding ([reconcile-rbac.yaml](../../../charts/insight/templates/ingestion/reconcile-rbac.yaml)). Roles needed:

- `secrets`, `configmaps` — get/list/watch (read airbyte-auth-secrets, connector secrets)
- `argoproj.io/workflows`, `argoproj.io/cronworkflows`, `argoproj.io/workflowtaskresults` — full CRUD (controller writes back results)
- `pods`, `pods/log` — get/watch/patch (Argo executor needs)

Argo workflows triggered by reconcile run under this SA via `serviceAccountName:` set in [sync-trigger.yaml.tpl](../../../src/ingestion/reconcile-connectors/templates/sync-trigger.yaml.tpl) and [cron-workflow.yaml.tpl](../../../src/ingestion/reconcile-connectors/templates/cron-workflow.yaml.tpl).

If the cluster's Argo controller already has its own `argo-workflow-executor` Role, it covers the executor side, but the reconcile SA still needs the secret-read perms — those are not standard Argo executor permissions.

## Argo controller `instanceID`

If the cluster's Argo workflow controller runs with `instanceID:` configured (e.g. dev-vhc has `instanceID: argo-workflows-insight`), set the matching value in Helm:

```yaml
ingestion:
  reconcile:
    argoInstanceId: "argo-workflows-insight"
```

Reconcile labels every Workflow / CronWorkflow it creates with `workflows.argoproj.io/controller-instanceid: <value>` so the controller picks them up. If the controller has no `instanceID` (vanilla install), leave the value empty — reconcile omits the label.

## Toolbox image

Ship with the toolbox image built off this branch. The Dockerfile (`src/ingestion/tools/toolbox/Dockerfile`) `COPY . /ingestion`, so as long as the build is invoked off `claude/strange-euler-6830fe` (or its merge into `main`) the image carries `/ingestion/reconcile-connectors/`.

`ingestion.toolboxImage` Helm value must point at the new tag. The chart does not ship a default tag (toolbox versioning is operator-owned).

Two debug images are already published:
- `ghcr.io/cyberfabric/insight-toolbox:2026.05.08.10.21-82fe1ad`
- `ghcr.io/cyberfabric/insight-toolbox:2026.05.08.10.35-82fe1ad-statfix` (with the GNU-stat ordering fix in `ab_get_token`)

Production deploys should rebuild after merge so the tag reflects the merged SHA.

## What was changed on dev-vhc cluster (manually, NOT via Helm)

The following were applied directly via `kubectl` for debugging. They will revert on the next `helm upgrade` of the relevant subcharts and need to be either persisted via the appropriate Helm values or re-applied:

| Resource | Change | Persistence path |
|---|---|---|
| `cm/airbyte-airbyte-env` | `WEBAPP_URL` → `airbyte-airbyte-server-svc:8001` | Airbyte subchart values (operator) |
| `secret/insight-m365-main` | added `insight_tenant_id`, `insight_source_id` keys | secret manager (operator); these fields are required by the connector manifest spec per ADR-0003 |
| `secret/insight-bamboohr-main` | same | same |
| `sa/insight-reconcile` + Role + RoleBinding | created manually because Helm release was not deployed on dev-vhc | will be auto-created by `helm install` of insight chart |
| `workflowtemplate/airbyte-sync` | replaced with chart's version (with resolve-by-name + OAuth) | `helm install` deploys it |
| (existing) destination `clickhouse` UUID `f715ef29-...` | left in place; reconcile picks it up via name `clickhouse` | next deploy, reconcile auto-creates `clickhouse-bronze` (default name); operator can keep using existing destination by setting `ingestion.reconcile.destinationName: clickhouse` |

## What the operator must configure on a fresh install

After this PR, the only operator-facing configuration for the reconcile loop is:

1. **Connector Secrets** — one K8s Secret per connector under namespace `insight`, label `app.kubernetes.io/part-of=insight`, annotation `insight.cyberfabric.com/connector=<slug>`, `insight.cyberfabric.com/source-id=<slug>-<instance>`. Required keys: connector-specific credentials (e.g. `azure_client_id`, ...) **plus** `insight_tenant_id` and `insight_source_id`.
2. **Tenant ID** — `ingestion.reconcile.tenantId` Helm value.
3. **Toolbox image tag** — `ingestion.toolboxImage` Helm value.
4. **(Conditional) Argo instance-id** — `ingestion.reconcile.argoInstanceId` Helm value, only if the cluster's Argo controller runs with `instanceID:` set.

Everything else (workspace UUID, destination UUID, ServiceAccount name, WorkflowTemplate, RBAC, schedule, ClickHouse connection details) is auto-derived by the chart or auto-resolved by reconcile at runtime.

## Outstanding (separate tickets)

- Toolbox image rebuild + publish off the merge SHA, bump `ingestion.toolboxImage` default in chart values to that tag (matches AppVersion convention) — see `(3)` in PR discussion.
- Counter accuracy: `_RECONCILE_CHANGED` increments on no-op cascade-delete cycles for connectors that have no Airbyte resource and no Secret. Cosmetic; not destructive.
- `ab_create_source` callers swallow rc=22 from `--fail-with-body` curl — separate fix to surface those errors.
- Rotate jwt-signature-secret references out of the chart entirely once nothing reads `jwtSecret` (currently retained for backward compat per ADR-0013).
