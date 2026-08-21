# Portal under a flat organisation

## Goal

A member of an organisation with no reporting lines can browse everyone and read
org-wide numbers. Today the portal collapses to their own Person page.

## Why it collapses

`use-zone-nav` gates every org zone on `orgZonesVisible = isManager || mgrPending`,
and `isManager` is `viewer.subordinates.length > 0`. Under a flat policy nobody
has subordinates, so every viewer is an IC and `IC_ZONES = {"person"}` is the
whole rail.

## Approach

Change the question, not the zones. Every org zone already reads one hook —
`useOrgScope` → `{pivot, roster, managerNodes, count}`. Give that hook a second
source and open the gate:

- **"can this viewer see anyone but themselves?"** replaces "does this viewer
  manage anyone?" — answered by `visibility_policy` from `GET /v1/me`.
- Under `flat` the roster comes from `GET /v1/visible-persons` instead of a walk
  over `subordinates`. The hook's return shape is unchanged, so the zones,
  `use-scope-coverage` and the metric requests need no edits.

## Decisions

1. **Gate on the policy**, not on roster size. One declarative field; a
   wildcard-grant holder on a hierarchical install is a later question.
2. **Fetch every roster page up front.** Simplest thing that serves a small
   org; carries a comment to revisit, and a hard page ceiling so a large tenant
   degrades instead of hanging.
3. **Landing pins Overview** under flat — the org rollup is the meaningful
   first screen; People is the browse surface.
4. **The scope control hides.** With no manager nodes there is nothing to pick,
   and `ScopeSelect` already returns null on an empty list. `directOnly` goes
   with it.
5. **Fail closed to `org_chart`** when the policy is unknown (pending or
   errored): an unknown answer never widens what the rail shows.

## Tasks

### F1 — the policy reaches the SPA
`api/identity-client.ts` (`MeResponse`), `queries/identity-me.ts`
(`useVisibilityPolicy`). Accept: `flat` and `org_chart` both read back; pending
and errored both read `org_chart`.

### F2 — the roster client
`api/identity-client.ts` (`listVisiblePersons`), `queries/visible-roster.ts`
(`useVisibleRoster`). Accept: pages until `next_cursor` is null; stops at the
page ceiling; a refusal surfaces as an error rather than an empty roster.

### F3 — the rail opens
`lib/portal/use-viewer-reach.ts` (new), `lib/portal/use-zone-nav.ts`,
`lib/portal/landing-zone.ts`. Accept: under `flat`, a viewer with no reports
gets the org zones and lands on Overview; under `org_chart` nothing changes.

### F4 — the scope resolves without a tree
`lib/portal/use-org-scope.ts`. Accept: under `flat` the roster is every visible
person except the viewer, `managerNodes` is empty, `canDirectOnly` is false, and
the label names the organisation.

### F5 — the People zone lists the roster
`components/portal/employees-view.tsx`, `mocks/handlers.ts`. Accept: rows come
from the roster under `flat`, the supervisor column is absent, an unnamed person
still gets a clickable row.

### F6 — hierarchy copy stops claiming a hierarchy — NOT NEEDED
`person-header` already degrades honestly: with no parent there is no manager to
query, so the peer switcher has no peers, and with no manager nodes the
Team/Peers button has nothing to scope to. Both hide themselves. Pinned by a
regression test rather than changed.

What is genuinely absent is a flat analogue — a switcher over the roster, so a
reader can step to the next person without returning to the People zone. New
UI, not a copy fix; left for a decision.

## Out of scope

- `metric-results` caps at 1000 person ids and is all-or-nothing, so an org
  larger than that cannot answer org-wide numbers. Not addressed here.
- A static scope chip (label + head-count) where the picker used to be.
- Provisional people render as "Unnamed person" until a seed run names them.
