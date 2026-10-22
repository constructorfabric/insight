# GitHub transformation tests

These tests execute dbt against a disposable ClickHouse and reset its GitHub
Bronze tables. Never point them at a shared database.

Bronze is created by the GitHub and GitHub directory connectors through the
real destination (`tests/bronze.py`), so `docker`, mikefarah `yq` v4 and `jq`
have to be on `PATH`. CI runs the suite in `.github/workflows/github-transform.yml`.

## Run it

A throwaway ClickHouse and the pinned dbt from `scripts/bootstrap-db/pins.env`:

```bash
. src/ingestion/scripts/bootstrap-db/pins.env
docker run -d --name github-dbt-test -p 18125:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \
  -e CLICKHOUSE_DB=insight -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  "$CLICKHOUSE_SERVER_IMAGE"
python3.12 -m venv /tmp/github-dbt-venv
/tmp/github-dbt-venv/bin/pip install "dbt-core==$DBT_CORE_VERSION" \
  "dbt-clickhouse==$DBT_CLICKHOUSE_VERSION" \
  -r src/ingestion/dbt/tests/github/transform/requirements.txt
```

```bash
cd src/ingestion/dbt/tests/github/transform
CLICKHOUSE_HOST=127.0.0.1 CLICKHOUSE_HTTP_PORT=18125 \
CLICKHOUSE_CONTAINER_HOST=host.docker.internal \
CLICKHOUSE_USER=insight CLICKHOUSE_PASSWORD=insight \
GITHUB_TEST_DISPOSABLE=yes \
/tmp/github-dbt-venv/bin/pytest -q
```

`CLICKHOUSE_CONTAINER_HOST` is how the destination's container reaches the same
server: `host.docker.internal` on Docker Desktop, the bridge gateway on Linux
(`docker network inspect bridge -f '{{ (index .IPAM.Config 0).Gateway }}'`).

## Writing a test

A test seeds bronze through `case.insert(table, tenant, row, extracted)` and
builds only the models it reads with `case.build("<selector>")`, ancestors
included (`+model`). Every test starts from empty bronze, and `github__item_events` is
dropped so its first build is a full one.

The scenarios cover two tenants sharing one source id: the item-event
watermark is kept per tenant and source, and a commit e-mail owned in one
tenant stays unowned in another. A bronze row without a tenant still builds,
and the `not_null` and `not_empty` tests on the tenant of the model that reads
it report the row.
