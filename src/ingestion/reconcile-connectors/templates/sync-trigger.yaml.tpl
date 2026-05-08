# Argo Workflow (one-shot) rendered by lib/argo.sh:argo_submit_sync_trigger
# on every data-affecting reconcile change (per ADR-0008).
# Variables: ${CONNECTOR} ${CONNECTION_NAME} ${TENANT} ${INSIGHT_NAMESPACE}
#            ${ARGO_INSTANCE_ID}.
# The `controller-instanceid` label is required when the Argo controller
# runs with `instanceID:` set (the standard Insight chart deploys it that
# way); without the matching label, controller ignores the workflow.
# generateName produces a unique name per submit; CronWorkflow above keeps
# its own deterministic name for the recurring schedule.
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: ${CONNECTOR}-${TENANT}-sync-now-
  namespace: ${INSIGHT_NAMESPACE}
  labels:
    app.kubernetes.io/name: insight-reconcile
    app.kubernetes.io/component: connector-sync-trigger
    insight.cyberfabric.com/connector: ${CONNECTOR}
    insight.cyberfabric.com/tenant: ${TENANT}
    insight.cyberfabric.com/trigger-reason: data-affecting-change
    workflows.argoproj.io/controller-instanceid: ${ARGO_INSTANCE_ID}
spec:
  serviceAccountName: ${ARGO_SERVICE_ACCOUNT}
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
