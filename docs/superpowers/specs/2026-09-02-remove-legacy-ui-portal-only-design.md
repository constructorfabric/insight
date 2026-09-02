# Remove the legacy UI — the portal is the only interface

Design, 2026-09-02.

## Goal

Delete the pre-portal shell and its screens from the frontend, so the portal is
the only interface the codebase can render. Move the two console surfaces that
have no portal home into the Manage zone rather than losing them.

## Current state

The portal is already the default. The old shell survives behind one
test-only hatch:

- `readLegacyShell()` in `src/frontend/src/lib/portal/portal-store.ts` reads
  `insight.legacyShell` from localStorage, once, at module load. The reader-facing
  key (`insight.portal`) is already retired and deleted on load.
- The only writer is `tests/stand/ui/conftest.py`, whose `context` fixture sets
  the key before any app code runs. No product surface offers it.
- `routes/__root.tsx` branches on it: portal paths render `PortalLayout`,
  everything else renders `AppSidebar` + `<Outlet/>`.
- `routes/index.tsx` and `routes/portal.tsx` each carry a second branch for the
  same hatch, and `MenuEntry` in `components/app-sidebar-footer.tsx` renders a
  legacy `<Link>` instead of a portal one.

`PortalLayout` renders `<ZoneContent/>`, not `<Outlet/>`. Under the portal the
route components below `/ic/$person/*` therefore never mount: Person and People
are `components/portal/person-view.tsx` and `people-view.tsx`, both
portal-native. `screens/dashboard.tsx` and `screens/team-view.tsx` are reachable
only through the hatch.

Routes and their portal equivalents:

| Route | Screen | Portal equivalent |
|---|---|---|
| `/` | `DashboardScreen` | `/portal` (already redirected unless the hatch is set) |
| `/metrics` | `MetricDefinitionsScreen` | Manage → Metric catalog (`MetricCatalogTable`) |
| `/whats-new` | `WhatsNewScreen` | Manage → What's new (`WhatsNewBody`) |
| `/custom-metrics` | `MetricsConsoleScreen` | none |
| `/queries` | `QueryConsoleScreen` | none |

`/custom-metrics` and `/queries` have no link pointing at them anywhere in the
app. They are reachable only by typing the URL.

## Decisions

1. Port both consoles into the Manage zone. They are real functionality with no
   replacement.
2. Ship them gated as `planned` through the instance nav policy in
   `insight-gitops`, in every cluster.
3. Delete the old paths outright. No redirects — the Manage items already exist
   for the two that have twins.
4. Remove `tests/stand/ui/` entirely. Which journeys to automate against the
   portal is not decided here; the PR description records what the removed ones
   covered.

## Scope

### 1. Manage gains two consoles

Add two entries to `MANAGE_ITEMS` in `src/frontend/src/lib/portal/nav-model.ts`:

- `custom-metrics` — custom metric authoring, today `MetricsConsoleScreen`.
- `query-console` — saved queries, today `QueryConsoleScreen`.

Both ids satisfy the `SEGMENT` pattern in `nav-policy.ts`
(`[a-z0-9][a-z0-9_-]*`), so both are addressable as
`zone:manage/item:<id>` in the instance nav policy.

Wire both in `ManageView` (`src/frontend/src/components/portal/manage-view.tsx`)
alongside the existing `ai-assistant` and `whats-new` branches.

Split each screen the way `screens/whats-new.tsx` is already split: keep the
body, drop the `<SidebarTrigger/>` header wrapper the portal has no use for.
`MetricsConsoleScreen` → `MetricsConsoleBody`, `QueryConsoleScreen` →
`QueryConsoleBody`. Everything under `components/widgets/metrics-console/` and
`components/widgets/query-console/` is unchanged.

Both need nav labels through i18n, following the existing
`metric_definitions.nav_label` / `whats_new.nav_label` keys.

### 2. Instance nav policy — `insight-gitops`, all clusters

Add both items to `frontend.nav.planned` in every environment:

- `environments/cfabric/values.yaml` — append to the Manage block.
- `environments/dev/values.yaml` — append to the Manage block.
- `environments/virtuozzo/values.yaml` — append to the Manage block.
- `environments/qaclust/values.yaml` — has no `frontend.nav` block at all, so
  one is added carrying `hide: []` and these two `planned` entries.

Entries:

```yaml
- zone:manage/item:custom-metrics
- zone:manage/item:query-console
```

`planned` demotes an item into the "Planned" group and hides it entirely from
readers who have not turned planned sections on
(`partitionByReadiness` in `nav-model.ts`, `usePortalShowPlanned`). The consoles
therefore ship without becoming visible to every reader.

This is a change in a second repository. It can land before or after the
frontend change — a policy entry naming an item the build does not have is
inert.

### 3. Legacy shell removal

- `lib/portal/portal-store.ts`: delete `LEGACY_SHELL_KEY`, the module-scope
  `legacyShell` read, and `readLegacyShell()`. Drop the covering cases in
  `portal-store.test.ts`.
- `routes/__root.tsx`: `RootLayout` keeps one branch — `isPortalShellPath`
  chooses `PortalLayout`, anything else (the `$` catch-all) renders `<Outlet/>`
  without app chrome. The `SidebarProvider` / `AppSidebar` / `SidebarInset`
  branch goes, along with the `MockBanner` and `ViewAsBanner` mounts that live
  inside it — `PortalLayout` mounts both already.
- `routes/index.tsx`: `beforeLoad` redirects to `/portal` unconditionally; the
  `IndexRoute` component and its `DashboardScreen` import go.
- `routes/portal.tsx`: `PortalRoute` returns `<PortalLayout/>`; the `Navigate`
  branch goes.
- `components/app-sidebar-footer.tsx`: `MenuEntry` loses the `screen` prop, the
  legacy `<Link to={screen}/>`, and the `pathname` comparison. It always renders
  the portal link and resolves its active state through `resolveZoneItem`.

### 4. Deletions

Files, with their tests and stories:

- `components/app-sidebar.tsx`
- `components/ic-view-toggle.tsx`
- `screens/dashboard.tsx`
- `screens/team-view.tsx`
- `screens/metric-definitions.tsx`
- `screens/whats-new.tsx` — the `WhatsNewScreen` export only; `WhatsNewBody`
  stays and the file stays.
- `routes/metrics.tsx`, `routes/custom-metrics.tsx`, `routes/queries.tsx`,
  `routes/whats-new.tsx`
- `components/widgets/dashboard/`: `dashboard-header.tsx`,
  `dashboard-empty-state.tsx`, `members-overview.tsx`,
  `team-members-attention.tsx`, `triage-list.tsx`, and anything they were the
  last consumer of.

Everything else under `components/widgets/dashboard/` is shared with the portal
(`explain-with-ai`, `group-drilldown-sheet`, `ic-needs-attention`, `kpi-tile`,
`members-grid`, `metric-sublabel`, `person-coverage`) and stays. So do
`components/org-tree.tsx`, `components/app-sidebar-footer.tsx`,
`components/sidebar-settings.tsx`, `components/mock-banner.tsx`,
`components/view-as-banner.tsx` and `components/ui/sidebar.tsx`, all of which
the portal renders.

`routeTree.gen.ts` is generated — it updates with the route files.

### 5. Person routes

`routes/ic.$person.personal.tsx` and `routes/ic.$person.team.tsx` render nothing
under the portal, so their components become empty and their screen imports go.

`ic.$person.personal.tsx` currently guards with `isPersonId(person)` and
redirects a malformed id to `/`. That guard runs only in the component, which is
dead under the portal. Move it into the route's `beforeLoad` as a redirect so a
malformed id still never reaches the metrics API.

### 6. Stand tests

Delete `tests/stand/ui/` — all 13 journeys, their page objects, `flows.py`,
`conftest.py` and the fixture that writes the hatch.

Two of the journeys already run against the portal rather than the legacy shell:
`test_platform_usage.py` builds its own context from `browser`, bypassing the
fixture, and `test_development_lenses.py` overrides the fixture to remove the
key. Deleting the directory discards those too.

The PR description names what the removed journeys covered — login, session
survival across navigation, catalog views, metric-evidence drilldowns,
team-grid and timeseries evidence, collaboration-card evidence, feedback round
trip, git-output timeseries, seeded data visibility, logged-out access refusal,
platform usage, development lenses — and states that portal automation scope is
not decided in this task.

## Out of scope

- Choosing which journeys to automate against the portal.
- Any change to the console surfaces themselves beyond the header split.
- Removing the `Planned` gating once the consoles are validated.

## Delivery

Two merge requests in `constructorfabric/insight`, plus one in `insight-gitops`.

1. **Consoles into Manage.** Additive. The nav entries, the `ManageView`
   branches, the body split, i18n labels. Nothing is deleted; the app still
   renders both shells. Shippable alone.
2. **Legacy UI removal.** Sections 3, 4, 5, 6. Lands after MR 1, so no window
   exists where the consoles are unreachable.
3. **`insight-gitops`.** Section 2, four environment files. Independent of the
   other two.

## Risks

- `/custom-metrics` and `/queries` have no coverage above the widget unit tests.
  Their move into Manage is unproven until someone opens the zone with planned
  sections on and exercises both.
- Deleting `tests/stand/ui/` removes the only browser-level proof that login,
  session-holding navigation and the evidence drilldowns work against a real
  stand. That gap is accepted here and stays open until portal journeys are
  written.
- `cfabric` lists `zone:manage/item:data-health` as planned, an id absent from
  `MANAGE_ITEMS`. Unrelated to this work, but it shows a stale policy entry is
  silent — the new entries need their ids to match the code exactly.

## Verification

- `pnpm typecheck` and `pnpm lint` clean — the primary gate, since most of the
  deletion surfaces as unresolved imports.
- `pnpm test` green.
- `pnpm test:storybook:ci` green (`sidebar-settings.stories.tsx` and
  `kpi-tile.stories.tsx` both survive the deletion).
- `grep` for `legacyShell`, `AppSidebar`, `DashboardScreen`, `TeamViewScreen`,
  `MetricDefinitionsScreen` returns nothing outside history.
- In a browser: `/portal` renders, `/metrics` and `/whats-new` hit the `$`
  catch-all, and Manage lists both consoles once planned sections are on.
