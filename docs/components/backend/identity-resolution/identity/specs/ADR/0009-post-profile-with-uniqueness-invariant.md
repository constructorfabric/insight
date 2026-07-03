# ADR-0009: `POST /v1/profiles` with Single-Result Invariant

**ID**: `cpt-insightspec-adr-0009-post-profile-with-uniqueness-invariant`

**Status:** Accepted

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [POST /v1/profiles with structured body + single-result invariant (chosen)](#post-v1profiles-with-structured-body--single-result-invariant-chosen)
  - [Batch lookup with array body](#batch-lookup-with-array-body)
  - [GET /v1/profiles with query params](#get-v1profiles-with-query-params)
  - [Multi-result list response](#multi-result-list-response)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

Phase 1 ships `GET /v1/persons/{email}` — a single-key, single-result
lookup keyed only on email. Phase 2 needs to broaden the lookup
surface so analytics-frontend and internal workflows can resolve
profiles by other identifier types (notably the source-native
`value_type='id'` observation within a specific
`(source_type, source_id)` instance, used by per-source workflows in
constructorfabric/insight#344).

The shape of the new endpoint must satisfy several constraints
simultaneously:
- Extensible to additional `id_type`s in Phase 3 (`username`,
  `employee_id`, `parent_person_id`) without re-routing the URL.
- Future-friendly for temporal range filtering
  (`start_date`/`end_date`) — Phase 3 work per Anton's PR #398 review.
- Honours the `persons` data invariant: at most one current
  `person_id` per (tenant, source_type, source_id, value_type='id')
  and at most one current `person_id` per (tenant, value_type='email')
  across all sources. A multi-match indicates corrupted state and must
  not be silently flattened to one row.

## Decision Drivers

- The endpoint must accept different identifier kinds (email vs
  source-native id) with different validation rules — cross-field
  constraints rule out a thin URL-template variant.
- Caller-supplied identifier values may contain reserved URL
  characters (emails with `+`, source ids with spaces); a body keeps
  encoding concerns server-side.
- Data-invariant violations must surface, not get masked — picking
  one of N matches at random would mislead callers about a corrupted
  `persons` state.
- The contract has to roll forward cleanly to Phase 3 without
  breaking callers — a structured body absorbs new fields more
  gracefully than additional query parameters.
- Anton's review on PR #398 explicitly preferred POST with structured
  parameters over GET-with-path-param for this surface.

## Considered Options

- `POST /v1/profiles` with a structured body and a single-result
  invariant.
- `POST /v1/profiles` with an array of lookups (batch) and a parallel
  array response.
- `GET /v1/profiles?email=…&person_id=…` with query params (the
  original constructorfabric/insight#347 spec).
- Multi-result list response (handler returns all matches, caller
  filters).

## Decision Outcome

Adopt `POST /v1/profiles` with a structured body:

```json
{ "value_type": "email", "value": "alice@example.com" }
```
or
```json
{ "value_type": "id", "value": "<source-native id>",
  "insight_source_type": "bamboohr",
  "insight_source_id": "<uuid>" }
```

The handler resolves over the canonical latest-per-source-instance
partition `(tenant, person, source_type, source_id, value_type)`
(ADR-0003) and returns:
- `200 OK` + full `ProfileResponse` (including `ids[]` list) on
  exactly one match,
- `404 Not Found` + `urn:insight:error:person_not_found` on zero
  matches,
- `422 Unprocessable Entity` +
  `urn:insight:error:ambiguous_profile` (with echoed lookup and
  matched `person_ids` list) on multiple matches,
- `400 Bad Request` + `urn:insight:error:*` for malformed bodies via
  FluentValidation.

### Consequences

- Phase 1 `GET /v1/persons/{email}` stays unchanged — existing
  callers (the analytics service, frontend via api-gateway proxy) continue
  to work; migration to POST is a per-caller refactor over time.
- Data-invariant violations surface as 422 in production — operators
  see real signal when the seed pipeline produces ambiguous rows.
- Phase 3 can extend the body with `start_date` / `end_date` /
  additional `value_type`s without a new endpoint URL.
- The new partition key in `SqlProfiles.cs` differs from the Phase-1
  `Sql.cs ResolvePersonIdByEmail` query: the Phase-2 partition
  includes `person_id` and excludes `value_id`, so a rebound email
  correctly fails to resolve to its prior person. The Phase-1 query
  is kept as-is for backwards compatibility.
- POST with non-idempotent semantics is fine here — the handler is
  read-only; HTTP method choice is driven by body-richness, not
  semantics. (Idempotency-Key header is not required.)

### Confirmation

Confirmed by eleven integration tests in
`Insight.Identity.Tests.Integration/ProfilesEndpointTests.cs`:
two happy-path lookups (email + id) plus an email-lowercase test,
three not-found paths (unknown email, unknown id within source,
rebound email old value), one ambiguity test (two persons sharing
an email → 422 with `person_ids` list), four validation paths
(missing value_type, id without source fields, email with source
fields, missing tenant).

## Pros and Cons of the Options

### POST /v1/profiles with structured body + single-result invariant (chosen)

- Good, because cross-field validation (value_type+source-fields) is
  ergonomic in a body — both with FluentValidation and OpenAPI doc.
- Good, because data-invariant violations surface as 422 rather than
  being hidden by silent picking.
- Good, because Phase 3 date-range and multi-id-type extensions can
  add body fields without breaking the URL.
- Good, because URL-encoding concerns (emails with `+`,
  source-native ids with spaces) move server-side.
- Bad, because the contract is non-idempotent at the HTTP level
  (response varies with `persons` state), but this is true of any
  read endpoint that reflects backing data.

### Batch lookup with array body

- Good, because callers needing N profiles can fetch in one
  round-trip.
- Bad, because the single-result invariant becomes per-element,
  which complicates the error shape — a 422 on one element shouldn't
  block the others; partial-success contracts are inherently more
  ambiguous.
- Bad, because frontend's first need is a single profile per
  request; premature batching adds API surface without a concrete
  consumer. Deferred to Phase 3 if a real caller asks.

### GET /v1/profiles with query params

- Good, because GET is conventionally idempotent and cacheable.
- Bad, because the request shape requires cross-field validation
  (`value_type='id'` ⇒ source fields required); expressing this in
  query params is awkward.
- Bad, because emails / source ids carry URL-reserved characters
  that need careful encoding; one bug in a client breaks lookups
  silently.
- Bad, because temporal range extension in Phase 3 stacks more query
  params; the body shape is cleaner.

### Multi-result list response

- Good, because no special-case 422; caller decides what to do with
  N matches.
- Bad, because the consumer is forced to encode invariant-violation
  detection itself — and probably won't, leading to wrong-person
  responses in production.
- Bad, because it hides data-quality problems that operators need to
  see.

## More Information

- constructorfabric/insight#347 — original Phase 2 issue (GET form).
- constructorfabric/insight#344 — parent epic.
- Anton's review on PR #398 — POST + structured body suggestion.
- ADR-0003 — latest-per-source-instance partition semantics.

## Traceability

- [`cpt-insightspec-fr-identity-profile-resolve`](../PRD.md#resolve-profile-by-email-or-source-native-id)
- [`cpt-insightspec-fr-identity-profile-ambiguous-422`](../PRD.md#surface-single-result-invariant-via-422)
- [`cpt-insightspec-fr-identity-profile-ids-list`](../PRD.md#project-full-alias-list-on-response)
- [`cpt-insightspec-fr-identity-profile-validation`](../PRD.md#validate-request-body-via-fluentvalidation)
