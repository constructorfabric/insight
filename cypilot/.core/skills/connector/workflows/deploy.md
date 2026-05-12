---
name: connector-deploy
description: "Deploy connector to Airbyte + Argo"
---

# Deploy Connector

Registers connector in Airbyte, creates connections, and sets up Argo workflows.

## Prerequisites (ALL mandatory)

- Connector package validated (`/connector validate <name>` passed)
- **Local testing completed**: `source.sh check`, `discover`, `read` all pass
- **Schema generated from real data**: `./airbyte-toolkit/generate-schema.sh <name>` run, schemas in manifest
- **All cursor fields exist in schema** (prevents ClickHouse destination NPE)
- Tenant config (`connections/<tenant>.yaml` with `tenant_id`) and K8s Secrets with credentials
- Cluster running (`./up.sh` completed)

## Phase 1: Register Connector

### Nocode (declarative YAML)

```bash
./update-connectors.sh
```

This updates the existing definition in Airbyte in-place (same definition ID).
If the connector is new, it creates a builder project and publishes a new definition.

### CDK (Python)

```bash
# Local (Kind):
./airbyte-toolkit/build-connector.sh {category}/{name}

# Remote (push to registry):
IMAGE_REGISTRY=ghcr.io/cyberfabric IMAGE_TAG=latest ./airbyte-toolkit/build-connector.sh {category}/{name} --push
```

This builds the Docker image, pushes to registry (or loads into Kind for local dev), and registers/updates the Airbyte source definition.
`update-connectors.sh --all` also auto-detects CDK connectors and delegates to `build-connector.sh`.

**Important**:
- `airbyte-toolkit/register.sh` updates definitions in-place — no duplicate IDs
- `register.sh` auto-detects connector type from `descriptor.yaml` (`type: cdk` vs `nocode`)
- After upload, definition ID is saved to toolkit state (`airbyte-toolkit/state.yaml`)
- All subsequent scripts read IDs from state, not by name lookup

## Phase 2: Create/Update Connections

```bash
./update-connections.sh <tenant>
```

This script is idempotent — it handles both creation and updates:
- **Destination**: creates if missing; updates database config if changed
- **Source**: creates if missing; recreates if definition ID changed (schema update)
- **Connection**: creates if missing; recreates if source was recreated (fresh discover)
- **Discover**: always runs against current source to get real schema with all fields

### Known pitfalls handled by the script

| Pitfall | How handled |
|---------|-------------|
| Duplicate definitions | Fixed: airbyte-toolkit/register.sh updates in-place (same ID) |
| Stale schema after manifest update | Detects definition change in state, recreates source → fresh discover |
| Missing cursor field in schema | Discover from updated definition includes all fields |
| `full_refresh` + `overwrite` = NPE | Always uses `append_dedup` for all streams |
| Name collision (built-in vs custom) | Eliminated: scripts use definition ID from state, not name lookup |

### Airbyte State

All resource IDs are stored in `airbyte-toolkit/state.yaml` (gitignored).
Scripts read/write this file automatically.

On host: `airbyte-toolkit/state.yaml`. In K8s: ConfigMap `airbyte-state` in namespace `data`.

## Phase 3: Create Workflows

```bash
./update-workflows.sh <tenant>
```

Generates CronWorkflow from `descriptor.yaml` schedule.

## Phase 4: Run First Sync

```bash
./run-sync.sh <name> <tenant>
```

Monitor with:
```bash
./logs.sh -f latest
```

If sync fails, get detailed Airbyte logs:
```bash
./logs.sh airbyte latest
```

Common sync failures:
- **NPE getCursor**: cursor field missing from schema → re-run `generate-schema.sh`, update manifest, re-deploy
- **Destination check failed**: ClickHouse database doesn't exist → `airbyte-toolkit/connect.sh` creates it
- **Source config validation error**: definition mismatch → re-upload manifest, re-run `update-connections.sh`
- **Breaking schema change** (e.g., renamed primary key): reset and re-deploy:
  ```bash
  ./airbyte-toolkit/reset-connector.sh <name> <tenant>
  ./airbyte-toolkit/build-connector.sh <path>          # CDK
  ./scripts/apply-connections.sh <tenant>
  ./run-sync.sh <name> <tenant>
  ```

## Phase 5: Verify Data

After sync completes:
```sql
-- Bronze
SELECT count(*) FROM bronze_<name>.<stream>;

-- Check mandatory fields
SELECT tenant_id, source_id, unique_key FROM bronze_<name>.<stream> LIMIT 3;

-- Staging (after dbt)
SELECT count(*) FROM staging.<name>__<domain>;

-- Silver (after dbt)
SELECT count(*) FROM silver.class_<domain>;
```

## Summary

```
=== Deployment: <name> ===

  Connector:  registered in Airbyte (definition updated in-place)
  Destination: bronze_<name> (ClickHouse)
  Connection: <name>-to-clickhouse-<tenant> (N streams, discover-based schema)
  Workflow:   <name>-sync (schedule: 0 2 * * *)
  First sync: PASS (N rows in bronze)
```
