# ADR-0014: Last-Admin Protection on `person_roles` Revoke


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Block the last-admin revoke (chosen)](#block-the-last-admin-revoke-chosen)
  - [Permit the revoke, log a warning](#permit-the-revoke-log-a-warning)
  - [No protection](#no-protection)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-0014-last-admin-protection`

**Status:** Accepted

## Context and Problem Statement

CRUD on every OrgChart Visibility table is admin-gated (ADR-0012). The admin gate
itself is satisfied by an active row in `person_roles` with
`role_id = Roles.Admin` and `valid_to IS NULL` for the caller in the
target tenant.

Without a guard, `DELETE /v1/person-roles/{id}` could remove the
last active admin assignment in a tenant — leaving the tenant un-
administrable. Subsequent CRUD calls all fail the gate; the only
recovery path is the bootstrap config + redeploy (#346 step 5,
documented in OPS).

Self-inflicted lockout is a foreseeable mistake, especially for
small tenants with a single ops user.

## Decision Drivers

- A foreseeable mistake that bricks the tenant deserves an explicit
  guard rather than a recovery procedure.
- The guard must be cheap — checked on every revoke, can't add
  noticeable latency.
- The guard shouldn't block legitimate revokes (multi-admin tenants
  removing one of several admins).

## Considered Options

- **(A) Block the last-admin revoke.** Before soft-delete, count
  active admin assignments in the target tenant. If `<= 1`, refuse
  with `urn:insight:error:last_admin_protected`.
- **(B) Permit the revoke, log a warning.** Tenant breaks; oncall is
  notified via warning log. Recovery via bootstrap.
- **(C) No protection.** Tenant breaks silently; ops figures it out
  by support ticket.

## Decision Outcome

**Chosen: (A).** Block at the endpoint level via a transactional
lock-then-update pattern. The count and the write happen inside a
single InnoDB transaction (RepeatableRead isolation); a
`SELECT … FOR UPDATE` taken on every active admin row in the target's
tenant **before** the UPDATE serialises concurrent revokes against
each other. Without that lock, the correlated `COUNT(*)` inside the
UPDATE's derived table is — per MariaDB's documented snapshot-read
semantics for plain subqueries — non-locking, so two concurrent
revokes targeting different admin rows in a 2-admin tenant could each
observe `count = 2` and both commit, leaving the tenant with zero
admins. The lock closes that race deterministically: the second
transaction blocks until the first commits, then re-evaluates the
count over a serialised view and refuses with 422.

Statement 1 — pin the count:

```sql
SELECT COUNT(*)
FROM person_roles
WHERE insight_tenant_id = (
        SELECT insight_tenant_id FROM person_roles
        WHERE person_role_id = @person_role_id
      )
  AND role_id  = @admin_role_id
  AND valid_to IS NULL
FOR UPDATE
```

Statement 2 — guarded soft-delete (same SQL as before, now running
under held locks):

```sql
UPDATE person_roles AS target
JOIN (
    SELECT
        pr.person_role_id,
        pr.role_id,
        (
            SELECT COUNT(*) FROM person_roles AS adm
            WHERE adm.insight_tenant_id = pr.insight_tenant_id
              AND adm.role_id           = @admin_role_id
              AND adm.valid_to IS NULL
        ) AS active_admin_cnt
    FROM person_roles AS pr
    WHERE pr.person_role_id = @person_role_id AND pr.valid_to IS NULL
) AS row_with_count
  ON row_with_count.person_role_id = target.person_role_id
SET target.valid_to = UTC_TIMESTAMP(6),
    target.reason   = COALESCE(@reason, target.reason)
WHERE target.valid_to IS NULL
  AND (
      row_with_count.role_id <> @admin_role_id
      OR row_with_count.active_admin_cnt > 1
  )
```

The endpoint pre-fetches the row for audit metadata, then inspects
`rows_affected`:

```csharp
var existing = await repo.GetPersonRoleByIdAsync(id, ct);
if (existing is null || existing.ValidTo is not null) return NotFound;

var rowsAffected = await repo.TrySoftDeletePersonRoleProtectingLastAdminAsync(
    id, Roles.Admin, body?.Reason, ct);
// Inside the repo: BEGIN TX RepeatableRead → SELECT … FOR UPDATE →
//                  UPDATE … → COMMIT.
if (rowsAffected == 1) { audit(…); return 204; }

// rowsAffected == 0 → re-read disambiguates 404 vs 422.
var refetched = await repo.GetPersonRoleByIdAsync(id, ct);
if (refetched is null || refetched.ValidTo is not null) return NotFound;
return 422 last_admin_protected;
```

The derived table (`row_with_count`) inside the UPDATE materialises
the row-to-revoke metadata AND the tenant's active-admin count in
one shot. MariaDB resolves the correlated COUNT inside the derived
table's SELECT list before the outer UPDATE acts, so the "cannot
SELECT from the table being updated" restriction does not bite.

The guard activates only when the row being revoked is an active
admin assignment. Non-admin role revokes (e.g. a future `auditor`)
skip the OR-branch entirely — the `role_id <> @admin_role_id` clause
short-circuits the count subquery.

Both the lock SELECT and the UPDATE's COUNT subquery hit the
`idx_role_current (insight_tenant_id, role_id, valid_to)` index —
the lock pins index records covering active admins in the target's
tenant, so the lock scope is exactly the rows the guard checks.
Per-revoke overhead is one extra DB round-trip; concurrent admin
revokes in the same tenant serialise on the lock instead of running
in parallel, but this is the desired behaviour — a tenant has at
most a handful of admins and revokes are rare. Non-admin revokes
(e.g. a future `auditor` role) take the same code path but exit
immediately on the `role_id <> @admin_role_id` short-circuit; the
lock acquired in that case still pins admin rows but releases on
commit microseconds later.

The original ADR (pre-PR #532 review) claimed a single-statement UPDATE
was sufficient. mitasov's review (#532) noted correctly that the
correlated `COUNT(*)` inside the derived table is — per MariaDB's
documented snapshot-read semantics for plain subqueries — non-locking,
so the single-statement guarantee was implementation-dependent. This
ADR was updated post-merge in the follow-up PR that introduces the
lock-then-update pattern.

### Consequences

- **Negative:** The guard is enforced only at the endpoint layer.
  A direct SQL UPDATE that bypasses the API would still drop the
  last admin. We accept this — the API is the only sanctioned write
  path; ops with direct DB access can intentionally do this for
  recovery scenarios (e.g. handing the tenant over to a new admin
  team).
- **Negative:** The guard does NOT prevent admin self-revocation
  when another admin exists. An admin can revoke themselves; if the
  remaining admins later lose access (e.g. they revoke each other),
  the tenant locks. This is consistent with the general "admin
  responsibility" model.
- **Positive:** Single tenant with a single admin cannot self-brick
  via the API.
- **Positive:** Multi-admin tenants can freely revoke individual
  admins as long as one remains active.

### Confirmation

Integration tests:
- `OrgChartVisibilityEndpointsTests.PersonRoles_revoke_last_admin_returns_422_last_admin_protected`
  — single admin scenario, expects 422.
- `OrgChartVisibilityEndpointsTests.PersonRoles_revoke_admin_when_another_admin_exists_succeeds`
  — two admins seeded, expects 204 on revoke of the first.

## Pros and Cons of the Options

### Block the last-admin revoke (chosen)

- **Pro:** prevents foreseeable self-lockout via the API.
- **Pro:** zero cost on the common path (index lookup, only on
  admin-role revokes).
- **Con:** guard lives only at the endpoint layer; direct SQL UPDATE
  still bypasses it. Accepted — ops with DB access can deliberately
  hand a tenant over.

### Permit the revoke, log a warning

- **Pro:** simplest impl, no new error code.
- **Con:** tenant breaks; recovery via bootstrap + redeploy. Foreseeable
  enough that an explicit guard is worth the few lines.

### No protection

- **Con:** silent self-lockout; first sign is a support ticket
  saying "all our admin calls return 403". Worst UX of the three.

## More Information

If a tenant somehow ends up with zero admins (direct SQL, bootstrap
mis-configuration, or a prior revoke under an earlier version of
this code), recovery is the same as initial provisioning: set
`APP__gears__identity-resolution__config__bootstrap_admin_person_id`
and restart. The idempotent `INSERT … WHERE NOT EXISTS` in the
first-admin bootstrap will mint the row on the next pod start.

## Traceability

- Endpoint: `src/backend/services/identity-resolution/src/api/person_roles.rs`
- SQL: the guarded soft-delete in `infra/db/person_roles_repo.rs`
- Tests: `OrgChartVisibilityEndpointsTests.PersonRoles_revoke_last_admin_returns_422_last_admin_protected`,
  `OrgChartVisibilityEndpointsTests.PersonRoles_revoke_admin_when_another_admin_exists_succeeds`
- Related: ADR-0012 (admin-only reads), ADR-0013 (roles hard-delete guard)
