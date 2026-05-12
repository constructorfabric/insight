# One-shot DAG that runs the Jira tt-enrich path on already-loaded Bronze data:
#   dbt(tag:jira) -> tt-enrich-jira-run -> dbt(tag:silver,tag:jira+).
#
# Variables resolved by run-tt-enrich-jira.sh and rendered via envsubst:
#   NAMESPACE, TENANT, TENANT_DASHED, SOURCE_ID
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: jira-${TENANT_DASHED}-tt-enrich-
  namespace: ${NAMESPACE}
  labels:
    tenant: "${TENANT}"
    connector: "jira"
    workflow-kind: "tt-enrich"
    # Controller picks up workflows by this label — value MUST match
    # the instanceID in the argo-workflows-workflow-controller ConfigMap.
    workflows.argoproj.io/controller-instanceid: argo-workflows-insight
spec:
  serviceAccountName: argo-workflow
  entrypoint: run
  templates:
    - name: run
      dag:
        tasks:
          - name: staging-jira
            templateRef:
              name: dbt-run
              template: run
            arguments:
              parameters:
                - name: dbt_select
                  value: "tag:jira"

          - name: enrich
            depends: staging-jira
            templateRef:
              name: tt-enrich-jira-run
              template: run
            arguments:
              parameters:
                - name: insight_source_id
                  value: "${SOURCE_ID}"

          - name: silver
            depends: enrich
            templateRef:
              name: dbt-run
              template: run
            arguments:
              parameters:
                - name: dbt_select
                  value: "tag:silver,tag:jira+"
