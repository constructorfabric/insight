# Org-Chart Domain

The Org-Chart domain manages the organizational hierarchy — the tree of departments, teams, and divisions — and the temporal assignments of persons to those organizational units. It enables analytics to attribute activity to the correct team at the time of the event.

## Documents

| Document | Description |
|---|---|
| [`specs/PRD.md`](specs/PRD.md) | Product requirements: hierarchy management, temporal assignments, re-org handling, point-in-time queries |
| [`specs/DESIGN.md`](specs/DESIGN.md) | Technical design: org_units hierarchy with materialized path, person_assignments temporal model, SCD2 references |

## Scope

This domain covers:
- `org_units` table — organizational hierarchy with materialized path and parent-child relationships
- `person_assignments` table — temporal assignment of persons to org units, roles, teams, managers
- Hierarchy management (create, update, deactivate org units)
- Re-org handling (parent changes with path recomputation, history preservation)
- Transfer handling (close-and-insert assignment pattern)
- Point-in-time queries ("who was in department X on date Y?")
- Legacy flat-string assignment types (`department`, `team`) for bootstrap

Out of scope:
- Person records (`persons` table, golden record) — see [`docs/domain/person/`](../person/)
- Alias resolution (`aliases`, `identity_inputs`, matching engine) — see [`docs/domain/identity-resolution/`](../identity-resolution/)
- SCD Type 2 snapshot table schemas — managed by dbt macros
- Permission / RBAC
- Connector implementation

## Cross-Domain References

- **Person domain**: `person_assignments.person_id` references `persons.id`. The Person domain owns person records; this domain assigns persons to org units. `persons.org_unit_id` references `org_units.id` (golden record field).
- **Identity Resolution domain**: `identity_inputs` (owned by IR domain) may carry org-related data as an alternative ingestion path. No direct table references between IR and Org-Chart.
