# Zendesk Connector

Extracts Zendesk ticket data, satisfaction ratings, and agent directory into the Bronze layer. Feeds the support domain Silver pipeline for CSAT, ticket volume, resolution rate, and agent workload analytics.

## Specification

- **Spec**: [`zendesk.md`](./zendesk.md) — Bronze table schemas, source mappings, Silver/Gold targets
- **PRD**: [`specs/PRD.md`](./specs/PRD.md) — functional and non-functional requirements
- **DESIGN**: [`specs/DESIGN.md`](./specs/DESIGN.md) — technical architecture, field mappings, collection strategy
- **Domain**: [`../README.md`](../README.md) — unified Support domain schema (Zendesk + JSM)

## Streams

| Stream | Table | Sync Mode |
|--------|-------|-----------|
| `support_tickets` | `bronze_zendesk.support_tickets` | Incremental (`updated_at`) |
| `support_ticket_ids` | `bronze_zendesk.support_ticket_ids` | Incremental (slim audit-substream parent) |
| `support_agents` | `bronze_zendesk.support_agents` | Full refresh |
| `zendesk_satisfaction_ratings` | `bronze_zendesk.zendesk_satisfaction_ratings` | Incremental (`updated_at`) |
| `support_ticket_events` | `bronze_zendesk.support_ticket_events` | Incremental (Ticket Audits — SHIPPED) |

`zendesk_ticket_ext` remains the only deferred Phase 2 stream — schema locked in `zendesk.md`.

## Source code

`src/ingestion/connectors/support/zendesk/`
