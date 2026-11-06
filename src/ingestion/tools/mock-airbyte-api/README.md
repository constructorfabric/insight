# Airbyte API fault proxy

Run this standard-library server from the toolbox image. The local-only Helm
chart and full API reference are in
[the scheduling harness guide](../../../../docs/testing/SCHEDULING-HARNESS.md#airbyte-api-fault-proxy).

For a host process, provide every runtime input explicitly:

```bash
MOCK_HOST=127.0.0.1 MOCK_PORT=8081 MOCK_JOURNAL_RETENTION=1000 MOCK_UPSTREAM_TIMEOUT=300 \
MOCK_CONTROL_TOKEN=synthetic-control-token \
MOCK_AIRBYTE_UPSTREAM=http://127.0.0.1:8001 \
python3 src/ingestion/tools/mock-airbyte-api/server.py
```

Keep the upstream origin pointed directly at Airbyte. All runtime scenarios,
barriers, call counters and journal rows are in memory and disappear on restart.
