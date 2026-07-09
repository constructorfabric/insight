"""API endpoint contract tests.

Together these modules exercise EVERY operation in the committed OpenAPI spec
(docs/components/backend/analytics/openapi.json) through the recording client,
so the endpoint-coverage gate needs no SKIP_LIST. One module per path group,
one test per (path, method, status-code) case:

  test_catalog.py            POST /v1/catalog/get_metrics
  test_metrics.py            GET+POST /v1/metrics · GET+PUT+DELETE /v1/metrics/{id}
                             POST /v1/metrics/{id}/query · POST /v1/metrics/queries
  test_metric_thresholds.py  GET+POST /v1/metrics/{id}/thresholds
                             PUT+DELETE /v1/metrics/{id}/thresholds/{tid}
  test_admin_thresholds.py   GET+POST /v1/admin/metric-thresholds
                             GET+PUT+DELETE /v1/admin/metric-thresholds/{id}
  test_columns.py            GET /v1/columns · GET /v1/columns/{table}
  test_persons.py            GET /v1/persons/{email}
  test_metric_results.py     POST /v1/metric-results

Resources come from fixtures (`api/conftest.py`): scratch metric / threshold /
admin-threshold rows created and deleted per test, so the catalog (the
metric-coverage gate's universe) is never touched. Per-op status-code coverage
and the BLOCKED exclusions (auth-disabled 401/403, no-rate-limit 429, and the
#1663/#1664 xfalled bugs) live in lib/api_coverage.py.
"""
