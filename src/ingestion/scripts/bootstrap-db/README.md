# bootstrap-db

Creates all connector bronze tables in ClickHouse without running Airbyte, then promotes them to ReplacingMergeTree and builds the dbt/gold layers. Table schemas come from the connectors themselves (`discover`), so they never drift from what a real sync would create.

How it works: for every connector the source image runs `discover` (every connector's stream schemas are static, so fake credentials work), the resulting catalog is fed to the same `destination-clickhouse` connector Airbyte uses with a zero-record input, which creates every stream table empty.

## Prerequisites

- `docker`, `jq`, `yq` (mikefarah v4)
- `python3.12` or `python3.11` on `PATH` — `run-dbt.sh` builds a local `.venv` with the pinned dbt from it (parity with the toolbox image; dbt-core 1.10 does not run on newer pythons). A `python -m venv`-capable interpreter is required (uv-managed pythons lack `ensurepip`; with those, pre-build the venv via `uv venv --seed .venv && .venv/bin/pip install dbt-core==<pin> dbt-clickhouse==<pin>`).
- ClickHouse reachable under `CLICKHOUSE_HOST` both from this machine (dbt) and from inside docker containers (destination connector). For a ClickHouse running on this machine use the machine's LAN IP (`ipconfig getifaddr en0`) — `host.docker.internal` resolves inside containers but not on the macOS host itself.

## Local ClickHouse for testing

Start a throwaway ClickHouse in docker, on the same version production runs (pinned in `pins.env`, must match the bitnami chart's appVersion in `deploy/gitops/Makefile`):

```bash
source pins.env
docker run -d --name bootstrap-db-clickhouse -p 8123:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight -e CLICKHOUSE_DB=insight \
  -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  "${CLICKHOUSE_SERVER_IMAGE}"
```

`CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1` lets the `insight` admin manage access (`CREATE ROLE`/`CREATE USER`/`GRANT`) so the run provisions the read-only `presentation_ro` role and the grant-less `presentation` user (`provision-presentation-access.sh` → `presentation-role.sql`, #1963/#1964); the official image disables it by default. Both compose stacks (`docker-compose.yml`, `tests/e2e/compose`) and the bitnami prod admin already have access-management, and provisioning is guarded (an admin lacking it is skipped with a warning), so this flag is only needed for this bare throwaway container. The `presentation` user is created only when `CLICKHOUSE_PRESENTATION_PASSWORD` is set (unset in this bare run → role only, which is fine for a snapshot); analytics connects as it in the real stacks.

Point `.env` at it: `CLICKHOUSE_HOST=$(ipconfig getifaddr en0)` (the LAN IP — reachable both for dbt on this machine and for the connector containers; see Prerequisites), `CLICKHOUSE_PORT=8123`, `CLICKHOUSE_PROTOCOL=http`, user/password/database `insight`. Check what got created:

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

2. Review the file. Every required config field gets a fake value, which is all `discover` needs. Should a future connector build its catalog from a live API, replace `value` with `env` to take that field from an environment variable at run time, so secrets never land in the file:

   ```yaml
connectors:
  salesforce:
    path: crm/salesforce
    config:
      insight_source_id:
        value: salesforce-acme-prod
      insight_tenant_id:
        value: fake
      salesforce_client_id:
        value: fake
      salesforce_client_secret:
        value: fake
      salesforce_instance_url:
        value: https://mycompany.my.salesforce.com
      salesforce_start_date:
        value: "2024-01-01"
  example-live-catalog-connector:
    path: category/name
    config:
      api_token:
        env: EXAMPLE_API_TOKEN
   ```

   The file contains no secrets and can be committed to the repository.

3. Copy `.env.bootstrap.example` to `.env` next to the scripts and fill in the values (or export the same variables yourself — the `.env` file is optional).

4. Run everything:

   ```bash
   ./bootstrap-db.sh connectors-config.yaml
   ```

   This creates the tables for every connector in the file (a failing connector is reported and skipped, the run continues), then runs all dbt models, then applies the ClickHouse migrations (`../apply-ch-migrations.sh`).

## Everything from scratch, one block

The full cycle — throwaway ClickHouse, fresh `.env`, bootstrap, snapshot re-dump, field-parity audit, cleanup — as a single copy-paste. No credentials needed: every connector discovers on fake config values. **Overwrites `.env`** next to the scripts.

```bash
cd src/ingestion/scripts/bootstrap-db

source pins.env
docker rm -f bootstrap-db-clickhouse 2>/dev/null
docker run -d --name bootstrap-db-clickhouse -p 8123:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight -e CLICKHOUSE_DB=insight \
  -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  "${CLICKHOUSE_SERVER_IMAGE}"

# The address must work from this machine AND from inside the connector
# containers: macOS → the LAN IP; Linux → the docker bridge gateway.
CH_HOST="$(ipconfig getifaddr en0 2>/dev/null \
  || docker network inspect bridge -f '{{ (index .IPAM.Config 0).Gateway }}')"
cat > .env <<EOF
CLICKHOUSE_HOST=${CH_HOST}
CLICKHOUSE_PORT=8123
CLICKHOUSE_PROTOCOL=http
CLICKHOUSE_USER=insight
CLICKHOUSE_PASSWORD=insight
CLICKHOUSE_DATABASE=insight
EOF

until curl -sf "http://localhost:8123/ping" >/dev/null; do sleep 1; done

./bootstrap-db.sh connectors-config.yaml

set -a; source .env; set +a   # dump-ddl.sh expects the CLICKHOUSE_* vars exported
./dump-ddl.sh              # refresh ../connectors-ddl/*.sql; commit the diff if any
./check-field-parity.py    # exit 1 on field-contract failures

docker rm -f bootstrap-db-clickhouse
```

Roughly 15–20 minutes end to end, most of it connector `discover` pulls and dbt. Skip the last `docker rm` to keep the warehouse around for poking at (`dump-ddl.sh` and `check-field-parity.py` can be re-run against it as long as the container lives).

## Auditing staging → silver field parity

A silver `class_*` model is a `UNION ALL` of every staging model tagged `silver:<target>` (the `union_by_tag` macro). ClickHouse matches UNION branches **by position** and takes the column names from the first branch, so a contributor that renames, reorders or retypes a column does not fail the build — it silently misaligns data or widens the published silver type. The bootstrap warehouse is the only place where every model is materialised at once, which makes it the right place to check that:

```bash
./check-field-parity.py
```

`.github/workflows/connectors-ddl.yml` runs this audit on every PR against `main` that touches `src/ingestion/**` (fork PRs included — every connector discovers on fake values, so the lane needs no secrets) and on every commit that lands on `main`, over a warehouse the lane rebuilds from scratch with `bootstrap-db.sh` — the same run whose re-dump gates the committed snapshot. The push run catches what a PR cannot: two PRs green apart can still leave `main` drifted, since each was validated against its own merge-base.

Like `bootstrap-db.sh`, it sources the `.env` next to it and lets those values win over the inherited environment — a stale `CLICKHOUSE_HOST` exported in the current shell would otherwise beat the file the rest of the pipeline runs on. Pass `--no-env-file` to audit another cluster (say dev) from the exported variables instead.

It reads `system.columns` for the structure and `../../dbt/target/manifest.json` for the staging → silver mapping (that mapping exists only in the dbt tags, not in the database). Every divergence fails the run, with one exception reported as a warning: the target published `Nullable(T)` where this contributor declares a plain `T`. ClickHouse widens like that when another branch is nullable, every value from this branch still fits, and readers already handle NULLs from the other branches. The mirror image — contributor `Nullable(T)` against a target that publishes `T` — is a failure: it means the silver table is not the supertype of its branches (something ALTERed it afterwards, e.g. a contract heal in `apply-ch-migrations.sh`) and NULLs are being coerced on insert. There is no baseline file. Exit code 1 means failures, 2 means usage or connection error; warnings alone exit 0.

The run also fails if a model present in the manifest has no relation in the warehouse: a connector whose `discover` failed would otherwise shrink the comparison silently and the audit would pass for the wrong reason.

Contributors whose physical table is not owned by dbt are covered too. `jira__task_field_history` is an ephemeral pass-through (`SELECT * FROM {{ source(...) }}`) over `staging.jira__task_field_history`, a table written by the `jira-enrich` Rust binary whose DDL lives in the `create_task_field_history_staging` macro — see ADR-003. The audit follows the `source()` dependency to that table and checks it against the silver target, so a future YouTrack twin of the enrich table gets the same field-parity guard for free, provided it keeps the shape: an ephemeral pass-through model carrying the `silver:<target>` tag. An ephemeral model that transforms its input publishes columns no relation holds and is reported as UNCHECKED instead.

## Scripts

| Script | What it does |
|---|---|
| `generate-connectors-config.sh [pattern]` | Finds `descriptor.yaml` files, extracts every required config field from the connector spec, writes the config YAML with fake values to stdout. |
| `seed-connectors.sh <config.yaml>` | Iterates over the config file, resolves `value`/`env` fields into a config JSON, calls `create-connector-tables.sh` per connector. Errors are printed and skipped. |
| `create-connector-tables.sh <connector-dir> <config.json>` | One connector: `discover` → configured catalog → `destination-clickhouse write` with a zero-record stream-status input (creates empty tables) → `dbt run --select <name>__bronze_promoted` (MergeTree → ReplacingMergeTree). |
| `bootstrap-db.sh <config.yaml>` | Sources `pins.env` and `.env` (if present), runs `seed-connectors.sh`, runs all dbt models, runs `../apply-ch-migrations.sh`. |
| `run-dbt.sh [dbt args]` | Helper: generates a profiles.yml from the `CLICKHOUSE_*` variables and runs `dbt run` in `src/ingestion/dbt`. |
| `check-field-parity.py [--manifest PATH]` | Audits every staging contributor against its silver union target (column set, positional order, exact type) plus manifest-vs-warehouse coverage. Same `CLICKHOUSE_*` env contract as the other scripts. Non-zero exit on any finding. |
| `dump-ddl.sh` | Dumps `SHOW CREATE` for every `bronze_*` table, the `person`/`identity`/`silver`/`insight` databases (tables and views), and the gold-referenced `staging` tables into `../connectors-ddl/*.sql` — the committed snapshot that `../create-bronze-placeholders.sh` applies on fresh clusters. **Run it manually** after `bootstrap-db.sh` (see step above) whenever a schema changes, and commit the diff. `.github/workflows/connectors-ddl.yml` re-runs the whole pipeline on every `src/ingestion/**` PR and on every commit to `main`, and fails loudly when the committed snapshot no longer matches, with the full diff inline and as the `connectors-ddl-drift` artifact. Regenerate with the one-block recipe above (no credentials needed) and commit the result. |

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
