# Domain Specifications

Domain-level specifications for the Insight platform. Each domain represents a bounded context with its own PRD, DESIGN, and ADR documents.

## Domains

| Domain | Description | Status |
|---|---|---|
| [`ingestion/`](ingestion/) | Data pipeline from source APIs to Silver step 1 (Airbyte + Argo Workflows + dbt) | Accepted |
| [`ingestion-monitoring/`](ingestion-monitoring/) | Bronze freshness SLA, trap detection, multi-driver alert delivery | Proposed |
| [`connector/`](connector/) | Connector development: Insight Connector packages, nocode and CDK patterns, packaging, debugging | Accepted |
| [`identity-resolution/`](identity-resolution/) | Person identity matching and resolution across sources | Proposed |
