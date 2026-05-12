---
name: connector-workflow
description: "Create or customize Argo Workflow templates for connector pipelines"
---

# Manage Workflows

Create or customize Argo Workflow templates and CronWorkflows.

## Available Workflow Types

| Type | Template | Description |
|------|----------|-------------|
| `sync` | `workflows/schedules/sync.yaml.tpl` | Standard: Airbyte sync → dbt run |
| custom | user-defined | Custom DAG with validation, staging, audit steps |

## Phase 1: Determine Action

- **Create new template**: for pipelines with custom steps (validation, staging area, audit)
- **Customize schedule**: change cron expression in descriptor.yaml
- **Update existing**: modify WorkflowTemplate in `workflows/templates/`

## Phase 2: Template Architecture

Standard templates in `workflows/templates/`:

```
airbyte-sync.yaml         — atomic: trigger Airbyte sync + poll until complete
dbt-run.yaml              — atomic: run dbt in toolbox container
ingestion-pipeline.yaml   — DAG: airbyte-sync → dbt-run
```

### Creating a Custom Pipeline

Example: sync → validate → promote

```yaml
# workflows/templates/validated-pipeline.yaml
apiVersion: argoproj.io/v1alpha1
kind: WorkflowTemplate
metadata:
  name: validated-pipeline
  namespace: argo
spec:
  entrypoint: pipeline
  templates:
    - name: pipeline
      inputs:
        parameters:
          - name: connection_id
          - name: dbt_select
            default: "+tag:silver"
          - name: validation_query
            default: "SELECT count(*) FROM staging.{table} WHERE tenant_id IS NULL"
      dag:
        tasks:
          - name: sync
            templateRef:
              name: airbyte-sync
              template: sync
            arguments:
              parameters:
                - name: connection_id
                  value: "{{inputs.parameters.connection_id}}"
          - name: transform
            depends: "sync"
            templateRef:
              name: dbt-run
              template: run
            arguments:
              parameters:
                - name: dbt_select
                  value: "{{inputs.parameters.dbt_select}}"
          - name: validate
            depends: "transform"
            template: run-validation
            arguments:
              parameters:
                - name: query
                  value: "{{inputs.parameters.validation_query}}"
          - name: promote
            depends: "validate"
            templateRef:
              name: dbt-run
              template: run
            arguments:
              parameters:
                - name: dbt_select
                  value: "tag:silver"

    - name: run-validation
      inputs:
        parameters:
          - name: query
      container:
        image: ghcr.io/cyberfabric/insight-toolbox:latest
        command: ["bash", "-c"]
        args:
          - |
            RESULT=$(curl -sf "http://insight-clickhouse.insight.svc.cluster.local:8123/?user=default&password=${CLICKHOUSE_PASSWORD}" \
              --data "{{inputs.parameters.query}}")
            if [ "$RESULT" != "0" ]; then
              echo "VALIDATION FAILED: $RESULT invalid rows"
              exit 1
            fi
            echo "Validation passed"
```

### Using Custom Pipeline in Descriptor

To use a custom workflow template, set `workflow` in descriptor.yaml:

```yaml
# descriptor.yaml
workflow: validated-pipeline    # matches WorkflowTemplate name
```

And create the template in `workflows/schedules/`:

```yaml
# workflows/schedules/validated-pipeline.yaml.tpl
# Same as sync.yaml.tpl but references validated-pipeline template
```

## Phase 3: Apply

```bash
# Apply workflow templates
kubectl apply -f workflows/templates/ --kubeconfig ~/.kube/kind-ingestion

# Regenerate CronWorkflows
./update-workflows.sh <tenant>
```

## Phase 4: Test

```bash
./run-sync.sh <connector> <tenant>
./logs.sh -f latest
```
