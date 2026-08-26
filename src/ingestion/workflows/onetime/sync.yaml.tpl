# One-shot Workflow that submits ingestion-pipeline for a single connector + tenant.
#
# Variables resolved by run-sync.sh and rendered via envsubst:
#   NAMESPACE, CONNECTOR, TENANT, TENANT_DASHED, CONNECTION_NAME, SOURCE_ID,
#   DATA_SOURCE, DBT_SELECT, DBT_SELECT_STAGING (may be empty for non-jira)
#
# CONNECTION_NAME pattern: {connector}-{source_id}-to-clickhouse-{tenant}.
# The airbyte-sync WorkflowTemplate's resolve-connection-by-name init-step
# resolves the UUID at submit time (per ADR-0005 / KEY DECISION #1).
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: ${CONNECTOR}-${TENANT_DASHED}-
  namespace: ${NAMESPACE}
  labels:
    tenant: "${TENANT}"
    connector: "${CONNECTOR}"
    # Controller picks up workflows by this label — value MUST match
    # the instanceID in the argo-workflows-workflow-controller ConfigMap.
    workflows.argoproj.io/controller-instanceid: argo-workflows-insight
spec:
  # Workflow steps need write access to argoproj.io/workflowtaskresults.
  # The argo chart creates this ServiceAccount via workflow.serviceAccount.create=true;
  # supplemental Role/Binding (provisioned with the Argo system release) grants the necessary verbs.
  serviceAccountName: argo-workflow
  # INVARIANT: `workflowTemplateRef`, not a `templateRef` step.
  #
  # The pipeline's terminal ledger row is written by its `spec.onExit`, and a
  # spec-level field arrives only through this reference. Wrapping the pipeline
  # in a local step instead pulls in that one template and leaves the handler
  # behind, so a manual run — succeeded or failed before its first step — would
  # record no outcome at all (spec FR-1).
  workflowTemplateRef:
    name: ingestion-pipeline
  arguments:
    parameters:
      - name: connection_name
        value: "${CONNECTION_NAME}"
      # Ledger identity, so a manual run is recorded like a scheduled one.
      - name: connector
        value: "${CONNECTOR}"
      - name: tenant_id
        value: "${TENANT}"
      - name: insight_source_id
        value: "${SOURCE_ID}"
      - name: data_source
        value: "${DATA_SOURCE}"
      - name: dbt_select
        value: "${DBT_SELECT}"
      - name: dbt_select_staging
        value: "${DBT_SELECT_STAGING}"
      # No chart-level default (ADR-0016); empty on every non-jira submit,
      # where the step that reads it does not run.
      - name: jira_enrich_image
        value: "${JIRA_ENRICH_IMAGE}"
