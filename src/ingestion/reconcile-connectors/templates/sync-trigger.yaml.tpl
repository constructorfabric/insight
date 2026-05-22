# Argo Workflow (one-shot) rendered by lib/argo.sh:argo_submit_sync_trigger
# on every data-affecting reconcile change (per ADR-0008).
# Variables (consumed by python/render_sync_trigger.py via string.Template):
#   ${CONNECTOR}            — connector slug
#   ${CONNECTION_NAME}      — Airbyte connection name; pattern
#                              {connector}-{source_id}-{tenant}-conn
#   ${TENANT}               — tenant slug
#   ${INSIGHT_SOURCE_ID}    — secret annotation insight.cyberfabric.com/source-id
#   ${DATA_SOURCE}          — `jira` for the jira-enrich path, else the
#                              connector slug
#   ${DBT_SELECT}           — descriptor.dbt_select
#   ${DBT_SELECT_STAGING}   — only set for jira; empty otherwise
#   ${DBT_FULL_REFRESH}     — "true" iff bump_kind == "major" on this run
#                              (per ADR-0015); else "false". One-shot only.
#   ${JIRA_ENRICH_IMAGE}    — descriptor.images.enrich.image for jira (per
#                              ADR-0016). Empty for non-jira; the
#                              tt-enrich-jira-run task is only invoked when
#                              DATA_SOURCE == "jira" anyway.
#   ${INSIGHT_NAMESPACE}    — release namespace
#   ${ARGO_INSTANCE_ID}     — controller-instanceid label (optional;
#                              empty drops the label)
#   ${ARGO_SERVICE_ACCOUNT} — SA the workflow pods run under
#
# Submits `ingestion-pipeline` (not bare `airbyte-sync`) so the
# chained DAG fires sync → dbt-run (and tt-enrich-jira-run for jira)
# after a data-affecting reconcile change. generateName produces a
# unique name per submit.
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
      - name: dbt_full_refresh
        value: "${DBT_FULL_REFRESH}"
      - name: jira_enrich_image
        value: "${JIRA_ENRICH_IMAGE}"
