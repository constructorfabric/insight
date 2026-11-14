---
name: connector
description: "Create, test, validate, and deploy Insight Connectors. Supports both nocode (declarative YAML) and CDK (Python) connector types. Commands: /connector create <name>, /connector test <name>, /connector schema <name>, /connector deploy <name>, /connector validate <name>."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, WebFetch, WebSearch, Agent
---

# Connector Skill

Manages the full lifecycle of Insight Connectors: creation, testing, schema generation, validation, and deployment.

## References

Before executing any workflow, read:
- **README**: `src/ingestion/README.md` — commands, project structure
- **PATTERNS**: `.cf-studio/config/rules/patterns.md` — descriptor shape, mandatory fields, package structure

The test ladder is L0 static → L1 mock → L2 live smoke; `workflows/test.md` carries the harness.

## Non-negotiable wiring invariants

A connector package can be internally perfect and still be **half-landed** —
missing an entry in a repo-wide file that other machinery iterates. That failure
mode does not show up on the PR, so it cannot be caught by review alone.

| Invariant | Where | Why it bites after merge |
|-----------|-------|--------------------------|
| `descriptor.yaml` `version:` is strict semver `MAJOR.MINOR.PATCH` | descriptor | `bump-descriptors` runs ONLY on push to `main`. `"1.0"` passes the PR, then aborts that job, so `images.<key>.image` is never patched and reconcile WARN+skips the connector forever. |
| Entry in `scripts/bootstrap-db/connectors-config.yaml` | bootstrap registry | Without it `bootstrap-db.sh` never creates `bronze_<snake>` → dbt `Code: 81 UNKNOWN_DATABASE` → `set -e` aborts the run, and every model downstream of the failure is skipped. The datapath suite runs the same file, so the connector's tables never exist there either. |
| Shared `silver:class_<X>` column types identical across sources | staging model | `union_by_tag` UNION ALLs the branches; one mismatched type raises `Code: 386 NO_COMMON_TYPE` and the shared class fails for ALL sources. |
| A data test reading the connector's own tables carries `tags=['connector_quality', '<connector-name>']` | dbt test | No cluster runs a bare `dbt test`, so an UNTAGGED check runs nowhere and the condition it watches goes unreported indefinitely. Tagged `data_quality` instead, it joins the install-wide scheduled catalog and errors on every tenant that lacks the connector. The connector slug is the second half of an INTERSECTION selector — omit it and the check matches nothing. |

ALWAYS run the guard before opening a PR — it is the only one of these that
fails *before* merge (the `Guards` job in CI):

```bash
python3 scripts/ci/connector_wiring.py
```

Details and fixes in [create.md](workflows/create.md) §3.8, which also carries
the two data-check catalogs (`data_quality` vs `connector_quality`) and how to
verify a check's selector resolves.

Do not regenerate `connectors-config.yaml` wholesale — it replaces the
hand-set values in every other connector's entry (URLs, UUIDs, list fields)
with generated placeholders. Generate one fragment:
`cd src/ingestion/scripts/bootstrap-db && ./generate-connectors-config.sh '<category>/<name>'`.

## Command Routing

Parse the user's command and route to the appropriate workflow:

| Command | Workflow | Description |
|---------|----------|-------------|
| `/connector create <name>` | [create.md](workflows/create.md) | Create new connector package |
| `/connector test <name>` | [test.md](workflows/test.md) | Test connector (mock tests, check, discover, read) |
| `/connector schema <name>` | [schema.md](workflows/schema.md) | Generate JSON schema from real data |
| `/connector validate <name>` | [validate.md](workflows/validate.md) | Validate package against spec |
| `/connector build <name>` | Direct | Build the CDK image and register/update its Airbyte source definition; reconcile wires source and connection |
| `/connector deploy <name>` | [deploy.md](workflows/deploy.md) | Deploy to Airbyte + Argo |
| `/connector reset <name> <tenant>` | Direct | Drop and recreate the connector's Airbyte source through reconcile |
| `/connector workflow <name>` | [workflow.md](workflows/workflow.md) | Create/customize Argo workflow templates |
| `/connector logs [job-id\|latest]` | Direct | Show Airbyte job or Argo workflow logs |

## CDK Build

For `/connector build <name>`, run `{INGESTION_DIR}/reconcile-connectors/lib/cdk-build.sh {CONNECTOR_PATH}`. It builds the image named in `descriptor.yaml` `images.cdk.image`, pushes it with `--push` or else loads it into the Kind cluster `${CLUSTER_NAME:-insight}` when one exists, and registers or updates the Airbyte source definition. Then run `./reconcile-connectors.sh --connector {CONNECTOR_NAME}` to wire the source and connection. Only for `type: cdk` connectors.

## Connector Reset

For `/connector reset <name> <tenant>` (breaking schema change, full re-sync): delete the connector's K8s Secret (`kubectl delete secret insight-<name>-<source-id> -n insight`), re-apply it with `./secrets/apply.sh`, then run `./reconcile-connectors.sh` — reconcile drops and recreates the source.

## Airbyte Logs

Read Airbyte job logs and Argo workflow logs with `{INGESTION_DIR}/logs.sh`, not the Airbyte REST API.

| Use case | Command |
|----------|---------|
| Airbyte job by ID | `./logs.sh airbyte <job-id>` |
| Latest Airbyte job | `./logs.sh airbyte latest` |
| Argo workflow logs | `./logs.sh <workflow-name\|latest>` |
| Only sync step | `./logs.sh <workflow\|latest> sync` |
| Only dbt step | `./logs.sh <workflow\|latest> dbt` |
| Follow live | `./logs.sh -f <workflow\|latest>` |

Run `logs.sh` from `{INGESTION_DIR}` with `KUBECONFIG` and `INSIGHT_NAMESPACE` set — the script exits when either is unset.

When a sync fails, read the Argo workflow logs first (`./logs.sh <workflow|latest>`), then the Airbyte job logs (`./logs.sh airbyte <job-id>`). Common causes: expired credentials in K8s Secret, source API errors, ClickHouse destination unreachable.

NEVER trust the Airbyte job status alone — it lies in both directions:
- A job can report `succeeded` with **0 records committed** while the replication workload actually FAILED (e.g. replication pod unschedulable — `FailedScheduling: Insufficient cpu` — or the orchestrator OOM-killed mid-stream).
- A job can stay `running` forever while the source is silently stalled (e.g. SQLite requests-cache explosion on a heavy substream parent); the Argo `poll` step times out (`Failed`) but the orphaned Airbyte job and replication pod keep spinning — cancel the job AND delete the pod.

The real health signal is per-job `aggregatedStats.recordsCommitted` (jobs API) and bronze freshness (`max(_airbyte_extracted_at)` per table) — check those, not the status column. A sync that commits only the first stream (e.g. bitbucket `repositories`) while later substreams show stale `_airbyte_extracted_at` means the replication died mid-job despite the green status.

## E2E Sync

E2E (end-to-end) sync means running the full pipeline through Argo, not just triggering an Airbyte sync via API. The Argo pipeline includes: Airbyte sync → dbt transformations (Bronze → Silver). Without Argo, dbt models are not executed and Silver tables are not populated.

Run an e2e sync with `{INGESTION_DIR}/run-sync.sh <connector> <tenant>`: it submits an Argo workflow that runs the complete ingestion pipeline.

Monitor it with `./logs.sh -f latest` or `./logs.sh latest` — the Argo workflow carries both the sync and the dbt steps.

A raw Airbyte API sync (`/api/v1/connections/sync`) is not e2e: it only populates Bronze tables.

| Step | What it does | Tool |
|------|-------------|------|
| Airbyte sync | API → ClickHouse Bronze tables | `run-sync.sh` (step 1) |
| dbt run | Bronze → Silver transformations | `run-sync.sh` (step 2) |
| Full e2e | Both steps via Argo DAG | `./run-sync.sh <connector> <tenant>` |

## Airbyte Architecture

### Shared Destination

All connectors share one ClickHouse destination; do not create per-connector destinations.

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

**Discovery**: reconcile (`reconcile-connectors.sh`) discovers Secrets by label `app.kubernetes.io/part-of=insight` and reads connector type from annotation `insight.cyberfabric.com/connector`.

**Tenant**: reconcile sets `insight_tenant_id` from `INSIGHT_TENANT_ID`, which the chart's reconcile CronJob fills from `ingestion.reconcile.tenantId`; without it, reconcile falls back to `tenant_id` in the `insight-config` ConfigMap. The local runner's `connections/<tenant>.yaml` holds only `tenant_id` — no connector config, no credentials. `insight_source_id` is set from Secret annotation `insight.cyberfabric.com/source-id`.

**Multi-instance**: Multiple Secrets with the same `connector` annotation create separate Airbyte sources (e.g., two M365 tenants).

**No inline fallback**: If no matching Secret is found, the connector is skipped with an error. All parameters (credentials, config fields like `start_date`) must be in the Secret.

**Per-connector docs**: Each connector's `README.md` documents the required Secret fields. See `src/ingestion/connectors/*/README.md`.

**Local development**: Create `.yaml` files in `src/ingestion/secrets/connectors/` (gitignored) and run `./secrets/apply.sh` to apply them. Secrets must contain ALL connector parameters — there is no inline fallback. See connector READMEs for required Secret fields.

### Airbyte Resource Identity

State lives in Airbyte itself; there is no local state file. Reconcile finds its definitions, sources and connections by the `insight` connection tag and the `app.kubernetes.io/part-of=insight` Secret label, and looks IDs up at runtime through the Airbyte API. When you look a resource up yourself, go by ID or tag, never by name — several resources can share one name.

### Password Rotation

The ClickHouse password is `clickhouse-password` in the `insight-db-creds` Secret of the release namespace (chart value `clickhouse.passwordSecret`). Reconcile writes it into the Bronze destination only when it creates or reconfigures that destination; a changed password alone is not detected (`ab_ensure_destination` skips masked keys), so after a rotation also set the new password on the `clickhouse-bronze` destination in Airbyte.

## Service Credentials

Credentials come from K8s Secrets, not from hardcoded values or ConfigMaps.

### ClickHouse

| Environment | How to get credentials |
|-------------|----------------------|
| Any cluster | `kubectl get secret insight-db-creds -n insight -o jsonpath='{.data.clickhouse-password}' \| base64 -d` |

ClickHouse is not part of the umbrella chart; the chart value `clickhouse.host` names it. Quick test: `clickhouse-client --host <clickhouse.host> --user <clickhouse.username> --password <password> --query "SELECT currentUser()"`

### Airbyte

| Environment | How to get credentials |
|-------------|----------------------|
| Local (Kind) | API at `http://localhost:8001` (port-forward) |
| In-cluster | API at `http://airbyte-airbyte-server-svc.airbyte.svc.cluster.local:8001` |
| Any cluster | `export AIRBYTE_URL=<api-url> INSIGHT_NAMESPACE=insight; source {INGESTION_DIR}/reconcile-connectors/lib/airbyte.sh` → `ab_get_token` prints a bearer token, `ab_workspace_id` the workspace ID (set `AIRBYTE_AUTH_SECRET_NAMESPACE` when Airbyte runs in its own namespace) |

Quick test: `curl -s -H "Authorization: Bearer $(ab_get_token)" "$AIRBYTE_URL/api/v1/health"`

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

Pass the full relative path (e.g. `collaboration/m365`, not just `m365`) to `tools/declarative-connector/source.sh` — it resolves `connectors/{path}/connector.yaml`.

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
