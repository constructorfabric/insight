---
status: accepted
date: 2026-09-09
---

# ADR-0005: MCP Clients Authorize in a Browser


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-browser-authorization-for-mcp`

## Context and Problem Statement

Authoring happens over MCP: a client writes metrics, widgets and dashboards by
calling this service's tools. Every such call needs a token carrying the
`mcp:author` scope, and nothing issues one — so the tools people would author
from, a terminal agent or an editor on a laptop, cannot author at all.

## Decision Drivers

* Authoring authority is the authority the person already has on the stand.
* A grant has to expire and be revocable.

## Considered Options

* A browser-obtained OAuth 2.1 authorization code — the person's own
  identity, scoped and expiring; needs a browser. **Chosen.**
* A long-lived person-scoped API token — works headless; a secret to rotate,
  and none exists yet. **Planned beside it.**
* One static token per instance, like ingest — trivial; every author is the
  same author, revocable only for everyone at once.

## Decision Outcome

Browser authorization ships first. Person-scoped tokens are planned beside it,
not instead of it.

### Consequences

* A person re-authorizes when the grant expires.
* Headless clients cannot author yet.

### Confirmation

[tests/mcp.sh](../../../../src/backend/services/insight-v3-core/tests/mcp.sh)
runs the challenge without a credential and the authoring half only with a
browser-issued token.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
