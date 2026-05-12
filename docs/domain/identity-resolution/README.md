# Identity Resolution Domain

Identity Resolution maps disparate identity signals — emails, usernames, employee IDs, platform-specific handles — from all connected source systems into canonical person records owned by the Person domain.

## Documents

| Document | Description |
|---|---|
| [`specs/PRD.md`](specs/PRD.md) | Product requirements: actors, functional requirements, use cases, acceptance criteria |
| [`specs/DESIGN.md`](specs/DESIGN.md) | Technical design: architecture layers, domain model, component model, API, database schemas, sequences |
| [`specs/DECOMPOSITION.md`](specs/DECOMPOSITION.md) | Feature decomposition: initial-seed, bootstrap-pipeline, matching-engine |

## Scope

This domain covers:
- Bootstrap mechanism (`identity_inputs` table, BootstrapJob component)
- Alias store (`aliases` table, alias resolution API)
- Matching engine (`match_rules`, confidence scoring, normalization pipeline)
- Unmapped alias queue (operator review workflow)
- Alias-level conflict detection
- Merge/split operations with audit trail (late phase)
- GDPR alias deletion (late phase)

Out of scope:
- Person registry (`persons` table, golden record assembly) — see [`docs/domain/person/`](../person/)
- Org hierarchy (`org_units`, `person_assignments`) — see [`docs/domain/org-chart/`](../org-chart/)
- Permission / RBAC — see `docs/domain/permissions/`
- Connector implementation
- Metric aggregation

## Cross-Domain References

- **Person domain**: `aliases.person_id` references `persons.person_id` (the stable UUIDv7 identity, not the auto-increment observation row PK). The Person domain owns person records; Identity Resolution links aliases to existing persons.
- **Org-Chart domain**: `identity_inputs` may carry org-related data consumed by the Org-Chart domain. No direct table references.
- **Shared table**: `identity_inputs` is owned by this domain and read by the Person domain (for person-attribute observations) and optionally by the Org-Chart domain.

## Source Documents (Inbox)

The `specs/DESIGN.md` synthesizes the following inbox documents:
- `inbox/architecture/IDENTITY_RESOLUTION_V2.md` — MariaDB reference, matching engine, API
- `inbox/architecture/IDENTITY_RESOLUTION_V3.md` — Silver layer contract, PostgreSQL added
- `inbox/architecture/IDENTITY_RESOLUTION_V4.md` — canonical v4: Golden Record, Source Federation, multi-tenancy
- `inbox/IDENTITY_RESOLUTION.md` — ClickHouse-native min-propagation algorithm
- `inbox/architecture/EXAMPLE_IDENTITY_PIPELINE.md` — end-to-end walkthrough
