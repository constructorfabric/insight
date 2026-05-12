---
name: connector
description: "Create, test, validate, and deploy Insight Connectors"
---

# Connector Skill

Manages the full lifecycle of Insight Connectors: creation, testing, schema generation, validation, and deployment.

## References

Before executing any workflow, read the connector specification:
- **DESIGN**: `docs/domain/connector/specs/DESIGN.md` — mandatory fields, manifest rules, package structure
- **README**: `src/ingestion/README.md` — commands, project structure

## Command Routing

Parse the user's command and route to the appropriate workflow:

| Command | Workflow | Description |
|---------|----------|-------------|
| `/connector create <name>` | [create.md](workflows/create.md) | Create new connector package |
| `/connector test <name>` | [test.md](workflows/test.md) | Test connector (check, discover, read) |
| `/connector schema <name>` | [schema.md](workflows/schema.md) | Generate JSON schema from real data |
| `/connector validate <name>` | [validate.md](workflows/validate.md) | Validate package against spec |
| `/connector build <name>` | Direct | Build CDK connector (Docker → registry/Kind → Airbyte definition) |
| `/connector deploy <name>` | [deploy.md](workflows/deploy.md) | Deploy to Airbyte + Argo |
| `/connector reset <name> <tenant>` | Direct | Delete connection/source/definition, drop Bronze tables, clean state |
| `/connector workflow <name>` | [workflow.md](workflows/workflow.md) | Create/customize Argo workflow templates |
| `/connector logs [job-id\|latest]` | Direct | Show Airbyte job or Argo workflow logs |

## CDK Build

For `/connector build <name>`, run `{INGESTION_DIR}/airbyte-toolkit/build-connector.sh {CONNECTOR_PATH}`. This builds the Docker image, pushes to registry (or loads into Kind for local dev), and registers/updates the Airbyte source definition. Only for `type: cdk` connectors. Use `--push` and `IMAGE_REGISTRY` env var for remote clusters.

## Connector Reset

For `/connector reset <name> <tenant>`, run `{INGESTION_DIR}/airbyte-toolkit/reset-connector.sh {CONNECTOR_NAME} <tenant>`. This deletes the Airbyte connection, source, and definition, drops the Bronze database in ClickHouse, and cleans state files. Use when schema has breaking changes or a full re-sync is needed.

## Airbyte Logs

ALWAYS use `{INGESTION_DIR}/logs.sh` to read Airbyte job logs or Argo workflow logs. NEVER call Airbyte REST API directly for log retrieval.

| Use case | Command |
|----------|---------|
| Airbyte job by ID | `./logs.sh airbyte <job-id>` |
| Latest Airbyte job | `./logs.sh airbyte latest` |
| Argo workflow logs | `./logs.sh <workflow-name\|latest>` |
| Only sync step | `./logs.sh <workflow\|latest> sync` |
| Only dbt step | `./logs.sh <workflow\|latest> dbt` |
| Follow live | `./logs.sh -f <workflow\|latest>` |

ALWAYS run `logs.sh` from `{INGESTION_DIR}` directory with `KUBECONFIG="${KUBECONFIG:-$HOME/.kube/kind-ingestion}"`.

ALWAYS check logs when a sync fails. Workflow failure → check Argo workflow logs first (`./logs.sh <workflow|latest>`), then Airbyte job logs (`./logs.sh airbyte <job-id>`). Common causes: expired credentials in K8s Secret, source API errors, ClickHouse destination unreachable.

## E2E Sync

E2E (end-to-end) sync means running the full pipeline through Argo, not just triggering an Airbyte sync via API. The Argo pipeline includes: Airbyte sync → dbt transformations (Bronze → Silver). Without Argo, dbt models are not executed and Silver tables are not populated.

ALWAYS use `{INGESTION_DIR}/run-sync.sh <connector> <tenant>` for e2e sync. This submits an Argo workflow that runs the complete ingestion pipeline.

ALWAYS use `./logs.sh -f latest` or `./logs.sh latest` to monitor the Argo workflow (which includes both sync and dbt steps).

NEVER consider a raw Airbyte API sync (`/api/v1/connections/sync`) as e2e — it only populates Bronze tables.

| Step | What it does | Tool |
|------|-------------|------|
| Airbyte sync | API → ClickHouse Bronze tables | `run-sync.sh` (step 1) |
| dbt run | Bronze → Silver transformations | `run-sync.sh` (step 2) |
| Full e2e | Both steps via Argo DAG | `./run-sync.sh <connector> <tenant>` |

## Airbyte Architecture

### Shared Destination

ALWAYS use a single shared ClickHouse destination for all connectors. Do NOT create per-connector destinations.

Each connection controls its own Bronze namespace via the `namespaceDefinition` and `namespaceFormat` fields:

| Field | Value | Purpose |
|-------|-------|---------|
| `namespaceDefinition` | `"customformat"` | Use custom namespace |
| `namespaceFormat` | `"bronze_{connector_name}"` | Per-connector ClickHouse database |

The shared destination is configured with a default database (e.g., `default` or `bronze`). Each connection overrides the namespace to route data to the correct Bronze database.

### Connector Credentials via K8s Secrets

Connector credentials are managed via Kubernetes Secrets, not inline in tenant YAML.

**Secret structure**:
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-{connector}-{source-id}       # naming convention
  labels:
    app.kubernetes.io/part-of: insight         # discovery label
  annotations:
    insight.cyberfabric.com/connector: {name}  # matches descriptor.yaml name
    insight.cyberfabric.com/source-id: {id}    # passed as insight_source_id
type: Opaque
stringData:
  {field}: {value}                             # fields from connector.yaml connection_specification
```

**Discovery**: `connect.sh` discovers Secrets by label `app.kubernetes.io/part-of=insight` and reads connector type from annotation `insight.cyberfabric.com/connector`.

**Tenant YAML**: Contains only `tenant_id`. No connector config, no credentials — everything comes from K8s Secrets. `insight_tenant_id` is set from tenant YAML `tenant_id`. `insight_source_id` is set from Secret annotation `insight.cyberfabric.com/source-id`.

**Multi-instance**: Multiple Secrets with the same `connector` annotation create separate Airbyte sources (e.g., two M365 tenants).

**No inline fallback**: If no matching Secret is found, the connector is skipped with an error. All parameters (credentials, config fields like `start_date`) must be in the Secret.

**Per-connector docs**: Each connector's `README.md` documents the required Secret fields. See `src/ingestion/connectors/*/README.md`.

**Local development**: Create `.yaml` files in `src/ingestion/secrets/connectors/` (gitignored) and run `./secrets/apply.sh` to apply them. Secrets must contain ALL connector parameters — there is no inline fallback. See connector READMEs for required Secret fields.

### Airbyte Resource Identity

Scripts identify Airbyte resources (definitions, sources, connections) by UUID from the state file — NEVER by name. Name matching is prohibited.

- ID not in state → create resource, save ID to state
- ID in state but Airbyte returns 404 → delete stale ID, recreate, save new ID
- Never search by name — multiple resources can share the same name
- Existing resources: always update config (credentials may have changed since creation)

### Password Rotation

When rotating ClickHouse password:
1. Update K8s Secret → `./secrets/apply.sh --infra-only`
2. Restart ClickHouse → `kubectl rollout restart deployment/clickhouse -n data` (strategy: Recreate — avoids PVC conflicts)
3. Sync Airbyte destination → `./scripts/connect.sh <tenant>` (updates destination password from Secret)

## Service Credentials

ALWAYS obtain credentials from K8s Secrets, not from hardcoded values or ConfigMaps.

### ClickHouse

| Environment | How to get credentials |
|-------------|----------------------|
| Any cluster | `kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' \| base64 -d` |
| Tenant config | `yq '.destination' {INGESTION_DIR}/connections/<tenant>.yaml` |

Quick test: `kubectl exec -n data deploy/clickhouse -- clickhouse-client --password <password> --query "SELECT currentUser()"`

### Airbyte

| Environment | How to get credentials |
|-------------|----------------------|
| Local (Kind) | API at `http://localhost:8001`, token via `{INGESTION_DIR}/airbyte-toolkit/lib/env.sh` |
| In-cluster | API at `http://airbyte-airbyte-server-svc.airbyte.svc.cluster.local:8001` |
| Any cluster | `source {INGESTION_DIR}/airbyte-toolkit/lib/env.sh` → sets `AIRBYTE_API`, `AIRBYTE_TOKEN`, `WORKSPACE_ID` |

Quick test: `curl -s -H "Authorization: Bearer $AIRBYTE_TOKEN" "$AIRBYTE_API/api/v1/health"`

### Argo

| Environment | How to get credentials |
|-------------|----------------------|
| Local (Kind) | UI at `http://localhost:30500`, no auth |
| Any cluster | `kubectl -n argo port-forward svc/argo-server 2746:2746` then `http://localhost:2746` |

Quick test: `kubectl get workflows -n argo --no-headers | tail -5`

### Argument Parsing

```
/connector <command> <name> [options]

<name>     Connector name (e.g. m365, bamboohr, jira)
           Or full path: collaboration/m365, hr-directory/bamboohr
```

If `<name>` is not a path, search `src/ingestion/connectors/` for it.

ALWAYS use the full relative path (e.g. `collaboration/m365`, not just `m365`) when calling `register.sh` directly — it resolves `connectors/{path}/connector.yaml`.

If `<command>` is omitted, show available commands and existing connectors.

### Context Variables

Set these before routing to workflow:

| Variable | Source | Example |
|----------|--------|---------|
| `CONNECTOR_NAME` | from argument | `m365` |
| `CONNECTOR_PATH` | resolved | `collaboration/m365` |
| `CONNECTOR_DIR` | full path | `src/ingestion/connectors/collaboration/m365` |
| `CONNECTOR_TYPE` | from user input (nocode default) | `nocode` or `cdk` |
| `INGESTION_DIR` | fixed | `src/ingestion` |
