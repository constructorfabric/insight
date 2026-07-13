# Connector Domain

Insight Connector development: package structure, manifest authoring, dbt transformations, local debugging.

An **Insight Connector** is a complete pipeline package: Airbyte Connector + descriptor + dbt transformations + credential template.

## Documents

| Document | Description |
|---|---|
| [`specs/DESIGN.md`](specs/DESIGN.md) | Connector specification: mandatory fields, manifest rules, CDK guide, schema, dbt, deployment |
| [`specs/PRD.md`](specs/PRD.md) | Requirements: connector framework, Bronze/Silver rules, packaging, RLS |
| [`specs/ADR/0001-connector-integration-protocol.md`](specs/ADR/0001-connector-integration-protocol.md) | Historical: stdout JSON protocol decision (superseded by Airbyte Protocol) |
| [`specs/feature-connector-mock-tests/FEATURE.md`](specs/feature-connector-mock-tests/FEATURE.md) | Mock-server tests: credential-free pytest suites over `connector.yaml` via `airbyte_cdk.test` (HttpMocker), stream coverage matrix, CI gate |

## Implementation

Source code: [`src/ingestion/`](../../../src/ingestion/)

```
src/ingestion/
  connectors/{category}/{name}/     # Insight Connector packages
    connector.yaml                  #   Airbyte declarative manifest
    descriptor.yaml                 #   Metadata: schedule, streams, dbt_select
    credentials.yaml.example        #   Credential template (tracked)
    configured_catalog.json         #   For local debugging (optional)
    schemas/                        #   Generated JSON schemas per stream
    dbt/                            #   Bronze → Silver transforms
  connections/                      #   Tenant configs (gitignored)
  tools/declarative-connector/      #   Local debugging: source.sh
  scripts/
    generate-schema.sh              #   Extract schemas from discover
    generate-catalog.sh             #   Generate configured_catalog.json
    airbyte-toolkit/register.sh     #   Register connectors in Airbyte
```

## Related Domains

| Domain | Relationship |
|---|---|
| [Ingestion](../ingestion/) | Parent: orchestration, deployment, pipeline |
