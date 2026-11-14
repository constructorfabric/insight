---
name: connector-deploy
description: "Deploy connector to Airbyte + Argo"
---

# Deploy Connector

Registers connector in Airbyte, creates connections, and sets up Argo workflows.

## Prerequisites (ALL mandatory)

- Connector package validated (`/connector validate <name>` passed)
- **Local testing completed**: `source.sh check`, `discover`, `read` all pass
- **Schema generated from real data**: `./tools/declarative-connector/generate-schema.sh <name> <tenant>` run, schemas in manifest
- **All cursor fields exist in schema** (prevents ClickHouse destination NPE)
- Tenant set: in the cluster the chart passes `INSIGHT_TENANT_ID` from `ingestion.reconcile.tenantId`; for a local `./reconcile-connectors.sh` run, export `INSIGHT_TENANT_ID`. Connector K8s Secrets applied with `./secrets/apply.sh`.
- Cluster running (umbrella chart installed — see `charts/insight/`)
- **`descriptor.yaml` `version` bumped whenever `connector.yaml` changed.** On reconcile-managed clusters the nocode manifest is republished ONLY on descriptor-version drift (reconcile compares descriptor `version` against the version stored on the Airbyte definition; equal → noop). A manifest-only PR without a version bump silently never reaches Airbyte — the deploy "succeeds" with the old manifest still active. CI auto-bumps descriptors only for image ref changes, NOT for manifest edits.

## Phase 1: Register Connector and Connections

### Nocode (declarative YAML)

Bump `descriptor.yaml` `version`, then reconcile:

```bash
./reconcile-connectors.sh --dry-run --connector <name>   # preview
./reconcile-connectors.sh --connector <name>
```

### CDK (Python)

```bash
./reconcile-connectors/lib/cdk-build.sh {category}/{name}          # build; load into Kind cluster `insight` if it exists
./reconcile-connectors/lib/cdk-build.sh {category}/{name} --push   # build and push to images.cdk.image
./reconcile-connectors.sh --connector <name>
```

`cdk-build.sh` also registers or updates the Airbyte source definition; reconcile then wires the source and connection.

Reconcile creates, updates and garbage-collects definitions, sources and connections to match the descriptor and the connector's K8s Secret. State lives in Airbyte itself — the descriptor version on the definition, `insight` and `cfg-hash:` tags on the connection — and IDs are looked up at runtime; there is no local state file. Streams are configured `append_dedup` (`normalize_catalog.py`).

## Phase 2: Create Workflows

```bash
./reconcile-connectors.sh --connector <name>
```

Reconcile renders and applies the connector's CronWorkflow from the
`descriptor.yaml` schedule (also done in-cluster by the reconcile CronWorkflow).

## Phase 3: Run First Sync

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
- **`An attempt to write null into not nullable column '<cursor>'`** (every attempt, failure origin `destination`): some records carry no cursor value → stamp a fallback cursor in the connector (create.md §5.6 `null_cursor`), release, let reconcile pick up the image
- **Source config validation error**: definition mismatch → bump `descriptor.yaml` `version`, re-run `./reconcile-connectors.sh --connector <name>`
- **Breaking schema change** (e.g., renamed primary key): drop and recreate the source:
  ```bash
  kubectl delete secret insight-<name>-<source-id> -n insight
  ./secrets/apply.sh
  ./reconcile-connectors.sh
  ./run-sync.sh <name> <tenant>
  ```

## Phase 4: Verify Data

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
