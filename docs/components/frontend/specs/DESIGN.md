---
status: draft
version: "0.5"
date: 2026-10-27
---

# Technical Design — Insight Portal User Interface

- [ ] `p3` - **ID**: `cpt-insightspec-portal-design-ui`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional context](#4-additional-context)
  - [4.1 Visual Source](#41-visual-source)
  - [4.2 Foundation Values](#42-foundation-values)
  - [4.3 Shared Components](#43-shared-components)
  - [4.4 Cross-Cutting Concerns](#44-cross-cutting-concerns)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The portal is a single-page application, and every view is named by its address. The address is the only source of truth for what is on screen: rail destination, pane item, reporting window, org scope, cohort, folder and tag filters. A view can therefore be bookmarked, shared and restored, and two parts of the screen can never disagree about the window or scope they show.

Browser storage holds only per-person preferences (§3.7), including the last-used portal window. That window is read only when an address carries no window, and is then written into the address, so an address always fully determines its view.

The services enforce reach and compute every figure. The portal turns service responses into view models with pure functions (window labelling, navigation filtering, comparisons, state classification) and draws them with one component library and one token set.

The visual layer is one foundation: semantic colour tokens defined once for light and dark, type roles, a small spacing and radius scale, Lucide icons, and FrontX kit components where the kit has them. The shell fixes a rail, a pane, a pinned toolbar and, in Dashboards, an Assistant column. Every page renders inside that shell.

### 1.2 Architecture Drivers

**ADRs**: `cpt-insightspec-portal-adr-one-period-control`

#### Functional Drivers

| Requirement | Design Response |
|-------------|------------------|
| `cpt-insightspec-portal-fr-rail` | Rail model builds destinations from the zones the viewer may reach and the installation policy; planned destinations use the rail item's planned state. |
| `cpt-insightspec-portal-fr-pane` | One pane component renders titled groups from a navigation model per rail destination; counts render as a count badge. |
| `cpt-insightspec-portal-fr-narrow-screens` | Layout tiers show the pane in flow, off-canvas, or in a drawer holding both navigation levels; picking a leaf closes the drawer. |
| `cpt-insightspec-portal-fr-period-control` | One period control component, with presets resolved by the services, on every page that shows period figures. |
| `cpt-insightspec-portal-fr-period-scope` | Portal window and dashboard window are separate address parameters, read by separate hooks and fed to the same control. |
| `cpt-insightspec-portal-fr-cohort` | Toolbar shows the cohort control beside the period control on comparison pages and writes the cohort to the address. A help tooltip says what it controls. With only one cohort, it is disabled with its reason. |
| `cpt-insightspec-portal-fr-org-scope` | Scope lives in the address, written by the work chart and the person page's Team action; scoped views show their scope label. |
| `cpt-insightspec-portal-fr-browse` | Browse selection lives in the address; the pane group and the chip row both read and write it. |
| `cpt-insightspec-portal-fr-share` | Share dialog driven by the sharing service's grant model and the dashboard's may-share flag. |
| `cpt-insightspec-portal-fr-states` | State components classify each section's read as loading, empty, not connected, refused or failed, each with its own presentation. |
| `cpt-insightspec-portal-fr-planned` | Navigation policy and the per-person "show planned" preference filter every navigation model; a planned item is never the default. |
| `cpt-insightspec-portal-fr-a11y` | Shell supplies a skip link and current-destination semantics; components come from an accessible kit; state components announce. |
| `cpt-insightspec-portal-fr-period-semantics` | Presets are tokens the services resolve. The period control labels each preset with dates from one shared rule, kept in step with the services by a contract test. |
| `cpt-insightspec-portal-fr-session` | Shell shows a session-ended notice when refresh fails, then starts sign-in; the sign-in error page covers denied and retryable failures. |
| `cpt-insightspec-portal-fr-not-found` | A catch-all route and unknown-item resolution render the not-found state component instead of redirecting. |
| `cpt-insightspec-portal-fr-view-as` | Shell shows the view-as banner, with an exit, and the demonstration-data banner above the toolbar. |
| `cpt-insightspec-portal-fr-dashboards-access`, `cpt-insightspec-portal-fr-open-dashboard`, `cpt-insightspec-portal-fr-create-dashboard`, `cpt-insightspec-portal-fr-widgets`, `cpt-insightspec-portal-fr-orphaned` | Dashboards area renders actions from the definition and sharing services' per-dashboard permissions. Member reads use the dashboard-scoped widget routes, with no drill-down. |
| `cpt-insightspec-portal-fr-lenses`, `cpt-insightspec-portal-fr-evidence`, `cpt-insightspec-portal-fr-ai-cost` | Explore's lens renderers and evidence dialog. Evidence and cost render only where the analytics service returns them for the viewer's grants. |
| `cpt-insightspec-portal-fr-person`, `cpt-insightspec-portal-fr-team` | People area; the members grid opens sorted by name. |
| `cpt-insightspec-portal-fr-manage`, `cpt-insightspec-portal-fr-ingestion-health`, `cpt-insightspec-portal-fr-identities` | Manage area; default item resolution skips placeholders. |
| `cpt-insightspec-portal-fr-destructive` | Confirm dialog with destructive styling for deletion; widening warnings in the share and move-to-folder flows. |

#### NFR Allocation

| NFR ID | NFR Summary | Allocated To | Design Response | Verification Approach |
|--------|-------------|--------------|-----------------|----------------------|
| `cpt-insightspec-portal-nfr-honest-states` | Failed or refused never renders as empty or zero | State components; view-model functions | Every section read is classified before rendering; a missing reading maps to "not tracked", never zero | Component tests per state; fault-injected stand journeys |
| `cpt-insightspec-portal-nfr-reach` | Nothing drawn outside the viewer's reach | Service clients; view models | Views render only service responses; refusals render as refusals; navigation hiding is cosmetic | Stand API and UI suites per persona |
| `cpt-insightspec-portal-nfr-bounded-requests` | Requests per list page constant in item count | List pages; service clients | List responses carry each card's display fields; cards never fetch individually | Network assertions in component tests |
| `cpt-insightspec-portal-nfr-feedback-latency` | Loading state within 200 ms | Router pending state; state components | Router pending view and per-section spinners appear on any unresolved read | UI journey timing checks |
| `cpt-insightspec-portal-nfr-accessibility` | WCAG 2.2 AA in both themes | Foundation tokens; kit components; shell | Kit components carry roles and names; shell adds skip link and current markers; token contrast follows the design file | Automated accessibility scan per page and theme; keyboard walkthrough |
| `cpt-insightspec-portal-nfr-exports` | Exports carry only what is shown | Evidence dialog; report builder | Export writes the rows already on screen; each export records a usage event naming the page | Component tests comparing exported rows with rendered rows |
| `cpt-insightspec-portal-nfr-screen-widths` | Usable from 360 px with no page scroll | Shell layout tiers; tables and charts | Phone-tier drawer; toolbar scrolls sideways; wide content scrolls in its own frame | Viewport tests at 360, 768 and 1440 px |
| `cpt-insightspec-portal-nfr-installation-policy` | Hide or plan any page by configuration | Navigation policy | Policy paths per zone, item, direction, lens, section and metric, applied to every navigation model | Policy unit tests; configured stand check |

### 1.3 Architecture Layers

```mermaid
flowchart TB
  P["Presentation — shell, pages, foundation components"]
  A["Application — routes, address state, query cache, hooks"]
  D["Domain — view models: windows, navigation, comparisons, states"]
  I["Infrastructure — service clients, auth session, installation config, telemetry"]
  P --> A --> D
  A --> I
```

- [ ] `p3` - **ID**: `cpt-insightspec-portal-tech-stack`

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| Presentation | Shell, pages, foundation components, charts, themes | React, Tailwind CSS, FrontX UI kit, Base UI primitives, Recharts, Lucide icons, Inter |
| Application | Routing, address state, data fetching and caching, per-person preferences | TanStack Router, TanStack Query, browser storage |
| Domain | Window labelling, navigation filtering, comparisons, state classification | TypeScript pure functions |
| Infrastructure | Service clients, session refresh, installation configuration, usage telemetry, error reporting | Fetch-based clients, runtime configuration script, Sentry |

## 2. Principles & Constraints

### 2.1 Design Principles

#### The Address Is the State

- [ ] `p1` - **ID**: `cpt-insightspec-portal-principle-address-state`

Everything that decides what a view shows is an address parameter. Browser storage holds only preferences that do not change what a shared address shows. Components read view state from the address and write it through navigation actions.

**ADRs**: `cpt-insightspec-portal-adr-one-period-control`

#### Services Decide Reach

- [ ] `p1` - **ID**: `cpt-insightspec-portal-principle-services-decide`

The portal renders what the services return for the signed-in person and shows their refusals. It never filters a broader response down, and never infers from absence that data does not exist.

#### One Foundation

- [ ] `p2` - **ID**: `cpt-insightspec-portal-principle-one-foundation`

Colour, type, spacing and radius come only from the foundation tokens and type roles (§4.2). A component that needs a new value adds a token, never a literal.

#### Kit First

- [ ] `p2` - **ID**: `cpt-insightspec-portal-principle-kit-first`

A control the FrontX kit provides is used from the kit, through a local wrapper. A local component exists only where the kit has none.

#### Classify Before Drawing

- [ ] `p1` - **ID**: `cpt-insightspec-portal-principle-classify-states`

Every independently loaded section classifies its read (loading, empty, not connected, refused, failed) before drawing. Each class has exactly one presentation across the portal.

#### Planned Is Policy

- [ ] `p2` - **ID**: `cpt-insightspec-portal-principle-planned-policy`

The navigation policy and a page's built state decide whether it is hidden, planned or live, never a branch in a view.

### 2.2 Constraints

#### FrontX Kit

- [ ] `p2` - **ID**: `cpt-insightspec-portal-constraint-kit`

The portal builds on the FrontX UI kit and its theme. Foundation tokens layer over the kit's theme and must not fork its components.

#### UTC Days

- [ ] `p1` - **ID**: `cpt-insightspec-portal-constraint-utc`

The services bucket every day at UTC midnight; the portal resolves and labels every window in UTC. Today's portal resolver uses local dates and ends rolling ranges yesterday; it moves to the shared UTC rule with ADR-0001.

#### Public Repository

- [ ] `p1` - **ID**: `cpt-insightspec-portal-constraint-public`

The portal's source, fixtures, stories and screenshots are public; every example name, figure and person in them is synthetic.

#### Configuration at Load

- [ ] `p2` - **ID**: `cpt-insightspec-portal-constraint-config-load`

Installation configuration arrives once, when the portal loads; a policy change takes effect on the next load.

## 3. Technical Architecture

### 3.1 Domain Model

**Technology**: TypeScript types

**Location**: [src/frontend/src/types](../../../../src/frontend/src/types), [src/frontend/src/lib/portal](../../../../src/frontend/src/lib/portal)

**Core Entities**:

| Entity | Description | Schema |
|--------|-------------|--------|
| ViewState | The current view's address parameters: rail destination, pane item, direction, lens, repository, scope, cohort, folder, tags, window | [portal-search.ts](../../../../src/frontend/src/lib/portal/portal-search.ts) |
| ReportingWindow | A preset token or a custom from–to pair, resolved to UTC dates | [period-to-date-range.ts](../../../../src/frontend/src/api/period-to-date-range.ts), [time-range.ts](../../../../src/frontend/src/lib/custom/time-range.ts) |
| RailItem | A top-level destination with its zones and planned state | [rail-model.ts](../../../../src/frontend/src/lib/portal/rail-model.ts) |
| PaneGroup, PaneItem | A titled group of pane destinations, each with icon, count, badge, readiness and gates | [nav-model.ts](../../../../src/frontend/src/lib/portal/nav-model.ts) |
| NavPolicy | Paths the installation hides or marks planned | [nav-policy.ts](../../../../src/frontend/src/lib/portal/nav-policy.ts) |
| LensConfig, SectionSpec | A lens and its ordered sections by kind | [lens-configs.ts](../../../../src/frontend/src/lib/portal/lens-configs.ts) |
| DashboardSummary | A listed dashboard's id, name, owner, change time, visibility and tags. New in this design: today's listing carries only id and name | [custom-types.ts](../../../../src/frontend/src/api/custom-types.ts) |
| ReadState | A section read's classified outcome: loading, empty, not connected, withheld, refused, failed. A single reading the source does not provide is not a read state: it renders in place as "not tracked". New in this design: today each page classifies ad hoc and renders through a placeholder card | [coming-soon.tsx](../../../../src/frontend/src/components/widgets/coming-soon.tsx) |

**Relationships**:
- RailItem → PaneGroup: a rail destination owns the pane groups of its zones.
- PaneItem → NavPolicy: the policy filters and marks a pane item.
- ViewState → ReportingWindow: the portal window and a dashboard window are separate fields of the view state.
- LensConfig → SectionSpec: a lens renders its sections in order; each section produces one ReadState.
- DashboardSummary → ViewState: browse selection, folder and tags in the view state select the summaries listed.

### 3.2 Component Model

```mermaid
flowchart LR
  Shell --> Rail
  Shell --> Pane
  Shell --> Toolbar
  Toolbar --> PeriodControl
  Toolbar --> CohortControl
  Shell --> Areas
  Areas --> Home
  Areas --> Dashboards
  Areas --> Explore
  Areas --> People
  Areas --> Reports
  Areas --> Manage
  Dashboards --> Assistant
  Areas --> StateComponents
  Rail --> NavModel
  Pane --> NavModel
  NavModel --> NavPolicy
  Areas --> Foundation
  Shell --> Foundation
```

#### Shell

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-shell`

##### Why this component exists

Every page needs the same navigation, view controls, banners and scroll behaviour in the same place.

##### Responsibility scope

- Lays out rail, pane and content.
- Titles Home's pane with the product name, and every other pane with its rail item's name.
- Owns the scroll container, so the toolbar pins above scrolling content.
- Switches layout tier by width:
  - phone, below 768 px: no rail; a drawer with both navigation levels;
  - narrow, 768–1023 px: rail and an off-canvas pane;
  - wide, from 1024 px: rail and pane in flow.
- Outside Dashboards, holds the toolbar: the cohort control (where the page draws comparisons and the organisation has more than one cohort), followed by the period control. Shows no toolbar in Dashboards.
- Shows the view-as and demonstration-data banners.
- Supplies the skip link.
- Hosts the settings menu and feedback dialog.

##### Responsibility boundaries

Does not decide which destinations exist (navigation model) or what a page shows. Holds no view state outside the address.

##### Related components (by ID)

- `cpt-insightspec-portal-component-nav-model` — depends on
- `cpt-insightspec-portal-component-period-control` — hosts
- `cpt-insightspec-portal-component-foundation` — styled by

#### Navigation Model and Policy

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-nav-model`

##### Why this component exists

The six rail destinations, their panes, and every direction and lens must be filtered by reach, installation policy and the person's planned preference in one consistent way.

##### Responsibility scope

- Defines rail destinations and pane groups.
- Applies the installation policy (hide, planned) by typed path: zone, item, direction, lens, section, metric.
- Moves planned items into a trailing Planned group.
- Resolves the default item as the first live, shown item the viewer may open.
- Exposes counts and badges.

##### Responsibility boundaries

Does not render. Does not enforce access: gates on the admin role or previews are display courtesies over the services.

##### Related components (by ID)

- `cpt-insightspec-portal-component-shell` — consumed by

#### Period Control

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-period-control`

##### Why this component exists

One reporting-window control replaces per-area controls with different preset lists.

##### Responsibility scope

Draws:

- Last 7 days and Last 30 days as a segmented pair;
- a More menu listing every preset with its resolved dates, with a check on the current one;
- the active range button, with a calendar icon and UTC marker;
- a range calendar with Apply, Cancel and Clear.

Refuses a custom range over the maximum length, with the reason. Reads and writes the window of the scope it is given: the portal, or one dashboard.

##### Responsibility boundaries

Does not compute figures. Does not decide which window a page uses; the page passes its window binding.

##### Related components (by ID)

- `cpt-insightspec-portal-component-shell` — hosted in the toolbar by
- `cpt-insightspec-portal-component-dashboards` — hosted in the open dashboard's header row, bound to that dashboard's window, by

#### Home Area

- [ ] `p2` - **ID**: `cpt-insightspec-portal-component-home`

##### Why this component exists

The landing view that answers what to look at.

##### Responsibility scope

Composes, in order: starter dashboard cards; the three most recently changed dashboards the viewer owns, with New Dashboard; per-person headline tiles with change; attention grouped by metric, with people counts; AI adoption; direction cards. Overview themes reuse the lens renderer.

##### Responsibility boundaries

Owns no data shape; each block reads through its section's renderer.

##### Related components (by ID)

- `cpt-insightspec-portal-component-explore` — shares the lens section renderers with
- `cpt-insightspec-portal-component-dashboards` — links to

#### Dashboards Area

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-dashboards`

##### Why this component exists

The one place for everything a person can view, save, share and author.

##### Responsibility scope

- Pane: Browse (All, My dashboards, Shared with me, Starred, Starter, with counts), Folders, Catalogue (Metrics, Widgets, Datasets, Alerts) and Pinned.
- List page: search, tag filter, count, browse chips mirroring the pane, and dashboard cards.
- Open dashboard: a header row (title, period control bound to the dashboard's window, Edit, menu) pinned above its scrolling widgets; then the meta line (owner, visibility, source chips, updated) and widgets in stored order.
- Create dialog (name, visibility, starting point) and share dialog.
- Folder, tag and pin actions.
- Catalogue lists and item pages; definition editor in form and text views; alert list, page and form.

##### Responsibility boundaries

Does not decide ownership, grants or what a person may do. Renders the sharing service's model and the may-share and may-edit flags.

##### Related components (by ID)

- `cpt-insightspec-portal-component-assistant` — hosts
- `cpt-insightspec-portal-component-period-control` — binds a dashboard window to

#### Assistant Panel

- [ ] `p2` - **ID**: `cpt-insightspec-portal-component-assistant`

##### Why this component exists

Describing a dashboard is the main way to make one, so the conversation must stay at hand across Dashboards pages.

##### Responsibility scope

A right column in Dashboards that:

- is collapsed by default to a 48 px strip with an open button, is 320 px when open, and sits below the content on narrow screens;
- keeps one conversation mounted across Dashboards navigation;
- renders replies with their kind, prose, result rows and links to what was built;
- restores a refused question to the input;
- accepts a starting name and visibility from the create dialog.

##### Responsibility boundaries

Does not apply changes itself; the Assistant service creates or replaces definitions.

##### Related components (by ID)

- `cpt-insightspec-portal-component-dashboards` — hosted by

#### Explore Area

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-explore`

##### Why this component exists

Directions and AI & Cost are read lens by lens over the org scope.

##### Responsibility scope

- Pane: direction folders (lens counts, lens rows) and AI & Cost (Overview, AI adoption, Cost).
- Person-grain lens renderer for the section kinds headline, stat tiles, trend, distribution, concentration, composition, event histogram, dimension table, ownership, contributors, heatmap by hour, participation, attention, direction cards and coverage levels.
- Organisation-grain renderer for CI-style lenses.
- Repository drill-down.
- AI & Cost view.
- Evidence dialog with people list, per-person records and CSV / Excel export.

##### Responsibility boundaries

Does not define metrics or what their sections mean. Lens configuration names the metrics; the services compute them.

##### Related components (by ID)

- `cpt-insightspec-portal-component-states` — uses

#### People Area

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-people`

##### Why this component exists

Person-level and team-level reading within the viewer's reach.

##### Responsibility scope

- Pane: Views (Me, My team, Roster · by role; Overview and Roster in a flat organisation), the person's Sections with data-state marks, and the work chart with search (a tree rooted at the viewer, or an alphabetical list when flat).
- Person page, section page, team view (summary, attention, sortable members grid) and roster table (search, role grouping).
- Writes org scope from the work chart and the Team action.

##### Responsibility boundaries

Does not decide reach or cohort membership.

##### Related components (by ID)

- `cpt-insightspec-portal-component-explore` — shares section renderers with

#### Reports Area

- [ ] `p3` - **ID**: `cpt-insightspec-portal-component-reports`

##### Why this component exists

Figures leave the product as spreadsheets.

##### Responsibility scope

Report builder: grain toggle, granularity toggle limited by the window, metric picker by family with search, preview dialog, CSV / Excel download, and a bottom action bar with selection count, blockers and cancel. Planned items show as planned rows.

##### Responsibility boundaries

Does not shape export rows; the analytics service returns them.

##### Related components (by ID)

- `cpt-insightspec-portal-component-nav-model` — planned rows from

#### Manage Area

- [ ] `p2` - **ID**: `cpt-insightspec-portal-component-manage`

##### Why this component exists

One place for installation health and configuration.

##### Responsibility scope

Groups Data, People & access and Platform. Holds the connector health table with expandable recent syncs; identity review queue and lookups; ingestion trends with live view and per-connector drill; previews; AI assistant configuration; What's new; portal usage. Lands on the first working destination.

##### Responsibility boundaries

Role gates are display courtesies; the services authorise every operation.

##### Related components (by ID)

- `cpt-insightspec-portal-component-nav-model` — items and gates from

#### State Components

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-states`

##### Why this component exists

Loading, empty, not connected, refused and failed must look the same everywhere and never be confused.

##### Responsibility scope

- Spinner with a loading announcement.
- Inline "not tracked" marker for a single reading a source does not provide.
- Empty card.
- Not-connected card naming the missing source.
- Withheld card stating the group-size reason.
- Refusal card showing the service's reason.
- Error card with retry.
- Planned row and badge.
- Not-found view and session-ended notice.

Densities: card, chip and row.

##### Responsibility boundaries

Does not classify reads; pages classify and pass the class.

##### Related components (by ID)

- `cpt-insightspec-portal-component-foundation` — styled by

#### Foundation

- [ ] `p1` - **ID**: `cpt-insightspec-portal-component-foundation`

##### Why this component exists

One set of tokens, type roles and components keeps every page visually consistent in both themes.

##### Responsibility scope

Semantic colour tokens for light and dark, type roles, spacing, radius and control sizes, shell dimensions, icon set and stroke, and the shared components with their variants and states (§4.2, §4.3).

##### Responsibility boundaries

Holds no product behaviour.

##### Related components (by ID)

- `cpt-insightspec-portal-component-shell` — styles

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-portal-interface-addresses-map`

- **Contracts**: `cpt-insightspec-portal-interface-addresses`, `cpt-insightspec-portal-contract-services`, `cpt-insightspec-portal-contract-installation`
- **Technology**: Browser addresses with search parameters
- **Location**: [portal-search.ts](../../../../src/frontend/src/lib/portal/portal-search.ts), [routes](../../../../src/frontend/src/routes)

**Endpoints Overview**:

| Method | Path | Description | Stability |
|--------|------|-------------|-----------|
| `GET` | `/portal` | Home, Explore, Reports and Manage views, selected by zone, item, direction, lens and repository parameters | stable |
| `GET` | `/portal/custom` | Dashboard list, with browse, folder and tag parameters | stable |
| `GET` | `/portal/custom/{dashboard-id}` | Open dashboard, addressed by its generated identity, with its own window parameter | stable |
| `GET` | `/portal/custom/metrics`, `/widgets`, `/datasets`, `/alerts`, each with `/{item}` | Catalogue lists and item pages. `metrics`, `widgets`, `datasets` and `alerts` are reserved and never dashboard identities | stable |
| `GET` | `/portal/custom/new/{kind}`, `/portal/custom/edit/{kind}/{item}` | Definition editor: create and edit | stable |
| `GET` | `/portal/custom/alerts/new`, `/portal/custom/alerts/{alert}/edit` | Alert form: create and edit | stable |
| `GET` | `/ic/{person}`, `/ic/{person}/personal`, `/ic/{person}/team` | Person page (the bare address opens the personal view) and team view | stable |
| `GET` | any other path | Not-found view | stable |

Shared view parameters: portal window (preset or from–to), scope and direct-only, cohort. A dashboard window parameter applies only on an open dashboard. Service interfaces are in §3.4; installation configuration is in §3.8.

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|----------|
| Analytics service | Metric results and catalogue interface | Every figure, trend, distribution and evidence list |
| Identity service | Viewer, roster, roles and reach interface | Who the viewer is, whom they may see, organisation placement |
| Definition service (Insight v3) | Definitions, folders, tags, pins interface | Catalogue, dashboards and their organisation |
| Sharing (Insight v3) | Ownership and grants interface | Owner, visibility and may-share / may-edit flags |
| Assistant service | Conversation interface | Answers and authored definitions |
| Authenticator | Session interface | Session probe, refresh and sign-in |

**Dependency Rules** (per project conventions):
- No circular dependencies
- Always use SDK modules for inter-module communication
- No cross-category sideways deps except through contracts
- Only integration/adapter modules talk to external systems
- `SecurityContext` must be propagated across all in-process calls

### 3.5 External Dependencies

#### FrontX UI Kit

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| FrontX UI kit | Component library and theme | Buttons, badges, cards, inputs, menus, dialogs, tabs, tooltips and their base theme |

#### Error Reporting

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| Sentry | Browser SDK | Unhandled errors from the error boundary |

**Dependency Rules**: as in §3.4.

### 3.6 Interactions & Sequences

#### Change the Portal Window

**ID**: `cpt-insightspec-portal-seq-change-window`

**Use cases**: `cpt-insightspec-portal-usecase-team-review`

**Actors**: `cpt-insightspec-portal-actor-lead`

```mermaid
sequenceDiagram
    Lead ->> PeriodControl: Pick Last quarter
    PeriodControl ->> Address: Write portal window
    Address ->> Page: View state changed
    Page ->> Analytics: Read sections for new window
    Page -->> Lead: Loading per section
    Analytics -->> Page: Results
    Page -->> Lead: Sections with new dates in toolbar
```

**Description**: The window is written to the address once; every section re-reads for it and shows its own loading state.

#### Open a Shared Dashboard

**ID**: `cpt-insightspec-portal-seq-open-shared`

**Use cases**: `cpt-insightspec-portal-usecase-open-shared`

**Actors**: `cpt-insightspec-portal-actor-viewer`

```mermaid
sequenceDiagram
    Viewer ->> Dashboards: Shared with me
    Dashboards ->> Definitions: List shared dashboards
    Definitions -->> Dashboards: Summaries with owner, visibility, tags
    Viewer ->> Dashboards: Open one
    Dashboards ->> Definitions: Read dashboard and widgets
    Dashboards ->> Analytics: Run widgets for the dashboard window
    Dashboards -->> Viewer: Meta line, widgets, menu without editing actions
```

**Description**: The list renders from one paged response. The open dashboard offers only the actions the services allow the viewer.

#### Create by Describing

**ID**: `cpt-insightspec-portal-seq-create-describe`

**Use cases**: `cpt-insightspec-portal-usecase-describe-dashboard`

**Actors**: `cpt-insightspec-portal-actor-owner`

```mermaid
sequenceDiagram
    Owner ->> CreateDialog: Name, Private, Describe it to the Assistant
    CreateDialog ->> AssistantPanel: Open with name and visibility
    Owner ->> AssistantPanel: Description
    AssistantPanel ->> AssistantService: Request
    AssistantService -->> AssistantPanel: Built dashboard link
    AssistantPanel ->> Dashboards: Navigate to new dashboard
```

**Description**: The dialog hands its choices to the Assistant; the service creates the dashboard and the panel opens it.

### 3.7 Database schemas & tables

- [ ] `p3` - **ID**: `cpt-insightspec-portal-db-browser-storage`

The portal has no database. Per-person preferences persist in browser storage, and none of them changes what a shared address shows.

#### Table: browser storage keys

**Schema**:

| Column | Type | Description |
|--------|------|-------------|
| key | string | Storage key |
| value | string | Stored preference |

**PK**: key

**Constraints**: Absent or unreadable values fall back to defaults.

**Additional info**:

- Preferences: `theme`, `insight.portal.showPlanned`, `insight.focus-mode`, `insight.explanations`, `insight.custom.assistant-hidden`, `insight.drilldown.expanded`, per-chart `insight.timeseries.{chart}.presentation`, per-dataset `insight.custom.dataset.{dataset}.columns`, and the last-used portal window, `insight.period` and `insight.period.custom`.
- Session coordination, not preferences: `insight.auth.refresh`, `insight.auth.refresh.leader`, `insight.auth.refresh.msg`, `insight.auth-error-attempts`.

A stored window from an earlier preset vocabulary is discarded and replaced by the first-visit default.

**Example**:

| key | value |
|--------|--------|
| theme | dark |
| insight.portal.showPlanned | false |

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-insightspec-portal-topology-static`

The portal is a static bundle served behind the installation's gateway. Installation configuration is a script served beside it and read once at load. The services are reached through the same gateway.

## 4. Additional context

### 4.1 Visual Source

The foundation and the drawn screens follow the Insight design file:

- Foundation page: tokens, type, scale, components;
- Key screens page: Home, Dashboards, open dashboard, navigation map, create dialog, dark theme, period selection.

Pages the design file does not draw follow §3.2 and the PRD.

### 4.2 Foundation Values

Colour tokens (light and dark), type roles, spacing, radius, control sizes and shell dimensions are defined once in the design file's Foundation page. The code carries them in [insight-theme.css](../../../../src/frontend/src/insight-theme.css) over the FrontX kit theme, and in [type-scale.ts](../../../../src/frontend/src/lib/type-scale.ts). This design does not repeat the values.

### 4.3 Shared Components

| Component | Variants and states |
|-----------|---------------------|
| Button | Primary, Outline, Ghost × Default, Hover, Focus, Disabled; Destructive for deletion |
| Badge | Neutral, Info, Success, Warning, Danger, Outline |
| Evidence pill | Strong, Directional, Gap, Broken; dot and label |
| Filter chip | Default, Selected; with count |
| Source chip | Connected, Partial, Missing; dot and source name |
| Rail item | Default, Hover, Active, Planned |
| Pane item | Default, Hover, Active, Planned; icon, label, count or badge |
| Pane header | Title of the pane |
| Search input | Default, Focus |
| Icon button | Default, Hover |
| Period control | Segmented presets, More menu with resolved dates, range button with UTC marker, calendar |
| KPI card | Title, change with direction, value with unit, caption |
| Widget header | Title, subtitle, actions menu |
| Dashboard card | Mine, Shared, Starter; icon, name, owner · age, visibility badge, tags, open, menu (Starter: open only) |
| Starter view card | Icon, name, question, source chips, Open, Save as dashboard |
| State card | Loading, Empty, Not connected, Withheld, Refused, Error with retry |

### 4.4 Cross-Cutting Concerns

**Session**: one browser tab leads session refresh, through a storage lease and a broadcast channel. When refresh fails, the shell shows the session-ended notice, then starts sign-in. A 401 from any service re-probes the session before treating it as ended.

**Untrusted content**: the portal renders service-provided text, including Assistant prose and definition descriptions, as text or restricted markdown, never as raw HTML.

**Feedback and usage paths**: feedback and usage events carry the redacted path of the screen, with person, scope, search and dashboard segments masked.

**Reads and writes**: a failed read shows a retry action. Writes are never retried automatically, so a failed write cannot apply twice. An edit conflict is shown with the option to load the latest version.

**Observability**: unhandled errors reach Sentry through the error boundary, which offers try-again and reload. Usage events: page view, window change, scope change, cohort change, drill-down, export, report column.

**Capacity and cost**: the portal is a static bundle with no server-side cost of its own. List pages are fetched 50 items at a time. Beyond a page's own reads, the only per-view load is the Assistant column and per-section reads.

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **ADRs**: [ADR/](./ADR/)
- **Features**: none yet
