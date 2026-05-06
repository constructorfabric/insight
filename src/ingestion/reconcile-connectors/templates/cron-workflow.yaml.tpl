# Argo CronWorkflow rendered per connector by lib/argo.sh:argo_apply_cronworkflow.
# Variables (consumed by python/render_cronworkflow.py via string.Template /
# envsubst):
#   ${CONNECTOR}         — connector slug (e.g. "github")
#   ${CONNECTION_NAME}   — Airbyte connection name; pattern
#                           {connector}-{source_id}-to-clickhouse-{tenant}
#   ${SCHEDULE}          — cron string; precedence resolved by caller
#                           (Secret annotation > descriptor.schedule > default)
#   ${TENANT}            — tenant slug
#   ${INSIGHT_NAMESPACE} — defaults to "insight" (resolved by env.sh / Helm)
apiVersion: argoproj.io/v1alpha1
kind: CronWorkflow
metadata:
  name: ${CONNECTOR}-${TENANT}-sync
  namespace: ${INSIGHT_NAMESPACE}
  labels:
    app.kubernetes.io/name: insight-reconcile
    app.kubernetes.io/component: connector-sync
    insight.cyberfabric.com/connector: ${CONNECTOR}
    insight.cyberfabric.com/tenant: ${TENANT}
spec:
  schedule: "${SCHEDULE}"
  concurrencyPolicy: Forbid
  startingDeadlineSeconds: 300
  workflowSpec:
    workflowTemplateRef:
      name: airbyte-sync
    arguments:
      parameters:
        - name: connection_name
          value: "${CONNECTION_NAME}"
        - name: connector
          value: "${CONNECTOR}"
        - name: tenant
          value: "${TENANT}"
