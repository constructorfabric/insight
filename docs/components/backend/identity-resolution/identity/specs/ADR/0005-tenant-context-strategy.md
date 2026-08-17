# ADR-0005: Composite Tenant Context With JWT Stub

**ID**: `cpt-insightspec-adr-0005-tenant-context-strategy`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Composite chain (chosen)](#composite-chain-chosen)
  - [Single header resolver](#single-header-resolver)
  - [JWT-only](#jwt-only)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Accepted

## Context and Problem Statement

The service is per-tenant by every query — `insight_tenant_id` is part
of every index and every SQL predicate. Two transport flows must
coexist:

- Internal callers (api-gateway, dbt-runner) send a header
  `X-Insight-Tenant-Id`.
- A future direct-call flow (cookie/JWT issued by api-gateway) will
  carry tenants in claims.

For local development, a single tenant is wired in by configuration so
operators don't have to hand-craft headers.

## Decision Drivers

- Header is what every current internal caller already sends.
- The future JWT flow must land without rewriting how tenants are
  resolved.
- Local dev must work header-less without compromising production
  safety.
- A missing tenant must fail loudly, not silently default.

## Considered Options

- Single `HeaderTenantContext` resolver — header-only.
- Composite chain (header → JWT stub → config default).
- Always derive tenant from JWT, refuse header-based callers.

## Decision Outcome

Implement three resolvers and a composite that walks them in
declaration order:

1. `HeaderTenantContext` — reads `X-Insight-Tenant-Id`.
2. `JwtTenantContext` — reads the `insight_tenant_id` claim. Stub for
   Phase 1.5; relies on api-gateway forwarding the principal.
3. `ConfigTenantContext` — returns
   `APP__gears__identity-resolution__config__tenant_default_id` when set.

If all return `null`, the endpoint returns 400 with an RFC 7807 body.

### Consequences

- A misconfigured default in a multi-tenant environment is a
  data-leak risk. Operators must leave the default unset in shared
  production.
- The composite is the only `ITenantContext` registered in DI; the
  individual resolvers are still classes for tests to instantiate
  directly.
- The JWT resolver is a stub in Phase 1 — it always returns null.
  The slot is wired so that turning it on in Phase 2 is a one-method
  change.

### Confirmation

Confirmed by `CompositeTenantContextTests` (unit) which builds the
chain with mocked individual resolvers and asserts the first non-null
wins. Integration test
`PersonsEndpointTests.MissingTenantReturns400` exercises the
all-null path.

## Pros and Cons of the Options

### Composite chain (chosen)

- Good, because today's header callers and future JWT callers both
  work without different endpoints.
- Good, because local dev gets a config-default escape hatch.
- Good, because the chain is trivially unit-testable — each resolver
  is its own class.
- Bad, because the chain order has to be documented (header wins
  over config to prevent dev defaults from masking prod headers).

### Single header resolver

- Good, simpler.
- Bad, because turning on JWT requires touching the endpoint, not
  just adding a resolver to DI.
- Bad, because local dev forces header gymnastics.

### JWT-only

- Good, because zero-trust friendly.
- Bad, because api-gateway doesn't have a BFF module today; the JWT
  isn't propagated yet.
- Bad, because dbt-runner / internal workflows have no JWT to send.

## More Information

- Review comment on PR #398 (the header-based tenant context)
  flagging that JWT verification must follow in Phase 2.

## Traceability

- [`cpt-insightspec-fr-identity-lookup-400-tenant`](../PRD.md#missing-tenant-returns-rfc-7807)
- [`cpt-insightspec-principle-identity-tenant-composite`](../DESIGN.md#composite-tenant-resolver-header-first)
