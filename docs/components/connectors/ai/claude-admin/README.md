# Claude Admin Connector — Documentation

Unified Anthropic Admin API connector for organization-level metadata, API usage, Claude Code usage, and costs.

This connector merges the former `claude-api` (programmatic API consumption) and `claude-team` (seats, Claude Code usage) connectors into a single package. Both used the same API (`api.anthropic.com`) and the same credential (Admin API key), with two duplicate endpoints (`/v1/organizations/workspaces`, `/v1/organizations/invites`). The consolidation produces 8 unique Bronze streams under `bronze_claude_admin`.

## Specifications

- **PRD**: [specs/PRD.md](./specs/PRD.md)
- **DESIGN**: [specs/DESIGN.md](./specs/DESIGN.md)
- **ADRs**: [specs/ADR/](./specs/ADR/) *(empty — inherited decisions documented inline in DESIGN §1.2 and §2.2)*

## Connector package

- **Source tree**: [`src/ingestion/connectors/ai/claude-admin/`](../../../../../src/ingestion/connectors/ai/claude-admin/)
- **Operator README**: [`src/ingestion/connectors/ai/claude-admin/README.md`](../../../../../src/ingestion/connectors/ai/claude-admin/README.md) — deployment, K8s Secret, stream tables, identity keys

## Related

- [claude-enterprise](../claude-enterprise/) — complementary connector for the Enterprise Analytics API (engagement data: DAU/WAU/MAU, chat projects, skill and connector adoption).
