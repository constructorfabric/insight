# Generic sync CronWorkflow template — single-namespace model.
#
# Variables resolved by sync-flows.sh from descriptor.yaml + connection state:
#   CONNECTOR, TENANT_ID, CONNECTION_NAME, SOURCE_ID, DATA_SOURCE, SCHEDULE,
#   DBT_SELECT, DBT_SELECT_STAGING (empty for non-jira), NAMESPACE
#
# CONNECTION_NAME pattern: {connector}-{source_id}-to-clickhouse-{tenant}.
# The airbyte-sync WorkflowTemplate's resolve-connection-by-name init-step
# resolves the UUID at submit time (per ADR-0005 / KEY DECISION #1).
#
# All "infrastructure" parameters (toolbox_image, jira_enrich_image,
# airbyte_url, clickhouse_host/port/user) come from the WorkflowTemplate
# defaults baked into the umbrella chart at install time. We only pass
# connection-specific values here.

apiVersion: argoproj.io/v1alpha1
kind: CronWorkflow
metadata:
  name: ${CONNECTOR}-${TENANT_ID}-sync
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/part-of: ingestion
    tenant: "${TENANT_ID}"
    connector: "${CONNECTOR}"
    # Controller picks up workflows by this label — value MUST match
    # `instanceID` in the argo-workflows-workflow-controller ConfigMap.
    workflows.argoproj.io/controller-instanceid: argo-workflows-insight
spec:
  schedules:
    - "${SCHEDULE}"
  timezone: UTC
  concurrencyPolicy: Replace
  startingDeadlineSeconds: 600
  workflowSpec:
    serviceAccountName: argo-workflow
    entrypoint: run
    templates:
      - name: run
        steps:
          - - name: pipeline
              templateRef:
                name: ingestion-pipeline
                template: pipeline
              arguments:
                parameters:
                  - name: connection_name
                    value: "${CONNECTION_NAME}"
                  - name: insight_source_id
                    value: "${SOURCE_ID}"
                  - name: data_source
                    value: "${DATA_SOURCE}"
                  - name: dbt_select
                    value: "${DBT_SELECT}"
                  - name: dbt_select_staging
                    value: "${DBT_SELECT_STAGING}"
