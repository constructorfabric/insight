# Argo CronWorkflow rendered per connector by lib/argo.sh:argo_apply_cronworkflow.
# Variables (consumed by python/render_cronworkflow.py via string.Template /
# envsubst):
#   ${CONNECTOR}            — connector slug (e.g. "github", "ms-entra")
#   ${CONNECTION_NAME}      — Airbyte connection name; pattern
#                              {connector}-{source_id}-{tenant}-conn
#   ${SCHEDULE}             — cron string; precedence resolved by caller
#                              (Secret annotation > descriptor.schedule > default)
#   ${TENANT}               — tenant slug
#   ${INSIGHT_SOURCE_ID}    — secret annotation insight.cyberfabric.com/source-id
#   ${DATA_SOURCE}          — `jira` for the jira-enrich path, else the
#                              connector slug (the pipeline branches on it)
#   ${DBT_SELECT}           — descriptor.dbt_select, e.g. `tag:ms-entra+`
#   ${DBT_SELECT_STAGING}   — only set for jira (data_source==jira); empty
#                              otherwise — the pipeline guards on data_source
#   ${JIRA_ENRICH_IMAGE}    — descriptor.images.enrich.image for jira (per
#                              ADR-0016); empty for connectors without an
#                              enrich step
#                              (the pipeline only invokes tt-enrich-jira-run
#                              when data_source == "jira")
#   ${INSIGHT_NAMESPACE}    — defaults to "insight" (resolved by env.sh / Helm)
#   ${ARGO_INSTANCE_ID}     — required: must match the Argo controller's
#                              `instanceID:` config, otherwise the controller
#                              ignores the workflow.
#   ${ARGO_SERVICE_ACCOUNT} — SA the workflow pods run under (chart provides
#                              {release}-reconcile)
#
# We submit `ingestion-pipeline` (not bare `airbyte-sync`) so the chained
# DAG fires sync → dbt-run (and tt-enrich-jira-run for jira). Otherwise
# Bronze rows would land but Silver / class_* tables would never get
# rebuilt, leaving downstream consumers on stale data.
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
    workflows.argoproj.io/controller-instanceid: ${ARGO_INSTANCE_ID}
spec:
  schedule: "${SCHEDULE}"
  concurrencyPolicy: Forbid
  startingDeadlineSeconds: 300
  workflowSpec:
    serviceAccountName: ${ARGO_SERVICE_ACCOUNT}
    workflowTemplateRef:
      name: ingestion-pipeline
    arguments:
      parameters:
        - name: connection_name
          value: "${CONNECTION_NAME}"
        - name: insight_source_id
          value: "${INSIGHT_SOURCE_ID}"
        - name: data_source
          value: "${DATA_SOURCE}"
        - name: dbt_select
          value: "${DBT_SELECT}"
        - name: dbt_select_staging
          value: "${DBT_SELECT_STAGING}"
        - name: jira_enrich_image
          value: "${JIRA_ENRICH_IMAGE}"
