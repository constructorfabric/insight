---
name: connector-workflow
description: "Create or customize Argo Workflow templates for connector pipelines"
---

# Manage Workflows

Create or customize Argo Workflow templates and CronWorkflows.

## Available Workflow Types

| Type | Template | Description |
|------|----------|-------------|
| `sync` | `reconcile-connectors/templates/cron-workflow.yaml.tpl` → `ingestion-pipeline` | Standard: Airbyte sync → dbt run |

## Phase 1: Determine Action

- **Customize schedule**: change cron expression in descriptor.yaml
- **Update existing**: modify WorkflowTemplate in `charts/insight/templates/ingestion/`

## Phase 2: Template Architecture

Standard templates in `charts/insight/templates/ingestion/` (installed by the umbrella chart):

```
airbyte-sync.yaml         — atomic: trigger Airbyte sync + poll until complete
dbt-run.yaml              — atomic: run dbt in toolbox container
ingestion-pipeline.yaml   — DAG: airbyte-sync → dbt-run
```

### Custom pipelines

Every connector's CronWorkflow targets `ingestion-pipeline` (`reconcile-connectors/templates/cron-workflow.yaml.tpl`). Reconcile does not read the descriptor's `workflow:` field, so setting it does not select a custom template.

## Phase 3: Apply

```bash
# Shared WorkflowTemplates are installed by the umbrella chart on helm install.
# Reconcile renders + applies each connector's CronWorkflow from descriptor.yaml:
./reconcile-connectors.sh --connector <name>
```

## Phase 4: Test

```bash
./run-sync.sh <connector> <tenant>
./logs.sh -f latest
```
