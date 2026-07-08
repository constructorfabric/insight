---
name: connector-schema
description: "Generate JSON schema from real API data"
---

# Generate Schema

Runs discover against the real API and generates JSON schema files + configured catalog.

## Prerequisites

- Connector package exists
- Tenant credentials configured

## Phase 1: Generate Schemas

```bash
./airbyte-toolkit/generate-schema.sh CONNECTOR_NAME <tenant>
```

Saves to `connectors/<path>/schemas/<stream>.json`

## Phase 2: Generate Catalog

```bash
./airbyte-toolkit/generate-catalog.sh CONNECTOR_NAME <tenant>
```

Saves to `connectors/<path>/configured_catalog.json`

## Phase 3: Update Manifest (optional)

Ask user: "Update inline schemas in connector.yaml from generated files? [y/n]"

If yes, for each stream:
1. Read `schemas/<stream>.json`
2. Update `schema_loader.schema` in `connector.yaml` for matching stream

## Phase 4: Summary

```
=== Schema Generation: <name> ===

  Streams discovered: N
  Schema files:
    schemas/email_activity.json     (M fields)
    schemas/teams_activity.json     (M fields)
  Catalog: configured_catalog.json  (N streams, all enabled)

Next: /connector test <name>
```
