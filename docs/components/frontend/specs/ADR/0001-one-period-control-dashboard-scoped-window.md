---
status: proposed
date: 2026-10-27
---

# ADR-0001: One Period Control, with a Window Scoped to Each Open Dashboard

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Keep a separate control per area](#keep-a-separate-control-per-area)
  - [One control and one portal-wide window that dashboards follow](#one-control-and-one-portal-wide-window-that-dashboards-follow)
  - [One control; a portal-wide window plus a window scoped to each open dashboard](#one-control-a-portal-wide-window-plus-a-window-scoped-to-each-open-dashboard)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-portal-adr-one-period-control`

## Context and Problem Statement

The portal shows period figures on Home, Explore, People, Reports and custom dashboards. A reader needs one way to pick the reporting window. But a dashboard is authored for a window and shared by address, while the rest of the portal is the reader's own working context. Which control picks the window, and whose window does an open dashboard read?

## Decision Drivers

* A reader learns one control and one preset list, and sees the same dates for the same preset everywhere.
* A shared dashboard address opens on the window its author meant, for every reader.
* Picking a window on one page must never silently change what another page shows.
* Every view stays reproducible from its address (`cpt-insightspec-portal-principle-address-state`).

## Considered Options

* Keep a separate control per area
* One control and one portal-wide window that dashboards follow
* One control; a portal-wide window plus a window scoped to each open dashboard

## Decision Outcome

Chosen option: "One control; a portal-wide window plus a window scoped to each open dashboard". It is the only option that gives readers one control, lets a shared dashboard open on its author's window, and keeps each page's window independent and in its address.

### Consequences

* Good, because every page uses the same control, presets and date resolution.
* Good, because a dashboard address carries its own window, so a shared link opens on the same dashboard and window for every reader; the figures still follow each reader's reach.
* Good, because changing the window on a dashboard never changes Home or Explore, and the reverse.
* Bad, because a reader moving from Explore to a dashboard may meet a different window and must read the toolbar to notice.
* Bad, because navigation code must keep two parameters apart: the portal-wide window and the dashboard window.
* Bad, because the services must resolve every preset the control offers, including Last 90 days, with the semantics the PRD states. The analytics and definition services own that change.
* Neutral, because from this decision on the portal ignores a dashboard's stored list of offered windows. The definition service keeps accepting that list until its own design retires it. A stored default the control no longer lists opens as a custom range with the same dates.
* Neutral, because a metric with a maximum window still refuses a longer preset; the widget shows that refusal with its reason instead of a figure.

### Confirmation

A design review of every page showing period figures confirms it renders the period control component, bound to either the portal window or its dashboard's window. A UI journey confirms that changing a dashboard's window leaves the portal window unchanged, and that a dashboard address with a window opens on that window.

## Pros and Cons of the Options

### Keep a separate control per area

Each area keeps its own control and preset list.

* Good, because no page changes.
* Bad, because readers learn several controls whose presets resolve to different dates.
* Bad, because a reader comparing a dashboard with a lens cannot pick the same window in both.

### One control and one portal-wide window that dashboards follow

A single window applies everywhere, dashboards included.

* Good, because one window is the simplest model to explain.
* Bad, because a shared dashboard opens on whatever window the reader last picked, not the one its author meant.
* Bad, because changing the window on a dashboard silently changes every other page.

### One control; a portal-wide window plus a window scoped to each open dashboard

The same control everywhere. Home, Explore, People and Reports share the portal window. An open dashboard reads its own window, opening on its stored default.

* Good, because readers learn one control.
* Good, because a dashboard's figures are reproducible from its address.
* Neutral, because the window shown on a dashboard may differ from the one just used elsewhere, though the toolbar always states the dates.
* Bad, because two window parameters must be kept distinct.

## More Information

Presets must resolve the same way on the services and in the portal, by `cpt-insightspec-portal-fr-period-semantics`, and the portal window and dashboard windows must use the same preset tokens. Today they differ: the portal resolves its own presets in local time and ends rolling ranges yesterday, and the services have no Last 90 days token. Both converge before this decision ships.

The per-dashboard restriction of which windows a reader may pick is retired: every dashboard offers every preset. The portal labels presets with dates from the rule the services use.

Decision owner: the portal's product owner. Revisit if readers need several dashboards to follow one shared window (for example, a board of dashboards read together), or if a dashboard must again restrict the windows it can be read over.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-portal-fr-period-control` — one control and preset list on every page
* `cpt-insightspec-portal-fr-period-scope` — portal-wide window and per-dashboard window
* `cpt-insightspec-portal-fr-period-semantics` — identical resolution of a preset everywhere
* `cpt-insightspec-portal-component-period-control` — the component bound to either window
