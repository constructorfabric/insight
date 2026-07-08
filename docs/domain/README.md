# Domain Specifications

Domain-level specifications for the Insight platform. Each domain represents a bounded context with its own PRD, DESIGN, and ADR documents.

## Domains

| Domain | Description | Status |
|---|---|---|
| [`ingestion/`](ingestion/) | Data pipeline from source APIs to Silver step 1 (Airbyte + Argo Workflows + dbt) | Accepted |
| [`connector/`](connector/) | Connector development: Insight Connector packages, nocode and CDK patterns, packaging, debugging | Accepted |
| [`identity-resolution/`](identity-resolution/) | Person identity matching and resolution across sources | Proposed |
| [`metrics/`](metrics/) | Unified metrics: typed registry + generic result runtime over source measure observations (`/v1/metric-results`) | Accepted |
