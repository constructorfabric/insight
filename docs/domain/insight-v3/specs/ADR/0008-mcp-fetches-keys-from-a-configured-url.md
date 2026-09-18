---
status: accepted
date: 2026-09-09
---

# ADR-0008: Where the Server Looks for the Gateway's Public Keys

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-mcp-fetches-keys-from-a-configured-url`

## Context and Problem Statement

An MCP client authorizes against the gateway and receives a signed token. Every
call it then makes to this service carries that token, and this service has to
check the signature — which means fetching the gateway's public keys, published
as a JWKS document at `/.well-known/jwks.json`.

Nothing tells the service where to fetch them, so it derives the address from
`mcp.public_url` — the one URL it already has for the gateway, the address it
advertises to clients.

That address is the client's, not the server's. On a local stand
`mcp.public_url` is `http://localhost:8080`, and inside this service's container
`localhost:8080` is the container itself. The fetch reaches nothing, no
signature can be checked, and every authorized MCP call answers 503.

## Decision Drivers

* A client sees one address for this resource: the audience its token carries
  and the metadata it discovers.
* The server has to reach the keys from wherever it happens to run.
* A deployment whose public address already routes internally should need no
  new setting.

## Considered Options

* Keep deriving the key address from the public one — nothing to configure;
  broken on every stand whose public address is not reachable from inside.
* Configure a second, internal base address and derive the key address and the
  rest from it — covers whatever endpoint comes next; describes the gateway
  twice, and the two descriptions are free to disagree.
* Configure the key address alone, empty meaning "derive it" — names the single
  call that has to route differently; a stand that needs it has to know to set
  it. **Chosen.**

## Decision Outcome

`mcp.jwks_url` says where to fetch the keys. Left empty it is derived from
`mcp.public_url`, which is what a deployment with a routable address wants; the
local stand sets it to the gateway's container address. What clients see is
unchanged either way.

### Consequences

* A local stand sets one variable:
  `mcp.jwks_url: http://gateway:8080/.well-known/jwks.json`.
* Point it at something that is not the token's issuer and every token is
  refused — a wrong value fails closed, and loudly.

### Confirmation

Compose sets it to the gateway's JWKS address, and the MCP test asserts that an
unauthenticated call is still challenged for the public resource and its scope.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
