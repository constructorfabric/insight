# ADR-0013: `roles` Table — Hard DELETE Guarded by Active-Assignment Count


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Hard DELETE with 422 in-use guard (chosen)](#hard-delete-with-422-in-use-guard-chosen)
  - [Add valid_to to roles, do soft-delete](#add-validto-to-roles-do-soft-delete)
  - [Cascade soft-delete all assignments](#cascade-soft-delete-all-assignments)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-0013-roles-hard-delete-with-in-use-guard`

**Status:** Accepted

## Context and Problem Statement

`visibility` and `person_roles` are SCD2 — soft-delete on revoke
preserves history (`valid_to = UTC_TIMESTAMP(6)` rather than `DELETE`).
The `roles` catalogue is **strict-minimum** by design (#346 design
rev 3.1): just `role_id` + `name`, no `valid_from`/`valid_to`/`author`
columns. There is no soft-delete slot.

So `DELETE /v1/roles/{id}` either:

- Hard-DELETEs the row (no history retained), or
- Refuses the operation entirely, or
- The table grows a `valid_to` column and joins the SCD2 family.

`person_roles` references `roles.role_id` (logical FK — no DB-level
constraint declared per the codebase pattern in DESIGN §3.8). A
DELETE on a role that still has active assignments would orphan
those rows.

## Decision Drivers

- Preserve the strict-minimum shape of `roles` (decided in #346 rev 3.1,
  re-confirmed during the PR #517 review).
- Don't silently orphan `person_roles` rows.
- Surface a friendly error to the caller, not an opaque DB-level
  failure on the next assignment read.

## Considered Options

- **(A) Hard DELETE with 422 in-use guard.** Before DELETE, count
  active `person_roles` rows referencing the role across all tenants.
  If `> 0`, refuse with `urn:insight:error:role_in_use`.
- **(B) Add `valid_to` to `roles`, do soft-delete.** Loses the strict-
  minimum shape; introduces SCD2 onto a catalogue that doesn't need
  history (the name is the identity, regenerate the row if needed).
- **(C) Cascade soft-delete all assignments.** Revoke every
  assignment on role deletion. Side effect is too wide — a single
  DELETE quietly invalidates many person_roles rows that an admin
  may have wanted to keep alive.

## Decision Outcome

**Chosen: (A).** Hard DELETE with active-assignment guard.

The guard and the write happen in a **single atomic SQL statement** —
no separate `COUNT` round-trip — so two concurrent admin DELETEs
cannot both pass a stale count and both succeed (TOCTOU). The DELETE
is conditional on `NOT EXISTS (… active assignments …)`:

```sql
DELETE FROM roles
WHERE role_id = @role_id
  AND NOT EXISTS (
      SELECT 1 FROM person_roles
      WHERE role_id = @role_id AND valid_to IS NULL
  )
```

The endpoint inspects `rows_affected`:

```csharp
var existing = await repo.GetRoleByIdAsync(id, ct);             // for audit + initial 404
if (existing is null) return NotFound("role", id);

var rowsAffected = await repo.TryDeleteRoleIfUnusedAsync(id, ct);
if (rowsAffected == 1) { audit(…); return 204; }

// rowsAffected == 0 → re-read disambiguates 404 vs 422.
var refetched = await repo.GetRoleByIdAsync(id, ct);
if (refetched is null) return NotFound("role", id);
var live = await repo.CountActiveAssignmentsByRoleAnyTenantAsync(id, ct);
return 422 role_in_use with live count;
```

The disambiguation re-read is benign — it only chooses between two
already-correct deny responses; the integrity invariant (no orphan
`person_roles`) is enforced by the atomic statement.

### Consequences

- **Negative:** No history of deleted roles. If a role is ever
  recreated with the same name, it gets a new `role_id` — anyone
  who cached the old id holds a dangling reference. Mitigated by
  the `Roles.Admin` constant being pinned at a deterministic UUID
  in migration `007_roles.sql` (the role that matters most can't
  be DELETEd anyway — see §"More Information").
- **Positive:** `roles` stays strict-minimum. No additional columns
  to migrate, no SCD2 query complexity.
- **Positive:** Operators get an actionable 422 instead of a database-
  level failure surfacing later when assignments are read.

### Confirmation

Integration test `OrgChartVisibilityEndpointsTests.Roles_delete_in_use_returns_422_role_in_use`
covers the guard path; `Roles_create_and_delete_round_trip` covers
the happy DELETE path (a freshly-created role with no assignments
deletes successfully).

## Pros and Cons of the Options

### Hard DELETE with 422 in-use guard (chosen)

- **Pro:** preserves the strict-minimum shape of `roles` (no extra
  columns, no SCD2 query complexity).
- **Pro:** orphaning is impossible — the guard fires before DELETE.
- **Con:** no history of deleted roles; recreating a role with the
  same name yields a new UUID.

### Add valid_to to roles, do soft-delete

- **Pro:** symmetric with `visibility` / `person_roles`.
- **Con:** loses strict-minimum shape — `roles` becomes SCD2 for a
  catalogue that doesn't need history; every role-lookup query
  grows a `WHERE valid_to IS NULL` clause.

### Cascade soft-delete all assignments

- **Pro:** DELETE always succeeds; no separate guard.
- **Con:** side effect too wide — one role DELETE quietly invalidates
  many `person_roles` rows that an admin may have intended to keep.

## More Information

The seeded `admin` role (UUID `a4d11000-0000-4000-8000-000000000001`)
is effectively undeletable in any tenant where bootstrap has run —
the bootstrap admin assignment counts as an active assignment, so
the guard fires. Operators who genuinely want to "drop the admin
role" must first revoke every admin assignment in every tenant; the
result is a tenant that can never use the CRUD endpoints again
without re-running bootstrap. See ADR-0014 for the related last-
admin protection.

## Traceability

- Endpoint: `src/backend/services/identity-resolution/src/api/roles.rs`
- SQL: `infra/db/roles_repo.rs` — the conditional delete (the in-use guard) and
  the active-assignment count (only for the refusal message, not for the guard)
- Tests: the identity contract suite's roles cases
