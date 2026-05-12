# Ingestion Domain

End-to-end data pipeline from external source APIs to unified Silver tables. Built on Airbyte (extraction), Argo Workflows (orchestration), dbt-clickhouse (transformation), all running in Kubernetes.

## Quick Start

```bash
cd src/ingestion

# 1. Copy and fill tenant credentials
cp connections/example-tenant.yaml.example connections/my-tenant.yaml
# Edit: fill in real API keys

# 2. Start everything
./dev-up.sh

# 3. Run a sync manually
./run-sync.sh m365 my-tenant
```

## Prerequisites

- Docker Desktop
- `kubectl`, `helm`, `kind` (`brew install kubectl helm kind`)

## Commands

### Lifecycle

| Command | What it does |
|---------|-------------|
| `./dev-up.sh` | Start all services. Idempotent — safe to re-run |
| `./dev-down.sh` | Stop all services. Data preserved |
| `./cleanup.sh` | Delete cluster and all data. Asks for confirmation |

### Operations

| Command | What it does |
|---------|-------------|
| `./run-sync.sh <connector> <tenant>` | Run sync + dbt pipeline now |
| `./update-connectors.sh` | Re-upload all connector manifests to Airbyte |
| `./update-connections.sh [tenant]` | Re-apply source + destination + connection configs |
| `./update-workflows.sh [tenant]` | Regenerate and apply CronWorkflows |

### Example

```bash
# Full setup from scratch
./dev-up.sh

# Update M365 connector manifest after editing connector.yaml
./update-connectors.sh

# Update connections after changing tenant config or descriptor.yaml
./update-connections.sh example-tenant

# Update schedules after changing descriptor.yaml
./update-workflows.sh

# Run M365 sync for example-tenant right now
./run-sync.sh m365 example_tenant

# Monitor in Argo UI
open http://localhost:30500
```

## Services

| Service | URL | Credentials |
|---------|-----|-------------|
| Airbyte | http://localhost:8000 | Printed by `dev-up.sh` |
| Argo UI | http://localhost:30500 | No auth (local) |
| ClickHouse | http://localhost:30123 | user: `default`, password: `clickhouse` |

## Configuration

### Connector package

Each connector is a self-contained package:

```
connectors/{category}/{source}/
  connector.yaml              # Airbyte declarative manifest
  descriptor.yaml             # Metadata: schedule, streams, dbt_select
  .env.local                  # Local test credentials (gitignored)
  dbt/
    to_{domain}.sql           # Bronze → Silver transformation
    schema.yml                # Source + model definitions
```

### Tenant config

Tenant credentials live in `connections/`:

```
connections/
  example-tenant.yaml.example  # Template (tracked in git)
  example-tenant.yaml          # Real credentials (gitignored)
  .state/                      # Generated state (gitignored, see airbyte-toolkit/state.yaml)
```

Format:

```yaml
tenant_id: my_tenant

destination:
  type: clickhouse
  host: insight-clickhouse.insight.svc.cluster.local
  port: 8123
  username: default
  password: clickhouse

connectors:
  m365:
    azure_tenant_id: "..."
    azure_client_id: "..."
    azure_client_secret: "..."
```

### Workflow schedules

Shared workflow templates in `workflows/schedules/`:

```yaml
# descriptor.yaml
schedule: "0 2 * * *"       # Cron expression
dbt_select: "+tag:silver"   # dbt selector
workflow: sync               # Template name (sync.yaml.tpl)
```

## Architecture

```
External APIs → Airbyte (4 streams) → ClickHouse Bronze → dbt → Silver
                    ↑                        ↑                    ↑
              Argo Workflows          K8s manifests         toolbox image
              (CronWorkflow)          (Deployment+PVC)      (dbt-clickhouse)
```

All tools run inside K8s via `insight-toolbox` Docker image.

## Documents

| Document | Description |
|----------|-------------|
| [specs/PRD.md](specs/PRD.md) | Product requirements |
| [specs/DESIGN.md](specs/DESIGN.md) | Technical architecture |
| [specs/ADR/0002-argo-over-kestra.md](specs/ADR/0002-argo-over-kestra.md) | Why Argo Workflows |
