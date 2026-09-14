# Notes — Insight v3

## Areas

1. Data ingestion
2. Identity resolution
3. Access control
4. Create metrics, widgets and dashboards
5. AI (including answer one time questions)
6. Review data ingestion status
7. Review platform usage
8. Alerts
9. Query optimization
10. Download data from Insight (API, manually in reports)

## MVP check

Built from scratch in `insight-v3-core`. Nothing from analytics.

- [x] 0. Pick the data — synthetic JSON events
- [x] 1. Send it to the DB — `POST /v1/raw-data` (exists)
- [x] 2. Create a metric in JSON, at runtime
- [x] 3. Create two widgets in JSON, at runtime — one table, one graph
- [x] 4. Create a dashboard in JSON
- [x] 5. Render it at `/portal/custom/{dashboard-name}`
- [x] 6. AI chat in the UI: answers a one-time question from the data, and does 2, 3 and 4 from a message

### Decided

- Metrics, widgets and dashboards are JSON, so a new one is added without a code change.
- One table per kind: `metrics`, `widgets`, `dashboards`.
- A widget names a metric; the metric carries the query over `raw_data`.
- The chat calls the model directly from `insight-v3-core`, with the token in service config. The chat engine gear comes later.
- A metric's JSON is a structured query the service interprets — table, select, group by, where. No SQL in the definition.
- The chat writes through the same endpoints the UI uses, so it needs the portal session, not the ingest token.
- For the MVP, definitions are global: no owner column, everyone on the stand sees the same ones. Per-user and sharing are feature scope, not MVP.
- The portal session guards every definition endpoint. Ingest keeps its own token.
- Time is whatever a metric filters on. No dashboard period selector in the MVP.
- The chat prompt carries the table names and the field names sampled from each table, so it stops guessing.
- A name already in use is replaced, and the reply says which names it replaced.
- A widget whose metric fails shows the error in place of its content; an empty result says so.
- `/portal/custom` lists all dashboards and links to each. The chat panel sits there too.
- A dashboard created in the chat appears in that list at once, and the page routes to it.
