# ADR-0015: Self-Scoped Visibility Read Without Admin

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Non-admin, self-scoped batch read (chosen)](#non-admin-self-scoped-batch-read-chosen)
  - [Admin-gate it like the rest of the family](#admin-gate-it-like-the-rest-of-the-family)
  - [Let each consumer derive the visible set itself](#let-each-consumer-derive-the-visible-set-itself)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-0015-self-scoped-visibility-read-without-admin`

**Status:** Accepted

## Context and Problem Statement

ADR-0012 gates every read of the OrgChart Visibility tables behind
the shared admin gate, and names as a decision driver that the whole family
"behaves identically under the same filter". It also leaves the door open:
a later ADR may relax specific reads, and offers
`GET /v1/person-roles?person=<self>` as the example.

Consumers now need that relaxation. A service serving per-person data must
know which of the people in a request the caller is allowed to see, and it
must ask on every request — the answer changes with grants and org moves.
Under an admin-only rule the only callers able to ask are administrators,
which is precisely inverted: the question is asked *on behalf of* ordinary
users.

## Decision Drivers

- The visible set is a property of the caller, so answering it for the
  caller reveals nothing the caller may not already reach.
- Consumers must not re-derive visibility. Two implementations of
  "who may see whom" drift, and the copy is the one that gets it wrong.
- One question per request, not one per person: the answer is needed for a
  whole batch on a hot path.

## Considered Options

- Non-admin, self-scoped batch read (chosen)
- Admin-gate it like the rest of the family
- Let each consumer derive the visible set itself

## Decision Outcome

`POST /v1/visible-persons` is authenticated but **not** admin-gated. It
takes canonical person ids (UUIDs) and answers with the subset the caller
may see, evaluated by the same union the rest of the service uses: the
caller, their active grants, the whole tenant on a wildcard grant, and
their `org_chart` descendants.

The request shape is UUIDs because the metrics runtime keys on `person_id`
since the identity cutover; the email-taking first cut of this endpoint
(and its `resolve_person_ids_by_emails` position-keyed resolution, needed
because `value_id` compares case- and accent-insensitively) is gone with
it. On a wildcard grant the answer is the request intersected with the
tenant's persons log: the grant covers everyone in the tenant, not everyone
whose UUID the caller can type, and consumers read this answer as
authorization — echoing a foreign tenant's id back would confirm it as
visible.

Three properties keep it least-privilege despite the missing admin gate:

- **Self-scoped.** The caller is taken from the gateway JWT. There is no
  acting-as parameter, so a caller can only ever ask about their own
  visible set.
- **No new disclosure.** The response echoes back a subset of the ids the
  caller supplied. Everything it reveals is already reachable through
  `POST /v1/profiles` one id at a time.
- **Absence carries the denial.** An id the caller may not see and an id
  that resolves to nobody are both simply absent, so the endpoint is not
  an existence oracle beyond what the caller's own grants already imply —
  a wildcard holder learns tenant membership, which their grant lets them
  enumerate through `POST /v1/profiles` anyway.

Roles stay out of the predicate. Holding the `admin` role confers no
visibility, exactly as before — administering identity and seeing people
remain separate powers.

### Consequences

- **Positive:** consumers gate on the same predicate the service enforces,
  so authorization cannot drift between services.
- **Positive:** a batch answer replaces one traversal per person.
- **Negative:** ADR-0012's "every endpoint behaves identically" no longer
  holds for the family as a whole. A reader must consult per-endpoint auth
  rather than assume the family rule; the route lives outside the
  `/v1/visibility` prefix to make the difference visible in the path.
- **Negative:** a consumer that fails closed on this endpoint makes
  identity a hard dependency of its own read path.

### Confirmation

Live-MariaDB cases assert the predicate directly: a caller with no reports
still sees themselves, a manager sees a transitive descendant and not an
unrelated person, an explicit grant reaches outside the reporting line, a
wildcard grant covers the tenant, and a holder of the `admin` role sees
no one extra. The e2e suite asserts the non-admin path end to end with a
non-admin caller.

## Pros and Cons of the Options

### Non-admin, self-scoped batch read (chosen)

- **Pro:** answers the question the consumers actually have, for the
  callers who actually have it.
- **Pro:** one round trip per request; a wildcard grant short-circuits
  before any traversal.
- **Con:** breaks the family's uniform auth shape (see Consequences).

### Admin-gate it like the rest of the family

- **Pro:** preserves ADR-0012 verbatim; nothing to re-reason about.
- **Con:** unusable for its purpose. Ordinary users are the ones whose
  visible set must be checked, and they would be refused.
- **Con:** pushes consumers toward a service token, which would make every
  check un-attributable to a person.

### Let each consumer derive the visible set itself

- **Pro:** no new endpoint.
- **Con:** a second implementation of the visibility rule. Grants and
  wildcard grants are easy to miss, and the copy fails open when it does.
- **Con:** requires reaching another service's tables, against the
  service-owned-schema rule.

## More Information

The predicate itself is unchanged by this ADR — only who may ask it, and
for how many people at once. A consumer that cannot reach this endpoint
must fail closed; treating an unreachable authorization backend as an
allow would be the failure this endpoint exists to prevent.

## Traceability

- Endpoint: `services/identity-resolution/src/api/visible_persons.rs`
- SQL: `subchart_repo::visible_targets`, `subchart_repo::has_wildcard_grant`
- Tests: `infra::db::visible_set_live_tests`,
  `tests/stand/api/identity/test_visible_persons.py`
- Related: ADR-0012 (admin-only reads — relaxed here for one read),
  ADR-0010 (org-chart cache), ADR-0011 (persons collation)
