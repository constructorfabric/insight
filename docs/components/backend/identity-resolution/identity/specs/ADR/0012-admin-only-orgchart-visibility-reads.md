# ADR-0012: Admin-Only Reads on OrgChart Visibility Tables


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Admin-only on every OrgChart Visibility read (chosen)](#admin-only-on-every-orgchart-visibility-read-chosen)
  - [Admin-only on writes, public on reads](#admin-only-on-writes-public-on-reads)
  - [Per-resource policies](#per-resource-policies)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-0012-admin-only-orgchart-visibility-reads`

**Status:** Accepted

## Context and Problem Statement

#346 step 4 introduced CRUD endpoints on three OrgChart Visibility tables:

- `GET /v1/visibility` / `GET /v1/roles` / `GET /v1/person-roles`
- plus the matching POST / DELETE.

Writes are clearly admin-only — minting visibility grants and role
assignments is a privileged operation. Reads are less obvious. Two
plausible audiences:

- **Admin tooling** — needs to list every grant in a tenant to audit
  who-sees-whom and who-holds-which-role.
- **Self-service callers** — could conceivably need to read their own
  row ("which roles do I hold?") without admin rights.

The question is whether to ship reads as admin-only by default or
to relax them per-resource for self-introspection.

## Decision Drivers

- Least-privilege default: hand out permissions only when a real use
  case asks for them.
- Predictable shape: every OrgChart Visibility endpoint behaves
  identically under the same admin gate — easier to
  reason about, easier to integration-test.
- Forward-compat: a future ADR can relax specific reads (e.g.
  `GET /v1/person-roles?person=<self>` without admin) without
  breaking existing admin tooling.

## Considered Options

- **(A) Admin-only on every OrgChart Visibility read.** Single gate,
  single response shape. Self-introspection blocked until a follow-up
  ADR opens it.
- **(B) Admin-only on writes, public on reads.** Anyone with a valid
  caller header can list all grants and assignments.
- **(C) Per-resource policies.** E.g. roles list is public, visibility
  list is admin-only, person-roles list is "self or admin".

## Decision Outcome

**Chosen: (A) — admin-only on every OrgChart Visibility read.**

Implemented in `VisibilityEndpoints` / `RolesEndpoints` /
`PersonRolesEndpoints` by passing every handler through
the shared admin gate. The check returns one of
four results (`NoCaller` / `NoTenant` / `NotAdmin` / `IsAdmin`)
mapped to 401 / 400 / 403 / proceed by `EndpointHelpers.GateResult`.

### Consequences

- **Negative:** A non-admin caller cannot read their own grants or
  role assignments. Self-introspection requires admin help today.
- **Positive:** Every OrgChart Visibility endpoint behaves identically.
  Adding a new endpoint to the family is a one-line gate inclusion.
- **Positive:** No accidental leakage if a future relaxation is
  done sloppily — the default is closed, not open.

### Confirmation

Integration tests `OrgChartVisibilityEndpointsTests.Post_visibility_without_caller_returns_401`
and `Post_visibility_as_non_admin_returns_403` exercise the gate on
the POST side; the GET branches use the same `EndpointHelpers.GateResult`
helper so the same code path is covered.

## Pros and Cons of the Options

### Admin-only on every OrgChart Visibility read (chosen)

- **Pro:** single gate, single response shape — every OrgChart
  Visibility endpoint behaves identically under the admin gate.
- **Pro:** default-closed — sloppy future relaxation can't accidentally
  open a new leak.
- **Con:** blocks self-introspection (caller can't read their own
  grants/roles).

### Admin-only on writes, public on reads

- **Pro:** self-introspection works without admin involvement.
- **Con:** anyone with a valid caller header can enumerate every
  visibility grant and role assignment in the tenant — significant
  metadata leak.

### Per-resource policies

- **Pro:** fine-grained access (roles public, person-roles self-or-admin,
  visibility admin-only).
- **Con:** N gates to reason about, N test surfaces, N future ADRs.

## More Information

When a self-introspection use case lands, the natural extension is a
parallel `CallerSelfCheck` service that returns IsSelf / NotSelf and
a per-endpoint policy combinator. That work is out of scope here.

## Traceability

- Implementations: `src/backend/services/identity-resolution/src/api/{visibility,roles,person_roles}.rs`
- Gate plumbing: `src/backend/services/identity-resolution/src/api/gate.rs`
- Tests: the identity contract suite's admin cases
