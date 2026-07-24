# bootstrap-db

Creates all connector bronze tables in ClickHouse without running Airbyte, then promotes them to ReplacingMergeTree and builds the dbt/gold layers. Table schemas come from the connectors themselves (`discover`), so they never drift from what a real sync would create.

How it works: for every connector the source image runs `discover` (schemas are static for most connectors, so fake credentials work), the resulting catalog is fed to the same `destination-clickhouse` connector Airbyte uses with a zero-record input, which creates every stream table empty.

## Prerequisites

- `docker`, `jq`, `yq` (mikefarah v4), `dbt` with `dbt-clickhouse`
- ClickHouse reachable under `CLICKHOUSE_HOST` both from this machine (dbt) and from inside docker containers (destination connector). For a ClickHouse running on this machine use `host.docker.internal`.

## Local ClickHouse for testing

Start a throwaway ClickHouse in docker, on the same version production runs (pinned in `pins.env`, must match the bitnami chart's appVersion in `deploy/gitops/Makefile`):

```bash
source pins.env
docker run -d --name bootstrap-db-clickhouse -p 8123:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight -e CLICKHOUSE_DB=insight \
  "${CLICKHOUSE_SERVER_IMAGE}"
```

Point `.env` at it: `CLICKHOUSE_HOST=host.docker.internal`, `CLICKHOUSE_PORT=8123`, `CLICKHOUSE_PROTOCOL=http`, user/password/database `insight` — the host name works both for dbt on this machine and for the connector containers. Check what got created:

```bash
curl -s "http://localhost:8123/" -H "X-ClickHouse-User: insight" -H "X-ClickHouse-Key: insight" \
  --data "SELECT database, name, engine FROM system.tables WHERE database LIKE 'bronze%'"
```

Throw it away with `docker rm -f bootstrap-db-clickhouse`.

## Usage

1. Generate the connectors config (all connectors, or a glob pattern on the connector name or `class/name` path):

   ```bash
   ./generate-connectors-config.sh > connectors-config.yaml
   ./generate-connectors-config.sh 'wiki/*' > wiki-only.yaml
   ./generate-connectors-config.sh 'bitbucket-cloud' > one.yaml
   ```

2. Review the file. Every required config field gets a fake value; that is enough for connectors with static stream schemas. Connectors that build schemas from a live API (`hubspot`, `salesforce`) need real credentials — replace `value` with `env` to take the value from an environment variable at run time, so secrets never land in the file:

   ```yaml
   connectors:
     hubspot:
       path: crm/hubspot
       config:
         hubspot_access_token:
           env: HUBSPOT_ACCESS_TOKEN
         insight_tenant_id:
           value: fake
   ```

   The file contains no secrets and can be committed to the repository.

3. Copy `.env.bootstrap.example` to `.env` next to the scripts and fill in the values (or export the same variables yourself — the `.env` file is optional).

4. Run everything:

   ```bash
   ./bootstrap-db.sh connectors-config.yaml
   ```

   This creates the tables for every connector in the file (a failing connector is reported and skipped, the run continues), then runs all dbt models, then applies the ClickHouse migrations (`../apply-ch-migrations.sh`).

## Scripts

| Script | What it does |
|---|---|
| `generate-connectors-config.sh [pattern]` | Finds `descriptor.yaml` files, extracts every required config field from the connector spec, writes the config YAML with fake values to stdout. |
| `seed-connectors.sh <config.yaml>` | Iterates over the config file, resolves `value`/`env` fields into a config JSON, calls `create-connector-tables.sh` per connector. Errors are printed and skipped. |
| `create-connector-tables.sh <connector-dir> <config.json>` | One connector: `discover` → configured catalog → `destination-clickhouse write` with a zero-record stream-status input (creates empty tables) → `dbt run --select <name>__bronze_promoted` (MergeTree → ReplacingMergeTree). |
| `bootstrap-db.sh <config.yaml>` | Sources `pins.env` and `.env` (if present), runs `seed-connectors.sh`, runs all dbt models, runs `../apply-ch-migrations.sh`. |
| `run-dbt.sh [dbt args]` | Helper: generates a profiles.yml from the `CLICKHOUSE_*` variables and runs `dbt run` in `src/ingestion/dbt`. |
| `dump-ddl.sh` | Dumps `SHOW CREATE` for every `bronze_*` table plus the `silver` and `insight` databases (tables and views) into `../connectors-ddl/*.sql` — the committed snapshot that `../create-bronze-placeholders.sh` applies on fresh clusters. Regenerated automatically by `.github/workflows/connectors-ddl.yml` on PRs. |

## Image pins (pins.env)

`pins.env` is committed and sourced by `bootstrap-db.sh` and CI:

- `CLICKHOUSE_SERVER_IMAGE` — must match production: the appVersion of the bitnami chart pinned as `CLICKHOUSE_VERSION` in `deploy/gitops/Makefile`.
- `DESTINATION_CLICKHOUSE_IMAGE` — must match the ClickHouse destination version your Airbyte installation actually runs. Airbyte seeds connector versions from its registry at install time, so the platform chart version does not determine it; ask the instance:

  ```bash
  curl -s -X POST "${AIRBYTE_URL}/api/v1/destination_definitions/list" \
    -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/json' \
    -d "{\"workspaceId\": \"${WORKSPACE_ID}\"}" \
    | jq -r '.destinationDefinitions[] | select(.name == "ClickHouse") | .dockerImageTag'
  ```

- `SOURCE_DECLARATIVE_MANIFEST_IMAGE` — runtime for nocode (declarative YAML) connectors.
