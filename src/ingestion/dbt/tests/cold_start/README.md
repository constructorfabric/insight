# Cold start

What a dbt run can build on a warehouse whose connectors have never synced.

Silver is a projection of bronze: a class nobody feeds has no shape to emit, so
it is not built at all. `union_by_tag` already unions whichever tagged
contributors are materialised — this lane covers the decision in front of it,
because dbt fixes its node set before the first model runs and a model body
cannot skip itself. `list_unbuildable_models` names the models whose sources are
not there, and `run-dbt.sh` passes them to `--exclude`.

No connector, no rows, no docker. The suite drops and recreates every database
in the warehouse it points at, so point it at a throwaway one.

## Running it

```sh
# 1. A ClickHouse on the pinned version.
. src/ingestion/scripts/bootstrap-db/pins.env
docker run -d --name cold-start-clickhouse -p 8123:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight -e CLICKHOUSE_DB=insight \
  "${CLICKHOUSE_SERVER_IMAGE}"

# 2. The venv run-dbt.sh builds for itself, so both share one dbt.
python3.12 -m venv src/ingestion/scripts/bootstrap-db/.venv
src/ingestion/scripts/bootstrap-db/.venv/bin/pip install \
  "dbt-core==${DBT_CORE_VERSION}" "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}" \
  -r src/ingestion/dbt/tests/cold_start/requirements.txt

CLICKHOUSE_HOST=127.0.0.1 CLICKHOUSE_HTTP_PORT=8123 \
CLICKHOUSE_USER=insight CLICKHOUSE_PASSWORD=insight CLICKHOUSE_DATABASE=insight \
  src/ingestion/scripts/bootstrap-db/.venv/bin/pytest -q src/ingestion/dbt/tests/cold_start
```

pytest runs from the same venv `run-dbt.sh` uses, which is how the rig finds a
`dbt` to ask for the exclusion list — and what keeps the lane to one dbt install.
